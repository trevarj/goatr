// SPDX-License-Identifier: GPL-3.0-or-later
//! Pair capabilities, retained host identity and the shared authorization/effect fence.
use crate::{protocol::*, state::State, storage::*};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::os::fd::OwnedFd;
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    future::Future,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore, watch};
use tokio::time::Instant;

pub const PAIR_SECONDS: u64 = 300;
pub const BOUNDARY_TIMEOUT: Duration = Duration::from_secs(10);
pub type SecurityResult<T> = Result<T, ErrorCode>;
fn thread_stop_requested(stop: &watch::Receiver<bool>) -> bool {
    stop.has_changed().is_err() || *stop.borrow()
}
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("string formatting");
    }
    text
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis()
        .try_into()
        .expect("clock overflow")
}
pub fn uid() -> u32 {
    // SAFETY: getuid has no pointer or lifetime preconditions.
    unsafe { libc::getuid() }
}
pub fn random32() -> io::Result<[u8; 32]> {
    let mut b = [0; 32];
    getrandom::fill(&mut b).map_err(io::Error::other)?;
    Ok(b)
}
pub fn store_error(e: StorageError) -> ErrorCode {
    match e {
        StorageError::Policy(code) => code,
        _ => ErrorCode::NotReady,
    }
}
fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe private path")
}
/// No normalization/following of a supplied alias: its spelling must already be canonical.
pub fn private_dir(path: &Path, create: bool) -> io::Result<()> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(unsafe_path());
    }
    if create && !path.try_exists()? {
        let parent = path.parent().ok_or_else(unsafe_path)?;
        if !parent.try_exists()? {
            private_dir(parent, true)?;
        }
        if fs::canonicalize(parent)? != parent {
            return Err(unsafe_path());
        }
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir()
        || m.uid() != uid()
        || m.mode() & 0o7777 != 0o700
        || fs::canonicalize(path)? != path
    {
        return Err(unsafe_path());
    }
    Ok(())
}
pub fn private_file(path: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.uid() != uid() || m.mode() & 0o7777 != 0o600 || m.nlink() != 1 {
        return Err(unsafe_path());
    }
    Ok(())
}
pub fn state_root() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(std::env::var_os("HOME").ok_or_else(unsafe_path)?).join(".local/state"),
    };
    if !base.is_absolute() {
        return Err(unsafe_path());
    }
    Ok(base.join("goatr"))
}
/// Held by serve until endpoint, admin and SQLite have all closed. Admin clients never call this.
pub struct HostIdentity {
    pub secret: iroh::SecretKey,
    pub receipt_key: [u8; 32],
    pub root: PathBuf,
    _lease: File,
}
impl HostIdentity {
    pub fn acquire(root: PathBuf) -> io::Result<Self> {
        private_dir(&root, true)?;
        let lock = root.join("serve.lock");
        let had_lease = fs::symlink_metadata(&lock).is_ok();
        match fs::symlink_metadata(&lock) {
            Ok(_) => private_file(&lock)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&lock)?;
        private_file(&lock)?;
        // SAFETY: the descriptor is valid and retained for the entire lease.
        if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "serve already owns this state",
            ));
        }
        let path = root.join("identity.keys");
        let marker = root.join("identity.initialized");
        let initialized = match fs::symlink_metadata(&marker) {
            Ok(_) => {
                private_file(&marker)?;
                let mut f = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&marker)?;
                if f.metadata()?.len() != 4 {
                    return Err(io::Error::other("protected identity marker corrupt"));
                }
                let mut magic = [0u8; 4];
                f.read_exact(&mut magic)?;
                if &magic != b"GAI1" {
                    return Err(io::Error::other("protected identity marker corrupt"));
                }
                true
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        let mut bytes = [0; 100];
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                if !initialized {
                    return Err(io::Error::other("protected identity marker missing"));
                }
                private_file(&path)?;
                let mut file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&path)?;
                if file.metadata()?.len() != 100 {
                    return Err(io::Error::other("protected identity corrupt"));
                }
                file.read_exact(&mut bytes)?;
                if &bytes[..4] != b"GAK1" || sha256(&bytes[..68]) != bytes[68..] {
                    bytes.fill(0);
                    return Err(io::Error::other("protected identity corrupt"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if initialized || had_lease {
                    return Err(io::Error::other(
                        "protected identity missing; restore the retained keys",
                    ));
                }
                // Installer bookkeeping alone is not a previous pairing identity. Everything
                // else (SQLite, receipts, unknown entries) prohibits fresh identity creation.
                for entry in fs::read_dir(&root)? {
                    let entry = entry?;
                    match entry.file_name().to_str() {
                        Some("serve.lock") => {}
                        Some("install") => private_dir(&entry.path(), false)?,
                        _ => {
                            return Err(io::Error::other(
                                "protected identity missing; restore the retained keys",
                            ));
                        }
                    }
                }
                let mut first = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&marker)?;
                first.write_all(b"GAI1")?;
                first.sync_all()?;
                File::open(&root)?.sync_all()?;
                bytes[..4].copy_from_slice(b"GAK1");
                bytes[4..36].copy_from_slice(&random32()?);
                bytes[36..68].copy_from_slice(&random32()?);
                let hash = sha256(&bytes[..68]);
                bytes[68..].copy_from_slice(&hash);
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                File::open(&root)?.sync_all()?;
            }
            Err(e) => return Err(e),
        }
        let secret = iroh::SecretKey::from_bytes(bytes[4..36].try_into().expect("fixed length"));
        let receipt_key = bytes[36..68].try_into().expect("fixed length");
        bytes.fill(0);
        Ok(Self {
            secret,
            receipt_key,
            root,
            _lease: lease,
        })
    }
}
impl Drop for HostIdentity {
    fn drop(&mut self) {
        self.receipt_key.fill(0);
    }
}
pub fn display_name() -> Label {
    let mut bytes = [0u8; 256];
    // SAFETY: valid writable bounded buffer; gethostname writes at most its length.
    let ok = unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } == 0;
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    if ok
        && let Ok(s) = std::str::from_utf8(&bytes[..end])
        && let Ok(name) = Label::new(s)
    {
        return name;
    }
    Label::new("Goatr host").expect("literal label")
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PairInvite {
    pub v: OneValue,
    pub host: EndpointId,
    pub name: Label,
    pub token: Key32,
    pub expires_at: Uint,
}
impl PairInvite {
    pub fn parse(uri: &str) -> SecurityResult<Self> {
        if uri.len() > 4096 {
            return Err(ErrorCode::InvalidRequest);
        }
        let data = uri
            .strip_prefix("goatr://pair/v1?data=")
            .ok_or(ErrorCode::InvalidRequest)?;
        if data.is_empty()
            || data.len() > 2731
            || !data
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(data)
            .map_err(|_| ErrorCode::InvalidRequest)?;
        if bytes.len() > 2048 || URL_SAFE_NO_PAD.encode(&bytes) != data {
            return Err(ErrorCode::InvalidRequest);
        }
        let value = parse_raw_json(&bytes).map_err(|_| ErrorCode::InvalidRequest)?;
        serde_json::from_value(value).map_err(|_| ErrorCode::InvalidRequest)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PairOpen {
    pub uri: String,
    pub expires_at: Uint,
}
struct Window {
    hash: [u8; 32],
    deadline: Instant,
}
#[derive(Default)]
struct Failures {
    next: u64,
    global: VecDeque<(u64, Instant)>,
    peers: HashMap<EndpointId, VecDeque<(u64, Instant)>>,
}
impl Failures {
    fn prune(&mut self, now: Instant) {
        let old = now - Duration::from_secs(60);
        self.global.retain(|(_, at)| *at > old);
        self.peers.retain(|_, v| {
            v.retain(|(_, at)| *at > old);
            !v.is_empty()
        });
    }
    fn reserve(&mut self, peer: &EndpointId) -> Option<u64> {
        let now = Instant::now();
        self.prune(now);
        if self.global.len() >= 30
            || self.peers.get(peer).is_some_and(|v| v.len() >= 5)
            || (self.peers.len() >= 256 && !self.peers.contains_key(peer))
        {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        let reservation = (self.next, now);
        self.global.push_back(reservation);
        self.peers
            .entry(peer.clone())
            .or_default()
            .push_back(reservation);
        Some(self.next)
    }
    fn finish(&mut self, peer: &EndpointId, id: u64, success: bool) {
        let now = Instant::now();
        self.prune(now);
        if success {
            self.global.retain(|(attempt, _)| *attempt != id);
            if let Some(attempts) = self.peers.get_mut(peer) {
                attempts.retain(|(attempt, _)| *attempt != id);
                if attempts.is_empty() {
                    self.peers.remove(peer);
                }
            }
        } else {
            if let Some((_, at)) = self.global.iter_mut().find(|(attempt, _)| *attempt == id) {
                *at = now;
            }
            if let Some(attempts) = self.peers.get_mut(peer)
                && let Some((_, at)) = attempts.iter_mut().find(|(attempt, _)| *attempt == id)
            {
                *at = now;
            }
        }
    }
}
/// Owns one pending failure allowance; cancellation/disconnect commits it exactly once.
pub struct HandshakeAttempt<'a> {
    security: &'a Security,
    peer: EndpointId,
    id: u64,
    success: bool,
}
impl HandshakeAttempt<'_> {
    pub fn succeeded(mut self) {
        self.success = true;
    }
}
impl Drop for HandshakeAttempt<'_> {
    fn drop(&mut self) {
        self.security
            .failures
            .lock()
            .expect("failures mutex")
            .finish(&self.peer, self.id, self.success);
    }
}
enum BoundaryOutcome {
    Receipt(ReceiptTransition),
    Creation(Box<CreationOutcome>),
}
pub(crate) type AgentTerminalAckGuard=Arc<dyn Fn(&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()>+Send+Sync>;
enum EnteredFacts {
    AgentShell {target:Target,source:Uuid,resources:ShellCreationResources,acknowledge:AgentTerminalAckGuard},
    Attachment {target:Target,progress:AttachmentProgress},
    WorktreeRemoval {target:Target,source:Uuid,candidate:Uuid,preview:Uuid,progress:WorktreeRemovalProgress},
    AgentAlias {target:Target,source:Uuid,result:Option<ActionResultAgentRenamed>},
    Agent((CreationResources,StageMetadata)),
    Shell {target:ShellTarget,source:Uuid,resources:ShellCreationResources},
    ShellInput {target:ShellTarget,source:Uuid,progress:ShellInputProgress},
}
impl EnteredFacts {
    #[cfg(test)]
    fn agent(&self)->Option<&(CreationResources,StageMetadata)> {match self {Self::Agent(facts)=>Some(facts),Self::AgentShell {..}|Self::Shell {..}|Self::ShellInput {..}|Self::AgentAlias {..}|Self::WorktreeRemoval {..}|Self::Attachment {..}=>None}}
    fn agent_mut(&mut self)->Option<&mut (CreationResources,StageMetadata)> {match self {Self::Agent(facts)=>Some(facts),Self::AgentShell {..}|Self::Shell {..}|Self::ShellInput {..}|Self::AgentAlias {..}|Self::WorktreeRemoval {..}|Self::Attachment {..}=>None}}
}
#[derive(Clone,Copy,PartialEq,Eq)]
enum EnteredEffectOwner {NativeSocket,AttachmentCleanup}
enum EnteredConnection {Native(OwnedFd),AttachmentCleanup}
struct Entered {
    effect_owner:EnteredEffectOwner,
    ticket: Mutex<Option<DispatchTicket>>,
    connection: StdMutex<Option<OwnedFd>>,
    invalidated: AtomicBool,
    started: AtomicBool,
    done: AtomicBool,
    synchronous: bool,
    outcome: StdMutex<Option<BoundaryOutcome>>,
    creation: Option<parking_lot::Mutex<EnteredFacts>>,
    admission_guard: StdMutex<Option<CreationAdmissionGuard>>,
}
impl Entered {
    fn invalidate(&self) -> SecurityResult<()> {
        if self.effect_owner==EnteredEffectOwner::AttachmentCleanup {
            self.invalidated.store(true,Ordering::Release);return Ok(())
        }
        if let Some(fd) = self
            .connection
            .lock()
            .expect("boundary descriptor mutex")
            .take()
        {
            // SAFETY: OwnedFd retains the actual connected stream socket through shutdown.
            let result = unsafe { libc::shutdown(fd.as_raw_fd(), libc::SHUT_RDWR) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOTCONN)
            {
                return Err(ErrorCode::NotReady);
            }
            self.invalidated.store(true, Ordering::Release);
        }
        if self.invalidated.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ErrorCode::NotReady)
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum BoundaryReadiness {
    Ready,
    Pending,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreationAdmissionGuard {
    Root,
    ShellEpoch,
    ResultView,
}
impl CreationAdmissionGuard {
    fn proof(self) -> Id {
        Id(match self {
            Self::Root => "shell_probe_root_changed_before_enqueue",
            Self::ShellEpoch => "shell_probe_shell_epoch_changed_before_enqueue",
            Self::ResultView => "shell_probe_result_view_changed_before_enqueue",
        }
        .into())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CreationAdmissionFailure {
    code: ErrorCode,
    guard: Option<CreationAdmissionGuard>,
}
impl CreationAdmissionFailure {
    pub(crate) fn guarded(code: ErrorCode, guard: CreationAdmissionGuard) -> Self {
        Self {
            code,
            guard: Some(guard),
        }
    }
    fn proof(self) -> Id {
        self.guard
            .map(CreationAdmissionGuard::proof)
            .unwrap_or_else(|| Id("native boundary admission denied before start".into()))
    }
}
impl From<ErrorCode> for CreationAdmissionFailure {
    fn from(code: ErrorCode) -> Self {
        Self { code, guard: None }
    }
}
/// Owns one original durable intent, not permission to write or a lookup by action ID.
pub(crate) struct UnstartedBoundary {
    security: Arc<Security>,
    auth: Arc<Authorized>,
    entry: Arc<Entered>,
    deadline: Instant,
    runtime: tokio::runtime::Handle,
}
impl UnstartedBoundary {
    /// Pending returns this same unstarted claim. The supplied effect is never polled then.
    pub(crate) async fn try_start<F, V>(
        self,
        validate: V,
        effect: F,
    ) -> SecurityResult<Option<Self>>
    where
        F: Future<Output = ReceiptTransition>,
        V: FnOnce() -> SecurityResult<BoundaryReadiness>,
    {
        let guard =
            match tokio::time::timeout_at(self.deadline, self.security.native_guard(&self.auth))
                .await
                .unwrap_or(Err(ErrorCode::Timeout))
            {
                Ok(guard) => guard,
                Err(code) => {
                    self.security.settle(&self.entry, no_effect(code)).await?;
                    return Err(code);
                }
            };
        let started = self
            .security
            .admit_native_boundary(
                &self.auth,
                &self.entry,
                guard,
                Some(self.deadline),
                validate,
                effect,
            )
            .await?;
        if started { Ok(None) } else { Ok(Some(self)) }
    }
    pub(crate) async fn fail(self, code: ErrorCode) -> SecurityResult<()> {
        if self.entry.started.load(Ordering::Acquire) {
            return Err(ErrorCode::NotReady);
        }
        self.security.settle(&self.entry, no_effect(code)).await
    }
}
impl Drop for UnstartedBoundary {
    fn drop(&mut self) {
        if self.entry.done.load(Ordering::Acquire) || self.entry.started.load(Ordering::Acquire) {
            return;
        }
        // Cancellation before start owns definitive no-effect proof. The fence keeps
        // the unique claim if cancellation interrupts its queued settlement.
        self.entry
            .outcome
            .lock()
            .expect("boundary outcome mutex")
            .get_or_insert_with(|| BoundaryOutcome::Receipt(no_effect(ErrorCode::NotReady)));
        let security = self.security.clone();
        let entry = self.entry.clone();
        self.runtime.spawn(async move {
            let _ = security.finish(&entry).await;
        });
    }
}
/// Non-clone facts checkpoint for the original entered creation boundary; no new ticket.
pub(crate) struct CreationCheckpoint<'a> {
    security: &'a Security,
    auth: &'a Authorized,
    entry: Arc<Entered>,
}
impl CreationCheckpoint<'_> {
    pub(crate) async fn guard(&self) -> SecurityResult<OwnedMutexGuard<()>> {
        let guard = self.security.native_guard(self.auth).await?;
        if self.entry.done.load(Ordering::Acquire)
            || self.entry.invalidated.load(Ordering::Acquire)
            || !self.entry.started.load(Ordering::Acquire)
            || self.entry.ticket.lock().await.is_none()
        {
            return Err(ErrorCode::NotReady);
        }
        Ok(guard)
    }
    pub(crate) async fn workspace(&self, resources:CreationResources, metadata:StageMetadata)->SecurityResult<()> {
        resources.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        {
            let ticket=self.entry.ticket.try_lock().map_err(|_|ErrorCode::NotReady)?;
            if ticket.is_none() || self.entry.done.load(Ordering::Acquire) || !self.entry.started.load(Ordering::Acquire) {return Err(ErrorCode::NotReady)}
            let mut held_facts=self.entry.creation.as_ref().ok_or(ErrorCode::NotReady)?.lock();
            let retained=held_facts.agent_mut().ok_or(ErrorCode::NotReady)?;
            crate::storage::retain_creation_facts(&retained.0,&retained.1,&resources,&metadata).map_err(store_error)?;
            // The original entered ticket owns these facts before the next await.
            *retained=(resources.clone(),metadata.clone());
        }
        let _guard=self.guard().await?;
        let entry=self.entry.clone();
        self.security.storage.call(move|store| {
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::NotReady))?;
            let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) {return Err(StorageError::Policy(ErrorCode::NotReady))}
            store.checkpoint_workspace_facts(ticket,resources,metadata,now_ms())
        }).await.map_err(store_error)
    }
    pub(crate) async fn acknowledge(&self, thread: Id) -> SecurityResult<()> {
        thread.validate().map_err(|_| ErrorCode::InvalidRequest)?;
        {
            let held = self
                .entry
                .ticket
                .try_lock()
                .map_err(|_| ErrorCode::NotReady)?;
            if held.is_none()
                || self.entry.done.load(Ordering::Acquire)
                || !self.entry.started.load(Ordering::Acquire)
            {
                return Err(ErrorCode::NotReady);
            }
            let mut held_facts = self
                .entry
                .creation
                .as_ref()
                .ok_or(ErrorCode::NotReady)?
                .lock();
            let creation=held_facts.agent_mut().ok_or(ErrorCode::NotReady)?;
            if creation.0.native_id.is_some() {
                return Err(ErrorCode::InvalidRequest);
            }
            // The admitted start's acknowledgement survives cancellation at the next await.
            creation.0.native_id = Some(thread.clone());
        }
        let _guard = self.guard().await?;
        let entry = self.entry.clone();
        self.security
            .storage
            .call(move |s| {
                let held = entry
                    .ticket
                    .try_lock()
                    .map_err(|_| StorageError::Policy(ErrorCode::NotReady))?;
                let ticket = held
                    .as_ref()
                    .ok_or(StorageError::Policy(ErrorCode::NotReady))?;
                if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) {
                    return Err(StorageError::Policy(ErrorCode::NotReady));
                }
                s.checkpoint_created_thread(ticket, thread, now_ms())
            })
            .await
            .map_err(store_error)
    }
}
pub struct PeerFence {
    gate: Arc<Mutex<()>>,
    stopped: AtomicBool,
    entered: StdMutex<Vec<Arc<Entered>>>,
    active: StdMutex<Option<(Uuid, iroh::endpoint::Connection)>>,
}
impl PeerFence {
    fn new() -> Self {
        Self {
            gate: Arc::new(Mutex::new(())),
            stopped: AtomicBool::new(false),
            entered: StdMutex::new(Vec::new()),
            active: StdMutex::new(None),
        }
    }
}
pub struct Authorized {
    pub proof: AuthenticatedPeer,
    pub fence: Arc<PeerFence>,
    pub connection_id: Uuid,
}
/// One concrete owner shared by transport, admin and later native adapters; no second peer cache.
pub struct Security {
    pub(crate) push:std::sync::OnceLock<Arc<crate::push::Publisher>>,
    pub(crate) push_failed:AtomicBool,
    pub storage: Arc<StorageOwner>,
    pub state: Arc<State>,
    pub host: EndpointId,
    pub boot: Uuid,
    pub backends: Vec<Backend>,
    window: Mutex<Option<Window>>,
    failures: StdMutex<Failures>,
    unauth: Arc<Semaphore>,
    peers: StdMutex<HashMap<Uuid, Arc<PeerFence>>>,
}
impl Security {
    pub fn new(
        storage: StorageOwner,
        state: Arc<State>,
        host: EndpointId,
        boot: Uuid,
        backends: Vec<Backend>,
    ) -> Arc<Self> {
        Arc::new(Self {
            push:std::sync::OnceLock::new(),
            push_failed:AtomicBool::new(false),
            storage: Arc::new(storage),
            state,
            host,
            boot,
            backends,
            window: Mutex::new(None),
            failures: StdMutex::new(Failures::default()),
            unauth: Arc::new(Semaphore::new(8)),
            peers: StdMutex::new(HashMap::new()),
        })
    }
    pub fn admission(&self) -> SecurityResult<OwnedSemaphorePermit> {
        self.unauth
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorCode::Busy)
    }
    pub fn handshake(&self, peer: &EndpointId) -> Option<HandshakeAttempt<'_>> {
        let id = self
            .failures
            .lock()
            .expect("failures mutex")
            .reserve(peer)?;
        Some(HandshakeAttempt {
            security: self,
            peer: peer.clone(),
            id,
            success: false,
        })
    }
    pub async fn open_pair(&self) -> SecurityResult<PairOpen> {
        let mut token = random32().map_err(|_| ErrorCode::NotReady)?;
        let expires = Uint::new(now_ms() + PAIR_SECONDS * 1000).map_err(|_| ErrorCode::NotReady)?;
        let invite = PairInvite {
            v: OneValue(1),
            host: self.host.clone(),
            name: display_name(),
            token: Key32(URL_SAFE_NO_PAD.encode(token)),
            expires_at: expires,
        };
        let hash = hmac_sha256(&[0; 32], &token);
        token.fill(0);
        let json = serde_json::to_vec(&invite).map_err(|_| ErrorCode::NotReady)?;
        if json.len() > 2048 {
            return Err(ErrorCode::ResourceLimit);
        }
        let uri = format!("goatr://pair/v1?data={}", URL_SAFE_NO_PAD.encode(json));
        *self.window.lock().await = Some(Window {
            hash,
            deadline: Instant::now() + Duration::from_secs(PAIR_SECONDS),
        });
        Ok(PairOpen {
            uri,
            expires_at: expires,
        })
    }
    pub async fn confirm(
        &self,
        connection: &iroh::endpoint::Connection,
        params: PairConfirmParams,
    ) -> SecurityResult<PairConfirmResult> {
        let remote = EndpointId(connection.remote_id().to_string());
        if connection.alpn() != crate::ALPN {
            return Err(ErrorCode::PairingFailed);
        }
        remote.validate().map_err(|_| ErrorCode::PairingFailed)?;
        params.validate().map_err(|_| ErrorCode::PairingFailed)?;
        let mut window = self.window.lock().await;
        let mut token = URL_SAFE_NO_PAD
            .decode(&params.token.0)
            .map_err(|_| ErrorCode::PairingFailed)?;
        let expected = window.as_ref().map_or([0; 32], |w| w.hash);
        let matches = verify_hmac_sha256(&[0; 32], &token, &expected);
        token.fill(0);
        if !matches || !window.as_ref().is_some_and(|w| Instant::now() < w.deadline) {
            return Err(ErrorCode::PairingFailed);
        }
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: remote,
            device_name: params.device_name,
            paired_at: Uint(now_ms()),
            last_seen_at: None,
        };
        let id = device.device_id.clone();
        // Retire before the cancellable await: a lost response/cancelled waiter cannot resurrect
        // a capability whose SQLite owner may already have committed. Failed commit stays spent.
        *window = None;
        self.storage
            .call(move |s| s.pair_device(&device))
            .await
            .map_err(|_| ErrorCode::PairingFailed)?;
        Ok(PairConfirmResult {
            host_id: self.host.clone(),
            device_id: id,
            paired: TrueValue(true),
        })
    }
    fn peer(&self, id: &Uuid) -> Arc<PeerFence> {
        self.peers
            .lock()
            .expect("peers mutex")
            .entry(id.clone())
            .or_insert_with(|| Arc::new(PeerFence::new()))
            .clone()
    }
    pub async fn hello(
        &self,
        remote: EndpointId,
        params: HelloParams,
        connection: iroh::endpoint::Connection,
    ) -> SecurityResult<Authorized> {
        if remote.0 != connection.remote_id().to_string() || connection.alpn() != crate::ALPN {
            return Err(ErrorCode::Unauthorized);
        }
        let (id, epoch) = self
            .storage
            .call(move |s| s.current_epoch(&remote))
            .await
            .map_err(store_error)?
            .ok_or(ErrorCode::Unauthorized)?;
        let fence = self.peer(&id);
        let gate = fence.gate.clone().lock_owned().await;
        if fence.stopped.load(Ordering::Acquire) {
            return Err(ErrorCode::Unauthorized);
        }
        let proof = self
            .storage
            .call(move |s| {
                let p = s.authenticated_hello(&id, epoch)?;
                s.mark_seen(&p, params.device_name, now_ms())?;
                Ok(p)
            })
            .await
            .map_err(store_error)?;
        let connection_id = self.state.connect(&proof);
        // Complete authorization before replacing the old connection; epoch stays unchanged.
        if let Some((old, c)) = fence
            .active
            .lock()
            .expect("active mutex")
            .replace((connection_id.clone(), connection))
        {
            self.state.disconnect(&old);
            c.close(0u32.into(), b"replaced");
        }
        drop(gate);
        Ok(Authorized {
            proof,
            fence,
            connection_id,
        })
    }
    /// Hold through every read/lookup/reservation/output boundary. Never hold during a full turn.
    pub async fn guard(&self, auth: &Authorized) -> SecurityResult<OwnedMutexGuard<()>> {
        let guard = self.native_guard(auth).await?;
        if !auth
            .fence
            .active
            .lock()
            .expect("active mutex")
            .as_ref()
            .is_some_and(|(id, _)| id == &auth.connection_id)
        {
            return Err(ErrorCode::Unauthorized);
        }
        Ok(guard)
    }
    /// Native jobs survive connection replacement/disconnect, but NEVER authorization removal.
    /// Request/read/output guards additionally require the currently active hello connection.
    pub async fn native_guard(&self, auth: &Authorized) -> SecurityResult<OwnedMutexGuard<()>> {
        if auth.fence.stopped.load(Ordering::Acquire) {
            return Err(ErrorCode::Unauthorized);
        }
        let guard = auth.fence.gate.clone().lock_owned().await;
        let p = auth.proof.clone();
        self.storage
            .call(move |s| s.recheck(&p))
            .await
            .map_err(store_error)?;
        if auth.fence.stopped.load(Ordering::Acquire) {
            return Err(ErrorCode::Unauthorized);
        }
        Ok(guard)
    }
    /// Matching receipts precede native readiness, but never authorization. Move the original
    /// request through the SQLite owner; no extra request/secret copy survives the later wait.
    pub(crate) async fn preflight_request(
        &self,
        auth: &Authorized,
        request: Request,
    ) -> SecurityResult<(Request, Option<Receipt>)> {
        let _guard = self.guard(auth).await?;
        let proof = auth.proof.clone();
        self.storage
            .call(move |s| {
                let existing = s.preflight_request(&proof, &request, now_ms())?;
                Ok((request, existing))
            })
            .await
            .map_err(store_error)
    }
    /// Register an original job only after commit, on the serialized storage owner and before
    /// replying to the cancellable caller. Existing receipts and failed reserves never call it.
    pub async fn reserve(
        &self,
        auth: &Authorized,
        request: Request,
        validate: impl FnOnce(&Request) -> Result<(), ErrorCode> + Send + 'static,
        committed: impl FnOnce(Receipt) + Send + 'static,
    ) -> SecurityResult<Reservation> {
        let _guard = self.guard(auth).await?;
        let p = auth.proof.clone();
        self.storage
            .call(move |s| {
                let reservation = s.reserve(&p, &request, now_ms(), validate)?;
                if let Reservation::Reserved(receipt) = &reservation {
                    committed(receipt.clone());
                }
                Ok(reservation)
            })
            .await
            .map_err(store_error)
    }
    pub fn disconnect(&self, auth: &Authorized) {
        let mut active = auth.fence.active.lock().expect("active mutex");
        if active
            .as_ref()
            .is_some_and(|(id, _)| id == &auth.connection_id)
        {
            active.take();
        }
        self.state.disconnect(&auth.connection_id);
    }
    /// Pass an owned duplicate of the ACTUAL provider stream socket used by `effect`.
    /// Kernel shutdown disables every duplicate FD; cancelling an async future is not proof.
    /// The target validator runs after durable intent, immediately before boundary admission.
    pub async fn boundary<F, V>(
        &self,
        auth: &Authorized,
        action: Uuid,
        validate: V,
        connection: OwnedFd,
        effect: F,
    ) -> SecurityResult<()>
    where
        F: Future<Output = ReceiptTransition>,
        V: FnOnce() -> SecurityResult<()>,
    {
        let (entry, guard) = self
            .prepare_native_boundary(auth, action, connection, None, None, |entry| entry)
            .await?;
        self.admit_native_boundary(
            auth,
            &entry,
            guard,
            None,
            || validate().map(|()| BoundaryReadiness::Ready),
            effect,
        )
        .await
        .map(|_| ())
    }
    pub(crate) async fn unstarted_boundary(
        self: &Arc<Self>,
        auth: &Arc<Authorized>,
        action: Uuid,
        connection: OwnedFd,
        deadline: Instant,
    ) -> SecurityResult<UnstartedBoundary> {
        let security = self.clone();
        let original = auth.clone();
        let runtime = tokio::runtime::Handle::current();
        let (checkpoint, guard) = self
            .prepare_native_boundary(auth, action, connection, Some(deadline), None, move |entry| {
                UnstartedBoundary {
                    security,
                    auth: original,
                    entry,
                    deadline,
                    runtime,
                }
            })
            .await?;
        drop(guard);
        Ok(checkpoint)
    }
    pub(crate) async fn shell_creation_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,target:ShellTarget,source:Uuid,connection:OwnedFd,
        validate:impl FnOnce()->SecurityResult<()>,effect:F,
    )->SecurityResult<()> where F:FnOnce(ShellCreationCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition> {
        let (entry,guard)=self.prepare_native_boundary(auth,action,connection,None,Some(EnteredFacts::Shell {target,source,resources:ShellCreationResources::default()}),|entry|entry).await?;
        let checkpoint=ShellCreationCheckpoint {security:self,auth,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||validate().map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
    pub(crate) async fn terminal_creation_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,target:Target,source:Uuid,connection:OwnedFd,acknowledge:AgentTerminalAckGuard,
        validate:impl FnOnce()->SecurityResult<()>,effect:F,
    )->SecurityResult<()> where F:FnOnce(AgentTerminalCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition> {
        let (entry,guard)=self.prepare_native_boundary(auth,action,connection,None,Some(EnteredFacts::AgentShell {target,source,resources:ShellCreationResources::default(),acknowledge}),|entry|entry).await?;
        let checkpoint=AgentTerminalCheckpoint {security:self,auth,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||validate().map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
    pub(crate) async fn shell_input_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,target:ShellTarget,source:Uuid,connection:OwnedFd,
        validate:impl FnOnce()->SecurityResult<()>,effect:F,
    )->SecurityResult<()> where F:FnOnce(ShellInputCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition> {
        let (entry,guard)=self.prepare_native_boundary(auth,action,connection,None,Some(EnteredFacts::ShellInput {target,source,progress:ShellInputProgress::default()}),|entry|entry).await?;
        let checkpoint=ShellInputCheckpoint {security:self,auth,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||validate().map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
    pub(crate) async fn agent_alias_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,target:Target,source:Uuid,connection:OwnedFd,
        validate:impl FnOnce()->SecurityResult<()>,effect:F,
    )->SecurityResult<()> where F:FnOnce(AgentAliasCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition> {
        let (entry,guard)=self.prepare_native_boundary(auth,action,connection,None,Some(EnteredFacts::AgentAlias {target,source,result:None}),|entry|entry).await?;
        let checkpoint=AgentAliasCheckpoint {security:self,auth,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||validate().map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
    pub(crate) async fn worktree_removal_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,params:WorktreesRemoveParams,source:Uuid,connection:OwnedFd,
        validate:impl FnOnce()->SecurityResult<()>,effect:F,
    )->SecurityResult<()> where F:FnOnce(WorktreeRemovalCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition> {
        let (entry,guard)=self.prepare_native_boundary(auth,action,connection,None,Some(EnteredFacts::WorktreeRemoval {target:params.target,source,candidate:params.candidate_id,preview:params.preview_id,progress:WorktreeRemovalProgress::default()}),|entry|entry).await?;
        let checkpoint=WorktreeRemovalCheckpoint {security:self,auth,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||validate().map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
    async fn prepare_native_boundary<T: Send + 'static>(
        &self,
        auth: &Authorized,
        action: Uuid,
        connection: OwnedFd,
        deadline: Option<Instant>,
        facts:Option<EnteredFacts>,
        wrap: impl FnOnce(Arc<Entered>) -> T + Send + 'static,
    ) -> SecurityResult<(T, OwnedMutexGuard<()>)> {
        stream_socket(&connection)?;
        self.prepare_entered_boundary(auth,action,EnteredConnection::Native(connection),deadline,facts,wrap).await
    }
    async fn prepare_entered_boundary<T: Send + 'static>(
        &self,
        auth: &Authorized,
        action: Uuid,
        connection: EnteredConnection,
        deadline: Option<Instant>,
        facts:Option<EnteredFacts>,
        wrap: impl FnOnce(Arc<Entered>) -> T + Send + 'static,
    ) -> SecurityResult<(T, OwnedMutexGuard<()>)> {
        let mut facts=facts;
        let (connection,effect_owner)=match connection {EnteredConnection::Native(fd)=>(Some(fd),EnteredEffectOwner::NativeSocket),EnteredConnection::AttachmentCleanup=>(None,EnteredEffectOwner::AttachmentCleanup)};
        let guard = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, self.native_guard(auth))
                .await
                .map_err(|_| ErrorCode::Timeout)??,
            None => self.native_guard(auth).await?,
        };
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(ErrorCode::Timeout);
        }
        if let Err(code) = self.boundary_capacity(&auth.fence) {
            let p = auth.proof.clone();
            self.storage
                .call(move |s| s.transition(&p, &action, no_effect(code)).map(|_| ()))
                .await
                .map_err(store_error)?;
            return Err(code);
        }
        let p = auth.proof.clone();
        let fence = auth.fence.clone();
        let entry = self
            .storage
            .call(move |s| {
                if let Some(EnteredFacts::Attachment {target,progress})=facts.as_mut() {
                    if attachment_claimed(&fence,&progress.upload_id){return Err(StorageError::Policy(ErrorCode::Busy))}
                    // Pin durable prior facts while this admission gate excludes competing upload jobs.
                    if let Some(record)=s.attachment_load(p.peer_id(),&progress.upload_id)? {
                        if record.target!=*target{return Err(StorageError::Policy(ErrorCode::StaleSession))}
                        *progress=record.progress();
                    }
                }

                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(StorageError::Policy(ErrorCode::Timeout));
                }
                s.transition(
                    &p,
                    &action,
                    ReceiptTransition {
                        state: ReceiptState::Dispatching,
                        proof: TransitionProof::Intent,
                        result: match facts.as_ref(){Some(EnteredFacts::Attachment {target,progress})=>Some(ActionResult::Attachment(ActionResultAttachment {target:target.clone(),progress:progress.clone()})),_=>None},
                        error: None,
                        at: now_ms(),
                    },
                )?;
                let ticket = s.dispatch_ticket(&p, &action)?;
                let entry = Arc::new(Entered {
                    effect_owner,
                    ticket: Mutex::new(Some(ticket)),
                    connection: StdMutex::new(connection),
                    invalidated: AtomicBool::new(false),
                    started: AtomicBool::new(false),
                    done: AtomicBool::new(false),
                    synchronous: false,
                    outcome: StdMutex::new(None),
                    creation: facts.map(parking_lot::Mutex::new),
                    admission_guard: StdMutex::new(None),
                });
                // Install ownership inside the SQLite job, BEFORE replying to a cancellable waiter.
                fence
                    .entered
                    .lock()
                    .expect("entered mutex")
                    .push(entry.clone());
                // Construct the original owner before a cancellable reply handoff.
                Ok(wrap(entry))
            })
            .await
            .map_err(store_error)?;
        Ok((entry, guard))
    }
    async fn admit_native_boundary<F, V>(
        &self,
        auth: &Authorized,
        entry: &Arc<Entered>,
        guard: OwnedMutexGuard<()>,
        deadline: Option<Instant>,
        validate: V,
        effect: F,
    ) -> SecurityResult<bool>
    where
        F: Future<Output = ReceiptTransition>,
        V: FnOnce() -> SecurityResult<BoundaryReadiness>,
    {
        if entry.done.load(Ordering::Acquire)
            || entry.started.load(Ordering::Acquire)
            || entry.invalidated.load(Ordering::Acquire)
        {
            return Err(ErrorCode::NotReady);
        }
        let admission = if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            Err(ErrorCode::Timeout)
        } else {
            validate()
        }
        .and_then(|readiness| {
            if auth.fence.stopped.load(Ordering::Acquire) {
                Err(ErrorCode::Unauthorized)
            } else if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                Err(ErrorCode::Timeout)
            } else {
                Ok(readiness)
            }
        });
        match admission {
            Err(code) => {
                self.settle(entry, no_effect(code)).await?;
                return Err(code);
            }
            Ok(BoundaryReadiness::Pending) => {
                drop(guard);
                return Ok(false);
            }
            Ok(BoundaryReadiness::Ready) => {}
        }
        entry.started.store(true, Ordering::Release);
        drop(guard);
        let outcome = match tokio::time::timeout(BOUNDARY_TIMEOUT, effect).await {
            Ok(outcome) => outcome,
            Err(_) => {
                entry.invalidate()?;
                lost()
            }
        };
        self.settle(entry, outcome).await?;
        Ok(true)
    }
    /// Each creation stage has its own short boundary. No job/turn lifetime holds the fence.
    pub async fn creation_boundary<F>(
        &self,
        auth: &Authorized,
        action: Uuid,
        intent: (CreationStage, StageMetadata),
        validate: impl FnOnce() -> SecurityResult<()>,
        connection: OwnedFd,
        effect: F,
    ) -> SecurityResult<()>
    where
        F: Future<Output = CreationOutcome>,
    {
        self.creation_boundary_guarded(
            auth,
            action,
            intent,
            || validate().map_err(Into::into),
            connection,
            effect,
        )
        .await
    }
    pub(crate) async fn creation_boundary_guarded<F>(
        &self,
        auth: &Authorized,
        action: Uuid,
        intent: (CreationStage, StageMetadata),
        validate: impl FnOnce() -> Result<(), CreationAdmissionFailure>,
        connection: OwnedFd,
        effect: F,
    ) -> SecurityResult<()>
    where
        F: Future<Output = CreationOutcome>,
    {
        self.creation_boundary_entered(auth, action, intent, validate, connection, None, None, |_| effect)
            .await
    }
    pub(crate) async fn workspace_creation_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,metadata:StageMetadata,
        validate:impl FnOnce()->SecurityResult<()>,connection:OwnedFd,effect:F,
    )->SecurityResult<()>
    where F:FnOnce(CreationCheckpoint<'a>)->E,E:Future<Output=CreationOutcome> {
        self.creation_boundary_entered(auth,action,(CreationStage::Workspace,metadata),||validate().map_err(Into::into),connection,None,None,|entry|effect(CreationCheckpoint {security:self,auth,entry})).await
    }
    pub(crate) async fn thread_creation_boundary<'a, F, E>(
        &'a self,
        auth: &'a Authorized,
        action: Uuid,
        metadata: StageMetadata,
        deadline: Instant,
        stop: watch::Receiver<bool>,
        validate: impl FnOnce() -> SecurityResult<()>,
        connection: OwnedFd,
        effect: F,
    ) -> SecurityResult<()>
    where
        F: FnOnce(CreationCheckpoint<'a>) -> E,
        E: Future<Output = CreationOutcome>,
    {
        self.creation_boundary_entered(
            auth,
            action,
            (CreationStage::Thread, metadata),
            || validate().map_err(Into::into),
            connection,
            Some(deadline),
            Some(stop),
            |entry| {
                effect(CreationCheckpoint {
                    security: self,
                    auth,
                    entry,
                })
            },
        )
        .await
    }
    async fn creation_boundary_entered<F, E>(
        &self,
        auth: &Authorized,
        action: Uuid,
        intent: (CreationStage, StageMetadata),
        validate: impl FnOnce() -> Result<(), CreationAdmissionFailure>,
        connection: OwnedFd,
        deadline: Option<Instant>,
        mut stop: Option<watch::Receiver<bool>>,
        effect: F,
    ) -> SecurityResult<()>
    where
        F: FnOnce(Arc<Entered>) -> E,
        E: Future<Output = CreationOutcome>,
    {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || stop.as_ref().is_some_and(thread_stop_requested)
        {
            return Err(ErrorCode::Timeout);
        }
        let guard = if let (Some(deadline), Some(stop)) = (deadline, stop.as_mut()) {
            tokio::select! {
                guarded = tokio::time::timeout_at(deadline, self.native_guard(auth)) => {
                    guarded.map_err(|_| ErrorCode::Timeout)??
                }
                changed = stop.changed() => {
                    let _ = changed;
                    return Err(ErrorCode::Timeout);
                }
            }
        } else {
            self.native_guard(auth).await?
        };
        if deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || stop.as_ref().is_some_and(thread_stop_requested)
        {
            drop(guard);
            return Err(ErrorCode::Timeout);
        }
        stream_socket(&connection)?;
        if let Err(code) = self.boundary_capacity(&auth.fence) {
            let p = auth.proof.clone();
            self.storage
                .call(move |s| s.transition(&p, &action, no_effect(code)).map(|_| ()))
                .await
                .map_err(store_error)?;
            return Err(code);
        }
        let (stage, metadata) = intent;
        let p = auth.proof.clone();
        let fence = auth.fence.clone();
        let entry = self
            .storage
            .call(move |s| {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(StorageError::Policy(ErrorCode::Timeout));
                }
                let current = s.status(&p, &action, now_ms())?;
                if current.is_some_and(|r| r.state == ReceiptState::Reserved) {
                    s.transition(
                        &p,
                        &action,
                        ReceiptTransition {
                            state: ReceiptState::Dispatching,
                            proof: TransitionProof::Intent,
                            result: None,
                            error: None,
                            at: now_ms(),
                        },
                    )?;
                }
                let receipt = s.creation_intent(&p, &action, stage, metadata.clone(), now_ms())?;
                let resources = receipt.creation.ok_or(StorageError::Corrupt)?.resources;
                let ticket = s.dispatch_ticket(&p, &action)?;
                let entry = Arc::new(Entered {
                    effect_owner:EnteredEffectOwner::NativeSocket,
                    ticket: Mutex::new(Some(ticket)),
                    connection: StdMutex::new(Some(connection)),
                    invalidated: AtomicBool::new(false),
                    started: AtomicBool::new(false),
                    done: AtomicBool::new(false),
                    synchronous: false,
                    outcome: StdMutex::new(None),
                    creation: Some(parking_lot::Mutex::new(EnteredFacts::Agent((resources, metadata)))),
                    admission_guard: StdMutex::new(None),
                });
                fence
                    .entered
                    .lock()
                    .expect("entered mutex")
                    .push(entry.clone());
                Ok(entry)
            })
            .await
            .map_err(store_error)?;
        let admission = validate().and_then(|()| {
            if auth.fence.stopped.load(Ordering::Acquire) {
                Err(ErrorCode::Unauthorized.into())
            } else if deadline.is_some_and(|deadline| Instant::now() >= deadline)
                || stop.as_ref().is_some_and(thread_stop_requested)
            {
                Err(ErrorCode::Timeout.into())
            } else {
                Ok(())
            }
        });
        if let Err(failure) = admission {
            if let Some(guard) = failure.guard {
                *entry
                    .admission_guard
                    .lock()
                    .expect("creation admission guard mutex") = Some(guard);
            }
            self.settle(&entry, no_effect_admission(failure)).await?;
            return Err(failure.code);
        }
        entry.started.store(true, Ordering::Release);
        drop(guard);
        if deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || stop.as_ref().is_some_and(thread_stop_requested)
        {
            entry.invalidate()?;
            return self.settle(&entry, lost()).await;
        }
        match (deadline, stop) {
            (Some(deadline), Some(mut stop)) => {
                let outcome = tokio::select! {
                    biased;
                    changed = stop.changed() => {
                        let _ = changed;
                        None
                    }
                    _ = tokio::time::sleep_until(deadline) => None,
                    outcome = effect(entry.clone()) => {
                        if Instant::now() >= deadline || thread_stop_requested(&stop) {
                            None
                        } else {
                            Some(outcome)
                        }
                    }
                };
                match outcome {
                    Some(outcome) => self.settle_creation(&entry, outcome).await,
                    None => {
                        entry.invalidate()?;
                        self.settle(&entry, lost()).await
                    }
                }
            }
            (None, None) => match tokio::time::timeout(BOUNDARY_TIMEOUT, effect(entry.clone())).await {
                Ok(outcome) => self.settle_creation(&entry, outcome).await,
                Err(_) => {
                    entry.invalidate()?;
                    self.settle(&entry, lost()).await
                }
            },
            _ => Err(ErrorCode::NotReady),
        }
    }
    /// Only the actual bounded synchronous OS/service start admission belongs in `start`.
    /// No await, readiness polling, turn/job wait or untracked async task is permitted inside it.
    /// Err means definite no effect; an ambiguous start returns an Uncertain CreationOutcome.
    /// The returned resource/ack is owned by the caller; revoke never kills an admitted process.
    pub async fn start_boundary<T>(
        &self,
        auth: &Authorized,
        action: Uuid,
        intent: (CreationStage, StageMetadata),
        validate: impl FnOnce() -> SecurityResult<()>,
        start: impl FnOnce() -> SecurityResult<(T, CreationOutcome)>,
    ) -> SecurityResult<T> {
        let guard = self.native_guard(auth).await?;
        if let Err(code) = self.boundary_capacity(&auth.fence) {
            let p = auth.proof.clone();
            self.storage
                .call(move |s| s.transition(&p, &action, no_effect(code)).map(|_| ()))
                .await
                .map_err(store_error)?;
            return Err(code);
        }
        let (stage, metadata) = intent;
        let p = auth.proof.clone();
        let fence = auth.fence.clone();
        let entry = self
            .storage
            .call(move |s| {
                let current = s.status(&p, &action, now_ms())?;
                if current.is_some_and(|r| r.state == ReceiptState::Reserved) {
                    s.transition(
                        &p,
                        &action,
                        ReceiptTransition {
                            state: ReceiptState::Dispatching,
                            proof: TransitionProof::Intent,
                            result: None,
                            error: None,
                            at: now_ms(),
                        },
                    )?;
                }
                let receipt = s.creation_intent(&p, &action, stage, metadata.clone(), now_ms())?;
                let resources = receipt.creation.ok_or(StorageError::Corrupt)?.resources;
                let ticket = s.dispatch_ticket(&p, &action)?;
                let entry = Arc::new(Entered {
                    effect_owner:EnteredEffectOwner::NativeSocket,
                    ticket: Mutex::new(Some(ticket)),
                    connection: StdMutex::new(None),
                    invalidated: AtomicBool::new(false),
                    started: AtomicBool::new(false),
                    done: AtomicBool::new(false),
                    synchronous: true,
                    outcome: StdMutex::new(None),
                    creation: Some(parking_lot::Mutex::new(EnteredFacts::Agent((resources, metadata)))),
                    admission_guard: StdMutex::new(None),
                });
                fence
                    .entered
                    .lock()
                    .expect("entered mutex")
                    .push(entry.clone());
                Ok(entry)
            })
            .await
            .map_err(store_error)?;
        let admission = validate().and_then(|()| {
            if auth.fence.stopped.load(Ordering::Acquire) {
                Err(ErrorCode::Unauthorized)
            } else {
                Ok(())
            }
        });
        if let Err(code) = admission {
            self.settle(&entry, no_effect(code)).await?;
            return Err(code);
        }
        entry.started.store(true, Ordering::Release);
        let (value, outcome) = match start() {
            Ok(v) => v,
            Err(code) => {
                self.settle(&entry, no_effect(code)).await?;
                return Err(code);
            }
        };
        *entry.outcome.lock().expect("boundary outcome mutex") =
            Some(BoundaryOutcome::Creation(Box::new(outcome)));
        drop(guard);
        self.finish(&entry).await?;
        Ok(value)
    }
    fn boundary_capacity(&self, fence: &PeerFence) -> SecurityResult<()> {
        let mut entries = fence.entered.lock().expect("entered mutex");
        entries.retain(|e| !e.done.load(Ordering::Acquire));
        if entries.len() >= 64 {
            Err(ErrorCode::Busy)
        } else {
            Ok(())
        }
    }
    async fn settle(&self, entry: &Arc<Entered>, outcome: ReceiptTransition) -> SecurityResult<()> {
        entry
            .outcome
            .lock()
            .expect("boundary outcome mutex")
            .get_or_insert(BoundaryOutcome::Receipt(outcome));
        self.finish(entry).await
    }
    async fn settle_creation(
        &self,
        entry: &Arc<Entered>,
        outcome: CreationOutcome,
    ) -> SecurityResult<()> {
        entry
            .outcome
            .lock()
            .expect("boundary outcome mutex")
            .get_or_insert_with(|| BoundaryOutcome::Creation(Box::new(outcome)));
        self.finish(entry).await
    }
    async fn finish(&self, entry: &Arc<Entered>) -> SecurityResult<()> {
        let mut held = entry.ticket.lock().await;
        if held.is_some() {
            // No unique ticket/outcome leaves the fence until the bounded owner guarantees space.
            let admission = self.storage.admission().await.map_err(store_error)?;
            let outcome = entry
                .outcome
                .lock()
                .expect("boundary outcome mutex")
                .take()
                .ok_or(ErrorCode::NotReady)?;
            let ticket = held.take().expect("held dispatch ticket");
            let completed = entry.clone();
            admission
                .call(move |s| {
                    match outcome {
                        BoundaryOutcome::Creation(outcome) => {
                            s.settle_creation_stage(ticket, *outcome)?
                        }
                        BoundaryOutcome::Receipt(outcome) => match &completed.creation {
                            Some(creation) => {
                                let creation = creation.lock();
                                let EnteredFacts::Agent((resources,metadata))=&*creation else {
                                    match &*creation {
                                        EnteredFacts::Shell {resources,..}=>s.settle_shell_dispatch(ticket,outcome,resources.clone())?,
                                        EnteredFacts::AgentShell {resources,acknowledge,..}=>s.settle_agent_terminal_dispatch(ticket,outcome,resources.clone(),|commit|acknowledge(&mut ||{if completed.invalidated.load(Ordering::Acquire){return Err(ErrorCode::StaleSession)}commit()}))?,
                                        EnteredFacts::ShellInput {progress,..}=>s.settle_shell_input_dispatch(ticket,outcome,progress.clone())?,
                                        EnteredFacts::Attachment {target,progress}=>s.settle_attachment_dispatch(ticket,outcome,ActionResultAttachment {target:target.clone(),progress:progress.clone()})?,
                                        EnteredFacts::WorktreeRemoval {target,candidate,preview,progress,..}=>s.settle_worktree_removal_dispatch(ticket,outcome,ActionResultWorktreeRemoval {target:target.clone(),candidate_id:candidate.clone(),preview_id:preview.clone(),progress:progress.clone()})?,
                                        EnteredFacts::AgentAlias {result,..}=>s.settle_agent_alias_dispatch(ticket,outcome,result.clone())?,
                                        EnteredFacts::Agent(_)=>return Err(StorageError::Corrupt),
                                    }
                                    completed.done.store(true,Ordering::Release);
                                    completed.connection.lock().expect("boundary descriptor mutex").take();
                                    return Ok(());
                                };
                                if outcome.state == ReceiptState::Failed {
                                    let definitive = completed
                                        .admission_guard
                                        .lock()
                                        .expect("creation admission guard mutex")
                                        .map(CreationAdmissionGuard::proof)
                                        .unwrap_or_else(|| {
                                            Id("native stage admission denied before start".into())
                                        });
                                    let stage = CreationOutcome {
                                        resources: resources.clone(),
                                        metadata: metadata.clone(),
                                        state: StageState::Failed,
                                        definitive: Some(definitive),
                                        at: outcome.at,
                                    };
                                    s.settle_creation_failure(ticket, stage, outcome)?;
                                } else {
                                    s.settle_creation_lost(
                                        ticket,
                                        resources.clone(),metadata.clone(),
                                        outcome.at,
                                    )?;
                                }
                            }
                            None => s.settle_dispatch(ticket, outcome)?,
                        },
                    }
                    completed.done.store(true, Ordering::Release);
                    completed
                        .connection
                        .lock()
                        .expect("boundary descriptor mutex")
                        .take();
                    Ok(())
                })
                .await
                .map_err(store_error)?;
        } else if !entry.done.load(Ordering::Acquire) {
            // A cancelled reply waiter may have transferred the claim to a still-queued job.
            self.storage
                .admission()
                .await
                .map_err(store_error)?
                .call(|_| Ok(()))
                .await
                .map_err(store_error)?;
            if !entry.done.load(Ordering::Acquire) {
                return Err(ErrorCode::NotReady);
            }
        }
        Ok(())
    }
    /// Stops admission before waiting for any gate, commits removal, then only waits bounded
    /// entered writes. Pending output/read state is removed; started desktop jobs survive.
    pub async fn revoke(&self, peer: Uuid, self_connection: Option<Uuid>) -> SecurityResult<bool> {
        let fence = self.peer(&peer);
        fence.stopped.store(true, Ordering::Release);
        if let Some(push)=self.push.get(){push.cancel(&peer);}
        let _guard = fence.gate.clone().lock_owned().await;
        let id = peer.clone();
        let changed = self
            .storage
            .admission()
            .await
            .map_err(store_error)?
            .call(move |s| s.revoke_epoch(&id, now_ms()))
            .await
            .map_err(store_error)?;
        if self_connection.is_none() {
            self.state.revoke(&peer);
        }
        if let Some((id, c)) = fence.active.lock().expect("active mutex").as_ref()
            && self_connection.as_ref() != Some(id)
        {
            c.close(0u32.into(), b"revoked");
        }
        drop(_guard);
        if let Some(push)=self.push.get(){push.drain_peer(&peer).await?;}
        let entries = fence.entered.lock().expect("entered mutex").clone();
        let deadline = Instant::now() + BOUNDARY_TIMEOUT;
        for entry in entries {
            if !entry.started.load(Ordering::Acquire) {
                if !entry.done.load(Ordering::Acquire) {
                    self.settle(&entry, no_effect(ErrorCode::Unauthorized))
                        .await?;
                }
                continue;
            }
            if entry.synchronous {
                if !entry.done.load(Ordering::Acquire) {
                    self.finish(&entry).await?;
                }
                continue;
            }
            while !entry.done.load(Ordering::Acquire) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            if !entry.done.load(Ordering::Acquire) {
                entry.invalidate()?;
                self.settle(&entry, lost()).await?;
            }
        }
        // Keep revoked entries out of the live map: durable epoch checks still reject old proofs.
        self.peers.lock().expect("peers mutex").remove(&peer);
        Ok(changed)
    }
    /// Foreground serve shutdown closes only transport/provider boundary handles, not desktop
    /// processes or pairing records. An interrupted entered effect is durably uncertain.
    pub async fn shutdown(&self) -> SecurityResult<()> {
        let peers = self
            .peers
            .lock()
            .expect("peers mutex")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for fence in &peers {
            fence.stopped.store(true, Ordering::Release);
        }
        // Drain prior SQLite intent jobs before observing their ownership records.
        self.storage
            .admission()
            .await
            .map_err(store_error)?
            .call(|_| Ok(()))
            .await
            .map_err(store_error)?;
        for fence in peers {
            if let Some((id, connection)) = fence.active.lock().expect("active mutex").take() {
                connection.close(0u32.into(), b"shutdown");
                self.state.disconnect(&id);
            }
            let entries = fence.entered.lock().expect("entered mutex").clone();
            for entry in entries {
                if !entry.done.load(Ordering::Acquire) {
                    if !entry.started.load(Ordering::Acquire) {
                        self.settle(&entry, no_effect(ErrorCode::NotReady)).await?;
                    } else if entry.synchronous {
                        self.finish(&entry).await?;
                    } else {
                        entry.invalidate()?;
                        self.settle(&entry, lost()).await?;
                    }
                }
            }
        }
        Ok(())
    }
    pub async fn devices(&self) -> SecurityResult<Vec<PairedDevice>> {
        self.storage
            .call(|s| s.devices())
            .await
            .map_err(store_error)
    }
}
fn lost() -> ReceiptTransition {
    ReceiptTransition {
        state: ReceiptState::Uncertain,
        proof: TransitionProof::LostOutcome,
        result: None,
        error: None,
        at: now_ms(),
    }
}
fn no_effect(code: ErrorCode) -> ReceiptTransition {
    no_effect_admission(code.into())
}
fn no_effect_admission(failure: CreationAdmissionFailure) -> ReceiptTransition {
    ReceiptTransition {
        state: ReceiptState::Failed,
        proof: TransitionProof::DefinitiveNoEffect(failure.proof()),
        result: None,
        error: Some(WireError {
            code: failure.code,
            message: SafeMessage("The native boundary was not entered".into()),
        }),
        at: now_ms(),
    }
}
fn stream_socket(fd: &OwnedFd) -> SecurityResult<()> {
    let mut kind = 0i32;
    let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
    // SAFETY: valid owned FD and writable correctly-sized SO_TYPE result buffer.
    if unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut i32).cast(),
            &mut length,
        )
    } != 0
        || kind != libc::SOCK_STREAM
    {
        Err(ErrorCode::NotReady)
    } else {
        Ok(())
    }
}

