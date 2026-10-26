// SPDX-License-Identifier: GPL-3.0-or-later
//! A single bounded read-only Git child; never a shell, helper runner or file authority.
use super::*;
use crate::herdr::ProcessEvidence;
use std::os::unix::fs::FileTypeExt;
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::process::CommandExt,
    process::{Child, ChildStdout, Command, Stdio},
};
const MAX_TRACEES: usize = 64;
const PROC_BYTES: usize = 128 * 1024;
const POLL: Duration = Duration::from_micros(100);
const GET_SYSCALL_INFO: libc::c_uint = 0x420e;
#[repr(C)]
struct SyscallInfo {
    operation: u8,
    padding: [u8; 3],
    architecture: u32,
    ip: u64,
    sp: u64,
    data: SyscallData,
}
#[repr(C)]
union SyscallData {
    entry: Entry,
    exit: Exit,
    seccomp: [u64; 8],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    number: u64,
    args: [u64; 6],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Exit {
    result: i64,
    error: u8,
    padding: [u8; 7],
}
#[repr(C)]
struct Ruleset {
    filesystem: u64,
    network: u64,
    scoped: u64,
}
#[repr(C, packed)]
struct PathRule {
    access: u64,
    parent: i32,
}
struct Landlock {
    file: File,
}
fn denied<T>() -> SecurityResult<T> {
    Err(ErrorCode::NotReady)
}
pub(super) fn supported() -> bool {
    cfg!(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )) && unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<Ruleset>(),
            0,
            1,
        )
    } >= 6
}
impl Landlock {
    fn new() -> SecurityResult<Self> {
        if !supported() {
            return denied();
        }
        let attributes = Ruleset {
            filesystem: (1 << 16) - 1,
            network: 3,
            scoped: 3,
        };
        let raw = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attributes,
                std::mem::size_of::<Ruleset>(),
                0,
            )
        };
        if raw < 0 {
            return denied();
        }
        Ok(Self {
            file: unsafe { File::from_raw_fd(raw as i32) },
        })
    }
    fn prepare(
        graph: &admin::Graph,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<Self> {
        let owner = Self::new()?;
        for root in &graph.roots {
            root.current(deadline).map_err(filesystem)?;
            owner.rule(&root.file, (1 << 2) | (1 << 3))?;
        }
        for root in &runtime.roots {
            owner.rule(&root.current(deadline)?, (1 << 2) | (1 << 3))?;
        }
        owner.rule(&runtime.executable.file, (1 << 0) | (1 << 2))?;
        owner.rule(&runtime.interpreter.file, (1 << 0) | (1 << 2))?;
        Ok(owner)
    }
    fn rule(&self, parent: &File, access: u64) -> SecurityResult<()> {
        let rule = PathRule {
            access,
            parent: parent.as_raw_fd(),
        };
        if unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                self.file.as_raw_fd(),
                1,
                &rule,
                0,
            )
        } != 0
        {
            return denied();
        }
        Ok(())
    }
    fn restrict(&self, cwd: i32) -> io::Result<()> {
        crate::codex_relay::drop_inherited_capabilities()?;
        if unsafe { libc::fchdir(cwd) } != 0
            || unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, self.file.as_raw_fd(), 0) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        for (resource, limit) in [
            (libc::RLIMIT_CORE, 0),
            (libc::RLIMIT_NOFILE, 256),
            (libc::RLIMIT_AS, 512 * 1024 * 1024),
            (libc::RLIMIT_CPU, 6),
        ] {
            let limits = libc::rlimit {
                rlim_cur: limit,
                rlim_max: limit,
            };
            if unsafe { libc::setrlimit(resource, &limits) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // Keep Rust's exec-error channel until exec; no unrelated inherited FD survives it.
        if unsafe {
            libc::syscall(
                libc::SYS_close_range,
                3u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            )
        } != 0
            || unsafe {
                libc::ptrace(
                    libc::PTRACE_TRACEME,
                    0,
                    std::ptr::null_mut::<libc::c_void>(),
                    std::ptr::null_mut::<libc::c_void>(),
                )
            } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Scope {
    Project,
    Runtime,
}
struct OpenProof {
    file: Option<File>,
    scope: Scope,
}
enum Pending {
    Open(OpenProof),
    Directory(File),
    Pipe(u64),
    Fork(bool),
}
struct Tracee {
    pid: i32,
    pidfd: Option<File>,
    proc: Option<File>,
    proof: Option<ProcessEvidence>,
    starting: bool,
    bootstrap_exit: bool,
    in_call: bool,
    lost: bool,
    pending: Option<Pending>,
}
impl Tracee {
    fn unpinned(pid: i32, starting: bool) -> Self {
        Self {
            pid,
            pidfd: None,
            proc: None,
            proof: None,
            starting,
            bootstrap_exit: true,
            in_call: false,
            lost: false,
            pending: None,
        }
    }
    fn pin(&mut self, deadline: Instant, parent: Option<&ProcessEvidence>) -> SecurityResult<()> {
        if self.pidfd.is_none() {
            // PIDFD_THREAD is O_EXCL in the frozen Linux UAPI. The trace stop reserves this exact TID.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, self.pid, libc::O_EXCL) };
            if fd < 0 {
                return denied();
            }
            self.pidfd = Some(unsafe { File::from_raw_fd(fd as i32) });
        }
        let proof = crate::herdr::process_evidence(self.pid as u32).map_err(filesystem)?;
        if let Some(parent) = parent {
            if proof.linux_boot_id != parent.linux_boot_id
                || proof.namespaces.pid != parent.namespaces.pid
                || proof.namespaces.mount != parent.namespaces.mount
                || proof.namespaces.user != parent.namespaces.user
            {
                return denied();
            }
        }
        let directory =
            super::super::context::process_directory(&proof, deadline).map_err(filesystem)?;
        self.proc = Some(directory);
        self.proof = Some(proof);
        Ok(())
    }
    fn directory(&self) -> SecurityResult<&File> {
        self.proc.as_ref().ok_or(ErrorCode::NotReady)
    }
    fn link(&self, name: &CStr) -> SecurityResult<PathBuf> {
        let mut buffer = [0u8; 4096];
        let count = unsafe {
            libc::readlinkat(
                self.directory()?.as_raw_fd(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if count <= 0 || count as usize == buffer.len() {
            return denied();
        }
        let text =
            std::str::from_utf8(&buffer[..count as usize]).map_err(|_| ErrorCode::NotReady)?;
        if !text.starts_with('/')
            || text.chars().any(char::is_control)
            || text.ends_with(" (deleted)")
        {
            return denied();
        }
        Ok(PathBuf::from(text))
    }
    fn object(&self, name: &CStr) -> SecurityResult<File> {
        let fd = unsafe {
            libc::openat(
                self.directory()?.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return denied();
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn fd(&self, fd: i32) -> SecurityResult<(PathBuf, File)> {
        if fd < 0 {
            return denied();
        }
        let name = CString::new(format!("fd/{fd}")).map_err(|_| ErrorCode::NotReady)?;
        Ok((self.link(&name)?, self.object(&name)?))
    }
    fn gone(&self) -> bool {
        let Some(fd) = &self.pidfd else {
            return false;
        };
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        (unsafe { libc::poll(&mut poll, 1, 0) }) > 0
            && poll.revents & (libc::POLLIN | libc::POLLHUP) != 0
    }
    fn kill(&self) {
        if let Some(fd) = &self.pidfd {
            let _ = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    1u32,
                )
            };
        } else if !self.lost {
            let _ = trace(
                self.pid,
                libc::PTRACE_KILL,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
    }
}
fn trace(
    pid: i32,
    request: libc::c_uint,
    address: *mut libc::c_void,
    data: *mut libc::c_void,
) -> SecurityResult<()> {
    if unsafe { libc::ptrace(request, pid, address, data) } != 0 {
        return denied();
    }
    Ok(())
}
fn resume(pid: i32, signal: i32) -> SecurityResult<()> {
    trace(
        pid,
        libc::PTRACE_SYSCALL,
        std::ptr::null_mut(),
        signal as usize as *mut libc::c_void,
    )
}
fn options(pid: i32) -> SecurityResult<()> {
    trace(
        pid,
        libc::PTRACE_SETOPTIONS,
        std::ptr::null_mut(),
        (libc::PTRACE_O_TRACESYSGOOD
            | libc::PTRACE_O_TRACEFORK
            | libc::PTRACE_O_TRACEVFORK
            | libc::PTRACE_O_TRACECLONE
            | libc::PTRACE_O_TRACEEXEC
            | libc::PTRACE_O_TRACEEXIT
            | libc::PTRACE_O_EXITKILL) as usize as *mut libc::c_void,
    )
}
fn event_pid(pid: i32) -> SecurityResult<i32> {
    let mut value = 0usize;
    trace(
        pid,
        libc::PTRACE_GETEVENTMSG,
        std::ptr::null_mut(),
        (&mut value as *mut usize).cast(),
    )?;
    i32::try_from(value)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or(ErrorCode::NotReady)
}
fn info(pid: i32) -> SecurityResult<SyscallInfo> {
    let mut info = std::mem::MaybeUninit::<SyscallInfo>::zeroed();
    let count = unsafe {
        libc::ptrace(
            GET_SYSCALL_INFO,
            pid,
            std::mem::size_of::<SyscallInfo>() as *mut libc::c_void,
            info.as_mut_ptr(),
        )
    };
    if count < 0 {
        return denied();
    }
    let info = unsafe { info.assume_init() };
    let architecture = if cfg!(target_arch = "x86_64") {
        0xc000003e
    } else {
        0xc00000b7
    };
    if info.architecture != architecture
        || !matches!(info.operation, 1 | 2)
        || count < if info.operation == 1 { 80 } else { 33 }
    {
        return denied();
    }
    Ok(info)
}
fn integer(value: u64) -> SecurityResult<i32> {
    let integer = value as u32 as i32;
    if value != integer as u32 as u64 && value != integer as i64 as u64 {
        return denied();
    }
    Ok(integer)
}
fn memory(pid: i32, address: u64, output: &mut [u8]) -> SecurityResult<()> {
    if address == 0 || address.checked_add(output.len() as u64).is_none() {
        return denied();
    }
    let word = std::mem::size_of::<libc::c_long>();
    let mut offset = 0;
    while offset < output.len() {
        let pointer = address + offset as u64;
        let aligned = pointer & !((word - 1) as u64);
        let skip = (pointer - aligned) as usize;
        unsafe {
            *libc::__errno_location() = 0;
        }
        let value = unsafe {
            libc::ptrace(
                libc::PTRACE_PEEKDATA,
                pid,
                aligned as *mut libc::c_void,
                std::ptr::null_mut::<libc::c_void>(),
            )
        };
        if value == -1 && unsafe { *libc::__errno_location() } != 0 {
            return denied();
        }
        let bytes = value.to_ne_bytes();
        let count = (word - skip).min(output.len() - offset);
        output[offset..offset + count].copy_from_slice(&bytes[skip..skip + count]);
        offset += count;
    }
    Ok(())
}
fn pathname(pid: i32, address: u64) -> SecurityResult<PathBuf> {
    if address == 0 {
        return denied();
    }
    let mut output = [0u8; 4096];
    let mut length = 0;
    while length < output.len() {
        // One aligned machine word never reads across an unproved following page.
        let count = (8 - ((address + length as u64) & 7) as usize).min(output.len() - length);
        memory(
            pid,
            address + length as u64,
            &mut output[length..length + count],
        )?;
        if let Some(end) = output[length..length + count]
            .iter()
            .position(|byte| *byte == 0)
        {
            let text =
                std::str::from_utf8(&output[..length + end]).map_err(|_| ErrorCode::NotReady)?;
            if text.is_empty() || text.chars().any(char::is_control) {
                return denied();
            }
            return Ok(PathBuf::from(text));
        }
        length += count;
    }
    Err(ErrorCode::ResourceLimit)
}
fn same(left: &File, right: &File, deadline: Instant) -> SecurityResult<()> {
    if object_identity(left, &left.metadata().map_err(filesystem)?, deadline).map_err(filesystem)?
        != object_identity(right, &right.metadata().map_err(filesystem)?, deadline)
            .map_err(filesystem)?
    {
        return denied();
    }
    Ok(())
}
fn path(tracee: &Tracee, dir: i32, value: PathBuf) -> SecurityResult<PathBuf> {
    if value.is_absolute() {
        return Ok(value);
    }
    let base = if dir == libc::AT_FDCWD {
        tracee.link(c"cwd")?
    } else {
        tracee.fd(dir)?.0
    };
    Ok(base.join(value))
}
fn opening(
    graph: &mut admin::Graph,
    runtime: &runtime::Runtime,
    path: &Path,
    deadline: Instant,
) -> SecurityResult<OpenProof> {
    graph.context.recheck(deadline)?;
    if runtime
        .roots
        .iter()
        .any(|root| path.strip_prefix(&root.path).is_ok())
    {
        Ok(OpenProof {
            file: runtime.request(path, deadline)?,
            scope: Scope::Runtime,
        })
    } else {
        Ok(OpenProof {
            file: graph.request(path, deadline)?,
            scope: Scope::Project,
        })
    }
}
fn inspect_fd(
    graph: &mut admin::Graph,
    runtime: &runtime::Runtime,
    tracee: &Tracee,
    fd: i32,
    execute: bool,
    deadline: Instant,
) -> SecurityResult<()> {
    let (path, file) = tracee.fd(fd)?;
    let proof = opening(graph, runtime, &path, deadline)?;
    same(
        proof.file.as_ref().ok_or(ErrorCode::NotReady)?,
        &file,
        deadline,
    )?;
    if execute && !matches!(proof.scope, Scope::Runtime) {
        return denied();
    }
    if matches!(proof.scope, Scope::Project) {
        graph.observe_fd(&path, &file, deadline)?;
    }
    Ok(())
}
fn readable_flags(flags: u64) -> bool {
    let allowed = (libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_DIRECTORY
        | libc::O_NONBLOCK
        | libc::O_LARGEFILE
        | libc::O_PATH
        | libc::O_NOATIME
        | libc::O_NOCTTY) as u64;
    flags & libc::O_ACCMODE as u64 == libc::O_RDONLY as u64 && flags & !allowed == 0
}
fn clone_flags(flags: u64) -> bool {
    const ALLOWED: u64 = 0xff
        | 0x100
        | 0x200
        | 0x400
        | 0x800
        | 0x4000
        | 0x10000
        | 0x40000
        | 0x80000
        | 0x100000
        | 0x200000
        | 0x1000000;
    flags & !ALLOWED == 0 && matches!(flags & 0xff, 0 | 17)
}
fn mapping_flags(protection: i32, flags: i32) -> bool {
    protection & !(libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC) == 0
        && !(protection & libc::PROT_EXEC != 0 && protection & libc::PROT_WRITE != 0)
        && !(protection & libc::PROT_WRITE != 0
            && matches!(
                flags & libc::MAP_TYPE,
                libc::MAP_SHARED | libc::MAP_SHARED_VALIDATE
            ))
        && !(flags & libc::MAP_ANONYMOUS != 0 && protection & libc::PROT_EXEC != 0)
}

fn simple(number: i64) -> bool {
    #[cfg(target_arch = "x86_64")]
    if matches!(number, libc::SYS_alarm | libc::SYS_getpgrp) {
        return true;
    }
    matches!(
        number,
        libc::SYS_close
            | libc::SYS_fstat
            | libc::SYS_lseek
            | libc::SYS_munmap
            | libc::SYS_brk
            | libc::SYS_rt_sigaction
            | libc::SYS_rt_sigprocmask
            | libc::SYS_rt_sigreturn
            | libc::SYS_sched_yield
            | libc::SYS_mremap
            | libc::SYS_mincore
            | libc::SYS_madvise
            | libc::SYS_nanosleep
            | libc::SYS_getitimer
            | libc::SYS_getpid
            | libc::SYS_exit
            | libc::SYS_exit_group
            | libc::SYS_wait4
            | libc::SYS_uname
            | libc::SYS_getcwd
            | libc::SYS_getrlimit
            | libc::SYS_getrusage
            | libc::SYS_getuid
            | libc::SYS_getgid
            | libc::SYS_geteuid
            | libc::SYS_getegid
            | libc::SYS_getppid
            | libc::SYS_getgroups
            | libc::SYS_getresuid
            | libc::SYS_getresgid
            | libc::SYS_getsid
            | libc::SYS_capget
            | libc::SYS_rt_sigpending
            | libc::SYS_rt_sigtimedwait
            | libc::SYS_rt_sigsuspend
            | libc::SYS_sigaltstack
            | libc::SYS_fstatfs
            | libc::SYS_getpriority
            | libc::SYS_sched_getparam
            | libc::SYS_sched_getscheduler
            | libc::SYS_sched_get_priority_max
            | libc::SYS_sched_get_priority_min
            | libc::SYS_sched_rr_get_interval
            | libc::SYS_gettid
            | libc::SYS_futex
            | libc::SYS_sched_getaffinity
            | libc::SYS_set_tid_address
            | libc::SYS_restart_syscall
            | libc::SYS_clock_gettime
            | libc::SYS_clock_getres
            | libc::SYS_clock_nanosleep
            | libc::SYS_waitid
            | libc::SYS_ppoll
            | libc::SYS_pselect6
            | libc::SYS_set_robust_list
            | libc::SYS_getrandom
            | libc::SYS_membarrier
            | libc::SYS_rseq
    )
}
fn metadata_path(number: i64, args: &[u64; 6]) -> Option<(i32, u64)> {
    #[cfg(target_arch = "x86_64")]
    if matches!(
        number,
        libc::SYS_stat | libc::SYS_lstat | libc::SYS_access | libc::SYS_statfs
    ) {
        return Some((libc::AT_FDCWD, args[0]));
    }
    if matches!(
        number,
        libc::SYS_newfstatat | libc::SYS_statx | libc::SYS_faccessat | libc::SYS_faccessat2
    ) {
        return integer(args[0]).ok().map(|fd| (fd, args[1]));
    }
    None
}
struct TraceOwner {
    leader: i32,
    tasks: BTreeMap<i32, Tracee>,
    stdout: Option<ChildStdout>,
    eof: bool,
    leader_status: Option<i32>,
    forks: usize,
    _job: Arc<ReadJob>,
}
impl TraceOwner {
    fn start(mut child: Child, job: Arc<ReadJob>) -> Self {
        let leader = child.id() as i32;
        let stdout = child.stdout.take();
        let mut tasks = BTreeMap::new();
        let mut tracee = Tracee::unpinned(leader, true);
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, leader, libc::O_EXCL) };
        if fd >= 0 {
            tracee.pidfd = Some(unsafe { File::from_raw_fd(fd as i32) });
        }
        tasks.insert(leader, tracee);
        Self {
            leader,
            tasks,
            stdout,
            eof: false,
            leader_status: None,
            forks: 0,
            _job: job,
        }
    }
    fn initial(
        &mut self,
        cwd: &Directory,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<()> {
        loop {
            ensure_before(deadline)?;
            let mut status = 0;
            let result =
                unsafe { libc::waitpid(self.leader, &mut status, libc::__WALL | libc::WNOHANG) };
            if result == self.leader {
                if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                    self.tasks.remove(&self.leader);
                    self.leader_status = Some(status);
                    return denied();
                }
                if !libc::WIFSTOPPED(status) || libc::WSTOPSIG(status) != libc::SIGTRAP {
                    return denied();
                }
                break;
            }
            if result < 0 {
                if io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
                    if let Some(tracee) = self.tasks.get_mut(&self.leader) {
                        tracee.lost = true;
                    }
                }
                return denied();
            }
            std::thread::sleep(POLL);
        }
        let caller = crate::herdr::process_evidence(std::process::id()).map_err(filesystem)?;
        let tracee = self
            .tasks
            .get_mut(&self.leader)
            .ok_or(ErrorCode::NotReady)?;
        tracee.pin(deadline, Some(&caller))?;
        tracee.starting = false;
        let actual = tracee.object(c"exe")?;
        if object_identity(&actual, &actual.metadata().map_err(filesystem)?, deadline)
            .map_err(filesystem)?
            != runtime.executable.object
        {
            return denied();
        }
        let actual = tracee.object(c"cwd")?;
        if object_identity(&actual, &actual.metadata().map_err(filesystem)?, deadline)
            .map_err(filesystem)?
            != cwd.object
        {
            return denied();
        }
        let fd = unsafe {
            libc::openat(
                tracee.directory()?.as_raw_fd(),
                c"maps".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return denied();
        }
        let maps = unsafe { File::from_raw_fd(fd) };
        let mut bytes = Vec::new();
        maps.take((PROC_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(filesystem)?;
        if bytes.len() > PROC_BYTES {
            return Err(ErrorCode::ResourceLimit);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| ErrorCode::NotReady)?;
        let expected = runtime
            .interpreter
            .path
            .to_str()
            .ok_or(ErrorCode::NotReady)?;
        if !text.lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.nth(4).and_then(|inode| inode.parse::<u64>().ok())
                == Some(runtime.interpreter.object.inode)
                && fields.next() == Some(expected)
        }) {
            return denied();
        }
        options(self.leader)?;
        let stdout = self.stdout.as_ref().ok_or(ErrorCode::NotReady)?;
        let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return denied();
        }
        resume(self.leader, 0)
    }
    fn drain(&mut self, output: &mut NativeOutput, used: &mut usize) -> SecurityResult<()> {
        if self.eof {
            return Ok(());
        }
        let stdout = self.stdout.as_mut().ok_or(ErrorCode::NotReady)?;
        let mut buffer = [0u8; 8192];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(count) => {
                    *used = used
                        .checked_add(count)
                        .filter(|used| *used <= MAX_OUTPUT)
                        .ok_or(ErrorCode::ResourceLimit)?;
                    output.append(&buffer[..count])?;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(filesystem(error)),
            }
        }
    }
    fn entry(
        &mut self,
        pid: i32,
        call: Entry,
        graph: &mut admin::Graph,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<Option<Pending>> {
        if call.number & 0x40000000 != 0 {
            return denied();
        }
        let number = i64::try_from(call.number).map_err(|_| ErrorCode::NotReady)?;
        let args = call.args;
        let tracee = self.tasks.get(&pid).ok_or(ErrorCode::NotReady)?;
        let open = if number == libc::SYS_openat {
            Some((integer(args[0])?, args[1], args[2]))
        } else {
            #[cfg(target_arch = "x86_64")]
            {
                if number == libc::SYS_open {
                    Some((libc::AT_FDCWD, args[0], args[1]))
                } else {
                    None
                }
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                None
            }
        };
        if let Some((dir, pointer, flags)) = open {
            if !readable_flags(flags) {
                return denied();
            }
            let path = path(tracee, dir, pathname(pid, pointer)?)?;
            return Ok(Some(Pending::Open(opening(
                graph, runtime, &path, deadline,
            )?)));
        }
        if number == libc::SYS_openat2 {
            let size = usize::try_from(args[3]).map_err(|_| ErrorCode::NotReady)?;
            if !(24..=64).contains(&size) {
                return denied();
            }
            let mut bytes = [0u8; 64];
            memory(pid, args[2], &mut bytes[..size])?;
            let flags = u64::from_ne_bytes(bytes[..8].try_into().map_err(|_| ErrorCode::NotReady)?);
            if !readable_flags(flags)
                || bytes[8..16].iter().any(|byte| *byte != 0)
                || bytes[24..size].iter().any(|byte| *byte != 0)
            {
                return denied();
            }
            let path = path(tracee, integer(args[0])?, pathname(pid, args[1])?)?;
            return Ok(Some(Pending::Open(opening(
                graph, runtime, &path, deadline,
            )?)));
        }
        if let Some((dir, pointer)) = metadata_path(number, &args) {
            let mut first = [0u8; 1];
            memory(pid, pointer, &mut first)?;
            if first[0] == 0 {
                let flags = if number == libc::SYS_statx {
                    args[2]
                } else {
                    args[3]
                };
                if flags & libc::AT_EMPTY_PATH as u64 == 0 {
                    return denied();
                }
                inspect_fd(graph, runtime, tracee, dir, false, deadline)?;
                return Ok(None);
            }
            let path = path(tracee, dir, pathname(pid, pointer)?)?;
            let ancestor = graph
                .roots
                .iter()
                .any(|root| root.pathname().starts_with(&path))
                || runtime
                    .roots
                    .iter()
                    .any(|root| root.path.starts_with(&path));
            if !ancestor {
                let _ = opening(graph, runtime, &path, deadline)?;
            }
            return Ok(None);
        }
        if matches!(
            number,
            libc::SYS_read | libc::SYS_pread64 | libc::SYS_readv | libc::SYS_getdents64
        ) {
            let fd = integer(args[0])?;
            if fd > 2 {
                let name = CString::new(format!("fd/{fd}")).map_err(|_| ErrorCode::NotReady)?;
                let file = tracee.object(&name)?;
                if !file.metadata().map_err(filesystem)?.file_type().is_fifo() {
                    inspect_fd(graph, runtime, tracee, fd, false, deadline)?;
                }
            } else if fd != 0 {
                return denied();
            }
            return Ok(None);
        }
        if matches!(number, libc::SYS_write | libc::SYS_writev) {
            let fd = integer(args[0])?;
            if !matches!(fd, 1 | 2) {
                let name = CString::new(format!("fd/{fd}")).map_err(|_| ErrorCode::NotReady)?;
                let file = tracee.object(&name)?;
                if !file.metadata().map_err(filesystem)?.file_type().is_fifo() {
                    return denied();
                }
            }
            return Ok(None);
        }
        if number == libc::SYS_mmap {
            let protection = args[2] as i32;
            let flags = args[3] as i32;
            if !mapping_flags(protection, flags) {
                return denied();
            }
            if flags & libc::MAP_ANONYMOUS == 0 {
                inspect_fd(
                    graph,
                    runtime,
                    tracee,
                    integer(args[4])?,
                    protection & libc::PROT_EXEC != 0,
                    deadline,
                )?;
            }
            return Ok(None);
        }
        if number == libc::SYS_mprotect {
            if args[2] as i32 & !(libc::PROT_READ | libc::PROT_WRITE) != 0 {
                return denied();
            }
            return Ok(None);
        }
        if number == libc::SYS_chdir {
            let path = path(tracee, libc::AT_FDCWD, pathname(pid, args[0])?)?;
            let proof = opening(graph, runtime, &path, deadline)?;
            if !matches!(proof.scope, Scope::Project) {
                return denied();
            }
            let file = proof.file.ok_or(ErrorCode::NotReady)?;
            if !file.metadata().map_err(filesystem)?.is_dir() {
                return denied();
            }
            return Ok(Some(Pending::Directory(file)));
        }
        if number == libc::SYS_fchdir {
            let (_, file) = tracee.fd(integer(args[0])?)?;
            inspect_fd(graph, runtime, tracee, integer(args[0])?, false, deadline)?;
            return Ok(Some(Pending::Directory(file)));
        }
        if number == libc::SYS_clone {
            if !clone_flags(args[0]) {
                return denied();
            }
            return self.reserve_fork();
        }
        #[cfg(target_arch = "x86_64")]
        if matches!(number, libc::SYS_fork | libc::SYS_vfork) {
            return self.reserve_fork();
        }
        if number == libc::SYS_pipe2 {
            if args[1] & !((libc::O_CLOEXEC | libc::O_NONBLOCK) as u64) != 0 {
                return denied();
            }
            return Ok(Some(Pending::Pipe(args[0])));
        }
        #[cfg(target_arch = "x86_64")]
        if number == libc::SYS_pipe {
            return Ok(Some(Pending::Pipe(args[0])));
        }
        if number == libc::SYS_fcntl {
            let command = integer(args[1])?;
            if !matches!(
                command,
                libc::F_DUPFD
                    | libc::F_DUPFD_CLOEXEC
                    | libc::F_GETFD
                    | libc::F_SETFD
                    | libc::F_GETFL
                    | libc::F_SETFL
            ) || command == libc::F_SETFD && args[2] & !(libc::FD_CLOEXEC as u64) != 0
                || command == libc::F_SETFL
                    && args[2] & !((libc::O_NONBLOCK | libc::O_LARGEFILE) as u64) != 0
            {
                return denied();
            }
            return Ok(None);
        }
        if matches!(number, libc::SYS_dup | libc::SYS_dup3) {
            return Ok(None);
        }
        #[cfg(target_arch = "x86_64")]
        if number == libc::SYS_dup2 {
            return Ok(None);
        }
        if number == libc::SYS_ioctl {
            if !matches!(integer(args[0])?, 0..=2) || !matches!(args[1], 0x5401 | 0x5413 | 0x541b) {
                return denied();
            }
            return Ok(None);
        }
        if number == libc::SYS_prlimit64 {
            if args[2] != 0 || args[0] != 0 && !self.tasks.contains_key(&integer(args[0])?) {
                return denied();
            }
            return Ok(None);
        }
        if number == libc::SYS_prctl {
            if !matches!(args[0], 3 | 15 | 16 | 21 | 39) {
                return denied();
            }
            return Ok(None);
        }
        #[cfg(target_arch = "x86_64")]
        if number == libc::SYS_arch_prctl {
            if !matches!(args[0], 0x1001..=0x1004 | 0x1021) {
                return denied();
            }
            return Ok(None);
        }
        if matches!(number, libc::SYS_kill | libc::SYS_tkill | libc::SYS_tgkill) {
            let target = if number == libc::SYS_tgkill {
                integer(args[1])?
            } else {
                integer(args[0])?
            };
            if !self.tasks.contains_key(&target)
                || number == libc::SYS_tgkill && !self.tasks.contains_key(&integer(args[0])?)
            {
                return denied();
            }
            return Ok(None);
        }
        if number == libc::SYS_readlinkat {
            let value = path(tracee, integer(args[0])?, pathname(pid, args[1])?)?;
            if value != Path::new("/proc/self/exe")
                && value != PathBuf::from(format!("/proc/{pid}/exe"))
            {
                return denied();
            }
            return Ok(None);
        }
        #[cfg(target_arch = "x86_64")]
        if number == libc::SYS_readlink {
            let value = path(tracee, libc::AT_FDCWD, pathname(pid, args[0])?)?;
            if value != Path::new("/proc/self/exe")
                && value != PathBuf::from(format!("/proc/{pid}/exe"))
            {
                return denied();
            }
            return Ok(None);
        }
        if simple(number) {
            return Ok(None);
        }
        denied()
    }
    fn reserve_fork(&mut self) -> SecurityResult<Option<Pending>> {
        if self.tasks.len() + self.forks >= MAX_TRACEES {
            return Err(ErrorCode::ResourceLimit);
        }
        self.forks += 1;
        Ok(Some(Pending::Fork(false)))
    }
    fn returned(
        &mut self,
        pid: i32,
        result: Exit,
        graph: &mut admin::Graph,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<()> {
        let pending = self
            .tasks
            .get_mut(&pid)
            .ok_or(ErrorCode::NotReady)?
            .pending
            .take();
        if result.error != 0 {
            let expected = matches!(
                &pending,
                Some(Pending::Open(OpenProof { file: Some(_), .. })) | Some(Pending::Directory(_))
            );
            if let Some(Pending::Fork(false)) = pending {
                self.forks = self.forks.saturating_sub(1);
            }
            let error = i32::try_from(result.result.checked_neg().ok_or(ErrorCode::NotReady)?)
                .map_err(|_| ErrorCode::NotReady)?;
            return match error {
                libc::ENOMEM | libc::EMFILE | libc::ENFILE | libc::EFBIG => {
                    Err(ErrorCode::ResourceLimit)
                }
                libc::EACCES | libc::EPERM | libc::EXDEV | libc::ELOOP | libc::EIO => denied(),
                _ if expected => denied(),
                _ => Ok(()),
            };
        }
        let tracee = self.tasks.get(&pid).ok_or(ErrorCode::NotReady)?;
        match pending {
            Some(Pending::Open(proof)) => {
                let (path, file) =
                    tracee.fd(i32::try_from(result.result).map_err(|_| ErrorCode::NotReady)?)?;
                same(
                    proof.file.as_ref().ok_or(ErrorCode::NotReady)?,
                    &file,
                    deadline,
                )?;
                match proof.scope {
                    Scope::Project => graph.observe_fd(&path, &file, deadline)?,
                    Scope::Runtime => {
                        same(&runtime.resolve(&path, deadline)?, &file, deadline)?;
                    }
                }
            }
            Some(Pending::Directory(expected)) => {
                same(&expected, &tracee.object(c"cwd")?, deadline)?
            }
            Some(Pending::Pipe(address)) => {
                let mut bytes = [0u8; 8];
                memory(pid, address, &mut bytes)?;
                for chunk in bytes.chunks_exact(4) {
                    let fd = i32::from_ne_bytes(chunk.try_into().map_err(|_| ErrorCode::NotReady)?);
                    let name = CString::new(format!("fd/{fd}")).map_err(|_| ErrorCode::NotReady)?;
                    if !tracee
                        .object(&name)?
                        .metadata()
                        .map_err(filesystem)?
                        .file_type()
                        .is_fifo()
                    {
                        return denied();
                    }
                }
            }
            Some(Pending::Fork(false)) => {
                self.forks = self.forks.saturating_sub(1);
                return denied();
            }
            Some(Pending::Fork(true)) | None => {}
        }
        Ok(())
    }
    fn stopped(
        &mut self,
        pid: i32,
        status: i32,
        graph: &mut admin::Graph,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<()> {
        let event = status >> 16;
        let signal = libc::WSTOPSIG(status);
        if matches!(
            event,
            libc::PTRACE_EVENT_FORK | libc::PTRACE_EVENT_VFORK | libc::PTRACE_EVENT_CLONE
        ) {
            let child = event_pid(pid)?;
            if self.tasks.contains_key(&child) {
                return denied();
            }
            let parent = self.tasks.get_mut(&pid).ok_or(ErrorCode::NotReady)?;
            if !matches!(parent.pending, Some(Pending::Fork(false))) {
                return denied();
            }
            parent.pending = Some(Pending::Fork(true));
            self.forks = self.forks.checked_sub(1).ok_or(ErrorCode::NotReady)?;
            let proof = parent.proof.clone().ok_or(ErrorCode::NotReady)?;
            self.tasks.insert(child, Tracee::unpinned(child, true));
            self.tasks
                .get_mut(&child)
                .ok_or(ErrorCode::NotReady)?
                .pin(deadline, Some(&proof))?;
            if self.tasks.len() > MAX_TRACEES {
                return Err(ErrorCode::ResourceLimit);
            }
            return resume(pid, 0);
        }
        if event == libc::PTRACE_EVENT_EXEC {
            return denied();
        }
        if event == libc::PTRACE_EVENT_EXIT {
            return resume(pid, 0);
        }
        let tracee = self.tasks.get_mut(&pid).ok_or(ErrorCode::NotReady)?;
        if tracee.starting {
            if !matches!(signal, libc::SIGSTOP | libc::SIGTRAP) {
                return denied();
            }
            tracee.starting = false;
            options(pid)?;
            return resume(pid, 0);
        }
        if signal == (libc::SIGTRAP | 0x80) {
            let info = info(pid)?;
            if info.operation == 1 {
                tracee.bootstrap_exit = false;
                if tracee.in_call {
                    return denied();
                }
                let pending =
                    self.entry(pid, unsafe { info.data.entry }, graph, runtime, deadline)?;
                let tracee = self.tasks.get_mut(&pid).ok_or(ErrorCode::NotReady)?;
                tracee.pending = pending;
                tracee.in_call = true;
            } else {
                let result = unsafe { info.data.exit };
                if tracee.bootstrap_exit && !tracee.in_call {
                    tracee.bootstrap_exit = false;
                    if result.result != 0 || result.error != 0 {
                        return denied();
                    }
                } else {
                    if !tracee.in_call {
                        return denied();
                    }
                    tracee.in_call = false;
                    self.returned(pid, result, graph, runtime, deadline)?;
                }
            }
            return resume(pid, 0);
        }
        if signal == libc::SIGTRAP || signal == libc::SIGSTOP {
            return denied();
        }
        resume(pid, signal)
    }
    fn run(
        &mut self,
        graph: &mut admin::Graph,
        runtime: &runtime::Runtime,
        deadline: Instant,
        used: &mut usize,
    ) -> SecurityResult<(i32, NativeOutput)> {
        let mut output = NativeOutput(Vec::new());
        while !self.tasks.is_empty() || !self.eof {
            self._job.current()?;
            ensure_before(deadline)?;
            self.drain(&mut output, used)?;
            let mut pids = [0i32; MAX_TRACEES + 1];
            let mut count = 0;
            for pid in self.tasks.keys() {
                if count == pids.len() {
                    return Err(ErrorCode::ResourceLimit);
                }
                pids[count] = *pid;
                count += 1;
            }
            let mut progress = false;
            for pid in &pids[..count] {
                let mut status = 0;
                let waited =
                    unsafe { libc::waitpid(*pid, &mut status, libc::__WALL | libc::WNOHANG) };
                if waited == 0 {
                    continue;
                }
                if waited < 0 {
                    if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    return denied();
                }
                progress = true;
                if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                    self.tasks.remove(pid);
                    if *pid == self.leader {
                        self.leader_status = Some(status);
                    } else if !libc::WIFEXITED(status) || libc::WEXITSTATUS(status) != 0 {
                        return denied();
                    }
                } else if libc::WIFSTOPPED(status) {
                    self.stopped(*pid, status, graph, runtime, deadline)?;
                } else {
                    return denied();
                }
            }
            if !progress {
                std::thread::sleep(POLL);
            }
        }
        let status = self.leader_status.ok_or(ErrorCode::NotReady)?;
        if !libc::WIFEXITED(status) {
            return denied();
        }
        Ok((libc::WEXITSTATUS(status), output))
    }
    fn cleanup(&mut self) {
        while !self.tasks.is_empty() {
            let mut pids = [0i32; MAX_TRACEES + 1];
            let mut count = 0;
            for (pid, tracee) in &self.tasks {
                tracee.kill();
                if count < pids.len() {
                    pids[count] = *pid;
                    count += 1;
                }
            }
            let mut progress = false;
            for pid in &pids[..count] {
                let mut status = 0;
                let result =
                    unsafe { libc::waitpid(*pid, &mut status, libc::__WALL | libc::WNOHANG) };
                if result == *pid {
                    progress = true;
                    if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                        self.tasks.remove(pid);
                        continue;
                    }
                    if libc::WIFSTOPPED(status) {
                        if matches!(
                            status >> 16,
                            libc::PTRACE_EVENT_FORK
                                | libc::PTRACE_EVENT_VFORK
                                | libc::PTRACE_EVENT_CLONE
                        ) {
                            if let Ok(child) = event_pid(*pid) {
                                if !self.tasks.contains_key(&child) {
                                    let mut tracee = Tracee::unpinned(child, true);
                                    let fd = unsafe {
                                        libc::syscall(libc::SYS_pidfd_open, child, libc::O_EXCL)
                                    };
                                    if fd >= 0 {
                                        tracee.pidfd =
                                            Some(unsafe { File::from_raw_fd(fd as i32) });
                                    }
                                    self.tasks.insert(child, tracee);
                                }
                            }
                        }
                        let _ = trace(
                            *pid,
                            libc::PTRACE_CONT,
                            std::ptr::null_mut(),
                            libc::SIGKILL as usize as *mut libc::c_void,
                        );
                    }
                } else if result < 0
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
                {
                    if let Some(tracee) = self.tasks.get_mut(pid) {
                        tracee.lost = true;
                        if tracee.gone() {
                            self.tasks.remove(pid);
                            progress = true;
                        }
                    }
                }
            }
            // A lost wait is not a fabricated reap. This owner and its quota survive
            // until the pinned original pidfd proves exit, even past the caller deadline.
            if !progress {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
impl Drop for TraceOwner {
    fn drop(&mut self) {
        self.cleanup();
    }
}
pub(super) fn run(
    observation: &mut Observation,
    cwd: &Directory,
    working: &Path,
    operation: &[&str],
) -> SecurityResult<(i32, NativeOutput)> {
    let Observation {
        graph,
        runtime,
        job,
        deadline,
        used,
        ..
    } = observation;
    let runtime = *runtime;
    let deadline = *deadline;
    let job = job.clone();
    ensure_before(deadline)?;
    job.current()?;
    graph.recheck(deadline)?;
    runtime.current(deadline)?;
    let rules = Landlock::prepare(graph, runtime, deadline)?;
    let mut command = Command::new(&runtime.executable.path);
    command.args([
        "--no-pager",
        "--no-replace-objects",
        "--literal-pathspecs",
        "-c",
        "color.ui=false",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.untrackedCache=false",
        "-c",
        "core.preloadIndex=false",
        "-c",
        "index.threads=1",
        "-c",
        "gc.auto=0",
        "-c",
        "maintenance.auto=false",
        "-c",
        "core.packedGitWindowSize=16m",
        "-c",
        "core.packedGitLimit=32m",
    ]);
    command
        .arg("-C")
        .arg(working)
        .args(operation)
        .env_clear()
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EXEC_PATH", &runtime.exec_path)
        .env("MALLOC_ARENA_MAX", "1");
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_ATTR_NOSYSTEM",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let directory = cwd.file.as_raw_fd();
    unsafe {
        command.pre_exec(move || rules.restrict(directory));
    }
    let child = command.spawn().map_err(filesystem)?;
    let mut owner = TraceOwner::start(child, job);
    owner.initial(cwd, runtime, deadline)?;
    let result = owner.run(graph, runtime, deadline, used)?;
    graph.recheck(deadline)?;
    runtime.current(deadline)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_only_open_and_closed_execution_policy_have_no_mutation_escape() {
        assert!(readable_flags(
            (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64
        ));
        for flags in [
            libc::O_WRONLY,
            libc::O_RDWR,
            libc::O_CREAT,
            libc::O_TRUNC,
            libc::O_APPEND,
            libc::O_TMPFILE,
        ] {
            assert!(!readable_flags(flags as u64));
        }
        assert!(!simple(libc::SYS_execve));
        assert!(!simple(libc::SYS_execveat));
        assert!(!simple(libc::SYS_socket));
        assert!(!simple(libc::SYS_connect));
        assert!(!simple(libc::SYS_unlinkat));
        assert!(!simple(i64::MAX));
        assert!(clone_flags(libc::SIGCHLD as u64));
        for flags in [
            0x8000, 0x20000, 0x800000, 0x2000000, 0x4000000, 0x8000000, 0x10000000, 0x20000000,
            0x40000000, 0x80000000,
        ] {
            assert!(!clone_flags(flags | libc::SIGCHLD as u64));
        }
        assert!(mapping_flags(
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE
        ));
        assert!(!mapping_flags(
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED
        ));
        assert!(!mapping_flags(
            libc::PROT_READ | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS
        ));
        assert!(mapping_flags(
            libc::PROT_READ | libc::PROT_EXEC,
            libc::MAP_PRIVATE
        ));
    }
    #[test]
    fn actual_child_kernel_rules_refuse_outside_reads_and_project_writes_without_changing_parent() {
        use std::os::unix::fs::DirBuilderExt;
        if !supported() {
            assert_eq!(Landlock::new().err(), Some(ErrorCode::NotReady));
            return;
        }
        let parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let directory = parent.join(format!("goatr-landlock-{}", Uuid::random().0));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let approved = directory.join("approved");
        std::fs::create_dir(&approved).unwrap();
        let inside = approved.join("inside");
        let outside = directory.join("outside");
        std::fs::write(&inside, b"x").unwrap();
        std::fs::write(&outside, b"private fixture").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let root = Root::open(&approved, deadline).unwrap();
        let rules = Landlock::new().unwrap();
        rules.rule(&root.file, (1 << 2) | (1 << 3)).unwrap();
        let outside = CString::new(outside.as_os_str().as_bytes()).unwrap();
        // The forked branch performs only direct async-signal-safe syscalls and
        // the exact pre-exec restriction; no allocator, formatter or shared lock.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe {
                if rules.restrict(root.file.as_raw_fd()).is_err() {
                    libc::_exit(10);
                }
                let read = libc::openat(
                    libc::AT_FDCWD,
                    c"inside".as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC,
                );
                if read < 0 {
                    libc::_exit(11);
                }
                let mut byte = 0u8;
                if libc::read(read, (&mut byte as *mut u8).cast(), 1) != 1 || byte != b'x' {
                    libc::_exit(12);
                }
                libc::close(read);
                let denied_read = libc::openat(
                    libc::AT_FDCWD,
                    outside.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC,
                );
                if denied_read >= 0 {
                    libc::close(denied_read);
                    libc::_exit(13);
                }
                if *libc::__errno_location() != libc::EACCES {
                    libc::_exit(14);
                }
                let write = libc::openat(
                    libc::AT_FDCWD,
                    c"inside".as_ptr(),
                    libc::O_WRONLY | libc::O_CLOEXEC,
                );
                if write >= 0 {
                    libc::close(write);
                    libc::_exit(15);
                }
                if *libc::__errno_location() != libc::EACCES {
                    libc::_exit(16);
                }
                libc::_exit(0);
            }
        }
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        assert!(libc::WIFEXITED(status));
        assert_eq!(libc::WEXITSTATUS(status), 0);
        assert_eq!(std::fs::read(&inside).unwrap(), b"x");
        assert_eq!(
            std::fs::read(directory.join("outside")).unwrap(),
            b"private fixture"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
