// SPDX-License-Identifier: GPL-3.0-or-later
//! Authenticated public-extension channel, never an alternate provider/session.
use super::*;
use crate::backends::{
    ControlsPublication, EffectiveControls, NativeControlModel, NativeModelIdentity,
    OriginalContext, PublicationGuard, PublicationKind, PublicationOrigin, SkillsPublication,
};
use serde::{Deserialize, Serialize};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::{OnceLock, Weak},
    time::Instant,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
};
const MODULE: &str = include_str!("omp_skills.ts");
const SOURCE: &str = include_str!("omp_skills_source.json");
const PACKAGE_VERSION: &str = "18.12.5";
const LIMITS: native_json::Limits = native_json::Limits {
    bytes: 262144,
    depth: 16,
    members: 4096,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedFile {
    path: String,
    sha256: String,
    size: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourcePin {
    version: String,
    files: Vec<PinnedFile>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnerRecord {
    version: u32,
    cap: String,
    endpoint: String,
    module_sha: String,
    package_root: String,
    files: Vec<PinnedFile>,
    allow_controls: bool,
    allow_images: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Hello {
    version: u32,
    cap: String,
    native_id: String,
    module_sha: String,
    package_version: String,
}
#[derive(Clone)]
struct LaunchScope {
    source: Source,
    pane: Id,
    terminal: Id,
    workspace: Id,
    tab: Id,
}
impl LaunchScope {
    fn matches(&self, original: &Connection) -> bool {
        if !self.source.same_physical_identity(&original.source)
            || original.key.terminal_id != self.terminal
        {
            return false;
        }
        original
            .source
            .snapshot
            .pane(&self.pane)
            .ok()
            .is_some_and(|pane| {
                pane.terminal_id == self.terminal
                    && pane.workspace_id == self.workspace
                    && pane.tab_id == self.tab
            })
    }
}
#[derive(Clone)]
struct Launch {
    cap: String,
    expires: Instant,
    used: bool,
    package: PathBuf,
    scope: LaunchScope,
}
struct Io {
    read: BufReader<OwnedReadHalf>,
    write: OwnedWriteHalf,
}
struct CatalogueState {
    generation: Uuid,
    fingerprint: Option<[u8; 32]>,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NativeEffect {
    None,
    Applied,
    Dispatched,
    Uncertain,
}
struct CallFailure {
    code: ErrorCode,
    entered: bool,
}
pub(super) enum OperationOutcome {
    Applied(EffectiveControls, Id),
    Dispatched,
    Refused(ErrorCode),
    Uncertain(ErrorCode),
}
pub(super) struct Channel {
    runtime: Weak<Runtime>,
    process: ProcessEvidence,
    native: Id,
    launch: LaunchScope,
    alive: AtomicBool,
    fd: OwnedFd,
    io: tokio::sync::Mutex<Io>,
    catalogue: Mutex<CatalogueState>,
    bound: Mutex<Option<(Target, Uuid, Weak<Connection>)>>,
    expires: Instant,
}
impl Channel {
    fn close(&self) {
        self.alive.store(false, Ordering::Release);
        unsafe {
            libc::shutdown(self.fd.as_raw_fd(), libc::SHUT_RDWR);
        }
    }
    fn current(&self) -> SecurityResult<()> {
        if !self.alive.load(Ordering::Acquire)
            || Instant::now() >= self.expires
            || herdr::process_evidence(self.process.pid).map_err(policy)? != self.process
        {
            return Err(ErrorCode::StaleSession);
        }
        let mut byte = 0u8;
        let n = unsafe {
            libc::recv(
                self.fd.as_raw_fd(),
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if n == 0 || n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::WouldBlock {
            return Err(ErrorCode::StaleSession);
        }
        Ok(())
    }
    fn authorize(&self, context: &OriginalContext) -> SecurityResult<()> {
        self.current()?;
        context.recheck()?;
        if self.process != context.process
            || self.native != context.native
            || !self.launch.source.same_physical_identity(&context.source)
            || !self
                .bound
                .lock()
                .as_ref()
                .is_some_and(|(target, epoch, _)| {
                    target == &context.target && epoch == &context.epoch
                })
        {
            return Err(ErrorCode::StaleSession);
        }
        Ok(())
    }
    pub(super) fn descriptor(&self) -> SecurityResult<OwnedFd> {
        self.current()?;
        self.fd.as_fd().try_clone_to_owned().map_err(policy)
    }
    async fn call_owned(
        self: &Arc<Self>,
        mut params: Value,
        deadline: tokio::time::Instant,
        context: &OriginalContext,
    ) -> Result<Value, CallFailure> {
        self.authorize(context).map_err(|code| CallFailure {
            code,
            entered: false,
        })?;
        let id = match params.get("id") {
            None => Uuid::random(),
            Some(id) => serde_json::from_value::<Uuid>(id.clone()).map_err(|_| CallFailure {
                code: ErrorCode::InvalidRequest,
                entered: false,
            })?,
        };
        params["id"] = json!(id);
        let mut bytes = serde_json::to_vec(&params).map_err(|_| CallFailure {
            code: ErrorCode::InvalidRequest,
            entered: false,
        })?;
        bytes.push(b'\n');
        if bytes.len() > LIMITS.bytes {
            return Err(CallFailure {
                code: ErrorCode::ResourceLimit,
                entered: false,
            });
        }
        let runtime = self.runtime.upgrade().ok_or(CallFailure {
            code: ErrorCode::NotReady,
            entered: false,
        })?;
        let channel = self.clone();
        let context = context.clone();
        let (send, receive) = oneshot::channel();
        // The runtime owns matching-reply drain even after a phone waiter disappears.
        // A queued canceled waiter writes nothing; an undrained entered outcome closes only this channel.
        {
            let mut jobs = runtime.jobs.lock();
            if jobs.len() >= 64 {
                return Err(CallFailure {
                    code: ErrorCode::Busy,
                    entered: false,
                });
            }
            jobs.spawn(async move {
                let mut started = false;
                let mut drained = false;
                let result = tokio::time::timeout_at(deadline, async {
                    let mut io = channel.io.lock().await;
                    if send.is_closed() {
                        return Err(ErrorCode::NotReady);
                    }
                    channel.authorize(&context)?;
                    if send.is_closed() {
                        return Err(ErrorCode::NotReady);
                    }
                    started = true;
                    io.write.write_all(&bytes).await.map_err(policy)?;
                    let response = line(&mut io.read).await?;
                    if response.get("id").and_then(Value::as_str) != Some(id.0.as_str())
                        || response.get("nativeId").and_then(Value::as_str)
                            != Some(channel.native.0.as_str())
                    {
                        return Err(ErrorCode::StaleSession);
                    }
                    let _: NativeEffect = serde_json::from_value(
                        response
                            .get("effect")
                            .cloned()
                            .ok_or(ErrorCode::StaleSession)?,
                    )
                    .map_err(|_| ErrorCode::StaleSession)?;
                    if let Some(error) = response.get("error")
                        && (!matches!(
                            error.as_str(),
                            Some("unavailable" | "busy" | "limit" | "refused" | "stale")
                        ) || response.as_object().is_none_or(|object| object.len() != 4))
                    {
                        return Err(ErrorCode::StaleSession);
                    }
                    drained = true;
                    channel.authorize(&context)?;
                    Ok(response)
                })
                .await
                .map_err(|_| ErrorCode::Timeout)
                .and_then(|result| result);
                if result.is_err() && started && !drained {
                    channel.close()
                }
                let _ = send.send(result.map_err(|code| CallFailure {
                    code,
                    entered: started,
                }));
            });
        }
        receive.await.map_err(|_| CallFailure {
            code: ErrorCode::NotReady,
            entered: true,
        })?
    }
    async fn authorized_call(
        self: &Arc<Self>,
        params: Value,
        deadline: tokio::time::Instant,
        context: &OriginalContext,
    ) -> SecurityResult<Value> {
        if !matches!(
            params.get("method").and_then(Value::as_str),
            Some("list" | "document" | "controls.read")
        ) {
            return Err(ErrorCode::InvalidRequest);
        }
        let response = self
            .call_owned(params, deadline, context)
            .await
            .map_err(|failure| failure.code)?;
        if response.get("effect").and_then(Value::as_str) != Some("none") {
            self.close();
            return Err(ErrorCode::StaleSession);
        }
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(native_error(error));
        }
        Ok(response)
    }
    pub(super) async fn operation(
        self: &Arc<Self>,
        params: Value,
        deadline: tokio::time::Instant,
        context: &OriginalContext,
    ) -> OperationOutcome {
        if !matches!(
            params.get("method").and_then(Value::as_str),
            Some("controls.apply" | "prompt")
        ) {
            return OperationOutcome::Refused(ErrorCode::InvalidRequest);
        }
        let response = match self.call_owned(params, deadline, context).await {
            Ok(response) => response,
            Err(failure) => {
                return if failure.entered {
                    OperationOutcome::Uncertain(failure.code)
                } else {
                    OperationOutcome::Refused(failure.code)
                };
            }
        };
        let effect: NativeEffect = match serde_json::from_value(response["effect"].clone()) {
            Ok(effect) => effect,
            Err(_) => return OperationOutcome::Uncertain(ErrorCode::StaleSession),
        };
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return if effect == NativeEffect::None {
                OperationOutcome::Refused(native_error(error))
            } else {
                OperationOutcome::Uncertain(native_error(error))
            };
        }
        if effect == NativeEffect::Dispatched
            && response.as_object().is_some_and(|object| object.len() == 3)
        {
            return OperationOutcome::Dispatched;
        }
        if matches!(effect, NativeEffect::None | NativeEffect::Applied)
            && response.as_object().is_some_and(|object| object.len() == 4)
            && let Some(current) = response.get("current")
            && let Ok(current) = serde_json::from_value::<NativeCurrent>(current.clone())
            && let Ok(current) = current.effective()
            && let Some(id) = response["id"].as_str()
        {
            return OperationOutcome::Applied(current, Id(id.to_owned()));
        }
        self.close();
        OperationOutcome::Uncertain(ErrorCode::StaleSession)
    }
}
fn native_error(error: &str) -> ErrorCode {
    match error {
        "busy" => ErrorCode::Busy,
        "limit" => ErrorCode::ResourceLimit,
        "refused" => ErrorCode::BackendError,
        "stale" => ErrorCode::StaleSession,
        _ => ErrorCode::NotReady,
    }
}
async fn line(reader: &mut BufReader<OwnedReadHalf>) -> SecurityResult<Value> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(policy)?;
        if available.is_empty() {
            return Err(ErrorCode::NotReady);
        }
        let end = available.iter().position(|b| *b == b'\n');
        let n = end.map_or(available.len(), |n| n + 1);
        if bytes.len().checked_add(n).is_none_or(|n| n > LIMITS.bytes) {
            return Err(ErrorCode::ResourceLimit);
        }
        bytes.extend_from_slice(&available[..n]);
        reader.consume(n);
        if end.is_some() {
            break;
        }
    }
    native_json::parse(&bytes, LIMITS).map_err(policy)
}
fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |n, (a, b)| n | (a ^ b)) == 0
}
fn pin_files(package: &Path, process: Option<&ProcessEvidence>) -> io::Result<Vec<PinnedFile>> {
    let pin: SourcePin = serde_json::from_str(SOURCE).map_err(|_| invalid())?;
    if pin.version != PACKAGE_VERSION || pin.files.len() > 16 {
        return Err(invalid());
    }
    pin_files_selected(package, process, pin)
}
fn pin_files_selected(
    package: &Path,
    process: Option<&ProcessEvidence>,
    pin: SourcePin,
) -> io::Result<Vec<PinnedFile>> {
    if let Some(process) = process
        && herdr::process_evidence(process.pid)? != *process
    {
        return Err(invalid());
    }
    for expected in &pin.files {
        let path = package.join(&expected.path);
        let object = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let before = object.metadata()?;
        if !before.is_file() || before.len() != expected.size || before.len() > 32 * 1024 * 1024 {
            return Err(invalid());
        }
        // Public code provenance only; this is not document-read authority.
        if let Some(process) = process
            && herdr::process_object(process, &path)? != (before.dev(), before.ino())
        {
            return Err(invalid());
        }
        let mut file = File::open(format!("/proc/self/fd/{}", object.as_raw_fd()))?;
        let mut total = 0u64;
        let mut digest = sha2::Sha256::new();
        let mut chunk = [0u8; 65536];
        use sha2::Digest;
        loop {
            let count = file.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            total = total.checked_add(count as u64).ok_or_else(invalid)?;
            if total > expected.size {
                return Err(invalid());
            }
            digest.update(&chunk[..count]);
        }
        let after = file.metadata()?;
        if total != expected.size
            || hex(&digest.finalize()) != expected.sha256
            || before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(invalid());
        }
        if let Some(process) = process
            && herdr::process_object(process, &path)? != (before.dev(), before.ino())
        {
            return Err(invalid());
        }
    }
    if let Some(process) = process
        && herdr::process_evidence(process.pid)? != *process
    {
        return Err(invalid());
    }
    Ok(pin.files)
}
pub(super) fn qualify_package_root(package: &Path) -> io::Result<()> {
    pin_files(package, None).map(|_| ())
}
fn exclusive(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

// A successful bind owns only this filesystem socket, never a stale predecessor.
struct SkillsSocket<'a> {
    listener: UnixListener,
    path: &'a Path,
    dev: u64,
    ino: u64,
    uid: u32,
}
impl<'a> SkillsSocket<'a> {
    fn bind(path: &'a Path) -> io::Result<Self> {
        crate::config::socket_path(path)?;
        pairing::private_dir(path.parent().ok_or_else(invalid)?, true)?;
        let listener = UnixListener::bind(path)?;
        let object = fs::symlink_metadata(path)?;
        if !object.file_type().is_socket() || object.uid() != pairing::uid() {
            return Err(invalid());
        }
        // Capture before chmod so a later failure still has an identity-checked owner.
        let socket = Self {
            listener,
            path,
            dev: object.dev(),
            ino: object.ino(),
            uid: object.uid(),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        let object = fs::symlink_metadata(path)?;
        if !socket.owns(&object) || object.mode() & 0o7777 != 0o600 {
            return Err(invalid());
        }
        Ok(socket)
    }
    fn owns(&self, object: &Metadata) -> bool {
        object.file_type().is_socket()
            && object.uid() == self.uid
            && object.dev() == self.dev
            && object.ino() == self.ino
    }
}
impl Drop for SkillsSocket<'_> {
    fn drop(&mut self) {
        if let Ok(object) = fs::symlink_metadata(self.path)
            && self.owns(&object)
        {
            let _ = fs::remove_file(self.path);
        }
    }
}