/// Endpoint bucket uses elapsed monotonic time and a fixed burst; no timer task per connection.
pub struct RequestRate {
    tokens: f64,
    at: Instant,
}
impl Default for RequestRate {
    fn default() -> Self {
        Self {
            tokens: 120.,
            at: Instant::now(),
        }
    }
}
impl RequestRate {
    pub fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + (now - self.at).as_secs_f64() * 60.).min(120.);
        self.at = now;
        if self.tokens < 1. {
            false
        } else {
            self.tokens -= 1.;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn connected() -> (
        iroh::Endpoint,
        iroh::Endpoint,
        iroh::endpoint::Connection,
        Arc<Security>,
    ) {
        let host = crate::endpoint_builder(iroh::SecretKey::from_bytes(&[41; 32]))
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        let phone = crate::endpoint_builder(iroh::SecretKey::from_bytes(&[42; 32]))
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        let endpoint = host.clone();
        let accept = tokio::spawn(async move { endpoint.accept().await.unwrap().await.unwrap() });
        let _client = phone.connect(host.addr(), crate::ALPN).await.unwrap();
        let connection = accept.await.unwrap();
        let storage = Storage::memory([3; 32], now_ms()).unwrap();
        let boot = storage.boot_id.clone();
        let security = Security::new(
            StorageOwner::new(storage).unwrap(),
            Arc::new(State::default()),
            EndpointId(host.id().to_string()),
            boot,
            vec![],
        );
        (host, phone, connection, security)
    }
    #[tokio::test]
    async fn monotonic_expiry_replacement_and_restart_do_not_retrust_token() {
        let (host, phone, connection, security) = connected().await;
        let old = PairInvite::parse(&security.open_pair().await.unwrap().uri).unwrap();
        let current = PairInvite::parse(&security.open_pair().await.unwrap().uri).unwrap();
        assert_eq!(
            security
                .confirm(
                    &connection,
                    PairConfirmParams {
                        token: old.token,
                        device_name: Label("phone".into())
                    }
                )
                .await
                .unwrap_err(),
            ErrorCode::PairingFailed
        );
        security.window.lock().await.as_mut().unwrap().deadline =
            Instant::now() - Duration::from_secs(1);
        assert_eq!(
            security
                .confirm(
                    &connection,
                    PairConfirmParams {
                        token: current.token,
                        device_name: Label("phone".into())
                    }
                )
                .await
                .unwrap_err(),
            ErrorCode::PairingFailed
        );
        assert!(security.devices().await.unwrap().is_empty());
        crate::close_endpoint(&host).await.unwrap();
        crate::close_endpoint(&phone).await.unwrap();
    }
    #[test]
    fn churn_and_burst_limits_are_bounded_and_expire() {
        let mut failures = Failures::default();
        for seed in 0..=255 {
            let peer = EndpointId(
                iroh::SecretKey::from_bytes(&[seed; 32])
                    .public()
                    .to_string(),
            );
            if let Some(attempt) = failures.reserve(&peer) {
                failures.finish(&peer, attempt, false);
            }
        }
        assert_eq!(failures.global.len(), 30);
        assert!(failures.peers.len() <= 256);
        let peer = EndpointId(iroh::SecretKey::from_bytes(&[7; 32]).public().to_string());
        assert!(failures.reserve(&peer).is_none());
        failures.prune(Instant::now() + Duration::from_secs(61));
        assert!(failures.global.is_empty());
        assert!(failures.peers.is_empty());
        let mut rate = RequestRate::default();
        for _ in 0..120 {
            assert!(rate.take());
        }
        assert!(!rate.take());
        rate.at -= Duration::from_secs(2);
        assert!(rate.take());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoke_during_post_intent_validation_rejects_unstarted_owned_socket_without_wait() {
        let (host, phone, connection, security) = connected().await;
        let invite = PairInvite::parse(&security.open_pair().await.unwrap().uri).unwrap();
        security
            .confirm(
                &connection,
                PairConfirmParams {
                    token: invite.token,
                    device_name: Label("phone".into()),
                },
            )
            .await
            .unwrap();
        let auth = Arc::new(
            security
                .hello(
                    EndpointId(connection.remote_id().to_string()),
                    HelloParams {
                        app_version: "test".into(),
                        device_name: Label("phone".into()),
                    },
                    connection,
                )
                .await
                .unwrap(),
        );
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::PromptSubmit(PromptSubmitParams {
                boot_id: security.boot.clone(),
                target: Target {
                    session_id: Uuid::random(),
                    generation: Uuid::random(),
                },
                text: PromptText("owned boundary".into()),
            }),
        };
        security
            .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
            .await
            .unwrap();
        let (native, other) = std::os::unix::net::UnixStream::pair().unwrap();
        native.set_nonblocking(true).unwrap();
        other.set_nonblocking(true).unwrap();
        let descriptor = native.try_clone().unwrap();
        let mut native = tokio::net::UnixStream::from_std(native).unwrap();
        let mut other = tokio::net::UnixStream::from_std(other).unwrap();
        let (paused, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let s = security.clone();
        let a = auth.clone();
        let boundary = tokio::spawn(async move {
            s.boundary(
                &a,
                request.id,
                move || {
                    paused.send(()).unwrap();
                    resume.recv().unwrap();
                    Ok(())
                },
                descriptor.into(),
                async move {
                    native.write_all(b"must not be written").await.unwrap();
                    lost()
                },
            )
            .await
        });
        ready.await.unwrap();
        let s = security.clone();
        let peer = auth.proof.peer_id().clone();
        let revoke = tokio::spawn(async move { s.revoke(peer, None).await });
        while !auth.fence.stopped.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        boundary.abort();
        release.send(()).unwrap();
        let _ = boundary.await;
        assert!(
            tokio::time::timeout(Duration::from_secs(2), revoke)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        );
        let mut byte = [0u8; 1];
        assert_eq!(other.read(&mut byte).await.unwrap(), 0);
        crate::close_endpoint(&host).await.unwrap();
        crate::close_endpoint(&phone).await.unwrap();
    }

    // The owner is genuinely blocked, not replaced by a fake SQLite queue. Polling call once
    // enqueues its closure; dropping only its reply waiter leaves the owner job intact.
    struct ReviewQueue(Option<std::sync::mpsc::Sender<()>>);
    impl ReviewQueue {
        fn block(owner: &StorageOwner, queued: usize) -> Self {
            let (release, resume) = std::sync::mpsc::channel();
            let (entered, ready) = std::sync::mpsc::channel();
            let mut job = Box::pin(owner.call(move |_| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                Ok(())
            }));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(job.as_mut().poll(&mut context).is_pending());
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            for _ in 0..queued {
                let mut job = Box::pin(owner.call(|_| Ok(())));
                assert!(job.as_mut().poll(&mut context).is_pending());
            }
            Self(Some(release))
        }
        fn release(&mut self) {
            if let Some(release) = self.0.take() {
                release.send(()).unwrap();
            }
        }
    }
    impl Drop for ReviewQueue {
        fn drop(&mut self) {
            self.release();
        }
    }
    async fn review_authorized() -> (
        iroh::Endpoint,
        iroh::Endpoint,
        Arc<Security>,
        Arc<Authorized>,
        PathBuf,
    ) {
        let (host, phone, connection, unused) = connected().await;
        drop(unused);
        let root = std::env::temp_dir().join(format!("goatr-review-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let mut storage =
            Storage::open(&root.join("state.sqlite3"), [9; 32], now_ms(), uid()).unwrap();
        storage
            .pair_device(&PairedDevice {
                device_id: Uuid::random(),
                endpoint_id: EndpointId(connection.remote_id().to_string()),
                device_name: Label("review fixture".into()),
                paired_at: Uint(now_ms()),
                last_seen_at: None,
            })
            .unwrap();
        let boot = storage.boot_id.clone();
        let security = Security::new(
            StorageOwner::new(storage).unwrap(),
            Arc::new(State::default()),
            EndpointId(host.id().to_string()),
            boot,
            vec![],
        );
        let auth = Arc::new(
            security
                .hello(
                    EndpointId(connection.remote_id().to_string()),
                    HelloParams {
                        app_version: "review".into(),
                        device_name: Label("phone".into()),
                    },
                    connection,
                )
                .await
                .unwrap(),
        );
        (host, phone, security, auth, root)
    }
    async fn review_close(
        host: iroh::Endpoint,
        phone: iroh::Endpoint,
        security: Arc<Security>,
        auth: Arc<Authorized>,
        root: PathBuf,
    ) {
        crate::close_endpoint(&host).await.unwrap();
        crate::close_endpoint(&phone).await.unwrap();
        drop(auth);
        let security = Arc::into_inner(security).expect("fixture owns security");
        Arc::into_inner(security.storage)
            .unwrap_or_else(|| panic!("fixture owns SQLite"))
            .close()
            .await
            .unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    fn review_request(boot: Uuid, creation: bool) -> Request {
        Request {
            id: Uuid::random(),
            body: if creation {
                RequestBody::SessionCreate(SessionCreateParams { source_target: None, checkout: None,
                    boot_id: boot,
                    server_id: Uuid::random(),
                    project_id: Uuid::random(),
                    provider: Provider::Codex,
                    name: Label("review fixture".into()),
                })
            } else {
                RequestBody::PromptSubmit(PromptSubmitParams {
                    boot_id: boot,
                    target: Target {
                        session_id: Uuid::random(),
                        generation: Uuid::random(),
                    },
                    text: PromptText("review owned socket".into()),
                })
            },
        }
    }
    fn review_resources() -> CreationResources {
        CreationResources { checkout: None,
            workspace_id: Some(Id("owned-workspace".into())),
            pane_id: Some(Id("owned-pane".into())),
            terminal_id: Some(Id("owned-terminal".into())),
            native_id: None,
            binding_id: None,
            session_id: None,
            generation: None,
            owner_started: None,
            relay_started: None,
        }
    }
    #[tokio::test]
    async fn interrupted_created_thread_checkpoint_retains_id_without_read_replay_or_live_fd() {
        for interruption in 0..3 {
            let (host, phone, security, auth, root) = review_authorized().await;
            let request = review_request(security.boot.clone(), true);
            security
                .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
                .await
                .unwrap();
            let proof = auth.proof.clone();
            let action = request.id.clone();
            let probe_path = root.join("probe");
            security
                .storage
                .call(move |s| {
                    s.transition(
                        &proof,
                        &action,
                        ReceiptTransition {
                            state: ReceiptState::Dispatching,
                            proof: TransitionProof::Intent,
                            result: None,
                            error: None,
                            at: now_ms(),
                        },
                    )?;
                    let mut resources = review_resources();
                    let probe = StageMetadata {
                        workspace_operation: WorkspaceOperation::ShellProbe,
                        shell_probe: Some(ShellProbeMetadata {
                            attempt_id: Uuid::random(),
                            action_id: action.clone(),
                            workspace_id: resources.workspace_id.clone().unwrap(),
                            pane_id: resources.pane_id.clone().unwrap(),
                            terminal_id: resources.terminal_id.clone().unwrap(),
                            shell: crate::herdr::process_evidence(std::process::id()).unwrap(),
                            executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
                            result_path: probe_path.to_str().unwrap().into(),
                            result_device: 1,
                            result_inode: 1,
                            parent_device: 1,
                            parent_inode: 1,
                            focused_workspace_id: Id("desktop".into()),
                        }),
                        ..StageMetadata::default()
                    };
                    for (stage, metadata) in [
                        (CreationStage::Workspace, StageMetadata::default()),
                        (CreationStage::Workspace, probe),
                        (CreationStage::Owner, StageMetadata::default()),
                    ] {
                        s.creation_intent(&proof, &action, stage, metadata.clone(), now_ms())?;
                        if stage == CreationStage::Owner {
                            resources.owner_started = Some(false);
                        }
                        s.creation_result(
                            &proof,
                            &action,
                            CreationOutcome {
                                resources: resources.clone(),
                                metadata,
                                state: StageState::Confirmed,
                                definitive: Some(Id(
                                    "earlier confirmed creation prerequisite".into()
                                )),
                                at: now_ms(),
                            },
                        )?;
                    }
                    Ok(())
                })
                .await
                .unwrap();
            let (native, mut provider) = std::os::unix::net::UnixStream::pair().unwrap();
            let mut retained = native.try_clone().unwrap();
            let (checkpointed, ready) = tokio::sync::oneshot::channel();
            let (_thread_stop_sender, thread_stop) = tokio::sync::watch::channel(false);
            let (s, a, action) = (security.clone(), auth.clone(), request.id.clone());
            let boundary = tokio::spawn(async move {
                s.thread_creation_boundary(
                    &a,
                    action,
                    StageMetadata::default(),
                    Instant::now() + BOUNDARY_TIMEOUT,
                    thread_stop,
                    || Ok(()),
                    native.into(),
                    |checkpoint| async move {
                        let blocked = if interruption == 2 {
                            let queue = ReviewQueue::block(&checkpoint.security.storage, 0);
                            {
                                let mut acknowledgement = Box::pin(
                                    checkpoint.acknowledge(Id("acknowledged-original-T".into())),
                                );
                                let mut context =
                                    std::task::Context::from_waker(std::task::Waker::noop());
                                assert!(acknowledgement.as_mut().poll(&mut context).is_pending());
                            }
                            Some(queue)
                        } else {
                            checkpoint
                                .acknowledge(Id("acknowledged-original-T".into()))
                                .await
                                .unwrap();
                            assert_eq!(
                                checkpoint.acknowledge(Id("substitute-T".into())).await,
                                Err(ErrorCode::InvalidRequest),
                            );
                            None
                        };
                        checkpointed
                            .send((checkpoint.entry.clone(), blocked))
                            .unwrap_or_else(|_| panic!("checkpoint receiver"));
                        // Interruption between the acknowledged start and materializing read.
                        std::future::pending::<CreationOutcome>().await
                    },
                )
                .await
            });
            let (entry, mut blocked) = ready.await.unwrap();
            assert_eq!(
                entry.creation.as_ref().unwrap().lock().agent().unwrap().0.native_id,
                Some(Id("acknowledged-original-T".into())),
            );
            if interruption != 2 {
                let existing = security
                    .reserve(
                        &auth,
                        request.clone(),
                        |_| panic!("existing ID must not validate"),
                        |_| panic!("existing ID must not register another original job"),
                    )
                    .await
                    .unwrap();
                let Reservation::Existing(receipt) = existing else {
                    panic!("durable original receipt")
                };
                let progress = receipt.creation.unwrap();
                assert_eq!(receipt.state, ReceiptState::Dispatching);
                assert_eq!(
                    (progress.stage, progress.stage_state),
                    (CreationStage::Thread, StageState::Dispatching)
                );
                assert_eq!(
                    progress.resources.native_id,
                    Some(Id("acknowledged-original-T".into()))
                );
                assert!(entry.ticket.lock().await.is_some());
                let proof = auth.proof.clone();
                let action = request.id.clone();
                security
                    .storage
                    .call(move |s| {
                        for stage in [CreationStage::Thread, CreationStage::Relay] {
                            assert!(matches!(
                                s.creation_intent(
                                    &proof,
                                    &action,
                                    stage,
                                    StageMetadata::default(),
                                    now_ms()
                                ),
                                Err(StorageError::Policy(ErrorCode::InvalidRequest)),
                            ));
                        }
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
            boundary.abort();
            assert!(boundary.await.unwrap_err().is_cancelled());
            if let Some(queue) = blocked.as_mut() {
                queue.release();
            }
            if interruption == 1 {
                security
                    .revoke(auth.proof.peer_id().clone(), None)
                    .await
                    .unwrap();
            } else {
                security.shutdown().await.unwrap();
            }
            assert!(entry.done.load(Ordering::Acquire));
            assert!(entry.ticket.lock().await.is_none());
            assert!(retained.write_all(b"late thread/read").is_err());
            assert_eq!(provider.read(&mut [0; 1]).unwrap(), 0);
            let stale = CreationCheckpoint {
                security: &security,
                auth: &auth,
                entry,
            };
            assert!(matches!(stale.guard().await, Err(ErrorCode::Unauthorized)));
            let db = rusqlite::Connection::open(root.join("state.sqlite3")).unwrap();
            let data: String = db
                .query_row(
                    "SELECT receipt FROM receipts WHERE peer=?1 AND action=?2",
                    rusqlite::params![auth.proof.peer_id().0, request.id.0],
                    |row| row.get(0),
                )
                .unwrap();
            let receipt: Receipt = serde_json::from_str(&data).unwrap();
            let progress = receipt.creation.unwrap();
            assert_eq!(receipt.state, ReceiptState::Uncertain);
            assert_eq!(
                (progress.stage, progress.stage_state),
                (CreationStage::Thread, StageState::Uncertain)
            );
            assert_eq!(
                progress.resources.native_id,
                Some(Id("acknowledged-original-T".into()))
            );
            assert!(
                progress.resources.binding_id.is_none() && progress.resources.session_id.is_none()
            );
            drop(db);
            drop(stale);
            review_close(host, phone, security, auth, root).await;
        }
    }
    async fn review_queue_full(variant: u8, cancel: bool) {
        let (host, phone, security, auth, root) = review_authorized().await;
        let request = review_request(security.boot.clone(), variant != 0);
        security
            .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
            .await
            .unwrap();
        let (native, other) = std::os::unix::net::UnixStream::pair().unwrap();
        native.set_nonblocking(true).unwrap();
        other.set_nonblocking(true).unwrap();
        let descriptor = native.try_clone().unwrap();
        let retained = native.try_clone().unwrap();
        let mut native = tokio::net::UnixStream::from_std(native).unwrap();
        let mut other = tokio::net::UnixStream::from_std(other).unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let s = security.clone();
        let a = auth.clone();
        let action = request.id.clone();
        let boundary_root = root.clone();
        let boundary = tokio::spawn(async move {
            if variant == 2 {
                s.start_boundary(
                    &a,
                    action,
                    (CreationStage::Workspace, StageMetadata::default()),
                    || Ok(()),
                    || {
                        // An actual OS resource admission, not a provider echo/start mock.
                        let marker = boundary_root.join("started-resource");
                        let file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(marker)
                            .unwrap();
                        file.sync_all().unwrap();
                        entered
                            .send(ReviewQueue::block(&s.storage, 64))
                            .unwrap_or_else(|_| panic!("fixture receiver"));
                        Ok((
                            (),
                            CreationOutcome {
                                resources: review_resources(),
                                metadata: StageMetadata::default(),
                                state: StageState::Confirmed,
                                definitive: Some(Id("owned file created once".into())),
                                at: now_ms(),
                            },
                        ))
                    },
                )
                .await
            } else {
                let effect = async {
                    native.write_all(b"one write").await.unwrap();
                    entered
                        .send(ReviewQueue::block(&s.storage, 64))
                        .unwrap_or_else(|_| panic!("fixture receiver"));
                    ReceiptTransition {
                        state: ReceiptState::Dispatched,
                        proof: TransitionProof::WriteCompleted,
                        result: None,
                        error: None,
                        at: now_ms(),
                    }
                };
                if variant == 0 {
                    s.boundary(&a, action, || Ok(()), descriptor.into(), effect)
                        .await
                } else {
                    s.creation_boundary(
                        &a,
                        action,
                        (CreationStage::Workspace, StageMetadata::default()),
                        || Ok(()),
                        descriptor.into(),
                        async {
                            let completed = effect.await;
                            CreationOutcome {
                                resources: review_resources(),
                                metadata: StageMetadata::default(),
                                state: StageState::Confirmed,
                                definitive: Some(Id("owned Unix write completed".into())),
                                at: completed.at,
                            }
                        },
                    )
                    .await
                }
            }
        });
        let mut queue = ready.await.unwrap();
        let entry = auth
            .fence
            .entered
            .lock()
            .expect("entered mutex")
            .last()
            .unwrap()
            .clone();
        // Settlement retains both the unique ticket and the actual completed outcome while full.
        let ticket_retained = match entry.ticket.try_lock() {
            Ok(ticket) => ticket.is_some() || entry.done.load(Ordering::Acquire),
            Err(_) => true,
        };
        if cancel {
            while entry
                .outcome
                .lock()
                .expect("boundary outcome mutex")
                .is_none()
            {
                tokio::task::yield_now().await;
            }
            boundary.abort();
        }
        queue.release();
        // Capacity comes back asynchronously as the blocked owner wakes. These are read-only
        // barriers, never a native retry.
        loop {
            match security.storage.call(|_| Ok(())).await {
                Ok(()) => break,
                Err(StorageError::Policy(ErrorCode::Busy)) => tokio::task::yield_now().await,
                Err(error) => panic!("{error}"),
            }
        }
        let boundary_result = boundary.await;
        let shutdown_result = security.shutdown().await;
        let done = entry.done.load(Ordering::Acquire);
        let proof = auth.proof.clone();
        let action = request.id.clone();
        let receipt = security
            .storage
            .call(move |s| s.status(&proof, &action, now_ms()))
            .await
            .unwrap()
            .unwrap();
        let revoke_result = security.revoke(auth.proof.peer_id().clone(), None).await;
        let _ = entry.invalidate();
        let late_descriptor = retained.try_clone().unwrap();
        retained.set_nonblocking(true).unwrap();
        let mut retained = tokio::net::UnixStream::from_std(retained).unwrap();
        let denied = security
            .boundary(
                &auth,
                request.id,
                || Ok(()),
                late_descriptor.into(),
                async move {
                    retained.write_all(b"must not retry").await.unwrap();
                    lost()
                },
            )
            .await;
        if variant != 2 {
            let mut written = [0; 9];
            other.read_exact(&mut written).await.unwrap();
            assert_eq!(&written, b"one write");
            assert_eq!(other.read(&mut [0; 1]).await.unwrap(), 0);
        }
        if variant == 2 {
            assert!(root.join("started-resource").is_file());
        }
        drop(entry);
        review_close(host, phone, security, auth, root).await;
        assert!(
            ticket_retained,
            "bounded queue rejection must not consume the sole dispatch ticket"
        );
        if cancel {
            assert!(
                boundary_result
                    .as_ref()
                    .is_err_and(|error| error.is_cancelled())
            );
        } else {
            assert!(
                boundary_result.unwrap().is_ok(),
                "completed native outcome must survive transient SQL saturation"
            );
        }
        assert!(
            shutdown_result.is_ok(),
            "settlement must recover after SQLite capacity returns"
        );
        assert!(done, "the entered boundary must become done");
        if variant == 0 {
            assert_eq!(receipt.state, ReceiptState::Dispatched);
        }
        if variant != 0 {
            let progress = receipt.creation.unwrap();
            assert_eq!(progress.stage_state, StageState::Confirmed);
            assert_eq!(progress.resources, review_resources());
        }
        assert!(
            revoke_result.is_ok(),
            "revoke must complete after settlement"
        );
        assert_eq!(
            denied,
            Err(ErrorCode::Unauthorized),
            "revocation must prevent any later native bytes"
        );
    }
    #[tokio::test]
    async fn review_queue_full_retains_owned_unix_dispatch_ticket() {
        review_queue_full(0, false).await
    }
    #[tokio::test]
    async fn review_queue_full_retains_creation_socket_ticket() {
        review_queue_full(1, false).await
    }
    #[tokio::test]
    async fn review_queue_full_retains_synchronous_start_ticket_and_outcome() {
        review_queue_full(2, false).await
    }
    #[tokio::test]
    async fn review_cancelled_queue_wait_keeps_completed_native_outcomes_for_shutdown() {
        for variant in 0..=2 {
            review_queue_full(variant, true).await;
        }
    }

    async fn review_no_effect(variant: u8, code: ErrorCode) {
        let (host, phone, security, auth, root) = review_authorized().await;
        let request = review_request(security.boot.clone(), true);
        security
            .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
            .await
            .unwrap();
        let (native, other) = std::os::unix::net::UnixStream::pair().unwrap();
        native.set_nonblocking(true).unwrap();
        other.set_nonblocking(true).unwrap();
        let descriptor = native.try_clone().unwrap();
        let mut native = tokio::net::UnixStream::from_std(native).unwrap();
        let mut other = tokio::net::UnixStream::from_std(other).unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = effects.clone();
        let s = security.clone();
        let a = auth.clone();
        let action = request.id.clone();
        let boundary = tokio::spawn(async move {
            let deny = || {
                entered
                    .send((ReviewQueue::block(&s.storage, 0), now_ms()))
                    .unwrap_or_else(|_| panic!("fixture receiver"));
                Err(code)
            };
            if variant == 0 {
                s.creation_boundary(
                    &a,
                    action,
                    (CreationStage::Workspace, StageMetadata::default()),
                    deny,
                    descriptor.into(),
                    async {
                        counted.fetch_add(1, Ordering::SeqCst);
                        native.write_all(b"forbidden").await.unwrap();
                        CreationOutcome {
                            resources: review_resources(),
                            metadata: StageMetadata::default(),
                            state: StageState::Confirmed,
                            definitive: Some(Id("forbidden".into())),
                            at: now_ms(),
                        }
                    },
                )
                .await
            } else {
                s.start_boundary(
                    &a,
                    action,
                    (CreationStage::Workspace, StageMetadata::default()),
                    deny,
                    || {
                        counted.fetch_add(1, Ordering::SeqCst);
                        Ok((
                            (),
                            CreationOutcome {
                                resources: review_resources(),
                                metadata: StageMetadata::default(),
                                state: StageState::Confirmed,
                                definitive: Some(Id("forbidden".into())),
                                at: now_ms(),
                            },
                        ))
                    },
                )
                .await
            }
        });
        let (mut queue, captured) = ready.await.unwrap();
        let entry = auth
            .fence
            .entered
            .lock()
            .expect("entered mutex")
            .last()
            .unwrap()
            .clone();
        // The ticket lock proves settle has captured no_effect and submitted its SQLite job.
        while entry.ticket.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
        let captured_after_enqueue = now_ms().max(captured);
        while now_ms() <= captured_after_enqueue {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        queue.release();
        let result = boundary.await.unwrap();
        let proof = auth.proof.clone();
        let action = request.id.clone();
        let (receipt, local) = security
            .storage
            .call(move |s| {
                Ok((
                    s.status(&proof, &action, now_ms())?.unwrap(),
                    s.creation_local(&proof, &action)?,
                ))
            })
            .await
            .unwrap();
        let done = entry.done.load(Ordering::Acquire);
        let shutdown = security.shutdown().await;
        let _ = entry.invalidate();
        assert_eq!(other.read(&mut [0; 1]).await.unwrap(), 0);
        drop(entry);
        review_close(host, phone, security, auth, root).await;
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(
            result,
            Err(code),
            "the original validator failure must survive queued settlement"
        );
        assert_eq!(receipt.creation.unwrap().stage_state, StageState::Failed);
        let stage = local.unwrap().stages.last().unwrap().clone();
        assert_eq!(stage.state, StageState::Failed);
        assert_eq!(
            stage.proof,
            Some(Id("native stage admission denied before start".into()))
        );
        assert_eq!(
            receipt.state,
            ReceiptState::Failed,
            "stage and receipt must both commit no-effect"
        );
        assert_eq!(receipt.error.unwrap().code, code);
        assert!(done, "definitive no-effect must finish the entry");
        assert!(shutdown.is_ok());
    }
    #[tokio::test]
    async fn review_creation_validator_failure_keeps_stage_and_receipt_timestamp_consistent() {
        review_no_effect(0, ErrorCode::NotReady).await
    }
    #[tokio::test]
    async fn review_creation_unauthorized_keeps_stage_and_receipt_timestamp_consistent() {
        review_no_effect(0, ErrorCode::Unauthorized).await
    }
    async fn review_guarded_creation_admission_failure(guard: CreationAdmissionGuard) {
        let (host, phone, security, auth, root) = review_authorized().await;
        let request = review_request(security.boot.clone(), true);
        let reserved = security
            .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
            .await
            .unwrap();
        let Reservation::Reserved(reserved) = reserved else {
            panic!("fresh review creation must reserve");
        };
        let resources = reserved.creation.unwrap().resources;
        let (native, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let descriptor = native.try_clone().unwrap();
        let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = effects.clone();
        let result = security
            .creation_boundary_guarded(
                &auth,
                request.id.clone(),
                (CreationStage::Workspace, StageMetadata::default()),
                || {
                    Err(CreationAdmissionFailure::guarded(
                        ErrorCode::NotReady,
                        guard,
                    ))
                },
                descriptor.into(),
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    CreationOutcome {
                        resources: review_resources(),
                        metadata: StageMetadata::default(),
                        state: StageState::Confirmed,
                        definitive: Some(Id("forbidden".into())),
                        at: now_ms(),
                    }
                },
            )
            .await;
        let proof = auth.proof.clone();
        let action = request.id.clone();
        let (receipt, local) = security
            .storage
            .call(move |s| {
                Ok((
                    s.status(&proof, &action, now_ms())?.unwrap(),
                    s.creation_local(&proof, &action)?,
                ))
            })
            .await
            .unwrap();
        let shutdown = security.shutdown().await;
        review_close(host, phone, security, auth, root).await;
        let progress = receipt.creation.unwrap();
        let stage = local.unwrap().stages.last().unwrap().clone();
        assert_eq!(result, Err(ErrorCode::NotReady));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(receipt.state, ReceiptState::Failed);
        assert_eq!(receipt.error.unwrap().code, ErrorCode::NotReady);
        assert_eq!(progress.stage, CreationStage::Workspace);
        assert_eq!(progress.stage_state, StageState::Failed);
        assert_eq!(progress.resources, resources);
        assert_eq!(stage.state, StageState::Failed);
        assert_eq!(stage.proof, Some(guard.proof()));
        assert!(shutdown.is_ok());
    }
    #[tokio::test]
    async fn guarded_shell_probe_admission_failures_keep_exact_workspace_proof_and_no_effect() {
        for guard in [
            CreationAdmissionGuard::Root,
            CreationAdmissionGuard::ShellEpoch,
            CreationAdmissionGuard::ResultView,
        ] {
            review_guarded_creation_admission_failure(guard).await;
        }
    }
    #[tokio::test]
    async fn review_start_validator_failure_keeps_stage_and_receipt_timestamp_consistent() {
        review_no_effect(1, ErrorCode::NotReady).await
    }

    fn review_limiter() -> Arc<Security> {
        let storage = Storage::memory([3; 32], now_ms()).unwrap();
        let boot = storage.boot_id.clone();
        Security::new(
            StorageOwner::new(storage).unwrap(),
            Arc::new(State::default()),
            EndpointId(iroh::SecretKey::from_bytes(&[99; 32]).public().to_string()),
            boot,
            vec![],
        )
    }
    fn review_peer(seed: u16) -> EndpointId {
        let mut bytes = [0; 32];
        bytes[..2].copy_from_slice(&seed.to_be_bytes());
        EndpointId(iroh::SecretKey::from_bytes(&bytes).public().to_string())
    }
    async fn review_close_limiter(security: Arc<Security>) {
        let security = Arc::into_inner(security).expect("fixture owns limiter");
        Arc::into_inner(security.storage)
            .unwrap_or_else(|| panic!("fixture owns SQLite"))
            .close()
            .await
            .unwrap();
    }
    async fn review_concurrent_checks(security: &Arc<Security>, peers: Vec<EndpointId>) -> usize {
        let barrier = tokio::sync::Barrier::new(peers.len());
        futures_util::future::join_all(peers.iter().map(|peer| async {
            let _permit = security.admission().unwrap();
            let attempt = security.handshake(peer);
            let admitted = attempt.is_some();
            barrier.wait().await;
            if let Some(attempt) = attempt {
                attempt.succeeded();
            }
            admitted
        }))
        .await
        .into_iter()
        .filter(|admitted| *admitted)
        .count()
    }
    #[tokio::test(start_paused = true)]
    async fn review_endpoint_budget_counts_eight_pending_admissions_after_four_failures() {
        let security = review_limiter();
        let peer = review_peer(1);
        for _ in 0..4 {
            drop(security.handshake(&peer).unwrap());
        }
        // Each owns the real unauthenticated permit until all eight have checked admission,
        // just as TLS connections parked before their first frame do.
        let admitted = review_concurrent_checks(&security, vec![peer; 8]).await;
        review_close_limiter(security).await;
        assert_eq!(
            admitted, 1,
            "pending attempts must reserve the fifth endpoint allowance"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn review_global_budget_counts_pending_admissions_and_survives_endpoint_churn() {
        let security = review_limiter();
        for seed in 0..29 {
            drop(security.handshake(&review_peer(seed)).unwrap());
        }
        let admitted =
            review_concurrent_checks(&security, (100..108).map(review_peer).collect()).await;
        review_close_limiter(security).await;
        assert_eq!(
            admitted, 1,
            "29 completed failures plus pending attempts must stay at 30"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn review_pending_rate_reservations_expire_without_unbounded_endpoint_memory() {
        let security = review_limiter();
        let attempts = (0..300)
            .filter_map(|seed| security.handshake(&review_peer(seed)))
            .collect::<Vec<_>>();
        let admitted = attempts.len();
        let bounded = security
            .failures
            .lock()
            .expect("failures mutex")
            .peers
            .len()
            <= 256;
        tokio::time::advance(Duration::from_secs(61)).await;
        let after_expiry = {
            let attempt = security.handshake(&review_peer(301));
            let admitted = attempt.is_some();
            if let Some(attempt) = attempt {
                attempt.succeeded();
            }
            admitted
        };
        drop(attempts);
        review_close_limiter(security).await;
        assert!(bounded);
        assert!(
            after_expiry,
            "expired pending reservations must not poison future admissions"
        );
        assert_eq!(
            admitted, 30,
            "endpoint churn must not evade the global pending allowance"
        );
    }
    #[tokio::test]
    async fn review_successful_native_hello_releases_its_failure_budget_reservation() {
        let (host, phone, security, auth, root) = review_authorized().await;
        let peer = EndpointId(phone.id().to_string());
        for _ in 0..4 {
            drop(security.handshake(&peer).unwrap());
        }
        let (stop, rx) = tokio::sync::watch::channel(false);
        let server_endpoint = host.clone();
        let server_security = security.clone();
        let server = tokio::spawn(async move {
            crate::transport::serve(&server_endpoint, server_security, None, None, None, rx)
                .await
                .unwrap();
        });
        let connection = phone.connect(host.addr(), crate::ALPN).await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::Hello(HelloParams {
                app_version: "review".into(),
                device_name: Label("phone".into()),
            }),
        };
        crate::transport::write_frame(
            &mut send,
            &encode_payload(&Envelope::Request(request)).unwrap(),
        )
        .await
        .unwrap();
        let bytes = crate::transport::read_frame(&mut recv, FRAME_MAX)
            .await
            .unwrap();
        assert!(matches!(
            decode_payload(&bytes, Some(Method::Hello)).unwrap(),
            Envelope::Response(Response {
                outcome: Ok(ResultBody::Hello(_)),
                ..
            })
        ));
        let admitted = review_concurrent_checks(&security, vec![peer; 8]).await;
        connection.close(0u32.into(), b"fixture done");
        stop.send(true).unwrap();
        server.await.unwrap();
        security.shutdown().await.unwrap();
        review_close(host, phone, security, auth, root).await;
        assert_eq!(
            admitted, 1,
            "successful hello must release its reservation, not erase four failures"
        );
    }
}

/// Shell facts stay on the same entered ticket through cancellation and revoke.
/// This checkpoint is not cloneable and confers no additional native permission.
pub(crate) struct ShellCreationCheckpoint<'a> {security:&'a Security,auth:&'a Authorized,entry:Arc<Entered>}
impl ShellCreationCheckpoint<'_> {
    pub(crate) fn retain(&self,target:&ShellTarget,source:&Uuid,resources:&ShellCreationResources)->SecurityResult<()> {
        resources.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let mut held=self.entry.creation.as_ref().ok_or(ErrorCode::NotReady)?.lock();
        let EnteredFacts::Shell {target:original,source:epoch,resources:known}=&mut *held else {return Err(ErrorCode::InvalidRequest)};
        if original!=target || epoch!=source {return Err(ErrorCode::StaleSession)}
        crate::storage::retain_shell_facts(known,resources).map_err(store_error)?;
        // Synchronous before the next await. Even a later failed durable checkpoint
        // leaves these facts with the exact entered settlement owner.
        *known=resources.clone();Ok(())
    }
    pub(crate) async fn checkpoint(&self)->SecurityResult<()> {
        let _guard=self.security.native_guard(&self.auth).await?;
        let entry=self.entry.clone();
        self.security.storage.call(move|store| {
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::NotReady))?;
            let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) || !entry.started.load(Ordering::Acquire) {return Err(StorageError::Policy(ErrorCode::NotReady))}
            let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();
            let EnteredFacts::Shell {resources,..}=&*facts else {return Err(StorageError::Corrupt)};
            store.checkpoint_shell_facts(ticket,resources.clone(),now_ms())
        }).await.map_err(store_error)
    }
}

pub(crate) struct ShellInputCheckpoint<'a> {security:&'a Security,auth:&'a Authorized,entry:Arc<Entered>}
impl ShellInputCheckpoint<'_> {
    pub(crate) fn retain(&self,target:&ShellTarget,source:&Uuid,progress:&ShellInputProgress)->SecurityResult<()> {
        progress.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let mut held=self.entry.creation.as_ref().ok_or(ErrorCode::NotReady)?.lock();
        let EnteredFacts::ShellInput {target:original,source:epoch,progress:known}=&mut *held else {return Err(ErrorCode::InvalidRequest)};
        if original!=target || epoch!=source {return Err(ErrorCode::StaleSession)}
        crate::storage::retain_shell_input_facts(known,progress).map_err(store_error)?;
        // Synchronous before the next await. Even a later failed durable checkpoint
        // leaves these facts with the exact entered settlement owner.
        *known=progress.clone();Ok(())
    }
    pub(crate) async fn checkpoint(&self)->SecurityResult<()> {
        let _guard=self.security.native_guard(&self.auth).await?;
        let entry=self.entry.clone();
        self.security.storage.call(move|store| {
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::NotReady))?;
            let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) || !entry.started.load(Ordering::Acquire) {return Err(StorageError::Policy(ErrorCode::NotReady))}
            let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();
            let EnteredFacts::ShellInput {progress,..}=&*facts else {return Err(StorageError::Corrupt)};
            store.checkpoint_shell_input_facts(ticket,progress.clone(),now_ms())
        }).await.map_err(store_error)
    }
}

/// Alias effect facts remain owned by the original entered ticket, including revoke.
pub(crate) struct AgentAliasCheckpoint<'a> {security:&'a Security,auth:&'a Authorized,entry:Arc<Entered>}
impl AgentAliasCheckpoint<'_> {
    pub(crate) fn retain(&self,target:&Target,source:&Uuid,result:ActionResultAgentRenamed)->SecurityResult<()> {
        result.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let mut held=self.entry.creation.as_ref().ok_or(ErrorCode::NotReady)?.lock();
        let EnteredFacts::AgentAlias {target:original,source:epoch,result:known}=&mut *held else {return Err(ErrorCode::InvalidRequest)};
        if original!=target || epoch!=source || result.target!=*target || known.as_ref().is_some_and(|old|old!=&result) {return Err(ErrorCode::StaleSession)}
        *known=Some(result);Ok(())
    }
    pub(crate) async fn checkpoint(&self)->SecurityResult<()> {
        let _guard=self.security.native_guard(self.auth).await?;let entry=self.entry.clone();
        self.security.storage.call(move|store| {
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::NotReady))?;
            let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) || !entry.started.load(Ordering::Acquire) {return Err(StorageError::Policy(ErrorCode::NotReady))}
            let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();
            let EnteredFacts::AgentAlias {result:Some(result),..}=&*facts else {return Err(StorageError::Corrupt)};
            store.checkpoint_agent_alias_facts(ticket,result.clone(),now_ms())
        }).await.map_err(store_error)
    }
}

