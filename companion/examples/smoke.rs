// SPDX-License-Identifier: GPL-3.0-or-later
//! Non-installed transport/provider/grid gates. Grid requires qualified fresh resources and retains its supervisor; never phone/cleanup authority.
use clap::{Arg, Command, value_parser};
use futures_util::StreamExt;
use goatr::{
    pairing::{PairInvite, hex, private_dir, private_file, random32, uid},
    protocol::*,
    transport,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "smoke/live.rs"]
mod live;
#[path = "smoke/review.rs"]
mod review;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HelperLocation {
    file: &'static str,
    line: u32,
}
impl HelperLocation {
    fn from_parts(file: &str, line: u32) -> Option<Self> {
        let file = match file {
            "examples/smoke.rs" | "companion/examples/smoke.rs" => "smoke.rs",
            "examples/smoke/live.rs" | "companion/examples/smoke/live.rs" => "smoke/live.rs",
            "examples/smoke/dev_continuity.rs" | "companion/examples/smoke/dev_continuity.rs" => {
                "smoke/dev_continuity.rs"
            }
            "examples/smoke/paging_proof.rs" | "companion/examples/smoke/paging_proof.rs" => {
                "smoke/paging_proof.rs"
            }
            "examples/smoke/ui.rs" | "companion/examples/smoke/ui.rs" => "smoke/ui.rs",
            "examples/smoke/camera.rs" | "companion/examples/smoke/camera.rs" => "smoke/camera.rs",
            "examples/smoke/review.rs" | "companion/examples/smoke/review.rs" => "smoke/review.rs",
            "examples/smoke/setup_acceptance.rs"
            | "companion/examples/smoke/setup_acceptance.rs" => "smoke/setup_acceptance.rs",
            _ => return None,
        };
        (1..=999_999).contains(&line).then_some(Self { file, line })
    }
    #[track_caller]
    fn caller() -> Option<Self> {
        let caller = std::panic::Location::caller();
        Self::from_parts(caller.file(), caller.line())
    }
}
#[derive(Debug)]
struct LocatedFailure {
    message: String,
    location: Option<HelperLocation>,
}
impl std::fmt::Display for LocatedFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for LocatedFailure {}
#[derive(Debug)]
struct PromptStopFailure {
    original: io::Error,
    location: Option<HelperLocation>,
    provider: Provider,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalPairStderr {
    None,
    PairTerminalRequired,
    InvalidPairingReply,
    LocalAdminInvalidRequest,
    LocalAdminUnsafeSocket,
    LocalAdminPeerUnauthorized,
    LocalAdminReadTimeout,
    LocalAdminWriteTimeout,
    LocalAdminConnectTimeout,
    LocalAdminBusy,
    LocalAdminTimeout,
    LocalAdminUnauthorized,
    LocalAdminInternalError,
    LocalAdminInvalidRequestReply,
    LocalAdminUnsupportedVersion,
    LocalAdminUnsupportedMethod,
    Unmatched,
    Truncated,
    IoError,
    Incomplete,
}
impl TerminalPairStderr {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PairTerminalRequired => "pair_terminal_required",
            Self::InvalidPairingReply => "invalid_pairing_reply",
            Self::LocalAdminInvalidRequest => "local_admin_invalid_request",
            Self::LocalAdminUnsafeSocket => "local_admin_unsafe_socket",
            Self::LocalAdminPeerUnauthorized => "local_admin_peer_unauthorized",
            Self::LocalAdminReadTimeout => "local_admin_read_timeout",
            Self::LocalAdminWriteTimeout => "local_admin_write_timeout",
            Self::LocalAdminConnectTimeout => "local_admin_connect_timeout",
            Self::LocalAdminBusy => "local_admin_busy",
            Self::LocalAdminTimeout => "local_admin_timeout",
            Self::LocalAdminUnauthorized => "local_admin_unauthorized",
            Self::LocalAdminInternalError => "local_admin_internal_error",
            Self::LocalAdminInvalidRequestReply => "local_admin_invalid_request_reply",
            Self::LocalAdminUnsupportedVersion => "local_admin_unsupported_version",
            Self::LocalAdminUnsupportedMethod => "local_admin_unsupported_method",
            Self::Unmatched => "unmatched",
            Self::Truncated => "truncated",
            Self::IoError => "io_error",
            Self::Incomplete => "incomplete",
        }
    }
}
const TERMINAL_PAIR_STDERR_LINES: [(&[u8], TerminalPairStderr); 15] = [
    (b"goatr: pair requires an explicitly requested local terminal; QR/token are never emitted into captured logs", TerminalPairStderr::PairTerminalRequired),
    (b"goatr: invalid pairing reply", TerminalPairStderr::InvalidPairingReply),
    (b"goatr: invalid local admin request", TerminalPairStderr::LocalAdminInvalidRequest),
    (b"goatr: unsafe local admin socket", TerminalPairStderr::LocalAdminUnsafeSocket),
    (b"goatr: local admin peer is unauthorized", TerminalPairStderr::LocalAdminPeerUnauthorized),
    (b"goatr: local admin read timed out", TerminalPairStderr::LocalAdminReadTimeout),
    (b"goatr: local admin write timed out", TerminalPairStderr::LocalAdminWriteTimeout),
    (b"goatr: local admin connect timed out", TerminalPairStderr::LocalAdminConnectTimeout),
    (b"goatr: local admin busy", TerminalPairStderr::LocalAdminBusy),
    (b"goatr: local admin timeout", TerminalPairStderr::LocalAdminTimeout),
    (b"goatr: local admin unauthorized", TerminalPairStderr::LocalAdminUnauthorized),
    (b"goatr: local admin internal_error", TerminalPairStderr::LocalAdminInternalError),
    (b"goatr: local admin invalid_request", TerminalPairStderr::LocalAdminInvalidRequestReply),
    (b"goatr: local admin unsupported_version", TerminalPairStderr::LocalAdminUnsupportedVersion),
    (b"goatr: local admin unsupported_method", TerminalPairStderr::LocalAdminUnsupportedMethod),
];
#[derive(Debug)]
struct TerminalPairStderrClassifier {
    candidates: [bool; TERMINAL_PAIR_STDERR_LINES.len()],
    at: usize,
    saw_bytes: bool,
    ended: bool,
    invalid: bool,
    truncated: bool,
    io_error: bool,
    eof: bool,
}
impl TerminalPairStderrClassifier {
    const MAX_BYTES: usize = 4096;

