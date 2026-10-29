// SPDX-License-Identifier: GPL-3.0-or-later
//! Actual default-admin/setup/user-unit consumer; requires a separately provisioned login.
use super::*;
use std::{
    ffi::OsString,
    io::Write,
    os::{fd::FromRawFd, unix::fs::OpenOptionsExt},
};
struct Terminal {
    child: Child,
    master: File,
    process: ProcessEvidence,
    bytes: Vec<u8>,
    prompts: usize,
}
impl Terminal {
    fn setup(binary: &Path, args: &[OsString]) -> io::Result<Self> {
        let fd = unsafe {
            libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC | libc::O_NONBLOCK)
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let master = unsafe { File::from_raw_fd(fd) };
        if unsafe { libc::grantpt(fd) } != 0 || unsafe { libc::unlockpt(fd) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut name = [0i8; 256];
        if unsafe { libc::ptsname_r(fd, name.as_mut_ptr(), name.len()) } != 0 {
            return Err(failure("owned setup PTY name unavailable"));
        }
        let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
        let slave = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
            .open(Path::new(
                std::str::from_utf8(name.to_bytes()).map_err(io::Error::other)?,
            ))?;
        let mut command = std::process::Command::new(binary);
        command.arg("setup").args(args)
            .stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave)).stderr(Stdio::null());
        #[cfg(target_os="linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
        let child = command.spawn()?;
        let process = herdr::process_evidence(child.id())?;
        Ok(Self {
            child,
            master,
            process,
            bytes: Vec::new(),
            prompts: 0,
        })
    }
    async fn finish(mut self, answers: &[&str], cancel: bool) -> io::Result<bool> {
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut chunk = [0u8; 4096];
        loop {
            match self.master.read(&mut chunk) {
                Ok(n) => {
                    if self.bytes.len() + n > 256 * 1024 {
                        return Err(failure("setup terminal output bound"));
                    }
                    self.bytes.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.raw_os_error() == Some(libc::EIO) => {}
                Err(e) => return Err(e),
            }
            let count = self.bytes.windows(9).filter(|w| *w == b"[yes/no] ").count();
            if count > self.prompts {
                self.prompts = count;
                if cancel {
                    if herdr::process_evidence(self.process.pid)? != self.process {
                        return Err(failure("setup cancellation process changed"));
                    }
                    unsafe { libc::kill(self.process.pid as i32, libc::SIGTERM) };
                } else {
                    let answer = answers
                        .get(count - 1)
                        .ok_or_else(|| failure("unexpected extra setup consent; no blanket yes"))?;
                    self.master.write_all(answer.as_bytes())?;
                }
            }
            if let Some(status) = self.child.try_wait()? {
                self.bytes.fill(0);
                return Ok(status.success());
            }
            if Instant::now() > deadline {
                return Err(failure("actual setup terminal deadline exceeded"));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.bytes.fill(0);
        if self.child.try_wait().is_ok_and(|s| s.is_none())
            && herdr::process_evidence(self.process.pid).is_ok_and(|p| p == self.process)
        {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
async fn systemd(args: &[&str]) -> io::Result<String> {
    let mut c = tokio::process::Command::new("systemctl");
    c.arg("--user").args(args);
    String::from_utf8(output(c, 15, 65536).await?).map_err(io::Error::other)
}
async fn cli(binary: &Path, command: &str, id: Option<&Uuid>) -> io::Result<String> {
    let mut c = tokio::process::Command::new(binary);
    c.arg(command);
    if let Some(id) = id {
        c.arg(&id.0);
    }
    String::from_utf8(output(c, 15, 65536).await?).map_err(io::Error::other)
}
fn add(args: &mut Vec<OsString>, name: &str, path: &Path) {
    args.push(name.into());
    args.push(path.as_os_str().to_owned());
}
fn state_objects(root: &Path) -> io::Result<Value> {
    let mut objects = Vec::new();
    for name in ["identity.keys", "identity.initialized", "serve.lock"] {
        let path = root.join(name);
        private_file(&path)?;
        objects.push(identity(&path)?);
    }
    Ok(json!(objects))
}
pub(super) async fn run(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    let declared=args.get_one::<u32>("disposable-uid").copied().ok_or_else(||failure("--setup requires --disposable-uid from a pre-provisioned separate login with its own unchanged runtime/user manager; no sudo/users/global installation"))?;
    if declared != uid() || declared == 1000 {
        return Err(failure(
            "setup acceptance refuses the recorded workstation production UID1000 or another login; use its separately provisioned disposable test UID",
        ));
    }
    let binary = packaged(args)?;
    let source = Config::load(args.get_one::<PathBuf>("config").ok_or_else(|| {
        failure("actual private --config template with installed outer wrappers required")
    })?)?;
    let config_path = goatr::config::default_path()?;
    let socket = goatr::admin::default_socket()?;
    let state = pairing::state_root()?;
    if fs::symlink_metadata(&config_path).is_ok()
        || fs::symlink_metadata(&socket).is_ok()
        || fs::symlink_metadata(&state).is_ok()
    {
        return Err(failure(
            "disposable default config/state/admin ownership is not absent; no production/unproven adoption",
        ));
    }
    let config_home = config_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| failure("default config home unavailable"))?;
    let unit = config_home.join("systemd/user/goatr.service");
    if fs::symlink_metadata(&unit).is_ok() {
        return Err(failure(
            "pre-existing serve unit blocks disposable setup acceptance",
        ));
    }
    let loaded = systemd(&[
        "show",
        "goatr.service",
        "--property=LoadState,FragmentPath,DropInPaths",
        "--all",
        "--no-pager",
    ])
    .await?;
    if loaded.lines().any(|s| {
        s.strip_prefix("FragmentPath=")
            .or_else(|| s.strip_prefix("DropInPaths="))
            .is_some_and(|p| !p.is_empty())
    }) {
        return Err(failure(
            "disposable manager already owns a foreign/declarative serve unit",
        ));
    }
    let workspace = fs::canonicalize(
        args.get_one::<PathBuf>("workspace-root")
            .cloned()
            .unwrap_or(std::env::current_dir()?),
    )?;
    goatr::config::project_directory(&workspace)?;
    let run = Uuid::random();
    let root = workspace.join(format!(".gs-{}", &run.0[..8]));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    for child in ["runtime", "project", "runtime/b"] {
        fs::DirBuilder::new().mode(0o700).create(root.join(child))?;
    }
    write_json(
        &evidence.join(format!("setup-owner-{}.json", run.0)),
        &json!({"runId":run,"uid":uid(),"root":identity(&root)?,"package":binary,"packageSha256":binary_sha256(&binary)?,"defaultConfig":config_path,"defaultAdmin":socket,"state":state,"unit":unit,"runtimeUnchanged":std::env::var_os("XDG_RUNTIME_DIR"),"scope":"pre-provisioned disposable login only; no setup/admin override or production installer"}),
    )?;
    let mut initial = Vec::new();
    add(&mut initial, "--config", &config_path);
    add(&mut initial, "--runtime-dir", &root.join("runtime"));
    add(&mut initial, "--herdr", &source.executables.herdr);
    add(&mut initial, "--project", &root.join("project"));
    if let Some(path) = &source.executables.omp {
        add(&mut initial, "--omp", path);
    }
    if let Some(path) = &source.executables.codex {
        add(&mut initial, "--codex", path);
        add(
            &mut initial,
            "--codex-owner-socket",
            &root.join("runtime/c.sock"),
        );
        add(
            &mut initial,
            "--codex-bindings-dir",
            &root.join("runtime/b"),
        );
    }
    if let Some(path) = &source.omp_registry_dir {
        add(&mut initial, "--omp-registry-dir", path);
    }
    for path in &source.omp_session_roots {
        add(&mut initial, "--omp-session-root", path);
    }
    if let Some(path) = &source.omp_blob_root {
        add(&mut initial, "--omp-blob-root", path);
    }
    initial.push("--allow-scoped-shell-probe".into());
    if !Terminal::setup(&binary, &initial)?
        .finish(&["yes\n", "no\n"], false)
        .await?
    {
        return Err(failure("actual writable setup decline failed"));
    }
    if config_path.exists() || unit.exists() || state.exists() {
        return Err(failure("actual setup decline changed config/unit/identity"));
    }
    if Terminal::setup(&binary, &initial)?
        .finish(&[], true)
        .await?
    {
        return Err(failure("cancelled actual setup reported success"));
    }
    if config_path.exists() || unit.exists() || state.exists() {
        return Err(failure("actual setup cancel changed config/unit/identity"));
    }
    if !Terminal::setup(&binary, &initial)?
        .finish(&["yes\n", "yes\n"], false)
        .await?
    {
        return Err(failure("actual absent private config setup failed"));
    }
    let saved = Config::load(&config_path)?;
    if saved.projects.len() != 1 || !saved.allow_scoped_shell_probe {
        return Err(failure(
            "actual setup did not save explicit approval/consent",
        ));
    }
    let project_id = saved.projects[0].id.clone();
    let config_source = goatr::setup::ExpectedFile::capture(&config_path)?;
    let repeat = vec![
        OsString::from("--config"),
        config_path.as_os_str().to_owned(),
        OsString::from("--project"),
        root.join("project").into_os_string(),
        OsString::from("--project"),
        root.join("project").into_os_string(),
    ];
    if !Terminal::setup(&binary, &repeat)?
        .finish(&[], false)
        .await?
    {
        return Err(failure("actual identical repeated setup failed"));
    }
    config_source.recheck(&config_path)?;
    if Config::load(&config_path)?.projects[0].id != project_id {
        return Err(failure(
            "actual setup duplicated/rekeyed the canonical project",
        ));
    }
    let mut service = repeat.clone();
    service.push("--serve-unit".into());
    if !Terminal::setup(&binary, &service)?
        .finish(&["no\n"], false)
        .await?
    {
        return Err(failure("actual optional unit decline failed"));
    }
    if unit.exists() || state.exists() {
        return Err(failure("unit decline changed unit or identity"));
    }
    if !Terminal::setup(&binary, &service)?
        .finish(&["yes\n", "no\n"], false)
        .await?
    {
        return Err(failure("actual owned unit install/start-decline failed"));
    }
    private_file(&unit)?;
    if state.exists() || socket.exists() {
        return Err(failure("declined service start created a daemon/identity"));
    }
    let unit_source = goatr::setup::ExpectedFile::capture(&unit)?;
    // Only in the already required disposable manager: a real foreign override must never run.
    let dropins = unit.with_file_name("goatr.service.d");
    fs::DirBuilder::new().mode(0o700).create(&dropins)?;
    let dropins_source = identity(&dropins)?;
    let override_path = dropins.join("override.conf");
    let canary = root.join("foreign-executable-ran");
    let touch = std::env::split_paths(
        &std::env::var_os("PATH").ok_or_else(|| failure("disposable PATH unavailable"))?,
    )
    .map(|dir| dir.join("touch"))
    .find(|path| goatr::config::executable(path).is_ok())
    .ok_or_else(|| {
        prerequisite("protected installed coreutils touch required for actual override canary")
    })?;
    let word = |path: &Path| -> io::Result<String> {
        let path = path
            .to_str()
            .ok_or_else(|| failure("canary path not UTF-8"))?;
        Ok(format!(
            "\"{}\"",
            path.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
                .replace('$', "$$")
        ))
    };
    let command = format!("{} {}", word(&touch)?, word(&canary)?);
    let content = format!(
        "[Service]\nExecStart=\nExecStart={command}\nExecStartPre={command}\nExecStartPost={command}\n"
    );
    let override_lock = goatr::setup::SetupLock::acquire(&dropins)?;
    override_lock.publish(
        &override_path,
        &goatr::setup::ExpectedFile::Absent,
        content.as_bytes(),
    )?;
    let override_source = goatr::setup::ExpectedFile::capture(&override_path)?;
    let lock_path = dropins.join(".setup.lock");
    let lock_source = goatr::setup::ExpectedFile::capture(&lock_path)?;
    for reload in [false, true] {
        if reload {
            systemd(&["daemon-reload"]).await?;
        }
        if Terminal::setup(&binary, &service)?
            .finish(&["yes\n", "yes\n"], false)
            .await?
        {
            return Err(failure(
                "actual setup accepted an unmanaged effective override",
            ));
        }
        if canary.exists() || state.exists() || socket.exists() {
            return Err(failure(
                "foreign ExecStart/Pre/Post ran before setup health rejection",
            ));
        }
        unit_source.recheck(&unit)?;
    }
    override_source.recheck(&override_path)?;
    fs::remove_file(&override_path)?;
    drop(override_lock);
    lock_source.recheck(&lock_path)?;
    fs::remove_file(&lock_path)?;
    if identity(&dropins)? != dropins_source {
        return Err(failure("owned override directory changed; cleanup refused"));
    }
    fs::remove_dir(&dropins)?;
    systemd(&["daemon-reload"]).await?;
    if !Terminal::setup(&binary, &service)?
        .finish(&["yes\n", "yes\n"], false)
        .await?
    {
        return Err(failure(
            "actual separately consented unit start/readiness failed",
        ));
    }
    let pid = systemd(&["show", "goatr.service", "--property=MainPID", "--value"])
        .await?
        .trim()
        .parse::<u32>()
        .map_err(|_| failure("actual owned unit PID unavailable"))?;
    let process = herdr::process_evidence(pid)?;
    if fs::read_link(format!("/proc/{pid}/exe"))? != binary {
        return Err(failure("actual unit did not exec the packaged binary"));
    }
    let expected_argv=vec![binary.to_string_lossy().into_owned(),"serve".into(),"--config".into(),config_path.to_string_lossy().into_owned()];
    if goatr::codex_relay::process_arguments(&process)?!=expected_argv
        || !goatr::codex_relay::capability_state(&fs::read_to_string(format!("/proc/{pid}/status"))?)?.zero() {
        return Err(failure("ordinary disposable setup must retain direct v1 argv and all four zero capability sets; Wake requires explicit qualified setup context"));
    }
    let before = state_objects(&state)?;
    let mut identity_source = goatr::setup::ExpectedFile::capture(&state.join("identity.keys"))?;
    let marker_source = goatr::setup::ExpectedFile::capture(&state.join("identity.initialized"))?;
    let endpoint = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?))
        .bind()
        .await
        .map_err(io::Error::other)?;
    let b = binary.clone();
    let mut uri = tokio::task::spawn_blocking(move || terminal_pair(&b, None, None,None))
        .await
        .map_err(io::Error::other)??;
    let invite =
        PairInvite::parse(&uri).map_err(|_| failure("actual private pair URI rejected"))?;
    unsafe { uri.as_bytes_mut().fill(0) };
    let host = invite.host.clone();
    let mut phone = connect(&endpoint, &host, Some(invite)).await?;
    phone.host().await?;
    let device = phone.hello.device_id.clone();
    if !cli(&binary, "devices", None).await?.contains(&device.0) {
        return Err(failure(
            "actual unchanged-default devices CLI missed real paired peer",
        ));
    }
    if !Terminal::setup(&binary, &service)?
        .finish(&["yes\n", "yes\n"], false)
        .await?
    {
        return Err(failure("actual owned repeated setup/unit reuse failed"));
    }
    if herdr::process_evidence(pid)? != process || state_objects(&state)? != before {
        return Err(failure(
            "repeat setup replaced owned process/identity objects",
        ));
    }
    config_source.recheck(&config_path)?;
    unit_source.recheck(&unit)?;
    identity_source.recheck(&state.join("identity.keys"))?;
    marker_source.recheck(&state.join("identity.initialized"))?;
    if let goatr::setup::ExpectedFile::Owned { bytes, .. } = &mut identity_source {
        bytes.fill(0);
    }
    cli(&binary, "revoke", Some(&device)).await?;
    if !cli(&binary, "devices", None).await?.is_empty() {
        return Err(failure("actual default-admin revoke did not remove peer"));
    }
    phone.connection.close(0u32.into(), b"revoked");
    if connect(&endpoint, &host, None).await.is_ok() {
        return Err(failure("actual revoked disposable peer authenticated"));
    }
    if herdr::process_evidence(pid)? != process {
        return Err(failure("unit process epoch changed before owned cleanup"));
    }
    unit_source.recheck(&unit)?;
    systemd(&["stop", "goatr.service"]).await?;
    goatr::close_endpoint(&endpoint)
        .await
        .map_err(io::Error::other)?;
    write_json(
        &evidence.join(format!("setup-service-{}.json", run.0)),
        &json!({"gate":"actual-setup-service","status":"pass","runId":run,"uid":uid(),"package":binary,"packageSha256":binary_sha256(&binary)?,"lockSha256":transport::lock_sha256(),"unit":identity(&unit)?,"process":process,"defaultAdmin":socket,"projectId":project_id,"checks":["actual absent config/decline/cancel; zero key/unit effects","explicit scoped consent and canonical stable project dedup","optional unit decline and separately installed/start-declined unit","actual protected devices.list health with exact packaged PID/argv/cwd","real default pair/hello/host.snapshot/devices/revoke","repeat config/unit byte+inode identity and no rekey/process replacement"],"retained":"owned config/records/unit/identity/receipts retained after exact proven unit stop; no product removal/GC/background claim"}),
    )?;
    println!(
        "{}",
        json!({"gate":"actual-setup-service","status":"pass","runId":run,"evidence":evidence})
    );
    Ok(true)
}