/// Correlated removal progress stays on the same non-clone entered ticket.
pub(crate) struct WorktreeRemovalCheckpoint<'a> {security:&'a Security,auth:&'a Authorized,entry:Arc<Entered>}
impl WorktreeRemovalCheckpoint<'_> {
    pub(crate) fn retain(&self,target:&Target,source:&Uuid,candidate:&Uuid,preview:&Uuid,progress:&WorktreeRemovalProgress)->SecurityResult<()> {
        progress.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let mut held=self.entry.creation.as_ref().ok_or(ErrorCode::NotReady)?.lock();
        let EnteredFacts::WorktreeRemoval {target:original,source:epoch,candidate:original_candidate,preview:original_preview,progress:known}=&mut *held else{return Err(ErrorCode::InvalidRequest)};
        if original!=target || epoch!=source || original_candidate!=candidate || original_preview!=preview || !progress.retains(known){return Err(ErrorCode::StaleSession)}
        *known=progress.clone();Ok(())
    }
    pub(crate) async fn checkpoint(&self)->SecurityResult<()> {
        let _guard=self.security.native_guard(self.auth).await?;let entry=self.entry.clone();
        self.security.storage.call(move|store| {
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::NotReady))?;let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) || !entry.started.load(Ordering::Acquire){return Err(StorageError::Policy(ErrorCode::NotReady))}
            let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();
            let EnteredFacts::WorktreeRemoval {target,candidate,preview,progress,..}=&*facts else{return Err(StorageError::Corrupt)};
            store.checkpoint_worktree_removal_facts(ticket,ActionResultWorktreeRemoval {target:target.clone(),candidate_id:candidate.clone(),preview_id:preview.clone(),progress:progress.clone()},now_ms())
        }).await.map_err(store_error)
    }
}

