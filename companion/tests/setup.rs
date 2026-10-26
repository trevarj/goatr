// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{
    config::*,
    protocol::Uuid,
    setup::{ExpectedFile, SetupLock},
};
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::FromRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let p = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".goatr-setup-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&p).unwrap();
        Self(p)
    }
    fn config(&self) -> Config {
        let wrapper = self.0.join("herdr");
        fs::write(&wrapper, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        Config::parse(&format!(
            "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\n[executables]\nherdr={:?}\n",
            self.0, self.0, wrapper
        ))
        .unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn expected_source_atomic_publish_noop_change_and_lock_boundaries() {
    let r = Root::new();
    let p = r.0.join("config.toml");
    let absent = ExpectedFile::capture(&p).unwrap();
    let l = SetupLock::acquire(&r.0).unwrap();
    assert!(SetupLock::acquire(&r.0).is_err());
    let bytes = r.config().serialized().unwrap();
    assert!(l.publish(&p, &absent, bytes.as_bytes()).unwrap());
    assert!(l.publish(&p, &absent, bytes.as_bytes()).is_err());
    let source = ExpectedFile::capture(&p).unwrap();
    let inode = fs::metadata(&p).unwrap().ino();
    assert!(!l.publish(&p, &source, bytes.as_bytes()).unwrap());
    assert_eq!(inode, fs::metadata(&p).unwrap().ino());
    let foreign = bytes.replace("version = 1", "version = 2");
    fs::write(&p, &foreign).unwrap();
    assert!(l.publish(&p, &source, bytes.as_bytes()).is_err());
    assert_eq!(fs::read_to_string(&p).unwrap(), foreign);
    fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink(r.0.join("herdr"), &p).unwrap();
    assert!(ExpectedFile::capture(&p).is_err());
}
#[test]
fn project_approval_serialization_dedup_preserves_owner_and_id() {
    let r = Root::new();
    let mut c = r.config();
    assert!(c.approve_project(r.0.to_str().unwrap()).unwrap());
    let id = c.projects[0].id.clone();
    let label = c.projects[0].label.clone();
    let alias = r.0.join("alias");
    std::os::unix::fs::symlink(&r.0, &alias).unwrap();
    assert!(!c.approve_project(alias.to_str().unwrap()).unwrap());
    assert_eq!(c.projects[0].id, id);
    assert_eq!(c.projects[0].label, label);
    let back = Config::parse(&c.serialized().unwrap()).unwrap();
    assert_eq!(back.projects[0].id, id);
    assert!(!back.allow_scoped_shell_probe);
    assert!(back.default_codex_owner_id.is_none());
}
fn setup_terminal(
    config: &Path,
    root: &Root,
    answer: Option<&str>,
    cancel: bool,
) -> std::process::ExitStatus {
    // SAFETY: POSIX allocates a private PTY; all effects target exclusively owned test paths.
    let fd = unsafe {
        libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC | libc::O_NONBLOCK)
    };
    assert!(fd >= 0);
    let mut master = unsafe { fs::File::from_raw_fd(fd) };
    assert_eq!(unsafe { libc::grantpt(fd) }, 0);
    assert_eq!(unsafe { libc::unlockpt(fd) }, 0);
    let mut name = [0i8; 256];
    assert_eq!(
        unsafe { libc::ptsname_r(fd, name.as_mut_ptr(), name.len()) },
        0
    );
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    let slave = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(Path::new(std::str::from_utf8(name.to_bytes()).unwrap()))
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_goatr"))
        .args(["setup", "--config"])
        .arg(config)
        .arg("--project")
        .arg(&root.0)
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut all = Vec::new();
    let mut buf = [0; 4096];
    loop {
        match master.read(&mut buf) {
            Ok(n) => all.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("{e}"),
        };
        if all.windows(9).any(|w| w == b"[yes/no] ") {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    if cancel {
        assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    } else {
        master.write_all(answer.unwrap().as_bytes()).unwrap();
    }
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn actual_cli_decline_cancel_and_nonterminal_have_zero_publication_effects() {
    let r = Root::new();
    let p = r.0.join("config.toml");
    let original = r.config().serialized().unwrap();
    fs::write(&p, &original).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::metadata(&p).unwrap().ino();
    assert!(setup_terminal(&p, &r, Some("no\n"), false).success());
    assert!(!setup_terminal(&p, &r, None, true).success());
    assert_eq!(fs::read_to_string(&p).unwrap(), original);
    assert_eq!(fs::metadata(&p).unwrap().ino(), before);
    assert!(!r.0.join(".setup.lock").exists());
    assert!(!r.0.join(".serve-unit.json").exists());
    let absent = r.0.join("absent/config.toml");
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_goatr"))
            .arg("setup")
            .arg("--config")
            .arg(&absent)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    assert!(!r.0.join("absent").exists());
}

#[test]
fn actual_setup_project_consent_preserves_explicit_deny_all_and_exact_selfhost_origins() {
    for policy in [Vec::<String>::new(), vec!["https://push.example:8443".into()]] {
        let root = Root::new();
        let path = root.0.join("config.toml");
        let mut configured = root.config();
        configured.push_allowed_origins = policy.clone();
        fs::write(&path, configured.serialized().unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(setup_terminal(&path, &root, Some("yes\n"), false).success());
        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.push_allowed_origins, policy);
        assert_eq!(saved.runtime_dir, configured.runtime_dir);
        assert_eq!(saved.runtime_dir_agent, configured.runtime_dir_agent);
        assert_eq!(saved.projects.len(), 1);
        assert_eq!(saved.projects[0].path, root.0);
        assert!(!saved.projects[0].allow_read_tools);
        assert!(!saved.allow_attachments && !saved.allow_terminal_control && !saved.allow_omp_controls);
        assert!(!root.0.join(".serve-unit.json").exists());
    }
}

#[tokio::test]
async fn owned_bound_listener_requires_exact_source_fd_socket_and_never_rebinds_or_closes_original()
{
    use std::os::fd::AsRawFd;
    let root = Root::new();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let fd = listener.as_raw_fd();
    let metadata = fs::metadata(format!("/proc/self/fd/{fd}")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut record = goatr::backends::omp::OwnedRelayListener {
        v: 1,
        fd,
        device: metadata.dev(),
        inode: metadata.ino(),
        port,
        parent: goatr::herdr::process_evidence(std::process::id()).unwrap(),
        runtime_dir: root.0.clone(),
    };
    let path = root.0.join("omp-relay.owner");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let relay = goatr::backends::omp::Relay::inherited(fd, &path, &root.0).unwrap();
    assert_eq!(relay.port(), port);
    drop(relay);
    assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_err());
    assert_eq!(listener.local_addr().unwrap().port(), port);
    record.inode += 1;
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(goatr::backends::omp::Relay::inherited(fd, &path, &root.0).is_err());
    assert_eq!(listener.local_addr().unwrap().port(), port);
}