pub(super) struct Manager {
    config: Config,
    endpoint: PathBuf,
    endpoint_agent: PathBuf,
    ready: AtomicBool,
    launches: Mutex<HashMap<Uuid, Launch>>,
    channels: Mutex<Vec<Arc<Channel>>>,
    runtime: OnceLock<Weak<Runtime>>,
}
impl Manager {
    pub(super) fn new(config: &Config) -> Arc<Self> {
        let filename = Uuid::random().0;
        Arc::new(Self {
            config: config.clone(),
            endpoint: config.runtime_dir.join("skills").join(&filename),
            endpoint_agent: config.runtime_dir_agent.join("skills").join(&filename),
            ready: AtomicBool::new(false),
            launches: Mutex::new(HashMap::new()),
            channels: Mutex::new(Vec::new()),
            runtime: OnceLock::new(),
        })
    }
    fn base(&self) -> PathBuf {
        self.config.runtime_dir.join("skills")
    }
    fn owner_record(
        &self,
        cap: String,
        package: &Path,
        files: Vec<PinnedFile>,
    ) -> io::Result<OwnerRecord> {
        crate::config::socket_path(&self.endpoint_agent)?;
        Ok(OwnerRecord {
            version: 2,
            cap,
            endpoint: self.endpoint_agent.to_str().ok_or_else(invalid)?.to_owned(),
            module_sha: hex(&sha256(MODULE.as_bytes())),
            package_root: package.to_str().ok_or_else(invalid)?.to_owned(),
            files,
            allow_controls: self.config.allow_omp_controls,
            allow_images: self.config.allow_attachments,
        })
    }
    pub(super) fn prepare(&self, source: Source, pane: &Id) -> io::Result<Vec<String>> {
        let native = source.snapshot.pane(pane).map_err(|_| invalid())?;
        let scope = LaunchScope {
            source: source.clone(),
            pane: native.pane_id.clone(),
            terminal: native.terminal_id.clone(),
            workspace: native.workspace_id.clone(),
            tab: native.tab_id.clone(),
        };
        if !self.ready.load(Ordering::Acquire) {
            return Err(invalid());
        }
        let package = self
            .config
            .omp_skills_package_root
            .as_ref()
            .ok_or_else(invalid)?;
        let files = pin_files(package, None)?;
        let mut launches = self.launches.lock();
        if launches.len() >= 32 {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "OMP extension launch resource limit",
            ));
        }
        let id = Uuid::random();
        let cap = URL_SAFE_NO_PAD.encode(pairing::random32()?);
        // Consume before exclusive allocation; failed prepares cannot create unbounded abandoned objects.
        launches.insert(
            id.clone(),
            Launch {
                cap: cap.clone(),
                expires: Instant::now(),
                used: true,
                package: package.clone(),
                scope: scope.clone(),
            },
        );
        let dir = self.base().join(&id.0);
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
        pairing::private_dir(&dir, false)?;
        let agent = self.config.runtime_dir_agent.join("skills").join(&id.0);
        exclusive(&dir.join("extension.ts"), MODULE.as_bytes())?;
        let owner = self.owner_record(cap.clone(), package, files)?;
        exclusive(
            &dir.join("owner.json"),
            &serde_json::to_vec(&owner).map_err(|_| invalid())?,
        )?;
        launches.insert(
            id,
            Launch {
                cap,
                expires: Instant::now() + Duration::from_secs(30),
                used: false,
                package: package.clone(),
                scope,
            },
        );
        Ok(vec![
            "--extension".into(),
            agent
                .join("extension.ts")
                .to_str()
                .ok_or_else(invalid)?
                .to_owned(),
        ])
    }
    pub(super) fn channel(&self, original: &Arc<Connection>) -> Option<Arc<Channel>> {
        let channel = self
            .channels
            .lock()
            .iter()
            .find(|channel| {
                channel.process == original.owner
                    && channel.native == original.key.native_conversation_id
                    && channel.launch.matches(original)
                    && channel.current().is_ok()
            })
            .cloned()?;
        {
            let mut bound = channel.bound.lock();
            match bound.as_ref() {
                Some((target, epoch, owner)) => {
                    if target != &original.target
                        || epoch != &original.epoch
                        || !owner
                            .upgrade()
                            .is_some_and(|owner| Arc::ptr_eq(&owner, original))
                    {
                        channel.close();
                        return None;
                    }
                }
                None => {
                    *bound = Some((
                        original.target.clone(),
                        original.epoch.clone(),
                        Arc::downgrade(original),
                    ))
                }
            }
        }
        Some(channel)
    }
    fn changed(&self) {
        if let Some(runtime) = self.runtime.get().and_then(Weak::upgrade) {
            runtime.refresh_skills_capabilities();
        }
    }
    pub(super) async fn run(
        self: Arc<Self>,
        runtime: Weak<Runtime>,
        mut stop: watch::Receiver<bool>,
    ) {
        let _ = self.runtime.set(runtime);
        if self.config.omp_skills_package_root.is_none() {
            return;
        }
        if crate::config::socket_path(&self.endpoint_agent).is_err() {
            return;
        }
        let Ok(socket) = SkillsSocket::bind(&self.endpoint) else {
            return;
        };
        self.ready.store(true, Ordering::Release);
        let permits = Arc::new(tokio::sync::Semaphore::new(8));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {biased;_=stop.changed()=>break,_=tasks.join_next(),if !tasks.is_empty()=>{},accepted=socket.listener.accept()=>{
             let Ok((stream,_))=accepted else{break};let Ok(permit)=permits.clone().try_acquire_owned()else{drop(stream);continue};let owner=self.clone();tasks.spawn(async move{let _permit=permit;let _=tokio::time::timeout(Duration::from_secs(30),owner.register(stream)).await;});
            }}
        }
        self.ready.store(false, Ordering::Release);
        for channel in self.channels.lock().iter() {
            channel.close()
        }
        self.changed();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    async fn register(self: Arc<Self>, stream: UnixStream) -> SecurityResult<()> {
        let cred = stream.peer_cred().map_err(policy)?;
        if cred.uid() != pairing::uid() {
            return Err(ErrorCode::Unauthorized);
        }
        let pid = cred
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or(ErrorCode::Unauthorized)?;
        let process = herdr::process_evidence(pid).map_err(policy)?;
        let fd = stream.as_fd().try_clone_to_owned().map_err(policy)?;
        let (read, write) = stream.into_split();
        let mut read = BufReader::new(read);
        let hello: Hello = serde_json::from_value(line(&mut read).await?).map_err(policy)?;
        if hello.version != 2
            || hello.module_sha != hex(&sha256(MODULE.as_bytes()))
            || hello.package_version != PACKAGE_VERSION
        {
            return Err(ErrorCode::Unauthorized);
        }
        let native = Id::new(hello.native_id).map_err(|_| ErrorCode::Unauthorized)?;
        let launch = {
            let mut launches = self.launches.lock();
            let matching = launches
                .values_mut()
                .find(|launch| {
                    !launch.used
                        && Instant::now() < launch.expires
                        && ct_eq(&launch.cap, &hello.cap)
                })
                .ok_or(ErrorCode::Unauthorized)?;
            matching.used = true;
            matching.clone()
        };
        pin_files(&launch.package, Some(&process)).map_err(policy)?;
        if herdr::process_evidence(pid).map_err(policy)? != process {
            return Err(ErrorCode::StaleSession);
        }
        let mut io = Io { read, write };
        io.write
            .write_all(
                &serde_json::to_vec(&json!({"ready":true,"nativeId":native})).map_err(policy)?,
            )
            .await
            .map_err(policy)?;
        io.write.write_all(b"\n").await.map_err(policy)?;
        let channel = Arc::new(Channel {
            runtime: self.runtime.get().cloned().unwrap_or_default(),
            process,
            native,
            launch: launch.scope,
            alive: AtomicBool::new(true),
            fd,
            io: tokio::sync::Mutex::new(io),
            catalogue: Mutex::new(CatalogueState {
                generation: Uuid::random(),
                fingerprint: None,
            }),
            bound: Mutex::new(None),
            expires: Instant::now() + Duration::from_secs(3600),
        });
        {
            let mut channels = self.channels.lock();
            if channels.len() >= 32 || channels.iter().any(|old| old.process == channel.process) {
                channel.close();
                return Err(ErrorCode::ResourceLimit);
            }
            channels.push(channel)
        }
        self.changed();
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeEntry {
    reference: Uuid,
    name: String,
    description: String,
    path: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeList {
    #[serde(rename = "id")]
    _id: Uuid,
    native_id: Id,
    effect: NativeEffect,
    catalogue: Uuid,
    entries: Vec<NativeEntry>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeDocument {
    #[serde(rename = "id")]
    _id: Uuid,
    native_id: Id,
    effect: NativeEffect,
    catalogue: Uuid,
    reference: Uuid,
    path: String,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeCurrent {
    provider: Option<String>,
    model: Option<String>,
    effort: Option<Text>,
}
impl NativeCurrent {
    fn effective(self) -> SecurityResult<EffectiveControls> {
        let model = match (self.provider, self.model) {
            (Some(provider), Some(model)) => {
                Id::new(provider.clone()).map_err(policy)?;
                Id::new(model.clone()).map_err(policy)?;
                Some(NativeModelIdentity { provider, model })
            }
            (None, None) => None,
            _ => return Err(ErrorCode::StaleSession),
        };
        if let Some(effort) = &self.effort {
            effort.validate().map_err(policy)?;
        }
        Ok(EffectiveControls {
            model,
            effort: self.effort,
            mode: None,
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeModel {
    reference: Uuid,
    provider: String,
    model: String,
    label: String,
    efforts: Vec<Text>,
    #[serde(rename = "images")]
    _images: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeControls {
    #[serde(rename = "id")]
    _id: Uuid,
    native_id: Id,
    effect: NativeEffect,
    catalogue: Uuid,
    models: Vec<NativeModel>,
    current: NativeCurrent,
}
fn catalogue_fingerprint(entries: &[NativeEntry]) -> [u8; 32] {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for row in entries {
        for value in [&row.name, &row.description, &row.path] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
    }
    hash.finalize().into()
}
impl Runtime {
    pub(super) fn refresh_skills_capabilities(&self) {
        let connections = self.connections.lock();
        for original in connections.values() {
            let channel = self.skills.channel(original).filter(|_| {
                original
                    .validate(&self.security.state, &self.topology)
                    .is_ok()
            });
            let generation = channel
                .as_ref()
                .map(|channel| channel.catalogue.lock().generation.clone());
            let _ = self.security.state.skills_generation(
                &original.target,
                &original.epoch,
                generation,
            );
            let _ = self.security.state.controls_available(
                &original.target,
                &original.epoch,
                self.topology.config.allow_omp_controls && channel.is_some(),
            );
        }
    }
    fn extension_context(
        self: &Arc<Self>,
        target: &Target,
    ) -> SecurityResult<(Arc<Connection>, Arc<Channel>, OriginalContext)> {
        let original = self.connection(target)?;
        let channel = self.skills.channel(&original).ok_or(ErrorCode::NotReady)?;
        let context = self.original_context(target)?;
        channel.authorize(&context)?;
        Ok((original, channel, context))
    }
    pub(crate) async fn skills_list(
        self: &Arc<Self>,
        auth: &Arc<Authorized>,
        target: &Target,
    ) -> SecurityResult<SkillsPublication> {
        self.security
            .state
            .clear_skills_publication(&auth.connection_id);
        let slot = self
            .security
            .state
            .reserve_publication(auth, PublicationKind::Skills)?;
        let (_, channel, context) = self.extension_context(target)?;
        let context = context.authorized(auth);
        let expires = Instant::now() + Duration::from_secs(300);
        let context = context.leased(expires);
        let native: NativeList = serde_json::from_value(
            channel
                .authorized_call(
                    json!({"method":"list"}),
                    tokio::time::Instant::now() + Duration::from_secs(5),
                    &context,
                )
                .await?,
        )
        .map_err(policy)?;
        if native.effect != NativeEffect::None
            || native.native_id != context.native
            || native.entries.len() > 128
        {
            return Err(ErrorCode::StaleSession);
        }
        let fingerprint = catalogue_fingerprint(&native.entries);
        let generation = {
            let mut state = channel.catalogue.lock();
            if state.fingerprint.is_some_and(|old| old != fingerprint) {
                state.generation = Uuid::random()
            }
            state.fingerprint = Some(fingerprint);
            state.generation.clone()
        };
        let mut names = HashSet::new();
        let mut skills = Vec::with_capacity(native.entries.len());
        let mut paths = Vec::new();
        for row in native.entries {
            if !row.name.starts_with("skill:")
                || !names.insert(row.name.clone())
                || row.name.chars().any(char::is_control)
            {
                return Err(ErrorCode::NotReady);
            }
            let index = skills.len();
            paths.push((index, PathBuf::from(row.path), Some(row.reference)));
            skills.push(SkillEntry {
                name: row
                    .name
                    .strip_prefix("skill:")
                    .ok_or(ErrorCode::NotReady)?
                    .to_owned(),
                description: row.description,
                path: None,
                scope: "enabled".into(),
                enabled: true,
                display_name: None,
                short_description: None,
                command_name: Some(row.name),
                document_ref: None,
                invocation_ref: None,
            });
        }
        let check_channel = channel.clone();
        let expected = generation.clone();
        let check = Arc::new(move || {
            check_channel.current()?;
            if check_channel.catalogue.lock().generation != expected {
                return Err(ErrorCode::StaleSession);
            }
            Ok(())
        });
        let expected = generation.clone();
        let fence_channel = channel.clone();
        let fence: PublicationGuard = Arc::new(move |commit| {
            fence_channel.current()?;
            let observed = fence_channel.catalogue.lock();
            if observed.generation != expected {
                return Err(ErrorCode::StaleSession);
            }
            commit()
        });
        let context = context.guarded(check, fence);
        self.refresh_skills_capabilities();
        let catalogue = SkillsListResult {
            target: target.clone(),
            catalogue_generation: generation,
            skills,
            errors: Vec::new(),
            provider: Provider::Omp,
        };
        self.topology
            .skill_documents
            .prepare(
                SkillsPublication {
                    origin: PublicationOrigin {
                        peer: auth.proof.peer_id().clone(),
                        peer_epoch: auth.proof.epoch(),
                        connection: auth.connection_id.clone(),
                        context,
                    },
                    catalogue,
                    documents: HashMap::new(),
                    document_expires: expires,
                    native_catalogue: Some(native.catalogue),
                    native_cwd: None,
                    slot,
                },
                paths,
            )
            .await
    }
    pub(crate) fn enqueue_skills_list(
        &self,
        connection: &Uuid,
        response_id: Uuid,
        publication: SkillsPublication,
    ) -> SecurityResult<()> {
        let context = publication.origin.context.clone();
        let mut publication = Some(publication);
        context.with_current(|| {
            self.security.state.skills_list_response(
                connection,
                response_id.clone(),
                publication.take().ok_or(ErrorCode::StaleSession)?,
            )
        })
    }
    pub(crate) async fn authorize_skill_document(
        self: &Arc<Self>,
        auth: &Authorized,
        publication: &Arc<SkillsPublication>,
        document: &Uuid,
    ) -> SecurityResult<()> {
        self.security
            .state
            .skills_publication_current(auth, publication)?;
        publication.origin.context.recheck()?;
        let selected = publication
            .documents
            .get(document)
            .ok_or(ErrorCode::NotReady)?;
        let reference = selected
            .native_reference
            .as_ref()
            .ok_or(ErrorCode::NotReady)?;
        let catalogue = publication
            .native_catalogue
            .as_ref()
            .ok_or(ErrorCode::NotReady)?;
        let (_, channel, _) = self.extension_context(&publication.catalogue.target)?;
        let native: NativeDocument = serde_json::from_value(
            channel
                .authorized_call(
                    json!({"method":"document","catalogue":catalogue,"reference":reference}),
                    tokio::time::Instant::now() + Duration::from_secs(5),
                    &publication.origin.context,
                )
                .await?,
        )
        .map_err(policy)?;
        if native.effect != NativeEffect::None
            || native.native_id != publication.origin.context.native
            || &native.catalogue != catalogue
            || &native.reference != reference
            || Path::new(&native.path) != selected.file.pathname()
        {
            return Err(ErrorCode::StaleSession);
        }
        publication.origin.context.recheck()
    }
    pub(crate) async fn controls_read(
        self: &Arc<Self>,
        auth: &Arc<Authorized>,
        target: &Target,
    ) -> SecurityResult<ControlsPublication> {
        if !self.topology.config.allow_omp_controls {
            return Err(ErrorCode::NotReady);
        }
        self.security
            .state
            .clear_controls_publication(&auth.connection_id);
        let slot = self
            .security
            .state
            .reserve_publication(auth, PublicationKind::Controls)?;
        let (original, channel, context) = self.extension_context(target)?;
        let context = context.authorized(auth);
        let observation = original.controls.lock().generation.clone();
        let expires = Instant::now() + Duration::from_secs(300);
        let context = context.leased(expires);
        let native: NativeControls = serde_json::from_value(
            channel
                .authorized_call(
                    json!({"method":"controls.read"}),
                    tokio::time::Instant::now() + Duration::from_secs(5),
                    &context,
                )
                .await?,
        )
        .map_err(policy)?;
        if native.effect != NativeEffect::None
            || native.native_id != context.native
            || native.models.len() > 256
            || original.controls.lock().generation != observation
        {
            return Err(ErrorCode::StaleSession);
        }
        let current = native.current.effective()?;
        let mut models = Vec::with_capacity(native.models.len());
        let mut bindings = Vec::with_capacity(native.models.len());
        for row in native.models {
            Id::new(row.provider.clone()).map_err(policy)?;
            Id::new(row.model.clone()).map_err(policy)?;
            models.push(SessionControlModel {
                reference: Uuid::random(),
                label: Label::new(row.label).map_err(policy)?,
                efforts: row.efforts,
            });
            bindings.push(NativeControlModel {
                identity: NativeModelIdentity {
                    provider: row.provider,
                    model: row.model,
                },
                native_reference: Some(row.reference),
            });
        }
        let model_ref = current
            .model
            .as_ref()
            .and_then(|current| bindings.iter().position(|model| &model.identity == current))
            .map(|index| models[index].reference.clone());
        let catalogue = SessionControlsReadResult {
            target: target.clone(),
            catalogue_generation: Uuid::random(),
            models,
            current: SessionControlsCurrent {
                model_ref,
                effort: current.effort,
                mode: None,
            },
            modes: Vec::new(),
            deliveries: vec![
                PromptDelivery::Native,
                PromptDelivery::Steer,
                PromptDelivery::FollowUp,
                PromptDelivery::Aside,
            ],
            queue_available: false,
        };
        catalogue.validate().map_err(|_| ErrorCode::ResourceLimit)?;
        Ok(ControlsPublication {
            origin: PublicationOrigin {
                peer: auth.proof.peer_id().clone(),
                peer_epoch: auth.proof.epoch(),
                connection: auth.connection_id.clone(),
                context,
            },
            catalogue,
            models: bindings,
            current_native: current.model,
            native_catalogue: Some(native.catalogue),
            observation,
            expires,
            slot,
        })
    }
    pub(crate) fn enqueue_controls(
        &self,
        connection: &Uuid,
        response_id: Uuid,
        publication: ControlsPublication,
    ) -> SecurityResult<()> {
        let original = self.connection(&publication.catalogue.target)?;
        if original.epoch != publication.origin.context.epoch {
            return Err(ErrorCode::StaleSession);
        }
        let context = publication.origin.context.clone();
        let observation = publication.observation.clone();
        let mut publication = Some(publication);
        context.with_current(|| {
            let controls = original.controls.lock();
            if controls.generation != observation {
                return Err(ErrorCode::StaleSession);
            }
            self.security.state.controls_response(
                connection,
                response_id.clone(),
                publication.take().ok_or(ErrorCode::StaleSession)?,
            )
        })
    }
    pub(super) fn validate_controls_selection(
        &self,
        original: &Arc<Connection>,
        publication: &ControlsPublication,
        params: &SessionControlsApplyParams,
    ) -> SecurityResult<()> {
        if !self.topology.config.allow_omp_controls
            || original.epoch != publication.origin.context.epoch
            || original.target != params.target
            || original.controls.lock().generation != publication.observation
        {
            return Err(ErrorCode::StaleSession);
        }
        publication.origin.context.recheck()?;
        publication.validate_apply(params)?;
        Ok(())
    }
    pub(super) fn controls_command(
        &self,
        publication: &ControlsPublication,
        params: &SessionControlsApplyParams,
    ) -> SecurityResult<Value> {
        let index = publication.validate_apply(params)?;
        let mut command = json!({"method":"controls.apply","catalogue":publication.native_catalogue.as_ref().ok_or(ErrorCode::NotReady)?});
        if params.model_ref.is_some() {
            command["modelRef"] = json!(
                index
                    .and_then(|index| publication.models[index].native_reference.as_ref())
                    .ok_or(ErrorCode::NotReady)?
            );
        }
        if let Some(effort) = &params.effort {
            command["effort"] = json!(effort);
        }
        if params.model_ref.is_none() {
            let model = publication
                .current_native
                .as_ref()
                .ok_or(ErrorCode::NotReady)?;
            command["expectedModel"] = json!({"provider":model.provider,"model":model.model});
        }
        Ok(command)
    }
    pub(super) async fn write_controls_once(
        self: &Arc<Self>,
        auth: &Arc<Authorized>,
        request: Request,
        deadline: tokio::time::Instant,
        original: Arc<Connection>,
        publication: Arc<ControlsPublication>,
    ) -> SecurityResult<Receipt> {
        let RequestBody::SessionControlsApply(params) = &request.body else {
            return Err(ErrorCode::InvalidRequest);
        };
        let mut connection = self
            .wait_connection(auth, &request, &params.target, deadline, original)
            .await
            .map_err(|(_, code)| code)?;
        let channel = self
            .skills
            .channel(&connection)
            .ok_or(ErrorCode::NotReady)?;
        let context = publication.origin.context.clone();
        let index = publication.validate_apply(params)?;
        let expected = publication.desired_model(index);
        let mut command = Some(self.controls_command(&publication, params)?);
        let mut boundary = Some(
            self.security
                .unstarted_boundary(auth, request.id.clone(), channel.descriptor()?, deadline)
                .await?,
        );
        let outcome = async {
            loop {
                boundary = boundary
                    .take()
                    .ok_or(ErrorCode::NotReady)?
                    .try_start(
                        || {
                            let readiness = self
                                .boundary_readiness(&request, &connection)
                                .map_err(|(_, code)| code)?;
                            if matches!(readiness, pairing::BoundaryReadiness::Ready) {
                                self.security
                                    .state
                                    .controls_publication_current(auth, &publication)?;
                                self.validate_controls_selection(
                                    &connection,
                                    &publication,
                                    params,
                                )?;
                            }
                            Ok(readiness)
                        },
                        async {
                            let operation = match command.take() {
                                Some(command) => {
                                    channel.operation(command, deadline, &context).await
                                }
                                None => OperationOutcome::Uncertain(ErrorCode::NotReady),
                            };
                            let (state, proof, error) = match operation {
                                OperationOutcome::Applied(current, id)
                                    if current.model.as_ref() == expected
                                        && params.effort.as_ref().is_none_or(|effort| {
                                            current.effort.as_ref() == Some(effort)
                                        }) =>
                                {
                                    (
                                        ReceiptState::Succeeded,
                                        TransitionProof::NativeEffectiveState(id),
                                        None,
                                    )
                                }
                                OperationOutcome::Refused(code) => (
                                    ReceiptState::Failed,
                                    TransitionProof::DefinitiveNoEffect(Id(
                                        "OMP extension preflight".into(),
                                    )),
                                    Some(code),
                                ),
                                OperationOutcome::Uncertain(code) => (
                                    ReceiptState::Uncertain,
                                    TransitionProof::LostOutcome,
                                    Some(code),
                                ),
                                OperationOutcome::Applied(..) | OperationOutcome::Dispatched => (
                                    ReceiptState::Uncertain,
                                    TransitionProof::LostOutcome,
                                    Some(ErrorCode::StaleSession),
                                ),
                            };
                            ReceiptTransition {
                                state,
                                proof,
                                result: None,
                                error: error.map(crate::transport::wire_error),
                                at: pairing::now_ms(),
                            }
                        },
                    )
                    .await?;
                if boundary.is_none() {
                    break;
                }
                connection = self
                    .wait_connection(auth, &request, &params.target, deadline, connection)
                    .await
                    .map_err(|(_, code)| code)?;
            }
            receipt(&self.security, auth, &request.id).await
        }
        .await;
        match (outcome, boundary) {
            (Err(code), Some(boundary)) => boundary.fail(code).await.and(Err(code)),
            (outcome, _) => outcome,
        }
    }
    pub(super) async fn write_prompt_extension(
        self: &Arc<Self>,
        auth: &Arc<Authorized>,
        request: Request,
        deadline: tokio::time::Instant,
        original: Arc<Connection>,
    ) -> SecurityResult<Receipt> {
        let RequestBody::PromptSubmit(params) = &request.body else {
            return Err(ErrorCode::InvalidRequest);
        };
        if params.skill.is_some()
            || params.native_turn_id.is_some()
            || params.delivery == Some(PromptDelivery::Queue)
            || params
                .delivery
                .is_some_and(|delivery| delivery != PromptDelivery::Native)
                && !self.topology.config.allow_omp_controls
        {
            return Err(ErrorCode::NotReady);
        }
        let mut connection = self
            .wait_connection(auth, &request, &params.target, deadline, original)
            .await
            .map_err(|(_, code)| code)?;
        let channel = self
            .skills
            .channel(&connection)
            .ok_or(ErrorCode::NotReady)?;
        let context = self.original_context(&params.target)?;
        if context.epoch != connection.epoch {
            return Err(ErrorCode::StaleSession);
        }
        let images = if params.images.is_some() {
            self.security
                .prepare_prompt_images(
                    auth,
                    &request,
                    crate::attachments::ImageSource {
                        original: context.clone(),
                        attachment: self.attachment_context(&params.target)?,
                    },
                )
                .await?
        } else {
            None
        };
        let mut boundary = None;
        let outcome = async {
            let mut command = json!({"method":"prompt","text":params.text.0});
            if let Some(delivery) = params.delivery {
                command["delivery"] = json!(delivery);
            }
            if let Some(images) = &images {
                command["images"] = json!(images.omp_references()?);
                command["id"] = json!(images.native_input());
            }
            let fd = channel.descriptor()?;
            boundary = Some(match &images {
                Some(images) => {
                    self.security
                        .unstarted_image_boundary(
                            auth,
                            request.id.clone(),
                            fd,
                            deadline,
                            images.clone(),
                        )
                        .await?
                }
                None => {
                    self.security
                        .unstarted_boundary(auth, request.id.clone(), fd, deadline)
                        .await?
                }
            });
            let mut command = Some(command);
            loop {
                let checkpoint = if images.is_some() {
                    Some(
                        boundary
                            .as_ref()
                            .ok_or(ErrorCode::NotReady)?
                            .image_checkpoint()?,
                    )
                } else {
                    None
                };
                boundary = boundary
                    .take()
                    .ok_or(ErrorCode::NotReady)?
                    .try_start(
                        || {
                            let readiness = self
                                .boundary_readiness(&request, &connection)
                                .map_err(|(_, code)| code)?;
                            if matches!(readiness, pairing::BoundaryReadiness::Ready) {
                                context.recheck()?;
                                if let Some(images) = &images {
                                    images.recheck()?;
                                }
                            }
                            Ok(readiness)
                        },
                        async {
                            if let Some(checkpoint) = &checkpoint
                                && let Err(code) = checkpoint.mark().await
                            {
                                let _ = checkpoint.no_effect().await;
                                return ReceiptTransition {
                                    state: ReceiptState::Failed,
                                    proof: TransitionProof::DefinitiveNoEffect(Id(
                                        "Original image preflight".into(),
                                    )),
                                    result: None,
                                    error: Some(crate::transport::wire_error(code)),
                                    at: pairing::now_ms(),
                                };
                            }
                            let operation = match command.take() {
                                Some(command) => {
                                    channel.operation(command, deadline, &context).await
                                }
                                None => OperationOutcome::Uncertain(ErrorCode::NotReady),
                            };
                            let (state, proof, result, error) = match operation {
                                OperationOutcome::Dispatched => {
                                    if let Some(checkpoint) = &checkpoint {
                                        let _ = checkpoint.read_complete(context.clone()).await;
                                    }
                                    (
                                        ReceiptState::Dispatched,
                                        TransitionProof::WriteCompleted,
                                        Some(ActionResult::Prompt(ActionResultPrompt {
                                            native_turn_id: None,
                                        })),
                                        None,
                                    )
                                }
                                OperationOutcome::Refused(code) => {
                                    if let Some(checkpoint) = &checkpoint {
                                        let _ = checkpoint.no_effect().await;
                                    }
                                    (
                                        ReceiptState::Failed,
                                        TransitionProof::DefinitiveNoEffect(Id(
                                            "OMP public prompt preflight".into(),
                                        )),
                                        None,
                                        Some(code),
                                    )
                                }
                                OperationOutcome::Uncertain(code) => (
                                    ReceiptState::Uncertain,
                                    TransitionProof::LostOutcome,
                                    None,
                                    Some(code),
                                ),
                                OperationOutcome::Applied(..) => (
                                    ReceiptState::Uncertain,
                                    TransitionProof::LostOutcome,
                                    None,
                                    Some(ErrorCode::StaleSession),
                                ),
                            };
                            ReceiptTransition {
                                state,
                                proof,
                                result,
                                error: error.map(crate::transport::wire_error),
                                at: pairing::now_ms(),
                            }
                        },
                    )
                    .await?;
                if boundary.is_none() {
                    break;
                }
                connection = self
                    .wait_connection(auth, &request, &params.target, deadline, connection)
                    .await
                    .map_err(|(_, code)| code)?;
            }
            receipt(&self.security, auth, &request.id).await
        }
        .await;
        let outcome = match (outcome, boundary) {
            (Err(code), Some(boundary)) => boundary.fail(code).await.and(Err(code)),
            (outcome, _) => outcome,
        };
        if outcome.is_err()
            && let Some(images) = images
            && !images.entered.load(Ordering::Acquire)
        {
            let _ = self.security.image_preparation_failed(images).await;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            // Real Unix-socket fixtures need short paths even under Nix's nested TMPDIR.
            let root = fs::canonicalize("/tmp")
                .unwrap()
                .join(format!("goatr-omp-{}", Uuid::random().0));
            fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            Self(root)
        }
        fn config(&self) -> Config {
            serde_json::from_value(json!({"version":1,"runtime_dir":self.0,"runtime_dir_agent":self.0,"omp_skills_package_root":"/not-executed","executables":{"herdr":"/not-executed"}})).unwrap()
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[tokio::test]
    async fn manager_lifetimes_leave_legacy_socket_untouched_and_advertise_the_bound_endpoint() {
        let root = Root::new();
        let config = root.config();
        let base = config.runtime_dir.join("skills");
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        let legacy = base.join("channel");
        drop(std::os::unix::net::UnixListener::bind(&legacy).unwrap());
        let before = fs::symlink_metadata(&legacy).unwrap();
        let mut previous = None;
        for _ in 0..2 {
            let manager = Manager::new(&config);
            let owner = serde_json::to_value(
                manager
                    .owner_record("test".into(), Path::new("/not-executed"), Vec::new())
                    .unwrap(),
            )
            .unwrap();
            let endpoint = PathBuf::from(owner["endpoint"].as_str().unwrap());
            assert_eq!(endpoint, manager.endpoint);
            assert_ne!(previous.as_ref(), Some(&endpoint));
            let (stop, stopped) = watch::channel(false);
            let run = manager.clone().run(Weak::new(), stopped);
            tokio::pin!(run);
            let client=tokio::time::timeout(Duration::from_secs(5),async {
    tokio::select!{biased;_=&mut run=>panic!("extension listener retired before connection"),client=UnixStream::connect(&endpoint)=>client.unwrap()}
   }).await.unwrap();
            assert_eq!(client.peer_cred().unwrap().uid(), pairing::uid());
            assert!(manager.ready.load(Ordering::Acquire));
            let socket = fs::symlink_metadata(&endpoint).unwrap();
            assert!(socket.file_type().is_socket());
            assert_eq!(socket.mode() & 0o7777, 0o600);
            drop(client);
            stop.send(true).unwrap();
            tokio::time::timeout(Duration::from_secs(5), &mut run)
                .await
                .unwrap();
            assert!(!manager.ready.load(Ordering::Acquire));
            assert_eq!(
                fs::symlink_metadata(&endpoint).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
            let after = fs::symlink_metadata(&legacy).unwrap();
            assert_eq!(
                (before.dev(), before.ino(), before.uid(), before.mode()),
                (after.dev(), after.ino(), after.uid(), after.mode())
            );
            previous = Some(endpoint);
        }
    }
    #[tokio::test]
    async fn manager_bind_collision_preserves_the_unproved_live_socket() {
        let root = Root::new();
        let config = root.config();
        let manager = Manager::new(&config);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(manager.base())
            .unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&manager.endpoint).unwrap();
        let before = fs::symlink_metadata(&manager.endpoint).unwrap();
        let (_stop, stopped) = watch::channel(false);
        tokio::time::timeout(
            Duration::from_secs(5),
            manager.clone().run(Weak::new(), stopped),
        )
        .await
        .unwrap();
        assert!(!manager.ready.load(Ordering::Acquire));
        let after = fs::symlink_metadata(&manager.endpoint).unwrap();
        assert_eq!(
            (before.dev(), before.ino(), before.uid(), before.mode()),
            (after.dev(), after.ino(), after.uid(), after.mode())
        );
        let _client = std::os::unix::net::UnixStream::connect(&manager.endpoint).unwrap();
        drop(listener);
    }
    #[tokio::test]
    async fn socket_drop_preserves_replacement_sockets_and_symlinks() {
        for symlink in [false, true] {
            let root = Root::new();
            let path = root.0.join("owned");
            let socket = SkillsSocket::bind(&path).unwrap();
            // Retain the original inode so replacement cannot pass through inode reuse.
            fs::rename(&path, root.0.join("original")).unwrap();
            let replacement = if symlink {
                root.0.join("replacement")
            } else {
                path.clone()
            };
            let listener = std::os::unix::net::UnixListener::bind(&replacement).unwrap();
            if symlink {
                std::os::unix::fs::symlink("replacement", &path).unwrap();
            }
            let before = fs::symlink_metadata(&path).unwrap();
            drop(socket);
            let after = fs::symlink_metadata(&path).unwrap();
            assert_eq!(
                (before.dev(), before.ino(), before.uid(), before.mode()),
                (after.dev(), after.ino(), after.uid(), after.mode())
            );
            assert_eq!(after.file_type().is_symlink(), symlink);
            let _client = std::os::unix::net::UnixStream::connect(&path).unwrap();
            drop(listener);
        }
    }
    #[test]
    fn selected_source_accepts_exact_public_bytes_but_rejects_changes_and_aliases() {
        let cleanup = Root::new();
        let root = &cleanup.0;
        let path = root.join("public.ts");
        fs::write(&path, b"public API").unwrap();
        let pin = || SourcePin {
            version: PACKAGE_VERSION.into(),
            files: vec![PinnedFile {
                path: "public.ts".into(),
                sha256: hex(&sha256(b"public API")),
                size: 10,
            }],
        };
        assert!(pin_files_selected(root, None, pin()).is_ok());
        fs::write(&path, b"changedAPI").unwrap();
        assert!(pin_files_selected(root, None, pin()).is_err());
        fs::write(&path, b"public API\n").unwrap();
        assert!(pin_files_selected(root, None, pin()).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(root.join("other.ts"), b"public API").unwrap();
        std::os::unix::fs::symlink("other.ts", &path).unwrap();
        assert!(pin_files_selected(root, None, pin()).is_err());
    }
}