pub(crate) struct AttachmentCheckpoint<'a> {security:&'a Security,entry:Arc<Entered>}
impl Security {
    pub(crate) async fn attachment_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,target:Target,upload:Uuid,connection:OwnedFd,
        validate:impl FnOnce()->SecurityResult<()>,effect:F,
    )->SecurityResult<()> where F:FnOnce(AttachmentCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition>{
        let facts=AttachmentProgress {upload_id:upload,committed_bytes:Uint(0),expires_at:None,provider_path:None,published:None,discarded:None};
        let (entry,guard)=self.prepare_native_boundary(auth,action,connection,None,Some(EnteredFacts::Attachment {target,progress:facts}),|entry|entry).await?;
        let checkpoint=AttachmentCheckpoint {security:self,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||validate().map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
    pub(crate) async fn attachment_cleanup_boundary<'a,F,E>(
        &'a self,auth:&'a Authorized,action:Uuid,params:&AttachmentsDiscardParams,context:&crate::attachments::CleanupContext,effect:F,
    )->SecurityResult<()> where F:FnOnce(AttachmentCheckpoint<'a>)->E,E:Future<Output=ReceiptTransition>{
        if params.boot_id!=self.boot{return Err(ErrorCode::StaleHost)}context.validate(auth.proof.peer_id(),params)?;
        let (entry,guard)=self.prepare_entered_boundary(auth,action,EnteredConnection::AttachmentCleanup,None,Some(EnteredFacts::Attachment {target:params.target.clone(),progress:context.progress()}),|entry|entry).await?;
        let checkpoint=AttachmentCheckpoint {security:self,entry:entry.clone()};
        self.admit_native_boundary(auth,&entry,guard,None,||context.validate(auth.proof.peer_id(),params).map(|()|BoundaryReadiness::Ready),effect(checkpoint)).await.map(|_|())
    }
}
impl AttachmentCheckpoint<'_> {
    pub(crate) async fn execute(&self,peer:Uuid,action:Uuid,body:RequestBody,context:Option<crate::attachments::ProviderContext>)->ReceiptTransition {
        let entry=self.entry.clone();
        let outcome=self.security.storage.call(move|store|{
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::Busy))?;
            let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            let (target,initial)={let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();let EnteredFacts::Attachment {target,progress}=&*facts else{return Err(StorageError::Corrupt)};(target.clone(),progress.clone())};
            if entry.invalidated.load(Ordering::Acquire)||entry.done.load(Ordering::Acquire)||!entry.started.load(Ordering::Acquire){return Err(StorageError::Policy(ErrorCode::NotReady))}
            store.checkpoint_attachment_facts(ticket,ActionResultAttachment {target:target.clone(),progress:initial},now_ms())?;
            let mut retain=|progress:AttachmentProgress|->Result<(),StorageError>{
                progress.validate()?;let mut facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();
                let EnteredFacts::Attachment {progress:known,..}=&mut *facts else{return Err(StorageError::Corrupt)};
                if !progress.retains(known){return Err(StorageError::Corrupt)}*known=progress;Ok(())
            };
            let result=store.attachment_effect(&peer,&action,&body,context.as_ref(),&||!entry.invalidated.load(Ordering::Acquire)&&!entry.done.load(Ordering::Acquire),&mut retain);
            let progress={let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();let EnteredFacts::Attachment {progress,..}=&*facts else{return Err(StorageError::Corrupt)};progress.clone()};
            store.checkpoint_attachment_facts(ticket,ActionResultAttachment {target:target.clone(),progress:progress.clone()},now_ms())?;
            Ok((target,progress,result.is_ok()&&!entry.invalidated.load(Ordering::Acquire)))
        }).await;
        match outcome {
            Ok((target,progress,true))=>ReceiptTransition {state:ReceiptState::Succeeded,proof:TransitionProof::CorrelatedAcknowledgement(Id("private attachment effect committed".into())),result:Some(ActionResult::Attachment(ActionResultAttachment {target,progress})),error:None,at:now_ms()},
            _=>ReceiptTransition {state:ReceiptState::Uncertain,proof:TransitionProof::LostOutcome,result:None,error:None,at:now_ms()},
        }
    }
}

