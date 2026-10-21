// SPDX-License-Identifier: GPL-3.0-or-later
//! Independently supervised same-binary Unix WebSocket relay; never launches a provider.
use crate::{
    config,
    herdr::{self, ProcessEvidence},
    native_json::{self, REGISTRY},
    pairing,
    protocol::*,
};
use futures_util::{Sink, SinkExt, StreamExt};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::RawValue;
use std::os::fd::{AsRawFd, FromRawFd};
use std::{
    borrow::Cow,
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, atomic::{AtomicBool, Ordering}},
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore, mpsc},
    task::JoinSet,
};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async_with_config, client_async_with_config,
    tungstenite::{
        Message,
        handshake::server::{Callback, ErrorResponse, Request, Response},
        http::StatusCode,
        protocol::WebSocketConfig,
    },
};

/// Drop this Linux thread's inherited capabilities before exec or worker creation.
#[cfg(target_os = "linux")]
pub fn drop_inherited_capabilities() -> io::Result<()> {
    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = CapHeader {
        version: 0x2008_0522,
        pid: 0,
    };
    let data = [
        CapData {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
        CapData {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
    ];
    // SAFETY: async-signal-safe syscalls with the Linux capability ABI v3 stack layout.
    unsafe {
        if libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
        if libc::syscall(libc::SYS_capset, &header as *const CapHeader, data.as_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// Stock 0.162.1 app-server-client remote.rs uses this finite ceiling for both.
pub const FRAME_BYTES: usize = 128 * 1024 * 1024;
pub const QUEUE_FRAMES: usize = 32;
pub const QUEUE_BYTES: usize = 2 * FRAME_BYTES;
pub const CORRELATIONS: usize = 64;
// Stock serde_json starts with128 and rejects the container that exhausts it.
const BRIDGE_JSON_DEPTH: usize = 128;
// Verified publisher sign-blob distribution, not independently reproduced source compilation.
pub const NATIVE_SHA256: &str = "628e35487d888a28f0427fc37501da529dd3d81fcbad7a2c3421c1e8670d92e1";
pub const NATIVE_BYTES: u64 = 294_415_048;
type ExecutableStamp = (u64, u64, u64, i64, i64, i64, i64, u32, u32, u32, u64);
struct DistributionProof {
    // A verified file-content fact, never cached process or namespace authority.
    path: PathBuf,
    file: Arc<File>,
    stamp: ExecutableStamp,
}
fn executable_stamp(m: &fs::Metadata) -> ExecutableStamp {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
        m.mode(),
        m.uid(),
        m.gid(),
        m.nlink(),
    )
}
fn distribution_content_hit(
    proofs: &parking_lot::Mutex<Vec<DistributionProof>>,
    path: &Path,
    captured: ExecutableStamp,
) -> bool {
    let held = {
        let proofs = proofs.lock();
        proofs
            .iter()
            .find(|proof| proof.path == path && proof.stamp == captured)
            .map(|proof| Arc::clone(&proof.file))
    };
    // No filesystem operation, including dropping the final held descriptor, holds the cache lock.
    held.is_some_and(|file| {
        file.metadata()
            .is_ok_and(|m| executable_stamp(&m) == captured)
    })
}
fn remember_distribution_content(
    proofs: &parking_lot::Mutex<Vec<DistributionProof>>,
    proof: DistributionProof,
) {
    let retired = {
        let mut proofs = proofs.lock();
        let replace = proofs
            .iter()
            .position(|old| old.path == proof.path && old.stamp == proof.stamp);
        let retired = replace
            .or_else(|| (proofs.len() >= 64).then_some(0))
            .map(|index| proofs.remove(index));
        proofs.push(proof);
        retired
    };
    drop(retired);
}
pub fn verify_distribution(process: &ProcessEvidence) -> io::Result<()> {
    use sha2::{Digest, Sha256};
    static PROOFS: std::sync::LazyLock<parking_lot::Mutex<Vec<DistributionProof>>> =
        std::sync::LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err(invalid(
            "No verified Codex publisher distribution for this platform",
        ));
    }
    if herdr::process_evidence(process.pid)? != *process {
        return Err(invalid("Codex process changed before executable proof"));
    }
    let proc = PathBuf::from(format!("/proc/{}/exe", process.pid));
    let path = fs::read_link(&proc)?;
    config::executable(&path)?;
    let mut file = File::open(&proc)?;
    let before = file.metadata()?;
    let captured = executable_stamp(&before);
    if before.len() != NATIVE_BYTES
        || before.mode() & 0o022 != 0
        || before.nlink() != 1
        || herdr::process_object(process, &path)? != (before.dev(), before.ino())
        || executable_stamp(&fs::metadata(&path)?) != captured
    {
        return Err(invalid(
            "Kernel-opened Codex executable/install/process-root objects disagree",
        ));
    }
    let cached = distribution_content_hit(&PROOFS, &path, captured);
    if !cached {
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        if magic != *b"\x7fELF" {
            return Err(invalid("Codex native executable is not ELF"));
        }
        let mut digest = Sha256::new();
        digest.update(magic);
        let mut bytes = [0; 65_536];
        loop {
            let read = file.read(&mut bytes)?;
            if read == 0 {
                break;
            }
            digest.update(&bytes[..read]);
        }
        if pairing::hex(&digest.finalize()) != NATIVE_SHA256 {
            return Err(invalid(
                "Codex executable differs from verified source-linked publisher distribution",
            ));
        }
    }
    if executable_stamp(&file.metadata()?) != captured
        || executable_stamp(&fs::metadata(&proc)?) != captured
        || executable_stamp(&fs::metadata(&path)?) != captured
        || herdr::process_object(process, &path)? != (before.dev(), before.ino())
        || herdr::process_evidence(process.pid)? != *process
    {
        return Err(invalid(
            "Codex executable/process changed during digest proof",
        ));
    }
    if !cached {
        remember_distribution_content(
            &PROOFS,
            DistributionProof {
                path,
                file: Arc::new(file),
                stamp: captured,
            },
        );
    }
    Ok(())
}
pub fn process_arguments(process: &ProcessEvidence) -> io::Result<Vec<String>> {
    if herdr::process_evidence(process.pid)? != *process {
        return Err(invalid("native argv process changed"));
    }
    let mut file = File::open(format!("/proc/{}/cmdline", process.pid))?;
    let mut bytes = Vec::new();
    (&mut file).take(65_537).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 65_536 || bytes.last() != Some(&0) {
        return Err(invalid("native argv unavailable or oversized"));
    }
    let argv = bytes[..bytes.len() - 1]
        .split(|b| *b == 0)
        .map(|b| {
            std::str::from_utf8(b)
                .map(str::to_owned)
                .map_err(|_| invalid("native argv is not UTF-8"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    if herdr::process_evidence(process.pid)? != *process {
        return Err(invalid("native argv process changed"));
    }
    Ok(argv)
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn decimal_u64(s: &str) -> io::Result<u64> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid("noncanonical native process number"));
    }
    s.parse()
        .map_err(|_| invalid("native process number overflow"))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SocketSource {
    root: [String; 2],
    host_parent: [String; 2],
    host_entry: [String; 2],
    agent_parent: [String; 2],
    alias: [String; 2],
    physical_parent: [String; 2],
}
impl SocketSource {
    fn pairs(&self) -> [&[String; 2]; 6] {
        [
            &self.root,
            &self.host_parent,
            &self.host_entry,
            &self.agent_parent,
            &self.alias,
            &self.physical_parent,
        ]
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OwnerEpoch {
    pub linux_boot_id: Uuid,
    pub pid: u32,
    pub start_ticks: String,
    pub socket_device: String,
    pub socket_inode: String,
    pub executable_sha256: String,
    pub socket_source: SocketSource,
}
impl OwnerEpoch {
    pub fn new(process: &ProcessEvidence, host: &Path, agent: &Path) -> io::Result<Self> {
        verify_distribution(process)?;
        let socket = OwnerSocket::capture_process(process, host, agent)?;
        let (device, inode) = socket.object()?;
        Ok(Self {
            linux_boot_id: process.linux_boot_id.clone(),
            pid: process.pid,
            start_ticks: process.start_ticks.to_string(),
            socket_device: device.to_string(),
            socket_inode: inode.to_string(),
            executable_sha256: NATIVE_SHA256.into(),
            socket_source: socket.source()?,
        })
    }
    pub fn validate(&self, host: &Path, agent: &Path) -> io::Result<ProcessEvidence> {
        self.linux_boot_id
            .validate()
            .map_err(|_| invalid("invalid boot identity"))?;
        let process = herdr::process_evidence(self.pid)?;
        if self.executable_sha256 != NATIVE_SHA256
            || process.linux_boot_id != self.linux_boot_id
            || process.start_ticks != decimal_u64(&self.start_ticks)?
        {
            return Err(invalid("Codex owner process/source replaced"));
        }
        verify_distribution(&process)?;
        OwnerSocket::capture_process(&process, host, agent)?.check_epoch(self)?;
        Ok(process)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HerdrBinding {
    pub server_id: Uuid,
    pub incarnation: Uuid,
    pub workspace_id: Id,
    pub terminal_id: Id,
    pub pane_id: Id,
    pub socket_path: PathBuf,
    pub evidence: crate::storage::ServerEvidence,
    #[serde(with = "process_record")]
    pub shell: ProcessEvidence,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Manifest {
    pub v: u32,
    pub binding_id: Uuid,
    pub owner_id: Uuid,
    pub upstream_host: PathBuf,
    pub upstream_agent: PathBuf,
    pub listen_host: PathBuf,
    pub listen_agent: PathBuf,
    pub owner_epoch: OwnerEpoch,
    pub herdr: HerdrBinding,
    pub expected_thread_id: Id,
    pub created_at: Uint,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Ready,
    Connected,
    Disconnected,
    TerminalClosed,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalProofAvailability {
    Available,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationModeKind {
    Default,
    Plan,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CollaborationModeObservation {
    pub mode: CollaborationModeKind,
    pub revision: u64,
    pub from_settings: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Presence {
    pub v: u32,
    pub binding_id: Uuid,
    pub owner_epoch: OwnerEpoch,
    #[serde(with = "process_record")]
    pub relay: ProcessEvidence,
    #[serde(with = "optional_process_record")]
    pub tui: Option<ProcessEvidence>,
    pub connection_epoch: Uuid,
    pub state: PresenceState,
    pub terminal_proof: TerminalProofAvailability,
    pub associated_thread_ids: Vec<Id>,
    pub resumed_thread_ids: Vec<Id>,
    pub resume_rejected_thread: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<CollaborationModeObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_default_plan: Option<bool>,
    pub updated_at: Uint,
    pub error: Option<SafeMessage>,
}
mod process_record {
    use super::*;
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    pub(super) struct Record {
        pub linux_boot_id: Uuid,
        pub pid: u32,
        pub start_ticks: String,
        pub namespaces: herdr::ProcessNamespaces,
    }
    impl From<&ProcessEvidence> for Record {
        fn from(p: &ProcessEvidence) -> Self {
            Self {
                linux_boot_id: p.linux_boot_id.clone(),
                pid: p.pid,
                start_ticks: p.start_ticks.to_string(),
                namespaces: p.namespaces.clone(),
            }
        }
    }
    impl Record {
        pub fn process<E: serde::de::Error>(self) -> Result<ProcessEvidence, E> {
            let start_ticks = decimal_u64(&self.start_ticks).map_err(E::custom)?;
            self.linux_boot_id.validate().map_err(E::custom)?;
            Ok(ProcessEvidence {
                linux_boot_id: self.linux_boot_id,
                pid: self.pid,
                start_ticks,
                namespaces: self.namespaces,
            })
        }
    }
    pub fn serialize<S: serde::Serializer>(p: &ProcessEvidence, s: S) -> Result<S::Ok, S::Error> {
        Record::from(p).serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<ProcessEvidence, D::Error> {
        Record::deserialize(d)?.process()
    }
}
mod optional_process_record {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        p: &Option<ProcessEvidence>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        p.as_ref().map(process_record::Record::from).serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Option<ProcessEvidence>, D::Error> {
        Option::<process_record::Record>::deserialize(d)?
            .map(|r| r.process())
            .transpose()
    }
}
pub fn manifest_path(dir: &Path, id: &Uuid) -> PathBuf {
    dir.join(format!("{}.json", id.0))
}
pub fn presence_path(path: &Path) -> PathBuf {
    path.with_extension("presence")
}
fn stamp(m: &fs::Metadata) -> (u64, u64, u64, i64, i64) {
    (m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec())
}
/// A bounded observation could not select a current publication; no bytes were accepted.
#[derive(Debug)]
pub(crate) struct PublicationUnstable;
impl std::fmt::Display for PublicationUnstable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private Codex publication did not stabilize")
    }
}
impl std::error::Error for PublicationUnstable {}
/// Classify only bounded publication churn, never a matching error message.
pub fn publication_unstable(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|error| error.is::<PublicationUnstable>())
}
pub fn private_read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    #[cfg(test)]
    {
        private_read_checked(path, |_| {})
    }
    #[cfg(not(test))]
    {
        private_read_checked(path)
    }
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrivateReadPhase {
    Opened,
    Read,
}
/// False means an otherwise valid publication was retired by an atomic replacement.
fn private_read_metadata(metadata: &fs::Metadata) -> io::Result<bool> {
    if !metadata.is_file()
        || metadata.uid() != pairing::uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() > 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe private path",
        ));
    }
    if metadata.len() > 65_536 {
        return Err(invalid("oversized or aliased private Codex file"));
    }
    Ok(metadata.nlink() == 1)
}
fn private_read_checked<T: DeserializeOwned>(
    path: &Path,
    #[cfg(test)] mut checkpoint: impl FnMut(PrivateReadPhase),
) -> io::Result<T> {
    config::absolute(path)?;
    pairing::private_dir(
        path.parent()
            .ok_or_else(|| invalid("private parent absent"))?,
        false,
    )?;
    for _ in 0..3 {
        let mut f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe private path")
                } else {
                    error
                }
            })?;
        #[cfg(test)]
        checkpoint(PrivateReadPhase::Opened);
        let before = f.metadata()?;
        // Check the opened object, not a pathname snapshot that rename can retire.
        if !private_read_metadata(&before)? {
            continue;
        }
        let mut bytes = Vec::new();
        (&mut f).take(65_537).read_to_end(&mut bytes)?;
        #[cfg(test)]
        checkpoint(PrivateReadPhase::Read);
        let after = f.metadata()?;
        let current = fs::symlink_metadata(path)?;
        let linked = private_read_metadata(&after)?;
        let current_linked = private_read_metadata(&current)?;
        if !linked
            || !current_linked
            || stamp(&before) != stamp(&after)
            || stamp(&before) != stamp(&current)
        {
            continue;
        }
        native_json::parse(&bytes, REGISTRY)?;
        return serde_json::from_slice(&bytes).map_err(|_| invalid("invalid private Codex record"));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        PublicationUnstable,
    ))
}
/// Immutable records use exclusive link admission; mutable presence uses matching-owner rename.
fn private_write_with_visibility<T: Serialize>(
    path: &Path,
    value: &T,
    replace: bool,
    before_visibility: impl FnOnce(&File) -> io::Result<()>,
    visible: impl FnOnce(),
) -> io::Result<()> {
    config::absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("private parent absent"))?;
    pairing::private_dir(parent, false)?;
    if replace {
        pairing::private_file(path)?;
        if fs::symlink_metadata(path)?.nlink() != 1 {
            return Err(invalid("aliased presence"));
        }
    }
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 65_536 {
        return Err(invalid("Codex private record exceeds64KiB"));
    }
    let temp = parent.join(format!(".{}.tmp", Uuid::random().0));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temp)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        before_visibility(&file)?;
        if replace {
            fs::rename(&temp, path)?;
        } else {
            fs::hard_link(&temp, path)?;
        }
        visible();
        if !replace {
            fs::remove_file(&temp)?;
        }
        File::open(parent)?.sync_all()
    })(); // Only the exclusively created temporary is ours.
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub fn private_write<T: Serialize>(path: &Path, value: &T, replace: bool) -> io::Result<()> {
    private_write_with_visibility(path, value, replace, |_| Ok(()), || {})
}
pub fn socket_object(path: &Path) -> io::Result<(u64, u64)> {
    config::socket_parent(path)?;
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_socket()
        || m.uid() != pairing::uid()
        || m.mode() & 0o7777 != 0o600
        || fs::canonicalize(path)? != path
    {
        return Err(invalid("unsafe Codex socket"));
    }
    Ok((m.dev(), m.ino()))
}
fn owned_listener_path(path: &Path, object: (u64, u64)) -> bool {
    config::socket_parent(path).is_ok()
        && fs::symlink_metadata(path).ok().is_some_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.uid() == pairing::uid()
                && (metadata.dev(), metadata.ino()) == object
                && fs::canonicalize(path).ok().as_deref() == Some(path)
        })
}
struct StartupListener {
    listener: Option<UnixListener>,
    path_file: File,
    path: PathBuf,
    object: (u64, u64),
}
impl StartupListener {
    fn bind(path: &Path) -> io::Result<Self> {
        let listener = UnixListener::bind(path)?;
        let path_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let object = file_object(&path_file)?;
        Ok(Self {
            listener: Some(listener),
            path_file,
            path: path.to_owned(),
            object,
        })
    }
    fn listener(&self) -> &UnixListener {
        self.listener.as_ref().expect("startup listener retained")
    }
    fn cleanup(&mut self) -> io::Result<()> {
        drop(self.listener.take());
        if file_object(&self.path_file).ok() == Some(self.object)
            && owned_listener_path(&self.path, self.object)
        {
            fs::remove_file(&self.path)?;
            File::open(
                self.path
                    .parent()
                    .ok_or_else(|| invalid("listen parent absent"))?,
            )?
            .sync_all()?;
        }
        Ok(())
    }
}
impl Drop for StartupListener {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn root_path(root: &File, path: &Path, flags: i32, resolve: u64) -> io::Result<File> {
    config::absolute(path)?;
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| invalid("invalid native socket pathname"))?;
    // SAFETY: Linux open_how is zero-initialized, including every reserved field.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (flags | libc::O_PATH | libc::O_CLOEXEC) as u64;
    how.resolve = libc::RESOLVE_IN_ROOT | libc::RESOLVE_NO_MAGICLINKS | resolve;
    // SAFETY: both descriptors and the complete open_how/pathname live across the syscall.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat2 returned a new descriptor exclusively owned here.
    Ok(unsafe { File::from_raw_fd(fd as libc::c_int) })
}
fn file_object(file: &File) -> io::Result<(u64, u64)> {
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
fn process_root(process: &ProcessEvidence) -> io::Result<File> {
    if herdr::process_evidence(process.pid)? != *process {
        return Err(invalid("owner process changed before root capture"));
    }
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(format!("/proc/{}/root", process.pid))?;
    if herdr::process_evidence(process.pid)? != *process {
        return Err(invalid("owner process changed during root capture"));
    }
    Ok(root)
}
fn private_socket_directory(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.uid() != pairing::uid() || metadata.mode() & 0o7777 != 0o700 {
        return Err(invalid("unsafe native socket directory"));
    }
    Ok(())
}
fn native_alias(file: &File, target: &Path) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_symlink()
        || metadata.uid() != pairing::uid()
        || metadata.mode() & 0o7777 != 0o777
        || metadata.nlink() != 1
    {
        return Err(invalid("unsafe native rendezvous alias"));
    }
    let mut bytes = [0u8; 108];
    // SAFETY: O_PATH|O_NOFOLLOW supports empty-path readlinkat; buffer is bounded.
    let length = unsafe {
        libc::readlinkat(
            file.as_raw_fd(),
            c"".as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if length < 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize >= bytes.len()
        || &bytes[..length as usize] != target.as_os_str().as_encoded_bytes()
    {
        return Err(invalid(
            "native rendezvous target is not the exact stock hash",
        ));
    }
    Ok(())
}
struct OwnerSocket {
    root: File,
    host: PathBuf,
    agent: PathBuf,
    host_parent: File,
    host_entry: File,
    agent_parent: File,
    alias: File,
    physical_parent: File,
    file: File,
}
impl OwnerSocket {
    fn capture_once(root: File, host: &Path, agent: &Path) -> io::Result<Self> {
        config::socket_parent(host)?;
        let physical = config::codex_socket_target(agent)?;
        let host_parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(
                host.parent()
                    .ok_or_else(|| invalid("host socket parent absent"))?,
            )?;
        private_socket_directory(&host_parent)?;
        let host_entry = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(host)?;
        let agent_parent = root_path(
            &root,
            agent
                .parent()
                .ok_or_else(|| invalid("agent socket parent absent"))?,
            libc::O_DIRECTORY,
            libc::RESOLVE_NO_SYMLINKS,
        )?;
        private_socket_directory(&agent_parent)?;
        let alias = root_path(&root, agent, libc::O_NOFOLLOW, libc::RESOLVE_NO_SYMLINKS)?;
        native_alias(&alias, &physical)?;
        let physical_parent = root_path(
            &root,
            physical
                .parent()
                .ok_or_else(|| invalid("physical socket parent absent"))?,
            libc::O_DIRECTORY,
            libc::RESOLVE_NO_SYMLINKS,
        )?;
        private_socket_directory(&physical_parent)?;
        let file = root_path(
            &root,
            &physical,
            libc::O_NOFOLLOW,
            libc::RESOLVE_NO_SYMLINKS,
        )?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != pairing::uid()
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(invalid("unsafe native physical socket"));
        }
        let host_metadata = host_entry.metadata()?;
        if host_metadata.file_type().is_symlink() {
            native_alias(&host_entry, &physical)?;
            if file_object(&host_parent)? != file_object(&agent_parent)?
                || file_object(&host_entry)? != file_object(&alias)?
            {
                return Err(invalid("advertised host/provider alias objects disagree"));
            }
        } else if socket_object(host)? != file_object(&file)?
            || file_object(&host_entry)? != file_object(&file)?
        {
            return Err(invalid("explicit host physical socket mapping disagrees"));
        }
        Ok(Self {
            root,
            host: host.to_owned(),
            agent: agent.to_owned(),
            host_parent,
            host_entry,
            agent_parent,
            alias,
            physical_parent,
            file,
        })
    }
    fn capture(root: File, host: &Path, agent: &Path) -> io::Result<Self> {
        let socket = Self::capture_once(root, host, agent)?;
        socket.recheck()?;
        Ok(socket)
    }
    fn capture_process(process: &ProcessEvidence, host: &Path, agent: &Path) -> io::Result<Self> {
        let socket = Self::capture(process_root(process)?, host, agent)?;
        socket.check_process(process)?;
        Ok(socket)
    }
    fn object(&self) -> io::Result<(u64, u64)> {
        file_object(&self.file)
    }
    fn stamp(&self) -> io::Result<[(u64, u64); 6]> {
        Ok([
            file_object(&self.root)?,
            file_object(&self.host_parent)?,
            file_object(&self.host_entry)?,
            file_object(&self.agent_parent)?,
            file_object(&self.alias)?,
            file_object(&self.physical_parent)?,
        ])
    }
    fn source(&self) -> io::Result<SocketSource> {
        let [
            root,
            host_parent,
            host_entry,
            agent_parent,
            alias,
            physical_parent,
        ] = self
            .stamp()?
            .map(|(device, inode)| [device.to_string(), inode.to_string()]);
        Ok(SocketSource {
            root,
            host_parent,
            host_entry,
            agent_parent,
            alias,
            physical_parent,
        })
    }
    fn check_epoch(&self, epoch: &OwnerEpoch) -> io::Result<()> {
        if self.object()?
            != (
                decimal_u64(&epoch.socket_device)?,
                decimal_u64(&epoch.socket_inode)?,
            )
        {
            return Err(invalid("native physical socket epoch replaced"));
        }
        self.check_source(&epoch.socket_source)
    }
    fn check_source(&self, source: &SocketSource) -> io::Result<()> {
        for (expected, actual) in source.pairs().into_iter().zip(self.stamp()?) {
            if actual != (decimal_u64(&expected[0])?, decimal_u64(&expected[1])?) {
                return Err(invalid("native alias/root/parent source epoch replaced"));
            }
        }
        Ok(())
    }
    fn recheck(&self) -> io::Result<()> {
        let current = Self::capture_once(self.root.try_clone()?, &self.host, &self.agent)?;
        if current.stamp()? != self.stamp()? || current.object()? != self.object()? {
            return Err(invalid("owner alias/root/socket source changed"));
        }
        Ok(())
    }
    fn check_process(&self, process: &ProcessEvidence) -> io::Result<()> {
        if file_object(&process_root(process)?)? != file_object(&self.root)? {
            return Err(invalid("owner process root changed"));
        }
        self.recheck()
    }
    async fn connect(&self) -> io::Result<UnixStream> {
        self.recheck()?;
        let path = format!("/proc/self/fd/{}", self.file.as_raw_fd());
        config::socket_path(Path::new(&path))?;
        let stream =
            tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(path)).await??;
        self.recheck()?;
        Ok(stream)
    }
}
pub fn websocket_config(frame_bytes: usize, queue_bytes: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(frame_bytes))
        .max_frame_size(Some(frame_bytes))
        .write_buffer_size(0)
        .max_write_buffer_size(queue_bytes)
}
pub async fn owner_stream(
    host: &Path,
    agent: &Path,
    epoch: &OwnerEpoch,
    websocket: WebSocketConfig,
) -> io::Result<(WebSocketStream<UnixStream>, ProcessEvidence)> {
    let process = epoch.validate(host, agent)?;
    let observed = OwnerSocket::capture_process(&process, host, agent)?;
    observed.check_epoch(epoch)?;
    let stream = observed.connect().await?;
    let credentials = stream.peer_cred()?;
    if credentials.uid() != pairing::uid()
        || credentials.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(process.pid)
    {
        return Err(invalid("native socket peer is not the proven owner"));
    }
    observed.check_process(&process)?;
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(5),
        client_async_with_config("ws://localhost/", stream, Some(websocket)),
    )
    .await?
    .map_err(|_| invalid("Codex Unix WebSocket handshake failed"))?;
    observed.check_process(&process)?;
    if epoch.validate(host, agent)? != process {
        return Err(invalid("Codex owner changed during handshake"));
    }
    Ok((socket, process))
}
fn startup_timeout() -> io::Error {
    invalid("relay readiness exceeded10s")
}
// This process has one relay invocation.  The latch preserves the original error
// for the caller while allowing `main` to avoid duplicating its raw text after a
// closed diagnostic line was already committed to the journal.
static STARTUP_DIAGNOSTIC_REPORTED: AtomicBool = AtomicBool::new(false);