    fn push(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.saw_bytes = true;
            if self.ended {
                self.invalid = true;
                continue;
            }
            if self.at >= Self::MAX_BYTES {
                self.truncated = true;
                self.invalid = true;
                continue;
            }
            if *byte == b'\n' {
                self.ended = true;
                if !TERMINAL_PAIR_STDERR_LINES
                    .iter()
                    .enumerate()
                    .any(|(index, (line, _))| self.candidates[index] && line.len() == self.at)
                {
                    self.invalid = true;
                }
            } else {
                for (index, (line, _)) in TERMINAL_PAIR_STDERR_LINES.iter().enumerate() {
                    self.candidates[index] &= line.get(self.at) == Some(byte);
                }
                if !self.candidates.iter().any(|candidate| *candidate) {
                    self.invalid = true;
                }
                self.at += 1;
            }
        }
    }
    fn eof(&mut self) {
        self.eof = true;
    }
    fn io_error(&mut self) {
        self.io_error = true;
    }
    fn result(&self) -> TerminalPairStderr {
        if self.io_error {
            TerminalPairStderr::IoError
        } else if self.truncated {
            TerminalPairStderr::Truncated
        } else if !self.eof {
            TerminalPairStderr::Incomplete
        } else if !self.saw_bytes {
            TerminalPairStderr::None
        } else if self.invalid {
            TerminalPairStderr::Unmatched
        } else if let Some((_, (_, category))) = TERMINAL_PAIR_STDERR_LINES
            .iter()
            .enumerate()
            .find(|(index, (line, _))| self.candidates[*index] && line.len() == self.at)
        {
            *category
        } else {
            TerminalPairStderr::Unmatched
        }
    }
}
impl Default for TerminalPairStderrClassifier {
    fn default() -> Self {
        Self {
            candidates: [true; TERMINAL_PAIR_STDERR_LINES.len()],
            at: 0,
            saw_bytes: false,
            ended: false,
            invalid: false,
            truncated: false,
            io_error: false,
            eof: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalPairExit {
    Code(i32),
    Signal(i32),
    Other,
}
impl TerminalPairExit {
    fn from_status(status: ExitStatus) -> Self {
        if let Some(code) = status.code() {
            Self::Code(code)
        } else if let Some(signal) = status.signal() {
            Self::Signal(signal)
        } else {
            Self::Other
        }
    }
}
#[derive(Debug)]
struct TerminalPairFailure {
    exit: TerminalPairExit,
    stderr: TerminalPairStderr,
    location: Option<HelperLocation>,
}
impl std::fmt::Display for TerminalPairFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("packaged pair command failed")
    }
}
impl std::error::Error for TerminalPairFailure {}
impl std::fmt::Display for PromptStopFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.original, f)
    }
}
impl std::error::Error for PromptStopFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.original)
    }
}
fn helper_error_location(error: &io::Error) -> Option<HelperLocation> {
    let inner = error.get_ref()?;
    inner
        .downcast_ref::<LocatedFailure>()
        .and_then(|failure| failure.location)
        .or_else(|| {
            inner
                .downcast_ref::<PromptStopFailure>()
                .and_then(|failure| failure.location)
        })
        .or_else(|| {
            inner
                .downcast_ref::<TerminalPairFailure>()
                .and_then(|failure| failure.location)
        })
}
#[track_caller]
fn located_message(
    kind: io::ErrorKind,
    message: &str,
    original: Option<HelperLocation>,
) -> io::Error {
    // Direct calls preserve track_caller; a function-pointer callback would lose it.
    let location = match original {
        Some(location) => Some(location),
        None => HelperLocation::caller(),
    };
    io::Error::new(
        kind,
        LocatedFailure {
            message: message.to_owned(),
            location,
        },
    )
}
#[track_caller]
fn failure(message: &str) -> io::Error {
    located_message(io::ErrorKind::Other, message, None)
}
#[track_caller]
fn prerequisite(message: &str) -> io::Error {
    located_message(io::ErrorKind::NotConnected, message, None)
}
#[track_caller]
fn prompt_stop_failure(provider: Provider, error: io::Error) -> io::Error {
    let location = match helper_error_location(&error) {
        Some(location) => Some(location),
        None => HelperLocation::caller(),
    };
    io::Error::new(
        error.kind(),
        PromptStopFailure {
            original: error,
            location,
            provider,
        },
    )
}
#[track_caller]
fn terminal_pair_failure(status: ExitStatus, stderr: TerminalPairStderr) -> io::Error {
    io::Error::other(TerminalPairFailure {
        exit: TerminalPairExit::from_status(status),
        stderr,
        location: HelperLocation::caller(),
    })
}
fn helper_error_metadata(error: &io::Error) -> Value {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::ConnectionRefused => "connection_refused",
        io::ErrorKind::ConnectionReset => "connection_reset",
        io::ErrorKind::ConnectionAborted => "connection_aborted",
        io::ErrorKind::NotConnected => "not_connected",
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::AlreadyExists => "already_exists",
        io::ErrorKind::WouldBlock => "would_block",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::InvalidData => "invalid_data",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::WriteZero => "write_zero",
        io::ErrorKind::Interrupted => "interrupted",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        io::ErrorKind::OutOfMemory => "out_of_memory",
        _ => "other",
    };
    let mut value = json!({"io_kind":kind});
    if let Some(location) = helper_error_location(error) {
        value["source_file"] = json!(location.file);
        value["source_line"] = json!(location.line);
    }
    if let Some(context) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<PromptStopFailure>())
    {
        value["stage"] = json!("prompt_stop");
        value["provider"] = json!(context.provider);
    }
    if let Some(context) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<TerminalPairFailure>())
    {
        let (code, signal) = match context.exit {
            TerminalPairExit::Code(code) => (Some(code), None),
            TerminalPairExit::Signal(signal) => (None, Some(signal)),
            TerminalPairExit::Other => (None, None),
        };
        value["pair_exit"] = json!({
            "code": code,
            "signal": signal,
            "stderr": context.stderr.as_str(),
        });
    }
    value
}
fn boot() -> io::Result<String> {
    Ok(fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .into())
}
fn start(pid: u32) -> io::Result<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    stat[stat
        .rfind(')')
        .ok_or_else(|| failure("invalid process identity"))?
        + 2..]
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
        .ok_or_else(|| failure("invalid process identity"))
}
fn identity(path: &Path) -> io::Result<Value> {
    let m = fs::symlink_metadata(path)?;
    Ok(json!({"path":path,"uid":m.uid(),"device":m.dev(),"inode":m.ino(),"mode":m.mode()&0o7777}))
}
fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| failure("invalid evidence parent"))?;
    private_dir(parent, false)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
    file.write_all(b"\n")?;
    file.sync_all()
}
fn binary_sha256(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path)?;
    let mut bytes = [0u8; 65536];
    let mut hash = Sha256::new();
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    let digest: [u8; 32] = hash.finalize().into();
    Ok(hex(&digest))
}
fn transport_identity(path: &Path, metadata: &fs::Metadata) -> Value {
    use std::os::unix::fs::FileTypeExt;
    let kind = if metadata.is_dir() {
        "directory"
    } else if metadata.is_file() {
        "file"
    } else if metadata.file_type().is_socket() {
        "socket"
    } else if metadata.file_type().is_symlink() {
        "symlink"
    } else {
        "other"
    };
    json!({"path":path,"uid":metadata.uid(),"device":metadata.dev(),"inode":metadata.ino(),
        "mode":metadata.mode()&0o7777,"kind":kind})
}
fn retained_resource(path: &Path, original: Option<&Value>) -> Value {
    let current = fs::symlink_metadata(path)
        .map(|metadata| transport_identity(path, &metadata))
        .map_err(|error| error.kind());
    let matches = original.is_some_and(|original| current.as_ref().is_ok_and(|current| current == original));
    json!({"original":original,"current":current.as_ref().ok(),"matchesOriginal":matches,
        "captureErrorKind":current.as_ref().err().map(|kind|format!("{kind:?}"))})
}
struct OwnedRun {
    child: Child,
    original_process: Result<goatr::herdr::ProcessEvidence, io::ErrorKind>,
    root: PathBuf,
    root_original: Value,
    admin_parent: PathBuf,
    admin_original: Value,
}
impl OwnedRun {
    fn spawn(command: &mut std::process::Command, root: PathBuf, root_original: Value, admin_parent: PathBuf, admin_original: Value) -> io::Result<Self> {
        let child = command.process_group(0).spawn()?;
        let original_process = goatr::herdr::process_evidence(child.id()).map_err(|error| error.kind());
        Ok(Self { child, original_process, root, root_original, admin_parent, admin_original })
    }
    fn matches_process(&self, current: &goatr::herdr::ProcessEvidence) -> bool {
        self.original_process.as_ref().is_ok_and(|original| original == current)
    }
    fn process_comparison(&self, current: Result<goatr::herdr::ProcessEvidence, io::ErrorKind>) -> Value {
        let matches = current.as_ref().is_ok_and(|current| self.matches_process(current));
        json!({"original":self.original_process.as_ref().ok(),"current":current.as_ref().ok(),
            "matchesOriginal":matches,
            "originalCaptureErrorKind":self.original_process.as_ref().err().map(|kind|format!("{kind:?}")),
            "currentCaptureErrorKind":current.as_ref().err().map(|kind|format!("{kind:?}"))})
    }
    fn retained_resources(&mut self, socket: &Path, original_socket: Option<&Value>) -> Value {
        let current = goatr::herdr::process_evidence(self.child.id()).map_err(|error| error.kind());
        let child_state = match self.child.try_wait() {
            Ok(None) => json!({"state":"running"}),
            Ok(Some(status)) => json!({"state":"exited","code":status.code(),"signal":status.signal()}),
            Err(error) => json!({"state":"unavailable","errorKind":format!("{:?}",error.kind())}),
        };
        json!({"childPid":self.child.id(),"childState":child_state,"process":self.process_comparison(current),
            "root":retained_resource(&self.root,Some(&self.root_original)),
            "adminParent":retained_resource(&self.admin_parent,Some(&self.admin_original)),
            "adminSocket":retained_resource(socket,original_socket),
            "cleanup":"pending_owner_review","cleanupPerformed":false,"cleanupAuthority":false})
    }
}
fn scoped(command: &mut std::process::Command, root: &Path, socket: &Path) {
    command
        .arg("--admin-socket")
        .arg(socket)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"));
}
fn nonblocking_stderr(stderr: &ChildStderr) -> io::Result<()> {
    // SAFETY: this only adds O_NONBLOCK to this invocation's owned pipe descriptor.
    let flags = unsafe { libc::fcntl(stderr.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor and current flags came from the owned ChildStderr above.
    if unsafe { libc::fcntl(stderr.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn drain_terminal_pair_stderr(
    stderr: &mut ChildStderr,
    classifier: &mut TerminalPairStderrClassifier,
) {
    if classifier.io_error || classifier.eof {
        return;
    }
    let mut chunk = [0u8; 4096];
    // Bound each polling pass so a noisy child cannot starve the original PTY/deadline loop.
    for _ in 0..4 {
        match stderr.read(&mut chunk) {
            Ok(0) => {
                classifier.eof();
                return;
            }
            Ok(n) => classifier.push(&chunk[..n]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                classifier.io_error();
                return;
            }
        }
    }
}
/// libc PTY only: pair insists on terminal stdout. The capability never reaches argv/env/files.
fn terminal_pair(binary: &Path, root: Option<&Path>, socket: Option<&Path>) -> io::Result<String> {
    // SAFETY: these POSIX calls allocate/configure an owned PTY descriptor and bounded name buffer.
    let fd = unsafe {
        libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC | libc::O_NONBLOCK)
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut master = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::grantpt(fd) } != 0 || unsafe { libc::unlockpt(fd) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut name = [0i8; 256];
    if unsafe { libc::ptsname_r(fd, name.as_mut_ptr(), name.len()) } != 0 {
        return Err(failure("PTY name unavailable"));
    }
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(Path::new(
            std::str::from_utf8(name.to_bytes()).map_err(io::Error::other)?,
        ))?;
    let mut command = std::process::Command::new(binary);
    command.arg("pair");
    match (root, socket) {
        (Some(root), Some(socket)) => scoped(&mut command, root, socket),
        (None, None) => {}
        _ => {
            return Err(failure(
                "pair requires either the exact owned override or unchanged production default",
            ));
        }
    }
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave))
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut stderr = None;
    let mut stderr_nonblocking = false;
    let mut stderr_classifier = TerminalPairStderrClassifier::default();
    let mut bytes = Vec::with_capacity(16384);
    let mut chunk = [0u8; 4096];
    // Development binaries still complete the real terminal-only QR command.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut exited = false;
    let result = (|| {
        stderr = child.stderr.take();
        let stderr = stderr
            .as_mut()
            .ok_or_else(|| failure("pair stderr pipe unavailable"))?;
        nonblocking_stderr(stderr)?;
        stderr_nonblocking = true;
        let mut pty_finished = false;
        loop {
            drain_terminal_pair_stderr(stderr, &mut stderr_classifier);
            if !pty_finished {
                match master.read(&mut chunk) {
                    Ok(0) => pty_finished = true,
                    Ok(n) => {
                        if bytes.len() + n > 128 * 1024 {
                            return Err(failure("pair terminal output exceeded bound"));
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // Command retains parent slave descriptors: preserve drained-output completion.
                        pty_finished = exited;
                    }
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => pty_finished = true,
                    Err(e) => return Err(e),
                }
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    drain_terminal_pair_stderr(stderr, &mut stderr_classifier);
                    return Err(terminal_pair_failure(status, stderr_classifier.result()));
                }
                exited = true;
            }
            if pty_finished && exited {
                break;
            }
            if Instant::now() > deadline {
                return Err(failure("pair terminal deadline exceeded"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let status = child.wait()?;
        drain_terminal_pair_stderr(stderr, &mut stderr_classifier);
        if !status.success() {
            return Err(terminal_pair_failure(status, stderr_classifier.result()));
        }
        let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
        let uri = text
            .lines()
            .find(|line| line.starts_with("goatr://pair/v1?data="))
            .ok_or_else(|| failure("complete terminal URI missing"))?
            .trim_end_matches('\r')
            .to_owned();
        PairInvite::parse(&uri).map_err(|_| failure("invalid terminal URI"))?;
        Ok(uri)
    })();
    bytes.fill(0);
    if child.try_wait()?.is_none() {
        if stderr_nonblocking && let Some(stderr) = stderr.as_mut() {
            drain_terminal_pair_stderr(stderr, &mut stderr_classifier);
        }
        child.kill()?;
        child.wait()?;
        if stderr_nonblocking && let Some(stderr) = stderr.as_mut() {
            drain_terminal_pair_stderr(stderr, &mut stderr_classifier);
        }
    }
    result
}
async fn local(
    binary: &Path,
    root: &Path,
    socket: &Path,
    command: &str,
    id: Option<&Uuid>,
    commands: &mut Vec<Value>,
) -> io::Result<String> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg(command);
    if let Some(id) = id {
        cmd.arg(&id.0);
    }
    cmd.args(["--admin-socket"])
        .arg(socket)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .kill_on_drop(true);
    commands.push(json!({"executable":binary,"command":command,"deviceId":id,"adminSocket":socket,"configHome":root.join("config"),"stateHome":root.join("state")}));
    let output = tokio::time::timeout(Duration::from_secs(15), cmd.output())
        .await
        .map_err(|_| failure("local command timed out"))??;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err(failure("packaged local command failed"));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}
struct WatchTasks {
    paths: tokio::task::JoinHandle<()>,
    changes: tokio::task::JoinHandle<()>,
}
impl Drop for WatchTasks {
    fn drop(&mut self) {
        self.paths.abort();
        self.changes.abort();
    }
}
async fn rpc(
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
    body: RequestBody,
) -> io::Result<ResultBody> {
    let method = body.method();
    let id = Uuid::random();
    let mut bytes = encode_payload(&Envelope::Request(Request {
        id: id.clone(),
        body,
    }))
    .map_err(|_| failure("request encoding failed"))?;
    let wrote = transport::write_frame(send, &bytes).await;
    bytes.fill(0);
    wrote?;
    let bytes = tokio::time::timeout(
        Duration::from_secs(15),
        transport::read_frame(recv, FRAME_MAX),
    )
    .await
    .map_err(|_| failure("framed exchange timed out"))??;
    let Envelope::Response(response) =
        decode_payload(&bytes, Some(method)).map_err(|_| failure("response codec failed"))?
    else {
        return Err(failure("unexpected exchange frame"));
    };
    if response.id != id {
        return Err(failure("response correlation failed"));
    }
    response
        .outcome
        .map_err(|_| failure("remote operation rejected"))
}
async fn exchange(
    phone: &iroh::Endpoint,
    addr: iroh::EndpointAddr,
    invite: PairInvite,
    mode: &str,
    events: Arc<Mutex<Vec<Value>>>,
    unpair: bool,
) -> io::Result<(iroh::endpoint::Connection, Uuid)> {
    let connection =
        tokio::time::timeout(Duration::from_secs(45), phone.connect(addr, goatr::ALPN))
            .await
            .map_err(|_| failure("real transport unavailable"))?
            .map_err(|_| failure("real transport unavailable"))?;
    let observation_start = events.lock().expect("path observation mutex").len();
    if connection.remote_id().to_string() != invite.host.0 {
        return Err(failure("TLS remote identity mismatch"));
    }
    let paths_connection = connection.clone();
    let observed = events.clone();
    let paths = tokio::spawn(async move {
        let mut stream = paths_connection.paths_stream();
        while let Some(paths) = stream.next().await {
            for path in paths.iter().filter(|p| p.is_selected()) {
                let transport = if path.is_ip() {
                    "direct"
                } else if path.is_relay() {
                    "relay"
                } else {
                    "other"
                };
                let mut events = observed.lock().expect("path observation mutex");
                if events.len() < 256 {
                    events.push(json!({"source":"Connection.paths_stream","event":"selectedPath","transport":transport,"remote":path.remote_addr().to_string()}));
                }
            }
        }
    });
    let observed = events.clone();
    let mut stream = connection.path_events();
    let changes = tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            let value = match event {
                iroh::endpoint::PathEvent::Selected { remote_addr, .. } => {
                    json!({"source":"Connection.path_events","event":"selected","remote":remote_addr.to_string()})
                }
                iroh::endpoint::PathEvent::Opened { remote_addr, .. } => {
                    json!({"source":"Connection.path_events","event":"opened","remote":remote_addr.to_string()})
                }
                iroh::endpoint::PathEvent::Closed { remote_addr, .. } => {
                    json!({"source":"Connection.path_events","event":"closed","remote":remote_addr.to_string()})
                }
                iroh::endpoint::PathEvent::Lagged { missed, .. } => {
                    json!({"source":"Connection.path_events","event":"lagged","missed":missed})
                }
                _ => continue,
            };
            let mut events = observed.lock().expect("path observation mutex");
            if events.len() < 256 {
                events.push(value);
            }
        }
    });
    let mut watches = WatchTasks { paths, changes };
    let result = async {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|_| failure("bidirectional stream failed"))?;
        let paired = rpc(
            &mut send,
            &mut recv,
            RequestBody::PairConfirm(PairConfirmParams {
                token: invite.token,
                device_name: Label::new("Goatr transport gate").expect("literal"),
            }),
        )
        .await?;
        let ResultBody::PairConfirm(paired) = paired else {
            return Err(failure("pair result missing"));
        };
        let hello = rpc(
            &mut send,
            &mut recv,
            RequestBody::Hello(HelloParams {
                app_version: env!("CARGO_PKG_VERSION").into(),
                device_name: Label::new("Goatr transport gate").expect("literal"),
            }),
        )
        .await?;
        let ResultBody::Hello(hello) = hello else {
            return Err(failure("hello result missing"));
        };
        if hello.device_id != paired.device_id || hello.host_id != invite.host {
            return Err(failure("authenticated hello mismatch"));
        }
        let snapshot = rpc(
            &mut send,
            &mut recv,
            RequestBody::HostSnapshot(HostSnapshotParams {}),
        )
        .await?;
        let ResultBody::HostSnapshot(snapshot) = snapshot else {
            return Err(failure("snapshot result missing"));
        };
        if snapshot.seq.0 != 0 {
            return Err(failure("snapshot sequence must be zero"));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while !events.lock().expect("path observation mutex")[observation_start..]
                .iter()
                .any(|e| e.get("transport").and_then(Value::as_str) == Some(mode))
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| failure("required selected path not observed"))?;
        if unpair
            && !matches!(
                rpc(
                    &mut send,
                    &mut recv,
                    RequestBody::DeviceUnpair(DeviceUnpairParams {
                        boot_id: hello.boot_id
                    })
                )
                .await?,
                ResultBody::DeviceUnpair(_)
            )
        {
            return Err(failure("self-unpair final response missing"));
        }
        Ok(hello.device_id)
    }
    .await;
    watches.paths.abort();
    watches.changes.abort();
    let _ = (&mut watches.paths).await;
    let _ = (&mut watches.changes).await;
    match result {
        Ok(id) => Ok((connection, id)),
        Err(e) => {
            connection.close(0u32.into(), b"gate failed");
            Err(e)
        }
    }
}
fn removable_tree(path: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if m.uid() != uid() || m.file_type().is_symlink() {
        return Err(failure("unowned cleanup entry"));
    }
    if m.is_dir() {
        if m.mode() & 0o7777 != 0o700 {
            return Err(failure("nonprivate cleanup directory"));
        }
        for entry in fs::read_dir(path)? {
            removable_tree(&entry?.path())?;
        }
    } else if !m.is_file() || m.mode() & 0o7777 != 0o600 || m.nlink() != 1 {
        return Err(failure("unsafe cleanup entry"));
    }
    Ok(())
}
#[cfg(test)]
mod terminal_pair_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn terminal_pair_classifier_accepts_one_authored_remote_admin_error() {
        let mut classifier = TerminalPairStderrClassifier::default();
        classifier.push(b"goatr: local admin busy\n");
        classifier.eof();
        assert_eq!(classifier.result(), TerminalPairStderr::LocalAdminBusy);
    }

    #[test]
    fn terminal_pair_classifier_discards_unmatched_and_truncated_stderr() {
        let mut unmatched = TerminalPairStderrClassifier::default();
        unmatched.push(b"untrusted private detail\ngoatr: local admin busy\n");
        unmatched.eof();
        assert_eq!(unmatched.result(), TerminalPairStderr::Unmatched);

        let mut truncated = TerminalPairStderrClassifier::default();
        truncated.push(&vec![b'x'; TerminalPairStderrClassifier::MAX_BYTES + 1]);
        truncated.eof();
        assert_eq!(truncated.result(), TerminalPairStderr::Truncated);
    }

    #[test]
    fn terminal_pair_exit_metadata_preserves_closed_status_without_stderr() {
        let code = terminal_pair_failure(
            ExitStatus::from_raw(23 << 8),
            TerminalPairStderr::LocalAdminBusy,
        );
        assert_eq!(
            helper_error_metadata(&code)["pair_exit"],
            json!({"code":23,"signal":null,"stderr":"local_admin_busy"})
        );

        let signal = terminal_pair_failure(
            ExitStatus::from_raw(libc::SIGTERM),
            TerminalPairStderr::None,
        );
        assert_eq!(
            helper_error_metadata(&signal)["pair_exit"],
            json!({"code":null,"signal":libc::SIGTERM,"stderr":"none"})
        );
        assert!(
            !helper_error_metadata(&code)
                .to_string()
                .contains("untrusted private detail")
        );
    }
}
#[cfg(test)]
mod transport_retention_tests {
    use super::*;

    fn fixture() -> (OwnedRun, std::process::ChildStdin, PathBuf, goatr::herdr::ProcessEvidence) {
        let parent = std::env::temp_dir().join(format!("goatr-transport-retention-{}",Uuid::random().0));
        let root = parent.join("root");
        let admin = parent.join("admin");
        for directory in [&parent,&root,&admin] {
            fs::DirBuilder::new().mode(0o700).create(directory).unwrap();
        }
        let root_original = transport_identity(&root,&fs::symlink_metadata(&root).unwrap());
        let admin_original = transport_identity(&admin,&fs::symlink_metadata(&admin).unwrap());
        // Keep stdin outside OwnedRun: dropping Child must not make this fixture exit.
        let mut command = std::process::Command::new("cat");
        command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut owned = OwnedRun::spawn(&mut command,root,root_original,admin,admin_original).unwrap();
        let input = owned.child.stdin.take().unwrap();
        let original = owned.original_process.as_ref().unwrap().clone();
        (owned,input,parent,original)
    }

    fn finish_fixture(input: std::process::ChildStdin, parent: &Path, pid: u32) {
        drop(input); // EOF, not a signal or unreviewed stop of the retained owner.
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid as libc::pid_t,&mut status,0) },pid as libc::pid_t);
        assert!(libc::WIFEXITED(status));
        assert_eq!(libc::WEXITSTATUS(status),0);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn transport_owner_drop_retains_original_child_and_private_roots() {
        let (owned,input,parent,original) = fixture();
        let root_original = owned.root_original.clone();
        let admin_original = owned.admin_original.clone();
        drop(owned);
        assert_eq!(goatr::herdr::process_evidence(original.pid).unwrap(),original);
        assert_eq!(retained_resource(&parent.join("root"),Some(&root_original))["matchesOriginal"],true);
        assert_eq!(retained_resource(&parent.join("admin"),Some(&admin_original))["matchesOriginal"],true);
        finish_fixture(input,&parent,original.pid);
    }

    #[test]
    fn retained_transport_launcher_isolates_child_from_controller_process_group() {
        let controller_group = unsafe { libc::getpgrp() };
        assert!(controller_group > 0);
        let (owned,input,parent,original) = fixture();
        let child_group = unsafe { libc::getpgid(original.pid as libc::pid_t) };
        assert_eq!(child_group,original.pid as libc::pid_t);
        assert_ne!(child_group,controller_group);
        assert_eq!(unsafe { libc::getpgrp() },controller_group);
        drop(owned);
        assert_eq!(goatr::herdr::process_evidence(original.pid).unwrap(),original);
        finish_fixture(input,&parent,original.pid);
    }

    #[tokio::test]
    async fn cancelled_transport_owner_retains_original_child_and_private_roots() {
        let (owned,input,parent,original) = fixture();
        let root_original = owned.root_original.clone();
        let admin_original = owned.admin_original.clone();
        let (entered,ready) = tokio::sync::oneshot::channel();
        let owner = tokio::spawn(async move {
            let retained = owned;
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
            drop(retained);
        });
        ready.await.unwrap();
        owner.abort();
        assert!(owner.await.unwrap_err().is_cancelled());
        assert_eq!(goatr::herdr::process_evidence(original.pid).unwrap(),original);
        assert_eq!(retained_resource(&parent.join("root"),Some(&root_original))["matchesOriginal"],true);
        assert_eq!(retained_resource(&parent.join("admin"),Some(&admin_original))["matchesOriginal"],true);
        finish_fixture(input,&parent,original.pid);
    }

    #[test]
    fn retained_comparisons_do_not_adopt_changed_or_previously_unproved_identities() {
        let (mut owned,input,parent,original) = fixture();
        let root_original = owned.root_original.clone();
        let mut changed = original.clone();
        changed.namespaces.mount.1 ^= 1;
        let changed_observation = owned.process_comparison(Ok(changed.clone()));
        assert_eq!(changed_observation["matchesOriginal"],false);
        assert_eq!(changed_observation["original"],json!(original));
        assert_eq!(changed_observation["current"],json!(changed));
        assert_eq!(owned.original_process.as_ref().unwrap(),&original);
        assert_eq!(owned.process_comparison(Err(io::ErrorKind::NotFound))["matchesOriginal"],false);

        fs::rename(&owned.root,parent.join("original-root")).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&owned.root).unwrap();
        let socket = owned.admin_parent.join("admin.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let observed = owned.retained_resources(&socket,None);
        assert_eq!(observed["root"]["matchesOriginal"],false);
        assert_eq!(observed["root"]["original"],root_original);
        assert_eq!(owned.root_original,root_original);
        assert_eq!(observed["adminSocket"]["matchesOriginal"],false);
        assert!(observed["adminSocket"]["original"].is_null());
        assert_eq!(observed["adminSocket"]["current"]["kind"],"socket");
        assert_eq!(observed["cleanupAuthority"],false);
        assert_eq!(observed["cleanupPerformed"],false);

        owned.original_process = Err(io::ErrorKind::PermissionDenied);
        let unproved = owned.process_comparison(Ok(original.clone()));
        assert_eq!(unproved["matchesOriginal"],false);
        assert!(unproved["original"].is_null());
        assert_eq!(unproved["current"],json!(original));
        assert_eq!(owned.original_process.as_ref().unwrap_err(),&io::ErrorKind::PermissionDenied);
        drop(owned);
        finish_fixture(input,&parent,original.pid);
    }
}
async fn run() -> io::Result<bool> {
    let args = Command::new("smoke")
        .about("Non-installed real packaged direct/public-N0 transport gate only")
        .arg(
            Arg::new("transport")
                .long("transport")
                .required(true)
                .value_parser(["direct", "relay"]),
        )
        .arg(
            Arg::new("evidence-dir")
                .long("evidence-dir")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("goatr")
                .long("goatr")
                .value_parser(value_parser!(PathBuf)),
        )
        .get_matches();
    let mode = args.get_one::<String>("transport").expect("required mode");
    let evidence = std::path::absolute(
        args.get_one::<PathBuf>("evidence-dir")
            .expect("required evidence"),
    )?;
    private_dir(&evidence, true)?;
    let binary = if let Some(p) = args.get_one::<PathBuf>("goatr") {
        p.clone()
    } else if let Some(p) = std::env::var_os("GOATR_BIN") {
        PathBuf::from(p)
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join("goatr"))
            .find(|p| p.is_file())
            .ok_or_else(|| {
                failure("packaged goatr missing; pass --goatr /nix/store/.../bin/goatr")
            })?
    };
    let binary = fs::canonicalize(binary)?;
    if !binary.starts_with("/nix/store") || binary.file_name().is_none_or(|s| s != "goatr") {
        return Err(failure("gate requires an actual packaged Nix goatr binary"));
    }
    let version = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(&binary)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| failure("packaged version deadline exceeded"))??;
    if !version.status.success() || version.stdout.len() > 4096 {
        return Err(failure("packaged version unavailable"));
    }
    let version = String::from_utf8(version.stdout)
        .map_err(io::Error::other)?
        .trim()
        .to_owned();
    let run_id = Uuid::random();
    let mut random = random32()?;
    let suffix = hex(&random[..6]);
    random.fill(0);
    let runtime = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| failure("unchanged XDG_RUNTIME_DIR required"))?,
    );
    if !runtime.is_absolute()
        || fs::canonicalize(&runtime)? != runtime
        || fs::symlink_metadata(&runtime)?.uid() != uid()
    {
        return Err(failure("runtime root is not canonical/same-UID"));
    }
    let linux_boot = boot()?;
    let _ = start(std::process::id())?;
    let workspace = fs::canonicalize(std::env::current_dir()?)?;
    let root = workspace.join(format!(".gs-{suffix}"));
    let admin_parent = runtime.join(format!("gs-{suffix}"));
    let socket = admin_parent.join("admin.sock");
    let hints = root.join("hints.json");
    if socket.to_str().is_none_or(|p| p.len() > 107) {
        return Err(failure("owned admin socket exceeds Linux pathname bound"));
    }
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    private_dir(&root, false)?;
    for child in ["config", "state"] {
        fs::DirBuilder::new().mode(0o700).create(root.join(child))?;
    }
    fs::DirBuilder::new().mode(0o700).create(&admin_parent)?;
    private_dir(&admin_parent, false)?;
    goatr::admin::validate_socket_path(&socket, false, false)?;
    let root_meta = fs::symlink_metadata(&root)?;
    let admin_meta = fs::symlink_metadata(&admin_parent)?;
    let root_original = transport_identity(&root, &root_meta);
    let admin_original = transport_identity(&admin_parent, &admin_meta);
    let ownership_path = evidence.join(format!("ownership-{}.json", run_id.0));
    let process_path = evidence.join(format!("process-{}.json", run_id.0));
    let gate_path = evidence.join(format!("gate-{}.json", run_id.0));
    let candidate_path = evidence.join(format!("cleanup-candidate-{}.json", run_id.0));
    write_json(
        &ownership_path,
        &json!({"runId":run_id,"linuxBoot":linux_boot,"root":root_original,"adminParent":admin_original,"adminSocket":socket,"configHome":root.join("config"),"stateHome":root.join("state"),"XDG_RUNTIME_DIR":runtime,"packageExecutable":binary,"stage":"owned roots; no child yet"}),
    )?;
    let mut cmd = std::process::Command::new(&binary);
    cmd.arg("serve");
    scoped(&mut cmd, &root, &socket);
    cmd.args(["--transport-smoke", mode, "--local-hints-file"])
        .arg(&hints)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut owned = OwnedRun::spawn(&mut cmd, root.clone(), root_original, admin_parent.clone(), admin_original)?;
    let process_record = write_json(
        &process_path,
        &json!({"runId":run_id,"linuxBoot":linux_boot,"pid":owned.child.id(),
            "originalProcess":owned.original_process.as_ref().ok(),
            "captureErrorKind":owned.original_process.as_ref().err().map(|kind|format!("{kind:?}")),
            "packageExecutable":binary,"adminSocket":socket,"cleanup":"pending_owner_review"}),
    );
    let process_record_written = process_record.is_ok();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut commands = vec![
        json!({"executable":binary,"command":"--version"}),
        json!({"executable":binary,"command":"serve","transportSmoke":mode,"adminSocket":socket,"hintsFile":hints,"configHome":root.join("config"),"stateHome":root.join("state"),"runtimeDirUnchanged":runtime}),
    ];
    let mut host_identity = None;
    let mut phone_identity = None;
    let mut admin_identity = None;
    let mut phone_owner: Option<iroh::Endpoint> = None;
    let mut result = tokio::select! {result=async {
        let deadline = Instant::now() + Duration::from_secs(60);
        process_record?;
        if let Err(kind) = owned.original_process.as_ref() {
            return Err(io::Error::new(*kind,"original owned process evidence unavailable; retained for owner review"));
        }
        while !hints.exists() || !socket.exists() {
            if owned.child.try_wait()?.is_some() {
                let mut diagnostic = String::new();
                if let Some(stderr) = owned.child.stderr.take() {
                    stderr.take(4096).read_to_string(&mut diagnostic)?;
                }
                if mode == "relay" && diagnostic.contains("deadline has elapsed") {
                    return Err(failure("public N0 relay readiness unavailable"));
                }
                return Err(failure("packaged serve exited before readiness"));
            }
            if Instant::now() > deadline {
                return Err(failure("public relay/discovery readiness unavailable"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        private_file(&hints)?;admin_identity=Some(transport_identity(&socket,&fs::symlink_metadata(&socket)?));let mut bytes=Vec::new();File::open(&hints)?.take(65537).read_to_end(&mut bytes)?;if bytes.len()>65536 {return Err(failure("address hints exceeded bound"));}let public:transport::LocalHints=serde_json::from_slice(&bytes).map_err(io::Error::other)?;if public.companion_lock_sha256!=transport::lock_sha256() || public.core_iroh!="1.3.0" || public.package_version!=env!("CARGO_PKG_VERSION") {return Err(failure("packaged source/lock transport provenance mismatch"));}let addr=public.endpoint_addr;host_identity=Some(addr.id.to_string());
        let mut builder=goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?));if mode=="direct" {builder=builder.relay_mode(iroh::RelayMode::Disabled).clear_address_lookup();if addr.ip_addrs().next().is_none() || addr.relay_urls().next().is_some() {return Err(failure("direct gate requires actual local hints and no relays"));}} else {builder=builder.clear_ip_transports();}
        phone_owner=Some(builder.bind().await.map_err(|_|failure("client endpoint unavailable"))?);let phone=phone_owner.as_ref().expect("bound client");phone_identity=Some(phone.id().to_string());
        async {
            commands.push(json!({"executable":binary,"command":"pair","adminSocket":socket,"output":"private PTY; URI/token memory only"}));let b=binary.clone();let r=root.clone();let s=socket.clone();let uri=tokio::task::spawn_blocking(move ||terminal_pair(&b,Some(&r),Some(&s))).await.map_err(|_|failure("PTY worker failed"))??;let invite=PairInvite::parse(&uri).map_err(|_|failure("pair URI rejected"))?;drop(uri);
            let (first,id)=exchange(phone,addr.clone(),invite,mode,events.clone(),false).await?;
            let list=local(&binary,&root,&socket,"devices",None,&mut commands).await?;if !list.contains(&id.0) {return Err(failure("real devices command missed paired peer"));}
            local(&binary,&root,&socket,"revoke",Some(&id),&mut commands).await?;first.close(0u32.into(),b"revoked");
            commands.push(json!({"executable":binary,"command":"pair","adminSocket":socket,"output":"private PTY; URI/token memory only"}));let b=binary.clone();let r=root.clone();let s=socket.clone();let uri=tokio::task::spawn_blocking(move ||terminal_pair(&b,Some(&r),Some(&s))).await.map_err(|_|failure("PTY worker failed"))??;let invite=PairInvite::parse(&uri).map_err(|_|failure("fresh pair URI rejected"))?;drop(uri);
            let (last,_)=exchange(phone,addr,invite,mode,events.clone(),true).await?;last.close(0u32.into(),b"unpaired");if !local(&binary,&root,&socket,"devices",None,&mut commands).await?.is_empty() {return Err(failure("self-unpair did not remove pairing"));}Ok(())
        }.await
    }=>result,_=tokio::signal::ctrl_c()=>Err(failure("transport gate cancelled")),_=terminate.recv()=>Err(failure("transport gate cancelled"))};
    if let Some(phone) = phone_owner.take() {
        let closed = goatr::close_endpoint(&phone).await;
        drop(phone);
        if result.is_ok() && closed.is_err() {
            result = Err(failure("client endpoint shutdown deadline"));
        }
    }
    let transport_observation = if result.is_ok() {
        "pass"
    } else if owned.original_process.is_err()
        || (mode == "relay" && result.as_ref().err().is_some_and(|error|error.to_string().contains("unavailable")))
    {
        "blocked"
    } else {
        "fail"
    };
    let status = if transport_observation == "pass" { "pending_owner_review" } else { transport_observation };
    let retained = owned.retained_resources(&socket, admin_identity.as_ref());
    let package_sha256 = binary_sha256(&binary);
    let report = json!({"gate":format!("transport-{mode}"),"status":status,
        "transportObservation":transport_observation,"cleanup":"pending_owner_review","fullGateComplete":false,
        "exitStatusScope":"transport_observation_only","runId":run_id,"version":version,
        "packageExecutable":binary,"packageSha256":package_sha256.as_ref().ok(),
        "packageHashErrorKind":package_sha256.as_ref().err().map(|error|format!("{:?}",error.kind())),
        "companionLockSha256":transport::lock_sha256(),"coreIroh":"1.3.0",
        "tls":"normal N0 public relay TLS; no insecure verification or custom relay",
        "scope":"transport-only; empty real State snapshot is not provider proof",
        "commands":commands,"linuxBoot":linux_boot,"retainedResources":retained,
        "hostEndpoint":host_identity,"phoneEndpoint":phone_identity,
        "selectedPath.events":events.lock().expect("path observation mutex").clone(),
        "failure":result.as_ref().err().map(|error|error.to_string()),
        "configHome":root.join("config"),"stateHome":root.join("state"),"XDG_RUNTIME_DIR":runtime,
        "ownershipRecord":ownership_path,"processRecord":process_path,"cleanupCandidate":candidate_path});
    let gate_record = write_json(&gate_path, &report);
    // This is passive evidence, never approval or a substitute for fresh owner revalidation.
    let candidate_record = write_json(&candidate_path, &json!({"runId":run_id,
        "kind":"retained_transport_resources","transportObservation":transport_observation,
        "cleanup":"pending_owner_review","cleanupPerformed":false,"cleanupAuthority":false,
        "fullGateComplete":false,"ownershipRecord":ownership_path,"processRecord":process_path,
        "processRecordWritten":process_record_written,"outcomeRecord":gate_path,
        "outcomeRecordWritten":gate_record.is_ok(),"resources":retained,
        "hostEndpoint":host_identity,"phoneEndpoint":phone_identity}));
    gate_record?;
    candidate_record?;
    println!("{}",json!({"gate":format!("transport-{mode}"),"status":status,
        "transportObservation":transport_observation,"cleanup":"pending_owner_review",
        "fullGateComplete":false,"exitStatusScope":"transport_observation_only",
        "evidence":gate_path,"cleanupCandidate":candidate_path}));
    Ok(transport_observation == "pass")
}
#[tokio::main(worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    let background=std::env::args_os().skip(1).any(|arg|arg.to_str().is_some_and(|arg|arg=="--background"||arg.starts_with("--background=")));
    let mut native_args = std::env::args_os().skip(1);
    if !background&&native_args.next().as_deref() == Some(std::ffi::OsStr::new("--owned-mcp-stdio")) {
        if std::env::args_os().any(|arg|arg==std::ffi::OsStr::new("--project-read-only")){return std::process::ExitCode::FAILURE;}
        let Some(lease) = native_args.next() else {
            return std::process::ExitCode::FAILURE;
        };
        if native_args.next().is_some() {
            return std::process::ExitCode::FAILURE;
        }
        return match live::owned_mcp_stdio(&PathBuf::from(lease)).await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => {
                eprintln!("owned_native_mcp_failed");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let result = if background {live::run().await} else if !std::env::args().any(|arg|matches!(arg.as_str(),"--terminal-only"|"--project-read-only"))&&std::env::args().any(|arg| arg == "--review-fixture") {
        review::run().await
    } else if std::env::args().any(|arg| {
        matches!(
            arg.as_str(),
            "--backend"
                | "--admission-only"
                | "--shared-agent-only"
                | "--terminal-only"
                | "--project-read-only"
                | "--grid"
                | "--ui"
                | "--prepare-emulator"
                | "--camera"
                | "--ime"
                | "--setup"
                | "--automate-native-inputs"
                | "--defer-retention-crossing"
                | "--physical-hello-capture"
        )
    }) {
        live::run().await
    } else {
        run().await
    };
    match result {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(e) => {
            let mut outcome = json!({"gate":"selected-smoke-mode","status":if matches!(e.kind(),io::ErrorKind::NotFound|io::ErrorKind::NotConnected|io::ErrorKind::ConnectionRefused|io::ErrorKind::TimedOut|io::ErrorKind::PermissionDenied){"blocked"}else{"fail"},"reason":e.to_string()});
            outcome.as_object_mut().expect("object").extend(
                helper_error_metadata(&e)
                    .as_object()
                    .expect("object")
                    .clone(),
            );
            println!("{}", outcome);
            std::process::ExitCode::FAILURE
        }
    }
}