fn attachment_claimed(fence:&PeerFence,upload:&Uuid)->bool {
    fence.entered.lock().expect("entered mutex").iter().any(|entry|{
        !entry.done.load(Ordering::Acquire) && entry.creation.as_ref().is_some_and(|facts|matches!(&*facts.lock(),EnteredFacts::Attachment {progress,..} if &progress.upload_id==upload))
    })
}
impl Security {pub(crate) fn attachment_claim_busy(&self,auth:&Authorized,upload:&Uuid)->bool {attachment_claimed(&auth.fence,upload)}}
impl Security {
    pub(crate) async fn attachment_maintenance(&self,now:u64)->SecurityResult<()> {
        let peers=self.storage.call(|store|store.attachment_peers()).await.map_err(store_error)?;
        for peer in peers {
            let fence=self.peer(&peer);let _gate=fence.gate.clone().lock_owned().await;
            let busy:Vec<Uuid>=fence.entered.lock().expect("entered mutex").iter().filter(|entry|!entry.done.load(Ordering::Acquire)).filter_map(|entry|entry.creation.as_ref().and_then(|facts|match &*facts.lock(){EnteredFacts::Attachment {progress,..}=>Some(progress.upload_id.clone()),_=>None})).collect();
            self.storage.call(move|store|store.attachment_maintenance_peer(&peer,&busy,now)).await.map_err(store_error)?;
        }Ok(())
    }
}

