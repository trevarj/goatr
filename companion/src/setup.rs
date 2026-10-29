// SPDX-License-Identifier: GPL-3.0-or-later
//! Concrete local setup; no live identity, receipt database or provider ownership.
use crate::{
    admin,
    config::{self, CONFIG_BYTES, CodexOwner, Config, Executables},
    pairing::{private_dir, private_file, uid},
    protocol::{Provider, Uuid},
};
use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, IsTerminal, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    time::Duration,
};
fn refused(message: &str) -> io::Error {
    io::Error::other(message)
}
fn stamp(m: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
/// Captured BEFORE consent. Retaining the FD prevents inode reuse from looking like ownership.
pub enum ExpectedFile {
    Absent,
    Owned {
        file: File,
        bytes: Vec<u8>,
        identity: (u64, u64, u64, i64, i64, i64, i64),
    },
}
impl ExpectedFile {
    pub fn capture(path: &Path) -> io::Result<Self> {
        config::absolute(path)?;
        trusted_parent(
            path.parent()
                .ok_or_else(|| refused("private source parent missing"))?,
        )?;
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::Absent),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        private_file(path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let before = stamp(&file.metadata()?);
        let mut bytes = Vec::new();
        (&mut file)
            .take((CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > CONFIG_BYTES
            || before != stamp(&fs::symlink_metadata(path)?)
            || before != stamp(&file.metadata()?)
        {
            return Err(refused("private source changed while reading"));
        }
        Ok(Self::Owned {
            file,
            bytes,
            identity: before,
        })
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Absent => None,
            Self::Owned { bytes, .. } => Some(bytes),
        }
    }
    pub fn recheck(&self, path: &Path) -> io::Result<()> {
        let now = Self::capture(path)?;
        match (self, now) {
            (Self::Absent, Self::Absent) => Ok(()),
            (
                Self::Owned {
                    file,
                    bytes,
                    identity,
                },
                Self::Owned {
                    bytes: new,
                    identity: current,
                    ..
                },
            ) if *identity == current && *identity == stamp(&file.metadata()?) && *bytes == new => {
                Ok(())
            }
            _ => Err(refused(
                "intervening private source change; reopen setup, no replacement performed",
            )),
        }
    }
}
fn trusted_parent(path: &Path) -> io::Result<()> {
    config::absolute(path)?;
    for entry in path.ancestors() {
        match fs::symlink_metadata(entry) {
            Ok(m)
                if m.is_dir()
                    && (m.uid() == uid() || m.uid() == 0)
                    && m.mode() & 0o022 == 0
                    && fs::canonicalize(entry)? == entry => {}
            Ok(_) => {
                return Err(refused(
                    "foreign, managed, symlinked or writable setup parent",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn create_private(path: &Path) -> io::Result<()> {
    trusted_parent(path)?;
    if !path.exists() {
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            create_private(parent)?;
        }
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    private_dir(path, false)
}
pub struct SetupLock {
    directory: File,
    path: PathBuf,
    _file: File,
    private_parent: bool,
}
impl SetupLock {
    pub fn acquire(parent: &Path) -> io::Result<Self> {
        create_private(parent)?;
        Self::open(parent, true)
    }
    fn unit_parent(parent: &Path) -> io::Result<Self> {
        trusted_parent(parent)?;
        if !parent.exists() {
            create_private(parent)?;
        }
        let m = fs::symlink_metadata(parent)?;
        if !m.is_dir()
            || m.uid() != uid()
            || m.mode() & 0o022 != 0
            || fs::canonicalize(parent)? != parent
        {
            return Err(refused(
                "unsafe user-unit directory; no adoption or permission rewrite",
            ));
        }
        Self::open(parent, false)
    }
    fn open(parent: &Path, private_parent: bool) -> io::Result<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)?;
        let path = parent.join(".setup.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        private_file(&path)?;
        if stamp(&file.metadata()?) != stamp(&fs::symlink_metadata(&path)?) {
            return Err(refused("setup lock replaced"));
        }
        // SAFETY: a live owned descriptor, nonblocking exclusive serialization only.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(refused(
                "setup busy; another invocation owns the private serialization lock",
            ));
        }
        Ok(Self {
            directory,
            path: parent.to_owned(),
            _file: file,
            private_parent,
        })
    }
    fn recheck(&self) -> io::Result<()> {
        if self.private_parent {
            private_dir(&self.path, false)?;
        } else {
            trusted_parent(&self.path)?;
        }
        let now = fs::symlink_metadata(&self.path)?;
        let old = self.directory.metadata()?;
        if now.uid() != uid() || (old.dev(), old.ino()) != (now.dev(), now.ino()) {
            return Err(refused("setup directory replaced"));
        }
        let lock = self.path.join(".setup.lock");
        private_file(&lock)?;
        if stamp(&self._file.metadata()?) != stamp(&fs::symlink_metadata(lock)?) {
            return Err(refused("setup lock replaced"));
        }
        Ok(())
    }
    pub fn publish(&self, path: &Path, expected: &ExpectedFile, bytes: &[u8]) -> io::Result<bool> {
        if path.parent() != Some(self.path.as_path()) || bytes.len() > CONFIG_BYTES {
            return Err(refused("publication outside held private setup directory"));
        }
        self.recheck()?;
        expected.recheck(path)?;
        if expected.bytes() == Some(bytes) {
            return Ok(false);
        }
        let name = format!(".setup-{}", Uuid::random().0);
        let temporary = self.path.join(&name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        let original = stamp(&file.metadata()?);
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            self.recheck()?;
            expected.recheck(path)?;
            let src = CString::new(name).map_err(io::Error::other)?;
            let dst = CString::new(
                path.file_name()
                    .ok_or_else(|| refused("config filename missing"))?
                    .as_bytes(),
            )
            .map_err(io::Error::other)?;
            // The flock serializes Goatr writers; noncooperating malicious same-UID writers
            // are outside the host trust model. Recheck exact inode AND bytes at cutover.
            let flags = if matches!(expected, ExpectedFile::Absent) {
                libc::RENAME_NOREPLACE
            } else {
                0
            };
            // SAFETY: retained directory FD and NUL-free relative basenames.
            if unsafe {
                libc::renameat2(
                    self.directory.as_raw_fd(),
                    src.as_ptr(),
                    self.directory.as_raw_fd(),
                    dst.as_ptr(),
                    flags,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            self.directory.sync_all()?;
            Ok(true)
        })();
        if temporary.exists() {
            let m = fs::symlink_metadata(&temporary)?;
            if (m.dev(), m.ino()) == (original.0, original.1) {
                fs::remove_file(&temporary)?;
                self.directory.sync_all()?;
            }
        }
        result
    }
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
async fn confirm(question: &str) -> io::Result<bool> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(refused(
            "setup requires a local interactive terminal for explicit consent",
        ));
    }
    print!("{question} [yes/no] ");
    io::stdout().flush()?;
    loop {
        let mut fd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one initialized pollfd, zero timeout; no terminal flags/settings changed.
        let n = unsafe { libc::poll(&mut fd, 1, 0) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n > 0 {
            if fd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                return Err(refused("setup terminal closed; cancelled"));
            }
            let mut line = String::new();
            io::stdin().lock().take(256).read_line(&mut line)?;
            return Ok(line.trim() == "yes");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
fn choose_path(sub: &ArgMatches, key: &str) -> Option<PathBuf> {
    sub.get_one::<PathBuf>(key).cloned()
}
fn new_config(sub: &ArgMatches) -> io::Result<Config> {
    let runtime=choose_path(sub,"runtime-dir").ok_or_else(||refused("absent config: supply --runtime-dir with an existing private wrapper-accessible directory"))?;
    let herdr = choose_path(sub, "herdr").ok_or_else(|| {
        refused("absent config: supply --herdr with the installed absolute outer executable")
    })?;
    let mut c = Config {
        version: 1,
        pairing_name: None,
        runtime_dir_agent: choose_path(sub, "runtime-dir-agent").unwrap_or_else(|| runtime.clone()),
        runtime_dir: runtime,
        allow_scoped_shell_probe: false,
            allow_attachments: false,
        allow_terminal_control: false,
        allow_omp_controls:false,
        skill_document_roots:Vec::new(),
        herdr_sockets: vec![],
        push_allowed_origins: config::default_push_allowed_origins(),
        omp_registry_dir: choose_path(sub, "omp-registry-dir"),
        omp_session_roots: sub
            .get_many::<PathBuf>("omp-session-root")
            .map(|p| p.cloned().collect())
            .unwrap_or_default(),
        omp_blob_root: choose_path(sub, "omp-blob-root"),
        omp_skills_package_root:None,
        executables: Executables {
            herdr,
            omp: choose_path(sub, "omp"),
            codex: choose_path(sub, "codex"),
        },
        projects: vec![],
        codex_owners: vec![],
        default_codex_owner_id: None,
    };
    if let Some(socket) = choose_path(sub, "codex-owner-socket") {
        let owner = CodexOwner {
            id: Uuid::random(),
            socket_agent: choose_path(sub, "codex-owner-socket-agent")
                .unwrap_or_else(|| socket.clone()),
            socket_host: socket,
            bindings_dir: choose_path(sub, "codex-bindings-dir")
                .ok_or_else(|| refused("--codex-owner-socket requires --codex-bindings-dir"))?,
            unit: sub
                .get_one::<String>("codex-unit")
                .cloned()
                .unwrap_or_else(|| "goatr-codex.service".into()),
        };
        c.default_codex_owner_id = Some(owner.id.clone());
        c.codex_owners.push(owner);
    }
    Ok(c)
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct UnitOwnership {
    version: u32,
    unit: PathBuf,
    executable: PathBuf,
    config: PathBuf,
    cwd: PathBuf,
    content: String,
}
#[derive(Debug, PartialEq, Eq)]
struct UnitPair { record_version: u32, unit_new: bool }
fn validate_unit_ownership(
    planned: &UnitOwnership,
    legacy: Option<&UnitOwnership>,
    unit: &ExpectedFile,
    record: &ExpectedFile,
) -> io::Result<UnitPair> {
    let Some(bytes) = record.bytes() else {
        if unit.bytes().is_some() { return Err(refused("present identical unit without private ownership is foreign")); }
        return Ok(UnitPair { record_version: 0, unit_new: false });
    };
    let old: UnitOwnership = serde_json::from_slice(bytes).map_err(|_| refused("invalid owned serve-unit record"))?;
    if old == *planned {
        if unit.bytes().is_some_and(|bytes| bytes != planned.content.as_bytes()) {
            return Err(refused("owned record/unit bytes differ; no replacement"));
        }
        return Ok(UnitPair { record_version: planned.version, unit_new: unit.bytes().is_some() });
    }
    if legacy.is_some_and(|legacy| old == *legacy) && planned.version == 2 {
        let bytes = unit.bytes().ok_or_else(|| refused("recognized migration requires the original owned unit"))?;
        if bytes == planned.content.as_bytes() || legacy.is_some_and(|legacy| bytes == legacy.content.as_bytes()) {
            return Ok(UnitPair { record_version: 1, unit_new: bytes == planned.content.as_bytes() });
        }
    }
    Err(refused("foreign/declarative/executable/config/cwd/version unit; no adoption or downgrade"))
}
fn unit_word(path: &Path, exec: bool) -> io::Result<String> {
    config::absolute(path)?;
    let value = path
        .to_str()
        .ok_or_else(|| refused("unit path is not UTF-8"))?
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    Ok(format!(
        "\"{}\"",
        if exec {
            value.replace('$', "$$")
        } else {
            value
        }
    ))
}
fn unit_environment(name: &str, path: &Path) -> io::Result<String> {
    let word = unit_word(path, false)?;
    Ok(format!(
        "Environment=\"{name}={}\"\n",
        &word[1..word.len() - 1]
    ))
}
fn unit_home() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config").into_os_string()
    }))
}
fn unit_environment_current() -> io::Result<String> {
    let user_home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| refused("HOME unavailable for durable serve unit"))?);
    let state_root = crate::pairing::state_root()?;
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| refused("unchanged XDG_RUNTIME_DIR required for serve unit"))?);
    private_dir(&runtime, false)?;
    Ok(unit_environment("HOME", &user_home)? + &unit_environment("XDG_CONFIG_HOME", &unit_home())?
        + &unit_environment("XDG_STATE_HOME", state_root.parent().ok_or_else(|| refused("state base unavailable"))?)?
        + &unit_environment("XDG_RUNTIME_DIR", &runtime)?)
}
fn unit_content(binary: &Path, config: &Path, cwd: &Path, environment: &str, version: u32) -> io::Result<String> {
    let (title, authority, context) = match version {
        1 => ("basic setup v1", "", ""),
        2 => ("unit Wake setup v2", "PrivateUsers=no\nAmbientCapabilities=CAP_WAKE_ALARM\nNoNewPrivileges=yes\n", " --expected-unit goatr.service"),
        _ => return Err(refused("unrecognized owned unit version")),
    };
    Ok(format!("# Goatr owned {title}\n[Unit]\nDescription=Goatr foreground companion\n[Service]\nType=exec\nUMask=0077\nRestart=no\n{authority}{environment}WorkingDirectory={}\nExecStart={} serve --config {}{context}\n[Install]\nWantedBy=default.target\n",
        unit_word(cwd, false)?, unit_word(binary, true)?, unit_word(config, true)?))
}
async fn manager(args: &[&str]) -> io::Result<String> {
    let mut command = tokio::process::Command::new("systemctl");
    command.arg("--user").args(args).kill_on_drop(true);
    let runtime=PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| refused("user manager runtime unavailable"))?);
    private_dir(&runtime,false)?;
    command.env("DBUS_SESSION_BUS_ADDRESS",format!("unix:path={}",runtime.join("systemd/private").display()))
        .env_remove("SYSTEMD_BUS_ADDRESS").env_remove("SYSTEMD_HOST").env_remove("SYSTEMD_MACHINE");
    #[cfg(target_os = "linux")]
    // SAFETY: the hook only clears capabilities with async-signal-safe syscalls.
    unsafe {
        command.pre_exec(crate::codex_relay::drop_inherited_capabilities);
    }
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command.output(),
    )
    .await
    .map_err(|_| refused("user manager deadline"))??;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err(refused("user manager unavailable or operation refused"));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}