pub fn take_startup_diagnostic_reported() -> bool {
    STARTUP_DIAGNOSTIC_REPORTED.swap(false, Ordering::AcqRel)
}

struct StartupDiagnostic {
    armed: bool,
}
impl StartupDiagnostic {
    fn new(armed: bool) -> Self {
        if armed {
            STARTUP_DIAGNOSTIC_REPORTED.store(false, Ordering::Release);
        }
        Self { armed }
    }
    fn observe<T>(&self, guard: &'static str, result: io::Result<T>) -> io::Result<T> {
        if let Err(error) = &result {
            self.failure(guard, error);
        }
        result
    }
    fn failure(&self, guard: &'static str, error: &io::Error) {
        if !self.armed
            || STARTUP_DIAGNOSTIC_REPORTED
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let io = match error.kind() {
            io::ErrorKind::NotFound => "not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::ConnectionRefused => "connection_refused",
            io::ErrorKind::ConnectionReset => "connection_reset",
            io::ErrorKind::ConnectionAborted => "connection_aborted",
            io::ErrorKind::NotConnected => "not_connected",
            io::ErrorKind::BrokenPipe => "broken_pipe",
            io::ErrorKind::TimedOut => "timed_out",
            io::ErrorKind::UnexpectedEof => "unexpected_eof",
            io::ErrorKind::InvalidData => "invalid_data",
            _ => "other",
        };
        let errno = error
            .raw_os_error()
            .filter(|value| (-4095..=4095).contains(value))
            .unwrap_or(0);
        let _ = writeln!(
            io::stderr().lock(),
            "goatr_codex_first_guard phase=relay_startup guard={guard} code=none io={io} errno={errno}"
        );
    }
}
fn ensure_startup_live(deadline: tokio::time::Instant) -> io::Result<()> {
    if tokio::time::Instant::now() >= deadline {
        return Err(startup_timeout());
    }
    Ok(())
}
async fn startup_blocking_proof<T: Send + 'static>(
    deadline: tokio::time::Instant,
    proof: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    ensure_startup_live(deadline)?;
    let result = tokio::time::timeout_at(deadline, tokio::task::spawn_blocking(proof))
        .await
        .map_err(|_| startup_timeout())?
        .map_err(|_| invalid("Owner/manifest source proof worker failed"))?;
    ensure_startup_live(deadline)?;
    result
}
async fn startup_owner_stream(
    manifest: &Manifest,
    deadline: tokio::time::Instant,
    capture: &StartupDiagnostic,
) -> io::Result<(WebSocketStream<UnixStream>, ProcessEvidence)> {
    let host = manifest.upstream_host.clone();
    let agent = manifest.upstream_agent.clone();
    let epoch = manifest.owner_epoch.clone();
    let process = capture.observe("owner_epoch", startup_blocking_proof(deadline, move || epoch.validate(&host, &agent)).await)?;
    let host = manifest.upstream_host.clone();
    let agent = manifest.upstream_agent.clone();
    let epoch = manifest.owner_epoch.clone();
    let captured = process.clone();
    let observed = capture.observe("owner_socket", startup_blocking_proof(deadline, move || {
        let observed = OwnerSocket::capture_process(&captured, &host, &agent)?;
        observed.check_epoch(&epoch)?;
        Ok(observed)
    })
    .await)?;
    let observed = capture.observe("owner_revalidation", startup_blocking_proof(deadline, move || {
        observed.recheck()?;
        Ok(observed)
    })
    .await)?;
    let path = format!("/proc/self/fd/{}", observed.file.as_raw_fd());
    capture.observe("owner_socket", config::socket_path(Path::new(&path)))?;
    capture.observe("owner_connect", ensure_startup_live(deadline))?;
    let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let startup_bounds_connect = deadline <= connect_deadline;
    let stream = match tokio::time::timeout_at(
        if startup_bounds_connect {
            deadline
        } else {
            connect_deadline
        },
        UnixStream::connect(path),
    )
    .await {
        Ok(result) => result,
        Err(error) => Err(if startup_bounds_connect {
            startup_timeout()
        } else {
            error.into()
        }),
    };
    let stream = capture.observe("owner_connect", stream)?;
    let observed = capture.observe("owner_revalidation", startup_blocking_proof(deadline, move || {
        observed.recheck()?;
        Ok(observed)
    })
    .await)?;
    let credentials = capture.observe("owner_credentials", stream.peer_cred())?;
    if credentials.uid() != pairing::uid()
        || credentials.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(process.pid)
    {
        return capture.observe("owner_credentials", Err(invalid("native socket peer is not the proven owner")));
    }
    let checked = process.clone();
    let observed = capture.observe("owner_revalidation", startup_blocking_proof(deadline, move || {
        observed.check_process(&checked)?;
        Ok(observed)
    })
    .await)?;
    capture.observe("owner_handshake", ensure_startup_live(deadline))?;
    let handshake_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let startup_bounds_handshake = deadline <= handshake_deadline;
    let socket = match tokio::time::timeout_at(
        if startup_bounds_handshake {
            deadline
        } else {
            handshake_deadline
        },
        client_async_with_config(
            "ws://localhost/",
            stream,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        ),
    )
    .await {
        Ok(result) => result
            .map(|(socket, _)| socket)
            .map_err(|_| invalid("Codex Unix WebSocket handshake failed")),
        Err(error) => Err(if startup_bounds_handshake {
            startup_timeout()
        } else {
            error.into()
        }),
    };
    let socket = capture.observe("owner_handshake", socket)?;
    let checked = process.clone();
    let _observed = capture.observe("owner_revalidation", startup_blocking_proof(deadline, move || {
        observed.check_process(&checked)?;
        Ok(observed)
    })
    .await)?;
    let host = manifest.upstream_host.clone();
    let agent = manifest.upstream_agent.clone();
    let epoch = manifest.owner_epoch.clone();
    if capture.observe("owner_epoch", startup_blocking_proof(deadline, move || epoch.validate(&host, &agent)).await)? != process {
        return capture.observe("owner_epoch", Err(invalid("Codex owner changed during handshake")));
    }
    Ok((socket, process))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresencePhase {
    Guard,
    PresenceRecord,
    RelayProcess,
    TuiProcess,
    TuiDistribution,
    TuiAgentSocket,
    HostListenSocket,
    ListenParent,
    OwnerProof,
    HostParent,
    TerminalProcess,
    TerminalParentObject,
}
impl PresencePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Guard => "guard",
            Self::PresenceRecord => "presence_record",
            Self::RelayProcess => "relay_process",
            Self::TuiProcess => "tui_process",
            Self::TuiDistribution => "tui_distribution",
            Self::TuiAgentSocket => "tui_agent_socket",
            Self::HostListenSocket => "host_listen_socket",
            Self::ListenParent => "listen_parent",
            Self::OwnerProof => "owner_proof",
            Self::HostParent => "host_parent",
            Self::TerminalProcess => "terminal_process",
            Self::TerminalParentObject => "terminal_parent_object",
        }
    }
}
pub(crate) struct PresenceFailure {
    pub(crate) phase: PresencePhase,
    pub(crate) error: io::Error,
}
pub enum ValidatedPresence {
    Fresh(Presence),
    Past(Presence),
    Unavailable(Presence),
}
pub(crate) fn freshness_at_completion(updated_at: u64, now: u64) -> Result<bool, PresenceFailure> {
    if updated_at > now.saturating_add(1_000) {
        return Err(invalid("Codex relay presence more than one second ahead").into());
    }
    Ok(now.saturating_sub(updated_at) <= 5_000)
}
impl PresenceFailure {
    fn at(phase: PresencePhase, error: io::Error) -> Self {
        Self { phase, error }
    }
}
impl From<io::Error> for PresenceFailure {
    fn from(error: io::Error) -> Self {
        Self::at(PresencePhase::Guard, error)
    }
}
impl Manifest {
    pub fn load(path: &Path) -> io::Result<Self> {
        let m: Self = private_read(path)?;
        m.validate()?;
        if path.file_name().and_then(|n| n.to_str())
            != Some(format!("{}.json", m.binding_id.0).as_str())
        {
            return Err(invalid("binding filename/UUID mismatch"));
        }
        Ok(m)
    }
    pub fn validate(&self) -> io::Result<()> {
        self.validate_checked().map_err(|failure| failure.error)
    }
    fn validate_checked(&self) -> Result<(), PresenceFailure> {
        if self.v != 1 {
            return Err(invalid("unknown Codex binding version").into());
        }
        self.binding_id
            .validate()
            .map_err(|_| invalid("invalid binding UUID"))?;
        self.owner_id
            .validate()
            .map_err(|_| invalid("invalid owner UUID"))?;
        self.expected_thread_id
            .validate()
            .map_err(|_| invalid("invalid thread ID"))?;
        for path in [
            &self.upstream_host,
            &self.upstream_agent,
            &self.listen_host,
            &self.listen_agent,
        ] {
            config::socket_path(path)?;
        }
        config::socket_parent(&self.listen_host)
            .map_err(|error| PresenceFailure::at(PresencePhase::ListenParent, error))?;
        self.herdr
            .server_id
            .validate()
            .map_err(|_| invalid("invalid server ID"))?;
        self.herdr
            .incarnation
            .validate()
            .map_err(|_| invalid("invalid incarnation"))?;
        for id in [
            &self.herdr.workspace_id,
            &self.herdr.terminal_id,
            &self.herdr.pane_id,
        ] {
            id.validate()
                .map_err(|_| invalid("invalid Herdr binding ID"))?;
        }
        self.owner_epoch
            .validate(&self.upstream_host, &self.upstream_agent)
            .map_err(|error| PresenceFailure::at(PresencePhase::OwnerProof, error))?;
        let parent = self
            .listen_host
            .parent()
            .ok_or_else(|| invalid("listen parent absent"))?;
        let agent_parent = self
            .listen_agent
            .parent()
            .ok_or_else(|| invalid("listen parent absent"))?;
        let p = fs::metadata(parent)
            .map_err(|error| PresenceFailure::at(PresencePhase::HostParent, error))?;
        if herdr::process_evidence(self.herdr.shell.pid)
            .map_err(|error| PresenceFailure::at(PresencePhase::TerminalProcess, error))?
            != self.herdr.shell
            || herdr::process_object(&self.herdr.shell, agent_parent)
                .map_err(|error| PresenceFailure::at(PresencePhase::TerminalParentObject, error))?
                != (p.dev(), p.ino())
        {
            return Err(invalid("relay host/provider parent object proof unavailable").into());
        }
        Ok(())
    }
    pub fn presence(&self, path: &Path) -> io::Result<Presence> {
        self.presence_checked(path).map_err(|failure| failure.error)
    }
    /// A past record provides comparison metadata only, never readiness authority.
    pub fn presence_observation(&self, path: &Path) -> io::Result<ValidatedPresence> {
        self.validated_presence(path)
            .map_err(|failure| failure.error)
    }
    pub(crate) fn presence_checked(&self, path: &Path) -> Result<Presence, PresenceFailure> {
        match self.validated_presence(path)? {
            ValidatedPresence::Fresh(presence) => Ok(presence),
            ValidatedPresence::Past(_) => {
                Err(invalid("Codex relay presence older than five seconds").into())
            }
            ValidatedPresence::Unavailable(_) => {
                Err(invalid("Codex relay terminal proof unavailable").into())
            }
        }
    }
    pub(crate) fn validated_presence(
        &self,
        path: &Path,
    ) -> Result<ValidatedPresence, PresenceFailure> {
        let p: Presence = private_read(&presence_path(path))
            .map_err(|error| PresenceFailure::at(PresencePhase::PresenceRecord, error))?;
        if p.v != 1
            || p.binding_id != self.binding_id
            || p.owner_epoch != self.owner_epoch
            || herdr::process_evidence(p.relay.pid)
                .map_err(|error| PresenceFailure::at(PresencePhase::RelayProcess, error))?
                != p.relay
        {
            return Err(invalid("Codex relay presence identity changed").into());
        }
        // A future record is invalid at observation even if a slow proof lets
        // wall time catch up before the completed-proof freshness decision.
        if p.updated_at.0 > pairing::now_ms().saturating_add(1_000) {
            return Err(invalid("Codex relay presence more than one second ahead").into());
        }
        p.connection_epoch
            .validate()
            .map_err(|_| invalid("invalid relay connection epoch"))?;
        if p.associated_thread_ids.len() > 64 || p.resumed_thread_ids.len() > 64 {
            return Err(invalid("relay association limit").into());
        }
        let mut unique = HashSet::new();
        for t in &p.associated_thread_ids {
            t.validate()
                .map_err(|_| invalid("invalid associated thread"))?;
            if !unique.insert(t) {
                return Err(invalid("duplicate associated thread").into());
            }
        }
        if p.state == PresenceState::Connected {
            let tui = p
                .tui
                .as_ref()
                .ok_or_else(|| invalid("TUI process proof absent"))?;
            if herdr::process_evidence(tui.pid)
                .map_err(|error| PresenceFailure::at(PresencePhase::TuiProcess, error))?
                != *tui
            {
                return Err(invalid("TUI process replaced").into());
            }
            verify_distribution(tui)
                .map_err(|error| PresenceFailure::at(PresencePhase::TuiDistribution, error))?;
            if herdr::process_object(tui, &self.listen_agent)
                .map_err(|error| PresenceFailure::at(PresencePhase::TuiAgentSocket, error))?
                != socket_object(&self.listen_host)
                    .map_err(|error| PresenceFailure::at(PresencePhase::HostListenSocket, error))?
            {
                return Err(invalid("TUI relay socket object differs").into());
            }
        } else if !p.associated_thread_ids.is_empty() || !p.resumed_thread_ids.is_empty() {
            return Err(invalid("disconnected relay advertises associations").into());
        }
        self.validate_checked()?;
        // Freshness is a decision after every physical proof, never a timestamp captured
        // before a slow executable, socket, or source check.
        let fresh = freshness_at_completion(p.updated_at.0, pairing::now_ms())?;
        if p.terminal_proof == TerminalProofAvailability::Unavailable {
            Ok(ValidatedPresence::Unavailable(p))
        } else if fresh {
            Ok(ValidatedPresence::Fresh(p))
        } else {
            Ok(ValidatedPresence::Past(p))
        }
    }
    /// Only a same-source successful snapshot may establish actual terminal closure.
    pub async fn terminal(&self, tui: Option<&ProcessEvidence>) -> io::Result<bool> {
        self.terminal_classified(tui)
            .await
            .map_err(TerminalFailure::into_error)
    }
    async fn terminal_classified(
        &self,
        tui: Option<&ProcessEvidence>,
    ) -> Result<bool, TerminalFailure> {
        let c = herdr::Client::connect(&self.herdr.socket_path)
            .await
            .map_err(|error| TerminalFailure::new("connect", error))?;
        if c.evidence != self.herdr.evidence {
            return Err(TerminalFailure::invalid(
                "snapshot_source",
                "Herdr binding source replaced",
            ));
        }
        let (e, s) = c
            .snapshot()
            .await
            .map_err(|error| TerminalFailure::new("snapshot", error))?;
        if e != self.herdr.evidence {
            return Err(TerminalFailure::invalid(
                "snapshot_source",
                "Herdr binding source replaced",
            ));
        }
        let Some(p) = s
            .panes
            .iter()
            .find(|p| p.terminal_id == self.herdr.terminal_id)
        else {
            return Ok(false);
        };
        if let Some(tui) = tui {
            let reporter = herdr::process_evidence(e.pid)
                .map_err(|error| TerminalFailure::new("reporter_process", error))?;
            let c = herdr::Client::connect(&self.herdr.socket_path)
                .await
                .map_err(|error| TerminalFailure::new("process_info_connect", error))?;
            if c.evidence != e {
                return Err(TerminalFailure::invalid(
                    "process_info_source",
                    "Herdr reporter replaced",
                ));
            }
            let info = c
                .process_info(&p.pane_id)
                .await
                .map_err(|error| TerminalFailure::new("process_info", error))?;
            let proof = tui.clone();
            tokio::task::spawn_blocking(move || verify_distribution(&proof))
                .await
                .map_err(|_| {
                    TerminalFailure::invalid(
                        "tui_distribution",
                        "TUI publisher proof worker failed",
                    )
                })?
                .map_err(|error| TerminalFailure::new("tui_distribution", error))?;
            let argv =
                process_arguments(tui).map_err(|error| TerminalFailure::new("tui_argv", error))?;
            let expected = [
                "--remote".to_owned(),
                format!("unix://{}", self.listen_agent.display()),
                "resume".into(),
                self.expected_thread_id.0.clone(),
            ];
            if argv.iter().filter(|arg| arg.as_str() == "--remote").count() != 1
                || !argv
                    .windows(expected.len())
                    .any(|args| args == expected.as_slice())
            {
                return Err(TerminalFailure::invalid(
                    "tui_argv",
                    "TUI kernel argv is not the exact bound remote resume",
                ));
            }
            if !info
                .foreground_processes
                .iter()
                .any(|p| herdr::reported_pid_matches(&reporter, tui, p.pid))
            {
                return Err(TerminalFailure::invalid(
                    "tui_foreground",
                    "exact TUI is not in bound terminal",
                ));
            }
        }
        Ok(true)
    }
}
struct TerminalFailure {
    phase: &'static str,
    error: io::Error,
}
impl TerminalFailure {
    fn new(phase: &'static str, error: io::Error) -> Self {
        Self { phase, error }
    }
    fn invalid(phase: &'static str, message: &'static str) -> Self {
        Self::new(phase, invalid(message))
    }
    fn into_error(self) -> io::Error {
        self.error
    }
}
struct ForwardFailure {
    error: &'static str,
    terminal_capture: Option<(&'static str, TerminalCapture)>,
    closed_capture: Option<SafeMessage>,
}
impl ForwardFailure {
    const fn plain(error: &'static str) -> Self {
        Self {
            error,
            terminal_capture: None,
            closed_capture: None,
        }
    }
    fn terminal(failure: TerminalFailure) -> Self {
        let error = terminal_failure(&failure.error);
        Self {
            error,
            terminal_capture: Some((error, terminal_capture(&failure))),
            closed_capture: None,
        }
    }
    fn closed(close: ForwardClose, capture: ClosedCapture) -> Self {
        let error = close.capture_error();
        Self {
            error,
            terminal_capture: None,
            closed_capture: Some(closed_capture_message(close, capture)),
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum ForwardClose {
    TuiEof,
    TuiClose,
    OwnerEof,
    OwnerClose,
    OwnerQueueDrained,
    TuiQueueDrained,
}
impl ForwardClose {
    const fn capture_error(self) -> &'static str {
        match self {
            Self::TuiEof => "Codex relay closed source=tui_read closed=tui_eof",
            Self::TuiClose => "Codex relay closed source=tui_read closed=tui_close",
            Self::OwnerEof => "Codex relay closed source=owner_read closed=owner_eof",
            Self::OwnerClose => "Codex relay closed source=owner_read closed=owner_close",
            Self::OwnerQueueDrained => {
                "Codex relay closed source=owner_queue closed=queue_drained"
            }
            Self::TuiQueueDrained => "Codex relay closed source=tui_queue closed=queue_drained",
        }
    }
}
const CLOSED_CAPTURE_MAX_BYTES: usize =
    "Codex relay closed source=owner_queue closed=queue_drained".len()
    + " lookup_seen=1".len()
    + " lookup_send_completed=1".len()
    + " lookup_reply=invalid_shape".len()
    + " reply_send_completed=1".len()
    + " account_seen=1".len()
    + " account_send_completed=1".len()
    + " account_reply=result".len()
    + " account_shape=result_invalid".len()
    + " auth_required=unobserved".len()
    + " account_reply_send_completed=1".len()
    + " bootstrap_model_list=1f".len()
    + " bootstrap_config_requirements=1f".len()
    + " resume_seen=1".len()
    + " first_error_family=bootstrap_config_requirements".len();
const _: () = assert!(CLOSED_CAPTURE_MAX_BYTES < 512);
fn closed_capture_message(close: ForwardClose, capture: ClosedCapture) -> SafeMessage {
    let annotation = format!(
        "{} lookup_seen={} lookup_send_completed={} lookup_reply={} reply_send_completed={} account_seen={} account_send_completed={} account_reply={} account_shape={} auth_required={} account_reply_send_completed={} resume_seen={} first_error_family={} bootstrap_model_list={:02x} bootstrap_config_requirements={:02x}",
        close.capture_error(),
        u8::from(capture.lookup_seen),
        u8::from(capture.lookup_send_completed),
        capture.lookup_reply.as_str(),
        u8::from(capture.reply_send_completed),
        u8::from(capture.account_seen),
        u8::from(capture.account_send_completed),
        capture.account_reply.as_str(),
        capture.account_shape.as_str(),
        capture.auth_required.as_str(),
        u8::from(capture.account_reply_send_completed),
        u8::from(capture.resume_seen),
        capture.first_error_family.as_str(),
        capture.bootstrap_model_list,
        capture.bootstrap_config_requirements,
    );
    debug_assert!(annotation.len() <= CLOSED_CAPTURE_MAX_BYTES);
    SafeMessage(annotation)
}
// Metadata maps retain only borrowed values; opaque payloads never become a Value tree.
#[derive(Default)]
struct NativeMetadata<'a> {
    id: Option<&'a RawValue>,
    method: Option<&'a RawValue>,
    params: Option<&'a RawValue>,
    result: Option<&'a RawValue>,
    error: Option<&'a RawValue>,
    thread_id: Option<&'a RawValue>,
    thread: Option<&'a RawValue>,
    status: Option<&'a RawValue>,
    code: Option<&'a RawValue>,
    message: Option<&'a RawValue>,
    collaboration_mode: Option<&'a RawValue>,
    thread_settings: Option<&'a RawValue>,
    include_turns: Option<&'a RawValue>,
    mode: Option<&'a RawValue>,
    data: Option<&'a RawValue>,
    account: Option<&'a RawValue>,
    account_type: Option<&'a RawValue>,
    requires_openai_auth: Option<&'a RawValue>,
}
struct BorrowedText<'a>(Cow<'a, str>);
impl<'de> Deserialize<'de> for BorrowedText<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor;
        impl<'de> serde::de::Visitor<'de> for TextVisitor {
            type Value = BorrowedText<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a native metadata string")
            }
            fn visit_borrowed_str<E: serde::de::Error>(
                self,
                value: &'de str,
            ) -> Result<Self::Value, E> {
                Ok(BorrowedText(Cow::Borrowed(value)))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(BorrowedText(Cow::Owned(value.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(BorrowedText(Cow::Owned(value)))
            }
        }
        deserializer.deserialize_str(TextVisitor)
    }
}
impl<'de> Deserialize<'de> for NativeMetadata<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MetadataVisitor;
        impl<'de> serde::de::Visitor<'de> for MetadataVisitor {
            type Value = NativeMetadata<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a native metadata object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut metadata = NativeMetadata::default();
                let mut seen = HashSet::new();
                while let Some(BorrowedText(key)) = map.next_key::<BorrowedText<'de>>()? {
                    let slot = match key.as_ref() {
                        "id" => Some(&mut metadata.id),
                        "method" => Some(&mut metadata.method),
                        "params" => Some(&mut metadata.params),
                        "result" => Some(&mut metadata.result),
                        "error" => Some(&mut metadata.error),
                        "threadId" => Some(&mut metadata.thread_id),
                        "thread" => Some(&mut metadata.thread),
                        "status" => Some(&mut metadata.status),
                        "code" => Some(&mut metadata.code),
                        "message" => Some(&mut metadata.message),
                        "collaborationMode" => Some(&mut metadata.collaboration_mode),
                        "threadSettings" => Some(&mut metadata.thread_settings),
                        "includeTurns" => Some(&mut metadata.include_turns),
                        "mode" => Some(&mut metadata.mode),
                        "data" => Some(&mut metadata.data),
                        "account" => Some(&mut metadata.account),
                        "type" => Some(&mut metadata.account_type),
                        "requiresOpenaiAuth" => Some(&mut metadata.requires_openai_auth),
                        _ => None,
                    };
                    if !seen.insert(key) {
                        return Err(serde::de::Error::custom("duplicate native metadata key"));
                    }
                    let value: &'de RawValue = map.next_value()?;
                    if let Some(slot) = slot {
                        *slot = Some(value); // Explicit null is present, not a missing member.
                    }
                }
                Ok(metadata)
            }
        }
        deserializer.deserialize_map(MetadataVisitor)
    }
}
fn escape_unit(bytes: &[u8]) -> io::Result<u16> {
    if bytes.len() != 4 {
        return Err(invalid("invalid native JSON escape"));
    }
    let mut unit = 0_u16;
    for byte in bytes {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(invalid("invalid native JSON escape")),
        };
        unit = (unit << 4) | u16::from(digit);
    }
    Ok(unit)
}
fn bridge_metadata(bytes: &[u8]) -> io::Result<NativeMetadata<'_>> {
    if bytes.is_empty() || bytes.len() > FRAME_BYTES {
        return Err(invalid("native bridge JSON byte limit"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("invalid native JSON UTF-8"))?;
    // RawValue ignores opaque values without enforcing recursion or surrogate pairing.
    // This allocation-free lexical guard fills those gaps; serde checks the JSON grammar.
    let mut at = 0;
    let mut depth = 0_usize;
    let mut string = false;
    while let Some(&byte) = bytes.get(at) {
        if string {
            match byte {
                b'"' => string = false,
                b'\\' => {
                    match bytes
                        .get(at + 1)
                        .copied()
                        .ok_or_else(|| invalid("invalid native JSON escape"))?
                    {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => at += 2,
                        b'u' => {
                            let unit = escape_unit(
                                bytes
                                    .get(at + 2..at + 6)
                                    .ok_or_else(|| invalid("invalid native JSON escape"))?,
                            )?;
                            if (0xd800..=0xdbff).contains(&unit) {
                                if bytes.get(at + 6..at + 8) != Some(b"\\u") {
                                    return Err(invalid("unpaired native JSON surrogate"));
                                }
                                let low =
                                    escape_unit(bytes.get(at + 8..at + 12).ok_or_else(|| {
                                        invalid("unpaired native JSON surrogate")
                                    })?)?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(invalid("unpaired native JSON surrogate"));
                                }
                                at += 12;
                            } else {
                                if (0xdc00..=0xdfff).contains(&unit) {
                                    return Err(invalid("unpaired native JSON surrogate"));
                                }
                                at += 6;
                            }
                        }
                        _ => return Err(invalid("invalid native JSON escape")),
                    }
                    continue;
                }
                0..=31 => return Err(invalid("unescaped native JSON control")),
                _ => {}
            }
        } else {
            match byte {
                b'"' => string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth >= BRIDGE_JSON_DEPTH {
                        return Err(invalid("native JSON depth limit"));
                    }
                }
                b'}' | b']' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("invalid native JSON nesting"))?;
                }
                _ => {}
            }
        }
        at += 1;
    }
    if string || depth != 0 {
        return Err(invalid("incomplete native JSON"));
    }
    metadata_object(text)
}
fn metadata_object(text: &str) -> io::Result<NativeMetadata<'_>> {
    if text
        .bytes()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        != Some(b'{')
    {
        return Err(invalid("native metadata must be an object"));
    }
    serde_json::from_str(text).map_err(|_| invalid("invalid native metadata object"))
}
fn metadata_string(value: &RawValue) -> io::Result<Cow<'_, str>> {
    if !value.get().starts_with('"') {
        return Err(invalid("native metadata must be a string"));
    }
    serde_json::from_str::<BorrowedText<'_>>(value.get())
        .map(|value| value.0)
        .map_err(|_| invalid("invalid native metadata string"))
}
fn observed_mode(value: &RawValue) -> Option<CollaborationModeKind> {
    let metadata = metadata_object(value.get()).ok()?;
    match metadata_string(metadata.mode?).ok()?.as_ref() {
        "default" => Some(CollaborationModeKind::Default),
        "plan" => Some(CollaborationModeKind::Plan),
        _ => None,
    }
}
fn observed_mode_catalogue(value: &RawValue) -> Option<bool> {
    struct Catalogue;
    impl<'de> serde::de::Visitor<'de> for Catalogue {
        type Value = bool;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("two native collaboration presets")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut values: A) -> Result<bool, A::Error> {
            let mut modes = [None; 2];
            for slot in &mut modes {
                let Some(value) = values.next_element::<&RawValue>()? else {
                    return Ok(false);
                };
                *slot = observed_mode(value);
            }
            if values.next_element::<&RawValue>()?.is_some() {
                return Ok(false);
            }
            Ok(matches!(
                modes,
                [
                    Some(CollaborationModeKind::Default),
                    Some(CollaborationModeKind::Plan)
                ] | [
                    Some(CollaborationModeKind::Plan),
                    Some(CollaborationModeKind::Default)
                ]
            ))
        }
    }
    use serde::Deserializer;
    let metadata = metadata_object(value.get()).ok()?;
    let data = metadata.data?;
    let mut decoder = serde_json::Deserializer::from_str(data.get());
    decoder.deserialize_seq(Catalogue).ok()
}
struct Correlation {
    id: NativeRpcId,
    method: String,
    thread: Option<Id>,
    lookup: bool,
    account: bool,
    bootstrap: Option<BootstrapRpc>,
    mode_revision: Option<u64>,
    catalogue_revision: Option<u64>,
}
#[derive(Clone, Copy)]
enum BootstrapRpc {
    ModelList,
    ConfigRequirementsRead,
}
#[derive(Clone, Copy)]
enum QueuedCapture {
    LookupSend,
    LookupReply,
    AccountSend,
    AccountReply,
    BootstrapRequest(BootstrapRpc),
    BootstrapReply(BootstrapRpc),
}
#[derive(Clone, Copy, Default)]
enum LookupReply {
    #[default]
    None,
    Error,
    IdMatch,
    IdMismatch,
    InvalidShape,
}
impl LookupReply {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Error => "error",
            Self::IdMatch => "id_match",
            Self::IdMismatch => "id_mismatch",
            Self::InvalidShape => "invalid_shape",
        }
    }
}
#[derive(Clone, Copy, Default)]
enum AccountReply {
    #[default]
    None,
    Result,
    Error,
}
impl AccountReply {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Result => "result",
            Self::Error => "error",
        }
    }
}
#[derive(Clone, Copy, Default)]
enum AccountShape {
    #[default]
    Unobserved,
    ResultInvalid,
    Absent,
    Null,
    ApiKey,
    Chatgpt,
    Bedrock,
    UnknownType,
    Invalid,
}
impl AccountShape {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unobserved => "unobserved",
            Self::ResultInvalid => "result_invalid",
            Self::Absent => "absent",
            Self::Null => "null",
            Self::ApiKey => "api_key",
            Self::Chatgpt => "chatgpt",
            Self::Bedrock => "bedrock",
            Self::UnknownType => "unknown_type",
            Self::Invalid => "invalid",
        }
    }
}
#[derive(Clone, Copy, Default)]
enum AuthRequired {
    #[default]
    Unobserved,
    Missing,
    False,
    True,
    Invalid,
}
impl AuthRequired {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unobserved => "unobserved",
            Self::Missing => "missing",
            Self::False => "false",
            Self::True => "true",
            Self::Invalid => "invalid",
        }
    }
}
const BOOTSTRAP_REQUEST_SEEN: u8 = 0x01;
const BOOTSTRAP_REQUEST_SENT: u8 = 0x02;
const BOOTSTRAP_RESULT_RECEIVED: u8 = 0x04;
const BOOTSTRAP_ERROR_RECEIVED: u8 = 0x08;
const BOOTSTRAP_REPLY_SENT: u8 = 0x10;
fn classify_account_result(result: &RawValue) -> (AccountShape, AuthRequired) {
    let Ok(result) = metadata_object(result.get()) else {
        return (AccountShape::ResultInvalid, AuthRequired::Invalid);
    };
    let auth_required = match result.requires_openai_auth {
        None => AuthRequired::Missing,
        Some(value) => match value.get().trim() {
            "false" => AuthRequired::False,
            "true" => AuthRequired::True,
            _ => AuthRequired::Invalid,
        },
    };
    let account_shape = match result.account {
        None => AccountShape::Absent,
        Some(value) if value.get().trim() == "null" => AccountShape::Null,
        Some(value) => match metadata_object(value.get()) {
            Ok(account) => match account
                .account_type
                .and_then(|value| metadata_string(value).ok())
            {
                Some(value) => match value.as_ref() {
                    "apiKey" => AccountShape::ApiKey,
                    "chatgpt" => AccountShape::Chatgpt,
                    "amazonBedrock" => AccountShape::Bedrock,
                    _ => AccountShape::UnknownType,
                },
                None => AccountShape::Invalid,
            },
            Err(_) => AccountShape::Invalid,
        },
    };
    (account_shape, auth_required)
}
#[derive(Clone, Copy, Default)]
enum FirstErrorFamily {
    #[default]
    None,
    BootstrapModelList,
    BootstrapConfigRequirementsRead,
    Initialize,
    ThreadRead,
    ThreadResume,
    OtherRpc,
}
impl FirstErrorFamily {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BootstrapModelList => "bootstrap_model_list",
            Self::BootstrapConfigRequirementsRead => "bootstrap_config_requirements",
            Self::Initialize => "initialize",
            Self::ThreadRead => "thread_read",
            Self::ThreadResume => "thread_resume",
            Self::OtherRpc => "other_rpc",
        }
    }
}
#[derive(Clone, Copy)]
struct ClosedCapture {
    lookup_seen: bool,
    lookup_send_completed: bool,
    lookup_reply: LookupReply,
    reply_send_completed: bool,
    account_seen: bool,
    account_send_completed: bool,
    account_reply: AccountReply,
    account_shape: AccountShape,
    auth_required: AuthRequired,
    account_reply_send_completed: bool,
    bootstrap_model_list: u8,
    bootstrap_config_requirements: u8,
    resume_seen: bool,
    first_error_family: FirstErrorFamily,
}
#[derive(Default)]
struct DiagnosticCapture {
    frozen: Option<ClosedCapture>,
    lookup_seen: bool,
    lookup_send_completed: bool,
    lookup_reply: LookupReply,
    reply_send_completed: bool,
    account_seen: bool,
    account_send_completed: bool,
    account_reply: AccountReply,
    account_shape: AccountShape,
    auth_required: AuthRequired,
    account_reply_send_completed: bool,
    bootstrap_model_list: u8,
    bootstrap_config_requirements: u8,
    resume_seen: bool,
    first_error_family: FirstErrorFamily,
}
impl DiagnosticCapture {
    fn lookup(&mut self) -> bool {
        if self.frozen.is_none() && !self.lookup_seen {
            self.lookup_seen = true;
            true
        } else {
            false
        }
    }
    fn sent(&mut self, capture: QueuedCapture) {
        if self.frozen.is_some() {
            return;
        }
        match capture {
            QueuedCapture::LookupSend => self.lookup_send_completed = true,
            QueuedCapture::LookupReply => self.reply_send_completed = true,
            QueuedCapture::AccountSend => self.account_send_completed = true,
            QueuedCapture::AccountReply => self.account_reply_send_completed = true,
            QueuedCapture::BootstrapRequest(rpc) => self.bootstrap_mark(rpc, BOOTSTRAP_REQUEST_SENT),
            QueuedCapture::BootstrapReply(rpc) => self.bootstrap_mark(rpc, BOOTSTRAP_REPLY_SENT),
        }
    }
    fn bootstrap_mask_mut(&mut self, rpc: BootstrapRpc) -> &mut u8 {
        match rpc {
            BootstrapRpc::ModelList => &mut self.bootstrap_model_list,
            BootstrapRpc::ConfigRequirementsRead => &mut self.bootstrap_config_requirements,
        }
    }
    fn bootstrap_request(&mut self, rpc: BootstrapRpc) -> bool {
        if self.frozen.is_some() {
            return false;
        }
        let mask = self.bootstrap_mask_mut(rpc);
        if *mask & BOOTSTRAP_REQUEST_SEEN != 0 {
            false
        } else {
            *mask |= BOOTSTRAP_REQUEST_SEEN;
            true
        }
    }
    fn bootstrap_mark(&mut self, rpc: BootstrapRpc, marker: u8) {
        if self.frozen.is_none() {
            *self.bootstrap_mask_mut(rpc) |= marker;
        }
    }
    fn reply(&mut self, reply: LookupReply) {
        if self.frozen.is_none() {
            self.lookup_reply = reply;
        }
    }
    fn account(&mut self) -> bool {
        if self.frozen.is_none() && !self.account_seen {
            self.account_seen = true;
            true
        } else {
            false
        }
    }
    fn account_reply(&mut self, reply: AccountReply) {
        if self.frozen.is_none() {
            self.account_reply = reply;
        }
    }
    fn account_result(&mut self, account_shape: AccountShape, auth_required: AuthRequired) {
        if self.frozen.is_none() {
            self.account_reply = AccountReply::Result;
            self.account_shape = account_shape;
            self.auth_required = auth_required;
        }
    }
    fn resume(&mut self) {
        if self.frozen.is_none() {
            self.resume_seen = true;
        }
    }
    fn first_error(&mut self, family: FirstErrorFamily) {
        if self.frozen.is_none() && matches!(self.first_error_family, FirstErrorFamily::None) {
            self.first_error_family = family;
        }
    }
    fn freeze(&mut self) -> ClosedCapture {
        *self.frozen.get_or_insert(ClosedCapture {
            lookup_seen: self.lookup_seen,
            lookup_send_completed: self.lookup_send_completed,
            lookup_reply: self.lookup_reply,
            reply_send_completed: self.reply_send_completed,
            account_seen: self.account_seen,
            account_send_completed: self.account_send_completed,
            account_reply: self.account_reply,
            account_shape: self.account_shape,
            auth_required: self.auth_required,
            account_reply_send_completed: self.account_reply_send_completed,
            bootstrap_model_list: self.bootstrap_model_list,
            bootstrap_config_requirements: self.bootstrap_config_requirements,
            resume_seen: self.resume_seen,
            first_error_family: self.first_error_family,
        })
    }
}
#[derive(Default)]
pub struct Observer {
    pending: Vec<Correlation>,
    associated: Vec<Id>,
    resumed: Vec<Id>,
    resume_error: Option<Id>,
    first_error: Option<(&'static str, i64)>,
    expected_thread: Option<Id>,
    collaboration_mode: Option<CollaborationModeObservation>,
    collaboration_default_plan: Option<bool>,
    mode_revision: u64,
    mode_current_thread: bool,
    catalogue_revision: u64,
    queued_capture: Option<QueuedCapture>,
    diagnostic: Option<Arc<StdMutex<DiagnosticCapture>>>,
}
impl Observer {
    fn for_thread(expected_thread: Id) -> Self {
        Self {
            expected_thread: Some(expected_thread),
            ..Self::default()
        }
    }
    fn for_capturing_thread(expected_thread: Id, diagnostic: Arc<StdMutex<DiagnosticCapture>>) -> Self {
        Self {
            expected_thread: Some(expected_thread),
            diagnostic: Some(diagnostic),
            ..Self::default()
        }
    }
    fn set_mode(&mut self, mode: Option<CollaborationModeKind>, from_settings: bool) {
        self.collaboration_mode = self.mode_revision.checked_add(1).and_then(|revision| {
            self.mode_revision = revision;
            mode.map(|mode| CollaborationModeObservation {
                mode,
                revision,
                from_settings,
            })
        });
    }
    fn queued_capture(&mut self) -> Option<QueuedCapture> {
        self.queued_capture.take()
    }
    fn lookup_result(&self, result: Option<&RawValue>) -> LookupReply {
        let Some(result) = result else {
            return LookupReply::InvalidShape;
        };
        let Ok(result) = metadata_object(result.get()) else {
            return LookupReply::InvalidShape;
        };
        let Some(thread) = result.thread else {
            return LookupReply::InvalidShape;
        };
        let Ok(thread) = metadata_object(thread.get()) else {
            return LookupReply::InvalidShape;
        };
        let Some(id) = thread.id else {
            return LookupReply::InvalidShape;
        };
        let Ok(id) = metadata_string(id) else {
            return LookupReply::InvalidShape;
        };
        let id = Id(id.into_owned());
        if id.validate().is_err() {
            return LookupReply::InvalidShape;
        }
        if self.expected_thread.as_ref() == Some(&id) {
            LookupReply::IdMatch
        } else {
            LookupReply::IdMismatch
        }
    }
    pub fn from_tui(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.queued_capture = None;
        let v = bridge_metadata(bytes)?;
        let Some(method) = v.method else {
            return Ok(());
        };
        let method = metadata_string(method)?;
        let Some(raw_id) = v.id else {
            return Ok(());
        };
        let id =
            NativeRpcId::from_raw(raw_id.get()).map_err(|_| invalid("invalid native RPC ID"))?;
        if self.pending.len() >= CORRELATIONS || self.pending.iter().any(|p| p.id == id) {
            return Err(invalid("untrustworthy or full TUI correlation set"));
        }
        let params = if matches!(
            method.as_ref(),
            "thread/start" | "thread/resume" | "thread/fork" | "thread/unsubscribe"
        ) {
            v.params
                .map(|params| metadata_object(params.get()))
                .transpose()?
        } else {
            None
        };
        let thread = if matches!(
            method.as_ref(),
            "thread/resume" | "thread/fork" | "thread/unsubscribe"
        ) {
            let raw = params
                .as_ref()
                .and_then(|params| params.thread_id)
                .ok_or_else(|| invalid("thread association request missing ID"))?;
            let t = Id(metadata_string(raw)?.into_owned());
            t.validate().map_err(|_| invalid("invalid thread ID"))?;
            Some(t)
        } else {
            None
        };
        let mut lookup = self.diagnostic.is_some()
            && method == "thread/read"
            && v
                .params
                .and_then(|params| metadata_object(params.get()).ok())
                .and_then(|params| {
                    let thread = params.thread_id.and_then(|value| metadata_string(value).ok())?;
                    let thread = Id(thread.into_owned());
                    thread.validate().ok()?;
                    (params.include_turns.is_none_or(|value| value.get() == "false")
                        && self.expected_thread.as_ref() == Some(&thread))
                    .then_some(())
                })
                .is_some();
        if lookup {
            lookup = self
                .diagnostic
                .as_ref()
                .is_some_and(|capture| capture.lock().expect("diagnostic capture mutex").lookup());
            if lookup {
                self.queued_capture = Some(QueuedCapture::LookupSend);
            }
        }
        let account_request = self.diagnostic.is_some() && method == "account/read";
        if method == "thread/resume" && thread.as_ref() == self.expected_thread.as_ref() {
            if let Some(capture) = &self.diagnostic {
                capture.lock().expect("diagnostic capture mutex").resume();
            }
        }
        let bootstrap = if self.diagnostic.is_some() {
            let rpc = match method.as_ref() {
                "model/list" => Some(BootstrapRpc::ModelList),
                "configRequirements/read" => Some(BootstrapRpc::ConfigRequirementsRead),
                _ => None,
            };
            rpc.filter(|rpc| {
                self.diagnostic.as_ref().is_some_and(|capture| {
                    capture
                        .lock()
                        .expect("diagnostic capture mutex")
                        .bootstrap_request(*rpc)
                })
            })
        } else {
            None
        };
        if let Some(rpc) = bootstrap {
            self.queued_capture = Some(QueuedCapture::BootstrapRequest(rpc));
        }
        let mut mode_revision = None;
        if matches!(
            method.as_ref(),
            "thread/start" | "thread/resume" | "thread/fork"
        ) || (method == "thread/unsubscribe" && thread.as_ref() == self.expected_thread.as_ref())
        {
            self.collaboration_mode = None;
            self.mode_current_thread = false;
            mode_revision = self.mode_revision.checked_add(1);
            if let Some(revision) = mode_revision {
                self.mode_revision = revision;
            }
        }
        let mut catalogue_revision = None;
        if method == "collaborationMode/list" {
            self.collaboration_default_plan = None;
            catalogue_revision = self.catalogue_revision.checked_add(1);
            if let Some(revision) = catalogue_revision {
                self.catalogue_revision = revision;
            }
        }
        self.pending.push(Correlation {
            id,
            method: method.into_owned(),
            thread,
            lookup,
            account: false,
            bootstrap,
            mode_revision,
            catalogue_revision,
        });
        if account_request
            && self
                .diagnostic
                .as_ref()
                .is_some_and(|capture| capture.lock().expect("diagnostic capture mutex").account())
        {
            self.pending
                .last_mut()
                .expect("accepted account correlation")
                .account = true;
            self.queued_capture = Some(QueuedCapture::AccountSend);
        }
        Ok(())
    }
    pub fn from_owner(&mut self, bytes: &[u8]) -> io::Result<bool> {
        self.queued_capture = None;
        let v = bridge_metadata(bytes)?;
        if let Some(method) = v.method {
            let method = metadata_string(method)?;
            if method == "thread/settings/updated" && v.id.is_none() {
                let params = v.params.and_then(|value| metadata_object(value.get()).ok());
                let thread = params
                    .as_ref()
                    .and_then(|params| params.thread_id)
                    .and_then(|value| metadata_string(value).ok());
                if thread.is_none() && self.mode_current_thread {
                    let changed = self.collaboration_mode.is_some();
                    self.set_mode(None, true);
                    return Ok(changed);
                }
                if thread.as_deref()
                    == self
                        .expected_thread
                        .as_ref()
                        .map(|thread| thread.0.as_str())
                    && self.mode_current_thread
                    && self
                        .expected_thread
                        .as_ref()
                        .is_some_and(|thread| self.resumed.contains(thread))
                {
                    let mode = params
                        .and_then(|params| params.thread_settings)
                        .and_then(|value| metadata_object(value.get()).ok())
                        .and_then(|settings| settings.collaboration_mode)
                        .and_then(observed_mode);
                    self.set_mode(mode, true);
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let Some(raw) = v.id else {
            return Ok(false);
        };
        let id = NativeRpcId::from_raw(raw.get()).map_err(|_| invalid("invalid native RPC ID"))?;
        let Some(index) = self.pending.iter().position(|p| p.id == id) else {
            return Ok(false);
        };
        // First ordinary result/error consumes callback even if malformed; it can never bind later.
        let pending = self.pending.remove(index);
        if let Some(error) = v.error {
            let error = metadata_object(error.get())?;
            let code = error.code.and_then(|code| code.get().parse::<i64>().ok());
            if v.result.is_some()
                || code.is_none()
                || error
                    .message
                    .is_none_or(|message| !message.get().starts_with('"'))
            {
                return Err(invalid("malformed native error reply"));
            }
            if pending.lookup {
                if let Some(capture) = &self.diagnostic {
                    capture.lock().expect("diagnostic capture mutex").reply(LookupReply::Error);
                }
                self.queued_capture = Some(QueuedCapture::LookupReply);
            }
            if pending.account {
                if let Some(capture) = &self.diagnostic {
                    capture
                        .lock()
                        .expect("diagnostic capture mutex")
                        .account_reply(AccountReply::Error);
                }
                self.queued_capture = Some(QueuedCapture::AccountReply);
            }
            if let Some(rpc) = pending.bootstrap {
                if let Some(capture) = &self.diagnostic {
                    capture
                        .lock()
                        .expect("diagnostic capture mutex")
                        .bootstrap_mark(rpc, BOOTSTRAP_ERROR_RECEIVED);
                }
                self.queued_capture = Some(QueuedCapture::BootstrapReply(rpc));
            }
            let mut changed = false;
            if let Some(capture) = &self.diagnostic {
                let family = match pending.bootstrap {
                    Some(BootstrapRpc::ModelList) => FirstErrorFamily::BootstrapModelList,
                    Some(BootstrapRpc::ConfigRequirementsRead) => FirstErrorFamily::BootstrapConfigRequirementsRead,
                    None => match pending.method.as_str() {
                        "initialize" => FirstErrorFamily::Initialize,
                        "thread/read" => FirstErrorFamily::ThreadRead,
                        "thread/resume" => FirstErrorFamily::ThreadResume,
                        _ => FirstErrorFamily::OtherRpc,
                    },
                };
                capture.lock().expect("diagnostic capture mutex").first_error(family);
            }
            if self.first_error.is_none() {
                let method = match pending.method.as_str() {
                    "initialize" => Some("initialize"),
                    "thread/read" => Some("thread/read"),
                    "thread/resume" => Some("thread/resume"),
                    _ => None,
                };
                if let (Some(method), Some(code)) = (method, code) {
                    self.first_error = Some((method, code));
                    changed = true;
                }
            }
            if pending.method == "thread/resume" && matches!(code, Some(-32602..=-32600)) {
                self.resume_error = pending.thread;
                return Ok(true);
            }
            if pending.method == "collaborationMode/list" {
                if pending.catalogue_revision != Some(self.catalogue_revision) {
                    return Ok(false);
                }
                self.collaboration_default_plan = None;
                return Ok(true);
            }
            return Ok(changed);
        }
        let r = v
            .result
            .ok_or_else(|| invalid("malformed correlated reply"))?;
        if pending.lookup {
            if let Some(capture) = &self.diagnostic {
                let reply = self.lookup_result(Some(r));
                capture.lock().expect("diagnostic capture mutex").reply(reply);
            }
            self.queued_capture = Some(QueuedCapture::LookupReply);
        }
        if pending.account {
            if let Some(capture) = &self.diagnostic {
                let (account_shape, auth_required) = classify_account_result(r);
                capture
                    .lock()
                    .expect("diagnostic capture mutex")
                    .account_result(account_shape, auth_required);
            }
            self.queued_capture = Some(QueuedCapture::AccountReply);
        }
        if let Some(rpc) = pending.bootstrap {
            if let Some(capture) = &self.diagnostic {
                capture
                    .lock()
                    .expect("diagnostic capture mutex")
                    .bootstrap_mark(rpc, BOOTSTRAP_RESULT_RECEIVED);
            }
            self.queued_capture = Some(QueuedCapture::BootstrapReply(rpc));
        }
        if pending.method == "collaborationMode/list" {
            if pending.catalogue_revision != Some(self.catalogue_revision) {
                return Ok(false);
            }
            self.collaboration_default_plan = observed_mode_catalogue(r);
            return Ok(true);
        }
        if !matches!(
            pending.method.as_str(),
            "thread/start" | "thread/resume" | "thread/fork" | "thread/unsubscribe"
        ) {
            return Ok(false);
        }
        let r = metadata_object(r.get())?;
        if pending.method == "thread/unsubscribe" {
            let status = r.status.map(metadata_string).transpose()?;
            if !matches!(
                status.as_deref(),
                Some("notLoaded" | "notSubscribed" | "unsubscribed")
            ) {
                return Err(invalid("invalid unsubscribe result"));
            }
            if let Some(t) = pending.thread {
                self.associated.retain(|i| i != &t);
                self.resumed.retain(|i| i != &t);
            }
            return Ok(true);
        }
        let raw_thread = r
            .thread
            .ok_or_else(|| invalid("correlated thread reply missing native ID"))?;
        let thread = metadata_object(raw_thread.get())?;
        let raw_id = thread
            .id
            .ok_or_else(|| invalid("correlated thread reply missing native ID"))?;
        let t = Id(metadata_string(raw_id)?.into_owned());
        t.validate()
            .map_err(|_| invalid("invalid native thread ID"))?;
        if pending.method == "thread/resume" && pending.thread.as_ref() != Some(&t) {
            return Err(invalid("resume returned a substitute thread"));
        }
        if !self.associated.contains(&t) {
            if self.associated.len() >= 64 {
                return Err(invalid("association limit"));
            }
            self.associated.push(t.clone());
        }
        if pending.method == "thread/resume" {
            if self.expected_thread.as_ref() == Some(&t)
                && pending.mode_revision == Some(self.mode_revision)
            {
                self.mode_current_thread = true;
                self.set_mode(r.collaboration_mode.and_then(observed_mode), false);
            }
            if self.resume_error.as_ref() == Some(&t) {
                self.resume_error = None;
            }
            if !self.resumed.contains(&t) {
                self.resumed.push(t);
            }
        }
        Ok(true)
    }
    pub fn associated(&self) -> &[Id] {
        &self.associated
    }
    pub fn resumed(&self) -> &[Id] {
        &self.resumed
    }
    pub fn rejected_resume(&self) -> Option<&Id> {
        self.resume_error.as_ref()
    }
}
pub fn message_bytes(message: &Message) -> io::Result<&[u8]> {
    match message {
        Message::Text(text) => Ok(text.as_bytes()),
        Message::Binary(bytes) => Ok(bytes.as_ref()),
        _ => Err(invalid("native JSON data message required")),
    }
}
struct Queued {
    message: Message,
    capture: Option<QueuedCapture>,
    _bytes: tokio::sync::OwnedSemaphorePermit,
    _frame: tokio::sync::OwnedSemaphorePermit,
}
fn enqueue_marked(
    tx: &mpsc::Sender<Queued>,
    budget: &Arc<Semaphore>,
    frames: &Arc<Semaphore>,
    message: Message,
    capture: Option<QueuedCapture>,
) -> io::Result<()> {
    if message.len() > FRAME_BYTES {
        return Err(invalid("native relay frame exceeds128MiB"));
    }
    let permit = budget
        .clone()
        .try_acquire_many_owned(message.len().max(1) as u32)
        .map_err(|_| invalid("native relay byte queue overflow"))?;
    let frame = frames
        .clone()
        .try_acquire_owned()
        .map_err(|_| invalid("native relay frame queue overflow"))?;
    tx.try_send(Queued {
        message,
        capture,
        _bytes: permit,
        _frame: frame,
    })
    .map_err(|_| invalid("native relay frame queue overflow"))
}
fn enqueue(
    tx: &mpsc::Sender<Queued>,
    budget: &Arc<Semaphore>,
    frames: &Arc<Semaphore>,
    message: Message,
) -> io::Result<()> {
    enqueue_marked(tx, budget, frames, message, None)
}
struct OriginGuard;
impl Callback for OriginGuard {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        if request.headers().contains_key("origin") {
            let mut r = ErrorResponse::new(Some("Origin forbidden".into()));
            *r.status_mut() = StatusCode::FORBIDDEN;
            Err(r)
        } else {
            Ok(response)
        }
    }
}
struct Publication {
    path: PathBuf,
    manifest: Manifest,
    presence: Presence,
    terminal_capture: bool,
    pending_terminal_capture: Option<(&'static str, TerminalCapture)>,
    pending_terminal_annotation: Option<(&'static str, SafeMessage)>,
}
#[derive(PartialEq, Eq)]
struct PublicationScope {
    binding_id: Uuid,
    owner_epoch: OwnerEpoch,
    relay: ProcessEvidence,
    tui: Option<ProcessEvidence>,
    connection_epoch: Uuid,
    state: PresenceState,
}
impl Publication {
    fn scope(&self) -> PublicationScope {
        PublicationScope {
            binding_id: self.presence.binding_id.clone(),
            owner_epoch: self.presence.owner_epoch.clone(),
            relay: self.presence.relay.clone(),
            tui: self.presence.tui.clone(),
            connection_epoch: self.presence.connection_epoch.clone(),
            state: self.presence.state.clone(),
        }
    }
    fn publish_if_current(&mut self, scope: &PublicationScope) -> io::Result<bool> {
        if &self.scope() != scope {
            return Ok(false);
        }
        self.publish()?;
        Ok(true)
    }
    fn set_terminal_proof_if_current(
        &mut self,
        scope: &PublicationScope,
        terminal_proof: TerminalProofAvailability,
    ) -> io::Result<bool> {
        if &self.scope() != scope {
            return Ok(false);
        }
        self.presence.terminal_proof = terminal_proof;
        self.publish()?;
        Ok(true)
    }
    fn publish(&mut self) -> io::Result<()> {
        self.presence.updated_at = Uint(pairing::now_ms());
        let path = presence_path(&self.path);
        let replace = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let p: Presence = private_read(&path)?;
                if p.binding_id != self.presence.binding_id || p.relay != self.presence.relay {
                    return Err(invalid("foreign relay presence"));
                }
                true
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        private_write(&path, &self.presence, replace)
    }
    fn publish_startup(
        &mut self,
        deadline: tokio::time::Instant,
        visible: &mut bool,
        object: &mut Option<(u64, u64)>,
    ) -> io::Result<()> {
        self.presence.updated_at = Uint(pairing::now_ms());
        let path = presence_path(&self.path);
        let replace = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let p: Presence = private_read(&path)?;
                if p.binding_id != self.presence.binding_id || p.relay != self.presence.relay {
                    return Err(invalid("foreign relay presence"));
                }
                true
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        private_write_with_visibility(
            &path,
            &self.presence,
            replace,
            |file| {
                *object = Some(file_object(file)?);
                ensure_startup_live(deadline)
            },
            || *visible = true,
        )
    }
    fn retain_modes(&mut self, observer: &Observer) {
        self.presence.collaboration_mode = observer.collaboration_mode;
        self.presence.collaboration_default_plan = observer.collaboration_default_plan;
    }
    fn retain_native_error(&mut self, observer: &Observer) {
        if self.presence.error.is_none()
            && let Some((method, code)) = observer.first_error
        {
            self.presence.error = Some(SafeMessage(format!(
                "Correlated native {method} error ({code})"
            )));
        }
    }
    fn retain_terminal_capture(&mut self, terminal_error: &'static str, capture: TerminalCapture) {
        if self.terminal_capture && self.presence.error.is_none() {
            self.pending_terminal_capture = Some((terminal_error, capture));
        }
    }
    fn retain_terminal_annotation(&mut self, terminal_error: &'static str, annotation: SafeMessage) {
        if self.terminal_capture && self.presence.error.is_none() {
            self.pending_terminal_annotation = Some((terminal_error, annotation));
        }
    }
    fn clear(&mut self, state: PresenceState, error: Option<&'static str>) -> io::Result<()> {
        self.presence.state = state;
        self.presence.associated_thread_ids.clear();
        self.presence.resumed_thread_ids.clear();
        self.presence.collaboration_mode = None;
        self.presence.collaboration_default_plan = None;
        if self.presence.error.is_none() {
            self.presence.error = self
                .pending_terminal_annotation
                .take()
                .filter(|(terminal_error, _)| error == Some(*terminal_error))
                .map(|(_, annotation)| annotation)
                .or_else(|| {
                    self.pending_terminal_capture
                        .take()
                        .filter(|(terminal_error, _)| error == Some(*terminal_error))
                        .map(|(_, capture)| terminal_capture_message(capture))
                })
                .or_else(|| error.map(|s| SafeMessage(s.into())));
        } else {
            self.pending_terminal_capture = None;
            self.pending_terminal_annotation = None;
        }
        self.publish()
    }
}
fn exact_private_object(path: &Path, object: (u64, u64)) -> bool {
    fs::symlink_metadata(path).ok().is_some_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.uid() == pairing::uid()
            && (metadata.dev(), metadata.ino()) == object
    })
}
// File/process/socket hashing is synchronous. Keep it off Tokio workers and never hold the
// publication lock while it runs; the caller rechecks the captured scope before publishing.
async fn blocking_proof(proof: impl FnOnce() -> io::Result<()> + Send + 'static) -> io::Result<()> {
    tokio::task::spawn_blocking(proof)
        .await
        .map_err(|_| invalid("Owner/manifest source proof worker failed"))?
}
async fn manifest_proof(manifest: Manifest) -> io::Result<()> {
    blocking_proof(move || manifest.validate()).await
}
const CODEX_TERMINAL_CAPTURE_ENV: &str = "GOATR_OWNED_CODEX_TERMINAL_CAPTURE";
#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct TerminalCapture {
    pub phase: &'static str,
    pub io: &'static str,
    pub errno: i32,
    pub response_timeout: bool,
}
fn terminal_capture_enabled(manifest: &Manifest) -> bool {
    std::env::var(CODEX_TERMINAL_CAPTURE_ENV).ok().as_deref()
        == Some(manifest.binding_id.0.as_str())
}
fn terminal_capture(failure: &TerminalFailure) -> TerminalCapture {
    let (io, errno) = herdr::codex_invalidation_io(&failure.error);
    TerminalCapture {
        phase: failure.phase,
        io,
        errno,
        response_timeout: matches!(failure.phase, "snapshot" | "process_info")
            && failure.error.kind() == io::ErrorKind::InvalidData
            && failure.error.to_string() == "Herdr response timeout",
    }
}
fn terminal_capture_message(capture: TerminalCapture) -> SafeMessage {
    SafeMessage(format!(
        "Codex terminal capture phase={} io={} errno={} response_timeout={}",
        capture.phase,
        capture.io,
        capture.errno,
        u8::from(capture.response_timeout),
    ))
}

#[doc(hidden)]
pub fn terminal_capture_annotation(value: &str) -> Option<TerminalCapture> {
    const MAX_BYTES: usize = 192;
    if value.len() > MAX_BYTES || !value.is_ascii() {
        return None;
    }
    let mut fields = value.split(' ');
    if fields.next()? != "Codex" || fields.next()? != "terminal" || fields.next()? != "capture" {
        return None;
    }
    let phase = match fields.next()?.strip_prefix("phase=")? {
        "connect" => "connect",
        "snapshot_source" => "snapshot_source",
        "snapshot" => "snapshot",
        "reporter_process" => "reporter_process",
        "process_info_connect" => "process_info_connect",
        "process_info_source" => "process_info_source",
        "process_info" => "process_info",
        "tui_distribution" => "tui_distribution",
        "tui_argv" => "tui_argv",
        "tui_foreground" => "tui_foreground",
        "tui_write_unpolled_not_pending" => "tui_write_unpolled_not_pending",
        "tui_write_ready_not_pending" => "tui_write_ready_not_pending",
        "tui_write_ready_pending" => "tui_write_ready_pending",
        "tui_write_start_send_not_pending" => "tui_write_start_send_not_pending",
        "tui_write_flush_not_pending" => "tui_write_flush_not_pending",
        "tui_write_flush_pending" => "tui_write_flush_pending",
        _ => return None,
    };
    let io = match fields.next()?.strip_prefix("io=")? {
        "not_found" => "not_found",
        "permission_denied" => "permission_denied",
        "connection_refused" => "connection_refused",
        "connection_reset" => "connection_reset",
        "connection_aborted" => "connection_aborted",
        "not_connected" => "not_connected",
        "broken_pipe" => "broken_pipe",
        "timed_out" => "timed_out",
        "unexpected_eof" => "unexpected_eof",
        "invalid_data" => "invalid_data",
        "other" => "other",
        _ => return None,
    };
    let errno = fields.next()?.strip_prefix("errno=")?.parse::<i32>().ok()?;
    if !(-4095..=4095).contains(&errno) {
        return None;
    }
    let response_timeout = match fields.next()?.strip_prefix("response_timeout=")? {
        "0" => false,
        "1" if matches!(phase, "snapshot" | "process_info") && io == "invalid_data" => true,
        _ => return None,
    };
    if fields.next().is_some() {
        return None;
    }
    if phase.starts_with("tui_write_") && (io != "timed_out" || errno != 0 || response_timeout) {
        return None;
    }
    let capture = TerminalCapture {
        phase,
        io,
        errno,
        response_timeout,
    };
    (terminal_capture_message(capture).0 == value).then_some(capture)
}

fn terminal_failure(error: &io::Error) -> &'static str {
    if error
        .get_ref()
        .is_some_and(|cause| cause.is::<herdr::ProcessInfoNotFound>())
    {
        "Herdr pane.process_info rejected: pane_not_found"
    } else {
        "Herdr terminal source proof failed"
    }
}
fn terminal_proof_unavailable(failure: &TerminalFailure) -> bool {
    matches!(failure.phase, "snapshot" | "process_info")
        && failure.error.kind() == io::ErrorKind::BrokenPipe
}
async fn forward_owner_to_tui_observed<S>(
    tui: &mut S,
    tui_queue: &mut mpsc::Receiver<Queued>,
    diagnostic: Option<Arc<StdMutex<DiagnosticCapture>>>,
) -> Result<ForwardClose, &'static str>
where
    S: Sink<Message> + Unpin,
{
    while let Some(queued) = tui_queue.recv().await {
        // The passive owner→TUI send keeps its charged queue entry until the
        // existing bridge termination path settles this one original frame.
        tui.send(queued.message)
            .await
            .map_err(|_| "TUI write failed")?;
        if let (Some(capture), Some(diagnostic)) = (queued.capture, diagnostic.as_ref()) {
            diagnostic
                .lock()
                .expect("diagnostic capture mutex")
                .sent(capture);
        }
    }
    Ok(ForwardClose::TuiQueueDrained)
}
async fn forward_owner_to_tui<S>(
    tui: &mut S,
    tui_queue: &mut mpsc::Receiver<Queued>,
) -> Result<ForwardClose, &'static str>
where
    S: Sink<Message> + Unpin,
{
    forward_owner_to_tui_observed(tui, tui_queue, None).await
}
async fn forward(
    tui: WebSocketStream<UnixStream>,
    owner: WebSocketStream<UnixStream>,
    publication: Arc<Mutex<Publication>>,
) -> Result<(), ForwardFailure> {
    let (mut tw, mut tr) = tui.split();
    let (mut ow, mut or) = owner.split();
    let (expected_thread, capture_closed) = {
        let publication = publication.lock().await;
        (
            publication.manifest.expected_thread_id.clone(),
            publication.terminal_capture,
        )
    };
    let (to_owner, mut owner_queue) = mpsc::channel::<Queued>(QUEUE_FRAMES);
    let (to_tui, mut tui_queue) = mpsc::channel::<Queued>(QUEUE_FRAMES);
    let diagnostic = capture_closed.then(|| Arc::new(StdMutex::new(DiagnosticCapture::default())));
    let observer = Arc::new(Mutex::new(if let Some(diagnostic) = diagnostic.clone() {
        Observer::for_capturing_thread(expected_thread, diagnostic)
    } else {
        Observer::for_thread(expected_thread)
    }));
    let mut jobs = JoinSet::new();
    let budget = Arc::new(Semaphore::new(QUEUE_BYTES));
    let frames = Arc::new(Semaphore::new(QUEUE_FRAMES));
    let observed = observer.clone();
    let published = publication.clone();
    jobs.spawn(async move {
        loop {
            let Some(message) = tr.next().await else {
                return Ok::<_, &'static str>(ForwardClose::TuiEof);
            };
            let message = message.map_err(|_| "TUI transport read failed")?;
            let mut capture = None;
            if matches!(message, Message::Text(_) | Message::Binary(_)) {
                let bytes = message_bytes(&message).map_err(|_| "TUI data frame invalid")?;
                let mut observer = observed.lock().await;
                let before = (
                    observer.collaboration_mode,
                    observer.collaboration_default_plan,
                );
                observer
                    .from_tui(bytes)
                    .map_err(|_| "TUI observation failed")?;
                capture = observer.queued_capture();
                if before
                    != (
                        observer.collaboration_mode,
                        observer.collaboration_default_plan,
                    )
                {
                    let (manifest, scope) = {
                        let publication = published.lock().await;
                        (publication.manifest.clone(), publication.scope())
                    };
                    manifest_proof(manifest)
                        .await
                        .map_err(|_| "Owner/manifest source proof failed")?;
                    let mut publication = published.lock().await;
                    if publication.scope() != scope {
                        return Err("Relay publication source changed");
                    }
                    publication.retain_modes(&observer);
                    if !publication
                        .publish_if_current(&scope)
                        .map_err(|_| "Native observation publication failed")?
                    {
                        return Err("Relay publication source changed");
                    }
                }
            }
            let close = message.is_close();
            enqueue_marked(&to_owner, &budget, &frames, message, capture)
                .map_err(|_| "Owner-bound relay queue failed")?;
            if close {
                return Ok(ForwardClose::TuiClose);
            }
        }
    });
    let budget = Arc::new(Semaphore::new(QUEUE_BYTES));
    let frames = Arc::new(Semaphore::new(QUEUE_FRAMES));
    let observed = observer.clone();
    let published = publication.clone();
    jobs.spawn(async move {
        loop {
            let Some(message) = or.next().await else {
                return Ok::<_, &'static str>(ForwardClose::OwnerEof);
            };
            let message = message.map_err(|_| "Owner transport read failed")?;
            let mut capture = None;
            if matches!(message, Message::Text(_) | Message::Binary(_)) {
                let bytes = message_bytes(&message).map_err(|_| "Owner data frame invalid")?;
                let mut observer = observed.lock().await;
                let changed = observer
                    .from_owner(bytes)
                    .map_err(|_| "Owner observation failed")?;
                capture = observer.queued_capture();
                if changed {
                    let (manifest, scope) = {
                        let mut publication = published.lock().await;
                        // Preserve the first correlated finite error even if source proof fails.
                        publication.retain_native_error(&observer);
                        (publication.manifest.clone(), publication.scope())
                    };
                    manifest_proof(manifest)
                        .await
                        .map_err(|_| "Owner/manifest source proof failed")?;
                    let mut publication = published.lock().await;
                    if publication.scope() != scope {
                        return Err("Relay publication source changed");
                    }
                    publication.presence.associated_thread_ids = observer.associated.clone();
                    publication.presence.resumed_thread_ids = observer.resumed.clone();
                    publication.presence.resume_rejected_thread = observer.resume_error.clone();
                    publication.retain_modes(&observer);
                    if publication.presence.error.is_none()
                        && observer.resume_error.as_ref() == Some(&publication.manifest.expected_thread_id)
                    {
                        publication.presence.error = Some(SafeMessage(
                            "Stock TUI rejected exact thread/resume; zero-turn rollout may be unmaterialized. No warm-up or substitute was attempted".into(),
                        ));
                    }
                    if !publication
                        .publish_if_current(&scope)
                        .map_err(|_| "Native observation publication failed")?
                    {
                        return Err("Relay publication source changed");
                    }
                }
            }
            let close = message.is_close();
            enqueue_marked(&to_tui, &budget, &frames, message, capture)
                .map_err(|_| "TUI-bound relay queue failed")?;
            if close {
                return Ok(ForwardClose::OwnerClose);
            }
        }
    });
    let observed = diagnostic.clone();
    jobs.spawn(async move {
        while let Some(queued) = owner_queue.recv().await {
            let capture = queued.capture;
            tokio::time::timeout(Duration::from_secs(5), ow.send(queued.message))
                .await
                .map_err(|_| "Owner write timed out")?
                .map_err(|_| "Owner write failed")?;
            if let Some(capture) = capture {
                if let Some(diagnostic) = observed.as_ref() {
                    diagnostic
                        .lock()
                        .expect("diagnostic capture mutex")
                        .sent(capture);
                }
            }
        }
        Ok::<_, &'static str>(ForwardClose::OwnerQueueDrained)
    });
    let observed = diagnostic.clone();
    jobs.spawn(async move {
        forward_owner_to_tui_observed(&mut tw, &mut tui_queue, observed).await
    });
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let result = async {
        loop {
            let heartbeat = async {
                tick.tick().await;
                let (manifest, tui, scope) = {
                    let publication = publication.lock().await;
                    (
                        publication.manifest.clone(),
                        publication.presence.tui.clone(),
                        publication.scope(),
                    )
                };
                manifest_proof(manifest.clone())
                    .await
                    .map_err(|_| ForwardFailure::plain("Owner/manifest source proof failed"))?;
                let terminal = match manifest.terminal_classified(tui.as_ref()).await {
                    Ok(terminal) => terminal,
                    Err(failure) if terminal_proof_unavailable(&failure) => {
                        let mut publication = publication.lock().await;
                        if !publication
                            .set_terminal_proof_if_current(
                                &scope,
                                TerminalProofAvailability::Unavailable,
                            )
                            .map_err(|_| {
                                ForwardFailure::plain("Relay presence publication failed")
                            })?
                        {
                            return Err(ForwardFailure::plain("Relay publication source changed"));
                        }
                        return Ok(());
                    }
                    Err(failure) => return Err(ForwardFailure::terminal(failure)),
                };
                if !terminal {
                    return Err(ForwardFailure::plain("Bound terminal closed"));
                }
                let mut publication = publication.lock().await;
                if !publication
                    .set_terminal_proof_if_current(&scope, TerminalProofAvailability::Available)
                    .map_err(|_| ForwardFailure::plain("Relay presence publication failed"))?
                {
                    return Err(ForwardFailure::plain("Relay publication source changed"));
                }
                Ok::<_, ForwardFailure>(())
            };
            tokio::select! {
                biased;
                result = jobs.join_next() => {
                    let result = result
                        .ok_or(ForwardFailure::plain("Relay workers absent"))?
                        .map_err(|_| ForwardFailure::plain("Relay worker failed"))?;
                    break match result {
                        Ok(close) if capture_closed => {
                            let capture = diagnostic
                                .as_ref()
                                .expect("capture enabled diagnostic")
                                .lock()
                                .expect("diagnostic capture mutex")
                                .freeze();
                            Err(ForwardFailure::closed(close, capture))
                        }
                        Ok(_) => Ok(()),
                        Err(error) => Err(ForwardFailure::plain(error)),
                    };
                },
                checked = heartbeat => {
                    if let Err(error) = checked {
                        break Err(error);
                    }
                },
            }
        }
    }
    .await;
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    result
}
enum AdmissionStop {
    Ignore,
    TerminalClosed,
}
struct TuiConnection {
    tui: ProcessEvidence,
    downstream: WebSocketStream<UnixStream>,
    upstream: WebSocketStream<UnixStream>,
}
type TuiAdmission = std::ops::ControlFlow<AdmissionStop, TuiConnection>;

fn ensure_admission_live(deadline: tokio::time::Instant) -> io::Result<()> {
    if tokio::time::Instant::now() >= deadline {
        return Err(invalid("TUI admission or reconnect grace expired"));
    }
    Ok(())
}

async fn ready_heartbeat(
    manifest: &Manifest,
    publication: &Mutex<Publication>,
    last_tui: Option<&ProcessEvidence>,
    deadline: tokio::time::Instant,
) -> io::Result<bool> {
    ensure_admission_live(deadline)?;
    manifest_proof(manifest.clone()).await?;
    ensure_admission_live(deadline)?;
    let live = manifest.terminal(last_tui).await?;
    ensure_admission_live(deadline)?;
    if !live {
        return Ok(false);
    }
    let mut publication = publication.lock().await;
    ensure_admission_live(deadline)?;
    publication.publish()?;
    ensure_admission_live(deadline)?;
    Ok(true)
}

async fn admission_with_heartbeat<F, H, P>(
    admission: F,
    tick: &mut tokio::time::Interval,
    deadline: tokio::time::Instant,
    mut heartbeat: H,
) -> io::Result<TuiAdmission>
where
    F: Future<Output = io::Result<TuiAdmission>>,
    H: FnMut() -> P,
    P: Future<Output = io::Result<bool>>,
{
    ensure_admission_live(deadline)?;
    tokio::pin!(admission);
    let expiry = tokio::time::sleep_until(deadline);
    tokio::pin!(expiry);
    loop {
        tokio::select! {
            biased;
            _ = &mut expiry => {
                return Err(invalid("TUI admission or reconnect grace expired"));
            }
            result = &mut admission => {
                ensure_admission_live(deadline)?;
                return result;
            }
            _ = tick.tick() => {
                ensure_admission_live(deadline)?;
                let checked = heartbeat();
                tokio::pin!(checked);
                tokio::select! {
                    biased;
                    _ = &mut expiry => {
                        return Err(invalid("TUI admission or reconnect grace expired"));
                    }
                    result = &mut admission => {
                        ensure_admission_live(deadline)?;
                        return result;
                    }
                    result = &mut checked => {
                        ensure_admission_live(deadline)?;
                        if !result? {
                            return Ok(TuiAdmission::Break(AdmissionStop::TerminalClosed));
                        }
                    }
                }
            }
        }
    }
}

/// The internal CLI owns only its listener/tasks. Companion absence is never closure.
pub async fn run(path: &Path) -> io::Result<()> {
    // SAFETY: internal dedicated service sets the file creation mask before bind/publication.
    unsafe {
        libc::umask(0o077);
    }
    let manifest = Manifest::load(path)?;
    let terminal_capture = terminal_capture_enabled(&manifest);
    let capture = StartupDiagnostic::new(terminal_capture);
    let relay = capture.observe("self_process", herdr::process_evidence(std::process::id()))?;
    match fs::symlink_metadata(presence_path(path)) {
        Ok(_) => {
            return capture.observe(
                "existing_presence",
                Err(invalid("existing relay presence cannot be adopted")),
            );
        }
        Err(_) => {}
    }
    let startup_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let (mut upstream, _) = startup_owner_stream(&manifest, startup_deadline, &capture).await?;
    let upstream_close = match tokio::time::timeout_at(startup_deadline, upstream.close(None)).await {
        Ok(result) => result.map_err(|_| invalid("upstream reachability close failed")),
        Err(_) => Err(startup_timeout()),
    };
    capture.observe("upstream_close", upstream_close)?;
    capture.observe("manifest_revalidation", ensure_startup_live(startup_deadline))?;
    let manifest_for_proof = manifest.clone();
    capture.observe(
        "manifest_revalidation",
        startup_blocking_proof(startup_deadline, move || manifest_for_proof.validate()).await,
    )?;
    let terminal = match tokio::time::timeout_at(startup_deadline, manifest.terminal(None)).await {
        Ok(result) => result,
        Err(_) => Err(startup_timeout()),
    };
    if !capture.observe("terminal", terminal)? {
        return capture.observe("terminal", Err(invalid("bound terminal already closed")));
    }
    capture.observe("terminal", ensure_startup_live(startup_deadline))?;
    let mut listener = capture.observe("listener", StartupListener::bind(&manifest.listen_host))?;
    capture.observe(
        "listener",
        fs::set_permissions(&manifest.listen_host, fs::Permissions::from_mode(0o600)),
    )?;
    let socket = capture.observe("listener", socket_object(&manifest.listen_host))?;
    if socket != listener.object {
        return capture.observe("listener", Err(invalid("bound listener object changed")));
    }
    capture.observe("listener", ensure_startup_live(startup_deadline))?;
    let publication = Arc::new(Mutex::new(Publication {
        path: path.to_owned(),
        manifest: manifest.clone(),
        presence: Presence {
            v: 1,
            binding_id: manifest.binding_id.clone(),
            owner_epoch: manifest.owner_epoch.clone(),
            relay,
            tui: None,
            connection_epoch: Uuid::random(),
            state: PresenceState::Ready,
            terminal_proof: TerminalProofAvailability::Available,
            associated_thread_ids: Vec::new(),
            resumed_thread_ids: Vec::new(),
            resume_rejected_thread: None,
            collaboration_mode: None,
            collaboration_default_plan: None,
            updated_at: Uint(pairing::now_ms()),
            error: None,
        },
        terminal_capture,
        pending_terminal_capture: None,
        pending_terminal_annotation: None,
    }));
    let mut startup_visible=false;
    let mut startup_object=None;
    let mut startup_complete=false;
    let mut initial_ready_completed_at_ms=None;
    let result=async {
        let initial = async {
            ensure_startup_live(startup_deadline)?;
            let mut initial_publication=tokio::time::timeout_at(startup_deadline,publication.lock()).await.map_err(|_|startup_timeout())?;
            ensure_startup_live(startup_deadline)?;
            initial_publication.publish_startup(startup_deadline,&mut startup_visible,&mut startup_object)?;
            if terminal_capture {
                initial_ready_completed_at_ms=Some(pairing::now_ms());
            }
            drop(initial_publication);
            ensure_startup_live(startup_deadline)
        }.await;
        capture.observe("initial_publication", initial)?;
        startup_complete=true;
        let mut deadline=tokio::time::Instant::now()+Duration::from_secs(120);let mut last_tui:Option<ProcessEvidence>=None;
        loop {
            let admitted = async {
                let mut tick=tokio::time::interval(Duration::from_secs(1));
                let accepted=loop {tokio::select!{
                    biased;
                    _=tokio::time::sleep_until(deadline)=>return Err(invalid("TUI admission or reconnect grace expired")),
                    accepted=listener.listener().accept()=>{
                        ensure_admission_live(deadline)?;
                        break accepted?
                    },
                    _=tick.tick()=>{
                        let live=tokio::time::timeout_at(deadline,ready_heartbeat(&manifest,&publication,last_tui.as_ref(),deadline))
                            .await.map_err(|_|invalid("TUI admission or reconnect grace expired"))??;
                        ensure_admission_live(deadline)?;
                        if !live {
                            publication.lock().await.clear(PresenceState::TerminalClosed,None)?;
                            return Ok(TuiAdmission::Break(AdmissionStop::TerminalClosed));
                        }
                    },
                }};
                // Until this single admission completes, no unproved live association is published.
                let admission = async {
                    let (stream, _) = accepted;
                    manifest_proof(manifest.clone()).await?;
                    let cred = stream.peer_cred()?;
                    if cred.uid() != pairing::uid() {
                        return Ok(TuiAdmission::Break(AdmissionStop::Ignore));
                    }
                    let tui = herdr::process_evidence(
                        cred.pid()
                            .and_then(|p| u32::try_from(p).ok())
                            .ok_or_else(|| invalid("TUI kernel PID unavailable"))?,
                    )?;
                    if last_tui.as_ref().is_some_and(|p| p != &tui) {
                        return Err(invalid("reconnect is not the same proven TUI process"));
                    }
                    if !manifest.terminal(Some(&tui)).await? {
                        return Ok(TuiAdmission::Break(AdmissionStop::TerminalClosed));
                    }
                    if herdr::process_object(&tui, &manifest.listen_agent)? != socket {
                        return Err(invalid("TUI/provider relay socket views differ"));
                    }
                    let downstream = match tokio::time::timeout(
                        Duration::from_secs(5),
                        accept_hdr_async_with_config(stream, OriginGuard, Some(websocket_config(FRAME_BYTES, QUEUE_BYTES))),
                    ).await {
                        Ok(Ok(ws)) => ws,
                        _ => return Ok(TuiAdmission::Break(AdmissionStop::Ignore)),
                    };
                    let (upstream, _) = owner_stream(
                        &manifest.upstream_host,
                        &manifest.upstream_agent,
                        &manifest.owner_epoch,
                        websocket_config(FRAME_BYTES, QUEUE_BYTES),
                    ).await?;
                    Ok(TuiAdmission::Continue(TuiConnection { tui, downstream, upstream }))
                };
                let admitted = admission_with_heartbeat(
                    admission, &mut tick, deadline,
                    || ready_heartbeat(&manifest, &publication, last_tui.as_ref(), deadline),
                ).await?;
                match &admitted {
                    TuiAdmission::Break(AdmissionStop::Ignore) => {}
                    TuiAdmission::Break(AdmissionStop::TerminalClosed) => {
                        publication.lock().await.clear(PresenceState::TerminalClosed, None)?;
                    }
                    TuiAdmission::Continue(connection) => {
                        let mut p=publication.lock().await;
                        ensure_admission_live(deadline)?;
                        p.presence.connection_epoch=Uuid::random();
                        p.presence.tui=Some(connection.tui.clone());
                        p.presence.state=PresenceState::Connected;
                        p.presence.terminal_proof=TerminalProofAvailability::Available;
                        p.presence.error=None;
                        p.presence.resume_rejected_thread=None;
                        p.presence.collaboration_mode=None;
                        p.presence.collaboration_default_plan=None;
                        ensure_admission_live(deadline)?;
                        p.publish()?;
                    }
                }
                Ok::<_, io::Error>(admitted)
            }.await?;
            let (tui, ws, owner) = match admitted {
                TuiAdmission::Break(AdmissionStop::Ignore) => continue,
                TuiAdmission::Break(AdmissionStop::TerminalClosed) => return Ok(()),
                TuiAdmission::Continue(TuiConnection { tui, downstream, upstream }) =>
                    (tui, downstream, upstream),
            };
            let active=forward(ws,owner,publication.clone());tokio::pin!(active);
            let mut listener_failed=false;
            let disconnected=loop{tokio::select!{r=&mut active=>break r,accepted=listener.listener().accept(),if !listener_failed=>{match accepted{Ok((mut second,_))=>{let _=tokio::time::timeout(Duration::from_secs(1),second.write_all(b"HTTP/1.1 409 Conflict\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")).await;},Err(_)=>listener_failed=true}}}};
            let terminal_capture = disconnected
                .as_ref()
                .err()
                .and_then(|failure| failure.terminal_capture);
            let closed_capture = disconnected
                .as_ref()
                .err()
                .and_then(|failure| failure.closed_capture.clone());
            let disconnected_error = disconnected
                .as_ref()
                .err()
                .map(|failure| failure.error)
                .unwrap_or("Native relay forwarding closed");
            {
                let mut publication_guard = publication.lock().await;
                if let Some((terminal_error, capture)) = terminal_capture {
                    publication_guard.retain_terminal_capture(terminal_error, capture);
                }
                if let Some(capture) = closed_capture {
                    publication_guard.retain_terminal_annotation(disconnected_error, capture);
                }
                publication_guard.clear(PresenceState::Disconnected, Some(disconnected_error))?;
            }
            if listener_failed{return Err(invalid("relay listener failed after its owned active transport settled"))}
            manifest_proof(manifest.clone()).await?;
            match manifest.terminal(Some(&tui)).await {Ok(false)=>{publication.lock().await.clear(PresenceState::TerminalClosed,None)?;return Ok(())},Ok(true)=>{},Err(e)=>return Err(e)}
            last_tui=Some(tui);deadline=tokio::time::Instant::now()+Duration::from_secs(30);
        }
    }.await;
    if result.is_err()
        && (startup_complete
            || (startup_visible
                && startup_object.is_some_and(|object| exact_private_object(&presence_path(path),object))))
    {
        let terminal_error="Relay identity, transport or bounded admission failed; explicit reconciliation required";
        let mut publication=publication.lock().await;
        if let Some(initial_ready_completed_at_ms)=initial_ready_completed_at_ms {
            publication.retain_terminal_annotation(terminal_error,SafeMessage(format!(
                "{terminal_error} initial_ready=completed initial_ready_completed_at_ms={initial_ready_completed_at_ms}"
            )));
        }
        let _=publication.clear(PresenceState::Failed,Some(terminal_error));
    }
    listener.cleanup()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_json::NATIVE;
    use serde_json::json;
    use tokio_tungstenite::tungstenite::{Error as WebSocketError, error::CapacityError};

    struct PrivateReadRoot(PathBuf);
    impl PrivateReadRoot {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            let path = std::env::temp_dir().join(format!("goatr-read-{}", Uuid::random().0));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for PrivateReadRoot {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn private_read_retries_retired_publications_without_returning_old_bytes() {
        for phase in [PrivateReadPhase::Opened, PrivateReadPhase::Read] {
            let root = PrivateReadRoot::new();
            let path = root.0.join("presence");
            private_write(&path, &json!({"revision":1}), false).unwrap();
            let mut replaced = false;
            let mut opened = 0;
            let value: serde_json::Value = private_read_checked(&path, |at| {
                opened += usize::from(at == PrivateReadPhase::Opened);
                if at == phase && !replaced {
                    private_write(&path, &json!({"revision":2}), true).unwrap();
                    replaced = true;
                }
            })
            .unwrap();
            assert_eq!(opened, 2);
            assert_eq!(value, json!({"revision":2}));
        }
        let root = PrivateReadRoot::new();
        let path = root.0.join("presence");
        private_write(&path, &json!({"revision":0}), false).unwrap();
        let mut opened = 0;
        let result: io::Result<serde_json::Value> = private_read_checked(&path, |at| {
            if at == PrivateReadPhase::Opened {
                opened += 1;
                private_write(&path, &json!({"revision":opened}), true).unwrap();
            } else {
                panic!("retired descriptor must not be read");
            }
        });
        assert_eq!(opened, 3);
        let error = result.unwrap_err();
        assert!(publication_unstable(&error));
        assert!(!publication_unstable(&io::Error::new(
            error.kind(),
            error.to_string()
        )));
    }

    #[test]
    fn private_read_rejects_unsafe_files_and_replacements_without_retry() {
        use std::os::unix::fs::symlink;
        for replacement in [false, true] {
            for policy in ["hardlink", "symlink", "mode", "directory"] {
                let root = PrivateReadRoot::new();
                let path = root.0.join("presence");
                private_write(&path, &json!({"revision":1}), false).unwrap();
                let violate = || match policy {
                    "hardlink" => fs::hard_link(&path, root.0.join("alias")).unwrap(),
                    "symlink" => {
                        fs::rename(&path, root.0.join("target")).unwrap();
                        symlink(root.0.join("target"), &path).unwrap();
                    }
                    "mode" => {
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap()
                    }
                    "directory" => {
                        fs::remove_file(&path).unwrap();
                        fs::create_dir(&path).unwrap();
                    }
                    _ => unreachable!(),
                };
                let mut opened = 0;
                if !replacement {
                    violate();
                }
                let result: io::Result<serde_json::Value> = private_read_checked(&path, |at| {
                    opened += usize::from(at == PrivateReadPhase::Opened);
                    if replacement && at == PrivateReadPhase::Read {
                        // A different pathname object must still pass every policy guard.
                        private_write(&path, &json!({"revision":2}), true).unwrap();
                        violate();
                    }
                });
                let error = result.unwrap_err();
                assert!(!publication_unstable(&error));
                assert!(opened <= 1);
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                assert_eq!(error.to_string(), "unsafe private path");
            }
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn held_content_cache_reuses_only_unchanged_file_not_process_authority() {
        use std::fs::FileTimes;
        use std::os::unix::fs::DirBuilderExt;

        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(std::env::temp_dir().join(format!("dc-{}", &Uuid::random().0[..8])));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let path = root.0.join("content");
        fs::write(&path, b"original").unwrap();
        let held = File::open(&path).unwrap();
        let before = held.metadata().unwrap();
        let captured = executable_stamp(&before);
        let proofs = parking_lot::Mutex::new(Vec::new());
        // This exercises retained-file identity only; it is not a native publisher admission.
        remember_distribution_content(
            &proofs,
            DistributionProof {
                path: path.clone(),
                file: Arc::new(held),
                stamp: captured,
            },
        );
        let second_open = File::open(&path).unwrap();
        let same_content = executable_stamp(&second_open.metadata().unwrap());
        assert_eq!(same_content, captured);
        assert!(distribution_content_hit(&proofs, &path, same_content));
        assert!(distribution_content_hit(&proofs, &path, captured));
        assert!(!distribution_content_hit(
            &proofs,
            &root.0.join("other-path"),
            captured
        ));

        let mut stale_process = herdr::process_evidence(std::process::id()).unwrap();
        stale_process.start_ticks += 1;
        let denied = verify_distribution(&stale_process).unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::InvalidData);
        assert!(distribution_content_hit(&proofs, &path, captured));

        let original_mtime = before.modified().unwrap();
        let original_ctime = (before.ctime(), before.ctime_nsec());
        let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
        writer.write_all(b"modified").unwrap();
        writer.sync_all().unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(2);
        let changed = loop {
            writer
                .set_times(FileTimes::new().set_modified(original_mtime))
                .unwrap();
            let metadata = writer.metadata().unwrap();
            if (metadata.ctime(), metadata.ctime_nsec()) != original_ctime {
                break metadata;
            }
            assert!(
                std::time::Instant::now() < until,
                "filesystem ctime must advance"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(changed.len(), before.len());
        assert_eq!(changed.modified().unwrap(), original_mtime);
        assert_ne!((changed.ctime(), changed.ctime_nsec()), original_ctime);
        assert!(!distribution_content_hit(
            &proofs,
            &path,
            executable_stamp(&changed)
        ));
        // Even a previously captured stamp cannot reuse a now-mutated retained descriptor.
        assert!(!distribution_content_hit(&proofs, &path, captured));
    }
    #[tokio::test]
    async fn opaque_stock_message_reaches_peer_without_widening_api_budget() {
        use tokio_tungstenite::tungstenite::protocol::Role;

        let opaque_members = NATIVE.members + 1;
        let mut text = String::with_capacity(NATIVE.bytes + opaque_members * 2 + 256);
        text.push_str(r#"{"id":-0,"result":{"opaque":{"items":["#);
        for _ in 0..opaque_members {
            text.push_str("0,");
        }
        text.push_str(r#"0],"text":""#);
        text.extend(std::iter::repeat_n('x', NATIVE.bytes + 1));
        text.push_str(r#"","unicode":"\uD834\uDD1E"},"thread":{"id":"T"}}}"#);
        assert!(text.len() > NATIVE.bytes && text.len() < FRAME_BYTES);
        assert!(native_json::parse(text.as_bytes(), NATIVE).is_err());
        assert!(
            native_json::parse(
                text.as_bytes(),
                native_json::Limits {
                    bytes: FRAME_BYTES,
                    ..NATIVE
                }
            )
            .is_err()
        );
        let expected = Message::Text(text.into());

        let (owner_socket, relay_socket) = UnixStream::pair().unwrap();
        let mut owner = WebSocketStream::from_raw_socket(
            owner_socket,
            Role::Server,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let mut relay_reader = WebSocketStream::from_raw_socket(
            relay_socket,
            Role::Client,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let owner_message = expected.clone();
        let owner_send = tokio::spawn(async move {
            owner.send(owner_message).await.unwrap();
        });
        let incoming = tokio::time::timeout(Duration::from_secs(5), relay_reader.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut observer = Observer::default();
        observer
            .from_tui(br#"{"id":-0,"method":"thread/resume","params":{"threadId":"T"}}"#)
            .unwrap();
        assert!(
            !observer
                .from_owner(br#"{"id":0,"result":{"thread":{"id":"T"}}}"#)
                .unwrap()
        );
        assert!(
            !observer
                .from_owner(br#"{"id":"-0","result":{"thread":{"id":"T"}}}"#)
                .unwrap()
        );
        assert!(
            observer
                .from_owner(message_bytes(&incoming).unwrap())
                .unwrap()
        );
        assert_eq!(observer.associated(), &[Id("T".into())]);
        assert_eq!(observer.resumed(), &[Id("T".into())]);

        let (tx, mut rx) = mpsc::channel(QUEUE_FRAMES);
        let bytes = Arc::new(Semaphore::new(QUEUE_BYTES));
        let frames = Arc::new(Semaphore::new(QUEUE_FRAMES));
        let length = incoming.len();
        enqueue(&tx, &bytes, &frames, incoming).unwrap();
        assert_eq!(bytes.available_permits(), QUEUE_BYTES - length);
        let (relay_socket, tui_socket) = UnixStream::pair().unwrap();
        let mut relay_writer = WebSocketStream::from_raw_socket(
            relay_socket,
            Role::Server,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let mut tui = WebSocketStream::from_raw_socket(
            tui_socket,
            Role::Client,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let relay_send = tokio::spawn(async move {
            let queued = rx.recv().await.unwrap();
            relay_writer.send(queued.message).await.unwrap();
        });
        let delivered = tokio::time::timeout(Duration::from_secs(5), tui.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(delivered, expected);
        owner_send.await.unwrap();
        relay_send.await.unwrap();
        assert_eq!(bytes.available_permits(), QUEUE_BYTES);
        assert_eq!(frames.available_permits(), QUEUE_FRAMES);
        assert!(
            !observer
                .from_owner(br#"{"id":-0,"result":{"thread":{"id":"substitute"}}}"#)
                .unwrap()
        );
        assert_eq!(observer.resumed(), &[Id("T".into())]);

        let (owner_socket, api_socket) = UnixStream::pair().unwrap();
        let mut api_owner = WebSocketStream::from_raw_socket(
            owner_socket,
            Role::Server,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let mut api = WebSocketStream::from_raw_socket(
            api_socket,
            Role::Client,
            Some(websocket_config(NATIVE.bytes, 16 * 1024 * 1024)),
        )
        .await;
        let api_send = tokio::spawn(async move { api_owner.send(expected).await });
        let api_error = tokio::time::timeout(Duration::from_secs(5), api.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(api_error, WebSocketError::Capacity(CapacityError::MessageTooLong { size, max_size })
            if size == length && max_size == NATIVE.bytes)
        );
        drop(api);
        let _ = api_send.await.unwrap();

        let (mut peer, relay_socket) = UnixStream::pair().unwrap();
        let mut limited = WebSocketStream::from_raw_socket(
            relay_socket,
            Role::Client,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let mut header = [0_u8; 10];
        header[0] = 0x81;
        header[1] = 127;
        header[2..].copy_from_slice(&(FRAME_BYTES as u64 + 1).to_be_bytes());
        peer.write_all(&header).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), limited.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(error, WebSocketError::Capacity(CapacityError::MessageTooLong { size, max_size })
            if size == FRAME_BYTES + 1 && max_size == FRAME_BYTES)
        );

        // These malformed/ambiguous native frames cross a real Unix WebSocket,
        // but can never establish an observed control association.
        let mut rejected: Vec<(Vec<u8>, bool, bool)> = [
            (br#"[]"#.as_slice(), false, false),
            (br#"null"#.as_slice(), false, false),
            (br#"{"id":null,"result":{"thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"\u0069d":1,"result":{"thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":null,"\u0069d":1,"result":{"thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":null,"\u0072esult":{"thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":[01],"thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":[1,],"thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":"\uD800","thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":"\uDC00","thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":"\uD800\u0041","thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":"\uXX00","thread":{"id":"T"}}}"#.as_slice(), false, false),
            (br#"{"id":1,"result":{"opaque":"\q","thread":{"id":"T"}}}"#.as_slice(), false, false),
            (b"{\"id\":1,\"result\":{\"opaque\":\"\xff\",\"thread\":{\"id\":\"T\"}}}".as_slice(), false, false),
            (b"{\"id\":1,\"result\":{\"opaque\":\"\x01\",\"thread\":{\"id\":\"T\"}}}".as_slice(), false, false),
            (br#"{"id":1,"result":{"thread":{"id":"T"}}} null"#.as_slice(), false, false),
            (br#"{"id":1}"#.as_slice(), false, true),
            (br#"{"id":1,"result":[]}"#.as_slice(), false, true),
            (br#"{"id":1,"result":null}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":[]}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":null}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":{"id":null}}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":{"id":[]}}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":{"id":"T","\u0069d":"T"}}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":{"id":"T"},"\u0074hread":{"id":"T"}}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":{"thread":{"id":"substitute"}}}"#.as_slice(), false, true),
            (br#"{"id":1,"error":[]}"#.as_slice(), false, true),
            (br#"{"id":1,"error":null}"#.as_slice(), false, true),
            (br#"{"id":1,"error":{"code":null,"message":"rejected"}}"#.as_slice(), false, true),
            (br#"{"id":1,"error":{"code":-1,"message":[]}}"#.as_slice(), false, true),
            (br#"{"id":1,"error":{"code":-1,"\u0063ode":-2,"message":"rejected"}}"#.as_slice(), false, true),
            (br#"{"id":1,"result":null,"error":{"code":-1,"message":"rejected"}}"#.as_slice(), false, true),
            (br#"[]"#.as_slice(), true, false),
            (br#"{"id":1,"method":null,"params":{}}"#.as_slice(), true, false),
            (br#"{"id":null,"method":"thread/resume","params":{"threadId":"T"}}"#.as_slice(), true, false),
            (br#"{"id":1,"method":"thread/resume","params":[]}"#.as_slice(), true, false),
            (br#"{"id":1,"method":"thread/resume","params":null}"#.as_slice(), true, false),
            (br#"{"id":1,"method":"thread/resume","params":{"threadId":null}}"#.as_slice(), true, false),
            (br#"{"id":1,"method":"thread/resume","params":{"threadId":"T","\u0074hreadId":"T"}}"#.as_slice(), true, false),
            (br#"{"id":1,"method":"thread/start","params":{"key":1,"\u006bey":2}}"#.as_slice(), true, false),
        ].into_iter().map(|(bytes, request, consumed)| (bytes.to_vec(), request, consumed)).collect();
        let mut deep = String::from(r#"{"id":1,"result":{"opaque":"#);
        deep.extend(std::iter::repeat_n('[', BRIDGE_JSON_DEPTH));
        deep.push('0');
        deep.extend(std::iter::repeat_n(']', BRIDGE_JSON_DEPTH));
        deep.push_str(r#","thread":{"id":"T"}}}"#);
        rejected.push((deep.into_bytes(), false, false));
        let (sender_socket, reader_socket) = UnixStream::pair().unwrap();
        let mut sender = WebSocketStream::from_raw_socket(
            sender_socket,
            Role::Server,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        let mut reader = WebSocketStream::from_raw_socket(
            reader_socket,
            Role::Client,
            Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
        )
        .await;
        for (bytes, request, consumed) in rejected {
            sender.send(Message::Binary(bytes.into())).await.unwrap();
            let message = tokio::time::timeout(Duration::from_secs(5), reader.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let mut observer = Observer::default();
            if request {
                assert!(observer.from_tui(message_bytes(&message).unwrap()).is_err());
            } else {
                observer
                    .from_tui(br#"{"id":1,"method":"thread/resume","params":{"threadId":"T"}}"#)
                    .unwrap();
                assert!(
                    observer
                        .from_owner(message_bytes(&message).unwrap())
                        .is_err()
                );
                if consumed {
                    assert!(
                        !observer
                            .from_owner(br#"{"id":1,"result":{"thread":{"id":"T"}}}"#)
                            .unwrap()
                    );
                }
            }
            assert!(observer.associated().is_empty());
            assert!(observer.resumed().is_empty());
        }
    }
    fn publication_fixture(root: &Path) -> Publication {
        let process = herdr::process_evidence(std::process::id()).unwrap();
        let pair = || ["0".to_owned(), "0".to_owned()];
        // Inert private-record metadata, deliberately invalid for native admission.
        let epoch = OwnerEpoch {
            linux_boot_id: process.linux_boot_id.clone(),
            pid: process.pid,
            start_ticks: process.start_ticks.to_string(),
            socket_device: "0".into(),
            socket_inode: "0".into(),
            executable_sha256: "not-a-publisher-proof".into(),
            socket_source: SocketSource {
                root: pair(),
                host_parent: pair(),
                host_entry: pair(),
                agent_parent: pair(),
                alias: pair(),
                physical_parent: pair(),
            },
        };
        let binding = Uuid::random();
        let path = manifest_path(root, &binding);
        let manifest = Manifest {
            v: 1,
            binding_id: binding.clone(),
            owner_id: Uuid::random(),
            upstream_host: root.join("c.sock"),
            upstream_agent: root.join("c.sock"),
            listen_host: root.join("t.sock"),
            listen_agent: root.join("t.sock"),
            owner_epoch: epoch.clone(),
            herdr: HerdrBinding {
                server_id: Uuid::random(),
                incarnation: Uuid::random(),
                workspace_id: Id("workspace".into()),
                terminal_id: Id("terminal".into()),
                pane_id: Id("pane".into()),
                socket_path: root.join("h.sock"),
                evidence: crate::storage::ServerEvidence {
                    linux_boot_id: process.linux_boot_id.clone(),
                    pid: process.pid,
                    pid_start_ticks: process.start_ticks,
                    socket_device: 0,
                    socket_inode: 0,
                },
                shell: process.clone(),
            },
            expected_thread_id: Id("fixture-thread".into()),
            created_at: Uint(pairing::now_ms()),
        };
        Publication {
            path,
            manifest,
            presence: Presence {
                v: 1,
                binding_id: binding,
                owner_epoch: epoch,
                relay: process.clone(),
                tui: Some(process),
                connection_epoch: Uuid::random(),
                state: PresenceState::Connected,
                terminal_proof: TerminalProofAvailability::Available,
                associated_thread_ids: vec![Id("prior-association".into())],
                resumed_thread_ids: vec![Id("prior-association".into())],
                resume_rejected_thread: None,
                collaboration_mode: None,
                collaboration_default_plan: None,
                updated_at: Uint(pairing::now_ms()),
                error: None,
            },
            terminal_capture: false,
            pending_terminal_capture: None,
            pending_terminal_annotation: None,
        }
    }

    #[tokio::test]
    async fn framed_mode_observation_is_correlated_private_and_connection_scoped() {
        use std::os::unix::fs::DirBuilderExt;
        use tokio_tungstenite::tungstenite::protocol::Role;
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = Root(std::env::temp_dir().join(format!("goatr-mode-{}", Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let mut publication = publication_fixture(&root.0);
        let thread = publication.manifest.expected_thread_id.clone();
        let mut observer = Observer::for_thread(thread.clone());
        let (tui, relay) = UnixStream::pair().unwrap();
        let mut tui = WebSocketStream::from_raw_socket(tui, Role::Client, None).await;
        let mut tui_reader = WebSocketStream::from_raw_socket(relay, Role::Server, None).await;
        let (owner, relay) = UnixStream::pair().unwrap();
        let mut owner = WebSocketStream::from_raw_socket(owner, Role::Server, None).await;
        let mut owner_reader = WebSocketStream::from_raw_socket(relay, Role::Client, None).await;
        async fn request(
            sender: &mut WebSocketStream<UnixStream>,
            reader: &mut WebSocketStream<UnixStream>,
            observer: &mut Observer,
            value: serde_json::Value,
        ) {
            sender
                .send(Message::Text(value.to_string().into()))
                .await
                .unwrap();
            let message = reader.next().await.unwrap().unwrap();
            observer.from_tui(message_bytes(&message).unwrap()).unwrap();
        }
        async fn reply(
            sender: &mut WebSocketStream<UnixStream>,
            reader: &mut WebSocketStream<UnixStream>,
            observer: &mut Observer,
            value: serde_json::Value,
        ) {
            sender
                .send(Message::Text(value.to_string().into()))
                .await
                .unwrap();
            let message = reader.next().await.unwrap().unwrap();
            observer
                .from_owner(message_bytes(&message).unwrap())
                .unwrap();
        }
        request(
            &mut tui,
            &mut tui_reader,
            &mut observer,
            json!({"id":"PRIVATE_RPC","method":"thread/resume","params":{"threadId":thread}}),
        )
        .await;
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":"foreign","result":{"thread":{"id":thread},"collaborationMode":{"mode":"plan"}}})).await;
        assert!(observer.collaboration_mode.is_none());
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":"PRIVATE_RPC","result":{"thread":{"id":thread},"collaborationMode":{
                "mode":"default","settings":{"model":"PRIVATE_MODEL","developer_instructions":"PRIVATE_INSTRUCTIONS"}}}})).await;
        let original = observer.collaboration_mode.unwrap();
        assert_eq!(original.mode, CollaborationModeKind::Default);
        assert!(!original.from_settings);
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":"PRIVATE_RPC","result":{"thread":{"id":thread},"collaborationMode":{"mode":"plan"}}})).await;
        assert_eq!(observer.collaboration_mode, Some(original));
        reply(
            &mut owner,
            &mut owner_reader,
            &mut observer,
            json!({"method":"thread/settings/updated","params":{"threadId":"foreign",
                "threadSettings":{"collaborationMode":{"mode":"plan"}}}}),
        )
        .await;
        assert_eq!(observer.collaboration_mode, Some(original));
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"method":"thread/settings/updated","params":{"threadId":thread,
                "threadSettings":{"collaborationMode":{"mode":"plan","settings":{"model":"PRIVATE_UPDATED_MODEL"}}}}})).await;
        let changed = observer.collaboration_mode.unwrap();
        assert_eq!(changed.mode, CollaborationModeKind::Plan);
        assert!(changed.from_settings && changed.revision > original.revision);
        for id in ["old-catalogue", "new-catalogue"] {
            request(
                &mut tui,
                &mut tui_reader,
                &mut observer,
                json!({"id":id,"method":"collaborationMode/list","params":{}}),
            )
            .await;
        }
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":"new-catalogue","result":{"data":[{"mode":"default","name":"PRIVATE_NAME","model":"PRIVATE_MODEL"},{"mode":"plan"}]}})).await;
        reply(
            &mut owner,
            &mut owner_reader,
            &mut observer,
            json!({"id":"old-catalogue","result":{"data":[{"mode":"plan"}]}}),
        )
        .await;
        assert_eq!(observer.collaboration_default_plan, Some(true));
        publication.retain_modes(&observer);
        publication.publish().unwrap();
        let record = fs::read(presence_path(&publication.path)).unwrap();
        assert!(
            !record
                .windows(b"PRIVATE_".len())
                .any(|window| window == b"PRIVATE_")
        );
        for (id, target) in [
            ("late-original", thread.0.as_str()),
            ("replacement", "different-thread"),
        ] {
            request(
                &mut tui,
                &mut tui_reader,
                &mut observer,
                json!({"id":id,"method":"thread/resume","params":{"threadId":target}}),
            )
            .await;
        }
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":"replacement","result":{"thread":{"id":"different-thread"},"collaborationMode":{"mode":"plan"}}})).await;
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":"late-original","result":{"thread":{"id":thread},"collaborationMode":{"mode":"default"}}})).await;
        reply(
            &mut owner,
            &mut owner_reader,
            &mut observer,
            json!({"method":"thread/settings/updated","params":{"threadId":thread,
                "threadSettings":{"collaborationMode":{"mode":"default"}}}}),
        )
        .await;
        assert!(observer.collaboration_mode.is_none());
        request(
            &mut tui,
            &mut tui_reader,
            &mut observer,
            json!({"id":23,"method":"thread/resume","params":{"threadId":thread}}),
        )
        .await;
        reply(&mut owner,&mut owner_reader,&mut observer,
            json!({"id":23,"result":{"thread":{"id":thread},"collaborationMode":{"mode":"unsupported"}}})).await;
        assert!(observer.resumed().contains(&thread));
        assert!(
            observer.collaboration_mode.is_none(),
            "unknown mode must not invent Default or deny ordinary resume"
        );
        publication
            .clear(PresenceState::Disconnected, None)
            .unwrap();
        let old_epoch = publication.presence.connection_epoch.clone();
        publication.presence.connection_epoch = Uuid::random();
        let mut fresh = Observer::for_thread(thread);
        reply(
            &mut owner,
            &mut owner_reader,
            &mut fresh,
            json!({"id":23,"result":{"collaborationMode":{"mode":"default"}}}),
        )
        .await;
        publication.retain_modes(&fresh);
        publication.publish().unwrap();
        let saved: Presence = private_read(&presence_path(&publication.path)).unwrap();
        assert_ne!(saved.connection_epoch, old_epoch);
        assert!(saved.collaboration_mode.is_none() && saved.collaboration_default_plan.is_none());
    }

    #[test]
    fn original_presence_phase_preserves_io_error_and_private_context() {
        use std::os::unix::fs::DirBuilderExt;

        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(std::env::temp_dir().join(format!("gp-{}", &Uuid::random().0[..8])));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let mut record = publication_fixture(&root.0);
        let assert_original = |failure: &PresenceFailure, original: &io::Error| {
            assert_eq!(failure.error.kind(), original.kind());
            assert_eq!(failure.error.raw_os_error(), original.raw_os_error());
            assert_eq!(failure.error.to_string(), original.to_string());
        };

        let missing_record = record.manifest.presence_checked(&record.path).unwrap_err();
        assert_eq!(missing_record.phase, PresencePhase::PresenceRecord);
        assert_eq!(missing_record.error.kind(), io::ErrorKind::NotFound);
        assert_eq!(missing_record.error.raw_os_error(), Some(libc::ENOENT));
        assert_original(
            &missing_record,
            &record.manifest.presence(&record.path).unwrap_err(),
        );

        record.presence.relay.pid = i32::MAX as u32;
        private_write(&presence_path(&record.path), &record.presence, false).unwrap();
        let missing_process = record.manifest.presence_checked(&record.path).unwrap_err();
        assert_eq!(missing_process.phase, PresencePhase::RelayProcess);
        assert_eq!(missing_process.error.kind(), io::ErrorKind::NotFound);
        assert_eq!(missing_process.error.raw_os_error(), Some(libc::ENOENT));
        assert_original(
            &missing_process,
            &record.manifest.presence(&record.path).unwrap_err(),
        );

        record.manifest.listen_host = root.0.join("absent").join("t.sock");
        let missing_parent = record.manifest.validate_checked().unwrap_err();
        assert_eq!(missing_parent.phase, PresencePhase::ListenParent);
        assert_eq!(missing_parent.error.kind(), io::ErrorKind::NotFound);
        assert_eq!(missing_parent.error.raw_os_error(), Some(libc::ENOENT));
        assert_original(&missing_parent, &record.manifest.validate().unwrap_err());

        let private = PresenceFailure::at(
            PresencePhase::OwnerProof,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "/PRIVATE_NATIVE_PATH payload",
            ),
        );
        let diagnostic = format!(
            "presence_phase={}; io_kind={:?}; errno={:?}",
            private.phase.as_str(),
            private.error.kind(),
            private.error.raw_os_error()
        );
        assert!(!diagnostic.contains("PRIVATE_NATIVE") && !diagnostic.contains("payload"));
        assert_eq!(private.error.to_string(), "/PRIVATE_NATIVE_PATH payload");
    }

    #[test]
    fn presence_age_is_decided_after_proof_without_accepting_future_records() {
        let root = PrivateReadRoot::new();
        let mut publication = publication_fixture(&root.0);
        assert!(matches!(freshness_at_completion(5_000, 10_000), Ok(true)));
        assert!(matches!(freshness_at_completion(5_000, 10_001), Ok(false)));
        let failure = freshness_at_completion(12_000, 10_000).err().unwrap();
        assert_eq!(failure.phase, PresencePhase::Guard);
        assert_eq!(
            failure.error.to_string(),
            "Codex relay presence more than one second ahead"
        );
        publication.presence.terminal_proof = TerminalProofAvailability::Unavailable;
        publication.presence.updated_at = Uint(pairing::now_ms().saturating_add(10_000));
        let path = presence_path(&publication.path);
        private_write(&path, &publication.presence, path.exists()).unwrap();
        let failure = publication
            .manifest
            .validated_presence(&publication.path)
            .err()
            .unwrap();
        assert_eq!(failure.phase, PresencePhase::Guard);
        assert_eq!(
            failure.error.to_string(),
            "Codex relay presence more than one second ahead"
        );
    }

    #[tokio::test]
    async fn blocked_proof_leaves_heartbeat_lock_free_and_cannot_publish_after_scope_loss() {
        let root = PrivateReadRoot::new();
        let mut record = publication_fixture(&root.0);
        record.publish().unwrap();
        let path = presence_path(&record.path);
        let publication = Arc::new(Mutex::new(record));
        let scope = publication.lock().await.scope();
        let (entered, begun) = tokio::sync::oneshot::channel::<()>();
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let proof = tokio::spawn(async move {
            blocking_proof(move || {
                entered
                    .send(())
                    .map_err(|_| invalid("proof observer absent"))?;
                held.blocking_recv()
                    .map_err(|_| invalid("proof release absent"))?;
                Ok(())
            })
            .await
        });
        begun.await.unwrap();
        {
            let mut current = tokio::time::timeout(Duration::from_secs(1), publication.lock())
                .await
                .expect("blocked proof must not hold publication lock");
            assert!(current.publish_if_current(&scope).unwrap());
            current.clear(PresenceState::Disconnected, None).unwrap();
        }
        let closed: Presence = private_read(&path).unwrap();
        release.send(()).unwrap();
        proof.await.unwrap().unwrap();
        assert!(!publication.lock().await.publish_if_current(&scope).unwrap());
        let after: Presence = private_read(&path).unwrap();
        assert_eq!(after.updated_at, closed.updated_at);
        assert_eq!(after.state, PresenceState::Disconnected);
        assert!(after.associated_thread_ids.is_empty());
    }

    #[tokio::test]
    async fn admission_heartbeat_keeps_ready_fresh_and_cancels_failed_admission() {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncReadExt;

        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(std::env::temp_dir().join(format!("gh-{}", &Uuid::random().0[..8])));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let mut record = publication_fixture(&root.0);
        record.presence.state = PresenceState::Ready;
        record.presence.tui = None;
        record.presence.associated_thread_ids.clear();
        record.presence.resumed_thread_ids.clear();
        record.publish().unwrap();
        let path = presence_path(&record.path);
        let process = record.presence.relay.clone();
        let epoch = record.presence.connection_epoch.clone();
        let initial = record.presence.updated_at.0;
        let publication = Mutex::new(record);
        let attempts = AtomicUsize::new(0);
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let (server, client) = UnixStream::pair().unwrap();
        // Real private-file publication and Unix upgrade; this inert manifest is never
        // accepted as publisher/native Ready proof. Hold blocking admission work beyond5s.
        let admission = async {
            attempts.fetch_add(1, Ordering::Relaxed);
            tokio::task::spawn_blocking(move || held.blocking_recv())
                .await
                .map_err(|_| invalid("held admission worker failed"))?
                .map_err(|_| invalid("held admission cancelled"))?;
            let socket = accept_hdr_async_with_config(
                server,
                OriginGuard,
                Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
            )
            .await
            .map_err(|_| invalid("fixture Unix upgrade failed"))?;
            drop(socket);
            Ok(TuiAdmission::Break(AdmissionStop::Ignore))
        };
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let awaiting = admission_with_heartbeat(
            admission,
            &mut tick,
            tokio::time::Instant::now() + Duration::from_secs(15),
            || async {
                if herdr::process_evidence(process.pid)? != process {
                    return Err(invalid("fixture publisher epoch changed"));
                }
                publication.lock().await.publish()?;
                Ok(true)
            },
        );
        let observe = async {
            let until = tokio::time::Instant::now() + Duration::from_millis(6_100);
            let mut reads = tokio::time::interval(Duration::from_millis(200));
            while tokio::time::Instant::now() < until {
                reads.tick().await;
                let saved: Presence = private_read(&path).unwrap();
                assert_eq!(saved.state, PresenceState::Ready);
                assert!(saved.tui.is_none());
                assert!(saved.associated_thread_ids.is_empty());
                assert!(saved.resumed_thread_ids.is_empty());
                assert_eq!(saved.connection_epoch, epoch);
                assert_eq!(saved.relay, process);
                let now = pairing::now_ms();
                assert!(now.saturating_sub(saved.updated_at.0) <= 5_000);
                assert!(saved.updated_at.0 <= now.saturating_add(1_000));
            }
            let saved: Presence = private_read(&path).unwrap();
            assert!(saved.updated_at.0 >= initial + 5_000);
            assert_eq!(attempts.load(Ordering::Relaxed), 1);
            release.send(()).unwrap();
            let (socket, _) = client_async_with_config(
                "ws://localhost/",
                client,
                Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
            )
            .await
            .unwrap();
            drop(socket);
        };
        let (result, ()) = tokio::join!(awaiting, observe);
        assert!(matches!(
            result,
            Ok(TuiAdmission::Break(AdmissionStop::Ignore))
        ));
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        let saved: Presence = private_read(&path).unwrap();
        assert_eq!(saved.state, PresenceState::Ready);
        assert!(saved.associated_thread_ids.is_empty());

        // An expired original admission deadline drops its held socket, not a late
        // connection/publication or a newly started admission.
        let (pending, mut peer) = UnixStream::pair().unwrap();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let result = admission_with_heartbeat(
            async move {
                let _socket = pending;
                std::future::pending::<io::Result<TuiAdmission>>().await
            },
            &mut tick,
            tokio::time::Instant::now() + Duration::from_millis(30),
            || async {
                publication.lock().await.publish()?;
                Ok(true)
            },
        )
        .await;
        assert!(result.is_err());
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );

        // A checked heartbeat that stalls across expiry cannot suspend the original
        // deadline or renew the actual private record after its source lease ends.
        let before: Presence = private_read(&path).unwrap();
        let entered = tokio::sync::Notify::new();
        let release = tokio::sync::Notify::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        let (pending, mut peer) = UnixStream::pair().unwrap();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let awaiting = admission_with_heartbeat(
            async move {
                let _socket = pending;
                std::future::pending::<io::Result<TuiAdmission>>().await
            },
            &mut tick,
            deadline,
            || async {
                entered.notify_one();
                release.notified().await;
                publication.lock().await.publish()?;
                Ok(true)
            },
        );
        let delayed_heartbeat = async {
            tokio::time::timeout(Duration::from_secs(2), entered.notified())
                .await
                .expect("checked heartbeat must start before its held-deadline scenario");
            tokio::time::sleep_until(deadline + Duration::from_millis(30)).await;
            release.notify_one();
        };
        let (result, ()) = tokio::join!(awaiting, delayed_heartbeat);
        assert!(result.is_err());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let after: Presence = private_read(&path).unwrap();
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(after.state, PresenceState::Ready);
        assert!(after.tui.is_none());
        assert!(after.associated_thread_ids.is_empty());
        assert!(after.resumed_thread_ids.is_empty());

        // With both branches ready, expiry wins before either admission or heartbeat
        // is polled. Dropping that ready admission still closes its owned socket.
        let ready_polls = AtomicUsize::new(0);
        let (pending, mut peer) = UnixStream::pair().unwrap();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let result = admission_with_heartbeat(
            async {
                let _socket = pending;
                ready_polls.fetch_add(1, Ordering::Relaxed);
                Ok(TuiAdmission::Break(AdmissionStop::Ignore))
            },
            &mut tick,
            tokio::time::Instant::now() - Duration::from_millis(1),
            || async {
                publication.lock().await.publish()?;
                Ok(true)
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(ready_polls.load(Ordering::Relaxed), 0);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let after: Presence = private_read(&path).unwrap();
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(after.state, PresenceState::Ready);
        assert!(after.tui.is_none());
        assert!(after.associated_thread_ids.is_empty());
        assert!(after.resumed_thread_ids.is_empty());

        // Actual foreign publication replacement rejects renewal, cancels the pending
        // socket and leaves that foreign record untouched. There is no detached ticker.
        let mut foreign: Presence = private_read(&path).unwrap();
        foreign.binding_id = Uuid::random();
        private_write(&path, &foreign, true).unwrap();
        let (pending, mut peer) = UnixStream::pair().unwrap();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let result = admission_with_heartbeat(
            async move {
                let _socket = pending;
                std::future::pending::<io::Result<TuiAdmission>>().await
            },
            &mut tick,
            tokio::time::Instant::now() + Duration::from_secs(10),
            || async {
                publication.lock().await.publish()?;
                Ok(true)
            },
        )
        .await;
        assert!(matches!(&result, Err(error) if error.kind() == io::ErrorKind::InvalidData));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let saved: Presence = private_read(&path).unwrap();
        assert_eq!(saved.binding_id, foreign.binding_id);
        assert_eq!(saved.updated_at, foreign.updated_at);
        assert_eq!(saved.state, PresenceState::Ready);
        assert!(saved.tui.is_none());
        assert!(saved.associated_thread_ids.is_empty());
        assert!(saved.resumed_thread_ids.is_empty());
    }

    #[test]
    fn correlated_read_error_survives_secondary_pane_absence_without_private_payload() {
        use std::os::unix::fs::DirBuilderExt;
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let mut observer = Observer::default();
        observer.from_tui(br#"{"id":"PRIVATE_RPC_ID","method":"thread/read","params":{"threadId":"PRIVATE_THREAD_ID","includeTurns":false}}"#).unwrap();
        assert!(!observer.from_owner(br#"{"id":"other","error":{"code":-32600,"message":"PRIVATE_MESSAGE","data":"PRIVATE_DATA"}}"#).unwrap());
        assert!(observer.first_error.is_none());
        observer.from_owner(br#"{"id":"PRIVATE_RPC_ID","error":{"code":-32600,"message":"PRIVATE_MESSAGE","data":"PRIVATE_DATA"}}"#).unwrap();
        assert_eq!(observer.first_error, Some(("thread/read", -32600)));
        assert!(observer.associated().is_empty());
        assert!(observer.resumed().is_empty());
        assert!(observer.rejected_resume().is_none());
        let root = Root(std::env::temp_dir().join(format!("gd-{}", &Uuid::random().0[..8])));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let mut publication = publication_fixture(&root.0);
        let path = publication.path.clone();
        publication.retain_native_error(&observer);
        publication
            .clear(
                PresenceState::Disconnected,
                Some("TUI transport read failed"),
            )
            .unwrap();
        let secondary = io::Error::new(io::ErrorKind::InvalidData, herdr::ProcessInfoNotFound);
        assert_eq!(
            terminal_failure(&secondary),
            "Herdr pane.process_info rejected: pane_not_found"
        );
        publication
            .clear(PresenceState::Failed, Some(terminal_failure(&secondary)))
            .unwrap();
        let saved: Presence = private_read(&presence_path(&path)).unwrap();
        assert_eq!(
            saved.error.unwrap().0,
            "Correlated native thread/read error (-32600)"
        );
        assert!(saved.associated_thread_ids.is_empty());
        assert!(saved.resumed_thread_ids.is_empty());
        assert!(saved.resume_rejected_thread.is_none());
        let serialized = fs::read(presence_path(&path)).unwrap();
        for sentinel in [
            "PRIVATE_RPC_ID",
            "PRIVATE_THREAD_ID",
            "PRIVATE_MESSAGE",
            "PRIVATE_DATA",
        ] {
            assert!(
                !serialized
                    .windows(sentinel.len())
                    .any(|bytes| bytes == sentinel.as_bytes())
            );
        }
    }

    #[test]
    fn terminal_proof_unavailable_retains_bridge_until_same_scope_heartbeat_recovers() {
        let root = PrivateReadRoot::new();
        let mut publication = publication_fixture(&root.0);
        let mut missing_availability = serde_json::to_value(&publication.presence).unwrap();
        missing_availability
            .as_object_mut()
            .unwrap()
            .remove("terminalProof");
        assert!(serde_json::from_value::<Presence>(missing_availability).is_err());
        let connected_scope = publication.scope();
        let bridge = (
            publication.presence.binding_id.clone(),
            publication.presence.relay.clone(),
            publication.presence.tui.clone(),
            publication.presence.connection_epoch.clone(),
            publication.presence.associated_thread_ids.clone(),
            publication.presence.resumed_thread_ids.clone(),
        );
        let broken_pipe =
            TerminalFailure::new("process_info", io::Error::from_raw_os_error(libc::EPIPE));
        assert!(terminal_proof_unavailable(&broken_pipe));
        assert!(terminal_proof_unavailable(&TerminalFailure::new(
            "snapshot",
            io::Error::from_raw_os_error(libc::EPIPE),
        )));
        assert!(!terminal_proof_unavailable(&TerminalFailure::new(
            "process_info_connect",
            io::Error::from_raw_os_error(libc::EPIPE),
        )));
        assert!(!terminal_proof_unavailable(&TerminalFailure::new(
            "snapshot",
            io::Error::from(io::ErrorKind::TimedOut),
        )));
        assert!(
            publication
                .set_terminal_proof_if_current(
                    &connected_scope,
                    TerminalProofAvailability::Unavailable,
                )
                .unwrap()
        );
        publication.retain_modes(&Observer::default());
        assert!(publication.publish_if_current(&connected_scope).unwrap());
        assert_eq!(
            publication.presence.terminal_proof,
            TerminalProofAvailability::Unavailable
        );
        assert_eq!(
            (
                publication.presence.binding_id.clone(),
                publication.presence.relay.clone(),
                publication.presence.tui.clone(),
                publication.presence.connection_epoch.clone(),
                publication.presence.associated_thread_ids.clone(),
                publication.presence.resumed_thread_ids.clone(),
            ),
            bridge
        );
        let unavailable_scope = publication.scope();
        assert!(
            publication
                .set_terminal_proof_if_current(
                    &unavailable_scope,
                    TerminalProofAvailability::Available,
                )
                .unwrap()
        );
        publication.presence.connection_epoch = Uuid::random();
        assert!(
            !publication
                .set_terminal_proof_if_current(
                    &unavailable_scope,
                    TerminalProofAvailability::Unavailable,
                )
                .unwrap()
        );
        publication
            .clear(PresenceState::TerminalClosed, None)
            .unwrap();
        assert!(publication.presence.associated_thread_ids.is_empty());
        assert!(publication.presence.resumed_thread_ids.is_empty());
    }

    #[test]
    fn terminal_capture_annotation_is_closed_and_preserves_the_original_error() {
        let timeout = TerminalFailure::new(
            "snapshot",
            io::Error::new(io::ErrorKind::InvalidData, "Herdr response timeout"),
        );
        let capture = terminal_capture(&timeout);
        assert!(capture.response_timeout);
        let annotation = terminal_capture_message(capture);
        let parsed = terminal_capture_annotation(&annotation.0).unwrap();
        assert_eq!(parsed.phase, "snapshot");
        assert_eq!(parsed.io, "invalid_data");
        assert!(parsed.response_timeout);
        for malformed in [
            format!("{} private", annotation.0),
            annotation.0.replace(' ', "  "),
            annotation.0.replace(' ', "\t"),
            format!("{}\n", annotation.0),
            annotation.0.replace("errno=0", "errno=+0"),
            annotation.0.replace("errno=0", "errno=00"),
            "Codex terminal capture phase=snapshot io=invalid_data errno=0 response_timeout=1 private".into(),
        ] {
            assert!(terminal_capture_annotation(&malformed).is_none());
        }
        assert!(terminal_capture_annotation(&"x".repeat(193)).is_none());
        for malformed in [
            "Codex terminal capture phase=tui_write_ready_pending io=other errno=0 response_timeout=0",
            "Codex terminal capture phase=tui_write_ready_pending io=timed_out errno=1 response_timeout=0",
            "Codex terminal capture phase=tui_write_ready_pending io=timed_out errno=0 response_timeout=1",
            "Codex terminal capture phase=tui_write_start_send_pending io=timed_out errno=0 response_timeout=0",
        ] {
            assert!(terminal_capture_annotation(malformed).is_none());
        }
        assert_eq!(timeout.into_error().to_string(), "Herdr response timeout");
    }

    #[test]
    fn terminal_capture_does_not_replace_a_prior_presence_error() {
        use std::os::unix::fs::DirBuilderExt;
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root =
            Root(std::env::temp_dir().join(format!("goatr-terminal-capture-{}", Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let mut publication = publication_fixture(&root.0);
        publication.terminal_capture = true;
        publication.presence.error = Some(SafeMessage(
            "Correlated native thread/read error (-1)".into(),
        ));
        let failure = TerminalFailure::new(
            "process_info",
            io::Error::new(io::ErrorKind::TimedOut, "private"),
        );
        publication
            .retain_terminal_capture(terminal_failure(&failure.error), terminal_capture(&failure));
        publication
            .clear(
                PresenceState::Disconnected,
                Some(terminal_failure(&failure.error)),
            )
            .unwrap();
        assert_eq!(
            publication.presence.error.unwrap().0,
            "Correlated native thread/read error (-1)"
        );
    }
    #[tokio::test]
    async fn native_rendezvous_held_socket_keeps_real_peer_and_rejects_alias_source_replacement() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::DirBuilderExt;
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(PathBuf::from("/tmp").join(format!("gr-{}", &Uuid::random().0[..8])));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        for path in ["r", "tmp", &format!("tmp/codex-daemon-{}", pairing::uid())] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(root.0.join(path))
                .unwrap();
        }
        let agent = Path::new("/r/c.sock");
        let hash = pairing::hex(&Sha256::digest(agent.as_os_str().as_encoded_bytes()));
        let physical = PathBuf::from(format!("/tmp/codex-daemon-{}", pairing::uid())).join(hash);
        let actual = root.0.join(physical.strip_prefix("/").unwrap());
        config::socket_path(&actual).unwrap();
        let listener = UnixListener::bind(&actual).unwrap();
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o600)).unwrap();
        let host = root.0.join("r/c.sock");
        std::os::unix::fs::symlink(&physical, &host).unwrap();
        assert!(
            socket_object(&host).is_err(),
            "direct relay sockets stay strict"
        );
        let root_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(&root.0)
            .unwrap();
        // Actual native alias/file objects and kernel peer only, not publisher or Ready proof.
        let captured = OwnerSocket::capture(root_file, &host, agent).unwrap();
        let metadata = fs::symlink_metadata(&actual).unwrap();
        assert_eq!(captured.object().unwrap(), (metadata.dev(), metadata.ino()));
        let saved: SocketSource =
            serde_json::from_slice(&serde_json::to_vec(&captured.source().unwrap()).unwrap())
                .unwrap();
        captured.check_source(&saved).unwrap();
        let mut incomplete = serde_json::to_value(&saved).unwrap();
        incomplete.as_object_mut().unwrap().remove("alias");
        assert!(serde_json::from_value::<SocketSource>(incomplete).is_err());
        let client = captured.connect().await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let peer = client.peer_cred().unwrap();
        assert_eq!(peer.uid(), pairing::uid());
        assert_eq!(peer.pid(), Some(std::process::id() as i32));
        assert_eq!(server.peer_cred().unwrap().pid(), peer.pid());
        fs::remove_file(&host).unwrap();
        std::os::unix::fs::symlink(&physical, &host).unwrap();
        assert!(
            captured.recheck().is_err(),
            "same target is not the original alias source"
        );
        assert_eq!(client.peer_cred().unwrap().pid(), peer.pid());
        fs::remove_file(&host).unwrap();
        std::os::unix::fs::symlink("/tmp/codex-daemon-0/foreign", &host).unwrap();
        assert!(OwnerSocket::capture(captured.root.try_clone().unwrap(), &host, agent,).is_err());
        fs::remove_file(&host).unwrap();
        std::os::unix::fs::symlink(&physical, &host).unwrap();
        let stable =
            OwnerSocket::capture(captured.root.try_clone().unwrap(), &host, agent).unwrap();
        assert!(
            stable.check_source(&saved).is_err(),
            "restart cannot accept a replacement alias"
        );
        let physical_parent = actual.parent().unwrap();
        fs::set_permissions(physical_parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(stable.recheck().is_err());
        fs::set_permissions(physical_parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(stable.recheck().is_err());
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_file(&actual).unwrap();
        let replacement = UnixListener::bind(&actual).unwrap();
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            stable.recheck().is_err(),
            "physical replacement keeps no source authority"
        );
        assert_eq!(client.peer_cred().unwrap().pid(), peer.pid());
        let moved = OwnerSocket::capture(captured.root.try_clone().unwrap(), &host, agent).unwrap();
        fs::rename(root.0.join("r"), root.0.join("r-old")).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.0.join("r"))
            .unwrap();
        std::os::unix::fs::symlink(&physical, &host).unwrap();
        assert!(
            moved.recheck().is_err(),
            "replaced advertised parent is a changed source"
        );
        drop((stable, moved, replacement));
        drop((client, server, listener, captured));
    }
    #[tokio::test]
    async fn queued_and_inflight_frames_share_both_hard_budgets() {
        let (tx, mut rx) = mpsc::channel(QUEUE_FRAMES);
        let bytes = Arc::new(Semaphore::new(QUEUE_BYTES));
        let frames = Arc::new(Semaphore::new(QUEUE_FRAMES));
        for _ in 0..QUEUE_FRAMES {
            enqueue(&tx, &bytes, &frames, Message::Binary(vec![1].into())).unwrap();
        }
        let inflight = rx.recv().await.unwrap();
        assert!(enqueue(&tx, &bytes, &frames, Message::Binary(vec![2].into())).is_err());
        drop(inflight);
        enqueue(&tx, &bytes, &frames, Message::Binary(vec![2].into())).unwrap();
        drop(rx);
        assert_eq!(bytes.available_permits(), QUEUE_BYTES);
        assert_eq!(frames.available_permits(), QUEUE_FRAMES);
        let (tx, mut rx) = mpsc::channel(QUEUE_FRAMES);
        // The same weighted-permit/inflight rule needs no maximum-size allocation.
        let bytes = Arc::new(Semaphore::new(2));
        enqueue(&tx, &bytes, &frames, Message::Binary(vec![0].into())).unwrap();
        enqueue(&tx, &bytes, &frames, Message::Binary(vec![0].into())).unwrap();
        assert!(enqueue(&tx, &bytes, &frames, Message::Binary(vec![0].into())).is_err());
        let inflight = rx.recv().await.unwrap();
        assert!(enqueue(&tx, &bytes, &frames, Message::Binary(vec![0].into())).is_err());
        drop(inflight);
        enqueue(&tx, &bytes, &frames, Message::Binary(vec![0].into())).unwrap();
        drop(rx);
        assert_eq!(bytes.available_permits(), 2);
    }
    #[tokio::test]
    async fn actual_upgrade_rejects_origin_and_nonupgrade_without_consuming_a_native_channel() {
        let (server, client) = UnixStream::pair().unwrap();
        let accepted = tokio::spawn(async move {
            accept_hdr_async_with_config(
                server,
                OriginGuard,
                Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
            )
            .await
        });
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = "ws://localhost/".into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", "https://foreign.invalid".parse().unwrap());
        assert!(
            client_async_with_config(
                request,
                client,
                Some(websocket_config(FRAME_BYTES, QUEUE_BYTES))
            )
            .await
            .is_err()
        );
        assert!(accepted.await.unwrap().is_err());
        let (server, mut client) = UnixStream::pair().unwrap();
        let accepted = tokio::spawn(async move {
            accept_hdr_async_with_config(
                server,
                OriginGuard,
                Some(websocket_config(FRAME_BYTES, QUEUE_BYTES)),
            )
            .await
        });
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        assert!(accepted.await.unwrap().is_err());
    }
    struct HeldTuiSink {
        release: tokio::sync::oneshot::Receiver<()>,
        open: bool,
        messages: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
        starts: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Sink<Message> for HeldTuiSink {
        type Error = io::Error;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn start_send(self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.starts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.messages
                .lock()
                .unwrap()
                .push(message_bytes(&item)?.to_vec());
            Ok(())
        }
        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            if self.open {
                return std::task::Poll::Ready(Ok(()));
            }
            match std::future::Future::poll(std::pin::Pin::new(&mut self.release), cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(Ok(())) => {
                    self.open = true;
                    std::task::Poll::Ready(Ok(()))
                }
                std::task::Poll::Ready(Err(_)) => {
                    std::task::Poll::Ready(Err(invalid("fixture TUI closed")))
                }
            }
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn passive_owner_to_tui_writer_keeps_inflight_permits_during_a_long_stall() {
        let (tx, mut rx) = mpsc::channel(QUEUE_FRAMES);
        let bytes = Arc::new(Semaphore::new(64));
        let frames = Arc::new(Semaphore::new(QUEUE_FRAMES));
        enqueue(&tx, &bytes, &frames, Message::Text("first".into())).unwrap();
        enqueue(&tx, &bytes, &frames, Message::Text("second".into())).unwrap();
        drop(tx);
        let (release, receiver) = tokio::sync::oneshot::channel();
        let messages = Arc::new(std::sync::Mutex::new(Vec::new()));
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut sink = HeldTuiSink {
            release: receiver,
            open: false,
            messages: messages.clone(),
            starts: starts.clone(),
        };
        {
            let writer = forward_owner_to_tui(&mut sink, &mut rx);
            tokio::pin!(writer);
            tokio::select! {
                _ = tokio::task::yield_now() => {}
                result = &mut writer => panic!("passive writer settled early: {result:?}"),
            }
            tokio::time::advance(Duration::from_secs(6)).await;
            assert!(futures_util::poll!(writer.as_mut()).is_pending());
            assert_eq!(
                bytes.available_permits(),
                64 - "first".len() - "second".len()
            );
            assert_eq!(frames.available_permits(), QUEUE_FRAMES - 2);
            assert_eq!(*messages.lock().unwrap(), [b"first".to_vec()]);
            assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 1);
            release.send(()).unwrap();
            assert!(writer.await.is_ok());
        }
        assert_eq!(
            *messages.lock().unwrap(),
            [b"first".to_vec(), b"second".to_vec()]
        );
        assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(bytes.available_permits(), 64);
        assert_eq!(frames.available_permits(), QUEUE_FRAMES);
    }

    #[tokio::test]
    async fn passive_owner_to_tui_writer_keeps_overflow_and_peer_closure_terminal() {
        use tokio_tungstenite::tungstenite::protocol::Role;

        let (tx, mut rx) = mpsc::channel(1);
        let bytes = Arc::new(Semaphore::new(8));
        let frames = Arc::new(Semaphore::new(1));
        enqueue(&tx, &bytes, &frames, Message::Text("frame".into())).unwrap();
        assert!(enqueue(&tx, &bytes, &frames, Message::Text("next".into())).is_err());
        drop(tx);

        let (relay_socket, tui_socket) = UnixStream::pair().unwrap();
        let mut relay = WebSocketStream::from_raw_socket(relay_socket, Role::Server, None).await;
        let mut tui = WebSocketStream::from_raw_socket(tui_socket, Role::Client, None).await;
        tui.get_mut().shutdown().await.unwrap();
        drop(tui);

        assert_eq!(
            forward_owner_to_tui(&mut relay, &mut rx).await.unwrap_err(),
            "TUI write failed"
        );
        assert_eq!(bytes.available_permits(), 8);
        assert_eq!(frames.available_permits(), 1);
    }
}