pub(crate) struct AgentTerminalCheckpoint<'a> {security:&'a Security,auth:&'a Authorized,entry:Arc<Entered>}
impl AgentTerminalCheckpoint<'_> {
    pub(crate) fn retain(&self,target:&Target,source:&Uuid,resources:&ShellCreationResources)->SecurityResult<()> {
        resources.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let mut held=self.entry.creation.as_ref().ok_or(ErrorCode::NotReady)?.lock();
        let EnteredFacts::AgentShell {target:original,source:epoch,resources:known,..}=&mut *held else {return Err(ErrorCode::InvalidRequest)};
        if original!=target || epoch!=source {return Err(ErrorCode::StaleSession)}
        crate::storage::retain_shell_facts(known,resources).map_err(store_error)?;
        // Synchronous before the next await. Even a later failed durable checkpoint
        // leaves these facts with the exact entered settlement owner.
        *known=resources.clone();Ok(())
    }
    pub(crate) async fn checkpoint(&self)->SecurityResult<()> {
        let _guard=self.security.native_guard(&self.auth).await?;
        let entry=self.entry.clone();
        self.security.storage.call(move|store| {
            let held=entry.ticket.try_lock().map_err(|_|StorageError::Policy(ErrorCode::NotReady))?;
            let ticket=held.as_ref().ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            if entry.done.load(Ordering::Acquire) || entry.invalidated.load(Ordering::Acquire) || !entry.started.load(Ordering::Acquire) {return Err(StorageError::Policy(ErrorCode::NotReady))}
            let facts=entry.creation.as_ref().ok_or(StorageError::Corrupt)?.lock();
            let EnteredFacts::AgentShell {resources,..}=&*facts else {return Err(StorageError::Corrupt)};
            store.checkpoint_shell_facts(ticket,resources.clone(),now_ms())
        }).await.map_err(store_error)
    }
}