fn manager_unit_source(shown: &str, unit: &Path, absent: bool, pending_owned_reload: bool) -> io::Result<bool> {
    let mut load = None;
    let mut fragment = None;
    let mut dropins = None;
    let mut reload = None;
    for line in shown.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| refused("invalid manager unit source"))?;
        let slot = match key {
            "LoadState" => &mut load,
            "FragmentPath" => &mut fragment,
            "DropInPaths" => &mut dropins,
            "NeedDaemonReload" => &mut reload,
            _ => return Err(refused("unexpected manager unit source property")),
        };
        if slot.replace(value).is_some() {
            return Err(refused("duplicate manager unit source property"));
        }
    }
    if dropins != Some("") || !(reload == Some("no") || pending_owned_reload && reload == Some("yes")) {
        return Err(refused(
            "unowned serve-unit drop-ins or changed effective source; no start",
        ));
    }
    match (load, fragment) {
        (Some("loaded"), Some(path)) if Path::new(path) == unit => Ok(true),
        (Some("not-found"), Some("")) if absent => Ok(false),
        _ => Err(refused(
            "serve unit is missing, masked, invalid or supplied by another fragment",
        )),
    }
}
fn manager_unit_text(text: &str, unit: &Path, content: &str) -> io::Result<()> {
    if text.trim_end() != format!("# {}\n{content}", unit.display()).trim_end() {
        return Err(refused(
            "effective serve fragment/drop-ins differ from exact owned source; no start",
        ));
    }
    Ok(())
}
fn reject_disk_dropins(paths: &str, unit: &Path) -> io::Result<()> {
    let mut includes_owned_parent = false;
    for root in paths.split_whitespace() {
        let root = Path::new(root);
        if !root.is_absolute() || root.as_os_str().as_encoded_bytes().contains(&b'\\') {
            return Err(refused("manager unit search paths cannot be proved"));
        }
        includes_owned_parent |= unit.parent() == Some(root);
        for name in ["goatr.service.d", "service.d"] {
            match fs::read_dir(root.join(name)) {
                Ok(entries) => {
                    for entry in entries {
                        if entry?.path().extension().is_some_and(|s| s == "conf") {
                            return Err(refused(
                                "unmanaged on-disk serve-unit drop-in; no reload or start",
                            ));
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    if !includes_owned_parent {
        return Err(refused(
            "owned unit parent is not in actual manager search paths",
        ));
    }
    Ok(())
}
async fn owned_unit_source(unit: &Path, content: &str, absent: bool) -> io::Result<()> {
    let paths = manager(&["show", "--property=UnitPath", "--value"]).await?;
    reject_disk_dropins(&paths, unit)?;
    let shown = manager(&[
        "show",
        "goatr.service",
        "--property=LoadState,FragmentPath,DropInPaths,NeedDaemonReload",
        "--all",
        "--no-pager",
    ])
    .await?;
    if manager_unit_source(&shown, unit, absent, false)? {
        expected_unit(unit, content)?;
        // Join current backing bytes too; the disk search above covers unreloaded additions.
        let text = manager(&["cat", "goatr.service", "--no-pager"]).await?;
        manager_unit_text(&text, unit, content)?;
    }
    Ok(())
}
fn owned_argv(owner: &UnitOwnership) -> io::Result<Vec<String>> {
    let mut argv = vec![owner.executable.to_str().ok_or_else(|| refused("owned executable not UTF-8"))?.to_owned(),
        "serve".into(), "--config".into(), owner.config.to_str().ok_or_else(|| refused("owned config not UTF-8"))?.to_owned()];
    if owner.version == 2 { argv.extend(["--expected-unit".into(), "goatr.service".into()]); }
    Ok(argv)
}
async fn migration_source(unit: &Path, owner: &UnitOwnership, legacy: &UnitOwnership, pending: bool, absent: bool, profile: Option<&crate::codex_relay::UnitWakePin>) -> io::Result<()> {
    if !pending { return owned_unit_source(unit, &owner.content, absent).await; }
    let paths = manager(&["show", "--property=UnitPath", "--value"]).await?;
    reject_disk_dropins(&paths, unit)?;
    let shown = manager(&["show", "goatr.service", "--property=LoadState,FragmentPath,DropInPaths,NeedDaemonReload", "--all", "--no-pager"]).await?;
    let loaded = manager_unit_source(&shown, unit, absent, true)?;
    expected_unit(unit, &owner.content)?;
    if !loaded { return Ok(()); }
    manager_unit_text(&manager(&["cat", "goatr.service", "--no-pager"]).await?, unit, &owner.content)?;
    let profile=profile.ok_or_else(||refused("pending Wake source requires the original qualified setup manager"))?;
    profile.recheck(None)?;
    profile.owned_serve_argv(&owned_argv(owner)?,&owned_argv(legacy)?)?;
    profile.recheck(None)?;
    Ok(())
}
/// Durable Wake startup must retain the private record AND exact effective owned source.
pub fn admit_owned_serve(pin: &crate::codex_relay::UnitWakePin, path: &Path) -> io::Result<()> {
    pin.recheck(None)?;
    let config = Config::load(path)?;
    let record_path = path.parent().ok_or_else(|| refused("owned config parent missing"))?.join(".serve-unit.json");
    let record = ExpectedFile::capture(&record_path)?;
    let owner: UnitOwnership = serde_json::from_slice(record.bytes().ok_or_else(|| refused("owned Wake record absent"))?)
        .map_err(|_| refused("invalid owned Wake record"))?;
    if owner.version != 2 || owner.config != path || owner.cwd != config.runtime_dir
        || owner.executable != fs::canonicalize(std::env::current_exe()?)? || pin.unit() != "goatr.service"
        || owner.unit != unit_home().join("systemd/user/goatr.service")
        || owner.content != unit_content(&owner.executable, path, &config.runtime_dir, &unit_environment_current()?, 2)?
        || crate::codex_relay::process_arguments(pin.process())? != owned_argv(&owner)? {
        return Err(refused("durable Wake process differs from private original ownership"));
    }
    let expected = ExpectedFile::capture(&owner.unit)?;
    validate_unit_ownership(&owner, None, &expected, &record)?;
    reject_disk_dropins(&pin.manager_output(&["--user", "show", "--property=UnitPath", "--value"])?, &owner.unit)?;
    let shown = pin.manager_output(&["--user", "show", "goatr.service", "--property=LoadState,FragmentPath,DropInPaths,NeedDaemonReload", "--all", "--no-pager"])?;
    if !manager_unit_source(&shown, &owner.unit, false, false)? { return Err(refused("owned Wake fragment absent")); }
    manager_unit_text(&pin.manager_output(&["--user", "cat", "goatr.service", "--no-pager"])?, &owner.unit, &owner.content)?;
    expected.recheck(&owner.unit)?; record.recheck(&record_path)?; pin.recheck(None)?; Ok(())
}
async fn owned_serve_peer(
    owner: &UnitOwnership,
    socket: &Path,
    profile: Option<&crate::codex_relay::UnitWakePin>,
) -> io::Result<crate::herdr::ProcessEvidence> {
    if owner.version == 2 { profile.ok_or_else(|| refused("qualified setup context required BEFORE inspecting Wake serve"))?.recheck(None)?; }
    let binary = owner.executable.as_path(); let cwd = owner.cwd.as_path();
    let pid = manager(&["show", "goatr.service", "--property=MainPID", "--value"])
        .await?
        .trim()
        .parse::<u32>()
        .map_err(|_| refused("owned serve unit PID unavailable"))?;
    let process = crate::herdr::process_evidence(pid)?;
    if fs::read_link(format!("/proc/{pid}/exe"))? != binary
        || fs::read_link(format!("/proc/{pid}/cwd"))? != cwd
    {
        return Err(refused(
            "serve unit executable/cwd differs from this owned package/runtime",
        ));
    }
    let args = crate::codex_relay::process_arguments(&process)?;
    if args != owned_argv(owner)? {
        return Err(refused(
            "serve unit actual argv differs from owned executable/config",
        ));
    }
    let executable = File::open(format!("/proc/{pid}/exe"))?;
    let runtime = if owner.version == 2 {
        Some(crate::codex_relay::UnitWakePin::capture(process.clone(), &executable, "goatr.service",None)?)
    } else {
        let mut status = String::new(); File::open(format!("/proc/{pid}/status"))?.take(65_537).read_to_string(&mut status)?;
        if !crate::codex_relay::capability_state(&status)?.zero() { return Err(refused("original v1 serve is not zero-authority")); }
        None
    };
    admin::validate_socket_path(socket, false, true)?;
    let before = fs::symlink_metadata(socket)?;
    let stream = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::UnixStream::connect(socket),
    )
    .await
    .map_err(|_| refused("owned serve admin connection deadline"))??;
    admin::same_uid(&stream)?;
    if stream.peer_cred()?.pid() != Some(pid as i32)
        || crate::herdr::process_evidence(pid)? != process
    {
        return Err(refused(
            "local admin socket is not this exact owned serve process",
        ));
    }
    let after = fs::symlink_metadata(socket)?;
    if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err(refused(
            "local admin socket changed during owned readiness proof",
        ));
    }
    if let Some(pin) = runtime { pin.recheck(None)?; }
    Ok(process)
}
async fn serve_unit(
    binary: &Path,
    path: &Path,
    config: &Config,
    lock: &SetupLock,
    config_changed: bool,
    profile: Option<&crate::codex_relay::UnitWakePin>,
) -> io::Result<()> {
    if !binary.starts_with("/nix/store") {
        return Err(refused(
            "owned serve unit requires the durable absolute packaged Nix executable, not checkout/target scripts",
        ));
    }
    let manifest = binary
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| refused("package root missing"))?
        .join("share/goatr/manifest.json");
    let mut metadata = Vec::new();
    File::open(manifest)?
        .take((CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut metadata)?;
    if metadata.len() > CONFIG_BYTES {
        return Err(refused("packaged provenance exceeds bound"));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&metadata)
        .map_err(|_| refused("packaged provenance unavailable"))?;
    if metadata["kind"].as_str() != Some("direct-rust")
        || metadata["lockSha256"].as_str() != Some(crate::transport::lock_sha256())
    {
        return Err(refused(
            "packaged executable/source lock provenance differs",
        ));
    }
    let parent = unit_home().join("systemd/user");
    trusted_parent(&parent)?;
    let unit = parent.join("goatr.service");
    let record = lock.path.join(".serve-unit.json");
    let environment = unit_environment_current()?;
    let content = unit_content(binary, path, &config.runtime_dir, &environment, 1)?;
    let legacy = UnitOwnership { version: 1, unit: unit.clone(), executable: binary.to_owned(),
        config: path.to_owned(), cwd: config.runtime_dir.clone(), content };
    let planned = UnitOwnership { version: if profile.is_some() { 2 } else { 1 },
        content: unit_content(binary, path, &config.runtime_dir, &environment, if profile.is_some() { 2 } else { 1 })?,
        unit: unit.clone(), executable: binary.to_owned(), config: path.to_owned(), cwd: config.runtime_dir.clone() };
    let expected = ExpectedFile::capture(&unit)?;
    let ownership = ExpectedFile::capture(&record)?;
    let pair = validate_unit_ownership(&planned, profile.map(|_| &legacy), &expected, &ownership)?;
    let old_content = pair.record_version == 1 && !pair.unit_new;
    let source = if old_content { &legacy } else { &planned };
    let pending = pair.record_version != 0 && pair.unit_new && planned.version == 2;
    migration_source(&unit, source, &legacy, pending, expected.bytes().is_none(), profile).await?;
    println!(
        "Optional independent serve unit: {}\n{}",
        unit.display(),
        planned.content
    );
    let socket = admin::default_socket()?;
    let mut active_original = None;
    if fs::symlink_metadata(&socket).is_ok() {
        if expected.bytes().is_none() || ownership.bytes().is_none() {
            return Err(refused(
                "a pre-existing companion/admin owner is not this unit; no second daemon or adoption",
            ));
        }
        // Running argv/credentials belong to the original epoch, not newly installed bytes.
        let pid = manager(&["show", "goatr.service", "--property=MainPID", "--value"]).await?.trim().parse::<u32>().map_err(|_| refused("active unit PID unavailable"))?;
        let process = crate::herdr::process_evidence(pid)?;
        let actual = crate::codex_relay::process_arguments(&process)?;
        let running_owner = if actual == owned_argv(&legacy)? { &legacy } else { &planned };
        active_original = Some((owned_serve_peer(running_owner, &socket, profile).await?, running_owner.version));
    }
    let active_state = manager(&["show", "goatr.service", "--property=ActiveState", "--value"]).await?;
    if active_state.trim() == "active" && active_original.is_none() {
        return Err(refused("active original serve has no proved protected admin peer; no staging/start/adoption"));
    }
    if !confirm("Install this exact owned serve unit (does not start or pair)?").await? {
        return Ok(());
    }
    ownership.recheck(&record)?;
    expected.recheck(&unit)?;
    migration_source(&unit, source, &legacy, pending, expected.bytes().is_none(), profile).await?;
    ownership.recheck(&record)?; expected.recheck(&unit)?;
    if let Some(pin) = profile { pin.recheck(None)?; }
    let unit_lock = SetupLock::unit_parent(&parent)?;
    if pair.record_version == 1 && planned.version == 2 {
        // Existing validated ownership: unit-first gives only old/old -> old/new -> new/new.
        unit_lock.publish(&unit, &expected, planned.content.as_bytes())?;
        ownership.recheck(&record)?;
        let installed = ExpectedFile::capture(&unit)?;
        migration_source(&unit, &planned, &legacy, true, false, profile).await?;
        installed.recheck(&unit)?;
        lock.publish(&record, &ownership, &serde_json::to_vec(&planned).map_err(io::Error::other)?)?;
    } else {
        // Initial install remains record-first; absent record/present identical unit is foreign.
        lock.publish(&record, &ownership, &serde_json::to_vec(&planned).map_err(io::Error::other)?)?;
        unit_lock.publish(&unit, &expected, planned.content.as_bytes())?;
    }
    let installed = ExpectedFile::capture(&unit)?;
    let installed_record = ExpectedFile::capture(&record)?;
    if planned.version == 2 {
        migration_source(&unit, &planned, &legacy, true, pair.record_version == 0 || expected.bytes().is_none(), profile).await?;
    }
    installed.recheck(&unit)?; installed_record.recheck(&record)?;
    if let Some(pin) = profile { pin.recheck(None)?; }
    manager(&["daemon-reload"]).await?;
    installed.recheck(&unit)?; installed_record.recheck(&record)?;
    owned_unit_source(&unit, &planned.content, false).await?;
    if let Some((process, version)) = &active_original
        && active_staged(*version, planned.version, config_changed) {
        let owner = if *version == 1 { &legacy } else { &planned };
        if owned_serve_peer(owner, &socket, profile).await? != *process { return Err(refused("active staged original changed")); }
        println!("STAGED: exact new owned configuration installed; original active process/argv/profile retained. No restart or new Wake readiness claimed.");
        return Ok(());
    }
    if !confirm(
        "Start only this owned serve unit now (separate consent; no provider/relay/TUI start)?",
    )
    .await?
    {
        return Ok(());
    }
    if planned.version == 2 { profile.ok_or_else(|| refused("qualified setup context required BEFORE Wake start"))?.recheck(None)?; }
    owned_unit_source(&unit, &planned.content, false).await?;
    let active = manager(&["show", "goatr.service", "--property=ActiveState", "--value"]).await?;
    if active.trim() != "active" {
        owned_unit_source(&unit, &planned.content, false).await?;
        manager(&["start", "goatr.service"]).await?;
    } else if config_changed {
        println!(
            "Already active: configuration is saved but NOT reloaded. Explicitly stop/start only this owned serve later to apply changes; setup never silently restarts it."
        );
    }
    let socket = admin::default_socket()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match admin::call(&socket, "devices.list", json!({})).await {
            Ok(value) => {
                let _: admin::Devices = serde_json::from_value(value).map_err(io::Error::other)?;
                let ready = owned_serve_peer(&planned, &socket, profile).await?;
                if active_original.as_ref().is_some_and(|(original,_)| *original != ready) { return Err(refused("original active owned serve was replaced")); }
                println!(
                    "This exact owned serve is responsive through protected devices.list; pairing has not been opened."
                );
                break;
            }
            Err(e) if tokio::time::Instant::now() >= deadline => return Err(e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    Ok(())
}
fn active_staged(running_version: u32, installed_version: u32, config_changed: bool) -> bool {
    running_version != installed_version || config_changed
}
fn expected_unit(path: &Path, content: &str) -> io::Result<()> {
    let source = ExpectedFile::capture(path)?;
    if source.bytes() != Some(content.as_bytes()) {
        return Err(refused("owned unit content changed"));
    }
    Ok(())
}
fn foreground_instructions(binary: &Path, path: &Path) {
    println!(
        "Foreground: {} serve --config {}\nThen explicitly: {} pair\nList: {} devices\nRevoke: {} revoke DEVICE_ID",
        quote(binary),
        quote(path),
        quote(binary),
        quote(binary),
        quote(binary)
    );
}
pub async fn run(sub: &ArgMatches, path: &Path, profile: Option<&crate::codex_relay::UnitWakePin>) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(refused(
            "setup requires a local interactive terminal; no noninteractive blanket consent",
        ));
    }
    config::absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| refused("config parent missing"))?;
    trusted_parent(parent)?;
    let expected = ExpectedFile::capture(path)?;
    let mut c = match expected.bytes() {
        Some(bytes) => Config::parse(std::str::from_utf8(bytes).map_err(io::Error::other)?)?,
        None => new_config(sub)?,
    };
    if expected.bytes().is_some() {
        private_dir(parent, false)?;
        for key in [
            "runtime-dir",
            "runtime-dir-agent",
            "herdr",
            "omp",
            "codex",
            "omp-registry-dir",
            "omp-session-root",
            "omp-blob-root",
            "codex-owner-socket",
            "codex-owner-socket-agent",
            "codex-bindings-dir",
            "codex-unit",
        ] {
            if sub.contains_id(key)
                && sub.value_source(key) == Some(clap::parser::ValueSource::CommandLine)
            {
                return Err(refused(
                    "existing configuration retains wrappers/runtime/owners; edit its explicit private source locally, then rerun",
                ));
            }
        }
    }
    c.preflight()?;
    if let Some(projects) = sub.get_many::<String>("project") {
        for p in projects {
            c.approve_project(p)?;
        }
    } else {
        println!(
            "Configured projects are retained. Known Herdr cwd suggestions require an explicit subsequent --project approval:"
        );
        match crate::herdr::discover(&c).await {
            Ok(slots) => {
                for slot in slots {
                    if let Ok(client) = crate::herdr::Client::connect(&slot.socket_path).await
                        && let Ok(client) = client.qualify_executable(&c.executables.herdr)
                        && let Ok((_, snapshot)) = client.snapshot().await
                    {
                        for pane in snapshot.panes {
                            if let Some(cwd) = pane.cwd
                                && config::project_directory(Path::new(&cwd)).is_ok()
                            {
                                println!("{}", quote(Path::new(&cwd)));
                            }
                        }
                    }
                }
            }
            Err(e) => println!("Herdr discovery unavailable: {e}"),
        }
    }
    if let Some(id) = sub.get_one::<String>("codex-owner") {
        let id = Uuid::new(id.clone()).map_err(io::Error::other)?;
        if !c.codex_owners.iter().any(|o| o.id == id) {
            return Err(refused(
                "--codex-owner must select an existing configured owner UUID",
            ));
        }
        c.default_codex_owner_id = Some(id);
    }
    if sub.get_flag("allow-scoped-shell-probe") && !c.allow_scoped_shell_probe {
        println!(
            "Scoped consent permits ONE fixed read-only resolution query only in each fresh Goatr-created idle no-focus root pane. Shell history/hooks may observe it; queued input cannot be recalled. It never probes old panes, rewrites PATH/rc, executes provider versions or proves readiness by itself."
        );
        if !confirm("Save this exact scoped shell-probe permission?").await? {
            println!("Scoped probe declined; no effects.");
            return Ok(());
        }
        c.allow_scoped_shell_probe = true;
    }
    if sub.get_flag("disable-scoped-shell-probe") {
        c.allow_scoped_shell_probe = false;
    }
    if sub.get_flag("allow-terminal-control") && !c.allow_terminal_control {
        println!("Terminal control permits an explicitly selected known non-agent pane in the connected Herdr session. Begin/Resize may resize every workspace/tab and let Herdr resume deferred agents; observed agent transitions retire Goatr input, but queued native bytes cannot be recalled. Native Close is unavailable.");
        if !confirm("Save this terminal-control permission? Changes require companion restart.").await? {
            println!("Terminal control declined; no effects.");
            return Ok(());
        }
        c.allow_terminal_control = true;
    }
    if sub.get_flag("disable-terminal-control") {
        c.allow_terminal_control = false;
    }
    if let Some(package)=sub.get_one::<PathBuf>("omp-skills-package-root") {
        config::absolute(package)?;
        let package=fs::canonicalize(package)?;
        if !package.is_dir() {return Err(refused("OMP package root is not a directory"))}
        crate::backends::omp::qualify_extension_package(&package)?;
        println!("Qualified public OMP extension source: {}",quote(&package));
        println!("Only future owned OMP launches may load the supported extension. Existing originals are never restarted, and disk source does not prove their loaded code.");
        if !confirm("Save this qualified package source? Changes require companion restart.").await? {return Ok(())}
        c.omp_skills_package_root=Some(package);
    }
    if sub.get_flag("allow-omp-controls")&&!c.allow_omp_controls {
        println!("OMP controls permit shared native model/thinking changes and explicit native delivery on an original that loaded the qualified authenticated extension. Opening a panel does nothing; Apply affects the shared session. Native Plan, queue editing and Skill invocation are unavailable.");
        if !confirm("Save this separate OMP-controls permission? Changes require companion restart.").await? {return Ok(())}
        c.allow_omp_controls=true;
    }
    if sub.get_flag("disable-omp-controls") {c.allow_omp_controls=false;}
    if sub.get_flag("clear-skill-document-roots") {c.skill_document_roots.clear();}
    if let Some(roots)=sub.get_many::<PathBuf>("skill-document-root") {
        let mut selected=Vec::new();
        for root in roots {
            config::absolute(root)?;
            let root=fs::canonicalize(root)?;
            config::project_directory(&root)?;
            println!("Enabled-Skill document read root: {}",quote(&root));
            if !selected.contains(&root) {selected.push(root);}
        }
        println!("This separately grants complete bounded UTF-8 previews only for currently enabled native Skill catalogue documents beneath these roots. It grants no arbitrary phone paths, creation, Git/file tools, attachments or model-tool approval. Roots are held for this companion lifetime.");
        if !confirm("Add these document-read roots? Changes require companion restart.").await? {return Ok(())}
        for root in selected {if !c.skill_document_roots.contains(&root) {c.skill_document_roots.push(root);}}
    }
    if let Some(paths)=sub.get_many::<PathBuf>("disable-project-read") {
        for path in paths {c.set_project_read_tools(path,false)?;}
    }
    if let Some(paths)=sub.get_many::<PathBuf>("allow-project-read") {
        let mut selected=Vec::new();
        for path in paths {
            config::absolute(path)?;
            let path=fs::canonicalize(path)?;config::project_directory(&path)?;
            if !c.projects.iter().any(|project|project.path==path) {return Err(refused("read permission requires an already configured project; approve it with --project first"))}
            println!("Project subtree read/export and confined Git-review root: {}",quote(&path));
            if !selected.contains(&path) {selected.push(path);}
        }
        println!("This separately permits bounded artifact browsing/export and read-only Git review beneath these held project roots. It grants no staging, commits, worktree mutation, arbitrary phone paths, terminal input, attachments or model-tool permission. Outside Git administration/configuration/alternates and unsupported confinement refuse the whole Git observation.");
        if !confirm("Save these separate project-read grants? Changes and revocation require companion restart.").await? {return Ok(())}
        for path in selected {c.set_project_read_tools(&path,true)?;}
    }
    c.preflight()?;
    for p in &c.projects {
        println!(
            "Approved project {} {} {} (read/export/Git: {})",
            p.id.0,
            p.label.0,
            quote(&p.path),
            if p.allow_read_tools {"explicitly granted"}else{"not granted"},
        );
    }
    for provider in [Provider::Omp, Provider::Codex] {
        println!("{provider:?}: {}",c.backend_reason(provider)?.unwrap_or("trusted local prerequisites; actual source/new-root wrapper views/native readiness are checked before creation, not proved by setup"));
    }
    println!(
        "Projects restrict creation; explicit per-project read grants separately authorize bounded subtree export/Git review. Paired phones still control all supported same-user Herdr conversations. No identity, keys, devices or receipts are opened/rekeyed."
    );
    println!("Push publishing permits only the configured exact HTTPS origins {:?}. The default is https://ntfy.sh; an explicit empty list denies all and self-hosted origins require local consent. Saving does not register a phone or select its delivery mode.",c.push_allowed_origins);
    let proposed = c.serialized()?;
    let identical = expected.bytes().is_some_and(|bytes| {
        Config::parse(std::str::from_utf8(bytes).unwrap_or(""))
            .and_then(|old| old.serialized())
            .is_ok_and(|old| old == proposed)
    });
    if !identical {
        println!(
            "Exact proposed private configuration at {}:\n{proposed}",
            path.display()
        );
        if !confirm("Save this configuration atomically?").await? {
            println!("Setup declined; no config/unit/key/provider effects.");
            return Ok(());
        }
    }
    let binary = fs::canonicalize(std::env::current_exe()?)?;
    config::executable(&binary)?;
    if identical && !sub.get_flag("serve-unit") {
        expected.recheck(path)?;
        println!("Identical setup: no file/config/unit/key/provider change.");
        foreground_instructions(&binary, path);
        return Ok(());
    }
    let lock = SetupLock::acquire(parent)?;
    expected.recheck(path)?;
    if !identical {
        lock.publish(path, &expected, proposed.as_bytes())?;
        println!("Private configuration saved durably.");
    } else {
        println!("Identical setup: config bytes and IDs unchanged.");
    }
    foreground_instructions(&binary, path);
    if sub.get_flag("serve-unit")
        && let Err(e) = serve_unit(&binary, path, &c, &lock, !identical, profile).await
    {
        eprintln!(
            "Serve unit unavailable/not healthy: {e}. Saved configuration remains usable with the absolute foreground instructions above."
        );
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn owned_unit_crash_pairs_migrate_only_recognized_versions_and_keep_active_old_staged() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap()).join(format!(".goatr-unit-pairs-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let unit = root.join("goatr.service"); let record = root.join("record");
        let make = |version| UnitOwnership { version, unit: unit.clone(), executable: root.join("goatr"),
            config: root.join("config"), cwd: root.clone(),
            content: unit_content(&root.join("goatr"), &root.join("config"), &root, "Environment=\"HOME=/owned\"\n", version).unwrap() };
        let old = make(1); let new = make(2);
        assert_eq!(owned_argv(&old).unwrap().len(),4);
        assert_eq!(&owned_argv(&new).unwrap()[4..],["--expected-unit","goatr.service"]);
        assert!(active_staged(1,2,false));assert!(active_staged(2,2,true));assert!(!active_staged(2,2,false));
        let put = |path:&Path, bytes:Option<Vec<u8>>| {
            if let Some(bytes)=bytes { fs::write(path,bytes).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap(); }
            else if path.exists() { fs::remove_file(path).unwrap(); }
        };
        for (record_version, unit_version, allowed) in [
            (0,0,true),(0,2,false),(1,1,true),(1,2,true),(2,2,true),(2,0,true),
            (2,1,false),(1,0,false),(0,1,false),
        ] {
            let owner=match record_version {1=>Some(&old),2=>Some(&new),_=>None};
            put(&record,owner.map(|owner|serde_json::to_vec(owner).unwrap()));
            put(&unit,match unit_version {1=>Some(old.content.as_bytes().to_vec()),2=>Some(new.content.as_bytes().to_vec()),_=>None});
            let held_unit=ExpectedFile::capture(&unit).unwrap();let held_record=ExpectedFile::capture(&record).unwrap();
            let pair=validate_unit_ownership(&new,Some(&old),&held_unit,&held_record);
            assert_eq!(pair.is_ok(),allowed,"record{record_version}/unit{unit_version}");
            if let Ok(pair)=pair { assert_eq!(pair.record_version,record_version);assert_eq!(pair.unit_new,unit_version==2); }
        }
        let mut foreign=make(1);foreign.executable=root.join("other-goatr");
        put(&record,Some(serde_json::to_vec(&foreign).unwrap()));put(&unit,Some(old.content.as_bytes().to_vec()));
        assert!(validate_unit_ownership(&new,Some(&old),&ExpectedFile::capture(&unit).unwrap(),&ExpectedFile::capture(&record).unwrap()).is_err());
        // A restored path cannot satisfy a previously held pre-publication identity.
        let held=ExpectedFile::capture(&unit).unwrap();put(&unit,Some(new.content.as_bytes().to_vec()));assert!(held.recheck(&unit).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn disk_search_rejects_unreloaded_named_and_service_wide_dropins() {
        let root = std::env::temp_dir().join(format!("goatr-dropins-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let unit = root.join("goatr.service");
        let paths = root.to_str().unwrap();
        reject_disk_dropins(paths, &unit).unwrap();
        for name in ["goatr.service.d", "service.d"] {
            let directory = root.join(name);
            fs::create_dir(&directory).unwrap();
            let foreign = directory.join("override.conf");
            fs::write(&foreign, "[Service]\nExecStartPre=/foreign/executable\n").unwrap();
            assert!(reject_disk_dropins(paths, &unit).is_err());
            fs::remove_file(foreign).unwrap();
            fs::remove_dir(directory).unwrap();
        }
        assert!(reject_disk_dropins("", &unit).is_err());
        assert!(reject_disk_dropins("/not-the-owned-parent", &unit).is_err());
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn effective_unit_rejects_loaded_and_pending_overrides_before_any_start() {
        let unit = Path::new("/owned/user/goatr.service");
        let clean = "LoadState=loaded\nFragmentPath=/owned/user/goatr.service\nDropInPaths=\nNeedDaemonReload=no\n";
        assert!(manager_unit_source(clean, unit, false, false).unwrap());
        let pending=clean.replace("NeedDaemonReload=no","NeedDaemonReload=yes");
        assert!(manager_unit_source(&pending,unit,false,false).is_err());
        assert!(manager_unit_source(&pending,unit,false,true).unwrap());
        for bad in [pending.replace("DropInPaths=","DropInPaths=/foreign/override.conf"),
            pending.replace("/owned/user/goatr.service","/foreign/goatr.service"),format!("{pending}NeedDaemonReload=no\n")] {
            assert!(manager_unit_source(&bad,unit,false,true).is_err());
        }
        for foreign in [
            clean.replace(
                "DropInPaths=",
                "DropInPaths=/owned/user/goatr.service.d/override.conf",
            ),
            clean.replace("/owned/user/goatr.service\n", "/foreign/goatr.service\n"),
            clean.replace("NeedDaemonReload=no", "NeedDaemonReload=yes"),
            clean.replace("LoadState=loaded", "LoadState=masked"),
            clean.replace("DropInPaths=\n", ""),
            format!("{clean}DropInPaths=\n"),
        ] {
            assert!(manager_unit_source(&foreign, unit, false, false).is_err());
        }
        let content = "[Service]\nExecStart=/owned/goatr serve\n";
        let clean = format!("# {}\n{content}", unit.display());
        manager_unit_text(&clean, unit, content).unwrap();
        for command in ["ExecStart", "ExecStartPre", "ExecStartPost"] {
            let foreign = format!(
                "{clean}\n# /owned/user/goatr.service.d/override.conf\n[Service]\n{command}=/foreign/executable\n"
            );
            assert!(manager_unit_text(&foreign, unit, content).is_err());
        }
    }
    #[test]
    fn unit_ownership_and_literal_paths_do_not_adopt_foreign_files_or_rewrite_parent_permissions() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".goatr-unit-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let units = root.join("user");
        fs::DirBuilder::new().mode(0o755).create(&units).unwrap();
        let path = units.join("goatr.service");
        let record = root.join("record");
        let planned = UnitOwnership {
            version: 1,
            unit: path.clone(),
            executable: root.join("goatr"),
            config: root.join("config"),
            cwd: root.clone(),
            content: "Type=exec\nRestart=no\n".into(),
        };
        let lock = SetupLock::unit_parent(&units).unwrap();
        assert!(SetupLock::unit_parent(&units).is_err());
        lock.publish(&path, &ExpectedFile::Absent, planned.content.as_bytes())
            .unwrap();
        let file = ExpectedFile::capture(&path).unwrap();
        assert!(validate_unit_ownership(&planned, None, &file, &ExpectedFile::Absent).is_err());
        fs::write(&record, serde_json::to_vec(&planned).unwrap()).unwrap();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        let owned = ExpectedFile::capture(&record).unwrap();
        validate_unit_ownership(&planned, None, &file, &owned).unwrap();
        fs::write(&path, "foreign bytes").unwrap();
        assert!(file.recheck(&path).is_err());
        assert_eq!(fs::metadata(&units).unwrap().mode() & 0o777, 0o755);
        let literal = Path::new("/absolute/${NAME}%suffix");
        assert_eq!(
            unit_word(literal, true).unwrap(),
            "\"/absolute/$${NAME}%%suffix\""
        );
        assert_eq!(
            unit_word(literal, false).unwrap(),
            "\"/absolute/${NAME}%%suffix\""
        );
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }
}