impl Security {
    pub(crate) async fn push_guard(&self,proof:&AuthenticatedPeer)->SecurityResult<OwnedMutexGuard<()>>{
        let fence=self.peer(proof.peer_id());if fence.stopped.load(Ordering::Acquire){return Err(ErrorCode::Unauthorized)}
        let guard=fence.gate.clone().lock_owned().await;let proof=proof.clone();self.storage.call(move|s|s.recheck(&proof)).await.map_err(store_error)?;
        if fence.stopped.load(Ordering::Acquire){return Err(ErrorCode::Unauthorized)}Ok(guard)
    }
    pub(crate) async fn publish_attention(&self,events:Vec<(Uuid,AttentionEvent)>)->SecurityResult<()> {
        for(peer,event)in events{
            let id=peer.clone();let Some(proof)=self.storage.call(move|s|s.attention_peer(&id)).await.map_err(store_error)?else{continue};
            let _guard=match self.push_guard(&proof).await{Ok(g)=>g,Err(_)=>continue};
            self.state.peer_event(&proof,EventBody::NotificationUpsert(NotificationUpsertEvent{event}))?;
            let p=proof.clone();if self.storage.call(move|s|s.push_sync_needed(&p)).await.map_err(store_error)?{self.state.peer_event(&proof,EventBody::SyncRequired(SyncRequiredEvent{reason:SyncReason::Overflow}))?;}
        }Ok(())
    }
}
