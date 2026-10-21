// SPDX-License-Identifier: GPL-3.0-or-later
//! Actual packaged/provider acceptance. Missing native prerequisites are blocked, never fixtures.
use super::{
    binary_sha256, boot, failure, identity, prerequisite, start, terminal_pair, write_json,
};
use clap::{Arg, ArgAction, ArgGroup, Command, value_parser};
use goatr::{
    config::{CodexOwner, Config, ConfiguredProject},
    herdr::{self, Client, PaneReadIdentity, ProcessEvidence, Snapshot},
    pairing::{self, PairInvite, hex, private_dir, private_file, random32, uid},
    protocol::*,
    storage::{CreationStageRecord, ServerEvidence},
    transport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use rusqlite::{Connection, OpenFlags};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "camera.rs"]
mod camera;
#[path = "dev_continuity.rs"]
mod dev_continuity;
#[path = "herdr_report_capture.rs"]
mod herdr_report_capture;
#[path = "paging_proof.rs"]
mod paging_proof;
#[path = "setup_acceptance.rs"]
mod setup_acceptance;
#[path = "ui.rs"]
mod ui;
const APP: &str = "io.github.trevarj.goatr.debug";
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Lease {
    schema: u32,
    run_id: String,
    owner_uid: u32,
    root: PathBuf,
    serial: String,
    avd_name: String,
    avd_home: PathBuf,
    android_user_home: PathBuf,
    avd_path: PathBuf,
    process_identity: EmulatorProcess,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EmulatorProcess {
    linux_boot_id: String,
    pid: u32,
    start_ticks: String,
    executable: PathBuf,
}
async fn sdk_adb() -> io::Result<PathBuf> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let sdk = PathBuf::from(
        std::env::var_os("ANDROID_HOME")
            .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "pinned SDK and existing ADB server required",
                )
            })?,
    );
    let path = sdk.join("platform-tools/adb");
    goatr::config::executable(&path)?;
    // Read-only smart-socket version, BEFORE invoking adb: no implicit server start/replace.
    let mut server = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 5037)),
    )
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "existing loopback ADB server unavailable; runner never starts one",
        )
    })??;
    tokio::time::timeout(Duration::from_secs(1), async {
        server.write_all(b"000chost:version").await?;
        let mut status = [0; 4];
        server.read_exact(&mut status).await?;
        if &status != b"OKAY" {
            return Err(failure("existing ADB smart-socket handshake rejected"));
        }
        server.read_exact(&mut status).await?;
        let len =
            usize::from_str_radix(std::str::from_utf8(&status).map_err(io::Error::other)?, 16)
                .map_err(io::Error::other)?;
        if len > 16 {
            return Err(failure("ADB version response exceeded bound"));
        }
        let mut version = [0; 16];
        server.read_exact(&mut version[..len]).await?;
        if u32::from_str_radix(
            std::str::from_utf8(&version[..len]).map_err(io::Error::other)?,
            16,
        )
        .map_err(io::Error::other)?
            != 41
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "existing ADB protocol differs from pinned SDK; no server kill/replacement",
            ));
        }
        Ok::<(), io::Error>(())
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "existing ADB server version deadline",
        )
    })??;
    Ok(path)
}
fn canonical_run_uuid(value: &str) -> bool {
    Uuid::new(value.to_owned()).is_ok()
}
fn supported_console_port(serial: &str) -> Option<u16> {
    let port = serial.strip_prefix("emulator-")?.parse::<u16>().ok()?;
    if (5554..=5682).contains(&port) && port % 2 == 0 && serial == format!("emulator-{port}") {
        Some(port)
    } else {
        None
    }
}
fn lease_console_port(l: &Lease) -> io::Result<u16> {
    supported_console_port(&l.serial)
        .ok_or_else(|| failure("owned emulator serial is not a canonical supported console port"))
}
fn emulator_runtime_dir(l: &Lease) -> io::Result<PathBuf> {
    if !canonical_run_uuid(&l.run_id) {
        return Err(failure("owned emulator run UUID is not canonical"));
    }
    Ok(l.root.join(format!("camera-runtime-{}", &l.run_id[..8])))
}
fn adb_selector(l: &Lease) -> io::Result<[&str; 6]> {
    lease_console_port(l)?;
    Ok(["-H", "127.0.0.1", "-P", "5037", "-s", &l.serial])
}
fn lease(path: &Path) -> io::Result<Lease> {
    let source = goatr::setup::ExpectedFile::capture(path)?;
    let bytes = source
        .bytes()
        .ok_or_else(|| failure("existing owned emulator lease unavailable"))?;
    if bytes.len() > 65536 {
        return Err(failure("owned emulator lease exceeds bound"));
    }
    let l: Lease = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if l.schema != 1
        || !canonical_run_uuid(&l.run_id)
        || l.owner_uid != uid()
        || supported_console_port(&l.serial).is_none()
        || path.parent() != Some(l.root.as_path())
        || fs::canonicalize(path)? != path
        || !l.root.is_absolute()
        || fs::canonicalize(&l.root)? != l.root
    {
        return Err(failure(
            "owned emulator lease is not a canonical selected resource",
        ));
    }
    private_dir(&l.root, false)?;
    for p in [&l.avd_home, &l.android_user_home, &l.avd_path] {
        if !p.starts_with(&l.root) || fs::canonicalize(p)? != *p || fs::metadata(p)?.uid() != uid()
        {
            return Err(failure("emulator AVD/resource ownership mismatch"));
        }
    }
    source.recheck(path)?;
    Ok(l)
}
fn selected_lease(args: &clap::ArgMatches, consumer: &str) -> io::Result<Lease> {
    let path = args
        .get_one::<PathBuf>("emulator-owner")
        .ok_or_else(|| failure("selected Android consumer requires an owned emulator lease"))?;
    let l = lease(path)?;
    selected_serial(
        args.get_one::<String>("serial").map(String::as_str),
        &l,
        consumer,
    )?;
    Ok(l)
}
fn selected_serial(serial: Option<&str>, l: &Lease, consumer: &str) -> io::Result<()> {
    if serial != Some(l.serial.as_str()) {
        return Err(failure(&format!(
            "{consumer} requires explicit --serial matching its selected owned emulator lease"
        )));
    }
    Ok(())
}
fn emulator_matches(l: &Lease) -> bool {
    boot().is_ok_and(|b| b == l.process_identity.linux_boot_id)
        && start(l.process_identity.pid).is_ok_and(|s| s == l.process_identity.start_ticks)
        && fs::metadata(format!("/proc/{}", l.process_identity.pid)).is_ok_and(|m| m.uid() == uid())
        && fs::read_link(format!("/proc/{}/exe", l.process_identity.pid))
            .is_ok_and(|p| p == l.process_identity.executable)
}
fn sdk_camera_operation(command: &tokio::process::Command) -> Option<&'static str> {
    let command = command.as_std();
    if Path::new(command.get_program())
        .file_name()
        .and_then(|name| name.to_str())
        != Some("adb")
    {
        return None;
    }
    let mut args = command.get_args();
    for expected in ["-H", "127.0.0.1", "-P", "5037", "-s"] {
        if args.next() != Some(std::ffi::OsStr::new(expected)) {
            return None;
        }
    }
    // This is only a closed SDK error-classification shape check. Lease authority is
    // established before the command is created and never by this parsed selector.
    supported_console_port(args.next()?.to_str()?)?;
    let first = args.next()?.to_str()?;
    let second = args.next().and_then(|arg| arg.to_str());
    let third = args.next().and_then(|arg| arg.to_str());
    match (first, second, third) {
        ("emu", Some("virtualscene-image"), Some("wall")) => Some("console_poster"),
        ("shell", Some("dumpsys"), Some("media.camera")) => Some("camera_dump"),
        ("shell", Some("am"), Some("instrument")) => Some("instrumentation"),
        ("shell", Some("am"), Some("broadcast")) => Some("camera_phase_broadcast"),
        ("shell", Some("am"), Some("force-stop")) => Some("owned_target_stop"),
        ("shell", Some("am"), Some("get-current-user")) => Some("guest_user"),
        ("shell", Some("pm"), Some("grant" | "revoke" | "clear-permission-flags"))
        | ("shell", Some("dumpsys"), Some("package")) => Some("permissions"),
        ("shell", Some("ps"), Some("-A")) => Some("pid_stat"),
        ("shell", Some("cat"), Some(path))
            if path.starts_with("/proc/") && path.ends_with("/stat")
                || path == "/proc/sys/kernel/random/boot_id" =>
        {
            Some("pid_stat")
        }
        ("shell", Some("cmd"), Some("sensor_privacy")) => {
            match args.next().and_then(|arg| arg.to_str()) {
                Some("help") | None => Some("sensor_policy_help"),
                Some("enable" | "disable") => Some("sensor_policy_set"),
                _ => None,
            }
        }
        ("install", Some("-r"), Some(_)) => Some("apk_install"),
        ("exec-out", Some("screencap"), Some("-p")) => Some("screen_capture"),
        ("emu", Some("avd"), Some("name"))
        | ("get-state", None, None)
        | ("shell", Some("getprop"), Some("ro.build.version.sdk" | "ro.product.cpu.abi"))
        | ("shell", Some("getconf"), Some("PAGE_SIZE")) => Some("emulator_identity"),
        _ => None,
    }
}
fn command_failure(operation: Option<&str>, out: &std::process::Output, limit: usize) -> io::Error {
    if operation == Some("apk_install") {
        return io::Error::other(ApkInstallFailure {
            category: install_failure_category(&out.stdout, &out.stderr),
            exit: out.status.code(),
        });
    }
    if let Some(operation) = operation {
        io::Error::other(format!(
            "SDK adb {operation} failed or exceeded output bound: exit {:?}, stdout {}/{limit} bytes, stderr {} bytes; raw output omitted",
            out.status.code(),
            out.stdout.len(),
            out.stderr.len()
        ))
    } else {
        failure("actual command failed or output bound exceeded")
    }
}
#[derive(Debug)]
struct ApkInstallFailure {
    category: &'static str,
    exit: Option<i32>,
}
impl std::fmt::Display for ApkInstallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SDK apk_install failed: category={}, exit={:?}; raw output omitted",
            self.category, self.exit
        )
    }
}
impl std::error::Error for ApkInstallFailure {}
fn install_failure_category(stdout: &[u8], stderr: &[u8]) -> &'static str {
    const KNOWN: &[&str] = &[
        "INSTALL_FAILED_UPDATE_INCOMPATIBLE",
        "INSTALL_FAILED_TEST_ONLY",
        "INSTALL_FAILED_VERSION_DOWNGRADE",
        "INSTALL_FAILED_INSUFFICIENT_STORAGE",
        "INSTALL_FAILED_INVALID_APK",
        "INSTALL_FAILED_NO_MATCHING_ABIS",
        "INSTALL_FAILED_OLDER_SDK",
        "INSTALL_FAILED_MISSING_SHARED_LIBRARY",
        "INSTALL_FAILED_SHARED_USER_INCOMPATIBLE",
        "INSTALL_FAILED_DUPLICATE_PERMISSION",
        "INSTALL_FAILED_USER_RESTRICTED",
        "INSTALL_FAILED_ABORTED",
        "INSTALL_FAILED_INTERNAL_ERROR",
        "INSTALL_FAILED_VERIFICATION_FAILURE",
        "INSTALL_PARSE_FAILED_NO_CERTIFICATES",
        "INSTALL_PARSE_FAILED_INCONSISTENT_CERTIFICATES",
        "INSTALL_PARSE_FAILED_BAD_MANIFEST",
        "INSTALL_PARSE_FAILED_MANIFEST_MALFORMED",
        "INSTALL_PARSE_FAILED_NOT_APK",
        "INSTALL_PARSE_FAILED_UNEXPECTED_EXCEPTION",
    ];
    let mut found = None;
    for bytes in [stdout, stderr] {
        for token in bytes[..bytes.len().min(65536)]
            .split(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            if let Some(&known) = KNOWN.iter().find(|known| known.as_bytes() == token) {
                if found.is_some_and(|previous| previous != known) {
                    return "other";
                }
                found = Some(known);
            }
        }
    }
    found.unwrap_or("other")
}
// Preserve native status for read-only inspection; normal/effectful commands use strict output().
async fn capture(
    mut c: tokio::process::Command,
    timeout: u64,
    limit: usize,
) -> io::Result<std::process::Output> {
    c.kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(timeout), c.output())
        .await
        .map_err(|_| failure("actual command deadline exceeded"))??;
    if out.stdout.len() > limit {
        return Err(command_failure(sdk_camera_operation(&c), &out, limit));
    }
    Ok(out)
}
async fn output(c: tokio::process::Command, timeout: u64, limit: usize) -> io::Result<Vec<u8>> {
    let camera_operation = sdk_camera_operation(&c);
    let out = capture(c, timeout, limit).await?;
    if !out.status.success() {
        return Err(command_failure(camera_operation, &out, limit));
    }
    Ok(out.stdout)
}
async fn adb(l: &Lease, args: &[&str]) -> io::Result<String> {
    let sdk = sdk_adb().await?;
    if !emulator_matches(l) {
        return Err(failure("owned emulator process identity changed"));
    }
    let mut c = tokio::process::Command::new(sdk);
    c.args(adb_selector(l)?).args(args);
    String::from_utf8(output(c, 15, 1024 * 1024).await?).map_err(io::Error::other)
}
async fn check_emulator(l: &Lease) -> io::Result<Value> {
    if !emulator_matches(l) {
        return Err(failure(
            "existing owned emulator is not live; prepare only its already-owned stopped AVD explicitly",
        ));
    }
    if adb(l, &["emu", "avd", "name"]).await?.lines().next() != Some(l.avd_name.as_str())
        || adb(l, &["get-state"]).await?.trim() != "device"
    {
        return Err(failure(
            "serial/console AVD does not join the retained emulator owner",
        ));
    }
    let api = adb(l, &["shell", "getprop", "ro.build.version.sdk"]).await?;
    let abi = adb(l, &["shell", "getprop", "ro.product.cpu.abi"]).await?;
    let page = adb(l, &["shell", "getconf", "PAGE_SIZE"]).await?;
    if api.trim() != "34" || abi.trim() != "x86_64" {
        return Err(failure(
            "existing owned emulator API/ABI differs from the approved resource",
        ));
    }
    Ok(
        json!({"runId":l.run_id,"serial":l.serial,"api":api.trim(),"abi":abi.trim(),"pageSize":page.trim(),"process":{"pid":l.process_identity.pid,"start":l.process_identity.start_ticks,"boot":l.process_identity.linux_boot_id,"executable":l.process_identity.executable},"limit":"4096-byte runtime is NOT a real16KiB pass"}),
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedSerialAvailability {
    Absent,
    Occupied,
    Uncertain,
}
fn selected_serial_availability(
    serial: &str,
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> SelectedSerialAvailability {
    if success {
        return SelectedSerialAvailability::Occupied;
    }
    let missing = format!("error: device '{serial}' not found");
    if stdout.is_empty()
        && stderr.len() <= 128
        && std::str::from_utf8(stderr)
            .ok()
            .is_some_and(|stderr| stderr.trim() == missing)
    {
        SelectedSerialAvailability::Absent
    } else {
        SelectedSerialAvailability::Uncertain
    }
}
async fn selected_serial_is_absent(l: &Lease) -> io::Result<()> {
    let sdk = sdk_adb().await?;
    let mut command = tokio::process::Command::new(sdk);
    command
        .args(adb_selector(l)?)
        .arg("get-state")
        .kill_on_drop(true);
    let output = capture(command, 10, 128).await?;
    match selected_serial_availability(
        &l.serial,
        output.status.success(),
        &output.stdout,
        &output.stderr,
    ) {
        SelectedSerialAvailability::Absent => Ok(()),
        SelectedSerialAvailability::Occupied => Err(prerequisite(
            "selected emulator serial is already occupied; launch refused",
        )),
        SelectedSerialAvailability::Uncertain => Err(prerequisite(
            "selected emulator serial availability is uncertain; launch refused",
        )),
    }
}
async fn prepare(path: &Path, l: &Lease, evidence: &Path) -> io::Result<bool> {
    let lock = goatr::setup::SetupLock::acquire(&l.root)?;
    let expected = goatr::setup::ExpectedFile::capture(path)?;
    selected_serial_is_absent(l).await?;
    if fs::metadata(format!("/proc/{}", l.process_identity.pid)).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "prepare requires the selected recorded emulator process to be stopped; no live/reused PID adoption",
        ));
    }
    let runtime = emulator_runtime_dir(l)?;
    for parent in [&runtime, &l.root.join("tmp")] {
        private_dir(parent, false)?;
    }
    let sdk = PathBuf::from(
        std::env::var_os("ANDROID_HOME")
            .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
            .ok_or_else(|| failure("pinned emulator SDK/image unavailable"))?,
    );
    let executable = sdk.join("emulator/emulator");
    goatr::config::executable(&executable)?;
    expected.recheck(path)?;
    if lease(path)? != *l {
        return Err(failure(
            "selected emulator lease changed before prepare; launch refused",
        ));
    }
    let console_port = lease_console_port(l)?.to_string();
    let mut c = std::process::Command::new(&executable);
    c.args([
        "-avd",
        &l.avd_name,
        "-port",
        &console_port,
        "-no-window",
        "-no-audio",
        "-no-metrics",
        "-crash-report-mode",
        "disabled",
        "-gpu",
        "swiftshader",
        "-no-snapshot",
        "-camera-back",
        "virtualscene",
        "-camera-front",
        "none",
        "-memory",
        "2048",
        "-cores",
        "2",
    ])
    .env("ANDROID_AVD_HOME", &l.avd_home)
    .env("ANDROID_USER_HOME", &l.android_user_home)
    .env("ANDROID_EMULATOR_HOME", &l.android_user_home)
    .env("XDG_RUNTIME_DIR", &runtime)
    .env("TMPDIR", l.root.join("tmp"))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let mut child = c.spawn()?;
    let parent = herdr::process_evidence(child.id())?;
    write_json(
        &evidence.join(format!("emulator-launch-{}.json", Uuid::random().0)),
        &json!({"gate":"owned-emulator-prepare","stage":"exact owned launcher before readiness","lease":path,"root":identity(&l.root)?,"launcher":executable,"process":parent,"expectedQemu":l.process_identity.executable,"AVD":l.avd_path,"serial":l.serial}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let process = loop {
        let mut candidates = vec![child.id()];
        if herdr::process_evidence(parent.pid).is_ok_and(|p| p == parent)
            && let Ok(children) =
                fs::read_to_string(format!("/proc/{}/task/{}/children", parent.pid, parent.pid))
        {
            for value in children.split_whitespace() {
                if candidates.len() == 32 {
                    return Err(failure("owned emulator launcher child bound"));
                }
                candidates.push(value.parse::<u32>().map_err(io::Error::other)?);
            }
        }
        let mut found = None;
        for pid in candidates {
            if fs::read_link(format!("/proc/{pid}/exe"))
                .is_ok_and(|exe| exe == l.process_identity.executable)
            {
                let p = herdr::process_evidence(pid)?;
                if found.replace(p).is_some() {
                    return Err(failure(
                        "multiple retained emulator ELF descendants; no adoption",
                    ));
                }
            }
        }
        if let Some(p) = found {
            break p;
        }
        if child.try_wait()?.is_some() || Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "owned launcher did not yield its exact retained QEMU ELF; launch ownership evidence retained",
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let mut record: Value = serde_json::from_slice(
        expected
            .bytes()
            .ok_or_else(|| failure("emulator ownership source disappeared"))?,
    )
    .map_err(io::Error::other)?;
    let old = record["processIdentity"].clone();
    record["previousProcesses"]
        .as_array_mut()
        .ok_or_else(|| failure("existing emulator lease history unavailable"))?
        .push(old);
    record["processIdentity"] = json!({"linuxBootId":process.linux_boot_id,"pid":process.pid,"startTicks":process.start_ticks.to_string(),"executable":l.process_identity.executable});
    record["state"] = json!("preparing");
    lock.publish(
        path,
        &expected,
        &serde_json::to_vec_pretty(&record).map_err(io::Error::other)?,
    )?;
    let active = lease(path)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(device) = check_emulator(&active).await
            && adb(&active, &["shell", "getprop", "sys.boot_completed"])
                .await?
                .trim()
                == "1"
        {
            let source = goatr::setup::ExpectedFile::capture(path)?;
            record["state"] = json!("booted");
            lock.publish(
                path,
                &source,
                &serde_json::to_vec_pretty(&record).map_err(io::Error::other)?,
            )?;
            println!(
                "{}",
                json!({"gate":"owned-emulator-prepare","status":"pass","lease":path,"device":device})
            );
            return Ok(true);
        }
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "owned same-AVD boot blocked; exact process/lease retained for inspection",
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
struct Desktop {
    child: Child,
    process: ProcessEvidence,
}
impl Desktop {
    fn spawn(mut c: std::process::Command) -> io::Result<Self> {
        use std::os::unix::process::CommandExt;
        #[cfg(target_os = "linux")]
        // SAFETY: the shared helper uses only stack values and async-signal-safe syscalls.
        unsafe {
            // Controller inspection authority must not become desktop/provider authority.
            c.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
        let child = c
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Retained native work must not share the controller terminal's cancellation group.
            .process_group(0)
            .spawn()?;
        let process = herdr::process_evidence(child.id())?;
        Ok(Self { child, process })
    }
    fn recheck(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_some() {
            return Err(failure(
                "owned desktop Herdr child exited; no desktop termination or replacement",
            ));
        }
        if herdr::process_evidence(self.process.pid)? != self.process {
            return Err(failure(
                "owned desktop PID/start/boot/namespace epoch changed",
            ));
        }
        Ok(())
    }
}
/// Only the companion is stopped by this gate. Desktop creation resources are retained,
/// never rolled back after unknown outcomes and never lifetime-coupled to companion exit.
struct ServeLaunch<'a> {
    diagnostic_capture: bool,
    codex_startup_terminal_capture: bool,
    herdr_report_capture: Option<&'a herdr_report_capture::Capture>,
    preserve_xdg_config_home: bool,
}
struct Companion {
    child: Child,
    process: ProcessEvidence,
    state_root: PathBuf,
    binary: PathBuf,
    guard_drain: Option<GuardDrain>,
    omp_capture_budget: Option<herdr_report_capture::Budget>,
    codex_cleanup_signal: CleanupSignal,
    codex_cleanup_active: bool,
    codex_startup_terminal: Option<CodexStartupTerminalResult>,
    codex_startup_journal: Option<CodexStartupJournalContext>,
    codex_startup_journal_admission: Option<CodexStartupJournalAdmission>,
}
const COMPANION_CLEANUP_BUDGET: Duration = Duration::from_secs(15);
#[derive(Clone, Copy, Default)]
struct CleanupSignal {
    attempted: bool,
    sent: bool,
}
enum GuardDrain {
    Codex {
        worker: std::thread::JoinHandle<()>,
        progress: Arc<CodexDrainProgress>,
    },
    Other(std::thread::JoinHandle<()>),
}
#[derive(Clone, Copy, Default)]
struct CodexDrainSummary {
    matching_prefix: u8,
    accepted: u8,
    rejected: u8,
    oversized: u8,
    eof: bool,
    completed: bool,
    output_failed: bool,
    termination: CodexDrainTermination,
    read_error: Option<(&'static str, Option<i32>)>,
    journal_availability: &'static str,
    journal_reason: Option<&'static str>,
    journal_accepted: bool,
}
#[derive(Clone, Copy)]
struct CodexStartupTerminalResult {
    guard: &'static str,
    code: &'static str,
    io: &'static str,
    errno: i32,
}
#[derive(Clone, Copy)]
struct CodexStartupJournalPins {
    state_root: (u64, u64),
    database: (u64, u64),
}
struct CodexStartupJournalAdmission {
    action: Uuid,
    pins: Result<CodexStartupJournalPins, &'static str>,
}
#[derive(Clone)]
struct CodexStartupJournalContext {
    receipt: Receipt,
    settlement_cutoff: u64,
    owner: CodexOwner,
    server: Uuid,
    source: ServerEvidence,
    state_root: (u64, u64),
    database: (u64, u64),
}
#[derive(Clone, Copy, Default)]
enum CodexDrainTermination {
    #[default]
    Reading,
    Eof,
    ReadError,
    ThreadPanic,
}
impl CodexDrainTermination {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Eof => "eof",
            Self::ReadError => "read_error",
            Self::ThreadPanic => "thread_panic",
        }
    }
}
#[derive(Default)]
struct CodexDrainProgress(Mutex<CodexDrainSummary>);
impl CodexDrainProgress {
    fn update(&self, update: impl FnOnce(&mut CodexDrainSummary)) {
        // Only finite field updates occur under this lock, never pipe or stdout I/O.
        let mut summary = self.0.lock().unwrap_or_else(|error| error.into_inner());
        update(&mut summary);
    }
    fn snapshot(&self) -> CodexDrainSummary {
        // Recover the retained observations after poison rather than inventing zero counts.
        *self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
    fn emit_startup_terminal_after_drain(&self, result: CodexStartupTerminalResult) {
        let summary = self.snapshot();
        if summary.accepted >= 8 || summary.output_failed {
            return;
        }
        if writeln!(
            std::io::stdout().lock(),
            "{}",
            json!({"gate":"owned-codex-invalidation-capture","phase":"terminal_visible_signature","guard":result.guard,"code":result.code,"io":result.io,"errno":result.errno,"report":Value::Null}),
        )
        .is_ok()
        {
            self.update(|summary| summary.accepted += 1);
        } else {
            self.update(|summary| {
                summary.rejected = summary.rejected.saturating_add(1);
                summary.output_failed = true;
                summary.completed = false;
            });
        }
    }
    fn journal_unavailable(&self, reason: &'static str) {
        self.update(|summary| {
            summary.journal_availability = "unavailable";
            summary.journal_reason = Some(reason);
            summary.journal_accepted = false;
        });
    }
    fn emit_startup_journal_after_drain(&self, report: Value) {
        let summary = self.snapshot();
        if summary.accepted >= 8 {
            self.journal_unavailable("capture_capacity");
            return;
        }
        if summary.output_failed {
            self.journal_unavailable("journal_unavailable");
            return;
        }
        if writeln!(std::io::stdout().lock(), "{report}").is_ok() {
            self.update(|summary| {
                summary.accepted += 1;
                summary.journal_availability = "available";
                summary.journal_reason = Some("none");
                summary.journal_accepted = true;
            });
        } else {
            self.update(|summary| {
                summary.journal_availability = "unavailable";
                summary.journal_reason = Some("journal_unavailable");
                summary.journal_accepted = false;
                summary.output_failed = true;
                summary.completed = false;
            });
        }
    }
}
impl CodexDrainSummary {
    fn emit(
        &self,
        cleanup_context: &'static str,
        signal: CleanupSignal,
        cleanup_result: &'static str,
    ) {
        println!(
            "{}",
            json!({
                "gate":"owned-codex-invalidation-capture",
                "phase":"capture_drain",
                "matchingPrefix":self.matching_prefix,
                "accepted":self.accepted,
                "rejected":self.rejected,
                "oversized":self.oversized,
                "eof":self.eof,
                "completed":self.completed,
                "outputFailure":self.output_failed,
                "termination":self.termination.as_str(),
                "readError":self.read_error.map(|(kind, errno)| json!({"io":kind,"errno":errno})),
                "journalAvailability":self.journal_availability,
                "journalReason":self.journal_reason,
                "journalAccepted":self.journal_accepted,
                "cleanupResult":cleanup_result,
                "status":if self.completed { "completed" } else { "capture_incomplete" },
                "cleanupContext":cleanup_context,
                "supervisorSigtermAttempted":signal.attempted,
                "supervisorSigtermSent":signal.sent,
            }),
        );
    }
}
struct CodexRelayJournalJoin {
    unit: String,
    manifest_path: PathBuf,
    boot: Uuid,
}

fn retained_codex_relay_join(
    state_root: &Path,
    context: &CodexStartupJournalContext,
    deadline: Instant,
) -> Result<CodexRelayJournalJoin, &'static str> {
    if Instant::now() >= deadline {
        return Err("cleanup_exhausted");
    }
    let database = state_root.join("state.sqlite3");
    if private_dir(state_root, false).is_err() || private_file(&database).is_err() {
        return Err("state_unavailable");
    }
    let root = fs::symlink_metadata(state_root).map_err(|_| "state_unavailable")?;
    let current = fs::symlink_metadata(&database).map_err(|_| "state_unavailable")?;
    if (root.dev(), root.ino()) != context.state_root
        || (current.dev(), current.ino()) != context.database
    {
        return Err("state_unavailable");
    }
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&database)
    {
        Ok(file) => file,
        Err(_) => return Err("state_unavailable"),
    };
    let before = match file.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return Err("state_unavailable"),
    };
    if private_file(&database).is_err()
        || !before.is_file()
        || before.nlink() != 1
        || before.len() > 64 * 1024 * 1024
        || (before.dev(), before.ino()) != context.database
    {
        return Err("state_unavailable");
    }
    let database_read = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    );
    let Ok(database_read) = database_read else {
        return Err("state_unavailable");
    };
    if database_read.busy_timeout(Duration::ZERO).is_err() {
        return Err("state_unavailable");
    }
    if Instant::now() >= deadline {
        return Err("cleanup_exhausted");
    }
    let mut query = match database_read.prepare(
        "SELECT receipt, stages FROM receipts JOIN creation USING (peer, action) WHERE action=?1",
    ) {
        Ok(query) => query,
        Err(_) => return Err("state_unavailable"),
    };
    let mut rows = match query.query([&context.receipt.action_id.0]) {
        Ok(rows) => rows,
        Err(_) => return Err("state_unavailable"),
    };
    let first: (String, String) = match rows.next() {
        Ok(Some(row)) => match (row.get(0), row.get(1)) {
            (Ok(receipt), Ok(stages)) => (receipt, stages),
            _ => return Err("state_unavailable"),
        },
        _ => return Err("state_unjoined"),
    };
    if !matches!(rows.next(), Ok(None)) {
        return Err("state_unjoined");
    }
    let after = match file.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return Err("state_unavailable"),
    };
    let root_after = fs::symlink_metadata(state_root).map_err(|_| "state_unavailable")?;
    let current_after = fs::symlink_metadata(&database).map_err(|_| "state_unavailable")?;
    if private_file(&database).is_err()
        || (root_after.dev(), root_after.ino()) != context.state_root
        || (current_after.dev(), current_after.ino()) != context.database
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || before.len() != after.len()
    {
        return Err("state_unavailable");
    }
    let Ok(receipt) = serde_json::from_str::<Receipt>(&first.0) else {
        return Err("state_unjoined");
    };
    let Ok(stages) = serde_json::from_str::<Vec<CreationStageRecord>>(&first.1) else {
        return Err("state_unjoined");
    };
    let Some(creation) = receipt.creation.as_ref() else {
        return Err("state_unjoined");
    };
    let Some(binding) = creation.resources.binding_id.as_ref() else {
        return Err("state_unjoined");
    };
    let unit = format!("goatr-codex-relay-{}.service", binding.0);
    let relay: Vec<_> = stages
        .iter()
        .filter(|stage| {
            stage.stage == CreationStage::Relay
                && stage.intent.unit_name.as_deref() == Some(&unit)
                && stage.intent.socket_path.is_some()
        })
        .collect();
    if receipt != context.receipt
        || receipt.method != ActionMethod::SessionCreate
        || receipt.boot_id != context.receipt.boot_id
        || creation.provider != Provider::Codex
        || creation.server_id != context.server
        || relay.len() != 1
        || context.owner.bindings_dir.as_os_str().is_empty()
    {
        return Err("state_unjoined");
    }
    let manifest_path = goatr::codex_relay::manifest_path(&context.owner.bindings_dir, binding);
    let Ok(manifest) = goatr::codex_relay::private_read::<goatr::codex_relay::Manifest>(&manifest_path) else {
        return Err("manifest_unavailable");
    };
    if Instant::now() >= deadline {
        return Err("cleanup_exhausted");
    }
    if manifest.binding_id != *binding
        || manifest.owner_id != context.owner.id
        || manifest.upstream_host != context.owner.socket_host
        || manifest.upstream_agent != context.owner.socket_agent
        || manifest.herdr.server_id != context.server
        || manifest.herdr.evidence != context.source
        || manifest.owner_epoch.linux_boot_id != context.source.linux_boot_id
        || manifest.herdr.workspace_id != *creation.resources.workspace_id.as_ref().ok_or("state_unjoined")?
        || manifest.herdr.pane_id != *creation.resources.pane_id.as_ref().ok_or("state_unjoined")?
        || manifest.herdr.terminal_id != *creation.resources.terminal_id.as_ref().ok_or("state_unjoined")?
        || manifest.expected_thread_id != *creation.resources.native_id.as_ref().ok_or("state_unjoined")?
        || relay[0].intent.socket_path.as_deref() != manifest.listen_host.to_str()
    {
        return Err("manifest_unavailable");
    }
    Ok(CodexRelayJournalJoin {
        unit,
        manifest_path,
        boot: manifest.owner_epoch.linux_boot_id,
    })
}

fn journalctl_path() -> Result<PathBuf, &'static str> {
    for parent in std::env::split_paths(&std::env::var_os("PATH").ok_or("journal_unavailable")?) {
        let candidate = parent.join("journalctl");
        if goatr::config::executable(&candidate).is_ok() {
            return Ok(candidate);
        }
    }
    Err("journal_unavailable")
}

fn journal_hex32(value: &str) -> Option<String> {
    let value = value.replace('-', "");
    (value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

async fn stop_journal_reader(child: &mut tokio::process::Child, deadline: Instant) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), child.wait()).await;
}

async fn retained_codex_startup_journal(
    state_root: &Path,
    binary: &Path,
    context: &CodexStartupJournalContext,
    deadline: Instant,
) -> Result<Value, &'static str> {
    use tokio::io::AsyncReadExt;

    if Instant::now() >= deadline {
        return Err("cleanup_exhausted");
    }
    let state_root = state_root.to_owned();
    let join_context = context.clone();
    let join = match tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        tokio::task::spawn_blocking(move || retained_codex_relay_join(&state_root, &join_context, deadline)),
    )
    .await
    {
        Ok(Ok(Ok(join))) => join,
        Ok(Ok(Err(reason))) => return Err(reason),
        Ok(Err(_)) => return Err("state_unavailable"),
        Err(_) => return Err("cleanup_exhausted"),
    };
    let journalctl = journalctl_path()?;
    if Instant::now() >= deadline {
        return Err("cleanup_exhausted");
    }
    let started = context.receipt.created_at.0;
    let boot = journal_hex32(&join.boot.0).ok_or("journal_invalid")?;
    let expected_command = format!(
        "{} codex-relay --binding {}",
        binary.display(),
        join.manifest_path.display()
    );
    let mut command = tokio::process::Command::new(journalctl);
    command
        .kill_on_drop(true)
        .arg("--user")
        .arg("--user-unit")
        .arg(&join.unit)
        .arg(format!("--boot={boot}"))
        .arg("-n")
        .arg("100")
        .arg(format!("--since=@{}.{:03}", started / 1_000, started % 1_000))
        .arg(format!(
            "--until=@{}.{:03}",
            context.settlement_cutoff / 1_000,
            context.settlement_cutoff % 1_000
        ))
        .arg("--output=json")
        .arg("--no-pager")
        .arg(format!("_UID={}", uid()))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    let mut child = command.spawn().map_err(|_| "journal_unavailable")?;
    let mut stdout = child.stdout.take().ok_or("journal_unavailable")?;
    let mut collected = Vec::new();
    match tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        stdout.take(1_048_577).read_to_end(&mut collected),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => {
            stop_journal_reader(&mut child, deadline).await;
            return Err("journal_unavailable");
        }
        Err(_) => {
            stop_journal_reader(&mut child, deadline).await;
            return Err("cleanup_exhausted");
        }
    }
    if collected.len() > 1_048_576 {
        stop_journal_reader(&mut child, deadline).await;
        return Err("journal_oversized");
    }
    let status = match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), child.wait()).await {
        Ok(Ok(status)) => status,
        _ => {
            stop_journal_reader(&mut child, deadline).await;
            return Err("cleanup_exhausted");
        }
    };
    if !status.success() {
        return Err("journal_unavailable");
    }
    let mut matching = Vec::new();
    let mut service_pid = None;
    let mut invocation = None;
    let mut records = 0u16;
    for line in collected.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
        records = records.saturating_add(1);
        if records > 100 {
            return Err("journal_oversized");
        }
        let value: Value = serde_json::from_slice(line).map_err(|_| "journal_invalid")?;
        if value["_SYSTEMD_USER_UNIT"].as_str() != Some(&join.unit)
            || value["_BOOT_ID"].as_str().and_then(journal_hex32).as_deref() != Some(&boot)
            || value["_UID"].as_str() != Some(&uid().to_string())
        {
            continue;
        }
        let Some(executable) = value["_EXE"].as_str() else {
            continue;
        };
        let Some(command) = value["_CMDLINE"].as_str() else {
            continue;
        };
        if executable != binary.to_string_lossy() || command != expected_command {
            return Err("journal_invalid");
        }
        let pid = value["_PID"]
            .as_str()
            .filter(|value| {
                value.parse::<u32>().is_ok_and(|pid| pid > 0)
                    && value.parse::<u32>().ok().is_some_and(|pid| pid.to_string() == *value)
            })
            .ok_or("journal_invalid")?;
        let invocation_raw = value["_SYSTEMD_INVOCATION_ID"]
            .as_str()
            .ok_or("journal_invalid")?;
        let current_invocation = journal_hex32(invocation_raw)
            .filter(|canonical| canonical == invocation_raw)
            .ok_or("journal_invalid")?;
        if service_pid.replace(pid.to_owned()).is_some_and(|prior| prior != pid)
            || invocation
                .replace(current_invocation.clone())
                .is_some_and(|prior| prior != current_invocation)
        {
            return Err("journal_ambiguous");
        }
        let timestamp = value["__REALTIME_TIMESTAMP"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| value / 1_000)
            .ok_or("journal_invalid")?;
        if timestamp < started || timestamp > context.settlement_cutoff {
            return Err("journal_late");
        }
        if let Some(message) = value["MESSAGE"].as_str()
            && let Some(report) = parse_codex_guard_line(message.as_bytes())
            && report["phase"].as_str() == Some("relay_startup")
        {
            matching.push(report);
        }
    }
    if service_pid.is_none() || invocation.is_none() || matching.len() != 1 {
        return Err("journal_unavailable");
    }
    Ok(matching.remove(0))
}
impl Companion {
    fn retain_codex_startup_terminal(&mut self, result: CodexStartupTerminalResult) {
        if self.codex_startup_terminal.is_none() {
            self.codex_startup_terminal = Some(result);
        }
    }
    fn pin_codex_startup_journal(&mut self, action: &Uuid) {
        if self.codex_startup_journal_admission.is_some() {
            return;
        }
        // Diagnostic admission is tied to the first original action, even if pinning fails.
        let pins = (|| -> io::Result<CodexStartupJournalPins> {
            private_dir(&self.state_root, false)?;
            let database = self.state_root.join("state.sqlite3");
            private_file(&database)?;
            let root = fs::symlink_metadata(&self.state_root)?;
            let database = fs::symlink_metadata(database)?;
            Ok(CodexStartupJournalPins {
                state_root: (root.dev(), root.ino()),
                database: (database.dev(), database.ino()),
            })
        })()
        .map_err(|_| "state_unavailable");
        self.codex_startup_journal_admission = Some(CodexStartupJournalAdmission {
            action: action.clone(),
            pins,
        });
    }
    fn retain_codex_startup_journal(
        &mut self,
        receipt: &Receipt,
        context: &CodexStartupTerminalContext<'_>,
        source: ServerEvidence,
        settlement_cutoff: u64,
    ) {
        if self.codex_startup_journal.is_none() && context.config.codex_owners.len() == 1 {
            let Some(admission) = self.codex_startup_journal_admission.as_ref() else {
                return;
            };
            if admission.action != receipt.action_id {
                return;
            }
            let Ok(pins) = admission.pins else {
                return;
            };
            self.codex_startup_journal = Some(CodexStartupJournalContext {
                receipt: receipt.clone(),
                settlement_cutoff,
                owner: context.config.codex_owners[0].clone(),
                server: context.server.clone(),
                source,
                state_root: pins.state_root,
                database: pins.database,
            });
        }
    }
    fn recheck(&self) -> io::Result<()> {
        if herdr::process_evidence(self.process.pid)? != self.process {
            return Err(failure("owned companion process epoch changed"));
        }
        Ok(())
    }
    async fn stop_process_until(&mut self, deadline: Instant) -> io::Result<()> {
        self.codex_cleanup_signal = CleanupSignal::default();
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }
        self.recheck()?;
        self.codex_cleanup_signal.attempted = true;
        self.codex_cleanup_signal.sent =
            unsafe { libc::kill(self.process.pid as i32, libc::SIGTERM) } == 0;
        while self.child.try_wait()?.is_none() {
            if Instant::now() > deadline {
                return Err(failure(
                    "owned companion graceful shutdown did not complete; no provider kill",
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(())
    }
    async fn stop(&mut self) -> io::Result<()> {
        let stopped = self
            .stop_process_until(Instant::now() + COMPANION_CLEANUP_BUDGET)
            .await;
        self.codex_cleanup_signal = CleanupSignal::default();
        stopped?;
        self.finish_guard_drain();
        Ok(())
    }
    async fn stop_codex_diagnostic(
        &mut self,
        cleanup_context: &'static str,
    ) -> io::Result<()> {
        let deadline = Instant::now() + COMPANION_CLEANUP_BUDGET;
        self.codex_cleanup_active = true;
        let stopped = self.stop_process_until(deadline).await;
        let Some(drain) = self.guard_drain.take() else {
            return Err(failure("owned Codex diagnostic stderr drain unavailable"));
        };
        let GuardDrain::Codex {
            worker: drain,
            progress,
        } = drain else {
            return Err(failure("owned Codex diagnostic stderr drain changed"));
        };
        if self.codex_startup_journal.is_some() {
            progress.update(|summary| {
                summary.journal_availability = "pending";
                summary.journal_reason = None;
                summary.journal_accepted = false;
            });
        } else {
            let reason = self.codex_startup_journal_admission.as_ref()
                .and_then(|admission| admission.pins.as_ref().err().copied())
                .unwrap_or("not_retained");
            progress.journal_unavailable(reason);
        }
        let cleanup_context = if self.codex_cleanup_signal.attempted {
            cleanup_context
        } else {
            "not_sent"
        };
        if let Err(error) = stopped {
            if drain.is_finished() && drain.join().is_err() {
                progress.update(|summary| {
                    summary.termination = CodexDrainTermination::ThreadPanic;
                    summary.completed = false;
                });
            }
            progress.update(|summary| {
                summary.journal_availability = "unavailable";
                summary.journal_reason = Some("stop_failure");
                summary.journal_accepted = false;
                summary.completed = false;
            });
            progress.snapshot().emit(
                cleanup_context,
                self.codex_cleanup_signal,
                "stop_failure",
            );
            return Err(error);
        }
        while !drain.is_finished() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if !drain.is_finished() {
            progress.update(|summary| {
                summary.journal_availability = "unavailable";
                summary.journal_reason = Some("drain_deadline");
                summary.journal_accepted = false;
                summary.completed = false;
            });
            progress.snapshot().emit(
                cleanup_context,
                self.codex_cleanup_signal,
                "drain_deadline",
            );
            return Err(failure("owned Codex diagnostic stderr drain incomplete"));
        }
        if drain.join().is_err() {
            progress.update(|summary| {
                summary.termination = CodexDrainTermination::ThreadPanic;
                summary.completed = false;
            });
            progress.update(|summary| {
                summary.journal_availability = "unavailable";
                summary.journal_reason = Some("join_failure");
                summary.journal_accepted = false;
                summary.completed = false;
            });
            progress
                .snapshot()
                .emit(cleanup_context, self.codex_cleanup_signal, "joined");
            return Err(failure("owned Codex diagnostic stderr drain panicked"));
        }
        if let Some(result) = self.codex_startup_terminal.take() {
            progress.emit_startup_terminal_after_drain(result);
        }
        if let Some(context) = self.codex_startup_journal.take() {
            match retained_codex_startup_journal(
                &self.state_root,
                &self.binary,
                &context,
                deadline,
            )
            .await
            {
                Ok(report) => progress.emit_startup_journal_after_drain(report),
                Err(reason) => progress.journal_unavailable(reason),
            }
        }
        let summary = progress.snapshot();
        summary.emit(cleanup_context, self.codex_cleanup_signal, "joined");
        if !summary.completed {
            return Err(failure("owned Codex diagnostic stderr drain incomplete"));
        }
        Ok(())
    }
    fn finish_guard_drain(&mut self) {
        let Some(drain) = self.guard_drain.take() else {
            return;
        };
        match drain {
            GuardDrain::Codex { worker: drain, .. } => {
                if drain.is_finished() {
                    let _ = drain.join();
                }
            }
            GuardDrain::Other(drain) => {
                if drain.is_finished() {
                    let _ = drain.join();
                }
            }
        }
    }
}
impl Drop for Companion {
    fn drop(&mut self) {
        let status = self.child.try_wait();
        if let Some(budget) = &self.omp_capture_budget {
            let status = match status.as_ref() {
                Ok(None) => "running",
                Ok(Some(status)) if status.success() => "exited_success",
                Ok(Some(status)) if status.code().is_some() => "exited_failure",
                Ok(Some(_)) => "signaled",
                Err(_) => "status_unavailable",
            };
            budget.child(json!({
                "gate":"owned-omp-creation-capture", "phase":"drop",
                "guard":"child_status", "code":"none", "io":"none",
                "status":status, "ack":null,
            }));
        }
        if status.is_ok_and(|s| s.is_none())
            && !self.codex_cleanup_active
            && self.recheck().is_ok()
        {
            unsafe { libc::kill(self.process.pid as i32, libc::SIGTERM) };
        }
    }
}
#[derive(Debug, Clone, Copy, Default)]
enum HistoryCommandPhase {
    #[default]
    BeforeInput,
    PreScope,
    PreGeometry,
    Dispatch,
    PostScope,
    PostGeometry,
    InputAcknowledged,
    Completion,
    AwaitEvent,
}
#[derive(Default)]
struct HistoryCommandTrace {
    phase: HistoryCommandPhase,
    method: Option<Method>,
    request_phase: &'static str,
    request_guard: &'static str,
    ready: bool,
    idle: bool,
    pending_empty: bool,
    marker_seen: bool,
    completed: bool,
}
impl HistoryCommandTrace {
    fn guard(&self, guard: &'static str) -> io::Error {
        failure(&format!(
            "original native history command rejected; guard={guard}; phase={:?}; method={:?}; request_phase={}; request_guard={}; ready={}; idle={}; pending_empty={}; marker_seen={}; completed={}; no replay",
            self.phase,
            self.method,
            self.request_phase,
            self.request_guard,
            self.ready,
            self.idle,
            self.pending_empty,
            self.marker_seen,
            self.completed,
        ))
    }
    fn deadline(&self) -> io::Error {
        failure(&format!(
            "original native history command outer deadline; phase={:?}; method={:?}; request_phase={}; request_guard={}; ready={}; idle={}; pending_empty={}; marker_seen={}; completed={}; no replay",
            self.phase,
            self.method,
            self.request_phase,
            self.request_guard,
            self.ready,
            self.idle,
            self.pending_empty,
            self.marker_seen,
            self.completed,
        ))
    }
}

/// Decode the pinned native display argv, never execute or expand shell syntax.
/// UserShell emits [environment_shell, "-lc", original_script] through shlex_join.
fn codex_shell_display_matches(display: &str, command: &str) -> bool {
    if display.len() > CONTENT_INLINE_MAX || display.contains(['\0', '\n', '\r']) {
        return false;
    }
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = display.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), ch) => word.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (None, ' ' | '\t') => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                    if words.len() >= 3 {
                        return false;
                    }
                }
            }
            (_, '\\') => {
                let Some(escaped) = chars.next() else {
                    return false;
                };
                if quote == Some('"') && !matches!(escaped, '$' | '`' | '"' | '\\') {
                    word.push('\\');
                }
                word.push(escaped);
                started = true;
            }
            (
                None,
                '$' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '#' | '~' | '*' | '?' | '['
                | ']',
            )
            | (Some('"'), '$' | '`') => return false,
            (_, ch) => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() || !started {
        return false;
    }
    words.push(word);
    if words.len() != 3 || words[1] != "-lc" || words[2] != command {
        return false;
    }
    let shell = Path::new(&words[0]);
    shell.is_absolute()
        && !words[0].chars().any(char::is_control)
        && shell.components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        && matches!(
            shell.file_name().and_then(|name| name.to_str()),
            Some("sh" | "bash" | "zsh")
        )
}

fn codex_shell_completion(
    snapshot: &SessionOpenResult,
    command: &str,
    marker: &str,
    trace: &mut HistoryCommandTrace,
) -> io::Result<Option<Id>> {
    trace.ready = snapshot.state.summary.connectivity == SessionConnectivity::Ready;
    trace.idle = snapshot.state.summary.status == SessionStatus::Idle;
    trace.pending_empty = snapshot.pending_requests.is_empty();
    trace.marker_seen = false;
    trace.completed = false;
    let mut completed = None;
    for record in &snapshot.history {
        let Some(tool) = &record.tool else { continue };
        if tool.name != "commandExecution" {
            continue;
        }
        let Some(Content::Text(output)) = &tool.output else {
            continue;
        };
        // Owned markers contain only ASCII letters/digits/hyphens, so JSON cannot
        // hide an exact match. Parse only candidates, not every retained tool body.
        if !output.text.0.contains(marker) {
            continue;
        }
        let output: Value =
            serde_json::from_str(&output.text.0).map_err(|_| trace.guard("output_projection"))?;
        let Some(text) = output["aggregatedOutput"].as_str() else {
            continue;
        };
        let count = text.lines().filter(|line| *line == marker).count();
        if count == 0 {
            continue;
        }
        trace.marker_seen = true;
        if count != 1 {
            return Err(trace.guard("marker_count"));
        }
        let Some(Content::Text(input)) = &tool.input else {
            return Err(trace.guard("input_projection_absent"));
        };
        let input: Value =
            serde_json::from_str(&input.text.0).map_err(|_| trace.guard("input_projection"))?;
        if !input["command"]
            .as_str()
            .is_some_and(|display| codex_shell_display_matches(display, command))
        {
            return Err(trace.guard("command_mismatch"));
        }
        if record.source_truncated {
            return Err(trace.guard("source_truncated"));
        }
        if !record.complete || tool.status == ToolStatus::Running {
            continue;
        }
        if tool.status != ToolStatus::Succeeded || output["exitCode"].as_i64() != Some(0) {
            return Err(trace.guard("unsuccessful_completion"));
        }
        if completed.replace(record.id.clone()).is_some() {
            return Err(trace.guard("duplicate_completion"));
        }
        trace.completed = true;
    }
    Ok(if trace.ready && trace.idle && trace.pending_empty {
        completed
    } else {
        None
    })
}

/// One original authenticated subscription, containing only server-merged live rows.
/// It is never populated by paging and never recovers a gap by opening again.
struct GenerationView {
    sequence: goatr::state::ClientSequence,
    snapshot: SessionOpenResult,
    record_bytes: HashMap<Id, usize>,
    bytes: usize,
    failed: bool,
}
impl GenerationView {
    fn new(mut snapshot: SessionOpenResult, expected: &Target) -> io::Result<Self> {
        if target(&snapshot.state.summary) != *expected
            || !matches!(
                snapshot.state.summary.connectivity,
                SessionConnectivity::Ready | SessionConnectivity::Synchronizing
            )
        {
            return Err(failure(
                "original history subscription Target/provider changed",
            ));
        }
        // This initial cursor is not paging authority after live changes. The final
        // original-Target snapshot obtains the real cursor for the separate page gate.
        let mut sequence = goatr::state::ClientSequence::default();
        sequence.snapshot(snapshot.subscription.clone());
        let records = std::mem::take(&mut snapshot.history);
        let mut view = Self {
            sequence,
            snapshot,
            record_bytes: HashMap::new(),
            bytes: 0,
            failed: false,
        };
        view.upsert(records)?;
        view.bounded()?;
        Ok(view)
    }
    fn bounded(&self) -> io::Result<()> {
        let extra = serde_json::to_vec(&(&self.snapshot.state, &self.snapshot.pending_requests))
            .map_err(io::Error::other)?
            .len();
        if self.snapshot.pending_requests.len() > 64
            || self.bytes.saturating_add(extra) > 64 * 1024 * 1024
        {
            return Err(failure(
                "original history subscription memory bound exceeded",
            ));
        }
        Ok(())
    }
    fn upsert(&mut self, records: Vec<HistoryRecord>) -> io::Result<()> {
        for record in records {
            let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?.len();
            if let Some(old) = self.record_bytes.insert(record.id.clone(), bytes) {
                self.bytes -= old;
            }
            self.bytes = self.bytes.saturating_add(bytes);
            if let Some(old) = self
                .snapshot
                .history
                .iter_mut()
                .find(|old| old.id == record.id)
            {
                // State::apply already merges source precedence before publishing.
                *old = record;
            } else {
                self.snapshot.history.push(record);
            }
        }
        self.snapshot.history.sort_by_key(|record| record.order.0);
        while self.snapshot.history.len() > goatr::state::HISTORY_RECORDS {
            let removed = self.snapshot.history.remove(0);
            self.bytes -= self
                .record_bytes
                .remove(&removed.id)
                .expect("retained row size");
        }
        self.bounded()
    }
    fn event(&mut self, event: Event) -> io::Result<()> {
        if self.failed {
            return Err(failure("original history subscription already failed"));
        }
        let result = self.apply(event);
        if result.is_err() {
            self.failed = true;
            self.sequence.disconnected();
        }
        result
    }
    fn routed_event(&mut self, event: Event) -> io::Result<()> {
        if event.subscription == self.snapshot.subscription {
            self.event(event)
        } else if matches!(
            event.body,
            EventBody::HostChanged(_)
                | EventBody::ActionStatus(_)
                | EventBody::NotificationUpsert(_)
                | EventBody::NotificationRead(_)
        ) {
            if self.failed {
                return Err(failure("original history subscription already failed"));
            }
            Ok(())
        } else {
            self.failed = true;
            self.sequence.disconnected();
            Err(failure(
                "foreign session event during original history subscription",
            ))
        }
    }
    fn apply(&mut self, event: Event) -> io::Result<()> {
        match self.sequence.event(&event) {
            goatr::state::SequenceDecision::Duplicate => return Ok(()),
            goatr::state::SequenceDecision::Resnapshot => {
                return Err(failure(
                    "original history subscription gap/source loss; no resnapshot",
                ));
            }
            goatr::state::SequenceDecision::Apply => {}
        }
        match event.body {
            EventBody::SessionState(value) => {
                let prior = &self.snapshot.state.summary;
                let next = &value.state.summary;
                if target(next) != target(prior)
                    || next.provider != prior.provider
                    || next.native_id != prior.native_id
                    || next.server_id != prior.server_id
                    || next.workspace_id != prior.workspace_id
                    || next.pane_id != prior.pane_id
                    || next.terminal_id != prior.terminal_id
                    || !matches!(
                        next.connectivity,
                        SessionConnectivity::Ready | SessionConnectivity::Synchronizing
                    )
                {
                    return Err(failure(
                        "original history subscription binding lost; no retarget",
                    ));
                }
                *self.snapshot.state = value.state;
            }
            EventBody::HistoryUpsert(value) => self.upsert(value.records)?,
            EventBody::ActivityUpdate(value) => self.snapshot.state.activity = value.activity,
            EventBody::RequestOpened(value) => {
                if value.request.provider != self.snapshot.state.summary.provider {
                    return Err(failure(
                        "original history subscription request provider changed",
                    ));
                }
                if let Some(old) = self
                    .snapshot
                    .pending_requests
                    .iter_mut()
                    .find(|old| old.request_key == value.request.request_key)
                {
                    *old = value.request;
                } else {
                    self.snapshot.pending_requests.push(value.request);
                }
            }
            EventBody::RequestClosed(value) => {
                if value.reason == RequestClosedReason::GenerationChanged {
                    return Err(failure(
                        "original history subscription request generation changed",
                    ));
                }
                self.snapshot
                    .pending_requests
                    .retain(|request| request.request_key != value.request_key);
            }
            _ => {
                return Err(failure(
                    "unexpected original history subscription event family",
                ));
            }
        }
        self.bounded()
    }
}

struct Phone {
    connection: iroh::endpoint::Connection,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    hello: HelloResult,
    events: u64,
    generation: Option<GenerationView>,
    history_command: Option<HistoryCommandTrace>,
    omp_capture_budget: Option<herdr_report_capture::Budget>,
    phone_close_reported: bool,
    codex_close_capture: bool,
}
fn closed_connection_reason(reason: Option<iroh::endpoint::ConnectionError>) -> &'static str {
    match reason {
        None => "open",
        Some(iroh::endpoint::ConnectionError::VersionMismatch) => "version_mismatch",
        Some(iroh::endpoint::ConnectionError::TransportError(_)) => "transport_error",
        Some(iroh::endpoint::ConnectionError::ConnectionClosed(_)) => "connection_closed",
        Some(iroh::endpoint::ConnectionError::ApplicationClosed(_)) => "application_closed",
        Some(iroh::endpoint::ConnectionError::Reset) => "reset",
        Some(iroh::endpoint::ConnectionError::TimedOut) => "timed_out",
        Some(iroh::endpoint::ConnectionError::LocallyClosed) => "locally_closed",
        Some(iroh::endpoint::ConnectionError::CidsExhausted) => "cids_exhausted",
    }
}
impl Phone {
    fn set_omp_capture(&mut self, budget: herdr_report_capture::Budget) {
        if self.omp_capture_budget.is_none() {
            self.omp_capture_budget = Some(budget);
        }
    }
    fn report_frame_read_close(&mut self) {
        if self.phone_close_reported {
            return;
        }
        let Some(budget) = &self.omp_capture_budget else {
            return;
        };
        let close = closed_connection_reason(self.connection.close_reason());
        self.phone_close_reported = true;
        budget.phone(json!({
            "gate":"owned-omp-creation-capture", "phase":"frame_read",
            "guard":"connection_close_reason", "code":"none", "io":"none",
            "close":close, "ack":null,
        }));
    }
    fn command_phase(&mut self, phase: HistoryCommandPhase) {
        if let Some(trace) = &mut self.history_command {
            trace.phase = phase;
            trace.method = None;
            trace.request_phase = "none";
            trace.request_guard = "none";
        }
    }
    fn request_phase(&mut self, method: Method, phase: &'static str, guard: &'static str) {
        if let Some(trace) = &mut self.history_command {
            trace.method = Some(method);
            trace.request_phase = phase;
            trace.request_guard = guard;
        }
    }
    fn event(&mut self, event: Event) -> io::Result<()> {
        self.events += 1;
        if let Some(view) = &mut self.generation {
            // The Phone also has its original host subscription. Only matching
            // session events feed this view; host/receipt events remain read-only.
            view.routed_event(event)?;
        }
        Ok(())
    }
    async fn request(
        &mut self,
        id: Uuid,
        body: RequestBody,
    ) -> io::Result<Result<ResultBody, WireError>> {
        self.request_at(id, body, "action").await
    }
    async fn request_at(
        &mut self,
        id: Uuid,
        body: RequestBody,
        phase: &'static str,
    ) -> io::Result<Result<ResultBody, WireError>> {
        let method = body.method();
        self.request_phase(method, phase, "payload_encode");
        let mut bytes = encode_payload(&Envelope::Request(Request {
            id: id.clone(),
            body,
        }))
        .map_err(|error| {
            request_failure(method, phase, "payload_encode", io::Error::other(error))
        })?;
        self.request_phase(method, phase, "frame_write");
        transport::write_frame(&mut self.send, &bytes)
            .await
            .map_err(|error| request_failure(method, phase, "frame_write", error))?;
        bytes.fill(0);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(125);
        loop {
            self.request_phase(method, phase, "frame_read");
            let bytes = tokio::time::timeout_at(
                deadline,
                transport::read_frame(&mut self.recv, FRAME_MAX),
            )
            .await
            .map_err(|_| {
                request_failure(
                    method,
                    phase,
                    "response_deadline",
                    failure(
                        "actual framed response deadline; original is status-only, never retried",
                    ),
                )
            })?
            .map_err(|error| {
                // Sample the original connection before error projection or cleanup.
                let close = self.codex_close_capture.then(|| {
                    closed_connection_reason(self.connection.close_reason())
                });
                self.report_frame_read_close();
                request_failure_with_close(method, phase, "frame_read", error, close)
            })?;
            match decode_payload(&bytes, Some(method)).map_err(|error| {
                request_failure(method, phase, "payload_decode", io::Error::other(error))
            })? {
                Envelope::Response(r) if r.id == id => {
                    self.request_phase(method, phase, "response_received");
                    return Ok(r.outcome);
                }
                Envelope::Event(event) => self.event(event)?,
                _ => {
                    return Err(request_failure(
                        method,
                        phase,
                        "correlation",
                        failure("actual framed correlation mismatch"),
                    ));
                }
            }
        }
    }
    async fn rpc(&mut self, body: RequestBody, phase: &'static str) -> io::Result<ResultBody> {
        let method = body.method();
        self.request_at(Uuid::random(), body, phase)
            .await?
            .map_err(|e| {
                failure(&format!(
                    "actual native operation rejected: method={method:?}; phase={phase}; code={:?}",
                    e.code
                ))
            })
    }
    async fn host(&mut self) -> io::Result<HostSnapshotResult> {
        match self
            .rpc(
                RequestBody::HostSnapshot(HostSnapshotParams {}),
                "host_snapshot",
            )
            .await?
        {
            ResultBody::HostSnapshot(s) => Ok(s),
            _ => Err(failure("host snapshot result mismatch")),
        }
    }
    async fn open(
        &mut self,
        target: &Target,
        phase: &'static str,
    ) -> io::Result<SessionOpenResult> {
        if self.generation.is_some() {
            return Err(failure(
                "original history subscription cannot be replaced by polling",
            ));
        }
        let ResultBody::SessionOpen(s) = self
            .rpc(
                RequestBody::SessionOpen(SessionOpenParams {
                    target: target.clone(),
                }),
                phase,
            )
            .await?
        else {
            return Err(failure("native open result missing"));
        };
        self.rpc(
            RequestBody::SessionUnsubscribe(SessionUnsubscribeParams {
                subscription: s.subscription.clone(),
            }),
            phase,
        )
        .await?;
        Ok(s)
    }
    async fn start_generation(&mut self, expected: &Target) -> io::Result<SessionOpenResult> {
        if self.generation.is_some() {
            return Err(failure("original history subscription already installed"));
        }
        let ResultBody::SessionOpen(snapshot) = self
            .rpc(
                RequestBody::SessionOpen(SessionOpenParams {
                    target: expected.clone(),
                }),
                "history_subscription_initial",
            )
            .await?
        else {
            return Err(failure("original history subscription snapshot missing"));
        };
        self.generation = Some(GenerationView::new(snapshot.clone(), expected)?);
        Ok(snapshot)
    }
    async fn finish_generation(&mut self, expected: &Target) -> io::Result<SessionOpenResult> {
        let view = self
            .generation
            .as_ref()
            .ok_or_else(|| failure("original history subscription missing"))?;
        if view.failed || target(&view.snapshot.state.summary) != *expected {
            return Err(failure(
                "original history subscription failed before final paging snapshot",
            ));
        }
        let subscription = view.snapshot.subscription.clone();
        self.rpc(
            RequestBody::SessionUnsubscribe(SessionUnsubscribeParams { subscription }),
            "history_subscription_final",
        )
        .await?;
        self.generation = None;
        self.open(expected, "history_paging_snapshot").await
    }
    async fn generation_event(&mut self) -> io::Result<()> {
        self.command_phase(HistoryCommandPhase::AwaitEvent);
        self.request_phase(
            Method::SessionOpen,
            "history_subscription_completion",
            "frame_read",
        );
        let bytes = transport::read_frame(&mut self.recv, FRAME_MAX)
            .await
            .map_err(|error| {
                request_failure(
                    Method::SessionOpen,
                    "history_subscription_completion",
                    "frame_read",
                    error,
                )
            })?;
        match decode_payload(&bytes, None).map_err(|error| {
            request_failure(
                Method::SessionOpen,
                "history_subscription_completion",
                "payload_decode",
                io::Error::other(error),
            )
        })? {
            Envelope::Event(event) => self.event(event),
            _ => Err(failure(
                "unsolicited response during original history subscription",
            )),
        }
    }
    async fn receipt(&mut self, id: &Uuid) -> io::Result<Receipt> {
        match self
            .rpc(
                RequestBody::ActionStatus(ActionStatusParams {
                    action_id: id.clone(),
                }),
                "action_status",
            )
            .await?
        {
            ResultBody::ActionStatus(ActionStatusResult::Known(r)) => Ok(r.receipt),
            _ => Err(failure(
                "actual original receipt unknown; resend prohibited",
            )),
        }
    }
    async fn settled_by(&mut self, id: &Uuid, deadline: Instant) -> io::Result<Receipt> {
        loop {
            let r = self.receipt(id).await?;
            if !matches!(r.state, ReceiptState::Reserved | ReceiptState::Dispatching) {
                return Ok(r);
            }
            if Instant::now() > deadline {
                return Err(failure(
                    "actual receipt remains ongoing; retained resources are not retried",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn settled(&mut self, id: &Uuid) -> io::Result<Receipt> {
        self.settled_by(id, Instant::now() + Duration::from_secs(125))
            .await
    }
    async fn creation_settled(&mut self, id: &Uuid, provider: Provider) -> io::Result<Receipt> {
        let seconds = if provider == Provider::Codex {
            305
        } else {
            125
        };
        self.settled_by(id, Instant::now() + Duration::from_secs(seconds))
            .await
    }
}
#[track_caller]
fn request_failure(
    method: Method,
    phase: &'static str,
    guard: &'static str,
    error: io::Error,
) -> io::Error {
    request_failure_with_close(method, phase, guard, error, None)
}
#[track_caller]
fn request_failure_with_close(
    method: Method,
    phase: &'static str,
    guard: &'static str,
    error: io::Error,
    close: Option<&'static str>,
) -> io::Error {
    let frame = transport::FrameFailure::from_error(&error);
    let code = error
        .get_ref()
        .and_then(|error| error.downcast_ref::<ProtocolFailure>())
        .map(|error| match error.kind {
            FailureKind::UnsupportedVersion => ErrorCode::UnsupportedVersion,
            FailureKind::UnsupportedMethod => ErrorCode::UnsupportedMethod,
            FailureKind::InvalidRequest | FailureKind::Incompatible => ErrorCode::InvalidRequest,
        });
    // Preserve the caller's I/O classification, while dropping all original error text/data.
    let close = close.map(|close| format!("; close={close}")).unwrap_or_default();
    super::located_message(
        error.kind(),
        &format!(
            "actual operation failed: method={method:?}; phase={phase}; guard={guard}; io_kind={:?}; frame={frame:?}; code={code:?}{close}; no replay",
            error.kind()
        ),
        super::helper_error_location(&error),
    )
}
async fn partial_creation(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    provider: Provider,
    server: &Uuid,
    serve: &mut Companion,
) -> io::Result<(Uuid, RequestBody, Receipt)> {
    let before = surface(socket).await?.1;
    let id = Uuid::random();
    let body = RequestBody::SessionCreate(SessionCreateParams { source_target: None, checkout: None,
        boot_id: phone.hello.boot_id.clone(),
        server_id: server.clone(),
        project_id: config.projects[0].id.clone(),
        provider,
        name: Label::new(format!("partial-{provider:?}-{}", &id.0[..8]))
            .map_err(io::Error::other)?,
    });
    let mut bytes = encode_payload(&Envelope::Request(Request {
        id: id.clone(),
        body: body.clone(),
    }))
    .map_err(io::Error::other)?;
    transport::write_frame(&mut phone.send, &bytes).await?;
    bytes.fill(0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let observed = loop {
        let bytes = tokio::time::timeout_at(
            deadline,
            transport::read_frame(&mut phone.recv, FRAME_MAX),
        )
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "actual partial creation window not observed; original is never resubmitted",
            )
        })??;
        match decode_payload(&bytes, Some(Method::SessionCreate)).map_err(io::Error::other)? {
            Envelope::Event(event) => {
                if let EventBody::ActionStatus(status) = event.body {
                    let r = status.receipt;
                    if r.action_id == id
                        && r.state == ReceiptState::Dispatching
                        && r.creation
                            .as_ref()
                            .is_some_and(|c| c.resources.workspace_id.is_some())
                    {
                        break r;
                    }
                }
            }
            Envelope::Response(response) if response.id == id => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "actual create settled before partial fault window; no second attempt or invented uncertainty",
                ));
            }
            _ => return Err(failure("actual partial creation frame correlation changed")),
        }
    };
    if focus(&surface(socket).await?.1) != focus(&before) {
        return Err(failure("actual partial creation stole focus"));
    }
    serve.recheck()?;
    if unsafe { libc::kill(serve.process.pid as i32, libc::SIGKILL) } != 0 {
        return Err(io::Error::last_os_error());
    }
    serve.child.wait()?;
    Ok((id, body, observed))
}
async fn refresh_native(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    native: &mut [NativeProof],
) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let host = phone.host().await?;
        if native.iter().all(|p| {
            host.sessions.iter().any(|s| {
                s.session_id == p.target.session_id
                    && s.native_id == p.native
                    && s.connectivity == SessionConnectivity::Ready
            })
        }) {
            for p in native.iter_mut() {
                let s = host
                    .sessions
                    .iter()
                    .find(|s| s.session_id == p.target.session_id)
                    .ok_or_else(|| failure("actual native identity vanished"))?;
                let current = proof(config, socket, s, None).await?;
                if p.owner != current.owner
                    || p.tui != current.tui
                    || p.relay != current.relay
                    || p.native != current.native
                {
                    return Err(failure(
                        "companion recovery replaced native owner/relay/TUI/session",
                    ));
                }
                *p = current;
            }
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "actual surviving native bindings not ready; no replacement/cold resume/replay",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn connect(
    endpoint: &iroh::Endpoint,
    host: &EndpointId,
    invite: Option<PairInvite>,
) -> io::Result<Phone> {
    let id = host
        .0
        .parse::<iroh::EndpointId>()
        .map_err(|_| failure("phone setup failed: phase=phone_connect; guard=host_id; no retry"))?;
    tokio::time::timeout(Duration::from_secs(45), endpoint.online())
        .await
        .map_err(|_| prerequisite("real N0 discovery/relay readiness unavailable"))?;
    let c = tokio::time::timeout(
        Duration::from_secs(45),
        endpoint.connect(iroh::EndpointAddr::new(id), goatr::ALPN),
    )
    .await
    .map_err(|_| prerequisite("real discovery/relay connectivity unavailable"))?
    .map_err(|_| failure("phone setup failed: phase=phone_connect; guard=connect; no retry"))?;
    let (send, recv) = c
        .open_bi()
        .await
        .map_err(|_| failure("phone setup failed: phase=phone_connect; guard=open_bi; no retry"))?;
    let mut p = Phone {
        connection: c,
        send,
        recv,
        hello: HelloResult {
            host_id: host.clone(),
            device_id: Uuid::random(),
            boot_id: Uuid::random(),
            protocol: ProtocolVersion(2),
            backends: vec![],
        },
        events: 0,
        generation: None,
        history_command: None,
        omp_capture_budget: None,
        phone_close_reported: false,
        codex_close_capture: false,
    };
    if let Some(i) = invite {
        p.rpc(
            RequestBody::PairConfirm(PairConfirmParams {
                token: i.token,
                device_name: Label::new("Goatr owned stock checkpoint")
                    .map_err(io::Error::other)?,
            }),
            "pair_confirm",
        )
        .await?;
    }
    let ResultBody::Hello(hello) = p
        .rpc(
            RequestBody::Hello(HelloParams {
                app_version: env!("CARGO_PKG_VERSION").into(),
                device_name: Label::new("Goatr owned stock checkpoint")
                    .map_err(io::Error::other)?,
            }),
            "hello",
        )
        .await?
    else {
        return Err(failure("real hello missing"));
    };
    if hello.host_id != *host {
        return Err(failure("real host identity mismatch"));
    }
    p.hello = hello;
    Ok(p)
}
fn receipt_result(body: ResultBody) -> io::Result<Receipt> {
    match body {
        ResultBody::SessionCreate(r) => Ok(r.receipt),
        ResultBody::PromptSubmit(r) => Ok(r.receipt),
        ResultBody::TurnCancel(r) => Ok(r.receipt),
        ResultBody::RequestRespond(r) => Ok(r.receipt),
        _ => Err(failure("native action receipt absent")),
    }
}
fn target(s: &SessionSummary) -> Target {
    Target {
        session_id: s.session_id.clone(),
        generation: s.generation.clone(),
    }
}
fn focus(s: &Snapshot) -> (Option<Id>, Option<Id>, Option<Id>) {
    (
        s.focused_workspace_id.clone(),
        s.focused_tab_id.clone(),
        s.focused_pane_id.clone(),
    )
}
async fn surface(socket: &Path) -> io::Result<(goatr::storage::ServerEvidence, Snapshot)> {
    Client::connect(socket).await?.snapshot().await
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeometryPhase {
    UiBefore,
    UiAfter,
    CreateBefore,
    CreateAfter,
    InputBefore,
    InputAfter,
}
impl std::fmt::Display for GeometryPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UiBefore => "ui_before",
            Self::UiAfter => "ui_after",
            Self::CreateBefore => "create_before",
            Self::CreateAfter => "create_after",
            Self::InputBefore => "input_before",
            Self::InputAfter => "input_after",
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeometryGuard {
    Connect,
    ResponseTimeout,
    NativeRejected,
    SourceChanged,
    ResponseIo,
    LayoutMissing,
    LayoutBounds,
}
impl GeometryGuard {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::ResponseTimeout => "response_timeout",
            Self::NativeRejected => "native_rejected",
            Self::SourceChanged => "source_changed",
            Self::ResponseIo => "response_io",
            Self::LayoutMissing => "layout_missing",
            Self::LayoutBounds => "layout_bounds",
        }
    }
}
impl std::fmt::Display for GeometryGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeometryIoKind {
    None,
    NotFound,
    TimedOut,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    BrokenPipe,
    NotConnected,
    PermissionDenied,
    InvalidData,
    Other,
}
impl GeometryIoKind {
    fn from_error(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::TimedOut => Self::TimedOut,
            io::ErrorKind::ConnectionRefused => Self::ConnectionRefused,
            io::ErrorKind::ConnectionReset => Self::ConnectionReset,
            io::ErrorKind::ConnectionAborted => Self::ConnectionAborted,
            io::ErrorKind::BrokenPipe => Self::BrokenPipe,
            io::ErrorKind::NotConnected => Self::NotConnected,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::InvalidData => Self::InvalidData,
            _ => Self::Other,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GeometryFailure {
    phase: GeometryPhase,
    guard: GeometryGuard,
    io_kind: GeometryIoKind,
    outer_kind: io::ErrorKind,
}
impl GeometryFailure {
    const fn new(phase: GeometryPhase, guard: GeometryGuard) -> Self {
        Self {
            phase,
            guard,
            io_kind: GeometryIoKind::None,
            outer_kind: io::ErrorKind::Other,
        }
    }
    fn connect(phase: GeometryPhase, error: &io::Error) -> Self {
        Self {
            phase,
            guard: GeometryGuard::Connect,
            io_kind: GeometryIoKind::from_error(error),
            outer_kind: error.kind(),
        }
    }
    fn response(phase: GeometryPhase, error: herdr::ApiError) -> Self {
        match error {
            herdr::ApiError::Rejected(_) => Self {
                phase,
                guard: GeometryGuard::NativeRejected,
                io_kind: GeometryIoKind::None,
                outer_kind: io::ErrorKind::NotConnected,
            },
            herdr::ApiError::Changed(_) => Self {
                phase,
                guard: GeometryGuard::SourceChanged,
                io_kind: GeometryIoKind::None,
                outer_kind: io::ErrorKind::NotConnected,
            },
            herdr::ApiError::Unavailable(error) => Self {
                phase,
                guard: if error.kind() == io::ErrorKind::InvalidData
                    && error.to_string() == "Herdr response timeout"
                {
                    GeometryGuard::ResponseTimeout
                } else {
                    GeometryGuard::ResponseIo
                },
                io_kind: GeometryIoKind::from_error(&error),
                outer_kind: io::ErrorKind::NotConnected,
            },
        }
    }
    fn into_io(self) -> io::Error {
        let kind = if self.phase == GeometryPhase::InputAfter {
            io::ErrorKind::Other
        } else {
            self.outer_kind
        };
        io::Error::new(kind, self)
    }
}
impl std::fmt::Display for GeometryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "owned Herdr geometry failed: phase={}; guard={}; io_kind={:?}; no retry",
            self.phase, self.guard, self.io_kind
        )
    }
}
impl std::error::Error for GeometryFailure {}
async fn geometry(
    socket: &Path,
    pane: &Id,
    phase: GeometryPhase,
) -> Result<Value, GeometryFailure> {
    let client = Client::connect(socket)
        .await
        .map_err(|error| GeometryFailure::connect(phase, &error))?;
    let result = client
        .request("pane.layout", json!({"pane_id":pane}), "pane_layout")
        .await
        .map_err(|error| GeometryFailure::response(phase, error))?;
    let layout = result
        .get("layout")
        .ok_or_else(|| GeometryFailure::new(phase, GeometryGuard::LayoutMissing))?;
    if layout["panes"].as_array().is_none_or(|p| p.len() > 512)
        || layout["splits"].as_array().is_none_or(|p| p.len() > 512)
    {
        return Err(GeometryFailure::new(phase, GeometryGuard::LayoutBounds));
    }
    Ok(layout.clone())
}
#[derive(Clone)]
struct NativeProof {
    target: Target,
    provider: Provider,
    native: Id,
    pane: Id,
    terminal: Id,
    workspace: Id,
    owner: ProcessEvidence,
    tui: Option<ProcessEvidence>,
    relay: Option<ProcessEvidence>,
    manifest: Option<PathBuf>,
    collaboration: Option<goatr::codex_relay::CollaborationModeObservation>,
    default_plan: Option<bool>,
    omp_observer: Option<Arc<OmpObserver>>,
    objects: Value,
}
struct NativeHistoryWitness {
    original: Target,
    provider: Provider,
    native: Id,
    owner: ProcessEvidence,
    older: Vec<Id>,
    retained: HashSet<Id>,
    qualified: Id,
    qualified_later_completed_lower_bound: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeHistoryMode {
    BoundedDiagnostic128,
    RetentionBoundary1024,
}
impl NativeHistoryMode {
    const fn command_count(self) -> usize {
        match self {
            Self::BoundedDiagnostic128 => 128,
            Self::RetentionBoundary1024 => 1024,
        }
    }
    const fn diagnostic(self) -> bool {
        matches!(self, Self::BoundedDiagnostic128)
    }
    const fn name(self) -> &'static str {
        match self {
            Self::BoundedDiagnostic128 => "bounded_diagnostic_128",
            Self::RetentionBoundary1024 => "retention_boundary_1024",
        }
    }
}
impl NativeProof {
    fn recheck_processes(&self) -> io::Result<()> {
        for p in std::iter::once(&self.owner)
            .chain(self.tui.iter())
            .chain(self.relay.iter())
        {
            if herdr::process_evidence(p.pid)? != *p {
                return Err(failure(
                    "native owner/relay/TUI process epoch changed; no replacement allowed",
                ));
            }
        }
        Ok(())
    }
    fn same_input_binding(&self, current: &Self) -> bool {
        self.target == current.target && self.same_physical_binding(current)
    }
    fn same_feedback_restart_binding(&self, current: &Self) -> bool {
        self.target.generation != current.target.generation && self.same_physical_binding(current)
    }
    // Companion generations are fresh after explicit restart; native owner/room/relay
    // epochs below are physical identity and must remain unchanged.
    fn same_physical_binding(&self, current: &Self) -> bool {
        let keys: &[&str] = match self.provider {
            Provider::Omp => &[
                "herdr",
                "instance",
                "roomGeneration",
                "runtime",
                "nativeHeader",
                "historyPath",
                "outerWrapper",
            ],
            Provider::Codex => &[
                "herdr",
                "ownerEpoch",
                "ownerId",
                "bindingId",
                "connectionEpoch",
                "listenHost",
                "upstreamHost",
                "outerWrapper",
            ],
        };
        self.target.session_id == current.target.session_id
            && self.provider == current.provider
            && self.native == current.native
            && self.pane == current.pane
            && self.terminal == current.terminal
            && self.workspace == current.workspace
            && self.owner == current.owner
            && self.tui == current.tui
            && self.relay == current.relay
            && self.manifest == current.manifest
            && keys.iter().all(
                |key| match (self.objects.get(*key), current.objects.get(*key)) {
                    (Some(before), Some(after)) => before == after,
                    _ => false,
                },
            )
    }
    fn evidence(&self) -> Value {
        json!({"target":self.target,"provider":self.provider,"nativeId":self.native,"pane":self.pane,"terminal":self.terminal,"workspace":self.workspace,"owner":self.owner,"tui":self.tui,"relay":self.relay,"manifest":self.manifest,"objects":self.objects,"collaborationMode":self.collaboration,"collaborationDefaultPlan":self.default_plan})
    }
}

// Timeout observation only: never serialize identifiers or free-text native reasons.
fn restart_observation(
    provider: Provider,
    target: &Target,
    native: &Id,
    rows: &[SessionSummary],
) -> Value {
    let matching = rows
        .iter()
        .filter(|row| row.session_id == target.session_id);
    let first = matching.clone().next();
    let same_native = rows
        .iter()
        .filter(|row| row.provider == provider && &row.native_id == native);
    json!({
        "provider":provider,
        "sameSessionRow":first.is_some(),
        "sameNative":matching.clone().any(|row| &row.native_id == native),
        "generationChanged":matching.clone().any(|row| row.generation != target.generation),
        "ready":matching.clone().any(|row| row.connectivity == SessionConnectivity::Ready),
        "allConjunctsSatisfied":rows.iter().any(|row| {
            row.session_id == target.session_id
                && &row.native_id == native
                && row.generation != target.generation
                && row.connectivity == SessionConnectivity::Ready
        }),
        "matchingRowCount":matching.take(512).count(),
        "sameNativeRows":same_native.clone().take(512).count(),
        "sameNativeOriginalSession":same_native.clone().any(|row| row.session_id == target.session_id),
        "connectivity":first.map(|row| row.connectivity),
        "status":first.map(|row| row.status)
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum McpKind {
    Form,
    Url,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct McpLease {
    v: u32,
    run: Uuid,
    directory: PathBuf,
    directory_object: (u64, u64),
    image: PathBuf,
    image_sha256: String,
    server: String,
    owner: Option<ProcessEvidence>,
    network: (u64, u64),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct McpOperation {
    run: Uuid,
    operation: Uuid,
    kind: McpKind,
    target: Target,
    native_id: Id,
}
struct NativeMcp {
    lease: PathBuf,
    directory: PathBuf,
    server: String,
    wrapper: PathBuf,
}
fn shell_literal(value: &str) -> io::Result<String> {
    if value.chars().any(char::is_control) {
        return Err(failure(
            "owned native wrapper literal contains a control character",
        ));
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}
impl NativeMcp {
    fn prepare(root: &Path, run: &Uuid, config: &mut Config, evidence: &Path) -> io::Result<Self> {
        let directory = root.join("project/.goatr-native");
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let original = config.executables.codex.as_ref().ok_or_else(|| {
            prerequisite("owned MCP registration requires the approved outer Codex wrapper")
        })?;
        let original_hash = binary_sha256(original)?;
        let current = fs::canonicalize(std::env::current_exe()?)?;
        let before = identity(&current)?;
        let image_hash = binary_sha256(&current)?;
        let image = directory.join("smoke");
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&current)?;
        let source_object = source.metadata()?;
        let kernel_image = fs::metadata("/proc/self/exe")?;
        if (source_object.dev(), source_object.ino()) != (kernel_image.dev(), kernel_image.ino()) {
            return Err(failure(
                "owned MCP source image differs from the running helper",
            ));
        }
        let mut copied = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&image)?;
        io::copy(&mut source, &mut copied)?;
        copied.sync_all()?;
        if identity(&current)? != before
            || binary_sha256(&current)? != image_hash
            || binary_sha256(&image)? != image_hash
        {
            return Err(failure("owned MCP executable copy/source changed"));
        }
        let metadata = copied.metadata()?;
        if metadata.uid() != uid() || metadata.mode() & 0o7777 != 0o500 || metadata.nlink() != 1 {
            return Err(failure("owned MCP executable copy permissions changed"));
        }
        let lease_path = directory.join("lease.json");
        let server = format!("goatr_owned_{}", &run.0[..8]);
        let directory_meta = fs::metadata(&directory)?;
        let network = fs::metadata("/proc/self/ns/net")?;
        let lease = McpLease {
            v: 1,
            run: run.clone(),
            directory: directory.clone(),
            directory_object: (directory_meta.dev(), directory_meta.ino()),
            image: image.clone(),
            image_sha256: image_hash.clone(),
            server: server.clone(),
            owner: None,
            network: (network.dev(), network.ino()),
        };
        goatr::codex_relay::private_write(&lease_path, &lease, false)?;
        let command = format!(
            "mcp_servers.{server}.command={}",
            serde_json::to_string(&image)?
        );
        let arguments = format!(
            "mcp_servers.{server}.args={}",
            serde_json::to_string(&[
                "--owned-mcp-stdio",
                lease_path
                    .to_str()
                    .ok_or_else(|| failure("owned MCP lease path is not UTF-8"))?
            ])?
        );
        let original_word = shell_literal(
            original
                .to_str()
                .ok_or_else(|| failure("approved Codex wrapper path is not UTF-8"))?,
        )?;
        let script = format!(
            "#!/bin/sh\nif [ \"${{1-}}\" = app-server ]; then\n  exec {original_word} -c {} -c {} \"$@\"\nfi\nexec {original_word} \"$@\"\n",
            shell_literal(&command)?,
            shell_literal(&arguments)?
        );
        let wrapper = directory.join("codex");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&wrapper)?;
        output.write_all(script.as_bytes())?;
        output.sync_all()?;
        if binary_sha256(original)? != original_hash {
            return Err(failure("approved original Codex outer wrapper changed"));
        }
        write_json(
            &evidence.join(format!("native-mcp-registration-{}.json", run.0)),
            &json!({
                "runId":run,"server":server,"generatedWrapper":identity(&wrapper)?,
                "generatedWrapperSha256":binary_sha256(&wrapper)?,"originalOuter":original,
                "originalOuterSha256":original_hash,"helperSource":before,"copiedHelper":identity(&image)?,
                "helperSha256":image_hash,"lease":lease_path,
                "argvPolicy":"MCP -c prefixes before unchanged app-server/listen/socket tail; all TUI arguments unchanged",
                "shellResolution":"candidate PATH only; actual production Herdr probe must prove this wrapper",
                "credentials":"unchanged; not copied","nativePermissionDefaults":"unchanged"
            }),
        )?;
        config.executables.codex = Some(wrapper.clone());
        Ok(Self {
            lease: lease_path,
            directory,
            server,
            wrapper,
        })
    }
    fn bind_owner(&self, p: &NativeProof) -> io::Result<()> {
        if p.provider != Provider::Codex
            || p.objects["outerWrapper"].as_str() != self.wrapper.to_str()
        {
            return Err(failure(
                "owned MCP was not created through its proved configured wrapper",
            ));
        }
        let mut lease: McpLease = goatr::codex_relay::private_read(&self.lease)?;
        if lease.owner.as_ref().is_some_and(|owner| owner != &p.owner) {
            return Err(failure("owned MCP owner changed; no service adoption"));
        }
        lease.owner = Some(p.owner.clone());
        goatr::codex_relay::private_write(&self.lease, &lease, true)
    }
    fn operation(&self, p: &NativeProof, kind: McpKind, run: &Uuid) -> io::Result<Uuid> {
        let operation = Uuid::random();
        let record = McpOperation {
            run: run.clone(),
            operation: operation.clone(),
            kind,
            target: p.target.clone(),
            native_id: p.native.clone(),
        };
        goatr::codex_relay::private_write(
            &self
                .directory
                .join(format!("operation-{}.json", operation.0)),
            &record,
            false,
        )?;
        Ok(operation)
    }
    async fn ready(&self, kind: McpKind) -> io::Result<()> {
        let lease: McpLease = goatr::codex_relay::private_read(&self.lease)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let mut count = 0;
            let mut failed = false;
            for entry in fs::read_dir(&self.directory)? {
                let path = entry?.path();
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("service-") && name.ends_with(".json"))
                {
                    continue;
                }
                count += 1;
                if count > 64 {
                    return Err(failure("owned MCP service record bound exceeded"));
                }
                let value: Value = goatr::codex_relay::private_read(&path)?;
                if value["runId"].as_str() != Some(lease.run.0.as_str())
                    || value["server"].as_str() != Some(self.server.as_str())
                    || value["imageSha256"].as_str() != Some(lease.image_sha256.as_str())
                {
                    return Err(failure(
                        "owned MCP service metadata belongs to another scope",
                    ));
                }
                if value["failed"] == true {
                    failed = true;
                    continue;
                }
                if value["initialized"] == true
                    && value["toolsListed"] == true
                    && value[if kind == McpKind::Form { "form" } else { "url" }] == true
                {
                    return Ok(());
                }
            }
            if failed {
                return Err(prerequisite(
                    "owned native MCP service failed; finite producer stage retained",
                ));
            }
            if Instant::now() >= deadline {
                return Err(prerequisite(
                    "actual owned native MCP initialize/tools-list/elicitation capability unavailable",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn observed_decline(&self, operation: &Uuid, desktop: bool) -> io::Result<Value> {
        let path = self.directory.join(format!("result-{}.json", operation.0));
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if path.try_exists()? {
                let result: Value = goatr::codex_relay::private_read(&path)?;
                let action = result["action"].as_str();
                if action == Some("decline") || (desktop && action == Some("cancel")) {
                    if result["operation"].as_str() != Some(operation.0.as_str())
                        || result["effectApplied"].as_bool() != Some(false)
                        || result["ownerVerified"].as_bool() != Some(true)
                    {
                        return Err(failure(
                            "actual MCP decline lacked exact owned no-effect proof",
                        ));
                    }
                    return Ok(json!({"operation":operation,"actualAction":action,
                        "ownerVerified":true,"effectApplied":false,"source":"real owned MCP response"}));
                }
                if result["stage"]
                    .as_str()
                    .is_some_and(|stage| stage != "requested")
                {
                    return Err(failure(
                        "actual owned MCP outcome was not the required decline/cancel",
                    ));
                }
            }
            if Instant::now() >= deadline {
                return Err(prerequisite(
                    "actual native MCP decline did not reach its owned protocol service",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
fn mcp_owner(lease: &McpLease) -> io::Result<()> {
    let owner = lease
        .owner
        .as_ref()
        .ok_or_else(|| failure("owned MCP original native owner is not bound"))?;
    // SAFETY: getppid has no pointer preconditions; the native client is our direct parent.
    let parent = herdr::process_evidence(unsafe { libc::getppid() } as u32)?;
    if parent.linux_boot_id != owner.linux_boot_id
        || parent.start_ticks != owner.start_ticks
        || parent.namespaces.pid != owner.namespaces.pid
        || parent.namespaces.user != owner.namespaces.user
        || parent.namespaces.mount != owner.namespaces.mount
        || owner.namespaces.nspids.last() != Some(&parent.pid)
    {
        return Err(failure(
            "owned MCP parent is not the recorded original native owner",
        ));
    }
    Ok(())
}
struct OwnedUrl {
    url: String,
    port: u16,
    stop: tokio::sync::watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<io::Result<bool>>>,
}
impl OwnedUrl {
    async fn start(operation: &Uuid, directory: &Path, network: (u64, u64)) -> io::Result<Self> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let actual = fs::metadata("/proc/self/ns/net")?;
        if (actual.dev(), actual.ino()) != network {
            return Err(prerequisite(
                "owned URL workflow does not share the recorded desktop network namespace",
            ));
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let route = format!("/{}", operation.0);
        let url = format!("http://{address}{route}");
        let label_path = directory.join(format!("label-{}.json", operation.0));
        let (stop, mut cancelled) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let mut applied = false;
            loop {
                let (mut stream, peer) = tokio::select! {
                    biased;
                    _=cancelled.changed()=>break,
                    accepted=listener.accept()=>accepted?,
                };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let request = async {
                    let mut bytes = Vec::with_capacity(4096);
                    let (header_end, content_length) =
                        loop {
                            if bytes.len() >= 8192 {
                                return Err(failure("owned URL request exceeds bound"));
                            }
                            let mut chunk = [0; 1024];
                            let n = stream.read(&mut chunk).await?;
                            if n == 0 {
                                return Err(failure("owned URL request ended early"));
                            }
                            bytes.extend_from_slice(&chunk[..n]);
                            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                                let header = std::str::from_utf8(&bytes[..end])
                                    .map_err(|_| failure("owned URL invalid headers"))?;
                                let mut length = None;
                                for line in header.lines().skip(1) {
                                    let (name, value) = line
                                        .split_once(':')
                                        .ok_or_else(|| failure("owned URL malformed header"))?;
                                    if name.eq_ignore_ascii_case("transfer-encoding") {
                                        return Err(failure(
                                            "owned URL transfer encoding is unsupported",
                                        ));
                                    }
                                    if name.eq_ignore_ascii_case("content-length") {
                                        if length.is_some() {
                                            return Err(failure("owned URL duplicate length"));
                                        }
                                        length =
                                            Some(value.trim().parse::<usize>().map_err(|_| {
                                                failure("owned URL invalid length")
                                            })?);
                                    }
                                }
                                break (end + 4, length.unwrap_or(0));
                            }
                        };
                    if content_length > 4096 {
                        return Err(failure("owned URL body exceeds bound"));
                    }
                    while bytes.len() < header_end + content_length {
                        let mut chunk = [0; 1024];
                        let n = stream.read(&mut chunk).await?;
                        if n == 0 {
                            return Err(failure("owned URL body ended early"));
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                    }
                    let first = std::str::from_utf8(&bytes[..header_end])
                        .map_err(|_| failure("owned URL header encoding"))?
                        .lines()
                        .next()
                        .ok_or_else(|| failure("owned URL request line absent"))?;
                    let (status, content_type, body) = if first == format!("GET {route} HTTP/1.1") {
                        (200, "text/html; charset=utf-8",
                            "<!doctype html><title>Owned acceptance label</title><form><label>Nonsecret owned label <input name=label maxlength=64 required></label><button>Save owned label</button></form><output></output><script>document.querySelector('form').onsubmit=async e=>{e.preventDefault();const r=await fetch(location.pathname,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({label:new FormData(e.target).get('label')})});document.querySelector('output').textContent=r.ok?'Owned label saved':'Not saved';};</script>".to_owned())
                    } else if first == format!("POST {route} HTTP/1.1") && !applied {
                        let value = goatr::native_json::parse(
                            &bytes[header_end..header_end + content_length],
                            goatr::native_json::REGISTRY,
                        )?;
                        let label = value["label"]
                            .as_str()
                            .filter(|label| !label.is_empty() && label.chars().count() <= 64)
                            .ok_or_else(|| failure("owned URL nonsecret label required"))?;
                        if value.as_object().is_none_or(|object| object.len() != 1) {
                            return Err(failure("owned URL unexpected form fields"));
                        }
                        goatr::codex_relay::private_write(
                            &label_path,
                            &json!({"label":label}),
                            false,
                        )?;
                        applied = true;
                        (200, "text/plain", "Owned label saved".to_owned())
                    } else {
                        (404, "text/plain", "Unavailable owned workflow".to_owned())
                    };
                    let response = format!(
                        "HTTP/1.1 {status} Owned\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await
                };
                tokio::select! {
                    biased;
                    _=cancelled.changed()=>break,
                    result=tokio::time::timeout(Duration::from_secs(5),request)=>{
                        if let Ok(Err(error))=result { return Err(error); }
                    }
                }
            }
            Ok(applied)
        });
        Ok(Self {
            url,
            port: address.port(),
            stop,
            task: Some(task),
        })
    }
    async fn close(mut self) -> io::Result<bool> {
        self.stop.send_replace(true);
        self.task
            .take()
            .expect("owned URL task")
            .await
            .map_err(io::Error::other)?
    }
}
impl Drop for OwnedUrl {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
struct McpStdin(tokio::io::unix::AsyncFd<File>);
impl McpStdin {
    fn open() -> io::Result<Self> {
        use std::os::fd::AsFd;
        let original = File::from(io::stdin().as_fd().try_clone_to_owned()?);
        let before = original.metadata()?;
        // Reopening gives this reader a separate open-file description. Never
        // change O_NONBLOCK on the inherited/global stdin description.
        let pipe = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/proc/self/fd/0")?;
        let after = pipe.metadata()?;
        if before.mode() & libc::S_IFMT != libc::S_IFIFO
            || after.mode() & libc::S_IFMT != libc::S_IFIFO
            || (before.dev(), before.ino()) != (after.dev(), after.ino())
        {
            return Err(failure("owned MCP stdin is not the original pipe"));
        }
        Ok(Self(tokio::io::unix::AsyncFd::new(pipe)?))
    }
}
impl tokio::io::AsyncRead for McpStdin {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        if buffer.remaining() == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        loop {
            let mut ready = std::task::ready!(self.0.poll_read_ready(cx))?;
            match ready.try_io(|inner| {
                let mut pipe = inner.get_ref();
                pipe.read(buffer.initialize_unfilled())
            }) {
                Ok(Ok(count)) => {
                    buffer.advance(count);
                    return std::task::Poll::Ready(Ok(()));
                }
                Ok(Err(error)) => return std::task::Poll::Ready(Err(error)),
                Err(_) => continue,
            }
        }
    }
}
async fn mcp_output(output: &mut tokio::io::Stdout, value: Value) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await
}
async fn mcp_line(input: &mut tokio::io::BufReader<McpStdin>) -> io::Result<Option<Value>> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let mut bytes = Vec::new();
    let count = (&mut *input)
        .take(1_048_577)
        .read_until(b'\n', &mut bytes)
        .await?;
    if count == 0 {
        return Ok(None);
    }
    if count > 1_048_576 || bytes.last() != Some(&b'\n') {
        return Err(failure(
            "owned MCP input frame exceeds bound or is incomplete",
        ));
    }
    goatr::native_json::parse(&bytes, goatr::native_json::NATIVE).map(Some)
}
async fn mcp_input(
    input: &mut tokio::io::BufReader<McpStdin>,
    deadline: Option<tokio::time::Instant>,
) -> Result<io::Result<Option<Value>>, ()> {
    match deadline {
        None => Ok(mcp_line(input).await),
        Some(deadline) => tokio::select! {
            biased;
            _=tokio::time::sleep_until(deadline)=>Err(()),
            value=mcp_line(input)=>Ok(value),
        },
    }
}
struct PendingMcp {
    call_id: Value,
    rpc_id: String,
    operation: McpOperation,
    deadline: tokio::time::Instant,
    workflow: Option<OwnedUrl>,
}
pub(super) async fn owned_mcp_stdio(path: &Path) -> io::Result<()> {
    let startup_started = Instant::now();
    let lease: McpLease = goatr::codex_relay::private_read(path)?;
    private_dir(&lease.directory, false)?;
    let directory = fs::metadata(&lease.directory)?;
    let image = fs::canonicalize(std::env::current_exe()?)?;
    let actual_image = fs::metadata(&image)?;
    if lease.v != 1
        || (directory.dev(), directory.ino()) != lease.directory_object
        || image != lease.image
        || actual_image.uid() != uid()
        || actual_image.mode() & 0o7777 != 0o500
        || actual_image.nlink() != 1
        || binary_sha256(&image)? != lease.image_sha256
    {
        return Err(failure("owned MCP lease/image identity rejected"));
    }
    let process = herdr::process_evidence(std::process::id())?;
    let service = Uuid::random();
    let status_path = lease.directory.join(format!("service-{}.json", service.0));
    let mut input = tokio::io::BufReader::new(McpStdin::open()?);
    let cwd = std::env::current_dir()?;
    let startup_ms = u64::try_from(startup_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    // This is a completed image/lease/process check, not MCP initialize or tool readiness.
    goatr::codex_relay::private_write(
        &status_path,
        &json!({"runId":lease.run,"service":service,"server":lease.server,"process":process,
            "cwd":cwd,"imageSha256":lease.image_sha256,"verifiedStartup":true,
            "verifiedStartupElapsedMs":startup_ms,"initialized":false,"toolsListed":false,
            "form":false,"url":false}),
        false,
    )?;
    let mut initialized = false;
    let mut form = false;
    let mut url = false;
    let mut listed = false;
    let mut pending: Option<PendingMcp> = None;
    let mut phase = "read_frame";
    let mut last_operation: Option<Uuid> = None;
    let mut output = tokio::io::stdout();
    let outcome = async {
    loop {
        phase = "read_frame";
        let incoming = match mcp_input(&mut input, pending.as_ref().map(|pending| pending.deadline)).await {
            Ok(value) => value?,
            Err(()) => {
                phase = "elicitation_deadline";
                return Err(failure("owned native MCP elicitation deadline; no success inferred"));
            }
        };
        let Some(mut value) = incoming else {
            if pending.is_some() { return Err(failure("owned MCP connection closed before elicitation outcome")); }
            return Ok(());
        };
        if value["jsonrpc"].as_str() != Some("2.0") {
            return Err(failure("owned MCP JSON-RPC version rejected"));
        }
        let id = value.get_mut("id").map(Value::take);
        if let Some(id) = &id {
            NativeRpcId::from_raw(&id.to_string()).map_err(io::Error::other)?;
        }
        if value.get("method").is_some()
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            phase = "request_envelope";
            return Err(failure("owned MCP request contains response fields"));
        }
        if value.get("method").is_none() {
            phase = "elicitation_response";
            let Some(mut active) = pending.take() else {
                return Err(failure("owned MCP unsolicited response"));
            };
            if id.as_ref().and_then(Value::as_str) != Some(active.rpc_id.as_str()) {
                return Err(failure("owned MCP elicitation response correlation changed"));
            }
            phase = "elicitation_envelope";
            if value.get("result").is_some() == value.get("error").is_some() {
                return Err(failure("owned MCP response must contain exactly one result or error"));
            }
            if value.get("error").is_some() {
                return Err(failure("owned MCP elicitation returned a native error"));
            }
            let current: McpLease = goatr::codex_relay::private_read(path)?;
            mcp_owner(&current)?;
            let action = value["result"]["action"].as_str()
                .filter(|action| matches!(*action,"accept"|"decline"|"cancel"));
            if action.is_none() {
                return Err(failure("owned MCP elicitation result action is malformed"));
            }
            let mut applied = match active.workflow.take() {
                Some(workflow) => workflow.close().await?,
                None => false,
            };
            let mut accepted = false;
            if action == Some("accept") && active.operation.kind == McpKind::Form {
                phase = "form_apply";
                let content = &value["result"]["content"];
                let label = content["label"].as_str().filter(|label| !label.is_empty() && label.chars().count() <= 64)
                    .ok_or_else(|| failure("owned MCP accepted form lacks its actual required label"))?;
                if content.as_object().is_none_or(|fields| fields.len() != 1) {
                    return Err(failure("owned MCP accepted unexpected form fields"));
                }
                goatr::codex_relay::private_write(
                    &lease.directory.join(format!("label-{}.json", active.operation.operation.0)),
                    &json!({"label":label}), false)?;
                applied = true;
                accepted = true;
            } else if action == Some("accept") && active.operation.kind == McpKind::Url {
                accepted = applied;
            }
            let stage = match action {
                Some("decline") => "declined",
                Some("cancel") => "cancelled",
                Some("accept") if accepted => "accepted",
                _ => "native_error_or_incomplete",
            };
            goatr::codex_relay::private_write(
                &lease.directory.join(format!("result-{}.json", active.operation.operation.0)),
                &json!({"runId":lease.run,"operation":active.operation.operation,"kind":active.operation.kind,
                    "service":service,"stage":stage,"action":action,"effectApplied":applied,
                    "ownerVerified":true,"process":process,"payloads":"not retained"}), true)?;
            last_operation = None;
            phase = "tool_result";
            mcp_output(&mut output, json!({"jsonrpc":"2.0","id":active.call_id,
                "result":{"content":[{"type":"text","text":match stage {
                    "declined"=>"Owned label request declined; no label success is claimed.",
                    "cancelled"=>"Owned label request cancelled; no label success is claimed.",
                    "accepted"=>"The actual owned label workflow completed.",
                    _=>"Owned label workflow did not complete.",
                }}],"isError":stage=="native_error_or_incomplete"}})).await?;
            continue;
        }
        let method = value["method"].as_str().ok_or_else(|| failure("owned MCP method shape"))?;
        if id.is_none() {
            if method == "notifications/initialized" { continue; }
            if method == "notifications/cancelled" {
                if let Some(active) = &pending
                    && value["params"]["requestId"] == active.call_id
                {
                    return Err(failure("owned native MCP tool call cancelled; no completion inferred"));
                }
                continue;
            }
            continue;
        }
        let id = id.expect("request ID present");
        let result = match method {
            "initialize" if !initialized => {
                phase = "initialize";
                let version = value["params"]["protocolVersion"].as_str()
                    .filter(|version| matches!(*version, "2024-11-05"|"2025-03-26"|"2025-06-18"|"2025-11-25"))
                    .ok_or_else(|| prerequisite("owned MCP client protocol version unsupported"))?;
                let capabilities = &value["params"]["capabilities"]["elicitation"];
                form = capabilities.get("form").is_some_and(Value::is_object)
                    || capabilities.as_object().is_some_and(serde_json::Map::is_empty);
                url = capabilities.get("url").is_some_and(Value::is_object);
                initialized = true;
                json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},
                    "serverInfo":{"name":"goatr-owned-acceptance","version":env!("CARGO_PKG_VERSION")}})
            }
            "ping" if initialized => json!({}),
            "tools/list" if initialized => {
                listed = true;
                json!({"tools":[
                    {"name":"label_form","description":"Collect one nonsecret label for the owned acceptance workspace through real form elicitation.","inputSchema":{"type":"object","properties":{"operation":{"type":"string","format":"uuid"}},"required":["operation"],"additionalProperties":false}},
                    {"name":"label_url","description":"Collect one nonsecret label through the real bounded owned local URL workflow.","inputSchema":{"type":"object","properties":{"operation":{"type":"string","format":"uuid"}},"required":["operation"],"additionalProperties":false}}
                ]})
            }
            "tools/call" if initialized && listed && pending.is_none() => {
                phase = "tool_call";
                let kind = match value["params"]["name"].as_str() {
                    Some("label_form") if form => McpKind::Form,
                    Some("label_url") if url => McpKind::Url,
                    _ => return Err(prerequisite("owned native client lacks the requested real elicitation mode")),
                };
                let arguments = &value["params"]["arguments"];
                if arguments.as_object().is_none_or(|args| args.len() != 1) {
                    return Err(failure("owned MCP operation arguments changed"));
                }
                let operation = Uuid::new(arguments["operation"].as_str()
                    .ok_or_else(|| failure("owned MCP operation ID missing"))?).map_err(io::Error::other)?;
                let record: McpOperation = goatr::codex_relay::private_read(
                    &lease.directory.join(format!("operation-{}.json", operation.0)))?;
                if record.run != lease.run || record.operation != operation || record.kind != kind {
                    return Err(failure("owned MCP operation lease changed"));
                }
                phase = "operation_claim";
                let claim = match OpenOptions::new().write(true).create_new(true).mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC)
                    .open(lease.directory.join(format!("claim-{}", operation.0)))
                {
                    Ok(claim)=>claim,
                    Err(error) if error.kind()==io::ErrorKind::AlreadyExists=>{
                        mcp_output(&mut output,json!({"jsonrpc":"2.0","id":id,
                            "error":{"code":-32602,"message":"Owned operation already claimed; never replayed"}})).await?;
                        continue;
                    }
                    Err(error)=>return Err(error),
                };
                last_operation = Some(operation.clone());
                phase = "owner_scope";
                let current: McpLease = goatr::codex_relay::private_read(path)?;
                mcp_owner(&current)?;
                claim.sync_all()?;
                phase = "workflow_setup";
                let workflow = if kind == McpKind::Url {
                    Some(OwnedUrl::start(&operation, &lease.directory, lease.network).await?)
                } else { None };
                let params = if let Some(workflow) = &workflow {
                    json!({"mode":"url","message":format!("Owned nonsecret label request {}", operation.0),
                        "url":workflow.url,"elicitationId":operation})
                } else {
                    json!({"mode":"form","message":format!("Owned nonsecret label request {}", operation.0),
                        "requestedSchema":{"type":"object","properties":{"label":{"type":"string","title":"Owned label","minLength":1,"maxLength":64}},"required":["label"],"additionalProperties":false}})
                };
                let rpc_id = format!("goatr-elicitation-{}", operation.0);
                goatr::codex_relay::private_write(
                    &lease.directory.join(format!("result-{}.json", operation.0)),
                    &json!({"runId":lease.run,"operation":operation,"kind":kind,"service":service,
                        "stage":"requested","action":null,"effectApplied":false,"ownerVerified":true,
                        "urlPort":workflow.as_ref().map(|workflow|workflow.port),
                        "process":process,"payloads":"not retained"}), false)?;
                phase = "elicitation_send";
                mcp_output(&mut output, json!({"jsonrpc":"2.0","id":rpc_id,
                    "method":"elicitation/create","params":params})).await?;
                pending = Some(PendingMcp { call_id: id, rpc_id, operation: record,
                    deadline: tokio::time::Instant::now()+Duration::from_secs(120), workflow });
                continue;
            }
            _ => {
                mcp_output(&mut output, json!({"jsonrpc":"2.0","id":id,
                    "error":{"code":-32601,"message":"Unsupported owned MCP request or busy operation"}})).await?;
                continue;
            }
        };
        goatr::codex_relay::private_write(&status_path,
            &json!({"runId":lease.run,"service":service,"server":lease.server,"process":process,
                "initialized":initialized,"toolsListed":listed,"form":form,"url":url,
                "cwd":cwd,"verifiedStartup":true,"verifiedStartupElapsedMs":startup_ms,
                "imageSha256":lease.image_sha256}), status_path.try_exists()?)?;
        mcp_output(&mut output, json!({"jsonrpc":"2.0","id":id,"result":result})).await?;
    }
    }.await;
    if let Err(error) = &outcome {
        let failure = json!({"runId":lease.run,"service":service,"server":lease.server,
            "process":process,"initialized":initialized,"toolsListed":listed,"form":form,"url":url,
            "cwd":cwd,"verifiedStartup":true,"verifiedStartupElapsedMs":startup_ms,
            "imageSha256":lease.image_sha256,"failed":true,"failureStage":phase,
            "ioKind":format!("{:?}",error.kind()),"payloads":"not retained"});
        goatr::codex_relay::private_write(&status_path, &failure, status_path.try_exists()?)?;
        if let Some(operation) = last_operation {
            let result = lease.directory.join(format!("result-{}.json", operation.0));
            goatr::codex_relay::private_write(
                &result,
                &json!({"runId":lease.run,"operation":operation,"service":service,
                    "stage":"failed","failureStage":phase,"action":null,"effectApplied":null,
                    "ownerVerified":false,"payloads":"not retained"}),
                result.try_exists()?,
            )?;
        }
    }
    outcome
}

enum NativeInput<'a> {
    Prompt(&'a str),
    Keys(&'a [&'a str]),
}
struct NativePlan {
    mode_id: Uuid,
    target: Target,
    original: goatr::codex_relay::CollaborationModeObservation,
    request: Option<Uuid>,
    plan_revision: u64,
    evidence: PathBuf,
}

struct ExpectedShell {
    command: String,
    marker: String,
    deadline: Instant,
    completion: Option<Id>,
    consumed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OmpObserverFailure {
    SocketError(&'static str),
    SocketEof,
    UnsupportedWs(&'static str),
    Decrypt(&'static str),
    JsonParse(&'static str),
    FrameValidation(&'static str),
    Expectation,
    CommandSequence,
    CompletionIdentity,
    TargetChanged,
    ReaderFinished,
    ScopeFailed(&'static str),
    CommandFailed(&'static str),
    CommandDeadline,
}
impl OmpObserverFailure {
    fn io_kind(kind: io::ErrorKind) -> &'static str {
        match kind {
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
        }
    }
    fn frame(frame: &Value) -> Self {
        Self::FrameValidation(match frame["t"].as_str() {
            Some("entry") => "entry",
            Some("event") => "event",
            Some("state") => "state",
            Some("agents") => "agents",
            Some("bus") => "bus",
            Some("ui-request") => "ui-request",
            Some("ui-request-end") => "ui-request-end",
            Some("transcript") => "transcript",
            None => "missing",
            Some(_) => "other",
        })
    }
}
impl std::fmt::Display for OmpObserverFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match self {
            Self::SocketError(_) => "socket_error",
            Self::SocketEof => "socket_eof",
            Self::UnsupportedWs(_) => "unsupported_ws",
            Self::Decrypt(_) => "decrypt",
            Self::JsonParse(_) => "json_parse",
            Self::FrameValidation(_) => "frame_validation",
            Self::Expectation => "expectation",
            Self::CommandSequence => "command_sequence",
            Self::CompletionIdentity => "completion_identity",
            Self::TargetChanged => "target_changed",
            Self::ReaderFinished => "reader_finished",
            Self::ScopeFailed(_) => "scope_failed",
            Self::CommandFailed(_) => "command_failed",
            Self::CommandDeadline => "command_deadline",
        };
        write!(f, "phase={phase}")?;
        match self {
            Self::SocketError(kind)
            | Self::Decrypt(kind)
            | Self::JsonParse(kind)
            | Self::ScopeFailed(kind)
            | Self::CommandFailed(kind) => write!(f, "; io_kind={kind}"),
            Self::UnsupportedWs(family) => write!(f, "; ws_family={family}"),
            Self::FrameValidation(family) => write!(f, "; frame_family={family}"),
            _ => Ok(()),
        }
    }
}
#[derive(Default)]
struct OmpObservation {
    failed: Option<OmpObserverFailure>,
    expected: Option<ExpectedShell>,
    marker_prefix: Option<String>,
    frames: u64,
    bytes: u64,
    completions: u64,
    scope_checks: u64,
}
impl OmpObservation {
    fn healthy(&self) -> io::Result<()> {
        if let Some(first) = self.failed {
            Err(failure(&format!(
                "original OMP observer failed; {first}; no rejoin or replay"
            )))
        } else {
            Ok(())
        }
    }
    fn fail(&mut self, reason: OmpObserverFailure) {
        // This is the first observed failure under the mutex, not necessarily its remote cause.
        self.failed.get_or_insert(reason);
    }
    fn register(&mut self, command: &str, marker: &str, deadline: Instant) -> io::Result<()> {
        self.healthy()?;
        if command.len() > 1024
            || marker.len() > 256
            || Instant::now() >= deadline
            || self
                .expected
                .as_ref()
                .is_some_and(|expected| !expected.consumed)
        {
            self.fail(OmpObserverFailure::Expectation);
            return Err(failure(
                "OMP observer expectation overlap or deadline; no input",
            ));
        }
        let prefix = marker
            .rsplit_once('-')
            .map(|(prefix, _)| format!("{prefix}-"))
            .ok_or_else(|| failure("owned shell marker scope absent"))?;
        if self
            .marker_prefix
            .as_ref()
            .is_some_and(|previous| previous != &prefix)
            || marker != format!("{prefix}{}", self.completions)
        {
            self.fail(OmpObserverFailure::CommandSequence);
            return Err(failure("OMP observer command sequence changed; no replay"));
        }
        self.marker_prefix = Some(prefix);
        self.expected = Some(ExpectedShell {
            command: command.into(),
            marker: marker.into(),
            deadline,
            completion: None,
            consumed: false,
        });
        Ok(())
    }
    fn entry(&mut self, entry: &Value) -> io::Result<()> {
        let id = Id::new(
            entry["id"]
                .as_str()
                .ok_or_else(|| failure("native entry ID absent"))?,
        )
        .map_err(io::Error::other)?;
        let kind = entry["type"]
            .as_str()
            .ok_or_else(|| failure("native entry type absent"))?;
        if kind != "message" {
            return Ok(());
        }
        let message = &entry["message"];
        let role = message["role"]
            .as_str()
            .ok_or_else(|| failure("native message role absent"))?;
        if role != "bashExecution" {
            return Ok(());
        }
        let command = message["command"]
            .as_str()
            .ok_or_else(|| failure("native shell command absent"))?;
        let output = message["output"]
            .as_str()
            .ok_or_else(|| failure("native shell output absent"))?;
        let owned = self
            .marker_prefix
            .as_ref()
            .is_some_and(|prefix| command.contains(prefix) || output.contains(prefix));
        let Some(expected) = self.expected.as_mut() else {
            return Ok(());
        };
        if command != expected.command && !owned {
            return Ok(());
        }
        if command != expected.command
            || expected.completion.is_some()
            || Instant::now() >= expected.deadline
            || output
                .lines()
                .filter(|line| *line == expected.marker)
                .count()
                != 1
            || message["exitCode"].as_i64() != Some(0)
            || message["cancelled"].as_bool() != Some(false)
            || message["truncated"].as_bool() != Some(false)
        {
            return Err(failure(
                "original native shell completion was duplicate or invalid; no replay",
            ));
        }
        expected.completion = Some(id);
        self.completions += 1;
        Ok(())
    }
    fn frame(&mut self, frame: &Value, bytes: usize) -> io::Result<()> {
        self.healthy()?;
        self.frames = self.frames.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes as u64);
        let result = match frame["t"].as_str() {
            Some("entry") => self.entry(&frame["entry"]),
            Some("event") if frame["event"].is_object() => Ok(()),
            Some("state") if frame["state"].is_object() => Ok(()),
            Some("agents") if frame["agents"].is_array() => Ok(()),
            Some("bus") if frame["channel"].is_string() && frame.get("data").is_some() => Ok(()),
            Some("ui-request") if frame["request"].is_object() => Ok(()),
            Some("ui-request-end") if frame["reqId"].as_u64().is_some() => Ok(()),
            Some("transcript")
                if frame["reqId"].as_u64().is_some()
                    && frame["text"].is_string()
                    && frame["newSize"].as_u64().is_some() =>
            {
                Ok(())
            }
            _ => Err(failure(
                "original OMP observer malformed/replaced/rejected stream",
            )),
        };
        if result.is_err() {
            self.fail(OmpObserverFailure::frame(frame));
        }
        result
    }
    fn completion(&self, command: &str, marker: &str) -> io::Result<Option<Id>> {
        self.healthy()?;
        let expected = self
            .expected
            .as_ref()
            .ok_or_else(|| failure("native shell expectation absent"))?;
        if expected.command != command
            || expected.marker != marker
            || Instant::now() >= expected.deadline
        {
            return Err(failure(
                "original native shell observation deadline/scope changed; no replay",
            ));
        }
        Ok(expected.completion.clone())
    }
    fn consume(&mut self, id: &Id) -> io::Result<()> {
        self.healthy()?;
        let expected = self
            .expected
            .as_mut()
            .ok_or_else(|| failure("native shell expectation absent"))?;
        if expected.completion.as_ref() != Some(id) || expected.consumed {
            self.fail(OmpObserverFailure::CompletionIdentity);
            return Err(failure("native completion was not uniquely observed"));
        }
        expected.consumed = true;
        Ok(())
    }
}
type OmpGuest =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
#[derive(Default)]
struct OmpInitial {
    boundary: Option<goatr::backends::omp::SnapshotBoundary>,
    header: Option<(String, String, u64)>,
    queued_frames: usize,
    queued_bytes: usize,
    observation: OmpObservation,
}
impl OmpInitial {
    fn frame(&mut self, frame: &Value, bytes: usize, session: &str, cwd: &str) -> io::Result<bool> {
        use goatr::backends::omp::{QUEUE_BYTES, QUEUE_FRAMES, SnapshotBoundary};
        match frame["t"].as_str() {
            Some("welcome") if self.boundary.is_none() => {
                if frame["header"]["type"] != "session"
                    || frame["header"]["cwd"].as_str() != Some(cwd)
                {
                    return Err(failure("actual OMP authenticated welcome/header mismatch"));
                }
                self.boundary = Some(SnapshotBoundary::new(frame, session)?);
                self.header = Some((
                    session.into(),
                    cwd.into(),
                    frame["entryCount"]
                        .as_u64()
                        .ok_or_else(|| failure("actual OMP snapshot count absent"))?,
                ));
            }
            Some("snapshot-chunk") if self.boundary.is_some() => {
                let finalized = self
                    .boundary
                    .as_mut()
                    .expect("checked boundary")
                    .chunk(frame)?;
                for entry in frame["entries"]
                    .as_array()
                    .ok_or_else(|| failure("actual OMP snapshot entries absent"))?
                {
                    self.observation.entry(entry)?;
                }
                self.observation.frames += 1;
                self.observation.bytes += bytes as u64;
                return Ok(finalized);
            }
            _ if self.boundary.is_some() => {
                self.queued_frames += 1;
                self.queued_bytes += bytes;
                if self.queued_frames > QUEUE_FRAMES || self.queued_bytes > QUEUE_BYTES {
                    return Err(failure("actual OMP independent snapshot activity overflow"));
                }
                self.observation.frame(frame, bytes)?;
                return Ok(false);
            }
            _ => return Err(failure("actual OMP initial welcome absent")),
        }
        self.observation.frames += 1;
        self.observation.bytes += bytes as u64;
        Ok(false)
    }
}
struct OmpObserver {
    target: Target,
    entry: goatr::backends::omp::RegistryEntry,
    registry: goatr::backends::omp::RegistrySnapshot,
    history: parking_lot::Mutex<goatr::backends::omp::HistoryFile>,
    state: Arc<parking_lot::Mutex<OmpObservation>>,
    task: tokio::task::JoinHandle<()>,
    started: Instant,
}
impl Drop for OmpObserver {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl OmpObserver {
    fn start(
        target: Target,
        entry: goatr::backends::omp::RegistryEntry,
        registry: goatr::backends::omp::RegistrySnapshot,
        history: goatr::backends::omp::HistoryFile,
        guest: OmpGuest,
        link: goatr::backends::omp::Link,
        observation: OmpObservation,
    ) -> Self {
        let state = Arc::new(parking_lot::Mutex::new(observation));
        let task = tokio::spawn(Self::read(guest, link, state.clone()));
        Self {
            target,
            entry,
            registry,
            history: parking_lot::Mutex::new(history),
            state,
            task,
            started: Instant::now(),
        }
    }
    async fn read(
        mut guest: OmpGuest,
        link: goatr::backends::omp::Link,
        state: Arc<parking_lot::Mutex<OmpObservation>>,
    ) {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        // One directly drained bounded frame, no retained live-frame queue or private payloads.
        loop {
            let message = match guest.next().await {
                Some(Ok(message)) => message,
                Some(Err(error)) => {
                    let kind = match error {
                        tokio_tungstenite::tungstenite::Error::Io(error) => {
                            OmpObserverFailure::io_kind(error.kind())
                        }
                        _ => "other",
                    };
                    state.lock().fail(OmpObserverFailure::SocketError(kind));
                    break;
                }
                None => {
                    state.lock().fail(OmpObserverFailure::SocketEof);
                    break;
                }
            };
            let mut bytes = match message {
                Message::Binary(bytes) => bytes.to_vec(),
                Message::Ping(_) | Message::Pong(_) => continue,
                unsupported => {
                    let family = match unsupported {
                        Message::Text(_) => "text",
                        Message::Close(_) => "close",
                        Message::Frame(_) => "frame",
                        _ => "other",
                    };
                    state.lock().fail(OmpObserverFailure::UnsupportedWs(family));
                    break;
                }
            };
            let length = bytes.len();
            let frame = match link.open(&mut bytes) {
                Ok(plain) => {
                    goatr::native_json::parse(plain, goatr::native_json::NATIVE).map_err(|error| {
                        OmpObserverFailure::JsonParse(OmpObserverFailure::io_kind(error.kind()))
                    })
                }
                Err(error) => Err(OmpObserverFailure::Decrypt(OmpObserverFailure::io_kind(
                    error.kind(),
                ))),
            };
            bytes.fill(0);
            let frame = match frame {
                Ok(frame) => frame,
                Err(reason) => {
                    state.lock().fail(reason);
                    break;
                }
            };
            if state.lock().frame(&frame, length).is_err() {
                break;
            }
        }
        state.lock().fail(OmpObserverFailure::ReaderFinished);
    }
    fn healthy(&self, target: &Target) -> io::Result<()> {
        let mut state = self.state.lock();
        if *target != self.target {
            state.fail(OmpObserverFailure::TargetChanged);
        }
        if self.task.is_finished() {
            state.fail(OmpObserverFailure::ReaderFinished);
        }
        state.healthy()
    }
    fn evidence(&self) -> Value {
        let state = self.state.lock();
        json!({"initializations":1,"liveCompletions":state.completions,"frames":state.frames,
            "bytes":state.bytes,"scopeChecks":state.scope_checks,"elapsedMs":self.started.elapsed().as_millis(),
            "healthy":state.failed.is_none() && !self.task.is_finished(),"retainedExpectations":usize::from(state.expected.is_some()),
            "liveQueueFrames":0,"payloads":"not retained"})
    }
}
fn write_native_outcome(intent: &Path, value: &Value) -> io::Result<()> {
    // Both files are exclusive and immutable. A lost terminal record never
    // permits rewriting the intent, adopting an existing result, or replaying input.
    write_json(&intent.with_extension("outcome.json"), value)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeGuardPhase {
    PreInputScope,
    ScopeHerdrSnapshot,
    CodexOwnerEpoch,
    CodexPresence,
    InputIntentEvidence,
    InputOutcomeEvidence,
    PostInputScope,
    HistoryProgressEvidence,
}
impl NativeGuardPhase {
    fn name(self) -> &'static str {
        match self {
            Self::PreInputScope => "pre_input_scope",
            Self::ScopeHerdrSnapshot => "scope_herdr_snapshot",
            Self::CodexOwnerEpoch => "codex_owner_epoch",
            Self::CodexPresence => "codex_presence",
            Self::InputIntentEvidence => "input_intent_evidence",
            Self::InputOutcomeEvidence => "input_outcome_evidence",
            Self::PostInputScope => "post_input_scope",
            Self::HistoryProgressEvidence => "history_progress_evidence",
        }
    }
}
#[derive(Debug)]
struct NativeGuardFailure {
    phase: NativeGuardPhase,
    unsafe_private_path: bool,
    kind: io::ErrorKind,
    publication_pending: bool,
    presence_past: bool,
}
impl std::fmt::Display for NativeGuardFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "owned native guard failed: phase={}; category={}; io_kind={:?}; no replay",
            self.phase.name(),
            if self.publication_pending {
                "publication_pending"
            } else if self.presence_past {
                "presence_past"
            } else if self.unsafe_private_path {
                "unsafe_private_path"
            } else {
                "other"
            },
            self.kind
        )
    }
}
impl std::error::Error for NativeGuardFailure {}

// `request_failure` already replaces the native error with this finite authored
// representation. Keep it when an outer scope adds context so method and static
// request phase remain available without accepting arbitrary `LocatedFailure` text.
fn closed_request_failure(error: &io::Error) -> bool {
    let Some(message) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<super::LocatedFailure>())
        .map(|failure| failure.message.as_str())
    else {
        return false;
    };
    let Some(message) = message
        .strip_prefix("actual operation failed: ")
        .and_then(|message| message.strip_suffix("; no replay"))
    else {
        return false;
    };
    let mut fields = message.split("; ");
    let Some(method) = fields
        .next()
        .and_then(|field| field.strip_prefix("method="))
    else {
        return false;
    };
    let Some(phase) = fields.next().and_then(|field| field.strip_prefix("phase=")) else {
        return false;
    };
    let Some(guard) = fields.next().and_then(|field| field.strip_prefix("guard=")) else {
        return false;
    };
    let Some(kind) = fields
        .next()
        .and_then(|field| field.strip_prefix("io_kind="))
    else {
        return false;
    };
    let Some(frame) = fields.next().and_then(|field| field.strip_prefix("frame=")) else {
        return false;
    };
    let Some(code) = fields.next().and_then(|field| field.strip_prefix("code=")) else {
        return false;
    };
    let closed_close = match fields.next() {
        None => true,
        Some(field) => guard == "frame_read" && matches!(
            field.strip_prefix("close="),
            Some("open" | "version_mismatch" | "transport_error" | "connection_closed"
                | "application_closed" | "reset" | "timed_out" | "locally_closed"
                | "cids_exhausted")
        ),
    };
    closed_close && fields.next().is_none()
        && matches!(
            method,
            "HostSnapshot"
                | "SessionOpen"
                | "SessionHistory"
                | "ActionStatus"
                | "SessionCreate"
                | "RequestRespond"
                | "TurnCancel"
        )
        && matches!(
            phase,
            "host_snapshot"
                | "input_pending_guard"
                | "native_idle"
                | "history_subscription_initial"
                | "history_subscription_completion"
                | "history_subscription_final"
                | "history_paging_snapshot"
                | "history_completion"
                | "marker_history"
                | "dialog_pending"
                | "desktop_dialog_closed"
                | "phone_dialog_closed"
                | "stop_working"
                | "stop_idle"
                | "existing_session_after_restart"
                | "action_status"
                | "create_initial"
                | "create_receipt_lookup"
        )
        && matches!(
            guard,
            "payload_encode"
                | "frame_write"
                | "response_deadline"
                | "frame_read"
                | "payload_decode"
                | "correlation"
        )
        && matches!(
            kind,
            "NotFound"
                | "PermissionDenied"
                | "ConnectionRefused"
                | "ConnectionReset"
                | "ConnectionAborted"
                | "NotConnected"
                | "BrokenPipe"
                | "TimedOut"
                | "UnexpectedEof"
                | "InvalidData"
                | "Other"
        )
        && closed_request_frame(frame)
        && matches!(
            code,
            "None"
                | "Some(InvalidRequest)"
                | "Some(UnsupportedVersion)"
                | "Some(UnsupportedMethod)"
        )
}

fn closed_request_frame(frame: &str) -> bool {
    if frame == "None" {
        return true;
    }
    let Some(frame) = frame
        .strip_prefix("Some(FrameFailure { stage: ")
        .and_then(|frame| frame.strip_suffix(" })"))
    else {
        return false;
    };
    let Some((stage, frame)) = frame.split_once(", io: ") else {
        return false;
    };
    let Some((io, code)) = frame.split_once(", code: ") else {
        return false;
    };
    matches!(
        stage,
        "FrameHeader"
            | "FrameLength"
            | "FrameCap"
            | "FramePayload"
            | "WriteLength"
            | "WriteHeader"
            | "WritePayload"
            | "WriteDeadline"
    ) && matches!(
        io,
        "None"
            | "Some(UnexpectedEof)"
            | "Some(ConnectionReset)"
            | "Some(NotConnected)"
            | "Some(TimedOut)"
            | "Some(Other)"
    ) && matches!(
        code,
        "None" | "Some(InvalidRequest)" | "Some(UnsupportedVersion)" | "Some(UnsupportedMethod)"
    )
}

fn native_guard_context(phase: NativeGuardPhase, error: io::Error) -> io::Error {
    if error
        .get_ref()
        .is_some_and(|inner| inner.is::<NativeGuardFailure>())
        || closed_request_failure(&error)
    {
        return error;
    }
    // Classify only the exact authored guard; discard its original text/source.
    let unsafe_private_path = error.kind() == io::ErrorKind::PermissionDenied
        && error.to_string() == "unsafe private path";
    io::Error::new(
        error.kind(),
        NativeGuardFailure {
            phase,
            unsafe_private_path,
            kind: error.kind(),
            publication_pending: phase == NativeGuardPhase::CodexPresence
                && goatr::codex_relay::publication_unstable(&error),
            presence_past: phase == NativeGuardPhase::CodexPresence
                && error.kind() == io::ErrorKind::InvalidData
                && error.to_string() == "Codex relay presence older than five seconds",
        },
    )
}
fn native_presence_unavailable(error: &io::Error) -> bool {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<NativeGuardFailure>())
        .is_some_and(|failure| {
            failure.phase == NativeGuardPhase::CodexPresence
                && (failure.publication_pending || failure.presence_past)
        })
}
fn scope_observation_waits(
    connectivity: SessionConnectivity,
    presence_unavailable: bool,
    now: Instant,
    deadline: Instant,
) -> bool {
    (connectivity == SessionConnectivity::Synchronizing
        || connectivity == SessionConnectivity::Ready && presence_unavailable)
        && now < deadline
}
struct NativeInputs<'a> {
    enabled: bool,
    paging_proof: bool,
    config: &'a Config,
    socket: &'a Path,
    herdr: &'a ProcessEvidence,
    evidence: &'a Path,
    run: &'a Uuid,
    mcp: Option<&'a NativeMcp>,
    plan: RefCell<Option<NativePlan>>,
    mcp_dialogs: RefCell<HashMap<Uuid, Uuid>>,
}
impl NativeInputs<'_> {
    async fn omp_scope(&self, p: &NativeProof, pane: &herdr::Pane) -> io::Result<()> {
        let observer = p
            .omp_observer
            .as_ref()
            .ok_or_else(|| failure("original OMP observer absent"))?;
        observer.healthy(&p.target)?;
        // The originally captured authenticated entry is mandatory. An unrelated
        // dead registry socket must not substitute for, or invalidate, this proof.
        let (selected, selected_owner) = observer.entry.snapshot().await?;
        if selected_owner != p.owner
            || selected.instance != observer.registry.instance
            || selected.generation != observer.registry.generation
            || selected.pid != observer.registry.pid
            || selected.session != observer.registry.session
            || selected.cwd != observer.registry.cwd
            || !selected.control
            || !selected.relay_connected
        {
            return Err(failure("original OMP registry binding changed"));
        }
        let registry = self.config.omp_registry_dir.as_ref().ok_or_else(|| {
            failure("actual OMP registry not configured; no cached native authority")
        })?;
        let info = Client::connect(self.socket)
            .await?
            .process_info(&p.pane)
            .await?;
        let mut matched = None;
        for entry in goatr::backends::omp::registry(registry)? {
            let (record, owner) = if entry == observer.entry {
                (selected.clone(), selected_owner.clone())
            } else {
                let Ok(candidate) = entry.snapshot().await else {
                    continue;
                };
                candidate
            };
            if record.session != p.native.0 {
                continue;
            }
            if matched.is_some()
                || owner != p.owner
                || p.objects["instance"].as_str() != Some(record.instance.as_str())
                || record.generation != p.objects["roomGeneration"].as_u64().unwrap_or_default()
                || record.pid != observer.registry.pid
                || record.cwd != observer.registry.cwd
                || !record.control
                || !record.relay_connected
                || goatr::backends::omp::terminal_owner(self.herdr, &owner, &info).is_err()
            {
                return Err(failure(
                    "owned OMP current registry/owner binding changed or became ambiguous",
                ));
            }
            matched = Some((record, owner));
        }
        let (record, owner) =
            matched.ok_or_else(|| failure("owned OMP current registry binding disappeared"))?;
        let path = pane
            .agent_session
            .as_ref()
            .filter(|session| session.agent == "omp" && session.kind == "path")
            .map(|session| PathBuf::from(&session.value))
            .ok_or_else(|| failure("owned OMP current history path changed"))?;
        if p.objects["historyPath"].as_str() != path.to_str()
            || record.session != p.objects["nativeHeader"]["id"].as_str().unwrap_or_default()
        {
            return Err(failure("owned OMP current history/header identity changed"));
        }
        let runtime = herdr::process_object(&owner, &self.config.runtime_dir_agent)?;
        let actual = fs::metadata(&self.config.runtime_dir)?;
        if runtime != (actual.dev(), actual.ino()) || p.objects["runtime"] != json!(runtime) {
            return Err(failure("owned OMP integration object changed"));
        }
        // Keep the captured inode/header, including one legitimate lazy materialization;
        // refresh scans only appended bytes instead of reconstructing growing history.
        observer.history.lock().refresh()?;
        observer.healthy(&p.target)?;
        observer.state.lock().scope_checks += 1;
        Ok(())
    }
    fn server_matches(&self, p: &NativeProof, actual: &goatr::storage::ServerEvidence) -> bool {
        let expected = &p.objects["herdr"];
        actual.pid == self.herdr.pid
            && actual.pid_start_ticks == self.herdr.start_ticks
            && actual.linux_boot_id == self.herdr.linux_boot_id
            && expected["linux_boot_id"].as_str() == Some(actual.linux_boot_id.0.as_str())
            && expected["pid"].as_u64() == Some(u64::from(actual.pid))
            && expected["pid_start_ticks"].as_u64() == Some(actual.pid_start_ticks)
            && expected["socket_device"].as_u64() == Some(actual.socket_device)
            && expected["socket_inode"].as_u64() == Some(actual.socket_inode)
    }
    async fn current(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
    ) -> io::Result<(Snapshot, NativeProof)> {
        let deadline = Instant::now() + Duration::from_secs(30);
        self.current_until(phone, p, deadline).await
    }
    async fn current_until(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        deadline: Instant,
    ) -> io::Result<(Snapshot, NativeProof)> {
        let phase = Cell::new("process_proof");
        let expired_on_entry = Instant::now() >= deadline;
        let provider = match p.provider {
            Provider::Omp => "omp",
            Provider::Codex => "codex",
        };
        let result = tokio::time::timeout_at(
            deadline.into(),
            self.current_checked(phone, p, deadline, &phase),
        )
        .await
        .map_err(|_| {
            failure(&format!(
                "original native scope deadline; provider={provider}; phase={}; expired_on_entry={expired_on_entry}; no replay",
                phase.get(),
            ))
        })
        .and_then(|result| result);
        if let Err(error) = &result
            && let Some(observer) = &p.omp_observer
        {
            observer.state.lock().fail(OmpObserverFailure::ScopeFailed(
                OmpObserverFailure::io_kind(error.kind()),
            ));
        }
        result
    }
    async fn current_checked(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        deadline: Instant,
        phase: &Cell<&'static str>,
    ) -> io::Result<(Snapshot, NativeProof)> {
        loop {
            phase.set("process_proof");
            if let Some(observer) = &p.omp_observer {
                observer.healthy(&p.target)?;
            }
            if herdr::process_evidence(self.herdr.pid)? != *self.herdr {
                return Err(failure(
                    "owned native input Herdr process changed; no replacement",
                ));
            }
            p.recheck_processes()?;
            phase.set("host_snapshot");
            let inventory = phone.host().await?;
            let session = inventory
                .sessions
                .iter()
                .find(|s| s.session_id == p.target.session_id)
                .ok_or_else(|| failure("owned native input Target disappeared"))?;
            if target(session) != p.target
                || session.provider != p.provider
                || session.native_id != p.native
                || session.pane_id != p.pane
                || session.terminal_id != p.terminal
                || session.workspace_id != p.workspace
            {
                let field = if target(session) != p.target {
                    "target"
                } else if session.provider != p.provider {
                    "provider"
                } else if session.native_id != p.native {
                    "native_id"
                } else if session.pane_id != p.pane {
                    "pane"
                } else if session.terminal_id != p.terminal {
                    "terminal"
                } else {
                    "workspace"
                };
                return Err(failure(&format!(
                    "owned native input routing guard: provider={:?}; field={field}; no retarget",
                    p.provider,
                )));
            }
            phase.set("herdr_snapshot");
            let (server, snapshot) = surface(self.socket).await.map_err(|error| {
                native_guard_context(NativeGuardPhase::ScopeHerdrSnapshot, error)
            })?;
            if !self.server_matches(p, &server) {
                return Err(failure(
                    "owned native input Herdr socket/incarnation changed",
                ));
            }
            let pane = snapshot.pane(&p.pane)?;
            let kind = match p.provider {
                Provider::Omp => "omp",
                Provider::Codex => "codex",
            };
            if pane.terminal_id != p.terminal
                || pane.workspace_id != p.workspace
                || pane.agent.as_deref() != Some(kind)
            {
                return Err(failure(
                    "owned native input current public pane/occupant changed",
                ));
            }
            let mut presence_unavailable = false;
            if session.connectivity == SessionConnectivity::Ready {
                let fresh = if p.provider == Provider::Omp {
                    phase.set("omp_scope");
                    self.omp_scope(p, pane).await?;
                    Some(p.clone())
                } else {
                    phase.set("codex_proof");
                    match proof(self.config, self.socket, session, None).await {
                        Ok(fresh) => Some(fresh),
                        Err(error) if native_presence_unavailable(&error) => {
                            presence_unavailable = true;
                            None
                        }
                        Err(error) => return Err(error),
                    }
                };
                if let Some(fresh) = fresh {
                    if p.provider == Provider::Codex && !p.same_input_binding(&fresh) {
                        return Err(failure("owned native input original source epoch changed"));
                    }
                    if p.provider == Provider::Codex && fresh.collaboration.is_none() {
                        return Err(prerequisite(
                            "actual current TUI expected-thread correlation/mode missing; no native input submitted",
                        ));
                    }
                    return Ok((snapshot, fresh));
                }
            }
            phase.set("observation_wait");
            if !scope_observation_waits(
                session.connectivity,
                presence_unavailable,
                Instant::now(),
                deadline,
            ) {
                return Err(prerequisite(
                    "owned native input authoritative Ready unavailable; no input submitted",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn send(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        input: NativeInput<'_>,
        pending: Option<&PendingRequest>,
        purpose: &'static str,
    ) -> io::Result<()> {
        if !self.enabled {
            return Err(failure(
                "native input automation was not consented for this invocation",
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        phone.command_phase(HistoryCommandPhase::PreScope);
        let (before, _) = self
            .current_until(phone, p, deadline)
            .await
            .map_err(|error| native_guard_context(NativeGuardPhase::PreInputScope, error))?;
        let pane = before.pane(&p.pane)?;
        if let Some(pending) = pending
            && !phone
                .open(&p.target, "input_pending_guard")
                .await?
                .pending_requests
                .iter()
                .any(|r| r.request_key == pending.request_key && r.kind == pending.kind)
        {
            return Err(failure(
                "owned native dialog resolved or changed before desktop input",
            ));
        }
        phone.command_phase(HistoryCommandPhase::PreGeometry);
        let before_geometry = geometry(self.socket, &pane.pane_id, GeometryPhase::InputBefore)
            .await
            .map_err(GeometryFailure::into_io)?;
        let client = Client::connect(self.socket).await?;
        if !self.server_matches(p, &client.evidence) {
            return Err(failure(
                "owned native input socket changed before submission",
            ));
        }
        let (method, params, response, acknowledgment) = match input {
            NativeInput::Prompt(text) => (
                "agent.prompt",
                json!({"target":pane.pane_id,"text":text}),
                "agent_prompted",
                "pty_text_and_enter_written",
            ),
            NativeInput::Keys(keys) => (
                "agent.send_keys",
                json!({"target":pane.pane_id,"keys":keys}),
                "ok",
                "queue_admitted",
            ),
        };
        let input_id = Uuid::random();
        let evidence = self
            .evidence
            .join(format!("native-input-{}-{}.json", self.run.0, input_id.0));
        let record = |state, category| {
            let value = json!({
                "runId":self.run,"inputId":input_id,"purpose":purpose,"method":method,
                "target":p.target,"provider":p.provider,"nativeId":p.native,
                "publicPane":pane.pane_id,"terminal":p.terminal,"workspace":p.workspace,
                "herdr":self.herdr,"owner":p.owner,"tui":p.tui,"relay":p.relay,
                "nativeSourceEpoch":match p.provider {
                    Provider::Omp => &p.objects["roomGeneration"],
                    Provider::Codex => &p.objects["connectionEpoch"],
                },
                "state":state,"category":category,"acknowledgment":acknowledgment,
                "scopeBefore":true,"scopeAfter":state=="acknowledged",
                "nativeEffect":"requires independent observation","payloads":"not retained",
                "uncertainRule":"never retry or redirect an unconfirmed input"
            });
            if state == "dispatching" {
                write_json(&evidence, &value).map_err(|error| {
                    native_guard_context(NativeGuardPhase::InputIntentEvidence, error)
                })
            } else {
                write_native_outcome(&evidence, &value).map_err(|error| {
                    native_guard_context(NativeGuardPhase::InputOutcomeEvidence, error)
                })
            }
        };
        if Instant::now() >= deadline {
            return Err(failure(
                "original native input deadline passed before dispatch; no input",
            ));
        }
        record("dispatching", "unconfirmed")?;
        phone.command_phase(HistoryCommandPhase::Dispatch);
        let mut result = match client.request(method, params, response).await {
            Ok(result) => result,
            Err(error) => {
                let category = match error {
                    herdr::ApiError::Rejected(_) => "native_rejected_or_partial",
                    herdr::ApiError::Changed(_) => "source_changed",
                    herdr::ApiError::Unavailable(_) => "transport_or_deadline",
                };
                record("uncertain", category)?;
                return Err(failure(
                    "owned native input outcome uncertain; work retained, no replay",
                ));
            }
        };
        if method == "agent.prompt" {
            let agent = result
                .get_mut("agent")
                .map(Value::take)
                .and_then(|value| serde_json::from_value::<herdr::Agent>(value).ok());
            let kind = match p.provider {
                Provider::Omp => "omp",
                Provider::Codex => "codex",
            };
            if agent.as_ref().is_none_or(|agent| {
                agent.pane_id != pane.pane_id
                    || agent.terminal_id != pane.terminal_id
                    || agent.workspace_id != pane.workspace_id
                    || agent.tab_id != pane.tab_id
                    || agent.agent.as_deref() != Some(kind)
                    || agent.agent_session != pane.agent_session
                    || agent.launch_pending
            }) {
                record("uncertain", "acknowledgment_identity")?;
                return Err(failure(
                    "owned native input acknowledgment identity changed; no replay",
                ));
            }
        }
        phone.command_phase(HistoryCommandPhase::PostScope);
        let after = match self.current_until(phone, p, deadline).await {
            Ok((after, _)) => after,
            Err(error) => {
                record("uncertain", "post_input_scope")?;
                return Err(native_guard_context(
                    NativeGuardPhase::PostInputScope,
                    error,
                ));
            }
        };
        phone.command_phase(HistoryCommandPhase::PostGeometry);
        let after_geometry =
            match geometry(self.socket, &pane.pane_id, GeometryPhase::InputAfter).await {
                Ok(value) => value,
                Err(error) => {
                    record("uncertain", "post_input_geometry")?;
                    return Err(error.into_io());
                }
            };
        if focus(&before) != focus(&after) || before_geometry != after_geometry {
            record("uncertain", "focus_or_geometry_changed")?;
            return Err(failure(
                "owned native input changed desktop focus/geometry; no restoration or replay",
            ));
        }
        record("acknowledged", "same_owned_source")?;
        phone.command_phase(HistoryCommandPhase::InputAcknowledged);
        Ok(())
    }
    async fn idle(&self, phone: &mut Phone, p: &NativeProof) -> io::Result<SessionOpenResult> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            p.recheck_processes()?;
            let state = phone.open(&p.target, "native_idle").await?;
            if state.state.summary.native_id != p.native {
                return Err(failure(
                    "owned native idle observation changed conversation",
                ));
            }
            if state.state.summary.connectivity == SessionConnectivity::Ready
                && state.state.summary.status == SessionStatus::Idle
                && state.pending_requests.is_empty()
            {
                return Ok(state);
            }
            if Instant::now() >= deadline {
                return Err(prerequisite(
                    "same owned native conversation did not become Ready/Idle without a pending modal",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn mode(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
    ) -> io::Result<(goatr::codex_relay::CollaborationModeObservation, bool)> {
        let (_, current) = self.current(phone, p).await?;
        let mode = current.collaboration.ok_or_else(|| prerequisite(
            "actual correlated native Codex collaboration mode was not observed; no Default is inferred"))?;
        Ok((mode, current.default_plan == Some(true)))
    }
    async fn mode_after(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        wanted: goatr::codex_relay::CollaborationModeKind,
        revision: u64,
    ) -> io::Result<goatr::codex_relay::CollaborationModeObservation> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let (current, _) = self.mode(phone, p).await?;
            if current.mode == wanted && current.from_settings && current.revision > revision {
                return Ok(current);
            }
            if Instant::now() >= deadline {
                return Err(prerequisite(
                    "actual same-source native settings update did not prove requested mode kind; no repeated toggle",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn trigger_dialog(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        kind: PresentationKind,
        chain_editor: bool,
        previous_select: Option<&Uuid>,
    ) -> io::Result<PendingRequest> {
        if !self.enabled {
            human(&format!(
                "Trigger actual {kind:?} in recorded {:?} native desktop session {}",
                p.provider, p.native.0
            ))
            .await?;
            return wait_dialog(phone, p, kind, None, None).await;
        }
        if p.provider == Provider::Omp {
            if !matches!(
                kind,
                PresentationKind::OmpSelect | PresentationKind::OmpEditor
            ) {
                return Err(failure("owned OMP trigger provider/kind mismatch"));
            }
            if kind == PresentationKind::OmpEditor && chain_editor {
                let previous = previous_select.ok_or_else(|| {
                    failure("owned editor has no observed preparatory Other select")
                })?;
                return wait_dialog(phone, p, kind, Some(previous), None).await;
            }
            self.idle(phone, p).await?;
            let other = chain_editor || kind == PresentationKind::OmpEditor;
            let options = if other {
                json!([])
            } else {
                json!([{"label":"Amber"},{"label":"Blue"}])
            };
            let arguments = json!({"questions":[{"id":"owned_choice",
                "question":"Choose the nonsecret owned acceptance label","options":options,"multi":false}]});
            let prompt = format!(
                "Invoke the installed native ask tool exactly once with these arguments: {arguments}. Leave its actual interactive UI pending for the user. Do not answer it yourself, use other tools, change files, or create/switch sessions."
            );
            self.send(phone, p, NativeInput::Prompt(&prompt), None, "omp_ask")
                .await?;
            let select = wait_dialog(phone, p, PresentationKind::OmpSelect, None, None).await?;
            if other {
                let Presentation::OmpSelect(actual) = &select.presentation else {
                    return Err(failure("actual ask selection presentation changed"));
                };
                if actual.options.len() != 2
                    || actual.options[0].value != "Other (type your own)"
                    || actual.options[1].value != "Chat about this"
                    || actual.markable_count != Some(Uint(0))
                {
                    return Err(failure(
                        "native ask did not produce the actual empty-options Other/editor sequence",
                    ));
                }
            }
            if kind == PresentationKind::OmpSelect {
                return Ok(select);
            }
            let receipt = receipt_result(
                phone
                    .request(
                        Uuid::random(),
                        RequestBody::RequestRespond(RequestRespondParams {
                            boot_id: phone.hello.boot_id.clone(),
                            target: p.target.clone(),
                            request_key: select.request_key.clone(),
                            answer: answer(&select)?,
                        }),
                    )
                    .await?
                    .map_err(|error| {
                        failure(&format!(
                            "actual preparatory Other selection rejected: {:?}",
                            error.code
                        ))
                    })?,
            )?;
            if receipt.state != ReceiptState::Dispatched
                || receipt.error.is_some()
                || receipt.target.as_ref() != Some(&p.target)
            {
                return Err(failure(
                    "actual preparatory Other selection did not dispatch; no retry",
                ));
            }
            return wait_dialog(phone, p, kind, Some(&select.request_key), None).await;
        }
        self.idle(phone, p).await?;
        let mut mcp_operation = None;
        let prompt = match kind {
            PresentationKind::CodexApproval => {
                let marker = format!("goatr-owned-approval-{}", &self.run.0[..8]);
                let arguments = json!({"cmd":format!("printf '%s\\n' '{marker}'"),
                    "sandbox_permissions":"require_escalated",
                    "justification":"Allow this owned nonsecret printf acceptance command?"});
                format!(
                    "Invoke the actual exec_command tool once with exactly {arguments}. Wait for its real native approval; do not approve it yourself. Do not use a different command, write files, access the network, or create/switch sessions."
                )
            }
            PresentationKind::CodexQuestions => {
                if self.plan.borrow().is_some() {
                    return Err(failure(
                        "an original native mode transition is still unresolved",
                    ));
                }
                let (original, cycle) = self.mode(phone, p).await?;
                if original.mode == goatr::codex_relay::CollaborationModeKind::Default && !cycle {
                    return Err(prerequisite(
                        "actual native Default/Plan catalogue pair unavailable; no speculative mode change",
                    ));
                }
                let mode_id = Uuid::random();
                let path = self
                    .evidence
                    .join(format!("native-mode-{}-{}.json", self.run.0, mode_id.0));
                write_json(
                    &path,
                    &json!({"runId":self.run,"modeId":mode_id,"target":p.target,
                    "nativeId":p.native,"owner":p.owner,"tui":p.tui,"relay":p.relay,
                    "connectionEpoch":p.objects["connectionEpoch"],"originalMode":original,
                    "state":"entering_plan","restorationScope":"mode_kind_only","fullSettingsRestorationClaimed":false}),
                )?;
                *self.plan.borrow_mut() = Some(NativePlan {
                    mode_id,
                    target: p.target.clone(),
                    original,
                    request: None,
                    plan_revision: original.revision,
                    evidence: path,
                });
                let arguments = json!({"questions":[{"id":"owned_choice","header":"Owned choice",
                    "question":"Which nonsecret label should this owned acceptance use?",
                    "options":[{"label":"Amber","description":"Use Amber"},{"label":"Blue","description":"Use Blue"}]}]});
                format!(
                    "/plan Invoke the installed request_user_input tool once with exactly {arguments}. Leave the real question pending. Do not write files, execute shell commands, or create/switch sessions."
                )
            }
            PresentationKind::CodexForm | PresentationKind::CodexUrl => {
                let mcp = self.mcp.ok_or_else(|| {
                    prerequisite("real owned native MCP service is not registered")
                })?;
                let requested = if kind == PresentationKind::CodexForm {
                    McpKind::Form
                } else {
                    McpKind::Url
                };
                mcp.ready(requested).await?;
                let operation = mcp.operation(p, requested, self.run)?;
                let tool = if requested == McpKind::Form {
                    "label_form"
                } else {
                    "label_url"
                };
                let prompt = format!(
                    "Invoke the real registered MCP tool mcp__{}__{tool} exactly once with {{\"operation\":\"{}\"}}. It genuinely needs a nonsecret owned-workspace label and will elicit it. Leave the native request pending. Do not supply an answer yourself, use other tools, alter files, or create/switch sessions.",
                    mcp.server, operation.0
                );
                mcp_operation = Some(operation);
                prompt
            }
            _ => return Err(failure("owned Codex trigger provider/kind mismatch")),
        };
        self.send(
            phone,
            p,
            NativeInput::Prompt(&prompt),
            None,
            "native_dialog_trigger",
        )
        .await?;
        if kind == PresentationKind::CodexQuestions {
            let original = self
                .plan
                .borrow()
                .as_ref()
                .expect("recorded native mode intent")
                .original;
            let plan = if original.mode == goatr::codex_relay::CollaborationModeKind::Plan {
                self.mode(phone, p).await?.0
            } else {
                self.mode_after(
                    phone,
                    p,
                    goatr::codex_relay::CollaborationModeKind::Plan,
                    original.revision,
                )
                .await?
            };
            if plan.mode != goatr::codex_relay::CollaborationModeKind::Plan {
                return Err(failure("actual native question mode is not Plan"));
            }
            self.plan
                .borrow_mut()
                .as_mut()
                .expect("recorded native mode intent")
                .plan_revision = plan.revision;
        }
        let request = wait_dialog(
            phone,
            p,
            kind,
            None,
            mcp_operation
                .as_ref()
                .map(|operation| (self.mcp.expect("owned MCP"), operation)),
        )
        .await?;
        if kind == PresentationKind::CodexQuestions {
            let Presentation::CodexQuestions(questions) = &request.presentation else {
                return Err(failure("actual native questions shape changed"));
            };
            if !questions.is_blocking
                || questions.questions.len() != 1
                || questions.questions[0].id.0 != "owned_choice"
                || questions.questions[0].is_secret
                || questions.questions[0].options.len() != 2
                || questions.questions[0].options[0].label.0 != "Amber"
                || questions.questions[0].options[1].label.0 != "Blue"
            {
                return Err(failure(
                    "actual native question ID/options/blocking state differ from owned invocation",
                ));
            }
            self.plan
                .borrow_mut()
                .as_mut()
                .expect("recorded native mode intent")
                .request = Some(request.request_key.clone());
        }
        if let Some(operation) = mcp_operation {
            let mcp = self.mcp.expect("owned MCP");
            let metadata: Value = goatr::codex_relay::private_read(
                &mcp.directory.join(format!("result-{}.json", operation.0)),
            )?;
            let title_matches = |content: &Content| {
                matches!(content,
                Content::Text(text) if text.text.0.contains(&operation.0))
            };
            let valid = match &request.presentation {
                Presentation::CodexForm(form) => {
                    title_matches(&form.title)
                        && form.fields.len() == 1
                        && form.fields[0].id.0 == "label"
                        && form.fields[0].required
                        && form.fields[0].r#type == FieldType::String
                }
                Presentation::CodexUrl(url) => {
                    title_matches(&url.title)
                        && metadata["urlPort"].as_u64().is_some_and(|port| {
                            url.url == format!("http://127.0.0.1:{port}/{}", operation.0)
                        })
                }
                _ => false,
            };
            if !valid || metadata["ownerVerified"] != true {
                return Err(failure(
                    "actual native elicitation did not match the owned protocol service operation",
                ));
            }
            self.mcp_dialogs
                .borrow_mut()
                .insert(request.request_key.clone(), operation);
        }
        Ok(request)
    }
    async fn finish_dialog(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        request: &Uuid,
        desktop: bool,
    ) -> io::Result<()> {
        let operation = self.mcp_dialogs.borrow_mut().remove(request);
        if let Some(operation) = operation {
            let proof = self
                .mcp
                .ok_or_else(|| failure("owned MCP counterpart missing"))?
                .observed_decline(&operation, desktop)
                .await?;
            write_json(
                &self.evidence.join(format!(
                    "native-mcp-result-{}-{}.json",
                    self.run.0, operation.0
                )),
                &proof,
            )?;
        }
        let restore = self
            .plan
            .borrow()
            .as_ref()
            .is_some_and(|plan| plan.request.as_ref() == Some(request));
        if !restore {
            return Ok(());
        }
        let plan = self.plan.borrow_mut().take().expect("original mode record");
        if plan.target != p.target {
            return Err(failure("mode restoration Target changed; no retarget"));
        }
        self.idle(phone, p).await?;
        let (current, cycle) = self.mode(phone, p).await?;
        let restored = if plan.original.mode == goatr::codex_relay::CollaborationModeKind::Plan {
            if current.mode != plan.original.mode {
                return Err(failure("native original Plan mode changed"));
            }
            current
        } else if current.mode == plan.original.mode
            && current.from_settings
            && current.revision > plan.plan_revision
        {
            current
        } else {
            if current.mode != goatr::codex_relay::CollaborationModeKind::Plan || !cycle {
                return Err(prerequisite(
                    "actual native mode/catalogue no longer proves one safe restoration toggle",
                ));
            }
            self.send(
                phone,
                p,
                NativeInput::Keys(&["shift+tab"]),
                None,
                "restore_mode_kind",
            )
            .await?;
            self.mode_after(phone, p, plan.original.mode, current.revision)
                .await?
        };
        write_native_outcome(
            &plan.evidence,
            &json!({"runId":self.run,"modeId":plan.mode_id,"target":p.target,
            "nativeId":p.native,"owner":p.owner,"tui":p.tui,"relay":p.relay,
            "connectionEpoch":p.objects["connectionEpoch"],"originalMode":plan.original,
            "restoredMode":restored,"state":"mode_kind_restored","restorationScope":"mode_kind_only",
            "fullSettingsRestorationClaimed":false}),
        )
    }
    async fn shell_completed(
        &self,
        phone: &mut Phone,
        p: &NativeProof,
        command: &str,
        marker: &str,
    ) -> io::Result<(SessionOpenResult, Id)> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            phone.command_phase(HistoryCommandPhase::Completion);
            p.recheck_processes()?;
            if p.provider == Provider::Codex {
                let view = phone.generation.as_ref().ok_or_else(|| {
                    failure("original native completion subscription missing; no reopen")
                })?;
                if view.failed
                    || target(&view.snapshot.state.summary) != p.target
                    || view.snapshot.state.summary.native_id != p.native
                {
                    return Err(failure("original native completion subscription lost"));
                }
                let trace = phone
                    .history_command
                    .as_mut()
                    .ok_or_else(|| failure("original command context missing"))?;
                if let Some(id) = codex_shell_completion(&view.snapshot, command, marker, trace)? {
                    return Ok((view.snapshot.clone(), id));
                }
                tokio::time::timeout_at(deadline.into(), phone.generation_event())
                    .await
                    .map_err(|_| {
                        failure("original native completion subscription deadline; no replay")
                    })??;
                continue;
            }
            let native_completion = if let Some(observer) = &p.omp_observer {
                observer.healthy(&p.target)?;
                observer.state.lock().completion(command, marker)?
            } else {
                None
            };
            let open = if let Some(view) = &phone.generation {
                if view.failed {
                    return Err(failure("original history subscription failed"));
                }
                view.snapshot.clone()
            } else {
                tokio::time::timeout_at(
                    deadline.into(),
                    phone.open(&p.target, "history_completion"),
                )
                .await
                .map_err(|_| failure("original completion deadline"))??
            };
            if target(&open.state.summary) != p.target || open.state.summary.native_id != p.native {
                return Err(failure(
                    "original native completion Target changed; no replay",
                ));
            }
            if let Some(trace) = &mut phone.history_command {
                trace.ready = open.state.summary.connectivity == SessionConnectivity::Ready;
                trace.idle = open.state.summary.status == SessionStatus::Idle;
                trace.pending_empty = open.pending_requests.is_empty();
                trace.marker_seen = native_completion.is_some();
                trace.completed = native_completion.as_ref().is_some_and(|id| {
                    open.history
                        .iter()
                        .any(|record| record.complete && record.id == *id)
                });
            }
            if open.state.summary.connectivity == SessionConnectivity::Ready
                && open.state.summary.status == SessionStatus::Idle
                && open.pending_requests.is_empty()
                && let Some(id) = native_completion.as_ref()
                && open
                    .history
                    .iter()
                    .any(|record| record.complete && &record.id == id)
            {
                let observer = p
                    .omp_observer
                    .as_ref()
                    .ok_or_else(|| failure("original OMP observer absent"))?;
                observer.healthy(&p.target)?;
                observer.state.lock().consume(id)?;
                return Ok((open, id.clone()));
            }
            if Instant::now() >= deadline {
                return Err(prerequisite(
                    "original owned native shell command completion was not observed; no command replay",
                ));
            }
            if phone.generation.is_some()
                && (native_completion.is_some() || open.state.summary.status != SessionStatus::Idle)
            {
                tokio::time::timeout_at(deadline.into(), phone.generation_event())
                    .await
                    .map_err(|_| {
                        failure("original native completion subscription deadline; no replay")
                    })??;
            } else {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    async fn marker(&self, phone: &mut Phone, p: &NativeProof, marker: &str) -> io::Result<()> {
        let prompt = format!(
            "Reply with exactly {marker} in this same native conversation. Do not write files, use tools, or create or switch sessions."
        );
        if self.enabled {
            self.send(
                phone,
                p,
                NativeInput::Prompt(&prompt),
                None,
                "desktop_marker",
            )
            .await
        } else {
            human(&format!(
                "Send this ordinary prompt in EXACT recorded native session {}: {prompt}",
                p.native.0
            ))
            .await
        }
    }
}
async fn proof(
    config: &Config,
    socket: &Path,
    s: &SessionSummary,
    _shell: Option<(&str, &str)>,
) -> io::Result<NativeProof> {
    proof_selected(config, socket, s, None).await
}
async fn created_codex_proof(
    config: &Config,
    socket: &Path,
    s: &SessionSummary,
    receipt: &Receipt,
    deadline: Instant,
) -> io::Result<NativeProof> {
    if Instant::now() >= deadline {
        return Err(failure("original native scope deadline; no replay"));
    }
    let creation = receipt
        .creation
        .as_ref()
        .filter(|creation| {
            receipt.state == ReceiptState::Succeeded
                && creation.provider == Provider::Codex
                && created_session_matches(&creation.resources, s, Provider::Codex)
        })
        .ok_or_else(|| failure("successful Codex receipt no longer matches its exact session"))?;
    let binding = creation
        .resources
        .binding_id
        .as_ref()
        .ok_or_else(|| failure("successful Codex receipt has no exact binding"))?;
    let proof = tokio::time::timeout_at(
        deadline.into(),
        proof_selected(config, socket, s, Some((binding, deadline))),
    )
    .await
    .map_err(|_| failure("original native scope deadline; no replay"))??;
    if Instant::now() >= deadline {
        return Err(failure("original native scope deadline; no replay"));
    }
    Ok(proof)
}
type CodexPresenceScope = (Uuid, ProcessEvidence, Option<ProcessEvidence>);
fn codex_static_presence_error_category(error: &str) -> Option<&'static str> {
    Some(match error {
        "Relay identity, transport or bounded admission failed; explicit reconciliation required" => {
            "relay_bounded_admission_failed"
        }
        "Stock TUI rejected exact thread/resume; zero-turn rollout may be unmaterialized. No warm-up or substitute was attempted" => {
            "stock_tui_resume_rejected"
        }
        "TUI transport read failed" => "tui_read",
        "TUI data frame invalid" => "tui_frame",
        "TUI observation failed" => "tui_observation",
        "Owner transport read failed" => "owner_read",
        "Owner data frame invalid" => "owner_frame",
        "Owner observation failed" => "owner_observation",
        "Owner/manifest source proof failed" => "manifest_proof",
        "Relay publication source changed" => "publication_changed",
        "Native observation publication failed" => "observation_publish",
        "Relay presence publication failed" => "presence_publish",
        "Owner-bound relay queue failed" => "owner_queue",
        "TUI-bound relay queue failed" => "tui_queue",
        "Owner write timed out" => "owner_write_timeout",
        "Owner write failed" => "owner_write",
        "TUI write timed out" => "tui_write_timeout",
        "TUI write failed" => "tui_write",
        "Herdr pane.process_info rejected: pane_not_found" => "pane_not_found",
        "Herdr terminal source proof failed" => "terminal_proof",
        "Bound terminal closed" => "terminal_closed",
        "Relay workers absent" => "relay_workers_absent",
        "Relay worker failed" => "relay_worker_failed",
        "Native relay forwarding closed" => "native_relay_closed",
        _ => return None,
    })
}
fn codex_terminal_capture_fields(error: Option<&str>) -> (&'static str, &'static str, i32, bool) {
    match error.and_then(goatr::codex_relay::terminal_capture_annotation) {
        Some(capture) => (
            capture.phase,
            capture.io,
            capture.errno,
            capture.response_timeout,
        ),
        None => ("none", "none", 0, false),
    }
}
fn codex_association_error(
    presence: &goatr::codex_relay::Presence,
    original_thread: &Id,
) -> io::Error {
    let resumed_original = presence.resumed_thread_ids.contains(original_thread);
    let presence_error = presence.error.as_ref().map(|error| error.0.as_str());
    let native_error = presence_error.and_then(|error| {
        [
            ("Correlated native initialize error (", "initialize"),
            ("Correlated native thread/read error (", "thread/read"),
            ("Correlated native thread/resume error (", "thread/resume"),
        ]
        .into_iter()
        .find_map(|(prefix, method)| {
            let code = error
                .strip_prefix(prefix)?
                .strip_suffix(')')?
                .parse::<i64>()
                .ok()?;
            Some((method, code))
        })
    });
    let (terminal_phase, terminal_io, terminal_errno, terminal_response_timeout) =
        codex_terminal_capture_fields(presence_error);
    let error_category = match (presence_error, native_error, terminal_phase) {
        (None, _, _) => "none",
        (_, Some(_), _) => "correlated_native_error",
        (_, None, phase) if phase != "none" => "terminal_capture",
        (Some(error), None, _) => codex_static_presence_error_category(error).unwrap_or("unknown"),
    };
    failure(&format!(
        "stock TUI live association proof unavailable; state={:?}; resumed_original={resumed_original}; presence_error={error_category}; terminal_phase={terminal_phase}; terminal_io={terminal_io}; terminal_errno={terminal_errno}; terminal_response_timeout={terminal_response_timeout}; retained_native_error={native_error:?}",
        presence.state,
    ))
}

fn original_presence_observation(
    pinned: &mut Option<CodexPresenceScope>,
    observed: goatr::codex_relay::ValidatedPresence,
    original_thread: &Id,
) -> io::Result<Option<goatr::codex_relay::Presence>> {
    let (presence, fresh) = match observed {
        goatr::codex_relay::ValidatedPresence::Fresh(presence) => (presence, true),
        goatr::codex_relay::ValidatedPresence::Past(presence)
        | goatr::codex_relay::ValidatedPresence::Unavailable(presence) => (presence, false),
    };
    if presence.state != goatr::codex_relay::PresenceState::Connected
        || !presence.associated_thread_ids.contains(original_thread)
        || !presence.resumed_thread_ids.contains(original_thread)
    {
        return Err(codex_association_error(&presence, original_thread));
    }
    let scope = (
        presence.connection_epoch.clone(),
        presence.relay.clone(),
        presence.tui.clone(),
    );
    if pinned.as_ref().is_some_and(|original| original != &scope) {
        return Err(failure(
            "original Codex presence connection/relay/TUI changed",
        ));
    }
    pinned.get_or_insert(scope);
    Ok(fresh.then_some(presence))
}
async fn original_presence_until(
    manifest: &goatr::codex_relay::Manifest,
    path: &Path,
    deadline: Instant,
) -> io::Result<goatr::codex_relay::Presence> {
    let mut pinned = None;
    loop {
        if Instant::now() >= deadline {
            return Err(failure("original native scope deadline; no replay"));
        }
        let observed = manifest
            .presence_observation(path)
            .map_err(|error| native_guard_context(NativeGuardPhase::CodexPresence, error));
        let fresh = match observed {
            Ok(observed) => {
                original_presence_observation(&mut pinned, observed, &manifest.expected_thread_id)?
            }
            Err(error) if native_presence_unavailable(&error) => None,
            Err(error) => return Err(error),
        };
        if Instant::now() >= deadline {
            return Err(failure("original native scope deadline; no replay"));
        }
        if let Some(presence) = fresh {
            return Ok(presence);
        }
        tokio::time::sleep_until(
            (Instant::now() + Duration::from_millis(100))
                .min(deadline)
                .into(),
        )
        .await;
    }
}
async fn proof_selected(
    config: &Config,
    socket: &Path,
    s: &SessionSummary,
    selected: Option<(&Uuid, Instant)>,
) -> io::Result<NativeProof> {
    let (e, snapshot) = surface(socket).await?;
    let pane = snapshot.pane(&s.pane_id)?;
    if pane.terminal_id != s.terminal_id || pane.workspace_id != s.workspace_id {
        return Err(failure(
            "actual native session routing differs from current Herdr surface",
        ));
    }
    let info = Client::connect(socket)
        .await?
        .process_info(&s.pane_id)
        .await?;
    let reporter = herdr::process_evidence(e.pid)?;
    match s.provider {
        Provider::Omp => {
            use futures_util::{SinkExt, StreamExt};
            use goatr::backends::omp::{FRAME_BYTES, QUEUE_BYTES};
            use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

            if s.connectivity != SessionConnectivity::Ready {
                return Err(failure(
                    "actual OMP binding is not Ready before independent proof",
                ));
            }
            let registry = config.omp_registry_dir.as_ref().ok_or_else(|| {
                failure("actual OMP registry not configured; enable collaboration explicitly")
            })?;
            let mut matches = Vec::new();
            for entry in goatr::backends::omp::registry(registry)? {
                if let Ok((record, owner)) = entry.snapshot().await
                    && record.session == s.native_id.0
                    && record.control
                    && record.relay_connected
                    && goatr::backends::omp::terminal_owner(&reporter, &owner, &info).is_ok()
                {
                    matches.push((entry, record, owner));
                }
            }
            if matches.len() != 1 {
                return Err(failure(
                    "exact OMP registry/native/kernel terminal owner join unavailable or ambiguous",
                ));
            }
            let (entry, record, owner) = matches.remove(0);
            let runtime = herdr::process_object(&owner, &config.runtime_dir_agent)?;
            let actual = fs::metadata(&config.runtime_dir)?;
            if runtime != (actual.dev(), actual.ino()) {
                return Err(failure(
                    "OMP provider integration-root object view mismatch",
                ));
            }
            let path = pane
                .agent_session
                .as_ref()
                .filter(|n| n.agent == "omp" && n.kind == "path")
                .ok_or_else(|| failure("actual Herdr OMP session path unavailable"))?;
            let history = goatr::backends::omp::HistoryFile::live(
                PathBuf::from(&path.value),
                config.omp_session_roots.clone(),
                owner.clone(),
                record.session.clone(),
            )?;
            history.recheck()?;
            let link = entry.link(&record, &owner).await?;
            let ws_config = WebSocketConfig::default()
                .max_message_size(Some(FRAME_BYTES))
                .max_frame_size(Some(FRAME_BYTES))
                .write_buffer_size(0)
                .max_write_buffer_size(QUEUE_BYTES);
            let (mut guest, _) = tokio::time::timeout(
                Duration::from_secs(15),
                tokio_tungstenite::connect_async_with_config(
                    format!("{}?role=guest", link.ws_url),
                    Some(ws_config),
                    true,
                ),
            )
            .await
            .map_err(|_| failure("actual OMP independent guest connection deadline"))?
            .map_err(|_| failure("actual OMP independent verified guest connection failed"))?;
            tokio::time::timeout(
                Duration::from_secs(5),
                guest.send(Message::Binary(link.hello()?.into())),
            )
            .await
            .map_err(|_| failure("actual OMP independent guest authentication deadline"))?
            .map_err(|_| failure("actual OMP independent guest authentication write failed"))?;
            let mut initial = OmpInitial::default();
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let message = guest
                        .next()
                        .await
                        .ok_or_else(|| failure("actual OMP independent guest disconnected"))?
                        .map_err(|_| failure("actual OMP independent guest read failed"))?;
                    let Message::Binary(bytes) = message else {
                        if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                            continue;
                        }
                        return Err(failure("actual OMP independent guest envelope unavailable"));
                    };
                    let frame_bytes = bytes.len();
                    let frame = match bytes.try_into_mut() {
                        Ok(mut envelope) => goatr::native_json::parse(
                            link.open(&mut envelope)?,
                            goatr::native_json::NATIVE,
                        )?,
                        Err(bytes) => goatr::native_json::parse(
                            link.open(&mut bytes.to_vec())?,
                            goatr::native_json::NATIVE,
                        )?,
                    };
                    if initial.frame(&frame, frame_bytes, &record.session, &record.cwd)? {
                        return Ok::<_, io::Error>(());
                    }
                }
            })
            .await
            .map_err(|_| failure("actual OMP independent authenticated FINAL deadline"))??;
            let (header_id, header_cwd, entry_count) = initial
                .header
                .ok_or_else(|| failure("actual OMP native welcome absent at FINAL"))?;
            // Continue draining immediately after the one FINAL, including while the
            // independent registry/topology guards below are being revalidated.
            let observer = Arc::new(OmpObserver::start(
                target(s),
                entry.clone(),
                record.clone(),
                history,
                guest,
                link,
                initial.observation,
            ));
            let (after, current_owner) = entry.snapshot().await?;
            if after.instance != record.instance
                || after.generation != record.generation
                || after.pid != record.pid
                || after.session != record.session
                || after.cwd != record.cwd
                || !after.control
                || !after.relay_connected
                || current_owner != owner
            {
                return Err(failure(
                    "actual OMP registry/owner changed across authenticated FINAL",
                ));
            }
            let (current_herdr, current_surface) = surface(socket).await?;
            let current_pane = current_surface.pane(&s.pane_id)?;
            if current_herdr != e
                || current_pane.terminal_id != pane.terminal_id
                || current_pane.workspace_id != pane.workspace_id
                || current_pane.agent_session != pane.agent_session
            {
                return Err(failure(
                    "actual OMP native topology changed across authenticated FINAL",
                ));
            }
            let current_processes = Client::connect(socket)
                .await?
                .process_info(&s.pane_id)
                .await?;
            goatr::backends::omp::terminal_owner(&reporter, &owner, &current_processes)?;
            observer.history.lock().refresh()?;
            observer.healthy(&target(s))?;
            let history_materialized = !observer.history.lock().header.is_null();
            let history_header_id = observer.history.lock().header.get("id").cloned();
            Ok(NativeProof {
                target: target(s),
                provider: s.provider,
                native: s.native_id.clone(),
                pane: s.pane_id.clone(),
                terminal: s.terminal_id.clone(),
                workspace: s.workspace_id.clone(),
                owner,
                tui: None,
                relay: None,
                manifest: None,
                collaboration: None,
                default_plan: None,
                omp_observer: Some(observer),
                objects: json!({"herdr":e,"instance":record.instance,"roomGeneration":record.generation,"runtime":runtime,"nativeHeader":{"id":header_id,"cwd":header_cwd},"writableProto3Welcome":true,"authenticatedFinal":true,"snapshotEntryCount":entry_count,"historyPath":path.value,"historyMaterialized":history_materialized,"historyHeaderId":history_header_id,"outerWrapper":config.executables.omp}),
            })
        }
        Provider::Codex => {
            let owner = config
                .creation_codex_owner()
                .map_err(|_| failure("actual selected shared Codex owner unavailable"))?;
            let mut found = Vec::new();
            let exact =
                selected.map(|(binding, _)| owner.bindings_dir.join(format!("{}.json", binding.0)));
            let directory = if exact.is_none() {
                Some(fs::read_dir(&owner.bindings_dir)?)
            } else {
                None
            };
            for path in exact.into_iter().map(Ok).chain(
                directory
                    .into_iter()
                    .flatten()
                    .map(|file| file.map(|file| file.path())),
            ) {
                let path = path?;
                if path.extension().and_then(|p| p.to_str()) != Some("json") {
                    continue;
                }
                let m =
                    match goatr::codex_relay::private_read::<goatr::codex_relay::Manifest>(&path) {
                        Ok(manifest) => manifest,
                        Err(error) if selected.is_some() => return Err(error),
                        Err(_) => continue,
                    };
                if let Some((binding, _)) = selected
                    && (m.binding_id != *binding
                        || m.expected_thread_id != s.native_id
                        || m.herdr.pane_id != s.pane_id
                        || m.herdr.terminal_id != s.terminal_id
                        || m.herdr.workspace_id != s.workspace_id
                        || m.herdr.server_id != s.server_id
                        || m.herdr.socket_path != socket
                        || m.herdr.evidence != e
                        || m.owner_id != owner.id
                        || m.upstream_host != owner.socket_host
                        || m.upstream_agent != owner.socket_agent)
                {
                    return Err(failure(
                        "successful Codex receipt manifest/source identity changed",
                    ));
                }
                if m.expected_thread_id == s.native_id
                    && m.herdr.terminal_id == s.terminal_id
                    && m.herdr.pane_id == s.pane_id
                {
                    let process = m
                        .owner_epoch
                        .validate(&m.upstream_host, &m.upstream_agent)
                        .map_err(|error| {
                            native_guard_context(NativeGuardPhase::CodexOwnerEpoch, error)
                        })?;
                    let presence = if let Some((_, deadline)) = selected {
                        original_presence_until(&m, &path, deadline).await?
                    } else {
                        m.presence(&path).map_err(|error| {
                            native_guard_context(NativeGuardPhase::CodexPresence, error)
                        })?
                    };
                    if presence.state != goatr::codex_relay::PresenceState::Connected
                        || !presence.resumed_thread_ids.contains(&s.native_id)
                    {
                        return Err(codex_association_error(&presence, &s.native_id));
                    }
                    let tui = presence
                        .tui
                        .clone()
                        .ok_or_else(|| failure("exact Codex TUI process missing"))?;
                    if !goatr::backends::omp::terminal_owner(&reporter, &tui, &info).is_ok() {
                        return Err(failure(
                            "actual Codex TUI/Herdr reporter namespace join missing",
                        ));
                    }
                    found.push(NativeProof{target:target(s),provider:s.provider,native:s.native_id.clone(),pane:s.pane_id.clone(),terminal:s.terminal_id.clone(),workspace:s.workspace_id.clone(),owner:process,tui:Some(tui),relay:Some(presence.relay.clone()),manifest:Some(path),collaboration:presence.collaboration_mode,default_plan:presence.collaboration_default_plan,omp_observer:None,objects:json!({"herdr":e,"ownerEpoch":m.owner_epoch,"ownerId":m.owner_id,"bindingId":m.binding_id,"connectionEpoch":presence.connection_epoch,"listenHost":m.listen_host,"upstreamHost":m.upstream_host,"outerWrapper":config.executables.codex})});
                }
            }
            if found.len() != 1 {
                return Err(failure(
                    "exact native Codex thread/owner/relay/TUI binding unavailable or ambiguous",
                ));
            }
            let p = found.remove(0);
            p.recheck_processes()?;
            Ok(p)
        }
    }
}
fn history_text(s: &SessionOpenResult) -> String {
    let mut out = String::new();
    for record in &s.history {
        for content in record
            .text
            .iter()
            .chain(record.tool.iter().flat_map(|t| t.output.iter()))
        {
            if let Content::Text(c) = content {
                out.push_str(&c.text.0);
                out.push('\n');
            }
        }
    }
    out
}
async fn wait_history(
    phone: &mut Phone,
    target: &Target,
    marker: &str,
    tool: bool,
) -> io::Result<SessionOpenResult> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let s = phone.open(target, "marker_history").await?;
        if history_text(&s).contains(marker)
            && (!tool || s.history.iter().any(|r| r.tool.is_some()))
        {
            return Ok(s);
        }
        if Instant::now() > deadline {
            return Err(prerequisite(
                "actual same-native history/tool prompt marker absent; model/credentials/tool prerequisites blocked",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
async fn human(phase: &str) -> io::Result<()> {
    if !io::stdin().is_terminal() {
        return Err(failure(
            "real desktop/dialog/camera phase requires a local human terminal; no mock acknowledgement",
        ));
    }
    println!(
        "{}",
        json!({"gate":"human-native-barrier","phase":phase,"instruction":"Perform this on the exact recorded OWNED actual desktop/session or native camera feed; enter yes when ready. The runner independently verifies native observations, not this acknowledgement."})
    );
    loop {
        let mut p = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut p, 1, 0) } > 0 {
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            if line.trim() == "yes" {
                return Ok(());
            }
            return Err(failure("human native phase declined"));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
fn resolved_without_effect(outcome: Result<ResultBody, WireError>) -> bool {
    match outcome {
        Err(error) => error.code == ErrorCode::AlreadyResolved,
        Ok(ResultBody::RequestRespond(result)) => {
            result.receipt.state == ReceiptState::Failed
                && result
                    .receipt
                    .error
                    .is_some_and(|e| e.code == ErrorCode::AlreadyResolved)
        }
        _ => false,
    }
}
async fn wait_dialog(
    phone: &mut Phone,
    p: &NativeProof,
    kind: PresentationKind,
    previous: Option<&Uuid>,
    mcp: Option<(&NativeMcp, &Uuid)>,
) -> io::Result<PendingRequest> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        p.recheck_processes()?;
        let current = phone.open(&p.target, "dialog_pending").await?;
        if let Some(request) = current
            .pending_requests
            .into_iter()
            .find(|request| request.kind == kind)
        {
            if previous == Some(&request.request_key) {
                return Err(failure(
                    "native editor reused its preparatory select request identity",
                ));
            }
            return Ok(request);
        }
        if let Some((service, operation)) = mcp {
            service
                .ready(if kind == PresentationKind::CodexForm {
                    McpKind::Form
                } else {
                    McpKind::Url
                })
                .await?;
            let path = service
                .directory
                .join(format!("result-{}.json", operation.0));
            if path.try_exists()? {
                let outcome: Value = goatr::codex_relay::private_read(&path)?;
                if outcome["stage"] == "failed" {
                    return Err(prerequisite(
                        "owned MCP producer failed before the actual card; finite failure stage retained",
                    ));
                }
                if outcome["stage"]
                    .as_str()
                    .is_some_and(|stage| stage != "requested")
                {
                    return Err(prerequisite(
                        "native MCP client resolved/declined before a source-bound card was observed; no UI effect is inferred",
                    ));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(prerequisite(
                "requested actual stock dialog was not observed; no synthetic projection or repeated trigger",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn desktop_dialog(
    phone: &mut Phone,
    p: &NativeProof,
    kind: PresentationKind,
    inputs: &NativeInputs<'_>,
) -> io::Result<Value> {
    let request = inputs.trigger_dialog(phone, p, kind, false, None).await?;
    if inputs.enabled {
        inputs
            .send(
                phone,
                p,
                NativeInput::Keys(&["esc"]),
                Some(&request),
                "desktop_dialog_resolution",
            )
            .await?;
    } else {
        human(&format!("Resolve that observed {kind:?} request ONLY on the same desktop TUI; the runner will verify its real close")).await?;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while phone
        .open(&p.target, "desktop_dialog_closed")
        .await?
        .pending_requests
        .iter()
        .any(|r| r.request_key == request.request_key)
    {
        if Instant::now() > deadline {
            return Err(failure(
                "actual desktop response did not close native request",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let late = phone
        .request(
            Uuid::random(),
            RequestBody::RequestRespond(RequestRespondParams {
                boot_id: phone.hello.boot_id.clone(),
                target: p.target.clone(),
                request_key: request.request_key.clone(),
                answer: answer(&request)?,
            }),
        )
        .await?;
    if !resolved_without_effect(late) {
        return Err(failure(
            "desktop-won late mobile response did not definitively reject without effect",
        ));
    }
    p.recheck_processes()?;
    inputs
        .finish_dialog(phone, p, &request.request_key, true)
        .await?;
    Ok(
        json!({"scenario":"actual-desktop-first-response-native-close","kind":kind,"target":p.target,"requestKey":request.request_key,"lateMobile":"already_resolved; no native effect","originals":"not persisted"}),
    )
}
fn history_command_deadline(
    proof_only: bool,
    provider: Provider,
    phase_deadline: Instant,
    command_started: Instant,
) -> Instant {
    // Coordinator-authorized exception for this independent Codex proof artifact.
    // Its input/proof and completion phases each retain their original 30s limit.
    let seconds = if proof_only && provider == Provider::Codex {
        60
    } else {
        30
    };
    phase_deadline.min(command_started + Duration::from_secs(seconds))
}
fn generation_idle_failure(
    commands: usize,
    ready: bool,
    idle: bool,
    pending_empty: bool,
) -> io::Error {
    // Keep the failed predicate observable without retaining state or request bodies.
    failure(&format!(
        "native generation lost its completed idle boundary; initial={}; ready={ready}; idle={idle}; pending_empty={pending_empty}; no replay",
        commands == 0,
    ))
}
async fn native_history_pages(
    phone: &mut Phone,
    p: &NativeProof,
    inputs: &NativeInputs<'_>,
) -> io::Result<Value> {
    if p.provider == Provider::Codex {
        return paging_proof::run(phone, p, inputs, NativeHistoryMode::RetentionBoundary1024).await;
    }
    let retained_limit = goatr::state::HISTORY_RECORDS;
    let mode = NativeHistoryMode::RetentionBoundary1024;
    let commands_required = mode.command_count();
    if p.omp_observer.is_none() {
        return Err(failure(
            "strict OMP boundary requires the original independent observer",
        ));
    }
    let mut initial = inputs.idle(phone, p).await?;
    if inputs.enabled {
        initial = phone.start_generation(&p.target).await?;
        if initial.state.summary.native_id != p.native {
            return Err(failure(
                "original history subscription native conversation changed",
            ));
        }
    }
    let initial_observed = initial.history.iter().filter(|r| r.complete).count();
    let mut observed = HashSet::new();
    let mut ordered = Vec::with_capacity(commands_required);
    let mut commands = 0usize;
    let started = Instant::now();
    // Keep a finite phase ceiling for the bounded native paging workload.
    let phase_deadline = started + Duration::from_secs(60 * 60);
    let mut latest = initial;
    if inputs.enabled {
        while observed.len() < commands_required {
            if commands >= commands_required || Instant::now() >= phase_deadline {
                return Err(prerequisite(
                    "bounded actual native history generation exceeded count/time budget; no replay",
                ));
            }
            // The last completion already supplied exact Ready/Idle and no-modal
            // state. send still obtains entirely fresh before/after input scopes.
            if latest.state.summary.connectivity != SessionConnectivity::Ready
                || latest.state.summary.status != SessionStatus::Idle
                || !latest.pending_requests.is_empty()
            {
                return Err(failure(
                    "native generation lost its completed idle boundary",
                ));
            }
            let marker = format!(
                "goatr-history-{}-{:?}-{commands}",
                &inputs.run.0[..8],
                p.provider
            );
            let command = format!("printf '%s\\n' '{marker}'");
            let command_started = Instant::now();
            let command_deadline = history_command_deadline(
                inputs.paging_proof,
                p.provider,
                phase_deadline,
                command_started,
            );
            // Observe the outer envelope; inner input/completion waits stay unchanged.
            phone.history_command = Some(HistoryCommandTrace::default());
            if let Some(observer) = &p.omp_observer {
                observer.healthy(&p.target)?;
                observer
                    .state
                    .lock()
                    .register(&command, &marker, command_deadline)?;
            }
            let completion = tokio::time::timeout_at(command_deadline.into(), async {
                inputs
                    .send(
                        phone,
                        p,
                        NativeInput::Prompt(&format!("!{command}")),
                        None,
                        "native_history_command",
                    )
                    .await?;
                inputs.shell_completed(phone, p, &command, &marker).await
            })
            .await;
            let (complete, id) = match completion {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => {
                    if let Some(observer) = &p.omp_observer {
                        observer
                            .state
                            .lock()
                            .fail(OmpObserverFailure::CommandFailed(
                                OmpObserverFailure::io_kind(error.kind()),
                            ));
                    }
                    return Err(error);
                }
                Err(_) => {
                    if let Some(observer) = &p.omp_observer {
                        observer
                            .state
                            .lock()
                            .fail(OmpObserverFailure::CommandDeadline);
                    }
                    return Err(phone
                        .history_command
                        .as_ref()
                        .expect("original command context")
                        .deadline());
                }
            };
            phone.history_command = None;
            if !observed.insert(id.clone()) {
                if let Some(observer) = &p.omp_observer {
                    observer
                        .state
                        .lock()
                        .fail(OmpObserverFailure::CompletionIdentity);
                }
                return Err(failure(
                    "native history command reused a completed native ID; no replay",
                ));
            }
            ordered.push(id);
            latest = complete;
            commands += 1;
            if commands == 16 || commands == 32 || commands.is_multiple_of(64) {
                write_json(
                    &inputs.evidence.join(format!(
                        "native-history-progress-{}-{:?}-{commands}.json",
                        inputs.run.0, p.provider
                    )),
                    &json!({"runId":inputs.run,"target":p.target,"provider":p.provider,
                        "commands":commands,"distinctCompletedIds":observed.len(),"elapsedMs":started.elapsed().as_millis(),
                        "observer":p.omp_observer.as_ref().map(|observer| observer.evidence()),"payloads":"not retained"}),
                ).map_err(|error| native_guard_context(NativeGuardPhase::HistoryProgressEvidence, error))?;
            }
            if commands == 16
                && started
                    .elapsed()
                    .as_millis()
                    .saturating_mul(commands_required as u128)
                    > 3600 * 1000 * 16
            {
                return Err(failure(
                    "measured OMP paging generation cannot fit its unchanged phase budget",
                ));
            }
        }
    } else if !latest.state.summary.history_coverage.older_available {
        human(&format!("Produce {commands_required} small real completed native records in SAME owned {:?} session {}; older native source, not just cached pages, is required",p.provider,p.native.0)).await?;
        latest = inputs.idle(phone, p).await?;
    }
    // This ordered barrier freezes the original consumer window before browse.
    phone.host().await?;
    let view = phone
        .generation
        .as_ref()
        .ok_or_else(|| failure("original OMP generation view missing"))?;
    if view.failed
        || target(&view.snapshot.state.summary) != p.target
        || view.snapshot.state.summary.connectivity != SessionConnectivity::Ready
        || view.snapshot.state.summary.status != SessionStatus::Idle
        || !view.snapshot.pending_requests.is_empty()
    {
        return Err(failure(
            "OMP generation checkpoint not original completed idle boundary",
        ));
    }
    let retained: HashSet<Id> = view
        .snapshot
        .history
        .iter()
        .filter(|r| r.complete)
        .map(|r| r.id.clone())
        .collect();
    let visible: HashSet<_> = view
        .snapshot
        .history
        .iter()
        .rev()
        .take(100)
        .map(|record| record.id.clone())
        .collect();
    if retained.len() > retained_limit {
        return Err(failure("retained upper bound exceeded"));
    }
    if observed.len() != commands_required || ordered.len() != commands_required {
        return Err(failure("OMP independent completion ledger count invalid"));
    }
    if let Some(observer) = &p.omp_observer {
        observer.healthy(&p.target)?;
        let state = observer.state.lock();
        if state.completions != commands_required as u64
            || !state
                .expected
                .as_ref()
                .is_some_and(|expected| expected.consumed)
        {
            return Err(failure(
                "OMP observer completion ledger was not fully consumed",
            ));
        }
    }
    write_json(
        &inputs.evidence.join(format!(
            "native-history-generation-{}-{:?}.json",
            inputs.run.0, p.provider
        )),
        &json!({"runId":inputs.run,"target":p.target,"provider":p.provider,
            "proofMode":mode.name(),"commands":commands,"distinctCompletedIds":observed.len(),
            "initialObservedRecords":initial_observed,"retainedRecordLimit":retained_limit,
            "retainedIdUpperBound":retained,"nativeCompletionLedger":ordered,
            "elapsedMs":started.elapsed().as_millis(),"generationComplete":true,
            "ready":true,"idle":true,"pendingEmpty":true,"olderPagingProven":false,
            "observer":p.omp_observer.as_ref().map(|observer| observer.evidence()),
            "payloads":"not retained"}),
    )
    .map_err(|error| native_guard_context(NativeGuardPhase::HistoryProgressEvidence, error))?;
    if inputs.enabled {
        inputs.current(phone, p).await?;
    }
    let mut cursor: Option<Id> = None;
    let mut view_id: Option<Uuid> = None;
    let mut ids = HashSet::new();
    let mut matched = HashSet::new();
    let mut outside = Vec::new();
    let mut pages = 0usize;
    let mut cursors = HashSet::new();
    let mut previous_oldest = usize::MAX;
    let mut qualified: Option<(Id, usize)> = None;
    let mut append_cursor_page: Option<(SessionHistoryParams, SessionHistoryResult)> = None;
    let paging_deadline = phase_deadline.min(Instant::now() + Duration::from_secs(15 * 60));
    loop {
        if pages == 64
            || Instant::now() >= paging_deadline
            || cursor
                .as_ref()
                .is_some_and(|next| !cursors.insert(next.clone()))
        {
            return Err(failure("native history cursor cycle or bound exceeded"));
        }
        let params = SessionHistoryParams {
            target: p.target.clone(),
            view_id: view_id.clone(),
            cursor: cursor.clone(),
            limit: Some(Uint(100)),
        };
        let ResultBody::SessionHistory(page) = phone
            .rpc(RequestBody::SessionHistory(params.clone()), "history_page")
            .await?
        else {
            return Err(failure("actual native history page absent"));
        };
        if view_id.as_ref().is_some_and(|view| view != &page.view_id) {
            return Err(failure("native browse view changed"));
        }
        view_id = Some(page.view_id.clone());
        let mut page_qualified = false;
        let mut indices = Vec::new();
        for record in &page.records {
            if !ids.insert(record.id.clone()) {
                return Err(failure("OMP native browse duplicated a record ID"));
            }
            if !record.complete {
                return Err(failure(
                    "native older page contains incomplete display data",
                ));
            }
            if observed.contains(&record.id) {
                if record.source != RecordSource::Persisted {
                    return Err(failure(
                        "OMP completed ledger record lost Persisted provenance",
                    ));
                }
                let index = ordered
                    .iter()
                    .position(|id| id == &record.id)
                    .ok_or_else(|| failure("OMP browse completion not in independent ledger"))?;
                if !matched.insert(record.id.clone()) {
                    return Err(failure("OMP native browse reused a completed ledger ID"));
                }
                indices.push(index);
                if !visible.contains(&record.id) && outside.len() < 16 {
                    outside.push(record.id.clone());
                }
                let later = commands_required
                    .checked_sub(index + 1)
                    .ok_or_else(|| failure("OMP completion order overflow"))?;
                if !retained.contains(&record.id) && later >= 1000 && qualified.is_none() {
                    qualified = Some((record.id.clone(), later));
                    page_qualified = true;
                    if !outside.contains(&record.id) {
                        if outside.len() == 16 {
                            outside.pop();
                        }
                        outside.push(record.id.clone());
                    }
                }
            }
        }
        if indices.windows(2).any(|pair| pair[0] >= pair[1])
            || indices
                .last()
                .is_some_and(|newest| *newest >= previous_oldest)
        {
            return Err(failure(
                "OMP native browse disagrees with independent completion order",
            ));
        }
        if let Some(oldest) = indices.first() {
            previous_oldest = *oldest;
        }
        if append_cursor_page.is_none()
            && params.cursor.is_some()
            && params.view_id.as_ref() == Some(&page.view_id)
            && page_qualified
            && !page.records.is_empty()
        {
            append_cursor_page = Some((params.clone(), page.clone()));
        }
        if cursor.is_some() {
            let ResultBody::SessionHistory(repeated) = phone
                .rpc(
                    RequestBody::SessionHistory(params.clone()),
                    "history_page_repeat",
                )
                .await?
            else {
                return Err(failure("same native history cursor repeat absent"));
            };
            if page.view_id != repeated.view_id
                || page.next_cursor != repeated.next_cursor
                || page.records != repeated.records
            {
                return Err(failure(
                    "same original native history cursor was not stable",
                ));
            }
        }
        pages += 1;
        cursor = page.next_cursor;
        if (matched.len() >= commands_required && outside.len() >= 16 && qualified.is_some())
            || cursor.is_none()
        {
            break;
        }
    }
    if matched.len() < commands_required
        || outside.len() < 16
        || retained.len() > retained_limit
        || qualified.is_none()
    {
        return Err(prerequisite(
            "strict native paging boundary not proved; cached cursor pages alone are insufficient",
        ));
    }
    let (append_params, append_page) = append_cursor_page.ok_or_else(|| {
        failure("strict native paging did not capture a qualifying explicit cursor page")
    })?;
    if inputs.enabled {
        inputs.current(phone, p).await?;
    }
    let append_view = phone
        .generation
        .as_ref()
        .ok_or_else(|| failure("OMP live append original generation view missing"))?;
    if append_view.failed
        || target(&append_view.snapshot.state.summary) != p.target
        || append_view.snapshot.state.summary.connectivity != SessionConnectivity::Ready
        || append_view.snapshot.state.summary.status != SessionStatus::Idle
        || !append_view.snapshot.pending_requests.is_empty()
        || Instant::now() >= paging_deadline
    {
        return Err(failure(
            "OMP live append requires original completed idle boundary",
        ));
    }
    let append_marker = format!(
        "goatr-history-{}-{:?}-{commands}",
        &inputs.run.0[..8],
        p.provider
    );
    let append_command = format!("printf '%s\\n' '{append_marker}'");
    let append_started = Instant::now();
    let append_deadline =
        history_command_deadline(false, p.provider, phase_deadline, append_started)
            .min(paging_deadline);
    phone.history_command = Some(HistoryCommandTrace::default());
    if let Some(observer) = &p.omp_observer {
        observer.healthy(&p.target)?;
        observer
            .state
            .lock()
            .register(&append_command, &append_marker, append_deadline)?;
    }
    let append = tokio::time::timeout_at(append_deadline.into(), async {
        inputs
            .send(
                phone,
                p,
                NativeInput::Prompt(&format!("!{append_command}")),
                None,
                "native_history_live_append",
            )
            .await?;
        inputs
            .shell_completed(phone, p, &append_command, &append_marker)
            .await
    })
    .await;
    let (append_snapshot, append_id) = match append {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => return Err(error),
        Err(_) => {
            return Err(phone
                .history_command
                .as_ref()
                .expect("original command context")
                .deadline());
        }
    };
    phone.history_command = None;
    if observed.contains(&append_id)
        || target(&append_snapshot.state.summary) != p.target
        || append_snapshot.state.summary.connectivity != SessionConnectivity::Ready
        || append_snapshot.state.summary.status != SessionStatus::Idle
        || !append_snapshot.pending_requests.is_empty()
    {
        return Err(failure(
            "OMP live append changed original identity or completion boundary",
        ));
    }
    if append_params.target != p.target
        || append_params.cursor.is_none()
        || append_params.view_id.as_ref() != Some(&append_page.view_id)
    {
        return Err(failure(
            "OMP captured browse cursor lost original view identity",
        ));
    }
    let after_append = tokio::time::timeout_at(
        paging_deadline.into(),
        phone.rpc(
            RequestBody::SessionHistory(append_params),
            "history_page_live_append_repeat",
        ),
    )
    .await
    .map_err(|_| failure("OMP live append repeat exceeded original view deadline"))??;
    let ResultBody::SessionHistory(after_append) = after_append else {
        return Err(failure("OMP live append repeat response absent"));
    };
    if after_append.view_id != append_page.view_id
        || after_append.next_cursor != append_page.next_cursor
        || after_append.records != append_page.records
    {
        return Err(failure(
            "OMP live append changed captured cursor membership/order/display/successor",
        ));
    }
    p.recheck_processes()?;
    // End the proof subscription before ordinary foreground/dialog checks resume.
    if phone.generation.is_some() {
        phone.finish_generation(&p.target).await?;
    }
    let (qualified_id, qualified_later_completed_lower_bound) =
        qualified.ok_or_else(|| failure("strict OMP boundary witness missing"))?;
    Ok(
        json!({"scenario":"actual-native-paging-live-append","proofMode":mode.name(),"localWindowLimit":retained_limit,"maxCursorAdvances":64,"viewExpirySeconds":900,"exhaustiveRetentionCrossing":"proved","strictRetentionBoundary":true,"target":p.target,"provider":p.provider,
        "totalPages":pages,"stableCursorRepeats":pages.saturating_sub(1),"liveAppendCursorStable":true,"viewId":view_id,"distinctCompletedRecords":matched.len(),
        "retainedRecordLimit":retained_limit,"retainedIdUpperBound":retained,
        "nativeOlderReturnedIds":outside,"qualifiedNativeId":qualified_id,"qualifiedLaterCompletedLowerBound":qualified_later_completed_lower_bound,"nativeOlderReadLowerBound":1,
        "nativeOlderSource":if p.provider==Provider::Omp{"protected_omp_file"}else{"codex_thread_turns_list_50"},
        "nativeSourceEpoch":p.objects,"initialObservedRecords":initial_observed,"generationCommands":commands,"liveAppendCommands":1,"totalNativeCommands":commands+1,
        "observer":p.omp_observer.as_ref().map(|observer| observer.evidence()),"generationElapsedMs":started.elapsed().as_millis(),
        "generationReadPolicy":"session.open and authenticated native observations only; no older page fetch",
        "scope":latest.state.summary.history_coverage,"noContentOriginalsInEvidence":true}),
    )
}
fn answer(request: &PendingRequest) -> io::Result<Answer> {
    let a = match &request.presentation {
        Presentation::OmpSelect(p) => Answer::OmpSelect(AnswerOmpSelect {
            action: SubmitAction::Submit,
            value: Some(
                p.options
                    .first()
                    .ok_or_else(|| failure("actual selection offers no option"))?
                    .value
                    .clone(),
            ),
        }),
        Presentation::OmpEditor(_) => Answer::OmpEditor(AnswerOmpEditor {
            action: SubmitAction::Submit,
            text: Some(PromptText("owned nonsecret checkpoint answer".into())),
        }),
        Presentation::CodexApproval(p) => Answer::CodexApproval(AnswerCodexApproval {
            choice_id: p
                .choices
                .iter()
                .find(|c| {
                    c.scope == ApprovalScope::Once
                        && (c.label.0.starts_with("Cancel;")
                            || c.label.0.starts_with("Decline;")
                            || c.label.0 == "Deny permissions")
                })
                .ok_or_else(|| failure("actual approval offers no decision"))?
                .choice_id
                .clone(),
        }),
        Presentation::CodexQuestions(p) => Answer::CodexQuestions(AnswerCodexQuestions {
            answers: p
                .questions
                .iter()
                .map(|q| QuestionAnswer {
                    id: q.id.clone(),
                    values: vec![
                        q.options
                            .first()
                            .map(|o| o.label.0.clone())
                            .unwrap_or_else(|| "owned nonsecret checkpoint answer".into()),
                    ],
                })
                .collect(),
        }),
        Presentation::CodexForm(_) => Answer::CodexForm(AnswerCodexForm {
            action: FormAction::Decline,
            values: None,
        }),
        Presentation::CodexUrl(_) => Answer::CodexUrl(AnswerCodexUrl {
            action: UrlAction::Decline,
        }),
        Presentation::DesktopOnly(_) => {
            return Err(failure(
                "this native dialog is desktop-only; no invented answer",
            ));
        }
    };
    validate_answer(&request.presentation, &a).map_err(io::Error::other)?;
    Ok(a)
}
async fn dialog(
    phone: &mut Phone,
    p: &NativeProof,
    kind: PresentationKind,
    inputs: &NativeInputs<'_>,
) -> io::Result<Value> {
    let request = inputs.trigger_dialog(phone, p, kind, false, None).await?;
    let key = request.request_key.clone();
    let id = Uuid::random();
    let result = phone
        .request(
            id.clone(),
            RequestBody::RequestRespond(RequestRespondParams {
                boot_id: phone.hello.boot_id.clone(),
                target: p.target.clone(),
                request_key: key.clone(),
                answer: answer(&request)?,
            }),
        )
        .await?
        .map_err(|_| failure("actual offered dialog answer rejected"))?;
    let receipt = receipt_result(result)?;
    let receipt = if matches!(
        receipt.state,
        ReceiptState::Reserved | ReceiptState::Dispatching
    ) {
        phone.settled(&id).await?
    } else {
        receipt
    };
    if receipt.action_id != id
        || receipt.target.as_ref() != Some(&p.target)
        || receipt.method != ActionMethod::RequestRespond
        || receipt.error.is_some()
        || receipt.state
            != if p.provider == Provider::Omp {
                ReceiptState::Dispatched
            } else {
                ReceiptState::Succeeded
            }
    {
        return Err(failure(
            "actual dialog answer receipt did not reach its native accepted outcome",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while phone
        .open(&p.target, "phone_dialog_closed")
        .await?
        .pending_requests
        .iter()
        .any(|r| r.request_key == key)
    {
        if Instant::now() > deadline {
            return Err(failure(
                "actual native request did not close after offered response",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let stale = phone
        .request(
            Uuid::random(),
            RequestBody::RequestRespond(RequestRespondParams {
                boot_id: phone.hello.boot_id.clone(),
                target: p.target.clone(),
                request_key: key.clone(),
                answer: answer(&request)?,
            }),
        )
        .await?;
    if !resolved_without_effect(stale) {
        return Err(failure(
            "late duplicate native dialog answer did not reject without effect",
        ));
    }
    inputs.finish_dialog(phone, p, &key, false).await?;
    Ok(
        json!({"kind":kind,"requestKey":key,"action":id,"receipt":receipt,"lateDuplicate":"rejected","originals":"memory only; no pending request/answer payload in evidence"}),
    )
}
fn arguments() -> clap::ArgMatches {
    feedback_arguments(arguments_base()).get_matches()
}
fn feedback_arguments(command: Command) -> Command {
    command.arg(Arg::new("defer-retention-crossing")
        .long("defer-retention-crossing")
        .action(ArgAction::SetTrue)
        .requires_all(["ui", "backend", "consent-owned-providers", "automate-native-inputs"])
        .conflicts_with_all(["camera", "ime", "setup", "prepare-emulator", "development-continuity"])
        .help("Explicitly defer exhaustive retention crossing and its older-row UI assertions; both providers and all other foreground gates remain required"))
        .arg(Arg::new("keep-foreground-for-feedback")
        .long("keep-foreground-for-feedback")
        .action(ArgAction::SetTrue)
        .requires_all(["ui", "backend", "consent-owned-providers", "automate-native-inputs"])
        .conflicts_with_all(["camera", "ime", "setup", "prepare-emulator", "development-continuity"])
        .help("After the selected both-provider Android checkpoint completes, freshly pair the owned app and supervise the same scoped companion until interrupted"))
}
fn exhaustive_paging(args: &clap::ArgMatches) -> &'static str {
    if args.get_flag("defer-retention-crossing") {
        "deferred_by_user"
    } else {
        "required"
    }
}
fn checkpoint_status(args: &clap::ArgMatches) -> &'static str {
    if args.get_flag("defer-retention-crossing") {
        "foreground_checkpoint_paging_deferred"
    } else {
        "pass"
    }
}
fn feedback_gate(args: &clap::ArgMatches) -> &'static str {
    if args.get_flag("defer-retention-crossing") {
        "post-checkpoint-feedback"
    } else {
        "post-pass-feedback"
    }
}
fn require_both_checkpoint_providers(
    args: &clap::ArgMatches,
    selected: &[Provider],
) -> io::Result<()> {
    if (args.get_flag("ui")
        || args.get_flag("keep-foreground-for-feedback")
        || args.get_flag("defer-retention-crossing"))
        && (selected.len() != 2
            || !selected.contains(&Provider::Omp)
            || !selected.contains(&Provider::Codex))
    {
        return Err(failure(
            "foreground UI, feedback, or retention deferral requires both original selected providers",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnedOmpCaptureMode {
    None,
    Creation,
    Offline,
    Backend,
    Conflict,
}
impl OwnedOmpCaptureMode {
    const fn requested(self) -> bool {
        !matches!(self, Self::None | Self::Conflict)
    }
    const fn requires_private_receiver(self) -> bool {
        matches!(self, Self::Creation | Self::Offline)
    }
    const fn requires_owned_relay(self) -> bool {
        self.requested()
    }
}
const fn owned_omp_capture_mode(
    creation: bool,
    offline: bool,
    backend: bool,
) -> OwnedOmpCaptureMode {
    match (creation, offline, backend) {
        (false, false, false) => OwnedOmpCaptureMode::None,
        (true, false, false) => OwnedOmpCaptureMode::Creation,
        (false, true, false) => OwnedOmpCaptureMode::Offline,
        (false, false, true) => OwnedOmpCaptureMode::Backend,
        _ => OwnedOmpCaptureMode::Conflict,
    }
}
fn selected_omp_capture_mode() -> OwnedOmpCaptureMode {
    owned_omp_capture_mode(
        owned_omp_creation_capture_requested(),
        owned_omp_offline_capture_requested(),
        owned_omp_backend_capture_requested(),
    )
}
fn owned_omp_capture_requested() -> bool {
    selected_omp_capture_mode().requested()
}
fn owned_omp_receiver_capture_requested() -> bool {
    selected_omp_capture_mode().requires_private_receiver()
}
fn owned_omp_creation_capture_requested() -> bool {
    std::env::var_os("GOATR_OWNED_OMP_CREATION_CAPTURE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
}
fn owned_omp_offline_capture_requested() -> bool {
    std::env::var_os("GOATR_OWNED_OMP_OFFLINE_CAPTURE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
}
fn owned_omp_backend_capture_requested() -> bool {
    std::env::var_os("GOATR_OWNED_OMP_BACKEND_CAPTURE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
}
fn owned_hello_capture_requested() -> bool {
    std::env::var_os("GOATR_OWNED_HELLO_CAPTURE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
}
pub(super) fn owned_omp_capture_conflicts() -> bool {
    selected_omp_capture_mode() == OwnedOmpCaptureMode::Conflict
}
#[derive(Clone, Copy)]
enum OwnedHerdrReportMode {
    OmpReceiver,
    CodexStartupTerminal,
}
fn owned_herdr_report_binary(
    capture: Option<OwnedHerdrReportMode>,
    workspace: &Path,
) -> io::Result<Option<PathBuf>> {
    let binary = std::env::var_os("GOATR_OWNED_HERDR_REPORT_BIN");
    let expected = std::env::var("GOATR_OWNED_HERDR_REPORT_SHA256").ok();
    if capture.is_none() {
        if binary.is_some() || expected.is_some() {
            return Err(failure(
                "private Herdr report binary is only valid with an owned diagnostic capture",
            ));
        }
        return Ok(None);
    }
    let (Some(binary), Some(expected)) = (binary, expected) else {
        return Err(failure(
            "owned diagnostic capture requires private Herdr binary and SHA-256",
        ));
    };
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(failure(
            "private Herdr report SHA-256 must be 64 lowercase hex characters",
        ));
    }
    let binary = PathBuf::from(binary);
    if !binary.is_absolute() || !binary.starts_with(workspace) {
        return Err(failure(
            "private Herdr report binary must remain under the owned workspace",
        ));
    }
    let metadata = fs::symlink_metadata(&binary)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != uid()
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o500
    {
        return Err(failure(
            "private Herdr report binary identity or mode rejected",
        ));
    }
    let canonical = fs::canonicalize(&binary)?;
    if canonical != binary || !canonical.starts_with(workspace) {
        return Err(failure("private Herdr report binary alias rejected"));
    }
    goatr::config::executable(&canonical)?;
    if binary_sha256(&canonical)? != expected {
        return Err(failure("private Herdr report binary SHA-256 mismatch"));
    }
    Ok(Some(canonical))
}

fn arguments_base() -> Command {
    Command::new("smoke")
        .group(
            ArgGroup::new("frozen-development-package")
                .args([
                    "development-continuity",
                    "codex-invalidation-diagnostic",
                    "physical-hello-capture",
                    "omp-creation-diagnostic",
                ]),
        )
        .arg(
            Arg::new("paging-proof")
                .long("paging-proof")
                .action(ArgAction::SetTrue)
                .requires_all(["backend", "consent-owned-providers", "automate-native-inputs"])
                .conflicts_with_all([
                    "ui",
                    "camera",
                    "ime",
                    "setup",
                    "prepare-emulator",
                    "keep-foreground-for-feedback",
                    "development-continuity",
                ])
                .help("Independent Codex retention proof only; not foreground/full acceptance"),
        )
        .arg(
            Arg::new("codex-invalidation-diagnostic")
                .long("codex-invalidation-diagnostic")
                .action(ArgAction::SetTrue)
                .requires_all([
                    "backend",
                    "consent-owned-providers",
                    "automate-native-inputs",
                    "goatr",
                    "expected-goatr-sha256",
                    "expected-companion-lock-sha256",
                ])
                .conflicts_with_all([
                    "ui",
                    "camera",
                    "ime",
                    "setup",
                    "prepare-emulator",
                    "paging-proof",
                    "development-continuity",
                    "defer-retention-crossing",
                    "keep-foreground-for-feedback",
                ])
                .help("Fresh Codex pre-input invalidation diagnostic only; not Android or paging acceptance"),
        )
        .arg(
            Arg::new("codex-startup-terminal-capture")
                .long("codex-startup-terminal-capture")
                .action(ArgAction::SetTrue)
                .requires_all(["codex-invalidation-diagnostic", "consent-owned-providers"])
                .help("One bounded post-settlement owned Codex terminal read; diagnostic only"),
        )
        .arg(
            Arg::new("omp-creation-diagnostic")
                .long("omp-creation-diagnostic")
                .action(ArgAction::SetTrue)
                .requires_all([
                    "backend",
                    "consent-owned-providers",
                    "config",
                    "workspace-root",
                    "goatr",
                    "expected-goatr-sha256",
                    "expected-companion-lock-sha256",
                ])
                .conflicts_with_all([
                    "ui",
                    "camera",
                    "ime",
                    "setup",
                    "prepare-emulator",
                    "paging-proof",
                    "codex-invalidation-diagnostic",
                    "physical-hello-capture",
                    "development-continuity",
                    "development-owner-feedback",
                    "development-android-only",
                    "defer-retention-crossing",
                    "keep-foreground-for-feedback",
                    "automate-native-inputs",
                    "apk",
                    "test-apk",
                    "expected-apk-sha256",
                    "expected-test-apk-sha256",
                ])
                .help("One owned OMP creation receipt diagnostic; no native input, lookup, proof, history, or Android workflow"),
        )
        .arg(
            Arg::new("physical-hello-capture")
                .long("physical-hello-capture")
                .action(ArgAction::SetTrue)
                .requires_all([
                    "config",
                    "goatr",
                    "admin-socket",
                    "expected-goatr-sha256",
                    "expected-companion-lock-sha256",
                ])
                .conflicts_with_all([
                    "ui",
                    "camera",
                    "ime",
                    "setup",
                    "prepare-emulator",
                    "paging-proof",
                    "codex-invalidation-diagnostic",
                    "development-continuity",
                    "defer-retention-crossing",
                    "keep-foreground-for-feedback",
                    "backend",
                ])
                .help("Owned physical-phone Hello boundary diagnostic only; no pairing, provider creation, or native input"),
        )
        .arg(
            Arg::new("development-continuity")
                .long("development-continuity")
                .action(ArgAction::SetTrue)
                .requires_all([
                    "backend",
                    "consent-owned-providers",
                    "automate-native-inputs",
                    "config",
                    "workspace-root",
                    "goatr",
                    "expected-goatr-sha256",
                    "expected-companion-lock-sha256",
                    "emulator-owner",
                    "apk",
                    "test-apk",
                    "expected-apk-sha256",
                    "expected-test-apk-sha256",
                ])
                .conflicts_with_all([
                    "ui",
                    "camera",
                    "ime",
                    "setup",
                    "prepare-emulator",
                    "paging-proof",
                    "defer-retention-crossing",
                    "keep-foreground-for-feedback",
                ])
                .help("Fresh owned OMP/Codex mobile-to-desktop continuity check; not Android/full acceptance"),
        )
        .arg(
            Arg::new("development-owner-feedback")
                .long("development-owner-feedback")
                .action(ArgAction::SetTrue)
                .requires("development-continuity")
                .conflicts_with("development-android-only")
                .help("Keep fresh native setup available for owner pairing; no automated Android acceptance claim"),
        )
        .arg(
            Arg::new("development-android-only")
                .long("development-android-only")
                .action(ArgAction::SetTrue)
                .requires("development-continuity")
                .help("Fresh native setup followed by Android feedback continuity; omit Rust history/continuity prelude"),
        )
        .arg(
            Arg::new("expected-goatr-sha256")
                .long("expected-goatr-sha256")
                .value_parser(value_parser!(String))
                .requires("frozen-development-package")
                .help("Required frozen SHA-256 for the exact development companion binary"),
        )
        .arg(
            Arg::new("expected-companion-lock-sha256")
                .long("expected-companion-lock-sha256")
                .value_parser(value_parser!(String))
                .requires("frozen-development-package")
                .help("Required frozen source lock SHA-256 for the development companion"),
        )
        .arg(
            Arg::new("expected-apk-sha256")
                .long("expected-apk-sha256")
                .value_parser(value_parser!(String))
                .requires("development-continuity")
                .help("Required frozen SHA-256 for the development debug APK"),
        )
        .arg(
            Arg::new("expected-test-apk-sha256")
                .long("expected-test-apk-sha256")
                .value_parser(value_parser!(String))
                .requires("development-continuity")
                .help("Required frozen SHA-256 for the development debug test APK"),
        )
        .about("Actual owned packaged OMP/Codex/provider/UI checkpoint; never fixture fallback")
        .arg(
            Arg::new("backend")
                .long("backend")
                .action(ArgAction::Append)
                .value_parser(["omp", "codex"]),
        )
        .arg(Arg::new("ui").long("ui").action(ArgAction::SetTrue))
        .args(["camera", "ime", "setup"].map(|name| {
            Arg::new(name)
                .long(name)
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["ui", "backend", "prepare-emulator", "development-continuity"])
        }))
        .arg(
            Arg::new("disposable-uid")
                .long("disposable-uid")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("prepare-emulator")
                .long("prepare-emulator")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("api")
                .long("api")
                .value_parser(value_parser!(u32))
                .default_value("34"),
        )
        .arg(Arg::new("serial").long("serial"))
        .arg(
            Arg::new("emulator-owner")
                .long("emulator-owner")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("admin-socket")
                .long("admin-socket")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("workspace-root")
                .long("workspace-root")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("goatr")
                .long("goatr")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("evidence-dir")
                .long("evidence-dir")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("consent-owned-providers")
                .long("consent-owned-providers")
                .action(ArgAction::SetTrue)
                .help("Explicitly create retained named Herdr/project/native provider resources and tiny test turns; never production transitions"),
        )
        .arg(
            Arg::new("automate-native-inputs")
                .long("automate-native-inputs")
                .requires("consent-owned-providers")
                .conflicts_with_all(["camera", "ime", "setup", "prepare-emulator"])
                .action(ArgAction::SetTrue)
                .help("Consent to once-only guarded inputs in this fresh run's owned native TUIs; uncertain inputs are never replayed"),
        )
        .arg(Arg::new("apk").long("apk").value_parser(value_parser!(PathBuf)))
        .arg(
            Arg::new("test-apk")
                .long("test-apk")
                .value_parser(value_parser!(PathBuf)),
        )
}
fn packaged(args: &clap::ArgMatches) -> io::Result<PathBuf> {
    if args.get_flag("codex-invalidation-diagnostic")
        || args.get_flag("omp-creation-diagnostic")
    {
        return dev_continuity::frozen_package(args);
    }
    let p = args
        .get_one::<PathBuf>("goatr")
        .cloned()
        .or_else(|| std::env::var_os("GOATR_BIN").map(PathBuf::from))
        .ok_or_else(|| failure("actual packaged --goatr /nix/store/.../bin/goatr required"))?;
    let p = fs::canonicalize(p)?;
    goatr::config::executable(&p)?;
    if !p.starts_with("/nix/store") || p.file_name().is_none_or(|n| n != "goatr") {
        return Err(failure(
            "stock gate requires actual single-package Nix goatr artifact",
        ));
    }
    let manifest: Value = serde_json::from_slice(&fs::read(
        p.parent()
            .and_then(Path::parent)
            .ok_or_else(|| failure("package root missing"))?
            .join("share/goatr/manifest.json"),
    )?)
    .map_err(io::Error::other)?;
    if manifest["kind"].as_str() != Some("direct-rust")
        || manifest["lockSha256"].as_str() != Some(transport::lock_sha256())
    {
        return Err(failure(
            "packaged goatr source/lock identity differs from this source runner",
        ));
    }
    Ok(p)
}
const CODEX_THREAD_START_METHOD_FAMILIES: [&str; 8] = [
    "config_warning",
    "mcp_startup",
    "thread_started",
    "auth_refresh",
    "attestation",
    "mcp_elicitation",
    "other_notification",
    "other_server_request",
];

fn parse_codex_thread_start_method_mask(mask: &str) -> Option<(u8, Vec<&'static str>)> {
    let bytes = mask.as_bytes();
    if bytes.is_empty() || (bytes.len() > 1 && bytes[0] == b'0') {
        return None;
    }
    let mut value = 0u16;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u16::from(*byte - b'0'))?;
        if value > u16::from(u8::MAX) {
            return None;
        }
    }
    let value = u8::try_from(value).ok()?;
    let families = CODEX_THREAD_START_METHOD_FAMILIES
        .iter()
        .enumerate()
        .filter_map(|(index, family)| (value & (1 << index) != 0).then_some(*family))
        .collect();
    Some((value, families))
}

fn parse_codex_guard_line(line: &[u8]) -> Option<Value> {
    let line = std::str::from_utf8(line).ok()?;
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "goatr_codex_first_guard" {
        return None;
    }
    let phase = fields.next()?.strip_prefix("phase=")?;
    if !matches!(
        phase,
        "creation"
            | "thread"
            | "connection"
            | "follower"
            | "reader"
            | "installation"
            | "herdr"
            | "shell_probe"
            | "transport"
            | "supervisor"
            | "owner_progress"
            | "relay_startup"
    ) {
        return None;
    }
    let guard = fields.next()?.strip_prefix("guard=")?;
    if phase == "owner_progress" {
        if !matches!(guard, "owner_start_settled" | "owner_child_status_returned" | "owner_native_initialize_entered")
            || fields.next()? != "code=none"
            || fields.next()? != "io=none"
            || fields.next()? != "errno=0"
            || fields.next().is_some()
        {
            return None;
        }
        return Some(json!({"gate":"owned-codex-invalidation-capture","phase":phase,"guard":guard,"code":"none","io":"none","errno":0,"report":null}));
    }
    let relay_startup_guard = matches!(
        guard,
        "self_process"
            | "existing_presence"
            | "owner_epoch"
            | "owner_socket"
            | "owner_revalidation"
            | "owner_connect"
            | "owner_credentials"
            | "owner_handshake"
            | "upstream_close"
            | "manifest_revalidation"
            | "terminal"
            | "listener"
            | "initial_publication"
    );
    if (phase == "relay_startup") != relay_startup_guard {
        return None;
    }
    let shell_probe_guard = matches!(
        guard,
        "query_effect_dropped"
            | "query_rejected_known"
            | "query_rejected_other"
            | "query_source_changed"
            | "query_unavailable"
            | "result_deadline"
            | "result_authorization"
            | "result_shell_changed"
            | "result_read_failed"
            | "result_invalid"
            | "final_source"
            | "final_root"
            | "final_root_server_id"
            | "final_root_incarnation"
            | "final_root_evidence"
            | "final_root_socket_path"
            | "final_root_invalidation"
            | "workspace_confirm_connect"
            | "workspace_confirm_evidence_epoch"
            | "workspace_confirm_snapshot"
            | "workspace_confirm_pane"
            | "workspace_confirm_created_workspace"
            | "workspace_confirm_terminal"
            | "workspace_confirm_focus"
            | "final_source_changed"
            | "final_connect"
            | "final_identity"
            | "final_process_info"
            | "final_shell_changed"
            | "final_shell_reporter"
            | "final_shell_absent"
            | "final_shell_not_idle"
            | "final_shell_process"
            | "final_shell_executable"
            | "final_shell_reporter_recheck"
            | "final_result_view"
            | "final_deadline"
            | "final_authorization"
            | "result_rejected"
            | "result_rejected_kind"
            | "result_rejected_path"
            | "result_unsupported"
    );
    if (phase == "shell_probe") != shell_probe_guard {
        return None;
    }
    let launch_pending_guard = matches!(
        guard,
        "launch_deadline_source_unavailable" | "launch_deadline_publication_pending"
            | "launch_deadline_presence_past" | "launch_deadline_presence_unavailable"
            | "launch_deadline_relay_not_connected" | "launch_deadline_resume_association_missing"
            | "launch_deadline_terminal_proof_false" | "launch_deadline_loaded_membership_false"
            | "launch_deadline_connection_absent" | "launch_deadline_connection_readiness_pending"
            | "launch_deadline_created_binding_pending" | "launch_deadline_source_recheck_pending"
            | "launch_deadline_unobserved" | "launch_deadline_drain"
            | "launch_stopping_source_unavailable" | "launch_stopping_publication_pending"
            | "launch_stopping_presence_past" | "launch_stopping_presence_unavailable"
            | "launch_stopping_relay_not_connected" | "launch_stopping_resume_association_missing"
            | "launch_stopping_terminal_proof_false" | "launch_stopping_loaded_membership_false"
            | "launch_stopping_connection_absent" | "launch_stopping_connection_readiness_pending"
            | "launch_stopping_created_binding_pending" | "launch_stopping_source_recheck_pending"
            | "launch_stopping_unobserved" | "launch_stopping_drain"
    );
    if launch_pending_guard && phase != "creation" {
        return None;
    }
    let pre_thread_guard = matches!(
        guard,
        "pre_thread_deadline_before_reproof"
            | "pre_thread_owner_reproof"
            | "pre_thread_deadline_after_reproof"
            | "pre_thread_stopping"
    );
    if pre_thread_guard && phase != "creation" {
        return None;
    }
    let transport_guard = matches!(
        guard,
        "reader_inflight"
            | "reader_frame"
            | "reader_rate_queue"
            | "reader_request_queue"
            | "reader_decode_queue"
            | "reader_decode_close"
            | "reader_non_request"
            | "extra_stream"
            | "extra_stream_error"
            | "response_guard"
            | "response_queue"
            | "output_guard"
            | "output_pop_error"
            | "delivery_metadata"
            | "output_write"
            | "delivery_storage"
            | "output_queue_closed"
            | "output_drained_after_reader"
            | "output_finish"
    );
    if (phase == "transport") != transport_guard {
        return None;
    }
    let supervisor_guard = matches!(
        guard,
        "selected_ctrl_c"
            | "selected_sigterm"
            | "selected_admin_ok"
            | "selected_admin_error"
            | "selected_transport_ok"
            | "selected_transport_error"
            | "selected_topology_ok"
            | "selected_topology_error"
            | "sibling_admin_error"
            | "sibling_transport_error"
            | "sibling_topology_error"
    );
    if (phase == "supervisor") != supervisor_guard {
        return None;
    }
    if !launch_pending_guard && !relay_startup_guard && !matches!(
        guard,
        "owner_or_native"
            | "owner_authorization"
            | "owner_exit"
            | "owner_native_process"
            | "owner_persist"
            | "owner_proof"
            | "owner_record_manager"
            | "owner_record_proof"
            | "owner_record_shape"
            | "owner_service_cwd"
            | "owner_service_environment"
            | "owner_service_exec_start"
            | "owner_service_identity"
            | "owner_service_kernel"
            | "owner_service_pid"
            | "owner_service_properties"
            | "owner_socket_epoch"
            | "owner_spawn"
            | "owner_wait"
            | "native_initialize"
            | "thread_start_send"
            | "thread_start_reply"
            | "thread_start_callback_lost"
            | "thread_start_id"
            | "thread_start_acknowledge"
            | "thread_effect_drop_start_send"
            | "thread_effect_drop_start_reply"
            | "thread_effect_drop_acknowledge"
            | "thread_effect_drop_owner_proof"
            | "thread_effect_drop_title_send"
            | "thread_effect_drop_title_reply"
            | "thread_effect_drop_read_send"
            | "thread_effect_drop_read_reply"
            | "thread_effect_drop_final_proof"
            | "thread_effect_drop_checkpoint_guard"
            | "physical_or_presence"
            | "state_before_presence"
            | "manifest_read"
            | "presence_observation"
            | "state_after_presence"
            | "manifest_encode"
            | "manifest_changed"
            | "owner_epoch"
            | "owner_changed"
            | "source_identity"
            | "original_presence_state"
            | "original_presence_state_relay_bounded_admission_failed"
            | "original_presence_state_stock_tui_resume_rejected"
            | "original_presence_state_tui_read"
            | "original_presence_state_tui_frame"
            | "original_presence_state_tui_observation"
            | "original_presence_state_owner_read"
            | "original_presence_state_owner_frame"
            | "original_presence_state_owner_observation"
            | "original_presence_state_manifest_proof"
            | "original_presence_state_publication_changed"
            | "original_presence_state_observation_publish"
            | "original_presence_state_presence_publish"
            | "original_presence_state_owner_queue"
            | "original_presence_state_tui_queue"
            | "original_presence_state_owner_write_timeout"
            | "original_presence_state_owner_write"
            | "original_presence_state_tui_write_timeout"
            | "original_presence_state_tui_write"
            | "original_presence_state_terminal_tui_write_unpolled_not_pending"
            | "original_presence_state_terminal_tui_write_ready_not_pending"
            | "original_presence_state_terminal_tui_write_ready_pending"
            | "original_presence_state_terminal_tui_write_start_send_not_pending"
            | "original_presence_state_terminal_tui_write_flush_not_pending"
            | "original_presence_state_terminal_tui_write_flush_pending"
            | "original_presence_state_pane_not_found"
            | "original_presence_state_terminal_proof"
            | "original_presence_state_terminal_connect"
            | "original_presence_state_terminal_snapshot_source"
            | "original_presence_state_terminal_snapshot"
            | "original_presence_state_terminal_snapshot_timeout"
            | "original_presence_state_terminal_reporter_process"
            | "original_presence_state_terminal_process_info_connect"
            | "original_presence_state_terminal_process_info_source"
            | "original_presence_state_terminal_process_info"
            | "original_presence_state_terminal_process_info_timeout"
            | "original_presence_state_terminal_tui_distribution"
            | "original_presence_state_terminal_tui_argv"
            | "original_presence_state_terminal_tui_foreground"
            | "original_presence_state_terminal_closed"
            | "original_presence_state_relay_workers_absent"
            | "original_presence_state_relay_worker_failed"
            | "original_presence_state_native_relay_closed"
            | "original_presence_binding"
            | "original_presence_owner_epoch"
            | "original_presence_relay"
            | "original_presence_connection_epoch"
            | "original_presence_tui"
            | "original_presence_associated"
            | "original_presence_resumed"
            | "observation_pending"
            | "presence_freshness"
            | "physical_state"
            | "topology_source"
            | "terminal_absent"
            | "startup_terminal_pin"
            | "startup_terminal_read"
            | "startup_terminal_empty"
            | "startup_terminal_truncated"
            | "startup_terminal_unclassified"
            | "startup_terminal_multiple"
            | "startup_terminal_account_transport"
            | "startup_terminal_account_decode"
            | "startup_terminal_account_server"
            | "startup_terminal_account_bootstrap_unknown"
            | "startup_terminal_models_bootstrap"
            | "startup_terminal_requirements_bootstrap"
            | "startup_terminal_thread_lookup"
            | "startup_terminal_event_stream_closed"
            | "startup_terminal_configuration_load"
            | "startup_terminal_configuration_toml"
            | "native_report_changed"
            | "effective_topology_loss"
            | "effective_topology_revalidation"
            | "effective_topology_unavailable"
            | "subscription_connect"
            | "snapshot_connect"
            | "snapshot_read"
            | "subscription_event"
            | "binding_superseded"
            | "readiness"
            | "native_read"
            | "connection_readiness"
            | "event_application"
            | "incarnation_changed"
            | "frame_closed"
            | "frame_protocol"
            | "frame_capacity"
            | "frame_other"
            | "frame_read"
            | "frame_close"
            | "frame_bytes"
            | "frame_parse"
            | "callback_frame"
            | "event_capacity"
            | "event_queue"
            | "reader_terminal"
            | "initial_installation"
            | "query_effect_dropped"
            | "query_rejected_known"
            | "query_rejected_other"
            | "query_source_changed"
            | "query_unavailable"
            | "result_deadline"
            | "result_authorization"
            | "result_shell_changed"
            | "result_read_failed"
            | "result_invalid"
            | "final_source"
            | "final_root"
            | "final_root_server_id"
            | "final_root_incarnation"
            | "final_root_evidence"
            | "final_root_socket_path"
            | "final_root_invalidation"
            | "workspace_confirm_connect"
            | "workspace_confirm_evidence_epoch"
            | "workspace_confirm_snapshot"
            | "workspace_confirm_pane"
            | "workspace_confirm_created_workspace"
            | "workspace_confirm_terminal"
            | "workspace_confirm_focus"
            | "final_source_changed"
            | "final_connect"
            | "final_identity"
            | "final_process_info"
            | "final_shell_changed"
            | "final_shell_reporter"
            | "final_shell_absent"
            | "final_shell_not_idle"
            | "final_shell_process"
            | "final_shell_executable"
            | "final_shell_reporter_recheck"
            | "final_result_view"
            | "final_deadline"
            | "final_authorization"
            | "result_rejected"
            | "result_rejected_kind"
            | "result_rejected_path"
            | "result_unsupported"
            | "pre_thread_deadline_before_reproof"
            | "pre_thread_owner_reproof"
            | "pre_thread_deadline_after_reproof"
            | "pre_thread_stopping"
            | "reader_inflight"
            | "reader_frame"
            | "reader_rate_queue"
            | "reader_request_queue"
            | "reader_decode_queue"
            | "reader_decode_close"
            | "reader_non_request"
            | "extra_stream"
            | "extra_stream_error"
            | "response_guard"
            | "response_queue"
            | "output_guard"
            | "output_pop_error"
            | "delivery_metadata"
            | "output_write"
            | "delivery_storage"
            | "output_queue_closed"
            | "output_drained_after_reader"
            | "output_finish"
            | "selected_ctrl_c"
            | "selected_sigterm"
            | "selected_admin_ok"
            | "selected_admin_error"
            | "selected_transport_ok"
            | "selected_transport_error"
            | "selected_topology_ok"
            | "selected_topology_error"
            | "sibling_admin_error"
            | "sibling_transport_error"
            | "sibling_topology_error"
    ) {
        return None;
    }
    let code = fields.next()?.strip_prefix("code=")?;
    if !matches!(
        code,
        "none"
            | "InvalidRequest"
            | "UnsupportedVersion"
            | "UnsupportedMethod"
            | "Unauthorized"
            | "PairingFailed"
            | "StaleHost"
            | "StaleSession"
            | "NotFound"
            | "NotReady"
            | "InvalidAnswer"
            | "AlreadyResolved"
            | "ActionIdConflict"
            | "ActionCapacity"
            | "ContentExpired"
            | "CursorExpired"
            | "Busy"
            | "ResourceLimit"
            | "RateLimited"
            | "Timeout"
            | "BackendError"
            | "PushRejected"
            | "StaleRegistration"
    ) {
        return None;
    }
    if code == "none" && !transport_guard && !supervisor_guard && !relay_startup_guard {
        return None;
    }
    let io = fields.next()?.strip_prefix("io=")?;
    if !matches!(
        io,
        "none"
            | "not_found"
            | "permission_denied"
            | "connection_refused"
            | "connection_reset"
            | "connection_aborted"
            | "not_connected"
            | "broken_pipe"
            | "timed_out"
            | "unexpected_eof"
            | "invalid_data"
            | "cancelled"
            | "other"
    ) {
        return None;
    }
    if io == "cancelled" && !transport_guard && !supervisor_guard {
        return None;
    }
    let errno = fields.next()?.strip_prefix("errno=")?.parse::<i32>().ok()?;
    if !(-4095..=4095).contains(&errno) {
        return None;
    }
    if relay_startup_guard
        && (code != "none"
            || !matches!(
                io,
                "not_found"
                    | "permission_denied"
                    | "connection_refused"
                    | "connection_reset"
                    | "connection_aborted"
                    | "not_connected"
                    | "broken_pipe"
                    | "timed_out"
                    | "unexpected_eof"
                    | "invalid_data"
                    | "other"
            ))
    {
        return None;
    }
    let valid_shell_probe = match guard {
        "launch_deadline_source_unavailable" | "launch_deadline_publication_pending"
        | "launch_deadline_presence_past" | "launch_deadline_presence_unavailable"
        | "launch_deadline_relay_not_connected" | "launch_deadline_resume_association_missing"
        | "launch_deadline_terminal_proof_false" | "launch_deadline_loaded_membership_false"
        | "launch_deadline_connection_absent" | "launch_deadline_connection_readiness_pending"
        | "launch_deadline_created_binding_pending" | "launch_deadline_source_recheck_pending"
        | "launch_deadline_unobserved" | "launch_deadline_drain"
        | "launch_stopping_source_unavailable" | "launch_stopping_publication_pending"
        | "launch_stopping_presence_past" | "launch_stopping_presence_unavailable"
        | "launch_stopping_relay_not_connected" | "launch_stopping_resume_association_missing"
        | "launch_stopping_terminal_proof_false" | "launch_stopping_loaded_membership_false"
        | "launch_stopping_connection_absent" | "launch_stopping_connection_readiness_pending"
        | "launch_stopping_created_binding_pending" | "launch_stopping_source_recheck_pending"
        | "launch_stopping_unobserved" | "launch_stopping_drain" => {
            code == "Timeout" && io == "none" && errno == 0
        }
        "query_effect_dropped"
        | "query_rejected_known"
        | "query_rejected_other"
        | "result_rejected"
        | "result_rejected_kind"
        | "result_rejected_path"
        | "result_unsupported" => code == "NotReady" && io == "none" && errno == 0,
        "query_source_changed" => code == "StaleSession" && io == "none" && errno == 0,
        "result_deadline" | "final_deadline" => {
            code == "Timeout" && io == "none" && errno == 0
        }
        "final_source_changed"
        | "final_identity"
        | "final_root_server_id"
        | "final_root_incarnation"
        | "final_root_evidence"
        | "final_root_socket_path"
        | "final_root_invalidation" => {
            code == "StaleSession" && io == "none" && errno == 0
        }
        "workspace_confirm_connect" | "workspace_confirm_snapshot"
        | "workspace_confirm_pane" => {
            matches!(code, "NotReady" | "ResourceLimit") && io != "none"
        }
        "workspace_confirm_evidence_epoch"
        | "workspace_confirm_created_workspace"
        | "workspace_confirm_terminal"
        | "workspace_confirm_focus" => {
            code == "StaleSession" && io == "invalid_data" && errno == 0
        }
        "result_shell_changed" => {
            (code == "NotReady" && io == "none" && errno == 0)
                || (code != "none" && io != "none")
        }
        "query_unavailable" | "result_read_failed" | "result_invalid" | "final_connect"
        | "final_process_info" | "final_result_view" => code != "none" && io != "none",
        "final_shell_absent" | "final_shell_not_idle" => {
            code == "NotReady" && io == "none" && errno == 0
        }
        "final_shell_reporter" | "final_shell_process" | "final_shell_executable" => {
            matches!(code, "NotReady" | "ResourceLimit") && io != "none" && errno == 0
        }
        "final_shell_reporter_recheck" => {
            matches!(code, "NotReady" | "ResourceLimit")
                && (io != "none" || code == "NotReady")
                && errno == 0
        }
        "result_authorization" | "final_source" | "final_root" | "final_shell_changed"
        | "final_authorization" => {
            code != "none" && io == "none" && errno == 0
        }
        _ => false,
    };
    if shell_probe_guard && !valid_shell_probe {
        return None;
    }
    if launch_pending_guard && !(code == "Timeout" && io == "none" && errno == 0) {
        return None;
    }
    if pre_thread_guard
        && (io != "none"
            || errno != 0
            || (matches!(
                guard,
                "pre_thread_deadline_before_reproof"
                    | "pre_thread_deadline_after_reproof"
                    | "pre_thread_stopping"
            ) && code != "Timeout"))
    {
        return None;
    }
    let frame = if transport_guard {
        let frame = fields.next()?.strip_prefix("frame=")?;
        if !matches!(
            frame,
            "none"
                | "frame_header"
                | "frame_length"
                | "frame_cap"
                | "frame_payload"
                | "write_length"
                | "write_header"
                | "write_payload"
                | "write_deadline"
        ) {
            return None;
        }
        frame
    } else {
        "none"
    };
    if transport_guard {
        let valid = match guard {
            "reader_inflight" => code == "none" && io == "none" && frame == "none",
            "reader_frame" => frame.starts_with("frame_") || (frame == "none" && io == "other"),
            "reader_rate_queue" => code == "RateLimited" && io == "cancelled" && frame == "none",
            "reader_request_queue" => code == "none" && io == "cancelled" && frame == "none",
            "reader_decode_queue" => code != "none" && io == "cancelled" && frame == "none",
            "reader_decode_close" => code != "none" && io == "none" && frame == "none",
            "reader_non_request" => code == "InvalidRequest" && io == "none" && frame == "none",
            "extra_stream" | "extra_stream_error" => code == "none" && io == "none" && frame == "none",
            "response_guard" | "response_queue" | "output_guard" | "output_pop_error" => code != "none" && io == "none" && frame == "none",
            "delivery_metadata" => frame.starts_with("frame_") || (code == "none" && io == "other" && frame == "none"),
            "output_write" => frame.starts_with("write_"),
            "output_finish" => frame.starts_with("write_") || (code == "none" && io == "other" && frame == "none"),
            "delivery_storage" => code == "none" && io == "other" && frame == "none",
            "output_queue_closed" | "output_drained_after_reader" => code == "none" && io == "none" && frame == "none",
            _ => false,
        };
        if !valid {
            return None;
        }
    }
    if supervisor_guard
        && ((matches!(guard, "selected_ctrl_c" | "selected_sigterm") && (code != "none" || io != "cancelled"))
            || (matches!(guard, "selected_admin_ok" | "selected_transport_ok" | "selected_topology_ok") && (code != "none" || io != "none"))
            || (!matches!(guard, "selected_ctrl_c" | "selected_sigterm" | "selected_admin_ok" | "selected_transport_ok" | "selected_topology_ok") && code != "none"))
    {
        return None;
    }
    let thread_start = if guard == "thread_effect_drop_start_reply" {
        // This is the only guard carrying the immutable original callback tuple.
        if phase != "thread" || code != "NotReady" || io != "none" || errno != 0 {
            return None;
        }
        let milestone = fields.next()?.strip_prefix("start=")?;
        if !matches!(milestone, "unseen" | "matched" | "delivered" | "receiver_closed") {
            return None;
        }
        let decoded_frame = match fields.next()?.strip_prefix("decoded=")? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let unmatched_response = match fields.next()?.strip_prefix("unmatched=")? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let result = fields.next()?.strip_prefix("result=")?;
        let start_send_elapsed = fields.next()?.strip_prefix("elapsed=")?;
        if !matches!(start_send_elapsed, "unobserved" | "lt_quarter" | "quarter_to_half"
            | "half_to_three_quarters" | "three_quarters_to_budget" | "at_or_over_budget")
        {
            return None;
        }
        let complete_data_frame = match fields.next()?.strip_prefix("data=")? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let (methods_before_mask, methods_before) = parse_codex_thread_start_method_mask(
            fields.next()?.strip_prefix("before=")?,
        )?;
        let (methods_after_mask, methods_after) = parse_codex_thread_start_method_mask(
            fields.next()?.strip_prefix("after=")?,
        )?;
        if !matches!(result, "none" | "BackendError" | "NotReady" | "ResourceLimit")
            || (milestone == "unseen" && result != "none")
            || (milestone != "unseen" && !decoded_frame)
            || (unmatched_response && !decoded_frame)
            || (decoded_frame && !complete_data_frame)
            || ((methods_before_mask != 0 || methods_after_mask != 0)
                && (!decoded_frame || !complete_data_frame))
            || (methods_after_mask != 0 && start_send_elapsed == "unobserved")
        {
            return None;
        }
        Some(json!({"milestone":milestone,"decodedFrame":decoded_frame,"unmatchedResponse":unmatched_response,"result":result,"startSendElapsed":start_send_elapsed,"completeDataFrame":complete_data_frame,"methodsBefore":methods_before,"methodsAfter":methods_after}))
    } else {
        None
    };
    let report = if guard == "native_report_changed" {
        let mut values = [false; 5];
        for (slot, expected) in values.iter_mut().zip([
            "before_present=",
            "after_present=",
            "agent_equal=",
            "kind_equal=",
            "equal=",
        ]) {
            *slot = match fields.next()?.strip_prefix(expected)? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
        }
        Some(
            json!({"beforePresent":values[0],"afterPresent":values[1],"agentEqual":values[2],"kindEqual":values[3],"equal":values[4]}),
        )
    } else {
        None
    };
    if fields.next().is_some() {
        return None;
    }
    let mut value = json!({"gate":"owned-codex-invalidation-capture","phase":phase,"guard":guard,"code":code,"io":io,"errno":errno,"report":report});
    if transport_guard {
        value["frame"] = Value::String(frame.into());
    }
    if let Some(thread_start) = thread_start {
        value["threadStart"] = thread_start;
    }
    Some(value)
}
fn drain_codex_guard(mut stderr: ChildStderr, progress: &CodexDrainProgress) {
    let mut line = Vec::with_capacity(256);
    let mut discard = false;
    let mut chunk = [0u8; 256];
    loop {
        let read = match stderr.read(&mut chunk) {
            Ok(read) => read,
            Err(error) => {
                let kind = OmpObserverFailure::io_kind(error.kind());
                let errno = error
                    .raw_os_error()
                    .filter(|errno| (-4095..=4095).contains(errno));
                progress.update(|summary| {
                    summary.termination = CodexDrainTermination::ReadError;
                    summary.read_error = Some((kind, errno));
                });
                return;
            }
        };
        if read == 0 {
            progress.update(|summary| {
                if !line.is_empty() && line.starts_with(b"goatr_codex_first_guard") {
                    summary.matching_prefix = summary.matching_prefix.saturating_add(1);
                    summary.rejected = summary.rejected.saturating_add(1);
                    if discard {
                        summary.oversized = summary.oversized.saturating_add(1);
                    }
                }
                summary.eof = true;
                summary.termination = CodexDrainTermination::Eof;
                summary.completed = summary.rejected == 0
                    && summary.oversized == 0
                    && !summary.output_failed;
            });
            return;
        }
        for byte in &chunk[..read] {
            if *byte == b'\n' {
                if line.starts_with(b"goatr_codex_first_guard") {
                    progress.update(|summary| {
                        summary.matching_prefix = summary.matching_prefix.saturating_add(1);
                    });
                    if !discard
                        && progress.snapshot().accepted < 8
                        && let Some(value) = parse_codex_guard_line(&line)
                    {
                        if writeln!(std::io::stdout().lock(), "{value}").is_ok() {
                            progress.update(|summary| summary.accepted += 1);
                        } else {
                            progress.update(|summary| {
                                summary.rejected = summary.rejected.saturating_add(1);
                                summary.output_failed = true;
                            });
                        }
                    } else {
                        progress.update(|summary| {
                            summary.rejected = summary.rejected.saturating_add(1);
                            if discard {
                                summary.oversized = summary.oversized.saturating_add(1);
                            }
                        });
                    }
                }
                line.clear();
                discard = false;
            } else if line.len() < 256 {
                line.push(*byte);
            } else {
                discard = true;
            }
        }
    }
}
fn parse_omp_guard_line(line: &[u8]) -> Option<Value> {
    let line = std::str::from_utf8(line).ok()?;
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "goatr_omp_first_guard" {
        return None;
    }
    let phase = fields.next()?.strip_prefix("phase=")?;
    if !matches!(
        phase,
        "herdr_launch"
            | "launch_preflight"
            | "omp_ready"
            | "omp_connection"
            | "follower"
            | "herdr"
            | "transport"
            | "shell_probe"
    ) {
        return None;
    }
    let guard = fields.next()?.strip_prefix("guard=")?;
    let launch_preflight_guard = matches!(
        guard,
        "project_lookup" | "project_path" | "creation_overlay" | "creation_overlay_path"
            | "diagnostic_extension_args" | "source_lookup" | "topology_source"
            | "topology_source_changed" | "topology_project" | "topology_focus"
            | "measurement_absent" | "wrapper_recheck_failure" | "wrapper_recheck_mismatch"
            | "pane_absent" | "pane_mismatch" | "resource_mismatch" | "resource_pane_absent"
            | "resource_terminal_absent" | "process_connect" | "process_evidence"
            | "process_info" | "shell_reporter" | "shell_absent" | "shell_not_idle"
            | "shell_process" | "shell_executable" | "shell_reporter_recheck"
            | "original_root" | "root_recheck_source" | "root_recheck_pane"
            | "root_recheck_workspace" | "root_recheck_terminal" | "root_recheck_focus"
            | "root_recheck_pane_focused" | "root_recheck_agent"
            | "root_recheck_agent_session" | "root_recheck_agent_status"
            | "root_recheck_agents" | "original_shell_process" | "original_shell_mismatch"
            | "original_executable_recheck" | "original_executable_mismatch" | "launch_connect"
            | "launch_evidence" | "launch_socket_fence"
    );
    if phase == "launch_preflight" && !launch_preflight_guard {
        return None;
    }
    if !matches!(
        guard,
        "herdr_response_timeout"
            | "herdr_response_io"
            | "ack_matched"
            | "ack_mismatch"
            | "rejected_known"
            | "rejected_other"
            | "source_changed"
            | "unavailable"
            | "project_lookup"
            | "project_path"
            | "creation_overlay"
            | "creation_overlay_path"
            | "diagnostic_extension_args"
            | "topology_source"
            | "topology_source_changed"
            | "topology_project"
            | "topology_focus"
            | "measurement_absent"
            | "wrapper_recheck_failure"
            | "wrapper_recheck_mismatch"
            | "pane_absent"
            | "pane_mismatch"
            | "resource_mismatch"
            | "resource_pane_absent"
            | "resource_terminal_absent"
            | "process_connect"
            | "process_evidence"
            | "original_root"
            | "root_recheck_source"
            | "root_recheck_pane"
            | "root_recheck_workspace"
            | "root_recheck_terminal"
            | "root_recheck_focus"
            | "root_recheck_pane_focused"
            | "root_recheck_agent"
            | "root_recheck_agent_session"
            | "root_recheck_agent_status"
            | "root_recheck_agents"
            | "original_shell_process"
            | "original_shell_mismatch"
            | "original_executable_recheck"
            | "original_executable_mismatch"
            | "launch_connect"
            | "launch_evidence"
            | "launch_socket_fence"
            | "topology_not_ready"
            | "topology_failure"
            | "native_report_absent"
            | "report_absent_after_interactive_ready"
            | "report_absent_without_interactive_ready"
            | "original_agent_absent"
            | "native_report_wrong"
            | "no_connection"
            | "native_identity"
            | "source_lookup"
            | "source_pending"
            | "source_identity"
            | "source_recheck_io"
            | "observation_pending"
            | "follower_native_report_absent"
            | "follower_native_report_wrong"
            | "follower_registry_absent"
            | "follower_registry_read"
            | "follower_registry_snapshot"
            | "follower_owner_join"
            | "follower_ambiguous"
            | "follower_history"
            | "follower_auth"
            | "follower_source"
            | "follower_other"
            | "follower_native_read"
            | "follower_registry"
            | "follower_registry_owner"
            | "follower_registry_control"
            | "follower_registry_relay"
            | "follower_validation"
            | "subscription_connect"
            | "subscription_event"
            | "topology_sync"
            | "subscription_ended"
            | "snapshot_connect"
            | "snapshot_read"
            | "incarnation_changed"
            | "native_report_changed"
            | "terminal_absent"
            | "effective_topology_loss"
            | "dispatch_returned"
            | "dispatch_deadline"
            | "response_queued"
            | "output_written_after_queue"
            | "reader_frame"
            | "response_queue"
            | "output_pop_error"
            | "output_queue_closed"
            | "output_guard"
            | "output_write"
            | "output_finish"
            | "delivery_metadata"
            | "delivery_storage"
            | "response_guard"
            | "extra_stream"
            | "extra_stream_error"
            | "reader_task_panicked"
            | "connection_task_panicked"
            | "source_unavailable_at_deadline"
            | "deadline"
            | "receipt"
            | "receipt_progress"
            | "local_absent"
            | "local_storage"
            | "precondition"
            | "focus_missing"
            | "authorization"
            | "root_source_recheck_at_deadline"
            | "root_pane_at_deadline"
            | "root_workspace_at_deadline"
            | "root_terminal_at_deadline"
            | "root_focus_at_deadline"
            | "root_pane_focused_at_deadline"
            | "root_agent_at_deadline"
            | "root_agent_session_at_deadline"
            | "root_agent_status_at_deadline"
            | "root_agents_at_deadline"
            | "root_recheck_source"
            | "root_recheck_pane"
            | "root_recheck_workspace"
            | "root_recheck_terminal"
            | "root_recheck_focus"
            | "root_recheck_pane_focused"
            | "root_recheck_agent"
            | "root_recheck_agent_session"
            | "root_recheck_agent_status"
            | "root_recheck_agents"
            | "source_connect"
            | "process_info"
            | "shell_reporter"
            | "shell_absent"
            | "shell_not_idle"
            | "shell_process"
            | "shell_executable"
            | "shell_reporter_recheck"
            | "artifact_create"
            | "nonce"
            | "wrapper"
            | "command"
            | "metadata_workspace"
            | "metadata_executable"
            | "metadata_result_path"
            | "fence_connect"
            | "fence_identity"
            | "fence_socket"
            | "query_rejected_known"
            | "query_rejected_other"
            | "query_source_changed"
            | "query_unavailable"
            | "query_effect_dropped"
            | "result_shell_changed"
            | "result_read_failed"
            | "result_invalid"
            | "result_deadline"
            | "result_rejected"
            | "result_rejected_kind"
            | "result_rejected_path"
            | "result_unsupported"
    ) {
        return None;
    }
    let code = fields.next()?.strip_prefix("code=")?;
    if phase == "launch_preflight" && code == "none" {
        return None;
    }
    if !matches!(
        code,
        "none"
            | "InvalidRequest"
            | "UnsupportedVersion"
            | "UnsupportedMethod"
            | "Unauthorized"
            | "PairingFailed"
            | "StaleHost"
            | "StaleSession"
            | "NotFound"
            | "NotReady"
            | "InvalidAnswer"
            | "AlreadyResolved"
            | "ActionIdConflict"
            | "ActionCapacity"
            | "ContentExpired"
            | "CursorExpired"
            | "Busy"
            | "ResourceLimit"
            | "RateLimited"
            | "Timeout"
            | "BackendError"
            | "PushRejected"
            | "StaleRegistration"
    ) {
        return None;
    }
    let io = fields.next()?.strip_prefix("io=")?;
    if !matches!(
        io,
        "none"
            | "not_found"
            | "timed_out"
            | "connection_refused"
            | "unexpected_eof"
            | "connection_reset"
            | "connection_aborted"
            | "broken_pipe"
            | "not_connected"
            | "permission_denied"
            | "invalid_data"
            | "other"
    ) {
        return None;
    }
    let frame = fields.next()?.strip_prefix("frame=")?;
    if !matches!(
        frame,
        "none"
            | "frame_header"
            | "frame_length"
            | "frame_cap"
            | "frame_payload"
            | "write_length"
            | "write_header"
            | "write_payload"
            | "write_deadline"
    ) {
        return None;
    }
    let ack = if matches!(guard, "ack_matched" | "ack_mismatch") {
        let mut values = [false; 6];
        for (slot, expected) in values.iter_mut().zip([
            "name=",
            "kind=",
            "pane=",
            "terminal=",
            "workspace=",
            "argv=",
        ]) {
            *slot = match fields.next()?.strip_prefix(expected)? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
        }
        Some(
            json!({"name":values[0],"kind":values[1],"pane":values[2],"terminal":values[3],"workspace":values[4],"argv":values[5]}),
        )
    } else {
        None
    };
    if fields.next().is_some() {
        return None;
    }
    Some(
        json!({"gate":"owned-omp-creation-capture","phase":phase,"guard":guard,"code":code,"io":io,"frame":frame,"ack":ack}),
    )
}
fn parse_hello_guard_line(line: &[u8]) -> Option<Value> {
    let line = std::str::from_utf8(line).ok()?;
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "goatr_hello_capture" {
        return None;
    }
    let phase = fields.next()?.strip_prefix("phase=")?;
    let guard = fields.next()?.strip_prefix("guard=")?;
    let code = fields.next()?.strip_prefix("code=")?;
    let io = fields.next()?.strip_prefix("io=")?;
    if fields.next().is_some() {
        return None;
    }
    let known_code = matches!(
        code,
        "none"
            | "InvalidRequest"
            | "UnsupportedVersion"
            | "UnsupportedMethod"
            | "Unauthorized"
            | "PairingFailed"
            | "StaleHost"
            | "StaleSession"
            | "NotFound"
            | "NotReady"
            | "InvalidAnswer"
            | "AlreadyResolved"
            | "ActionIdConflict"
            | "ActionCapacity"
            | "ContentExpired"
            | "CursorExpired"
            | "Busy"
            | "ResourceLimit"
            | "RateLimited"
            | "Timeout"
            | "BackendError"
            | "PushRejected"
            | "StaleRegistration"
    );
    let frame_io = matches!(
        io,
        "none" | "unexpected_eof" | "connection_reset" | "not_connected" | "timed_out" | "other"
    );
    let accepted = matches!(
        (phase, guard, code, io),
        ("admission", "entered", "none", "none")
            | ("admission", "alpn_mismatch", "none", "none")
            | ("admission", "incoming_error", "none", "other")
            | ("admission", "incoming_deadline", "Timeout", "timed_out")
            | ("admission", "handshake_rejected", "none", "none")
            | ("stream_accept", "entered", "none", "none")
            | ("stream_accept", "stream_error", "none", "other")
            | ("stream_accept", "deadline", "Timeout", "timed_out")
            | ("hello_frame", "entered", "none", "none")
            | ("hello_frame", "deadline", "Timeout", "timed_out")
            | ("hello_frame", "invalid_request", "InvalidRequest", "none")
            | ("hello_frame", "not_hello", "Unauthorized", "none")
            | ("hello_authorization", "entered", "none", "none")
            | ("security_guard", "entered", "none", "none")
            | ("backend_capabilities", "entered", "none", "none")
            | ("response_write", "entered", "none", "none")
            | ("complete", "completed", "none", "none")
    ) || (known_code && frame_io && matches!(
        (phase, guard),
        ("hello_frame", "read_error") | ("response_write", "write_error")
    )) || (known_code && io == "none" && matches!(
        (phase, guard),
        ("hello_authorization", "rejected") | ("security_guard", "rejected")
    ));
    accepted.then(|| json!({"gate":"owned-hello-capture","phase":phase,"guard":guard,"code":code,"io":io}))
}
fn drain_hello_guard(mut stderr: ChildStderr) {
    let mut line = Vec::with_capacity(192);
    let mut discard = false;
    let mut emitted = 0u8;
    let mut chunk = [0u8; 192];
    loop {
        let Ok(read) = stderr.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        for byte in &chunk[..read] {
            if *byte == b'\n' {
                if !discard
                    && emitted < 8
                    && let Some(value) = parse_hello_guard_line(&line)
                {
                    let _ = writeln!(std::io::stdout().lock(), "{value}");
                    emitted += 1;
                }
                line.clear();
                discard = false;
            } else if line.len() < 192 {
                line.push(*byte);
            } else {
                discard = true;
            }
        }
    }
}
fn drain_omp_guard(mut stderr: ChildStderr, budget: herdr_report_capture::Budget) {
    let mut line = Vec::with_capacity(256);
    let mut discard = false;
    let mut emitted = 0u8;
    let mut chunk = [0u8; 256];
    loop {
        let Ok(read) = stderr.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        for byte in &chunk[..read] {
            if *byte == b'\n' {
                if !discard
                    && emitted < 8
                    && let Some(value) = parse_omp_guard_line(&line)
                {
                    budget.omp(value);
                    emitted += 1;
                }
                line.clear();
                discard = false;
            } else if line.len() < 256 {
                line.push(*byte);
            } else {
                discard = true;
            }
        }
    }
}
fn start_serve(
    binary: &Path,
    config: &Path,
    root: &Path,
    socket: &Path,
    listener: Option<&std::net::TcpListener>,
) -> io::Result<Companion> {
    start_serve_with_codex_capture(binary, config, root, socket, listener, false, false, None)
}
fn start_serve_with_codex_capture(
    binary: &Path,
    config: &Path,
    root: &Path,
    socket: &Path,
    listener: Option<&std::net::TcpListener>,
    diagnostic_capture: bool,
    codex_startup_terminal_capture: bool,
    herdr_report_capture: Option<&herdr_report_capture::Capture>,
) -> io::Result<Companion> {
    start_serve_with_capture(
        binary,
        config,
        root,
        socket,
        listener,
        ServeLaunch {
            diagnostic_capture,
            codex_startup_terminal_capture,
            herdr_report_capture,
            preserve_xdg_config_home: false,
        },
    )
}
fn start_serve_with_capture(
    binary: &Path,
    config: &Path,
    root: &Path,
    socket: &Path,
    listener: Option<&std::net::TcpListener>,
    launch: ServeLaunch<'_>,
) -> io::Result<Companion> {
    let codex_capture = launch.diagnostic_capture
        || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    if launch.codex_startup_terminal_capture && (!codex_capture || listener.is_none()) {
        return Err(failure(
            "Codex startup terminal capture requires the validated owned Codex relay",
        ));
    }
    let omp_creation_capture = owned_omp_creation_capture_requested();
    let omp_offline_capture = owned_omp_offline_capture_requested();
    let omp_backend_capture = owned_omp_backend_capture_requested();
    let hello_capture = owned_hello_capture_requested();
    if owned_omp_capture_conflicts() {
        return Err(failure(
            "owned OMP creation, post-Ready offline, and backend-only captures are mutually exclusive",
        ));
    }
    let omp_receiver_capture = omp_creation_capture || omp_offline_capture;
    if omp_receiver_capture && launch.herdr_report_capture.is_none() {
        return Err(failure(
            "owned OMP creation or post-Ready offline capture requires private receiver",
        ));
    }
    if omp_backend_capture && launch.herdr_report_capture.is_some() {
        return Err(failure(
            "owned OMP backend-only capture does not use private receiver",
        ));
    }
    let omp_capture = omp_receiver_capture || omp_backend_capture;
    if codex_capture && omp_capture {
        return Err(failure("owned Codex and OMP diagnostic captures are mutually exclusive"));
    }
    if hello_capture && (omp_capture || codex_capture) {
        return Err(failure(
            "owned Hello, OMP, and Codex diagnostic captures are mutually exclusive",
        ));
    }
    if (omp_capture || hello_capture) && listener.is_none() {
        return Err(failure(
            "owned diagnostic capture requires validated owned relay",
        ));
    }
    let capture = codex_capture || omp_capture || hello_capture;
    let omp_budget = if let Some(capture) = launch.herdr_report_capture {
        Some(capture.budget())
    } else {
        omp_backend_capture.then(herdr_report_capture::Budget::new_backend)
    };
    let drain_budget = omp_budget.clone();
    let mut c = std::process::Command::new(binary);
    c.arg("serve")
        .arg("--config")
        .arg(config)
        .arg("--admin-socket")
        .arg(socket);
    if !launch.preserve_xdg_config_home {
        c.env(
            "XDG_CONFIG_HOME",
            socket
                .parent()
                .ok_or_else(|| failure("owned admin parent missing"))?
                .join("config"),
        );
    }
    c.env("XDG_STATE_HOME", root.join("state"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    c.env_remove("GOATR_OWNED_CODEX_DIAGNOSTIC");
    c.env_remove("GOATR_OWNED_CODEX_STARTUP_TERMINAL_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_CREATION_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_OFFLINE_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_BACKEND_CAPTURE");
    c.env_remove("GOATR_OWNED_HELLO_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_PRODUCER_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_PRODUCER_OVERLAY_SHA256");
    c.env_remove("GOATR_OWNED_OMP_PRODUCER_HOOK_SHA256");
    if codex_capture {
        c.env("GOATR_OWNED_CODEX_DIAGNOSTIC", "1");
        if launch.codex_startup_terminal_capture {
            c.env("GOATR_OWNED_CODEX_STARTUP_TERMINAL_CAPTURE", "1");
        }
    } else if omp_capture {
        if omp_creation_capture {
            c.env("GOATR_OWNED_OMP_CREATION_CAPTURE", "1");
        } else if omp_offline_capture {
            c.env("GOATR_OWNED_OMP_OFFLINE_CAPTURE", "1");
        } else {
            c.env("GOATR_OWNED_OMP_BACKEND_CAPTURE", "1");
        }
    } else if hello_capture {
        c.env("GOATR_OWNED_HELLO_CAPTURE", "1");
    }
    if let Some(capture) = launch.herdr_report_capture {
        capture.configure_companion(&mut c);
    }
    let inherited = if let Some(listener) = listener {
        let fd = listener.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        c.arg("--owned-omp-relay-fd").arg(fd.to_string());
        Some((fd, flags))
    } else {
        None
    };
    let spawned = c.spawn();
    if let Some((fd, flags)) = inherited
        && unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut child = spawned?;
    let process = herdr::process_evidence(child.id())?;
    let guard_drain = if capture {
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| failure("owned diagnostic capture pipe unavailable"))?;
        if codex_capture {
            let progress = Arc::new(CodexDrainProgress::default());
            let worker_progress = progress.clone();
            Some(GuardDrain::Codex {
                worker: std::thread::spawn(move || {
                    drain_codex_guard(stderr, &worker_progress)
                }),
                progress,
            })
        } else {
            Some(GuardDrain::Other(std::thread::spawn(move || {
                if hello_capture {
                drain_hello_guard(stderr);
                } else if let Some(budget) = drain_budget {
                    drain_omp_guard(stderr, budget);
                }
            })))
        }
    } else {
        None
    };
    Ok(Companion {
        child,
        process,
        state_root: root.join("state").join("goatr"),
        binary: binary.to_owned(),
        guard_drain,
        omp_capture_budget: omp_budget,
        codex_cleanup_signal: CleanupSignal::default(),
        codex_cleanup_active: false,
        codex_startup_terminal: None,
        codex_startup_journal: None,
        codex_startup_journal_admission: None,
    })
}
async fn healthy(serve: &mut Companion, socket: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if serve.child.try_wait()?.is_some() {
            return Err(prerequisite(
                "actual packaged configured serve exited; source/wrapper/host prerequisites unavailable",
            ));
        }
        if goatr::admin::call(socket, "devices.list", json!({}))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(prerequisite(
                "actual private devices.list readiness unavailable",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn physical_hello_capture(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    let hello_capture = owned_hello_capture_requested();
    let omp_backend_capture = owned_omp_backend_capture_requested();
    if (hello_capture == omp_backend_capture)
        || (!hello_capture && selected_omp_capture_mode() != OwnedOmpCaptureMode::Backend)
        || (hello_capture && selected_omp_capture_mode() != OwnedOmpCaptureMode::None)
        || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("1"))
        || std::env::var_os("GOATR_OWNED_OMP_PRODUCER_CAPTURE").as_deref()
            == Some(std::ffi::OsStr::new("1"))
    {
        return Err(failure(
            "physical diagnostic capture requires exactly one Hello or OMP backend capture",
        ));
    }
    let binary = dev_continuity::frozen_package(args)?;
    let config_path = fs::canonicalize(
        args.get_one::<PathBuf>("config")
            .ok_or_else(|| failure("physical Hello capture requires --config"))?,
    )?;
    let config = Config::load(&config_path)?;
    private_dir(&config.runtime_dir, false)?;
    let root = config
        .runtime_dir
        .parent()
        .ok_or_else(|| failure("physical Hello runtime parent missing"))?
        .to_path_buf();
    private_dir(&root, false)?;
    private_dir(&root.join("state"), false)?;
    let socket = args
        .get_one::<PathBuf>("admin-socket")
        .cloned()
        .ok_or_else(|| failure("physical Hello capture requires --admin-socket"))?;
    if !socket.is_absolute() {
        return Err(failure(
            "physical Hello admin socket must be the absent exact owned controller path",
        ));
    }
    match fs::symlink_metadata(&socket) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(failure(
                "physical Hello admin socket must be the absent exact owned controller path",
            ));
        }
        Err(error) => return Err(error),
    }
    let admin = socket
        .parent()
        .ok_or_else(|| failure("physical Hello admin parent missing"))?;
    private_dir(admin, false)?;
    goatr::admin::validate_socket_path(&socket, false, false)?;
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 7466))?;
    if listener.local_addr()?.port() != 7466 {
        return Err(failure("physical Hello relay listener port changed"));
    }
    listener.set_nonblocking(true)?;
    let metadata = fs::metadata(format!("/proc/self/fd/{}", listener.as_raw_fd()))?;
    let record = goatr::backends::omp::OwnedRelayListener {
        v: 1,
        fd: listener.as_raw_fd(),
        device: metadata.dev(),
        inode: metadata.ino(),
        port: 7466,
        parent: herdr::process_evidence(std::process::id())?,
        runtime_dir: config.runtime_dir.clone(),
    };
    let relay_owner = config.runtime_dir.join("omp-relay.owner");
    let lock = goatr::setup::SetupLock::acquire(&config.runtime_dir)?;
    lock.publish(
        &relay_owner,
        &goatr::setup::ExpectedFile::Absent,
        &serde_json::to_vec(&record).map_err(io::Error::other)?,
    )?;
    drop(lock);
    let run = Uuid::random();
    write_json(
        &evidence.join(format!("physical-hello-capture-intent-{}.json", run.0)),
        &json!({"runId":run,"config":identity(&config_path)?,"runtimeDir":identity(&config.runtime_dir)?,"stateHome":identity(&root.join("state"))?,"adminParent":identity(admin)?,"adminSocket":socket,"relayOwner":identity(&relay_owner)?,"listener":record,"package":binary,"scope":"ordinary serve only; no setup, pair, provider creation, input, transport smoke, or second session"}),
    )?;
    let mut serve = start_serve_with_capture(
        &binary,
        &config_path,
        &root,
        &socket,
        Some(&listener),
        ServeLaunch {
            diagnostic_capture: false,
            codex_startup_terminal_capture: false,
            herdr_report_capture: None,
            // This controller observes the existing paired desktop configuration.
            preserve_xdg_config_home: true,
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        serve.recheck()?;
        match fs::symlink_metadata(&socket) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && metadata.uid() == uid()
                    && metadata.mode() & 0o7777 == 0o600 =>
            {
                break;
            }
            Ok(_) => return Err(failure("physical Hello admin socket identity rejected")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(failure("physical Hello owned serve socket was not observed"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    write_json(
        &evidence.join(format!("physical-hello-capture-spawn-{}.json", run.0)),
        &json!({"runId":run,"parent":record.parent,"relayOwner":identity(&relay_owner)?,"listener":record,"companion":serve.process,"adminSocket":identity(&socket)?,"config":identity(&config_path)?,"stateHome":identity(&root.join("state"))?,"scope":"original parent and inherited relay FD retained until explicit termination"}),
    )?;
    println!("{}", json!({"gate":"physical-hello-capture","status":"ready","runId":run,"adminSocket":identity(&socket)?,"scope":"ordinary paired-phone serve; first accepted connection diagnostic only"}));
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate.recv() => {},
        result = async {
            loop {
                if serve.child.try_wait()?.is_some() {
                    return Err(failure("physical Hello owned serve exited before explicit termination"));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => result?,
    }
    serve.stop().await?;
    write_json(
        &evidence.join(format!("physical-hello-capture-outcome-{}.json", run.0)),
        &json!({"runId":run,"status":"terminated","companion":serve.process,"relayOwner":identity(&relay_owner)?,"scope":"explicit controller termination; no provider cleanup"}),
    )?;
    Ok(true)
}
async fn pair(
    binary: &Path,
    root: &Path,
    socket: &Path,
    endpoint: &iroh::Endpoint,
) -> io::Result<Phone> {
    let b = binary.to_owned();
    let r = root.to_owned();
    let s = socket.to_owned();
    let mut uri = tokio::task::spawn_blocking(move || terminal_pair(&b, Some(&r), Some(&s)))
        .await
        .map_err(|_| {
            failure("phone setup failed: phase=phone_pair; guard=task_join; no retry")
        })??;
    let invite =
        PairInvite::parse(&uri).map_err(|_| failure("actual private pair URI rejected"))?;
    unsafe { uri.as_bytes_mut().fill(0) };
    uri.clear();
    let host = invite.host.clone();
    connect(endpoint, &host, Some(invite)).await
}

#[derive(Clone, Copy)]
pub(super) struct FeedbackPresentation {
    pub(super) provider: Provider,
    pub(super) focus_changed: bool,
    pub(super) geometry_changed: Option<bool>,
}

impl FeedbackPresentation {
    fn changed(self) -> bool {
        self.focus_changed || self.geometry_changed == Some(true)
    }
}

fn feedback_presentation_recordable_parts(state: ReceiptState, has_error: bool) -> bool {
    state == ReceiptState::Succeeded && !has_error
}

fn feedback_presentation_recordable(receipt: &Receipt) -> bool {
    feedback_presentation_recordable_parts(receipt.state, receipt.error.is_some())
}

fn creation_presentation(
    provider: Provider,
    feedback_presentation: bool,
    focus_changed: bool,
    geometry_changed: Option<bool>,
) -> io::Result<Option<FeedbackPresentation>> {
    let presentation = FeedbackPresentation {
        provider,
        focus_changed,
        geometry_changed,
    };
    if presentation.changed() && !feedback_presentation {
        return Err(failure(
            "actual desktop focus/geometry changed during create; runner never restores it",
        ));
    }
    Ok(feedback_presentation.then_some(presentation))
}

pub(super) fn write_feedback_presentation(
    evidence: &Path,
    run: &Uuid,
    receipt: &Receipt,
    presentation: FeedbackPresentation,
) -> io::Result<()> {
    if !feedback_presentation_recordable(receipt) {
        return Err(failure(
            "owner feedback presentation evidence requires the original successful create receipt",
        ));
    }
    write_json(
        &evidence.join(format!(
            "development-owner-feedback-presentation-{}-{:?}.json",
            run.0, presentation.provider
        )),
        &json!({
            "gate":"development-owner-feedback",
            "runId":run,
            "provider":presentation.provider,
            "focusChanged":presentation.focus_changed,
            "geometryChanged":presentation.geometry_changed,
            "noFocusAcceptance":false,
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexStartupTerminalPin {
    action: Uuid,
    server_id: Uuid,
    source_incarnation: Uuid,
    source_evidence: ServerEvidence,
    socket_path: PathBuf,
    workspace_id: Id,
    tab_id: Id,
    pane_id: Id,
    terminal_id: Id,
    read_identity: PaneReadIdentity,
}
struct CodexStartupTerminalContext<'a> {
    config: &'a Config,
    socket: &'a Path,
    server: &'a Uuid,
    provider: Provider,
    desktop: &'a ProcessEvidence,
}

// Immutable private records retain their complete object stamp across the native read.
type StartupTerminalFileStamp = (u64, u64, u64, i64, i64, i64, i64, u32, u32, u32, u64);
fn startup_terminal_file_stamp(metadata: &fs::Metadata) -> io::Result<StartupTerminalFileStamp> {
    if !metadata.is_file()
        || metadata.uid() != uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(failure("terminal capture private record metadata rejected"));
    }
    Ok((metadata.dev(), metadata.ino(), metadata.len(), metadata.mtime(),
        metadata.mtime_nsec(), metadata.ctime(), metadata.ctime_nsec(),
        metadata.mode(), metadata.uid(), metadata.gid(), metadata.nlink()))
}
fn read_codex_startup_terminal_pin(
    runtime_dir: &Path,
    action: &Uuid,
) -> io::Result<(PathBuf, File, StartupTerminalFileStamp, CodexStartupTerminalPin)> {
    const PIN_MAX: u64 = 4096;
    pairing::private_dir(runtime_dir, false)?;
    let path = runtime_dir.join(format!("codex-startup-terminal-{}.json", action.0));
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)?;
    let held = startup_terminal_file_stamp(&file.metadata()?)?;
    if held.2 == 0 || held.2 > PIN_MAX
        || startup_terminal_file_stamp(&fs::symlink_metadata(&path)?)? != held
    {
        return Err(failure("terminal capture pin size or path rejected"));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(held.2).map_err(io::Error::other)?);
    (&mut file).take(PIN_MAX + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != held.2
        || startup_terminal_file_stamp(&file.metadata()?)? != held
        || startup_terminal_file_stamp(&fs::symlink_metadata(&path)?)? != held
    {
        return Err(failure("terminal capture pin changed during bounded read"));
    }
    let pin = serde_json::from_slice(&bytes)?;
    Ok((path, file, held, pin))
}
fn recheck_startup_terminal_desktop(
    pin: &CodexStartupTerminalPin,
    context: &CodexStartupTerminalContext<'_>,
) -> io::Result<()> {
    let evidence = &pin.source_evidence;
    if evidence.pid != context.desktop.pid
        || evidence.pid_start_ticks != context.desktop.start_ticks
        || evidence.linux_boot_id != context.desktop.linux_boot_id
        || herdr::process_evidence(context.desktop.pid)? != *context.desktop
    {
        return Err(failure("terminal capture original desktop identity changed"));
    }
    let socket = fs::symlink_metadata(context.socket)?;
    if !socket.file_type().is_socket()
        || socket.uid() != uid()
        || socket.mode() & 0o7777 != 0o600
        || (socket.dev(), socket.ino()) != (evidence.socket_device, evidence.socket_inode)
    {
        return Err(failure("terminal capture original desktop socket changed"));
    }
    Ok(())
}

fn codex_startup_terminal_classifier(text: &str, truncated: bool) -> &'static str {
    if truncated {
        return "startup_terminal_truncated";
    }
    let compact = text
        .split_whitespace()
        .collect::<String>();
    if compact.is_empty() {
        return "startup_terminal_empty";
    }
    let account_context = "account/readfailedduringTUIbootstrap:";
    let mut families = Vec::new();
    let mut account = None;
    let mut remaining = compact.as_str();
    while let Some(index) = remaining.find(account_context) {
        let tail = &remaining[index + account_context.len()..];
        let current = if tail.starts_with("account/readtransporterror:") {
            "startup_terminal_account_transport"
        } else if tail.starts_with("account/readresponsedecodeerror:") {
            "startup_terminal_account_decode"
        } else if tail.starts_with("account/readfailed:") {
            "startup_terminal_account_server"
        } else {
            "startup_terminal_account_bootstrap_unknown"
        };
        if account.is_some_and(|prior| prior != current) {
            return "startup_terminal_multiple";
        }
        account = Some(current);
        remaining = tail;
    }
    if let Some(account) = account {
        families.push(account);
    }
    if compact.contains("model/listfailedduringTUIbootstrap:") {
        families.push("startup_terminal_models_bootstrap");
    }
    if compact.contains("configRequirements/readfailedduringTUIbootstrap:") {
        families.push("startup_terminal_requirements_bootstrap");
    }
    if compact.contains("thread/readfailedduringTUIsessionlookup") {
        families.push("startup_terminal_thread_lookup");
    }
    if compact.contains("terminalinputstreamclosedduringstartup") {
        families.push("startup_terminal_event_stream_closed");
    }
    if compact.contains("Errorloadingconfiguration:") {
        families.push("startup_terminal_configuration_load");
    }
    if compact.contains("Errorloadingconfig.toml:") {
        families.push("startup_terminal_configuration_toml");
    }
    match families.as_slice() {
        [] => "startup_terminal_unclassified",
        [guard] => guard,
        _ => "startup_terminal_multiple",
    }
}

async fn capture_codex_startup_terminal(
    runtime_dir: &Path,
    receipt: &Receipt,
    context: &CodexStartupTerminalContext<'_>,
) -> CodexStartupTerminalResult {
    let result = async {
        let (pin_path, pin_file, pin_object, pin) =
            read_codex_startup_terminal_pin(runtime_dir, &receipt.action_id)?;
        let resources = receipt
            .creation
            .as_ref()
            .filter(|creation| creation.provider == context.provider && creation.server_id == *context.server)
            .map(|creation| &creation.resources)
            .ok_or_else(|| failure("terminal capture receipt resources absent"))?;
        let binding = resources
            .binding_id
            .as_ref()
            .ok_or_else(|| failure("terminal capture binding absent"))?;
        let native = resources
            .native_id
            .as_ref()
            .ok_or_else(|| failure("terminal capture native id absent"))?;
        if context.config.codex_owners.len() != 1 {
            return Err(failure("terminal capture owner scope rejected"));
        }
        let owner = &context.config.codex_owners[0];
        let manifest_path = goatr::codex_relay::manifest_path(&owner.bindings_dir, binding);
        let manifest_before = startup_terminal_file_stamp(&fs::symlink_metadata(&manifest_path)?)?;
        let manifest = goatr::codex_relay::Manifest::load(&manifest_path)?;
        let manifest_after = startup_terminal_file_stamp(&fs::symlink_metadata(&manifest_path)?)?;
        if manifest_before != manifest_after
            || manifest.owner_id != owner.id
            || manifest.upstream_host != owner.socket_host
            || manifest.upstream_agent != owner.socket_agent
            || manifest.binding_id != *binding
            || manifest.expected_thread_id != *native
            || manifest.herdr.server_id != *context.server
            || manifest.herdr.server_id != pin.server_id
            || manifest.herdr.incarnation != pin.source_incarnation
            || manifest.herdr.workspace_id != pin.workspace_id
            || manifest.herdr.pane_id != pin.pane_id
            || manifest.herdr.terminal_id != pin.terminal_id
            || manifest.herdr.socket_path != pin.socket_path
            || manifest.herdr.socket_path != context.socket
            || manifest.herdr.evidence != pin.source_evidence
        {
            return Err(failure("terminal capture manifest provenance rejected"));
        }
        if pin.action != receipt.action_id
            || resources.workspace_id.as_ref() != Some(&pin.workspace_id)
            || resources.pane_id.as_ref() != Some(&pin.pane_id)
            || resources.terminal_id.as_ref() != Some(&pin.terminal_id)
        {
            return Err(failure("terminal capture retained provenance rejected"));
        }
        recheck_startup_terminal_desktop(&pin, context)?;
        let snapshot_client = Client::connect(&pin.socket_path).await?;
        if snapshot_client.evidence != pin.source_evidence {
            return Err(failure("terminal capture original socket evidence changed"));
        }
        let (evidence, snapshot) = snapshot_client.snapshot().await?;
        if evidence != pin.source_evidence {
            return Err(failure("terminal capture snapshot evidence changed"));
        }
        let pane = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == pin.pane_id)
            .ok_or_else(|| failure("terminal capture pane absent"))?;
        let identity = pane
            .read_identity
            .clone()
            .ok_or_else(|| failure("terminal capture read identity absent"))?;
        if pane.workspace_id != pin.workspace_id
            || pane.tab_id != pin.tab_id
            || pane.terminal_id != pin.terminal_id
            || identity.herdr_incarnation != pin.read_identity.herdr_incarnation
            || identity.runtime_epoch != pin.read_identity.runtime_epoch
        {
            return Err(failure("terminal capture replacement rejected"));
        }
        // A fresh binding epoch cannot adopt a different agent on the pinned runtime.
        if pane.agent.as_deref().is_some_and(|agent| agent != "codex")
            || pane.agent_session.as_ref().is_some_and(|session| {
                session.source != "herdr:codex" || session.agent != "codex"
                    || session.kind != "id" || session.value != native.0
            })
        {
            return Err(failure("terminal capture conflicting native binding"));
        }
        let read_client = Client::connect(&pin.socket_path).await?;
        if read_client.evidence != pin.source_evidence {
            return Err(failure("terminal capture read socket evidence changed"));
        }
        let read = read_client
            .read_bound(&pin.pane_id, &pin.terminal_id, &identity)
            .await?;
        if read.workspace_id != pin.workspace_id || read.tab_id != pin.tab_id {
            return Err(failure("terminal capture read routing changed"));
        }
        manifest.validate()?;
        recheck_startup_terminal_desktop(&pin, context)?;
        if startup_terminal_file_stamp(&pin_file.metadata()?)? != pin_object
            || startup_terminal_file_stamp(&fs::symlink_metadata(&pin_path)?)? != pin_object
            || startup_terminal_file_stamp(&fs::symlink_metadata(&manifest_path)?)? != manifest_before
        {
            return Err(failure("terminal capture post-read provenance changed"));
        }
        Ok::<_, io::Error>(codex_startup_terminal_classifier(&read.text, read.truncated))
    }
    .await;
    match result {
        Ok(guard) => CodexStartupTerminalResult {
            guard,
            code: "none",
            io: "none",
            errno: 0,
        },
        Err(error) => {
            let io = OmpObserverFailure::io_kind(error.kind());
            let errno = error
                .raw_os_error()
                .filter(|value| (-4095..=4095).contains(value))
                .unwrap_or(0);
            CodexStartupTerminalResult {
                guard: "startup_terminal_read",
                code: "none",
                io,
                errno,
            }
        }
    }
}

async fn create(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    provider: Provider,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<(NativeProof, Receipt)> {
    let (session, receipt) =
        create_session(phone, config, socket, provider, server, evidence, run).await?;
    let p = proof(config, socket, &session, None).await?;
    Ok((p, receipt))
}

async fn create_feedback(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    provider: Provider,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<(NativeProof, Receipt, FeedbackPresentation)> {
    let (session, receipt, presentation) =
        create_session_presented(phone, config, socket, provider, server, evidence, run, true, None)
            .await?;
    let session = session.ok_or_else(|| non_succeeded_creation_error(provider, &receipt))?;
    let p = proof(config, socket, &session, None).await?;
    Ok((
        p,
        receipt,
        presentation.expect("feedback creation records a presentation observation"),
    ))
}
async fn create_omp_creation_diagnostic(
    phone: &mut Phone,
    config: &Config,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<Receipt> {
    let id = Uuid::random();
    let params = SessionCreateParams { source_target: None, checkout: None,
        boot_id: phone.hello.boot_id.clone(),
        server_id: server.clone(),
        project_id: config.projects[0].id.clone(),
        provider: Provider::Omp,
        name: Label::new(format!("owned-Omp-{}", &id.0[..8])).map_err(io::Error::other)?,
    };
    let initial = receipt_result(
        phone
            .request_at(
                id.clone(),
                RequestBody::SessionCreate(params),
                "create_initial",
            )
            .await?
            .map_err(|e| {
                failure(&format!(
                    "actual create blocked: method=SessionCreate; phase=create_initial; code={:?}",
                    e.code
                ))
            })?,
    )?;
    let receipt = if matches!(initial.state, ReceiptState::Reserved | ReceiptState::Dispatching) {
        phone.creation_settled(&id, Provider::Omp).await?
    } else {
        initial
    };
    if receipt.action_id != id
        || receipt.method != ActionMethod::SessionCreate
        || receipt.boot_id != phone.hello.boot_id
        || receipt
            .creation
            .as_ref()
            .is_some_and(|creation| creation.provider != Provider::Omp)
    {
        return Err(failure(
            "original OMP creation receipt identity/method/boot/provider mismatch",
        ));
    }
    write_json(
        &evidence.join(format!("omp-creation-diagnostic-{}.json", run.0)),
        &json!({"runId":run,"action":id,"receipt":receipt,
            "scope":"one original OMP creation receipt only; no same-ID lookup, native proof, input, history, or Android workflow",
            "resourcesRetained":true}),
    )?;
    Ok(receipt)
}
fn created_session_matches(
    resources: &CreationResources,
    s: &SessionSummary,
    provider: Provider,
) -> bool {
    s.provider == provider
        && resources.session_id.as_ref() == Some(&s.session_id)
        && resources.generation.as_ref() == Some(&s.generation)
        && resources.native_id.as_ref() == Some(&s.native_id)
        && resources.pane_id.as_ref() == Some(&s.pane_id)
        && resources.terminal_id.as_ref() == Some(&s.terminal_id)
        && resources.workspace_id.as_ref() == Some(&s.workspace_id)
}
async fn create_session(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    provider: Provider,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<(SessionSummary, Receipt)> {
    let (session, receipt, _) = create_session_presented(
        phone, config, socket, provider, server, evidence, run, false, None,
    )
    .await?;
    session
        .map(|session| (session, receipt.clone()))
        .ok_or_else(|| non_succeeded_creation_error(provider, &receipt))
}

async fn create_session_feedback(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    provider: Provider,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<(SessionSummary, Receipt, FeedbackPresentation)> {
    let (session, receipt, presentation) =
        create_session_presented(phone, config, socket, provider, server, evidence, run, true, None)
            .await?;
    let session = session.ok_or_else(|| non_succeeded_creation_error(provider, &receipt))?;
    Ok((
        session,
        receipt,
        presentation.expect("feedback creation records a presentation observation"),
    ))
}

fn non_succeeded_creation_error(provider: Provider, receipt: &Receipt) -> io::Error {
    let (stage, stage_state) =
        receipt
            .creation
            .as_ref()
            .map_or(("none".to_owned(), "none".to_owned()), |creation| {
                (
                    format!("{:?}", creation.stage),
                    format!("{:?}", creation.stage_state),
                )
            });
    let code = receipt
        .error
        .as_ref()
        .map_or("none".to_owned(), |error| format!("{:?}", error.code));
    failure(&format!(
        "actual creation did not succeed: method=SessionCreate; provider={provider:?}; state={:?}; stage={stage}; stageState={stage_state}; code={code}",
        receipt.state,
    ))
}

// Keep strict callers unchanged while sharing their original creation sequence.
#[allow(clippy::too_many_arguments)]
async fn create_session_presented(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    provider: Provider,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
    feedback_presentation: bool,
    mut startup_terminal_capture: Option<(&mut Companion, &CodexStartupTerminalContext<'_>, bool)>,
) -> io::Result<(Option<SessionSummary>, Receipt, Option<FeedbackPresentation>)> {
    let (before_source, before) = surface(socket).await?;
    before.no_focus()?;
    let anchor = before
        .focused_pane_id
        .clone()
        .ok_or_else(|| failure("focused native pane absent"))?;
    let before_geometry = geometry(socket, &anchor, GeometryPhase::CreateBefore)
        .await
        .map_err(GeometryFailure::into_io)?;
    let id = Uuid::random();
    let params = SessionCreateParams { source_target: None, checkout: None,
        boot_id: phone.hello.boot_id.clone(),
        server_id: server.clone(),
        project_id: config.projects[0].id.clone(),
        provider,
        name: Label::new(format!("owned-{provider:?}-{}", &id.0[..8])).map_err(io::Error::other)?,
    };
    // Earlier provider activity can invalidate coarse topology eligibility. Wait only
    // before this action's first submission; dispatch still owns all native guards.
    let admission_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut observed = None;
    let mut snapshot_timed_out = false;
    let available = loop {
        let inventory = match tokio::time::timeout_at(admission_deadline, phone.host()).await {
            Ok(result) => result?,
            Err(_) => {
                snapshot_timed_out = true;
                break false;
            }
        };
        observed = inventory
            .servers
            .iter()
            .find(|row| &row.server_id == server)
            .map(|row| {
                (
                    row.connectivity,
                    match provider {
                        Provider::Omp => row.can_create.omp,
                        Provider::Codex => row.can_create.codex,
                    },
                )
            });
        if tokio::time::Instant::now() >= admission_deadline {
            break false;
        }
        if observed.is_some_and(|(connectivity, can_create)| {
            connectivity == ServerConnectivity::Online && can_create
        }) {
            break true;
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(100)).min(admission_deadline),
        )
        .await;
    };
    write_json(
        &evidence.join(format!("creation-admission-{}-{provider:?}.json", run.0)),
        &json!({"phase":"before_first_create","provider":provider,
            "originalServerPresent":observed.is_some(),
            "connectivity":observed.map(|(connectivity, _)| connectivity),
            "providerCanCreate":observed.map(|(_, can_create)| can_create),
            "available":available,"snapshotTimedOut":snapshot_timed_out}),
    )?;
    if !available {
        return Err(prerequisite(
            "original owned server/provider creation prerequisites did not become available; no create submitted",
        ));
    }
    if let Some((companion, _, _)) = startup_terminal_capture.as_mut() {
        companion.pin_codex_startup_journal(&id);
    }
    let r = receipt_result(
        phone
            .request_at(
                id.clone(),
                RequestBody::SessionCreate(params.clone()),
                "create_initial",
            )
            .await?
            .map_err(|e| {
                failure(&format!(
                    "actual create blocked: method=SessionCreate; phase=create_initial; code={:?}",
                    e.code
                ))
            })?,
    )?;
    let receipt = if matches!(r.state, ReceiptState::Reserved | ReceiptState::Dispatching) {
        phone.creation_settled(&id, provider).await?
    } else {
        r
    };
    // Freeze the causal window before any fallible capture or filesystem work.
    let settlement_cutoff = startup_terminal_capture.as_ref().map(|_| pairing::now_ms());
    let selected_terminal_capture = startup_terminal_capture
        .as_ref()
        .is_some_and(|(_, _, enabled)| *enabled);
    if let Some((companion, context, terminal_enabled)) = startup_terminal_capture {
        if let Some(cutoff) = settlement_cutoff {
            companion.retain_codex_startup_journal(&receipt, context, before_source, cutoff);
        }
        if terminal_enabled {
            let result = capture_codex_startup_terminal(&config.runtime_dir, &receipt, context).await;
            companion.retain_codex_startup_terminal(result);
        }
    }
    if selected_terminal_capture && receipt.state != ReceiptState::Succeeded {
        return Ok((None, receipt, None));
    }
    let (_, after) = surface(socket).await?;
    let focus_changed = focus(&before) != focus(&after);
    // Preserve the original short-circuit: a focus change never triggers a
    // second geometry RPC, so its closed geometry observation is null.
    let geometry_changed = if focus_changed {
        None
    } else {
        Some(
            geometry(socket, &anchor, GeometryPhase::CreateAfter)
                .await
                .map_err(GeometryFailure::into_io)?
                != before_geometry,
        )
    };
    let presentation = creation_presentation(
        provider,
        feedback_presentation,
        focus_changed,
        geometry_changed,
    )?;
    if receipt.state != ReceiptState::Succeeded {
        return Ok((None, receipt, presentation));
    }
    let repeated = receipt_result(
        phone
            .request_at(id.clone(), RequestBody::SessionCreate(params), "create_receipt_lookup")
            .await?
            .map_err(|e| failure(&format!("matching original creation receipt lookup rejected: method=SessionCreate; phase=create_receipt_lookup; code={:?}", e.code)))?,
    )?;
    if repeated.action_id != receipt.action_id
        || surface(socket).await?.1.workspaces.len() != after.workspaces.len()
    {
        return Err(failure(
            "same-ID actual creation replay produced another workspace",
        ));
    }
    let session = phone
        .host()
        .await?
        .sessions
        .into_iter()
        .find(|s| {
            receipt
                .creation
                .as_ref()
                .is_some_and(|c| created_session_matches(&c.resources, s, provider))
        })
        .ok_or_else(|| failure("actual native Ready session not published"))?;
    Ok((Some(session), receipt, presentation))
}
async fn prompt_and_stop(
    phone: &mut Phone,
    p: &NativeProof,
    run: &Uuid,
    inputs: &NativeInputs<'_>,
) -> io::Result<Value> {
    let marker = format!("goatr-mobile-{}-{:?}", &run.0[..8], p.provider);
    let text = format!(
        "In this exact existing session, run the native shell tool to execute printf '%s\\n' '{marker}'. Do not write files or start another process/session. Then repeat {marker} in your reply."
    );
    let id = Uuid::random();
    let receipt = receipt_result(
        phone
            .request(
                id.clone(),
                RequestBody::PromptSubmit(PromptSubmitParams {
                    boot_id: phone.hello.boot_id.clone(),
                    target: p.target.clone(),
                    text: PromptText(text),
                }),
            )
            .await?
            .map_err(|e| failure(&format!("actual native prompt rejected: {:?}", e.code)))?,
    )?;
    if !matches!(
        receipt.state,
        ReceiptState::Dispatched | ReceiptState::Succeeded
    ) {
        return Err(failure("actual native prompt did not dispatch"));
    }
    let history = wait_history(phone, &p.target, &marker, true).await?;
    let desktop = format!("goatr-desktop-{}-{:?}", &run.0[..8], p.provider);
    inputs.marker(phone, p, &desktop).await?;
    wait_history(phone, &p.target, &desktop, false).await?;
    let stop_prompt = Uuid::random();
    phone
        .request(
            stop_prompt.clone(),
            RequestBody::PromptSubmit(PromptSubmitParams {
                boot_id: phone.hello.boot_id.clone(),
                target: p.target.clone(),
                text: PromptText(
                    "Run the native shell tool sleep 30, then report done; no file changes.".into(),
                ),
            }),
        )
        .await?
        .map_err(|_| failure("actual native current-work prompt rejected"))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let state = loop {
        let s = phone.open(&p.target, "stop_working").await?;
        if s.state.summary.status == SessionStatus::Working
            && s.state.summary.connectivity == SessionConnectivity::Ready
            && (p.provider == Provider::Omp || s.state.native_turn_id.is_some())
        {
            break s.state;
        }
        if Instant::now() > deadline {
            return Err(prerequisite(
                "actual stock current-work/native-turn never appeared for Stop",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let cancel = Uuid::random();
    let initial_stop = receipt_result(
        phone
            .request(
                cancel.clone(),
                RequestBody::TurnCancel(TurnCancelParams {
                    boot_id: phone.hello.boot_id.clone(),
                    target: p.target.clone(),
                    native_turn_id: state.native_turn_id.clone(),
                }),
            )
            .await?
            .map_err(|error| failure(&format!(
                "actual observed Stop rejected: {:?}; observed connectivity={:?}; observed working={}",
                error.code, state.summary.connectivity,
                state.summary.status == SessionStatus::Working,
            )))?,
    )?;
    if initial_stop.action_id != cancel
        || initial_stop.method != ActionMethod::TurnCancel
        || initial_stop.target.as_ref() != Some(&p.target)
    {
        return Err(failure(
            "actual observed Stop initial receipt did not match the original action/Target",
        ));
    }
    let stop = if matches!(
        initial_stop.state,
        ReceiptState::Reserved | ReceiptState::Dispatching
    ) {
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            phone.settled(&cancel),
        )
        .await
        .map_err(|_| {
            prerequisite(
                "actual original Stop receipt did not settle within original scenario budget",
            )
        })??
    } else {
        initial_stop
    };
    let accepted = match p.provider {
        Provider::Omp => ReceiptState::Dispatched,
        Provider::Codex => ReceiptState::Succeeded,
    };
    if stop.action_id != cancel
        || stop.method != ActionMethod::TurnCancel
        || stop.target.as_ref() != Some(&p.target)
        || stop.state != accepted
        || stop.error.is_some()
    {
        return Err(failure(&format!(
            "actual observed Stop receipt rejected: state={:?}; code={:?}",
            stop.state,
            stop.error.as_ref().map(|error| error.code),
        )));
    }
    loop {
        if Instant::now() >= deadline {
            return Err(prerequisite(
                "actual same-Target Ready/Idle not observed after Stop within original scenario budget",
            ));
        }
        let stopped = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline), phone.open(&p.target, "stop_idle"),
        ).await.map_err(|_| prerequisite(
            "actual same-Target Ready/Idle not observed after Stop within original scenario budget",
        ))??;
        if Instant::now() >= deadline {
            return Err(prerequisite(
                "actual same-Target Ready/Idle not observed after Stop within original scenario budget",
            ));
        }
        if stopped.state.summary.connectivity == SessionConnectivity::Ready
            && stopped.state.summary.status == SessionStatus::Idle
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    p.recheck_processes()?;
    Ok(
        json!({"prompt":id,"receipt":receipt,"mobileMarker":marker,"desktopMarker":desktop,"historyIds":history.history.iter().map(|r|&r.id).collect::<Vec<_>>(),"toolObserved":history.history.iter().any(|r|r.tool.is_some()),"stopPrompt":stop_prompt,"stopAction":cancel,"stopReceipt":stop,"stopIdleObserved":true,"nativeTurn":state.native_turn_id,"scope":"real native operations, payloads not retained; no OMP acknowledgment/winner fabricated"}),
    )
}
async fn checkpoint(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    if !args.get_flag("consent-owned-providers") {
        return Err(failure(
            "explicit --consent-owned-providers required before owned desktop/provider effects",
        ));
    }
    let selected = args
        .get_many::<String>("backend")
        .ok_or_else(|| {
            failure("select --backend omp --backend codex; missing backend is not silently skipped")
        })?
        .map(|s| {
            if s == "omp" {
                Provider::Omp
            } else {
                Provider::Codex
            }
        })
        .collect::<Vec<_>>();
    if args.get_flag("paging-proof") && selected != vec![Provider::Codex] {
        return Err(failure("paging proof requires only one Codex backend"));
    }
    if args.get_flag("codex-invalidation-diagnostic") && selected != vec![Provider::Codex] {
        return Err(failure(
            "Codex invalidation diagnostic requires only one Codex backend",
        ));
    }
    let omp_creation_diagnostic = args.get_flag("omp-creation-diagnostic");
    if omp_creation_diagnostic && selected != vec![Provider::Omp] {
        return Err(failure(
            "OMP creation diagnostic requires only one OMP backend",
        ));
    }
    if omp_creation_diagnostic && selected_omp_capture_mode() != OwnedOmpCaptureMode::Backend {
        return Err(failure(
            "OMP creation diagnostic requires only GOATR_OWNED_OMP_BACKEND_CAPTURE=1",
        ));
    }
    let mut unique = HashSet::new();
    if selected.iter().any(|p| !unique.insert(*p)) {
        return Err(failure("duplicate selected backend"));
    }
    require_both_checkpoint_providers(args, &selected)?;
    let deployment=Config::load(args.get_one::<PathBuf>("config").ok_or_else(||failure("actual deployment --config with preserved installed outer wrappers/source roots required"))?)?;
    for p in &selected {
        if let Some(reason) = deployment.backend_reason(*p)? {
            return Err(prerequisite(reason));
        }
    }
    if !deployment.allow_scoped_shell_probe {
        return Err(failure(
            "explicit local scoped probe consent must be saved by actual setup before stock creation gate",
        ));
    }
    let binary = packaged(args)?;
    let workspace = fs::canonicalize(
        args.get_one::<PathBuf>("workspace-root")
            .cloned()
            .unwrap_or(std::env::current_dir()?),
    )?;
    goatr::config::project_directory(&workspace)?;
    let runtime = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| failure("unchanged real user XDG_RUNTIME_DIR required"))?,
    );
    private_dir(&runtime, false)?;
    let run = Uuid::random();
    write_json(
        &evidence.join(format!("native-input-consent-{}.json", run.0)),
        &json!({"runId":run,"automateNativeInputs":args.get_flag("automate-native-inputs"),
            "exhaustivePaging":exhaustive_paging(args),
            "diagnosticOnly":selected.len()==1,"finalBothProviderAndroidAcceptanceRequired":true,
            "scope":"only this fresh run's explicitly owned native TUIs; no existing work adoption",
            "acknowledgmentIsNotEffect":true,"uncertainInputsNeverReplayed":true}),
    )?;
    let suffix = hex(&random32()?[..6]);
    let root = workspace.join(format!(".gs-{suffix}"));
    let admin = runtime.join(format!("gs-{suffix}"));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    fs::DirBuilder::new().mode(0o700).create(&admin)?;
    for p in [
        root.join("state"),
        root.join("runtime"),
        root.join("project"),
        root.join("runtime/b"),
        admin.join("config"),
    ] {
        fs::DirBuilder::new().mode(0o700).create(&p)?;
    }
    let socket = admin.join("admin.sock");
    let name = format!("gs-{suffix}");
    let herdr_socket = admin.join(format!("config/herdr/sessions/{name}/herdr.sock"));
    goatr::config::socket_path(&herdr_socket)?;
    for p in [
        admin.join("config/herdr"),
        admin.join("config/herdr/sessions"),
        admin.join(format!("config/herdr/sessions/{name}")),
    ] {
        fs::DirBuilder::new().mode(0o700).create(p)?;
    }
    let omp_capture = owned_omp_capture_requested();
    let omp_receiver_capture = owned_omp_receiver_capture_requested();
    let omp_backend_capture = owned_omp_backend_capture_requested();
    if owned_omp_capture_conflicts() {
        return Err(failure(
            "owned OMP creation, post-Ready offline, and backend-only captures are mutually exclusive",
        ));
    }
    let codex_capture = args.get_flag("codex-invalidation-diagnostic")
        || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let codex_startup_terminal_capture = args.get_flag("codex-startup-terminal-capture");
    if omp_capture && codex_capture {
        return Err(failure(
            "owned OMP and Codex diagnostic captures are mutually exclusive",
        ));
    }
    if omp_capture && !selected.contains(&Provider::Omp) {
        return Err(failure(
            "owned OMP report capture requires the selected OMP provider",
        ));
    }
    let private_herdr = owned_herdr_report_binary(
        if omp_receiver_capture {
            Some(OwnedHerdrReportMode::OmpReceiver)
        } else if codex_startup_terminal_capture {
            Some(OwnedHerdrReportMode::CodexStartupTerminal)
        } else {
            None
        },
        &workspace,
    )?;
    let mut config = deployment.clone();
    config.runtime_dir = root.join("runtime");
    config.runtime_dir_agent = config.runtime_dir.clone();
    config.herdr_sockets = vec![herdr_socket.clone()];
    if let Some(private_herdr) = private_herdr {
        // Every owned-run proof below observes this exact private diagnostic binary.
        config.executables.herdr = private_herdr;
    }
    if args.get_flag("paging-proof") {
        config.omp_registry_dir = None;
        config.omp_blob_root = None;
        config.omp_session_roots.clear();
        config.executables.omp = None;
    }
    config.projects = vec![ConfiguredProject {
        id: Uuid::random(),
        label: Label::new("Owned checkpoint project").map_err(io::Error::other)?,
        path: root.join("project"),
    }];
    if selected.contains(&Provider::Codex) {
        let owner = CodexOwner {
            id: Uuid::random(),
            socket_host: root.join("runtime/c.sock"),
            socket_agent: root.join("runtime/c.sock"),
            bindings_dir: root.join("runtime/b"),
            unit: format!("goatr-stock-{}.service", &run.0[..8]),
        };
        config.default_codex_owner_id = Some(owner.id.clone());
        config.codex_owners = vec![owner];
    } else {
        config.codex_owners.clear();
        config.default_codex_owner_id = None;
    }
    let native_mcp = if !args.get_flag("paging-proof")
        && args.get_flag("automate-native-inputs")
        && selected.contains(&Provider::Codex)
        && !args.get_flag("codex-invalidation-diagnostic")
    {
        Some(NativeMcp::prepare(&root, &run, &mut config, evidence)?)
    } else {
        None
    };
    let config_path = root.join("config.toml");
    let config_lock = goatr::setup::SetupLock::acquire(&root)?;
    config_lock.publish(
        &config_path,
        &goatr::setup::ExpectedFile::Absent,
        config.serialized()?.as_bytes(),
    )?;
    drop(config_lock);
    let listener = if args.get_flag("paging-proof") {
        None
    } else {
        Some(std::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        ))?)
    };
    let (port, listener_record, listener_source) = if let Some(listener) = &listener {
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let metadata = fs::metadata(format!("/proc/self/fd/{}", listener.as_raw_fd()))?;
        let record = goatr::backends::omp::OwnedRelayListener {
            v: 1,
            fd: listener.as_raw_fd(),
            device: metadata.dev(),
            inode: metadata.ino(),
            port,
            parent: herdr::process_evidence(std::process::id())?,
            runtime_dir: config.runtime_dir.clone(),
        };
        let path = config.runtime_dir.join("omp-relay.owner");
        let lock = goatr::setup::SetupLock::acquire(&config.runtime_dir)?;
        lock.publish(
            &path,
            &goatr::setup::ExpectedFile::Absent,
            &serde_json::to_vec(&record).map_err(io::Error::other)?,
        )?;
        drop(lock);
        (Some(port), Some(record), Some(identity(&path)?))
    } else {
        (None, None, None)
    };
    if selected_omp_capture_mode().requires_owned_relay() && listener.is_none() {
        return Err(failure(
            "owned OMP report capture requires validated owned relay",
        ));
    }
    let mut herdr_report_capture = if omp_receiver_capture {
        Some(herdr_report_capture::Capture::bind(
            &root,
            uid(),
            hex(&random32()?),
            owned_omp_offline_capture_requested(),
        )?)
    } else {
        None
    };
    let _overlay = if let (Some(port), Some(capture)) = (port, herdr_report_capture.as_mut()) {
        if capture.prepare_omp_producer(&config, port)? {
            config.runtime_dir_agent.join("omp-collab-producer.yml")
        } else {
            goatr::backends::omp::overlay(&config, port)?
        }
    } else if let Some(port) = port {
        goatr::backends::omp::overlay(&config, port)?
    } else {
        config.runtime_dir_agent.join("omp-collab.yml")
    };
    // Capture before acceptance so continuation cannot adopt a replaced run root or package.
    let feedback_root = identity(&root)?;
    let feedback_package = binary_sha256(&binary)?;
    write_json(
        &evidence.join(format!("resources-{}.json", run.0)),
        &json!({"runId":run,"herdrReportCapture":herdr_report_capture.is_some(),"ompBackendCapture":omp_backend_capture,"root":identity(&root)?,"adminParent":identity(&admin)?,"adminSocket":socket,"herdrSocket":herdr_socket,"herdrName":name,"config":identity(&config_path)?,"runtimeUnchanged":runtime,"package":binary,"packageSha256":binary_sha256(&binary)?,"companionLockSha256":transport::lock_sha256(),"OMPownedPort":port,"listener":listener_record,"listenerSource":listener_source,"nativeCreationResources":"retained; never automatically rollback/kill desktop work","stage":"owned roots/exact kernel listener source before process effects"}),
    )?;
    let mut command = std::process::Command::new(&config.executables.herdr);
    command
        .args(["--session", &name, "server"])
        .current_dir(&config.projects[0].path)
        .env("HERDR_STARTUP_CWD", &config.projects[0].path)
        .env("XDG_CONFIG_HOME", admin.join("config"))
        .env("XDG_STATE_HOME", root.join("state"));
    for key in [
        "GOATR_OWNED_OMP_PRODUCER_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256",
        "GOATR_OWNED_OMP_PRODUCER_BUN",
        "GOATR_OWNED_OMP_PRODUCER_BUN_SHA256",
    ] {
        command.env_remove(key);
    }
    if let Some(capture) = &herdr_report_capture {
        capture.configure(&mut command);
    }
    if let Some(mcp) = &native_mcp {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(mcp.directory.clone()).chain(std::env::split_paths(&inherited)),
        )
        .map_err(io::Error::other)?;
        command.env("PATH", path);
    }
    let mut desktop = Desktop::spawn(command)?;
    if let Some(capture) = &herdr_report_capture {
        capture.pin(&desktop.process);
    }
    write_json(
        &evidence.join(format!("herdr-{}.json", run.0)),
        &json!({"runId":run,"process":desktop.process,"executable":config.executables.herdr,"outerSha256":binary_sha256(&config.executables.herdr)?,"socket":herdr_socket,"cwd":config.projects[0].path,"XDG_CONFIG_HOME":admin.join("config"),"XDG_RUNTIME_DIR":runtime}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if surface(&herdr_socket).await.is_ok() {
            break;
        }
        if Instant::now() > deadline {
            return Err(prerequisite(
                "owned actual Herdr source/socket/PID namespace prerequisites unavailable; recorded desktop retained",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut serve = start_serve_with_codex_capture(
        &binary,
        &config_path,
        &root,
        &socket,
        listener.as_ref(),
        args.get_flag("codex-invalidation-diagnostic"),
        codex_startup_terminal_capture,
        herdr_report_capture.as_ref(),
    )?;
    healthy(&mut serve, &socket).await?;
    write_json(
        &evidence.join(format!("serve-{}.json", run.0)),
        &json!({"runId":run,"process":serve.process,"adminSocket":identity(&socket)?,"absoluteConfig":config_path,"OMPownedPort":port,"serveArgument":"--admin-socket identical on pair/list/revoke/restart","scope":"ordinary configured packaged serve, not hidden transport smoke"}),
    )?;
    println!(
        "{}",
        json!({"gate":"owned-runtime-ready","runId":run,"root":root,"herdrSocket":herdr_socket,"OMPownedPort":port,"scope":"sanitized resource rendezvous; relay is live, no provider readiness claimed"}),
    );
    let endpoint = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?))
        .bind()
        .await
        .map_err(io::Error::other)?;
    let mut phone = pair(&binary, &root, &socket, &endpoint).await?;
    phone.codex_close_capture = args.get_flag("codex-invalidation-diagnostic");
    if let Some(budget) = serve.omp_capture_budget.clone() {
        phone.set_omp_capture(budget);
    }
    let host_id = phone.hello.host_id.clone();
    let admission_deadline = Instant::now() + Duration::from_secs(30);
    let server = loop {
        let inventory = phone.host().await?;
        if let Some(server) = inventory
            .servers
            .iter()
            .find(|server| server.name.0 == name)
        {
            let available = server.connectivity == ServerConnectivity::Online
                && selected.iter().all(|provider| match provider {
                    Provider::Omp => server.can_create.omp,
                    Provider::Codex => server.can_create.codex,
                });
            if available || Instant::now() >= admission_deadline {
                write_json(
                    &evidence.join(format!("creation-admission-{}.json", run.0)),
                    &json!({"runId":run,"serverId":server.server_id,"connectivity":server.connectivity,"canCreate":server.can_create,"reason":server.reason,"selected":selected,"available":available,"scope":"read-only current inventory before one native creation; no mutation retry"}),
                )?;
                if available {
                    break server.server_id.clone();
                }
            }
        }
        if Instant::now() >= admission_deadline {
            return Err(prerequisite(
                "owned named Herdr creation prerequisites did not become available; no create submitted",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let mut native = Vec::new();
    let mut gates = Vec::new();
    let mut history_witnesses = Vec::new();
    for p in selected.iter().copied() {
        if omp_creation_diagnostic {
            let receipt = create_omp_creation_diagnostic(
                &mut phone,
                &config,
                &server,
                evidence,
                &run,
            )
            .await?;
            let (stage, stage_state) = receipt
                .creation
                .as_ref()
                .map_or(("none".to_owned(), "none".to_owned()), |creation| {
                    (
                        format!("{:?}", creation.stage),
                        format!("{:?}", creation.stage_state),
                    )
                });
            let error_code = receipt
                .error
                .as_ref()
                .map(|error| format!("{:?}", error.code));
            phone
                .connection
                .close(0u32.into(), b"OMP creation diagnostic complete");
            serve.stop().await?;
            println!(
                "{}",
                json!({"gate":"omp-creation-diagnostic","runId":run,
                    "action":receipt.action_id,"state":format!("{:?}", receipt.state),
                    "stage":stage,"stageState":stage_state,"errorCode":error_code,
                    "status":"closed"}),
            );
            return Ok(true);
        }
        if p == Provider::Codex && args.get_flag("codex-invalidation-diagnostic") {
            // Preserve the original helper result while the selected capture reaches EOF.
            let result: io::Result<()> = async {
        let startup_terminal = CodexStartupTerminalContext {
            config: &config,
            socket: &herdr_socket,
            server: &server,
            provider: p,
            desktop: &desktop.process,
        };
        let (session, receipt, _) = create_session_presented(
            &mut phone,
            &config,
            &herdr_socket,
            p,
            &server,
            evidence,
            &run,
            false,
            Some((&mut serve, &startup_terminal, codex_startup_terminal_capture)),
        )
        .await?;
        let session = session.ok_or_else(|| non_succeeded_creation_error(p, &receipt))?;
        let proof = proof(&config, &herdr_socket, &session, None).await?;
                write_json(
                    &evidence.join(format!("native-{}-{p:?}.json", run.0)),
                    &json!({"runId":run,"proof":proof.evidence(),"creationReceipt":receipt,"noFocus":true}),
                )?;
                let inputs = NativeInputs {
                    enabled: args.get_flag("automate-native-inputs"),
                    paging_proof: args.get_flag("paging-proof"),
                    config: &config,
                    socket: &herdr_socket,
                    herdr: &desktop.process,
                    evidence,
                    run: &run,
                    mcp: native_mcp.as_ref(),
                    plan: RefCell::new(None),
                    mcp_dialogs: RefCell::new(HashMap::new()),
                };
            // Exercise the existing original binding guard before any native input,
            // model request, history generation, or Android workflow.
            let _ = inputs.current(&mut phone, &proof).await?;
            let mut witness = paging_proof::Witness::connect(&proof, &inputs).await?;
            witness.admit(&proof, false).await?;
            let initial = phone.start_generation(&proof.target).await?;
            if initial.state.summary.native_id != proof.native
                || initial.state.summary.connectivity != SessionConnectivity::Ready
                || initial.state.summary.status != SessionStatus::Idle
                || !initial.pending_requests.is_empty()
            {
                return Err(generation_idle_failure(0, false, false, false));
            }
            let marker = format!("goatr-history-{}-Codex-0", &inputs.run.0[..8]);
            let command = format!("printf '%s\\n' '{marker}'");
            let command_started = Instant::now();
            let command_deadline = history_command_deadline(
                inputs.paging_proof,
                Provider::Codex,
                command_started + Duration::from_secs(30),
                command_started,
            );
            phone.history_command = Some(HistoryCommandTrace::default());
            let completion = tokio::time::timeout_at(command_deadline.into(), async {
                inputs
                    .send(
                        &mut phone,
                        &proof,
                        NativeInput::Prompt(&format!("!{command}")),
                        None,
                        "native_history_command",
                    )
                    .await?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                let ((snapshot, id), (native, turn)) = tokio::try_join!(
                    async {
                        tokio::time::timeout_at(
                            deadline,
                            inputs.shell_completed(&mut phone, &proof, &command, &marker),
                        )
                        .await
                        .map_err(|_| failure("Goatr completion deadline; no replay"))?
                    },
                    witness.completed(&proof, &command, &marker, deadline)
                )?;
                if id != native
                    || snapshot
                        .history
                        .iter()
                        .find(|record| record.id == id)
                        .and_then(|record| record.turn_id.as_ref())
                        != Some(&turn)
                {
                    return Err(failure(
                        "independent native and adapter completion IDs differ",
                    ));
                }
                Ok::<_, io::Error>((snapshot, id, turn))
            })
            .await;
            let (snapshot, id, turn) = match completion {
                Ok(result) => result?,
                Err(_) => {
                    return Err(phone
                        .history_command
                        .as_ref()
                        .expect("command trace")
                        .deadline());
                }
            };
            phone.history_command = None;
            if target(&snapshot.state.summary) != proof.target
                || snapshot.state.summary.native_id != proof.native
                || snapshot.state.summary.connectivity != SessionConnectivity::Ready
                || snapshot.state.summary.status != SessionStatus::Idle
                || !snapshot.pending_requests.is_empty()
                || snapshot
                    .history
                    .iter()
                    .find(|record| record.id == id)
                    .and_then(|record| record.turn_id.as_ref())
                    != Some(&turn)
            {
                return Err(failure(
                    "native diagnostic command changed original identity or completion boundary",
                ));
            }
            drop(witness);
            proof.recheck_processes()?;
            desktop.recheck()?;
            serve.recheck()?;
            Ok(())
            }
            .await;
            phone.connection.close(
                0u32.into(),
                b"Codex invalidation diagnostic complete",
            );
            let cleanup = serve
                .stop_codex_diagnostic(if result.is_err() {
                    "after_helper_failure"
                } else {
                    "after_helper_success"
                })
                .await;
            return match result {
                Ok(()) => {
                    cleanup?;
                    write_json(
                        &evidence.join(format!("codex-invalidation-diagnostic-{}.json", run.0)),
                        &json!({
                            "runId":run,
                            "gate":"codex-invalidation-diagnostic",
                            "status":"pass",
                            "provider":"codex",
                            "diagnosticOnly":true,
                            "scope":"one original Codex create/proof/current guard and one witnessed native history command",
                            "strictPaging":false,
                            "paging":false,
                            "strictBoundary":false,
                            "android":false,
                            "nativeInput":true,
                            "nativeCommands":1,
                            "independentCompletion":true,
                            "originalIdentityRetained":true,
                            "modelCalls":false,
                            "foregroundAcceptance":false,
                            "resourcesRetained":true,
                        }),
                    )?;
                    Ok(true)
                }
                Err(error) => {
                    let _ = cleanup;
                    Err(error)
                }
            };
        }
        let (proof, receipt) = create(
            &mut phone,
            &config,
            &herdr_socket,
            p,
            &server,
            evidence,
            &run,
        )
        .await?;
        if p == Provider::Codex
            && let Some(mcp) = &native_mcp
        {
            mcp.bind_owner(&proof)?;
        }
        write_json(
            &evidence.join(format!("native-{}-{p:?}.json", run.0)),
            &json!({"runId":run,"proof":proof.evidence(),"creationReceipt":receipt,"noFocus":true}),
        )?;
        let inputs = NativeInputs {
            enabled: args.get_flag("automate-native-inputs"),
            paging_proof: args.get_flag("paging-proof"),
            config: &config,
            socket: &herdr_socket,
            herdr: &desktop.process,
            evidence,
            run: &run,
            mcp: native_mcp.as_ref(),
            plan: RefCell::new(None),
            mcp_dialogs: RefCell::new(HashMap::new()),
        };
        if inputs.paging_proof {
            let result = paging_proof::run(
                &mut phone,
                &proof,
                &inputs,
                NativeHistoryMode::BoundedDiagnostic128,
            )
            .await?;
            write_json(
                &evidence.join(format!("native-history-{}-Codex.json", run.0)),
                &result,
            )?;
            write_json(
                &evidence.join(format!("paging-proof-{}.json", run.0)),
                &json!({"runId":run,"gate":"bounded-diagnostic-128","status":"pass","proofMode":NativeHistoryMode::BoundedDiagnostic128.name(),"strictRetentionBoundary":false,"foregroundAcceptance":false,"nativeHistory":result,"resourcesRetained":true}),
            )?;
            phone
                .connection
                .close(0u32.into(), b"independent proof complete");
            serve.stop().await?;
            return Ok(true);
        }
        gates.push(
            prompt_and_stop(&mut phone, &proof, &run, &inputs)
                .await
                .map_err(|error| super::prompt_stop_failure(proof.provider, error))?,
        );
        if args.get_flag("defer-retention-crossing") {
            let deferred = json!({"scenario":"actual-native-history-continuations","target":proof.target,
                "provider":proof.provider,"exhaustivePaging":"deferred_by_user","olderPagingProven":false});
            write_json(
                &evidence.join(format!("native-history-{}-{p:?}.json", run.0)),
                &deferred,
            )?;
            gates.push(deferred);
        } else {
            let history_gate = native_history_pages(&mut phone, &proof, &inputs).await?;
            let older: Vec<Id> =
                serde_json::from_value(history_gate["nativeOlderReturnedIds"].clone())
                    .map_err(io::Error::other)?;
            let retained: HashSet<Id> =
                serde_json::from_value(history_gate["retainedIdUpperBound"].clone())
                    .map_err(io::Error::other)?;
            let qualified: Id = serde_json::from_value(history_gate["qualifiedNativeId"].clone())
                .map_err(io::Error::other)?;
            let qualified_later_completed_lower_bound =
                history_gate["qualifiedLaterCompletedLowerBound"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| failure("strict native boundary witness lower bound absent"))?;
            if older.is_empty()
                || older.len() > 16
                || retained.len() > goatr::state::HISTORY_RECORDS
                || qualified_later_completed_lower_bound < 1000
                || !older.contains(&qualified)
                || history_gate["strictRetentionBoundary"].as_bool() != Some(true)
            {
                return Err(failure("actual older-history witness boundary invalid"));
            }
            write_json(
                &evidence.join(format!("native-history-{}-{p:?}.json", run.0)),
                &history_gate,
            )?;
            history_witnesses.push(NativeHistoryWitness {
                original: proof.target.clone(),
                provider: proof.provider,
                native: proof.native.clone(),
                owner: proof.owner.clone(),
                older,
                retained,
                qualified,
                qualified_later_completed_lower_bound,
            });
            gates.push(history_gate);
        }
        if !args.get_flag("ui") {
            let kinds = if p == Provider::Omp {
                vec![PresentationKind::OmpSelect, PresentationKind::OmpEditor]
            } else {
                vec![
                    PresentationKind::CodexApproval,
                    PresentationKind::CodexQuestions,
                    PresentationKind::CodexForm,
                    PresentationKind::CodexUrl,
                ]
            };
            for kind in kinds {
                gates.push(dialog(&mut phone, &proof, kind, &inputs).await?);
                gates.push(desktop_dialog(&mut phone, &proof, kind, &inputs).await?);
            }
        }
        native.push(proof);
    }
    let before_boot = phone.hello.boot_id.clone();
    let before_native = native.iter().map(NativeProof::evidence).collect::<Vec<_>>();
    let mut replay = Vec::new();
    for p in &native {
        let id = Uuid::random();
        let req = RequestBody::PromptSubmit(PromptSubmitParams {
            boot_id: phone.hello.boot_id.clone(),
            target: p.target.clone(),
            text: PromptText(format!(
                "Reply only goatr-once-{}-{:?}",
                &run.0[..8],
                p.provider
            )),
        });
        let receipt = receipt_result(
            phone
                .request(id.clone(), req.clone())
                .await?
                .map_err(|_| failure("original once-only prompt rejected"))?,
        )?;
        replay.push((id, req, receipt));
    }
    let approved_projects = config.projects.clone();
    config.projects.clear();
    let expected_config = goatr::setup::ExpectedFile::capture(&config_path)?;
    let config_lock = goatr::setup::SetupLock::acquire(&root)?;
    config_lock.publish(
        &config_path,
        &expected_config,
        config.serialized()?.as_bytes(),
    )?;
    drop(config_lock);
    phone
        .connection
        .close(0u32.into(), b"owned connection gap; no write replay");
    for p in &native {
        p.recheck_processes()?;
    }
    phone = connect(&endpoint, &host_id, None).await?;
    serve.stop().await?;
    desktop.recheck()?;
    for p in &native {
        p.recheck_processes()?;
    }
    serve = start_serve(&binary, &config_path, &root, &socket, listener.as_ref())?;
    healthy(&mut serve, &socket).await?;
    phone.connection.close(0u32.into(), b"companion restart");
    phone = connect(&endpoint, &host_id, None).await?;
    if phone.hello.boot_id == before_boot {
        return Err(failure("companion boot did not replace mutation contexts"));
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let host = phone.host().await?;
        if native.iter().all(|p| {
            host.sessions.iter().any(|s| {
                s.session_id == p.target.session_id
                    && s.native_id == p.native
                    && s.generation != p.target.generation
                    && s.connectivity == SessionConnectivity::Ready
            })
        }) {
            break;
        }
        if Instant::now() > deadline {
            let observation = json!({"phase":"restart_reconciliation","providers":native.iter().take(2)
                .map(|p|restart_observation(p.provider,&p.target,&p.native,&host.sessions)).collect::<Vec<_>>()});
            // Preserve the original failure even when exclusive evidence storage fails.
            if write_json(
                &evidence.join(format!("restart-reconciliation-{}.json", run.0)),
                &observation,
            )
            .is_err()
            {
                eprintln!("restart reconciliation observation evidence unavailable");
            }
            return Err(failure(
                "same native session restart reconciliation unavailable; no owner/relay/TUI/thread fallback",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for (id, req, receipt) in &replay {
        let known = phone.receipt(id).await?;
        let looked = receipt_result(
            phone
                .request(id.clone(), req.clone())
                .await?
                .map_err(|_| failure("old-boot matching receipt lookup failed"))?,
        )?;
        if looked.action_id != receipt.action_id
            || looked.created_at != receipt.created_at
            || known.method != receipt.method
        {
            return Err(failure("matching old-boot action was replayed or lost"));
        }
    }
    for p in &mut native {
        let now = phone
            .host()
            .await?
            .sessions
            .into_iter()
            .find(|s| s.session_id == p.target.session_id)
            .ok_or_else(|| failure("preserved native session absent after restart"))?;
        let current = proof(&config, &herdr_socket, &now, None).await?;
        if p.owner != current.owner
            || p.tui != current.tui
            || p.relay != current.relay
            || p.native != current.native
        {
            return Err(failure(
                "companion restart replaced native owner/relay/TUI/conversation",
            ));
        }
        *p = current;
    }
    if !phone.host().await?.projects.is_empty() {
        return Err(failure(
            "owned config project cutover was not actually loaded",
        ));
    }
    for p in &native {
        let state = phone
            .open(&p.target, "existing_session_after_restart")
            .await?;
        if state.state.summary.native_id != p.native || !state.state.summary.capabilities.prompt {
            return Err(failure(
                "existing same-user native session was filtered/disabled by creation projects",
            ));
        }
    }
    gates.push(json!({"scenario":"actual-existing-native-inventory-control-outside-creation-project-list","targets":native.iter().map(|p|&p.target).collect::<Vec<_>>(),"projects":[]}));
    serve.stop().await?;
    config.projects = approved_projects;
    let expected_config = goatr::setup::ExpectedFile::capture(&config_path)?;
    let config_lock = goatr::setup::SetupLock::acquire(&root)?;
    config_lock.publish(
        &config_path,
        &expected_config,
        config.serialized()?.as_bytes(),
    )?;
    drop(config_lock);
    serve = start_serve(&binary, &config_path, &root, &socket, listener.as_ref())?;
    healthy(&mut serve, &socket).await?;
    phone
        .connection
        .close(0u32.into(), b"restore owned creation approvals");
    phone = connect(&endpoint, &host_id, None).await?;
    refresh_native(&mut phone, &config, &herdr_socket, &mut native).await?;
    if !args.get_flag("ui") {
        for index in 0..native.len() {
            for p in &native {
                p.recheck_processes()?;
            }
            let provider = native[index].provider;
            let (id, body, before) = partial_creation(
                &mut phone,
                &config,
                &herdr_socket,
                provider,
                &server,
                &mut serve,
            )
            .await?;
            write_json(
                &evidence.join(format!("partial-before-{}-{provider:?}.json", run.0)),
                &json!({"runId":run,"observed":before,"exactOwnedCompanionKilled":serve.process,"desktopRetained":true}),
            )?;
            desktop.recheck()?;
            for p in &native {
                p.recheck_processes()?;
            }
            serve = start_serve(&binary, &config_path, &root, &socket, listener.as_ref())?;
            healthy(&mut serve, &socket).await?;
            phone
                .connection
                .close(0u32.into(), b"partial response loss");
            phone = connect(&endpoint, &host_id, None).await?;
            let receipt = phone.receipt(&id).await?;
            if receipt.state != ReceiptState::Uncertain
                || receipt
                    .creation
                    .as_ref()
                    .is_none_or(|c| c.resources.workspace_id.is_none())
            {
                return Err(failure(
                    "actual creation interruption did not retain partial/uncertain resources",
                ));
            }
            let workspaces = surface(&herdr_socket).await?.1.workspaces.len();
            let matching = receipt_result(
                phone
                    .request(id.clone(), body)
                    .await?
                    .map_err(|_| failure("partial original receipt lookup rejected"))?,
            )?;
            if matching != receipt || surface(&herdr_socket).await?.1.workspaces.len() != workspaces
            {
                return Err(failure(
                    "partial original lookup replayed/deleted desktop work",
                ));
            }
            gates.push(json!({"scenario":"actual-partial-create-response-loss-restart-no-replay","provider":provider,"before":before,"after":receipt,"workspaceCountUnchanged":true}));
            refresh_native(&mut phone, &config, &herdr_socket, &mut native).await?;
        }
    }
    if args.get_flag("ui") {
        ui::run(
            args,
            evidence,
            ui::Context {
                binary: &binary,
                root: &root,
                socket: &socket,
                config: &config,
                herdr_socket: &herdr_socket,
                herdr_process: &desktop.process,
                mcp: native_mcp.as_ref(),
                serve: &mut serve,
                listener: listener
                    .as_ref()
                    .ok_or_else(|| failure("foreground owned OMP relay missing"))?,
                endpoint: &endpoint,
                phone: &mut phone,
                native: &mut native,
                history_witnesses: &history_witnesses,
                feedback_presentation: &[],
            },
            &run,
        )
        .await?;
    }
    let devices = goatr::admin::call(&socket, "devices.list", json!({})).await?;
    let own = phone.hello.device_id.clone();
    if !devices["devices"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|r| r["deviceId"].as_str() == Some(&own.0)))
    {
        return Err(failure(
            "actual protected devices.list missed owned paired peer",
        ));
    }
    goatr::admin::call(&socket, "device.revoke", json!({"deviceId":own})).await?;
    let revoked = connect(&endpoint, &host_id, None).await;
    if revoked.is_ok() {
        return Err(failure("revoked actual peer authenticated again"));
    }
    desktop.recheck()?;
    for p in &native {
        p.recheck_processes()?;
    }
    serve.stop().await?;
    goatr::close_endpoint(&endpoint)
        .await
        .map_err(io::Error::other)?;
    write_json(
        &evidence.join(format!("stock-{}.json", run.0)),
        &json!({"gate":"stock-providers","status":checkpoint_status(args),"exhaustivePaging":exhaustive_paging(args),"runId":run,"automatedNativeInputs":args.get_flag("automate-native-inputs"),"gates":gates,"beforeRestart":before_native,"afterRestart":native.iter().map(NativeProof::evidence).collect::<Vec<_>>(),"receiptLookups":replay.iter().map(|(id,_,r)|json!({"action":id,"receipt":r})).collect::<Vec<_>>(),"bootChanged":true,"revoke":"protected local revoke + actual hello rejection","desktopPreserved":true,"retainedResources":{"root":root,"herdr":desktop.process,"native":native.iter().map(NativeProof::evidence).collect::<Vec<_>>()},"limits":["No production source/default socket/provider transitions","Created desktop work retained, not rolled back or wildcard-cleaned","Native inputs require explicit automation consent or genuine manual action; acknowledgements never substitute for observed effects","Missing actual native forms/mode/catalogue/MCP policy or native history proof never silently skip","Real partial/uncertain create interruption is a separately asserted gate, not implied by successful create"]}),
    )?;
    println!(
        "{}",
        json!({"gate":"stock-providers","status":checkpoint_status(args),"exhaustivePaging":exhaustive_paging(args),"runId":run,"evidence":evidence,"retainedDesktopRoot":root})
    );
    if args.get_flag("keep-foreground-for-feedback") {
        // Only a completed selected checkpoint, including an explicit paging deferral,
        // can reach this branch after the ordinary cleanup and result publication.
        let feedback = async {
            desktop.recheck()?;
            if identity(&root)? != feedback_root || binary_sha256(&binary)? != feedback_package {
                return Err(failure("passed feedback root/package changed"));
            }
            for p in &native {
                p.recheck_processes()?;
            }
            serve = start_serve(&binary, &config_path, &root, &socket, listener.as_ref())?;
            healthy(&mut serve, &socket).await?;
            // The passed phone/actions stay revoked. A fresh read peer obtains this
            // boot's authoritative Targets without reusing subscriptions or requests.
            let guard_endpoint = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?))
                .bind().await.map_err(io::Error::other)?;
            let guard_identity = guard_endpoint.id().to_string();
            let mut guard_admitted = false;
            let refreshed = async {
                let before: goatr::admin::Devices = serde_json::from_value(
                    goatr::admin::call(&socket, "devices.list", json!({})).await?
                ).map_err(io::Error::other)?;
                if before.devices.iter().any(|device| device.endpoint_id.0 == guard_identity) {
                    return Err(failure("fresh feedback guard identity already paired; adoption refused"));
                }
                guard_admitted = true;
                let mut guard = pair(&binary, &root, &socket, &guard_endpoint).await?;
                if guard.hello.host_id != host_id || guard.hello.boot_id == phone.hello.boot_id {
                    return Err(failure("feedback guard did not authenticate the original host's new boot"));
                }
                let mut candidate = native.clone();
                refresh_native(&mut guard, &config, &herdr_socket, &mut candidate).await?;
                for (passed, current) in native.iter().zip(&candidate) {
                    passed.recheck_processes()?;
                    current.recheck_processes()?;
                    if !passed.same_feedback_restart_binding(current) {
                        return Err(failure("feedback restart changed original native binding or lacked a fresh generation"));
                    }
                }
                guard.connection.close(0u32.into(), b"feedback guard scope captured");
                Ok(candidate)
            }.await;
            // Match only the newly generated endpoint, including confirm-success/hello-loss.
            // Always close its endpoint even if scope capture or revocation fails.
            let revoked = async {
                if !guard_admitted { return Ok(()); }
                let devices: goatr::admin::Devices = serde_json::from_value(
                    goatr::admin::call(&socket, "devices.list", json!({})).await?
                ).map_err(io::Error::other)?;
                let own = devices.devices.into_iter().filter(|device| device.endpoint_id.0 == guard_identity).collect::<Vec<_>>();
                if own.len() > 1 {
                    return Err(failure("feedback guard device identity is ambiguous; no foreign revoke"));
                }
                if let Some(device) = own.first() {
                    goatr::admin::call(&socket, "device.revoke", json!({"deviceId":device.device_id})).await?;
                } else if refreshed.is_ok() {
                    return Err(failure("authenticated feedback guard vanished before protected revoke"));
                }
                if connect(&guard_endpoint, &host_id, None).await.is_ok() {
                    return Err(failure("revoked feedback guard authenticated again"));
                }
                Ok(())
            }.await;
            let closed = goatr::close_endpoint(&guard_endpoint).await.map_err(io::Error::other);
            let candidate = refreshed?;
            revoked?;
            closed?;
            // Publish all fresh scopes together only after original binding and cleanup proof.
            native = candidate;
            ui::feedback(
                args,
                evidence,
                &run,
                ui::FeedbackContext {
                    binary: &binary,
                    root: &root,
                    socket: &socket,
                    host: &host_id,
                    herdr: &desktop.process,
                    native: &native,
                    serve: &mut serve,
                    root_identity: &feedback_root,
                    package_sha256: &feedback_package,
                },
            )
            .await
        }
        .await;
        if feedback.is_err() {
            let stopped = serve.stop().await;
            let outcome = evidence.join(format!("feedback-outcome-{}.json", run.0));
            if !outcome.try_exists()? {
                write_json(
                    &outcome,
                    &json!({"gate":feedback_gate(args),"exhaustivePaging":exhaustive_paging(args),"runId":run,
                    "status":"failed","stage":"admission","serveStopped":stopped.is_ok(),
                    "desktopRetained":true,"acceptanceUnchanged":true}),
                )?;
            }
        }
        feedback?;
    }
    Ok(true)
}
pub async fn run() -> io::Result<bool> {
    let args = arguments();
    let evidence = std::path::absolute(
        args.get_one::<PathBuf>("evidence-dir")
            .expect("required evidence"),
    )?;
    private_dir(&evidence, true)?;
    if args.get_flag("physical-hello-capture") {
        return physical_hello_capture(&args, &evidence).await;
    }
    if args.get_flag("development-continuity") {
        selected_lease(&args, "development Android consumer")?;
        return dev_continuity::run(&args, &evidence).await;
    }
    if args.get_one::<u32>("api") != Some(&34) {
        return Err(failure(
            "only API34 selected owned emulator resources are authorized",
        ));
    }
    if args.get_flag("prepare-emulator") {
        let l = selected_lease(&args, "emulator prepare")?;
        return prepare(
            args.get_one::<PathBuf>("emulator-owner").ok_or_else(|| {
                failure("existing --emulator-owner lease required; no arbitrary AVD creation")
            })?,
            &l,
            &evidence,
        )
        .await;
    }
    if args.get_flag("camera") || args.get_flag("ime") {
        return ui::components(&args, &evidence).await;
    }
    if args.get_flag("setup") {
        return setup_acceptance::run(&args, &evidence).await;
    }
    if args.get_flag("ui") {
        // Reject a stale or mismatched CLI selector before creating a native/provider run.
        selected_lease(&args, "UI runner")?;
        checkpoint(&args, &evidence).await
    } else {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {r=checkpoint(&args,&evidence)=>r,_=tokio::signal::ctrl_c()=>Err(failure("stock gate cancelled; exact recorded desktop work retained, no replay/rollback")),_=terminate.recv()=>Err(failure("stock gate cancelled; exact recorded desktop work retained, no replay/rollback"))}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_guard_parser_accepts_original_presence_guard_only_as_closed_line() {
        let line = b"goatr_codex_first_guard phase=connection guard=original_presence_connection_epoch code=StaleSession io=none errno=0";
        let value = parse_codex_guard_line(line).expect("closed original-presence guard");
        assert_eq!(value["guard"], "original_presence_connection_epoch");
        assert_eq!(value["code"], "StaleSession");
        let terminal = parse_codex_guard_line(
            b"goatr_codex_first_guard phase=connection guard=original_presence_state_terminal_proof code=StaleSession io=none errno=0",
        )
        .expect("closed terminal-proof presence guard");
        assert_eq!(terminal["guard"], "original_presence_state_terminal_proof");
        let terminal_timeout = parse_codex_guard_line(
            b"goatr_codex_first_guard phase=connection guard=original_presence_state_terminal_snapshot_timeout code=StaleSession io=invalid_data errno=0",
        )
        .expect("closed terminal snapshot timeout guard");
        assert_eq!(
            terminal_timeout["guard"],
            "original_presence_state_terminal_snapshot_timeout"
        );
        assert_eq!(terminal_timeout["io"], "invalid_data");
        let tui_write = parse_codex_guard_line(
            b"goatr_codex_first_guard phase=connection guard=original_presence_state_terminal_tui_write_flush_pending code=StaleSession io=timed_out errno=0",
        )
        .expect("closed TUI write capture guard");
        assert_eq!(
            tui_write["guard"],
            "original_presence_state_terminal_tui_write_flush_pending"
        );
        for guard in [
            "subscription_connect",
            "snapshot_connect",
            "snapshot_read",
            "subscription_event",
            "effective_topology_revalidation",
            "effective_topology_unavailable",
        ] {
            let line = format!(
                "goatr_codex_first_guard phase=herdr guard={guard} code=NotReady io=broken_pipe errno=32"
            );
            assert!(parse_codex_guard_line(line.as_bytes()).is_some());
        }
        assert!(parse_codex_guard_line(
            b"goatr_codex_first_guard phase=connection guard=original_presence_state_terminal_tui_write_flush_pending code=StaleSession io=timed_out errno=0 private=value"
        )
        .is_none());
        assert!(parse_codex_guard_line(
            b"goatr_codex_first_guard phase=connection guard=original_presence_connection_epoch code=StaleSession io=none errno=0 private=value"
        )
        .is_none());
        assert!(parse_codex_guard_line(
            b"goatr_codex_first_guard phase=connection guard=unknown code=StaleSession io=none errno=0"
        )
        .is_none());
    }

    #[test]
    fn omp_launch_capture_backend_only_needs_owned_relay_but_not_private_receiver() {
        let mode = owned_omp_capture_mode(false, false, true);
        assert_eq!(mode, OwnedOmpCaptureMode::Backend);
        assert!(mode.requested());
        assert!(mode.requires_owned_relay());
        assert!(!mode.requires_private_receiver());
        assert_eq!(
            owned_omp_capture_mode(true, false, true),
            OwnedOmpCaptureMode::Conflict
        );
    }

    #[test]
    fn omp_launch_capture_non_succeeded_creation_error_is_closed() {
        let receipt = Receipt {
                    shell_target: None,
            action_id: Uuid::random(),
            method: ActionMethod::SessionCreate,
            boot_id: Uuid::random(),
            target: None,
            created_at: Uint(0),
            updated_at: Uint(0),
            state: ReceiptState::Uncertain,
            error: None,
            result: None,
            creation: None,
        };
        let error = non_succeeded_creation_error(Provider::Omp, &receipt);
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            "actual creation did not succeed: method=SessionCreate; provider=Omp; state=Uncertain; stage=none; stageState=none; code=none"
        );
        assert!(!error.to_string().contains(receipt.action_id.0.as_str()));
    }

    #[test]
    fn connection_output_creation_progress_remains_parseable() {
        for guard in [
            "dispatch_returned",
            "dispatch_deadline",
            "response_queued",
            "output_written_after_queue",
        ] {
            let line = format!(
                "goatr_omp_first_guard phase=transport guard={guard} code=none io=none frame=none"
            );
            let value = parse_omp_guard_line(line.as_bytes()).expect("existing Creation progress");
            assert_eq!(value["guard"], guard);
            assert_eq!(value["frame"], "none");
        }
    }

    #[test]
    fn omp_creation_terminal_guard_parses_into_shared_creation_budget() {
        let budget = herdr_report_capture::Budget::new(false);
        for (phase, guard) in [
            ("herdr_launch", "ack_matched"),
            ("omp_ready", "topology_not_ready"),
            ("omp_ready", "native_report_absent"),
            ("follower", "follower_native_report_absent"),
            ("omp_ready", "report_absent_after_interactive_ready"),
        ] {
            let line = if matches!(guard, "ack_matched") {
                format!(
                    "goatr_omp_first_guard phase={phase} guard={guard} code=none io=none frame=none name=1 kind=1 pane=1 terminal=1 workspace=1 argv=1"
                )
            } else {
                format!(
                    "goatr_omp_first_guard phase={phase} guard={guard} code=NotReady io=none frame=none"
                )
            };
            let value = parse_omp_guard_line(line.as_bytes()).expect("closed Creation guard");
            budget.omp(value);
        }
        for guard in [
            "report_absent_after_interactive_ready",
            "report_absent_without_interactive_ready",
            "original_agent_absent",
        ] {
            let line = format!(
                "goatr_omp_first_guard phase=omp_ready guard={guard} code=NotReady io=none frame=none"
            );
            assert_eq!(
                parse_omp_guard_line(line.as_bytes()).expect("closed terminal guard")["guard"],
                guard
            );
        }
        let shell_probe = parse_omp_guard_line(
            b"goatr_omp_first_guard phase=shell_probe guard=root_recheck_focus code=NotReady io=none frame=none",
        )
        .expect("closed pre-intent terminal guard");
        assert_eq!(shell_probe["phase"], "shell_probe");
        assert_eq!(shell_probe["guard"], "root_recheck_focus");
        let shell_budget = herdr_report_capture::Budget::new(false);
        for line in [
            b"goatr_omp_first_guard phase=herdr_launch guard=ack_matched code=none io=none frame=none name=1 kind=1 pane=1 terminal=1 workspace=1 argv=1".as_slice(),
            b"goatr_omp_first_guard phase=follower guard=follower_registry_read code=NotReady io=none frame=none".as_slice(),
        ] {
            shell_budget.omp(parse_omp_guard_line(line).expect("closed nonterminal guard"));
        }
        shell_budget.omp(shell_probe);
        assert_eq!(shell_budget.creation_counts(), (3, 2, true));
        assert!(parse_omp_guard_line(
            b"goatr_omp_first_guard phase=omp_ready guard=original_agent_absent code=NotReady io=none frame=none private=value",
        )
        .is_none());
        assert!(parse_omp_guard_line(
            b"goatr_omp_first_guard phase=shell_probe guard=root_recheck_focus code=NotReady io=none frame=none native=value",
        )
        .is_none());
    }

    #[test]
    fn connection_output_phone_close_reason_is_closed() {
        assert_eq!(closed_connection_reason(None), "open");
        assert_eq!(
            closed_connection_reason(Some(iroh::endpoint::ConnectionError::Reset)),
            "reset"
        );
        assert_eq!(
            closed_connection_reason(Some(iroh::endpoint::ConnectionError::TimedOut)),
            "timed_out"
        );
    }

    #[test]
    fn apk_install_failure_category_never_retains_private_diagnostics() {
        assert_eq!(
            install_failure_category(
                b"",
                b"Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: private path and certificate]"
            ),
            "INSTALL_FAILED_UPDATE_INCOMPATIBLE"
        );
        assert_eq!(
            install_failure_category(b"INSTALL_FAILED_TEST_ONLY", b"INSTALL_FAILED_INVALID_APK"),
            "other"
        );
        assert_eq!(
            install_failure_category(
                b"",
                b"privateINSTALL_FAILED_TEST_ONLY INSTALL_FAILED_TEST_ONLY2 secret"
            ),
            "other"
        );
        assert_eq!(
            install_failure_category(b"", b"INSTALL_FAILED_FUTURE_PRIVATE_VALUE"),
            "other"
        );
        let failure = ApkInstallFailure {
            category: "other",
            exit: Some(1),
        };
        assert_eq!(
            failure.to_string(),
            "SDK apk_install failed: category=other, exit=Some(1); raw output omitted"
        );
    }
    #[test]
    fn geometry_failure_display_is_closed_for_all_callsite_phases() {
        let failures = [
            GeometryFailure::connect(
                GeometryPhase::CreateBefore,
                &io::Error::new(io::ErrorKind::NotFound, "private socket path"),
            ),
            GeometryFailure::response(
                GeometryPhase::CreateAfter,
                herdr::ApiError::Unavailable(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Herdr response timeout",
                )),
            ),
            GeometryFailure::response(
                GeometryPhase::InputBefore,
                herdr::ApiError::Rejected("private rejection value".into()),
            ),
            GeometryFailure::response(
                GeometryPhase::InputAfter,
                herdr::ApiError::Changed(json!({"private":"native response"})),
            ),
        ];
        let rendered: Vec<_> = failures.iter().map(ToString::to_string).collect();
        assert_eq!(
            rendered[0],
            "owned Herdr geometry failed: phase=create_before; guard=connect; io_kind=NotFound; no retry"
        );
        assert_eq!(
            rendered[1],
            "owned Herdr geometry failed: phase=create_after; guard=response_timeout; io_kind=InvalidData; no retry"
        );
        assert_eq!(
            rendered[2],
            "owned Herdr geometry failed: phase=input_before; guard=native_rejected; io_kind=None; no retry"
        );
        assert_eq!(
            rendered[3],
            "owned Herdr geometry failed: phase=input_after; guard=source_changed; io_kind=None; no retry"
        );
        assert_eq!(
            GeometryFailure::new(GeometryPhase::UiBefore, GeometryGuard::LayoutMissing).to_string(),
            "owned Herdr geometry failed: phase=ui_before; guard=layout_missing; io_kind=None; no retry"
        );
        assert_eq!(
            GeometryFailure::new(GeometryPhase::UiAfter, GeometryGuard::LayoutBounds).to_string(),
            "owned Herdr geometry failed: phase=ui_after; guard=layout_bounds; io_kind=None; no retry"
        );
        assert!(rendered.iter().all(|text| !text.contains("private")));
        assert_eq!(failures[0].into_io().kind(), io::ErrorKind::NotFound);
        assert_eq!(failures[1].into_io().kind(), io::ErrorKind::NotConnected);
        assert_eq!(failures[2].into_io().kind(), io::ErrorKind::NotConnected);
        assert_eq!(failures[3].into_io().kind(), io::ErrorKind::Other);
    }

    #[test]
    fn geometry_failure_preserves_outer_kinds_without_exposing_private_details() {
        let unknown_connect = GeometryFailure::connect(
            GeometryPhase::InputBefore,
            &io::Error::new(io::ErrorKind::WouldBlock, "private unclassified connect"),
        );
        assert_eq!(unknown_connect.io_kind, GeometryIoKind::Other);
        assert_eq!(unknown_connect.into_io().kind(), io::ErrorKind::WouldBlock);

        let create_after_connect = GeometryFailure::connect(
            GeometryPhase::CreateAfter,
            &io::Error::new(io::ErrorKind::ConnectionRefused, "private refused socket"),
        );
        assert_eq!(
            create_after_connect.into_io().kind(),
            io::ErrorKind::ConnectionRefused
        );

        let timeout_like = GeometryFailure::response(
            GeometryPhase::InputAfter,
            herdr::ApiError::Unavailable(io::Error::new(
                io::ErrorKind::InvalidData,
                "private timeout text",
            )),
        );
        assert_eq!(timeout_like.guard, GeometryGuard::ResponseIo);
        assert_eq!(timeout_like.io_kind, GeometryIoKind::InvalidData);
        assert_eq!(timeout_like.into_io().kind(), io::ErrorKind::Other);
        let unavailable = GeometryFailure::response(
            GeometryPhase::CreateBefore,
            herdr::ApiError::Unavailable(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "private transport text",
            )),
        );
        assert_eq!(unavailable.guard, GeometryGuard::ResponseIo);
        assert_eq!(unavailable.io_kind, GeometryIoKind::ConnectionReset);
        assert_eq!(unavailable.into_io().kind(), io::ErrorKind::NotConnected);
        assert_eq!(
            GeometryFailure::new(GeometryPhase::CreateBefore, GeometryGuard::LayoutMissing)
                .into_io()
                .kind(),
            io::ErrorKind::Other
        );
        assert_eq!(
            GeometryFailure::new(GeometryPhase::CreateAfter, GeometryGuard::LayoutBounds)
                .into_io()
                .kind(),
            io::ErrorKind::Other
        );
        for failure in [
            GeometryFailure::connect(
                GeometryPhase::InputAfter,
                &io::Error::new(io::ErrorKind::PermissionDenied, "private input socket"),
            ),
            GeometryFailure::response(
                GeometryPhase::InputAfter,
                herdr::ApiError::Rejected("private input rejection".into()),
            ),
            GeometryFailure::new(GeometryPhase::InputAfter, GeometryGuard::LayoutMissing),
        ] {
            assert_eq!(failure.into_io().kind(), io::ErrorKind::Other);
        }
        assert_eq!(
            GeometryFailure::new(GeometryPhase::InputAfter, GeometryGuard::LayoutMissing)
                .to_string(),
            "owned Herdr geometry failed: phase=input_after; guard=layout_missing; io_kind=None; no retry"
        );
        assert_eq!(
            GeometryFailure::new(GeometryPhase::InputAfter, GeometryGuard::LayoutBounds)
                .to_string(),
            "owned Herdr geometry failed: phase=input_after; guard=layout_bounds; io_kind=None; no retry"
        );
    }

    #[test]
    fn connection_output_guard_parser_accepts_only_closed_server_fields() {
        let parsed = parse_omp_guard_line(
            b"goatr_omp_first_guard phase=transport guard=output_pop_error code=NotReady io=none frame=none",
        )
        .expect("closed server guard");
        assert_eq!(parsed["phase"], "transport");
        assert_eq!(parsed["guard"], "output_pop_error");
        let reader = parse_omp_guard_line(
            b"goatr_omp_first_guard phase=transport guard=reader_frame code=none io=connection_reset frame=frame_payload",
        )
        .expect("closed reader frame guard");
        assert_eq!(reader["io"], "connection_reset");
        assert_eq!(reader["frame"], "frame_payload");
        for line in [
            b"goatr_omp_first_guard phase=transport guard=reader_frame code=none io=other frame=unknown"
                .as_slice(),
            b"goatr_omp_first_guard phase=transport guard=unknown code=none io=none frame=none".as_slice(),
            b"goatr_omp_first_guard phase=transport guard=reader_frame code=none io=other frame=none native=private"
                .as_slice(),
        ] {
            assert!(parse_omp_guard_line(line).is_none());
        }
    }

    fn selected_emulator_lease(run_id: &str, serial: &str, root: &str) -> Lease {
        let root = PathBuf::from(root);
        let avd_home = root.join("avd-home");
        Lease {
            schema: 1,
            run_id: run_id.into(),
            owner_uid: uid(),
            root: root.clone(),
            serial: serial.into(),
            avd_name: "owned-avd".into(),
            android_user_home: root.join("android-user"),
            avd_path: avd_home.join("owned-avd.avd"),
            avd_home,
            process_identity: EmulatorProcess {
                linux_boot_id: Uuid::random().0,
                pid: 1,
                start_ticks: "1".into(),
                executable: PathBuf::from("/nix/store/owned-emulator/qemu"),
            },
        }
    }
    #[test]
    fn selected_emulator_selector_is_lease_local_and_closed() {
        let first = selected_emulator_lease(
            "8bd5376a-b170-4fa3-80c0-d449834fed4d",
            "emulator-5594",
            "/owned/first",
        );
        let second = selected_emulator_lease(
            "606603e5-e52a-4b57-90c2-8214c1f38071",
            "emulator-5582",
            "/owned/second",
        );
        assert_eq!(lease_console_port(&first).unwrap(), 5594);
        assert_eq!(
            adb_selector(&first).unwrap(),
            ["-H", "127.0.0.1", "-P", "5037", "-s", "emulator-5594"]
        );
        assert_eq!(
            emulator_runtime_dir(&first).unwrap(),
            PathBuf::from("/owned/first/camera-runtime-8bd5376a")
        );
        assert_eq!(lease_console_port(&second).unwrap(), 5582);
        assert_ne!(
            adb_selector(&first).unwrap(),
            adb_selector(&second).unwrap()
        );
        assert_ne!(
            emulator_runtime_dir(&first).unwrap(),
            emulator_runtime_dir(&second).unwrap()
        );
        assert!(selected_serial(None, &first, "fixture").is_err());
        assert!(selected_serial(Some("emulator-5582"), &first, "fixture").is_err());
        assert!(selected_serial(Some("emulator-5594"), &first, "fixture").is_ok());
        for invalid in [
            "emulator-5595",
            "emulator-5552",
            "emulator-5684",
            "emulator-05594",
        ] {
            assert!(supported_console_port(invalid).is_none());
        }
        assert_eq!(
            selected_serial_availability(
                "emulator-5594",
                false,
                b"",
                b"error: device 'emulator-5594' not found\n"
            ),
            SelectedSerialAvailability::Absent
        );
        assert_eq!(
            selected_serial_availability("emulator-5594", true, b"device\n", b""),
            SelectedSerialAvailability::Occupied
        );
        assert_eq!(
            selected_serial_availability("emulator-5594", false, b"", b"device offline\n"),
            SelectedSerialAvailability::Uncertain
        );
    }
    fn generation_snapshot() -> SessionOpenResult {
        serde_json::from_value(json!({
            "subscription":Uuid::random(),"seq":0,"pendingRequests":[],"history":[],
            "state":{"summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),
                "serverId":Uuid::random(),"workspaceId":"owned","terminalId":"terminal",
                "paneId":"pane","provider":"codex","nativeId":"thread","title":"fixture",
                "status":"idle","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                    "promptMode":"submit_or_steer","cancelMode":"native_turn"},
                "historyCoverage":{"scope":"thread","source":"native_pages",
                    "olderAvailable":true,"sourceTruncated":false,"desktopBranchKnown":false}},
                "desktopSelection":"bound_launch_thread","activity":{"state":"idle"}}
        }))
        .unwrap()
    }
    fn generation_record(index: u64) -> HistoryRecord {
        let marker = format!("goatr-history-fixture-{index}");
        // Pinned UserShell display argv is shlex-joined, not the bare script.
        let display = format!(r#"/bin/bash -lc "printf '%s\\n' '{marker}'""#);
        serde_json::from_value(json!({"id":format!("native-{index}"),"turnId":"turn",
            "order":index,"role":"tool","kind":"tool","source":"live",
            "complete":true,"sourceTruncated":false,
            "tool":{"name":"commandExecution","status":"succeeded",
                "input":{"kind":"text","text":json!({"command":display}).to_string()},
                "output":{"kind":"text","text":json!({"aggregatedOutput":format!("{marker}\n"),"exitCode":0}).to_string()}}})).unwrap()
    }
    fn generation_event(view: &GenerationView, seq: u64, body: EventBody) -> Event {
        Event {
            subscription: view.snapshot.subscription.clone(),
            seq: Uint(seq),
            body,
        }
    }
    #[test]
    fn generation_shell_display_matches_pinned_literal_argv_without_evaluation() {
        let command = r#"printf '%s\n' 'marker'"#;
        for display in [
            r#"/bin/bash -lc "printf '%s\\n' 'marker'""#,
            r#"/nix/store/fixture/bin/zsh -lc 'printf '"'"'%s\n'"'"' '"'"'marker'"'"''"#,
            r#"/bin/sh -lc printf\ \'%s\\n\'\ \'marker\'"#,
        ] {
            assert!(codex_shell_display_matches(display, command));
        }
        // Quoting preserves apostrophes, backslashes and literal shell metacharacters.
        assert!(codex_shell_display_matches(
            r#"'/absolute path/bash' -lc "printf 'it\"s' '\$literal' '\`literal\`'""#,
            r#"printf 'it"s' '$literal' '`literal`'"#
        ));
        for display in [
            command,
            r#"bash -lc "printf '%s\\n' 'marker'""#,
            r#"/bin/fish -lc "printf '%s\\n' 'marker'""#,
            r#"/bin/bash -c "printf '%s\\n' 'marker'""#,
            r#"/bin/bash -lc "printf '%s\\n' 'marker'" extra"#,
            r#"/bin/bash -lc "printf '%s\\n' 'marker'"; other"#,
            r#"/bin/bash -lc "$(private-command)""#,
            r#"/bin/bash -lc "`private-command`""#,
            r#"/bin/bash -lc "printf '%s\\n' 'different'""#,
            r#"/bin/bash -lc 'unterminated"#,
            "'/bin/\tbash' -lc command",
        ] {
            assert!(!codex_shell_display_matches(display, command));
        }
    }
    #[test]
    fn generation_subscription_reduces_interleaved_events_and_bounds_original_history() {
        let snapshot = generation_snapshot();
        snapshot.validate().unwrap();
        let expected = target(&snapshot.state.summary);
        let mut view = GenerationView::new(snapshot, &expected).unwrap();
        for index in 0..=goatr::state::HISTORY_RECORDS as u64 {
            // Existing RPC receipt/host traffic must not consume the session sequence.
            view.routed_event(Event {
                subscription: Uuid::random(),
                seq: Uint(index + 1),
                body: EventBody::HostChanged(HostChangedEvent {}),
            })
            .unwrap();
            let event = generation_event(
                &view,
                index + 1,
                EventBody::HistoryUpsert(HistoryUpsertEvent {
                    records: vec![generation_record(index)],
                }),
            );
            view.routed_event(event.clone()).unwrap();
            view.routed_event(event).unwrap(); // protocol duplicate is idempotent
        }
        assert_eq!(view.snapshot.history.len(), goatr::state::HISTORY_RECORDS);
        assert_eq!(view.snapshot.history.first().unwrap().id.0, "native-1");
        assert_eq!(view.record_bytes.len(), goatr::state::HISTORY_RECORDS);
        assert_eq!(
            view.bytes,
            view.snapshot
                .history
                .iter()
                .map(|record| serde_json::to_vec(record).unwrap().len())
                .sum::<usize>()
        );
        let mut replacement = generation_record(1000);
        replacement.tool.as_mut().unwrap().status = ToolStatus::Failed;
        let event = generation_event(
            &view,
            1002,
            EventBody::HistoryUpsert(HistoryUpsertEvent {
                records: vec![replacement.clone()],
            }),
        );
        view.routed_event(event).unwrap();
        assert_eq!(view.snapshot.history.last(), Some(&replacement));
    }
    #[test]
    fn generation_subscription_pending_sync_and_loss_never_reopen_or_retarget() {
        let snapshot = generation_snapshot();
        let expected = target(&snapshot.state.summary);
        let mut view = GenerationView::new(snapshot.clone(), &expected).unwrap();
        let pending = PendingRequest {
            request_key: Uuid::random(),
            provider: Provider::Codex,
            kind: PresentationKind::DesktopOnly,
            presentation: Presentation::DesktopOnly(PresentationDesktopOnly {
                title: Content::Text(ContentText {
                    text: InlineText("fixture".into()),
                }),
                reason: "fixture".into(),
            }),
        };
        let event = generation_event(
            &view,
            1,
            EventBody::RequestOpened(RequestOpenedEvent {
                request: pending.clone(),
            }),
        );
        view.routed_event(event).unwrap();
        assert_eq!(view.snapshot.pending_requests, vec![pending.clone()]);
        let event = generation_event(
            &view,
            2,
            EventBody::RequestClosed(RequestClosedEvent {
                request_key: pending.request_key,
                reason: RequestClosedReason::Resolved,
            }),
        );
        view.routed_event(event).unwrap();
        assert!(view.snapshot.pending_requests.is_empty());
        let mut state = *snapshot.state.clone();
        state.summary.connectivity = SessionConnectivity::Synchronizing;
        let event = generation_event(
            &view,
            3,
            EventBody::SessionState(SessionStateEvent { state }),
        );
        view.routed_event(event).unwrap();
        assert_eq!(
            view.snapshot.state.summary.connectivity,
            SessionConnectivity::Synchronizing
        );
        for fault in ["gap", "source", "generation", "offline", "sync"] {
            let mut view = GenerationView::new(snapshot.clone(), &expected).unwrap();
            let mut state = *snapshot.state.clone();
            if fault == "generation" {
                state.summary.generation = Uuid::random();
            }
            if fault == "offline" {
                state.summary.connectivity = SessionConnectivity::Offline;
            }
            let body = if fault == "sync" {
                EventBody::SyncRequired(SyncRequiredEvent {
                    reason: SyncReason::SourceGap,
                })
            } else {
                EventBody::SessionState(SessionStateEvent { state })
            };
            let mut event = generation_event(&view, if fault == "gap" { 2 } else { 1 }, body);
            if fault == "source" {
                event.subscription = Uuid::random();
            }
            assert!(view.routed_event(event).is_err(), "{fault}");
            assert!(view.failed);
            let event = generation_event(
                &view,
                1,
                EventBody::HistoryUpsert(HistoryUpsertEvent { records: vec![] }),
            );
            assert!(view.routed_event(event).is_err(), "{fault}");
        }
    }
    #[tokio::test(start_paused = true)]
    async fn generation_command_envelope_preserves_inner_and_phase_timeouts() {
        for (
            proof_only,
            provider,
            phase_seconds,
            input_seconds,
            completion_seconds,
            expected,
            elapsed,
        ) in [
            (true, Provider::Codex, 3600, 14, 18, "completed", 32),
            (false, Provider::Codex, 3600, 14, 18, "outer_timeout", 30),
            (true, Provider::Omp, 3600, 14, 18, "outer_timeout", 30),
            (true, Provider::Codex, 25, 14, 18, "outer_timeout", 25),
            (true, Provider::Codex, 3600, 31, 1, "input_timeout", 30),
            (
                true,
                Provider::Codex,
                3600,
                14,
                31,
                "completion_timeout",
                44,
            ),
        ] {
            let started = tokio::time::Instant::now();
            let deadline = history_command_deadline(
                proof_only,
                provider,
                (started + Duration::from_secs(phase_seconds)).into_std(),
                started.into_std(),
            );
            let mut dispatches = 0;
            let mut completed = false;
            let result = tokio::time::timeout_at(deadline.into(), async {
                dispatches += 1;
                tokio::time::timeout(
                    Duration::from_secs(30),
                    tokio::time::sleep(Duration::from_secs(input_seconds)),
                )
                .await
                .map_err(|_| "input_timeout")?;
                tokio::time::timeout(
                    Duration::from_secs(30),
                    tokio::time::sleep(Duration::from_secs(completion_seconds)),
                )
                .await
                .map_err(|_| "completion_timeout")?;
                completed = true;
                Ok::<_, &str>(())
            })
            .await;
            let actual = match result {
                Ok(Ok(())) => "completed",
                Ok(Err(phase)) => phase,
                Err(_) => "outer_timeout",
            };
            assert_eq!(actual, expected);
            assert_eq!(started.elapsed(), Duration::from_secs(elapsed));
            assert_eq!(dispatches, 1, "timeouts must never replay input");
            assert_eq!(completed, expected == "completed");
        }
    }
    #[test]
    fn generation_completion_waits_for_idle_and_keeps_closed_context() {
        let mut snapshot = generation_snapshot();
        snapshot.history.push(generation_record(0));
        snapshot.state.summary.status = SessionStatus::Working;
        let mut trace = HistoryCommandTrace::default();
        assert_eq!(
            codex_shell_completion(
                &snapshot,
                "printf '%s\\n' 'goatr-history-fixture-0'",
                "goatr-history-fixture-0",
                &mut trace
            )
            .unwrap(),
            None
        );
        assert!(trace.completed && !trace.idle);
        let error = trace.deadline().to_string();
        assert!(
            error.contains("completed=true")
                && error.contains("idle=false")
                && error.contains("phase=")
        );
        assert!(!error.contains("fixture") && !error.contains("native-0"));
        snapshot.state.summary.status = SessionStatus::Idle;
        assert_eq!(
            codex_shell_completion(
                &snapshot,
                "printf '%s\\n' 'goatr-history-fixture-0'",
                "goatr-history-fixture-0",
                &mut trace
            )
            .unwrap(),
            Some(Id("native-0".into()))
        );
        assert!(trace.completed && trace.idle);
    }
    #[test]
    fn generation_completion_requires_exact_successful_native_effect_and_closed_timeout_context() {
        let mut snapshot = generation_snapshot();
        snapshot.history.push(generation_record(0));
        let command = "printf '%s\\n' 'goatr-history-fixture-0'";
        let marker = "goatr-history-fixture-0";
        let mut trace = HistoryCommandTrace::default();
        assert_eq!(
            codex_shell_completion(&snapshot, command, marker, &mut trace).unwrap(),
            Some(Id("native-0".into()))
        );
        snapshot.state.summary.connectivity = SessionConnectivity::Synchronizing;
        assert_eq!(
            codex_shell_completion(&snapshot, command, marker, &mut trace).unwrap(),
            None
        );
        assert!(trace.completed && trace.marker_seen && !trace.ready);
        snapshot.state.summary.connectivity = SessionConnectivity::Ready;
        for (fault, guard) in [
            ("command", "command_mismatch"),
            ("marker", "marker_count"),
            ("exit", "unsuccessful_completion"),
            ("failed", "unsuccessful_completion"),
            ("truncated", "source_truncated"),
            ("duplicate", "duplicate_completion"),
        ] {
            let mut changed = snapshot.clone();
            match fault {
                "command" => {
                    changed.history[0].tool.as_mut().unwrap().input =
                        Some(Content::Text(ContentText {
                            text: InlineText(
                                json!({"command":"other private command"}).to_string(),
                            ),
                        }))
                }
                "exit" => {
                    changed.history[0].tool.as_mut().unwrap().output =
                        Some(Content::Text(ContentText {
                            text: InlineText(
                                json!({"aggregatedOutput":format!("{marker}\n"),"exitCode":1})
                                    .to_string(),
                            ),
                        }))
                }
                "marker" => changed.history[0].tool.as_mut().unwrap().output = Some(Content::Text(ContentText {
                    text: InlineText(json!({"aggregatedOutput":format!("{marker}\n{marker}\n"),"exitCode":0}).to_string()),
                })),
                "failed" => changed.history[0].tool.as_mut().unwrap().status = ToolStatus::Failed,
                "truncated" => changed.history[0].source_truncated = true,
                "duplicate" => {
                    let mut duplicate = changed.history[0].clone();
                    duplicate.id = Id("duplicate".into());
                    changed.history.push(duplicate);
                }
                _ => unreachable!(),
            }
            let error = codex_shell_completion(&changed, command, marker, &mut trace)
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("guard={guard};")), "{fault}");
            assert!(error.contains("marker_seen=true") && error.contains("ready=true"));
            for secret in [
                command,
                marker,
                "native-0",
                "private",
                &snapshot.subscription.0,
            ] {
                assert!(!error.contains(secret));
            }
        }
        trace.phase = HistoryCommandPhase::AwaitEvent;
        trace.method = Some(Method::SessionOpen);
        trace.request_phase = "history_subscription_completion";
        trace.request_guard = "frame_read";
        let error = trace.deadline().to_string();
        assert!(
            error.contains("AwaitEvent")
                && error.contains("SessionOpen")
                && error.contains("frame_read")
        );
        for secret in [
            command,
            marker,
            "native-0",
            "private",
            &snapshot.subscription.0,
        ] {
            assert!(!error.contains(secret));
        }
    }
    #[test]
    fn feedback_restart_accepts_only_fresh_generation_with_original_physical_binding() {
        let process = ProcessEvidence {
            linux_boot_id: Uuid::random(),
            pid: 100,
            start_ticks: 10,
            namespaces: herdr::ProcessNamespaces {
                pid: (1, 2),
                mount: (1, 3),
                user: (1, 4),
                nspids: vec![100],
            },
        };
        for provider in [Provider::Omp, Provider::Codex] {
            let passed = NativeProof {
                target: Target {
                    session_id: Uuid::random(),
                    generation: Uuid::random(),
                },
                provider,
                native: Id("original-native".into()),
                pane: Id("original-pane".into()),
                terminal: Id("original-terminal".into()),
                workspace: Id("original-workspace".into()),
                owner: process.clone(),
                tui: Some(process.clone()),
                relay: Some(process.clone()),
                manifest: Some(PathBuf::from("/owned/binding.json")),
                collaboration: None,
                default_plan: None,
                omp_observer: None,
                objects: json!({"herdr":"original-herdr", "instance":"original-instance",
                    "roomGeneration":"original-room", "runtime":"original-runtime",
                    "nativeHeader":"original-header", "historyPath":"original-history",
                    "outerWrapper":"original-wrapper", "ownerEpoch":"original-owner",
                    "ownerId":"original-owner-id", "bindingId":"original-binding",
                    "connectionEpoch":"original-relay-connection", "listenHost":"original-listen",
                    "upstreamHost":"original-upstream"}),
            };
            assert!(passed.same_input_binding(&passed));
            assert!(!passed.same_feedback_restart_binding(&passed));
            let mut current = passed.clone();
            current.target.generation = Uuid::random();
            assert!(passed.same_feedback_restart_binding(&current));
            // Old action admission still requires its exact original generation.
            assert!(!passed.same_input_binding(&current));
            let changes: &[fn(&mut NativeProof)] = &[
                |p| p.target.session_id = Uuid::random(),
                |p| {
                    p.provider = if p.provider == Provider::Omp {
                        Provider::Codex
                    } else {
                        Provider::Omp
                    }
                },
                |p| p.native = Id("foreign-native".into()),
                |p| p.pane = Id("foreign-pane".into()),
                |p| p.terminal = Id("foreign-terminal".into()),
                |p| p.workspace = Id("foreign-workspace".into()),
                |p| p.owner.start_ticks += 1,
                |p| p.tui.as_mut().unwrap().start_ticks += 1,
                |p| p.relay.as_mut().unwrap().start_ticks += 1,
                |p| p.manifest = Some(PathBuf::from("/foreign/binding.json")),
            ];
            for change in changes {
                let mut changed = current.clone();
                change(&mut changed);
                assert!(!passed.same_feedback_restart_binding(&changed));
            }
            let keys: &[&str] = if provider == Provider::Omp {
                &[
                    "herdr",
                    "instance",
                    "roomGeneration",
                    "runtime",
                    "nativeHeader",
                    "historyPath",
                    "outerWrapper",
                ]
            } else {
                &[
                    "herdr",
                    "ownerEpoch",
                    "ownerId",
                    "bindingId",
                    "connectionEpoch",
                    "listenHost",
                    "upstreamHost",
                    "outerWrapper",
                ]
            };
            for key in keys {
                let mut changed = current.clone();
                changed.objects[*key] = json!("foreign-provenance");
                assert!(!passed.same_feedback_restart_binding(&changed));
                changed.objects.as_object_mut().unwrap().remove(*key);
                assert!(!passed.same_feedback_restart_binding(&changed));
            }
        }
    }
    #[test]
    fn retention_deferral_requires_both_owned_ui_providers_and_labels_checkpoint() {
        let accepted = [
            "smoke",
            "--defer-retention-crossing",
            "--ui",
            "--backend",
            "omp",
            "--backend",
            "codex",
            "--consent-owned-providers",
            "--automate-native-inputs",
            "--evidence-dir",
            "/unused",
        ];
        let deferred = feedback_arguments(arguments_base())
            .try_get_matches_from(accepted)
            .unwrap();
        require_both_checkpoint_providers(&deferred, &[Provider::Omp, Provider::Codex]).unwrap();
        for selected in [
            vec![],
            vec![Provider::Omp],
            vec![Provider::Codex],
            vec![Provider::Omp, Provider::Omp],
        ] {
            assert!(require_both_checkpoint_providers(&deferred, &selected).is_err());
        }
        assert_eq!(exhaustive_paging(&deferred), "deferred_by_user");
        assert_eq!(
            checkpoint_status(&deferred),
            "foreground_checkpoint_paging_deferred"
        );
        assert_eq!(feedback_gate(&deferred), "post-checkpoint-feedback");
        for omitted in [
            "--ui",
            "--consent-owned-providers",
            "--automate-native-inputs",
        ] {
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(accepted.iter().copied().filter(|arg| *arg != omitted))
                    .is_err()
            );
        }
        for forbidden in ["--camera", "--ime", "--setup", "--prepare-emulator"] {
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(
                        accepted.iter().copied().chain(std::iter::once(forbidden))
                    )
                    .is_err()
            );
        }
        let strict = feedback_arguments(arguments_base())
            .try_get_matches_from(
                accepted
                    .iter()
                    .copied()
                    .filter(|arg| *arg != "--defer-retention-crossing"),
            )
            .unwrap();
        assert!(!strict.get_flag("defer-retention-crossing"));
        assert_eq!(exhaustive_paging(&strict), "required");
        assert_eq!(checkpoint_status(&strict), "pass");
        assert_eq!(feedback_gate(&strict), "post-pass-feedback");
        for selected in [vec![], vec![Provider::Omp], vec![Provider::Codex]] {
            assert!(require_both_checkpoint_providers(&strict, &selected).is_err());
        }
        let feedback = feedback_arguments(arguments_base())
            .try_get_matches_from(
                accepted
                    .iter()
                    .copied()
                    .chain(std::iter::once("--keep-foreground-for-feedback")),
            )
            .unwrap();
        require_both_checkpoint_providers(&feedback, &[Provider::Omp, Provider::Codex]).unwrap();
    }
    #[test]
    fn feedback_option_requires_ui_consent_and_rejects_diagnostics() {
        let accepted = [
            "smoke",
            "--keep-foreground-for-feedback",
            "--ui",
            "--backend",
            "omp",
            "--backend",
            "codex",
            "--consent-owned-providers",
            "--automate-native-inputs",
            "--evidence-dir",
            "/unused",
        ];
        assert!(
            feedback_arguments(arguments_base())
                .try_get_matches_from(accepted)
                .is_ok()
        );
        for omitted in [
            "--ui",
            "--consent-owned-providers",
            "--automate-native-inputs",
        ] {
            let args = accepted.iter().copied().filter(|arg| *arg != omitted);
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(args)
                    .is_err()
            );
        }
        for forbidden in [
            "--codex-history-diagnostic",
            "--camera",
            "--ime",
            "--setup",
            "--prepare-emulator",
        ] {
            let args = accepted.iter().copied().chain(std::iter::once(forbidden));
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(args)
                    .is_err()
            );
        }
    }
    #[test]
    fn development_continuity_requires_both_owned_providers_and_frozen_package() {
        let accepted = [
            "smoke",
            "--development-continuity",
            "--backend",
            "omp",
            "--backend",
            "codex",
            "--goatr",
            "/nix/store/test-goatr/bin/goatr",
            "--expected-goatr-sha256",
            "0123456789012345678901234567890123456789012345678901234567890123",
            "--expected-companion-lock-sha256",
            "0123456789012345678901234567890123456789012345678901234567890123",
            "--config",
            "/owned/config.toml",
            "--workspace-root",
            "/owned/workspace",
            "--emulator-owner",
            "/owned/emulator.json",
            "--apk",
            "/owned/app.apk",
            "--test-apk",
            "/owned/app-test.apk",
            "--expected-apk-sha256",
            "0123456789012345678901234567890123456789012345678901234567890123",
            "--expected-test-apk-sha256",
            "0123456789012345678901234567890123456789012345678901234567890123",
            "--consent-owned-providers",
            "--automate-native-inputs",
            "--evidence-dir",
            "/owned/evidence",
        ];
        let value_options = [
            "--backend",
            "--goatr",
            "--expected-goatr-sha256",
            "--expected-companion-lock-sha256",
            "--config",
            "--workspace-root",
            "--emulator-owner",
            "--apk",
            "--test-apk",
            "--expected-apk-sha256",
            "--expected-test-apk-sha256",
        ];
        assert!(
            feedback_arguments(arguments_base())
                .try_get_matches_from(accepted)
                .is_ok()
        );
        for omitted in [
            "--backend",
            "--goatr",
            "--expected-goatr-sha256",
            "--expected-companion-lock-sha256",
            "--config",
            "--workspace-root",
            "--emulator-owner",
            "--apk",
            "--test-apk",
            "--expected-apk-sha256",
            "--expected-test-apk-sha256",
            "--consent-owned-providers",
            "--automate-native-inputs",
        ] {
            let mut skip_value = false;
            let args = accepted.iter().copied().filter(|argument| {
                if skip_value {
                    skip_value = false;
                    return false;
                }
                if *argument == omitted {
                    skip_value = value_options.contains(argument);
                    return false;
                }
                true
            });
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(args)
                    .is_err()
            );
        }
        assert!(
            feedback_arguments(arguments_base())
                .try_get_matches_from(accepted.iter().copied().chain(["--ui"]))
                .is_err()
        );
    }
    #[test]
    fn native_guard_context_preserves_inner_phase_and_discards_private_errors() {
        let inner = native_guard_context(
            NativeGuardPhase::CodexPresence,
            io::Error::new(io::ErrorKind::PermissionDenied, "unsafe private path"),
        );
        let contextual = native_guard_context(NativeGuardPhase::PreInputScope, inner);
        assert_eq!(contextual.kind(), io::ErrorKind::PermissionDenied);
        let detail = contextual
            .get_ref()
            .unwrap()
            .downcast_ref::<NativeGuardFailure>()
            .unwrap();
        assert_eq!(detail.phase, NativeGuardPhase::CodexPresence);
        assert!(detail.unsafe_private_path);
        assert!(std::error::Error::source(detail).is_none());
        assert!(
            contextual
                .to_string()
                .contains("phase=codex_presence; category=unsafe_private_path")
        );
        for (kind, raw) in [
            (
                io::ErrorKind::PermissionDenied,
                "unsafe private path: /private/native-secret",
            ),
            (io::ErrorKind::InvalidData, "unsafe private path"),
            (io::ErrorKind::NotFound, "/private/native-secret missing"),
        ] {
            let contextual = native_guard_context(
                NativeGuardPhase::InputIntentEvidence,
                io::Error::new(kind, raw),
            );
            assert_eq!(contextual.kind(), kind);
            let detail = contextual
                .get_ref()
                .unwrap()
                .downcast_ref::<NativeGuardFailure>()
                .unwrap();
            assert!(!detail.unsafe_private_path);
            let text = contextual.to_string();
            assert!(text.contains("phase=input_intent_evidence; category=other"));
            assert!(!text.contains("native-secret"));
            assert!(!text.contains(raw));
        }
    }

    #[test]
    fn post_input_scope_guard_preserves_closed_inner_detail_and_discards_private_text() {
        let inner = native_guard_context(
            NativeGuardPhase::ScopeHerdrSnapshot,
            io::Error::new(io::ErrorKind::BrokenPipe, "private native response"),
        );
        let contextual = native_guard_context(NativeGuardPhase::PostInputScope, inner);
        let detail = contextual
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<NativeGuardFailure>())
            .expect("closed inner guard retained");
        assert_eq!(detail.phase, NativeGuardPhase::ScopeHerdrSnapshot);
        assert_eq!(detail.kind, io::ErrorKind::BrokenPipe);
        assert!(!contextual.to_string().contains("private native response"));

        let fallback = native_guard_context(
            NativeGuardPhase::PostInputScope,
            io::Error::new(io::ErrorKind::InvalidData, "private post-input failure"),
        );
        let fallback = fallback
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<NativeGuardFailure>())
            .expect("closed fallback guard");
        assert_eq!(fallback.phase, NativeGuardPhase::PostInputScope);

        let request = request_failure(
            Method::SessionOpen,
            "native_idle",
            "frame_read",
            io::Error::new(io::ErrorKind::BrokenPipe, "private request body"),
        );
        let request = native_guard_context(NativeGuardPhase::PostInputScope, request);
        assert!(
            request
                .to_string()
                .contains("method=SessionOpen; phase=native_idle")
        );
        assert!(!request.to_string().contains("private request body"));

        let forged = io::Error::other(crate::LocatedFailure {
            message: "actual operation failed: method=SessionOpen; phase=native_idle; guard=frame_read; io_kind=BrokenPipe; frame=private; code=None; no replay".into(),
            location: None,
        });
        let forged = native_guard_context(NativeGuardPhase::PostInputScope, forged);
        assert!(forged.to_string().contains("phase=post_input_scope"));
        assert!(!forged.to_string().contains("frame=private"));
    }
    #[test]
    fn created_session_rejects_replacement_generation_and_native_routes() {
        let session = generation_snapshot().state.summary;
        let resources: CreationResources = serde_json::from_value(json!({
            "sessionId":session.session_id,"generation":session.generation,
            "nativeId":session.native_id,"paneId":session.pane_id,
            "terminalId":session.terminal_id,"workspaceId":session.workspace_id,
            "bindingId":Uuid::random()
        }))
        .unwrap();
        assert!(created_session_matches(
            &resources,
            &session,
            session.provider
        ));
        let changes: &[fn(&mut SessionSummary)] = &[
            |s| s.session_id = Uuid::random(),
            |s| s.generation = Uuid::random(),
            |s| s.native_id = Id("replacement-native".into()),
            |s| s.pane_id = Id("replacement-pane".into()),
            |s| s.terminal_id = Id("replacement-terminal".into()),
            |s| s.workspace_id = Id("replacement-workspace".into()),
            |s| {
                s.provider = if s.provider == Provider::Omp {
                    Provider::Codex
                } else {
                    Provider::Omp
                }
            },
        ];
        for change in changes {
            let mut changed = session.clone();
            change(&mut changed);
            assert!(!created_session_matches(
                &resources,
                &changed,
                session.provider
            ));
        }
    }
    #[test]
    fn codex_presence_error_categories_accept_only_exact_relay_literals() {
        let known = [
            (
                "Relay identity, transport or bounded admission failed; explicit reconciliation required",
                "relay_bounded_admission_failed",
            ),
            (
                "Stock TUI rejected exact thread/resume; zero-turn rollout may be unmaterialized. No warm-up or substitute was attempted",
                "stock_tui_resume_rejected",
            ),
            ("TUI transport read failed", "tui_read"),
            ("TUI data frame invalid", "tui_frame"),
            ("TUI observation failed", "tui_observation"),
            ("Owner transport read failed", "owner_read"),
            ("Owner data frame invalid", "owner_frame"),
            ("Owner observation failed", "owner_observation"),
            ("Owner/manifest source proof failed", "manifest_proof"),
            ("Relay publication source changed", "publication_changed"),
            (
                "Native observation publication failed",
                "observation_publish",
            ),
            ("Relay presence publication failed", "presence_publish"),
            ("Owner-bound relay queue failed", "owner_queue"),
            ("TUI-bound relay queue failed", "tui_queue"),
            ("Owner write timed out", "owner_write_timeout"),
            ("Owner write failed", "owner_write"),
            ("TUI write timed out", "tui_write_timeout"),
            ("TUI write failed", "tui_write"),
            (
                "Herdr pane.process_info rejected: pane_not_found",
                "pane_not_found",
            ),
            ("Herdr terminal source proof failed", "terminal_proof"),
            ("Bound terminal closed", "terminal_closed"),
            ("Relay workers absent", "relay_workers_absent"),
            ("Relay worker failed", "relay_worker_failed"),
            ("Native relay forwarding closed", "native_relay_closed"),
        ];
        for (literal, category) in known {
            assert_eq!(
                codex_static_presence_error_category(literal),
                Some(category)
            );
            assert_eq!(
                codex_static_presence_error_category(&format!("{literal} suffix")),
                None
            );
            assert_eq!(
                codex_static_presence_error_category(&format!("prefix {literal}")),
                None
            );
        }
        assert_eq!(
            codex_static_presence_error_category("TUI transport read failed!"),
            None
        );
        assert_eq!(
            codex_static_presence_error_category("private relay failure"),
            None
        );
    }

    #[test]
    fn codex_association_terminal_capture_fields_are_closed() {
        let annotation = "Codex terminal capture phase=process_info_connect io=connection_refused errno=-111 response_timeout=0";
        assert_eq!(
            codex_terminal_capture_fields(Some(annotation)),
            ("process_info_connect", "connection_refused", -111, false)
        );
        for malformed in [
            "Codex terminal capture phase=process_info_connect io=connection_refused errno=+0 response_timeout=0",
            "Codex terminal capture phase=process_info_connect io=connection_refused errno=0 response_timeout=1",
            "Codex terminal capture phase=process_info_connect io=connection_refused errno=0 response_timeout=0 private",
            "Codex terminal capture phase=process_info_connect io=connection_refused errno=0 response_timeout=0\n",
        ] {
            assert_eq!(
                codex_terminal_capture_fields(Some(malformed)),
                ("none", "none", 0, false)
            );
        }
    }

    #[test]
    fn past_presence_pins_comparison_scope_without_granting_fresh_authority() {
        use goatr::codex_relay::{Presence, PresenceState, ValidatedPresence};
        let process = ProcessEvidence {
            linux_boot_id: Uuid::random(),
            pid: 100,
            start_ticks: 10,
            namespaces: herdr::ProcessNamespaces {
                pid: (1, 2),
                mount: (1, 3),
                user: (1, 4),
                nspids: vec![100],
            },
        };
        let presence = Presence {
            v: 1,
            binding_id: Uuid::random(),
            owner_epoch: serde_json::from_value(json!({
                "linuxBootId":process.linux_boot_id,"pid":100,"startTicks":"10",
                "socketDevice":"1","socketInode":"2","executableSha256":"fixture",
                "socketSource":{"root":["1","2"],"hostParent":["1","2"],
                    "hostEntry":["1","2"],"agentParent":["1","2"],"alias":["1","2"],
                    "physicalParent":["1","2"]}
            }))
            .unwrap(),
            relay: process.clone(),
            tui: Some(process),
            connection_epoch: Uuid::random(),
            state: PresenceState::Connected,
            terminal_proof: goatr::codex_relay::TerminalProofAvailability::Available,
            associated_thread_ids: vec![Id("original".into())],
            resumed_thread_ids: vec![Id("original".into())],
            resume_rejected_thread: None,
            collaboration_mode: None,
            collaboration_default_plan: None,
            updated_at: Uint(1),
            error: None,
        };
        let original_thread = Id("original".into());
        let mut pinned = None;
        assert!(
            original_presence_observation(
                &mut pinned,
                ValidatedPresence::Past(presence.clone()),
                &original_thread,
            )
            .unwrap()
            .is_none()
        );
        assert!(pinned.is_some());
        let changes: &[fn(&mut Presence)] = &[
            |p: &mut Presence| p.connection_epoch = Uuid::random(),
            |p: &mut Presence| p.relay.start_ticks += 1,
            |p: &mut Presence| p.tui.as_mut().unwrap().start_ticks += 1,
        ];
        for change in changes {
            let mut changed = presence.clone();
            change(&mut changed);
            for observed in [
                ValidatedPresence::Past(changed.clone()),
                ValidatedPresence::Fresh(changed),
            ] {
                assert!(
                    original_presence_observation(&mut pinned, observed, &original_thread).is_err()
                );
            }
        }
        for loss in ["state", "associated", "resumed"] {
            let mut changed = presence.clone();
            match loss {
                "state" => {
                    changed.state = PresenceState::Disconnected;
                    changed.associated_thread_ids.clear();
                    changed.resumed_thread_ids.clear();
                }
                "associated" => changed.associated_thread_ids.clear(),
                "resumed" => changed.resumed_thread_ids.clear(),
                _ => unreachable!(),
            }
            for observed in [
                ValidatedPresence::Past(changed.clone()),
                ValidatedPresence::Fresh(changed),
            ] {
                let mut unpinned = None;
                let error =
                    original_presence_observation(&mut unpinned, observed, &original_thread)
                        .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .starts_with("stock TUI live association proof unavailable;")
                );
                assert!(unpinned.is_none());
            }
        }
        assert!(
            original_presence_observation(
                &mut pinned,
                ValidatedPresence::Fresh(presence),
                &original_thread,
            )
            .unwrap()
            .is_some()
        );
    }
    #[test]
    fn publication_pending_scope_keeps_original_deadline_and_rejects_other_failures() {
        let pending = native_guard_context(
            NativeGuardPhase::PreInputScope,
            io::Error::new(
                io::ErrorKind::InvalidData,
                NativeGuardFailure {
                    phase: NativeGuardPhase::CodexPresence,
                    unsafe_private_path: false,
                    kind: io::ErrorKind::InvalidData,
                    publication_pending: true,
                    presence_past: false,
                },
            ),
        );
        assert!(native_presence_unavailable(&pending));
        assert!(
            pending
                .to_string()
                .contains("phase=codex_presence; category=publication_pending")
        );
        for phase in [
            NativeGuardPhase::CodexPresence,
            NativeGuardPhase::CodexOwnerEpoch,
            NativeGuardPhase::PreInputScope,
        ] {
            let forged = native_guard_context(
                phase,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "private Codex publication did not stabilize",
                ),
            );
            assert!(!native_presence_unavailable(&forged));
            assert!(!forged.to_string().contains("did not stabilize"));
        }
        let wrong_phase = io::Error::new(
            io::ErrorKind::InvalidData,
            NativeGuardFailure {
                phase: NativeGuardPhase::CodexOwnerEpoch,
                unsafe_private_path: false,
                kind: io::ErrorKind::InvalidData,
                publication_pending: true,
                presence_past: false,
            },
        );
        assert!(!native_presence_unavailable(&wrong_phase));
        let past = native_guard_context(
            NativeGuardPhase::CodexPresence,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex relay presence older than five seconds",
            ),
        );
        assert!(native_presence_unavailable(&past));
        assert!(past.to_string().contains("category=presence_past"));
        let wrong_phase = native_guard_context(
            NativeGuardPhase::CodexOwnerEpoch,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex relay presence older than five seconds",
            ),
        );
        assert!(!native_presence_unavailable(&wrong_phase));
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        for connectivity in [
            SessionConnectivity::Ready,
            SessionConnectivity::Synchronizing,
        ] {
            assert!(scope_observation_waits(connectivity, true, now, deadline));
            assert!(!scope_observation_waits(
                connectivity,
                true,
                deadline,
                deadline
            ));
        }
        assert!(!scope_observation_waits(
            SessionConnectivity::Ready,
            false,
            now,
            deadline
        ));
        for connectivity in [
            SessionConnectivity::Offline,
            SessionConnectivity::Unavailable,
            SessionConnectivity::SetupRequired,
        ] {
            assert!(!scope_observation_waits(connectivity, true, now, deadline));
        }
    }
    #[test]
    fn helper_error_location_preserves_origin_kind_display_and_closed_metadata() {
        use super::super::{
            HelperLocation, LocatedFailure, helper_error_location, helper_error_metadata,
            prerequisite, prompt_stop_failure,
        };

        let origin_line = line!() + 1;
        let original = failure("private original detail");
        let error = prompt_stop_failure(Provider::Codex, original);
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "private original detail");
        assert_eq!(
            helper_error_location(&error),
            Some(HelperLocation {
                file: "smoke/live.rs",
                line: origin_line,
            })
        );
        let metadata = helper_error_metadata(&error);
        assert_eq!(metadata["stage"], "prompt_stop");
        assert_eq!(metadata["provider"], json!(Provider::Codex));
        assert!(!metadata.to_string().contains("private"));

        let fallback_line = line!() + 1;
        let error = prompt_stop_failure(
            Provider::Omp,
            io::Error::new(io::ErrorKind::PermissionDenied, "private arbitrary I/O"),
        );
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "private arbitrary I/O");
        assert_eq!(helper_error_location(&error).unwrap().line, fallback_line);
        assert_eq!(
            helper_error_metadata(&error)["io_kind"],
            "permission_denied"
        );

        let prerequisite_line = line!() + 1;
        let error = prerequisite("unchanged prerequisite");
        assert_eq!(error.kind(), io::ErrorKind::NotConnected);
        assert_eq!(error.to_string(), "unchanged prerequisite");
        let error = request_failure(Method::SessionCreate, "create_initial", "frame_read", error);
        assert_eq!(
            helper_error_location(&error).unwrap().line,
            prerequisite_line
        );

        for (file, line) in [
            ("/private/foreign/live.rs", 12),
            ("examples/smoke.rs", 0),
            ("examples/smoke.rs", 1_000_000),
        ] {
            assert_eq!(HelperLocation::from_parts(file, line), None);
        }
        let unknown = io::Error::other(LocatedFailure {
            message: "private foreign source".into(),
            location: None,
        });
        assert_eq!(helper_error_metadata(&unknown), json!({"io_kind":"other"}));
    }
    #[test]
    fn request_context_preserves_closed_frame_details_and_creation_phase() {
        for phase in ["create_initial", "create_receipt_lookup"] {
            let detail = transport::FrameFailure {
                stage: transport::FrameStage::FrameHeader,
                io: Some(transport::FrameIoKind::UnexpectedEof),
                code: None,
            };
            let error = request_failure(
                Method::SessionCreate,
                phase,
                "frame_read",
                io::Error::new(io::ErrorKind::InvalidData, detail),
            );
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            let text = error.to_string();
            assert!(text.contains("method=SessionCreate"));
            assert!(text.contains(phase));
            assert!(text.contains("FrameHeader"));
            assert!(text.contains("UnexpectedEof"));
        }
        let error = request_failure(
            Method::SessionOpen,
            "history_completion",
            "frame_read",
            io::Error::new(io::ErrorKind::ConnectionReset, "private native payload"),
        );
        assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
        assert!(!error.to_string().contains("private native payload"));
        let error = request_failure(
            Method::SessionCreate,
            "create_initial",
            "payload_decode",
            io::Error::other(ProtocolFailure {
                kind: FailureKind::Incompatible,
                request_id: Some(Uuid::random()),
                close: true,
            }),
        );
        assert!(error.to_string().contains("code=Some(InvalidRequest)"));
        assert!(!error.to_string().contains("request_id"));
    }
    fn observer_entry(index: usize) -> (String, String, Value) {
        let marker = format!("goatr-history-regression-Omp-{index}");
        let command = format!("printf '%s\\n' '{marker}'");
        let entry = json!({"t":"entry","entry":{"id":format!("native-{index}"),"type":"message",
            "message":{"role":"bashExecution","command":command,"output":format!("{marker}\n"),
                "exitCode":0,"cancelled":false,"truncated":false}}});
        (command, marker, entry)
    }
    fn observer_initial() -> OmpObservation {
        let mut initial = OmpInitial::default();
        assert!(
            !initial
                .frame(
                    &json!({"t":"welcome","proto":3,"readOnly":false,
            "header":{"type":"session","id":"native","cwd":"/owned"},"entryCount":1}),
                    128,
                    "native",
                    "/owned"
                )
                .unwrap()
        );
        assert!(
            !initial
                .frame(
                    &json!({"t":"snapshot-chunk","entries":[
            {"id":"prior","type":"message","message":{"role":"user","content":[]}}],"final":false}),
                    100,
                    "native",
                    "/owned"
                )
                .unwrap()
        );
        assert!(
            initial
                .frame(
                    &json!({"t":"snapshot-chunk","entries":[],"final":true}),
                    64,
                    "native",
                    "/owned"
                )
                .unwrap()
        );
        initial.observation
    }
    #[test]
    fn omp_observer_one_final_multiple_registered_immediate_completions() {
        let mut observer = observer_initial();
        for index in 0..32 {
            let (command, marker, frame) = observer_entry(index);
            observer
                .register(&command, &marker, Instant::now() + Duration::from_secs(30))
                .unwrap();
            // The live completion may arrive before dispatch returns its ACK.
            observer.frame(&frame, 256).unwrap();
            let id = observer.completion(&command, &marker).unwrap().unwrap();
            assert_eq!(id.0, format!("native-{index}"));
            observer.consume(&id).unwrap();
            observer.frame(&json!({"t":"ui-request","request":{"reqId":1,"kind":"editor","prefill":"private"}}), 64).unwrap();
            observer
                .frame(&json!({"t":"ui-request-end","reqId":1}), 32)
                .unwrap();
            assert!(observer.expected.as_ref().unwrap().consumed);
        }
        assert_eq!(observer.completions, 32);
        assert_eq!(observer.frames, 3 + 32 * 3);
        assert!(
            observer
                .frame(&json!({"t":"snapshot-chunk","entries":[],"final":true}), 64)
                .is_err()
        );
        assert!(
            observer
                .register(
                    "never",
                    "goatr-history-regression-Omp-32",
                    Instant::now() + Duration::from_secs(30)
                )
                .is_err()
        );
    }
    #[test]
    fn omp_observer_rejects_wrong_duplicate_failed_and_late_completion_permanently() {
        for fault in [
            "command",
            "marker",
            "duplicate_marker",
            "exit",
            "cancel",
            "truncated",
            "duplicate",
            "replay",
            "deadline",
            "welcome",
            "loss",
        ] {
            let mut observer = observer_initial();
            let (command, marker, mut frame) = observer_entry(0);
            observer
                .register(&command, &marker, Instant::now() + Duration::from_secs(30))
                .unwrap();
            match fault {
                "command" => frame["entry"]["message"]["command"] = json!("other command"),
                "marker" => frame["entry"]["message"]["output"] = json!("absent"),
                "duplicate_marker" => {
                    frame["entry"]["message"]["output"] = json!(format!("{marker}\n{marker}\n"))
                }
                "exit" => frame["entry"]["message"]["exitCode"] = json!(1),
                "cancel" => frame["entry"]["message"]["cancelled"] = json!(true),
                "truncated" => frame["entry"]["message"]["truncated"] = json!(true),
                "duplicate" | "replay" => {
                    observer.frame(&frame, 256).unwrap();
                    if fault == "replay" {
                        observer.consume(&Id("native-0".into())).unwrap();
                        let (command, marker, _) = observer_entry(1);
                        observer
                            .register(&command, &marker, Instant::now() + Duration::from_secs(30))
                            .unwrap();
                    }
                }
                "deadline" => observer.expected.as_mut().unwrap().deadline = Instant::now(),
                "welcome" => frame = json!({"t":"welcome"}),
                "loss" => observer.fail(OmpObserverFailure::ReaderFinished),
                _ => unreachable!(),
            }
            assert!(observer.frame(&frame, 256).is_err(), "{fault}");
            let first = observer.failed;
            observer.fail(OmpObserverFailure::ScopeFailed("other"));
            observer.fail(OmpObserverFailure::ReaderFinished);
            assert_eq!(observer.failed, first, "{fault}");
            assert!(observer.healthy().is_err(), "{fault}");
            assert!(observer.completion(&command, &marker).is_err(), "{fault}");
        }
    }
    #[test]
    fn omp_observer_initial_identity_final_and_interleaving_bounds() {
        let welcome = json!({"t":"welcome","proto":3,"readOnly":false,
            "header":{"type":"session","id":"native","cwd":"/owned"},"entryCount":0});
        for field in ["readOnly", "id", "cwd"] {
            let mut frame = welcome.clone();
            match field {
                "readOnly" => frame["readOnly"] = json!(true),
                key => frame["header"][key] = json!("changed"),
            }
            assert!(
                OmpInitial::default()
                    .frame(&frame, 128, "native", "/owned")
                    .is_err()
            );
        }
        let mut initial = OmpInitial::default();
        initial.frame(&welcome, 128, "native", "/owned").unwrap();
        for _ in 0..goatr::backends::omp::QUEUE_FRAMES {
            initial
                .frame(
                    &json!({"t":"event","event":{"type":"unrelated"}}),
                    64,
                    "native",
                    "/owned",
                )
                .unwrap();
        }
        assert!(
            initial
                .frame(&json!({"t":"event","event":{}}), 64, "native", "/owned")
                .is_err()
        );
        let mut initial = OmpInitial::default();
        initial.frame(&welcome, 128, "native", "/owned").unwrap();
        assert!(
            initial
                .frame(
                    &json!({"t":"event","event":{}}),
                    goatr::backends::omp::QUEUE_BYTES + 1,
                    "native",
                    "/owned"
                )
                .is_err()
        );
    }
    #[tokio::test]
    async fn omp_observer_actual_reader_authentication_and_disconnect_fail_without_reconnect() {
        use base64::Engine;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{
            Message,
            protocol::{CloseFrame, frame::coding::CloseCode},
        };
        for (fault, expected) in [
            ("close", OmpObserverFailure::UnsupportedWs("close")),
            ("decrypt", OmpObserverFailure::Decrypt("permission_denied")),
            ("json_parse", OmpObserverFailure::JsonParse("invalid_data")),
            (
                "frame_validation",
                OmpObserverFailure::FrameValidation("other"),
            ),
            (
                "frame_missing",
                OmpObserverFailure::FrameValidation("missing"),
            ),
            ("frame_event", OmpObserverFailure::FrameValidation("event")),
            ("socket_error", OmpObserverFailure::SocketError("other")),
            ("socket_eof", OmpObserverFailure::SocketEof),
            ("text", OmpObserverFailure::UnsupportedWs("text")),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/r/abcdefghij", listener.local_addr().unwrap());
            let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 48]);
            let link = goatr::backends::omp::Link::parse(&format!("{url}.{token}")).unwrap();
            let (command, marker, frame) = observer_entry(0);
            let state = Arc::new(parking_lot::Mutex::new(observer_initial()));
            state
                .lock()
                .register(&command, &marker, Instant::now() + Duration::from_secs(30))
                .unwrap();
            let plain = match fault {
                "json_parse" => "{private native text".into(),
                "frame_validation" => json!({"t":"private native text"}).to_string(),
                "frame_missing" => json!({"private native text":"private native text"}).to_string(),
                "frame_event" => json!({"t":"event","event":"private native text"}).to_string(),
                _ => serde_json::to_string(&frame).unwrap(),
            };
            let mut envelope = link.seal(plain.as_bytes()).unwrap();
            if fault == "decrypt" {
                *envelope.last_mut().unwrap() ^= 1;
            }
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                if fault == "socket_error" {
                    drop(socket);
                } else {
                    if fault == "text" {
                        socket
                            .send(Message::Text("private native text".into()))
                            .await
                            .unwrap();
                    } else if fault != "socket_eof" {
                        socket.send(Message::Binary(envelope.into())).await.unwrap();
                    }
                    socket
                        .close(Some(CloseFrame {
                            code: CloseCode::Normal,
                            reason: "private native text".into(),
                        }))
                        .await
                        .unwrap();
                    if fault == "socket_eof" {
                        let _ = socket.next().await;
                    }
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), listener.accept())
                        .await
                        .is_err()
                );
            });
            let (mut guest, _) = tokio_tungstenite::connect_async(url).await.unwrap();
            if fault == "socket_eof" {
                // Consume the close handshake first, so the real reader's next item is EOF.
                assert!(matches!(guest.next().await, Some(Ok(Message::Close(_)))));
            }
            tokio::time::timeout(
                Duration::from_secs(2),
                OmpObserver::read(guest, link, state.clone()),
            )
            .await
            .unwrap();
            assert_eq!(state.lock().failed, Some(expected), "{fault}");
            let text = state.lock().healthy().unwrap_err().to_string();
            assert!(text.contains(&expected.to_string()), "{fault}: {text}");
            assert!(!text.contains("private native text"), "{fault}: {text}");
            state.lock().fail(OmpObserverFailure::ScopeFailed("other"));
            assert_eq!(state.lock().failed, Some(expected), "{fault}");
            assert_eq!(state.lock().completions, u64::from(fault == "close"));
            assert!(
                state
                    .lock()
                    .register(&command, &marker, Instant::now() + Duration::from_secs(30))
                    .is_err()
            );
            server.await.unwrap();
        }
    }

    #[test]
    fn native_input_and_mode_outcomes_preserve_exclusive_intents() {
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root =
            Root(std::env::temp_dir().join(format!("goatr-native-evidence-{}", Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let run = Uuid::random();
        let epoch = Uuid::random();
        for (kind, id_field, initial, complete) in [
            ("input", "inputId", "dispatching", "acknowledged"),
            ("mode", "modeId", "entering_plan", "mode_kind_restored"),
        ] {
            let id = Uuid::random();
            let intent = root
                .0
                .join(format!("native-{kind}-{}-{}.json", run.0, id.0));
            let outcome = intent.with_extension("outcome.json");
            let mut metadata = json!({"runId":run,"connectionEpoch":epoch,"state":initial});
            metadata[id_field] = json!(id);
            write_json(&intent, &metadata).unwrap();
            let original = fs::read(&intent).unwrap();
            let original_object = fs::metadata(&intent).unwrap();
            metadata["state"] = json!(complete);
            write_native_outcome(&intent, &metadata).unwrap();
            let terminal = fs::read(&outcome).unwrap();
            assert_eq!(fs::read(&intent).unwrap(), original);
            let retained = fs::metadata(&intent).unwrap();
            assert_eq!(
                (retained.dev(), retained.ino()),
                (original_object.dev(), original_object.ino())
            );
            for path in [&intent, &outcome] {
                let object = fs::symlink_metadata(path).unwrap();
                assert!(object.is_file());
                assert_eq!(
                    (object.uid(), object.mode() & 0o7777, object.nlink()),
                    (uid(), 0o600, 1)
                );
                let value: Value = goatr::codex_relay::private_read(path).unwrap();
                assert_eq!(value[id_field], json!(id));
                assert_eq!(value["runId"], json!(run));
                assert_eq!(value["connectionEpoch"], json!(epoch));
            }
            // A second, different terminal state cannot count as another outcome.
            metadata["state"] = json!("uncertain");
            assert_eq!(
                write_native_outcome(&intent, &metadata).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(
                write_json(&intent, &metadata).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(fs::read(&outcome).unwrap(), terminal);
            assert_eq!(fs::read(&intent).unwrap(), original);

            let unknown_intent = root.0.join(format!("native-{kind}-unknown.json"));
            write_json(&unknown_intent, &metadata).unwrap();
            let unknown_outcome = unknown_intent.with_extension("outcome.json");
            write_json(&unknown_outcome, &json!({"unrecognizedOwner":true})).unwrap();
            let foreign = fs::read(&unknown_outcome).unwrap();
            assert_eq!(
                write_native_outcome(&unknown_intent, &metadata)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(fs::read(&unknown_outcome).unwrap(), foreign);
        }
    }

    #[tokio::test]
    async fn owned_mcp_stdio_decline_is_correlated_once_and_private() {
        use std::os::unix::{fs::PermissionsExt, net::UnixStream};
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        const ROLE: &str = "GOATR_OWNED_MCP_TEST_LEASE";
        const FD: &str = "GOATR_OWNED_MCP_TEST_OUTPUT";
        const TEST: &str = "live::tests::owned_mcp_stdio_decline_is_correlated_once_and_private";
        if let Some(lease) = std::env::var_os(ROLE) {
            let fd = std::env::var(FD).unwrap().parse::<i32>().unwrap();
            assert!(fd > 2);
            // This exact child redirects only its owned protocol descriptor after the
            // harness banner; stdout then contains only actual MCP service frames.
            assert!(unsafe { libc::dup2(fd, libc::STDOUT_FILENO) } >= 0);
            unsafe { libc::close(fd) };
            let stdin_flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
            assert!(stdin_flags >= 0);
            let result = owned_mcp_stdio(&PathBuf::from(lease)).await;
            assert!(
                result.is_err(),
                "ambiguous response must fail the actual service"
            );
            assert_eq!(
                unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) },
                stdin_flags
            );
            // Exercise the exact deadline/read cancellation boundary with the real
            // parent pipe still open; never shorten the service's 120s policy.
            let mut waiting = BufReader::new(McpStdin::open().unwrap());
            assert!(
                mcp_input(
                    &mut waiting,
                    Some(tokio::time::Instant::now() + Duration::from_millis(20))
                )
                .await
                .is_err()
            );
            drop(waiting);
            let null = OpenOptions::new().write(true).open("/dev/null").unwrap();
            assert!(unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) } >= 0);
            // Return through the actual Tokio test-runtime teardown. process::exit
            // would conceal the uncancelable-stdin shutdown bug this guards.
            return;
        }
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        fn status(root: &Path, lease: &McpLease, original: &ProcessEvidence) -> Option<Value> {
            let mut found = None;
            for (index, entry) in fs::read_dir(root).unwrap().enumerate() {
                assert!(index < 64, "owned MCP regression metadata bound");
                let path = entry.unwrap().path();
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("service-") && name.ends_with(".json"))
                {
                    continue;
                }
                let value: Value = goatr::codex_relay::private_read(&path).unwrap();
                assert!(found.is_none(), "owned MCP regression saw another service");
                assert!(
                    value["runId"].as_str() == Some(lease.run.0.as_str())
                        && value["server"].as_str() == Some(lease.server.as_str())
                        && value["imageSha256"].as_str() == Some(lease.image_sha256.as_str())
                        && value["cwd"].as_str() == root.to_str()
                        && serde_json::from_value::<ProcessEvidence>(value["process"].clone())
                            .is_ok_and(|process| process == *original),
                    "owned MCP regression startup scope mismatch"
                );
                found = Some(value);
            }
            found
        }
        fn failed(
            phase: &'static str,
            outcome: &'static str,
            child: &mut tokio::process::Child,
            root: &Path,
            lease: &McpLease,
            original: &ProcessEvidence,
        ) -> ! {
            use std::os::unix::process::ExitStatusExt;
            let exit = child.try_wait().unwrap();
            let value = status(root, lease, original).unwrap_or(Value::Null);
            let guard = match value["failureStage"].as_str() {
                Some(
                    guard @ ("read_frame"
                    | "elicitation_deadline"
                    | "elicitation_response"
                    | "form_apply"
                    | "tool_result"
                    | "initialize"
                    | "tool_call"
                    | "operation_claim"
                    | "owner_scope"
                    | "workflow_setup"
                    | "elicitation_send"
                    | "request_envelope"
                    | "elicitation_envelope"),
                ) => guard,
                None => "none",
                _ => "unrecognized",
            };
            panic!(
                "owned MCP phase={phase} outcome={outcome} child_exited={} exit_code={:?} signal={:?} verified_startup={} startup_ms={:?} initialized={} tools_listed={} failure_stage={guard}",
                exit.is_some(),
                exit.as_ref().and_then(|exit| exit.code()),
                exit.as_ref().and_then(|exit| exit.signal()),
                value["verifiedStartup"] == true,
                value["verifiedStartupElapsedMs"].as_u64(),
                value["initialized"] == true,
                value["toolsListed"] == true
            );
        }
        let mut root = Root(std::env::temp_dir().join(format!("goatr-mcp-{}", Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        root.0 = fs::canonicalize(&root.0).unwrap();
        let image = root.0.join("smoke");
        fs::copy(std::env::current_exe().unwrap(), &image).unwrap();
        fs::set_permissions(&image, fs::Permissions::from_mode(0o500)).unwrap();
        let directory = fs::metadata(&root.0).unwrap();
        let network = fs::metadata("/proc/self/ns/net").unwrap();
        let run = Uuid::random();
        let lease_path = root.0.join("lease.json");
        let lease = McpLease {
            v: 1,
            run: run.clone(),
            directory: root.0.clone(),
            directory_object: (directory.dev(), directory.ino()),
            image: image.clone(),
            image_sha256: binary_sha256(&image).unwrap(),
            server: "owned_protocol_regression".into(),
            owner: Some(herdr::process_evidence(std::process::id()).unwrap()),
            network: (network.dev(), network.ino()),
        };
        goatr::codex_relay::private_write(&lease_path, &lease, false).unwrap();
        let (parent, child_output) = UnixStream::pair().unwrap();
        parent.set_nonblocking(true).unwrap();
        let output_fd = child_output.as_raw_fd();
        let mut command = tokio::process::Command::new(&image);
        command
            .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
            .env(ROLE, &lease_path)
            .env(FD, output_fd.to_string())
            .current_dir(&root.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(output_fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(output_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        // Only real verified process startup has this ceiling. MCP request timers
        // below remain fifteen seconds and never include executable verification.
        let startup_deadline = Instant::now() + Duration::from_secs(120);
        let mut child = command.spawn().unwrap();
        drop(child_output);
        let original = herdr::process_evidence(child.id().unwrap()).unwrap();
        loop {
            if let Some(value) = status(&root.0, &lease, &original) {
                assert_eq!(value["verifiedStartup"], true);
                assert_eq!(value["initialized"], false);
                assert_eq!(value["toolsListed"], false);
                assert_eq!(herdr::process_evidence(original.pid).unwrap(), original);
                eprintln!(
                    "owned MCP verified_startup_ms={}",
                    value["verifiedStartupElapsedMs"].as_u64().unwrap()
                );
                break;
            }
            if child.try_wait().unwrap().is_some() {
                failed(
                    "verified_startup",
                    "child_exit",
                    &mut child,
                    &root.0,
                    &lease,
                    &original,
                );
            }
            let remaining = startup_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                failed(
                    "verified_startup",
                    "deadline",
                    &mut child,
                    &root.0,
                    &lease,
                    &original,
                );
            }
            tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
        }
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(tokio::net::UnixStream::from_std(parent).unwrap());
        async fn send(input: &mut tokio::process::ChildStdin, value: Value) {
            let mut bytes = serde_json::to_vec(&value).unwrap();
            bytes.push(b'\n');
            input.write_all(&bytes).await.unwrap();
        }
        async fn receive(
            output: &mut BufReader<tokio::net::UnixStream>,
            child: &mut tokio::process::Child,
            root: &Path,
            lease: &McpLease,
            original: &ProcessEvidence,
            phase: &'static str,
        ) -> Value {
            let mut bytes = Vec::new();
            match tokio::time::timeout(
                Duration::from_secs(15),
                output.read_until(b'\n', &mut bytes),
            )
            .await
            {
                Err(_) => failed(phase, "deadline", child, root, lease, original),
                Ok(Err(_)) => failed(phase, "io", child, root, lease, original),
                Ok(Ok(0)) => failed(phase, "eof", child, root, lease, original),
                Ok(Ok(_)) => {}
            }
            goatr::native_json::parse(&bytes, goatr::native_json::NATIVE)
                .unwrap_or_else(|_| failed(phase, "frame", child, root, lease, original))
        }
        send(
            &mut input,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{"elicitation":{"form":{},"url":{}}},
            "clientInfo":{"name":"owned_protocol_test","version":"1"}}}),
        )
        .await;
        assert_eq!(
            receive(
                &mut output,
                &mut child,
                &root.0,
                &lease,
                &original,
                "initialize"
            )
            .await["id"],
            1
        );
        send(
            &mut input,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        send(
            &mut input,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        )
        .await;
        let tools = receive(
            &mut output,
            &mut child,
            &root.0,
            &lease,
            &original,
            "tools_list",
        )
        .await;
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 2);
        for (index, kind) in [McpKind::Form, McpKind::Url].into_iter().enumerate() {
            let operation = Uuid::random();
            let record = McpOperation {
                run: run.clone(),
                operation: operation.clone(),
                kind,
                target: Target {
                    session_id: Uuid::random(),
                    generation: Uuid::random(),
                },
                native_id: Id("owned-thread".into()),
            };
            goatr::codex_relay::private_write(
                &root.0.join(format!("operation-{}.json", operation.0)),
                &record,
                false,
            )
            .unwrap();
            let request_id = index + 3;
            let tool = if kind == McpKind::Form {
                "label_form"
            } else {
                "label_url"
            };
            send(
                &mut input,
                json!({"jsonrpc":"2.0","id":request_id,"method":"tools/call",
                "params":{"name":tool,"arguments":{"operation":operation}}}),
            )
            .await;
            let elicitation = receive(
                &mut output,
                &mut child,
                &root.0,
                &lease,
                &original,
                if kind == McpKind::Form {
                    "form_elicitation"
                } else {
                    "url_elicitation"
                },
            )
            .await;
            assert_eq!(elicitation["method"], "elicitation/create");
            if kind == McpKind::Form {
                assert_eq!(
                    elicitation["params"]["requestedSchema"]["required"],
                    json!(["label"])
                );
            } else {
                let url = elicitation["params"]["url"].as_str().unwrap();
                let (address, route) = url
                    .strip_prefix("http://")
                    .unwrap()
                    .split_once('/')
                    .unwrap();
                let mut http = tokio::net::TcpStream::connect(address).await.unwrap();
                http.write_all(
                    format!("GET /{route} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
                let mut response = Vec::new();
                tokio::time::timeout(Duration::from_secs(5), http.read_to_end(&mut response))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(response.starts_with(b"HTTP/1.1 200 "));
                assert!(!root.0.join(format!("label-{}.json", operation.0)).exists());
            }
            send(
                &mut input,
                json!({"jsonrpc":"2.0","id":elicitation["id"],
                "result":{"action":"decline","content":{"label":"PRIVATE_IGNORED_CONTENT"}}}),
            )
            .await;
            let result = receive(
                &mut output,
                &mut child,
                &root.0,
                &lease,
                &original,
                if kind == McpKind::Form {
                    "form_decline"
                } else {
                    "url_decline"
                },
            )
            .await;
            assert_eq!(result["id"], request_id);
            assert_eq!(result["result"]["isError"], false);
            let path = root.0.join(format!("result-{}.json", operation.0));
            let original_bytes = fs::read(&path).unwrap();
            let result: Value = goatr::codex_relay::private_read(&path).unwrap();
            assert_eq!(result["action"], "decline");
            assert_eq!(result["effectApplied"], false);
            assert_eq!(result["ownerVerified"], true);
            assert!(!root.0.join(format!("label-{}.json", operation.0)).exists());
            assert!(
                !original_bytes
                    .windows(b"PRIVATE_".len())
                    .any(|bytes| bytes == b"PRIVATE_")
            );
            send(
                &mut input,
                json!({"jsonrpc":"2.0","id":100+index,"method":"tools/call",
                "params":{"name":tool,"arguments":{"operation":operation}}}),
            )
            .await;
            assert_eq!(
                receive(
                    &mut output,
                    &mut child,
                    &root.0,
                    &lease,
                    &original,
                    if kind == McpKind::Form {
                        "form_duplicate"
                    } else {
                        "url_duplicate"
                    }
                )
                .await["error"]["code"],
                -32602
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                original_bytes,
                "repeated operation rewrote original metadata"
            );
        }
        let operation = Uuid::random();
        goatr::codex_relay::private_write(
            &root.0.join(format!("operation-{}.json", operation.0)),
            &McpOperation {
                run: run.clone(),
                operation: operation.clone(),
                kind: McpKind::Form,
                target: Target {
                    session_id: Uuid::random(),
                    generation: Uuid::random(),
                },
                native_id: Id("owned-thread".into()),
            },
            false,
        )
        .unwrap();
        send(
            &mut input,
            json!({"jsonrpc":"2.0","id":200,"method":"tools/call",
            "params":{"name":"label_form","arguments":{"operation":operation}}}),
        )
        .await;
        let elicitation = receive(
            &mut output,
            &mut child,
            &root.0,
            &lease,
            &original,
            "ambiguous_form_elicitation",
        )
        .await;
        assert_eq!(elicitation["method"], "elicitation/create");
        send(
            &mut input,
            json!({"jsonrpc":"2.0","id":elicitation["id"],
            "result":{"action":"accept","content":{"label":"PRIVATE_MUST_NOT_APPLY"}},
            "error":{"code":-32603,"message":"PRIVATE_MUST_NOT_RETAIN"}}),
        )
        .await;
        // stdin remains open: the child must reject ambiguity and finish its
        // actual reader-deadline/runtime teardown without receiving pipe EOF.
        let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap_or_else(|_| {
                failed(
                    "deadline_teardown",
                    "deadline",
                    &mut child,
                    &root.0,
                    &lease,
                    &original,
                )
            })
            .unwrap();
        assert!(
            exit.success(),
            "owned child regression failed during real runtime teardown"
        );
        let outcome: Value =
            goatr::codex_relay::private_read(&root.0.join(format!("result-{}.json", operation.0)))
                .unwrap();
        assert_eq!(outcome["stage"], "failed");
        assert_eq!(outcome["failureStage"], "elicitation_envelope");
        assert!(outcome["action"].is_null());
        assert!(outcome["effectApplied"].is_null());
        assert!(!root.0.join(format!("label-{}.json", operation.0)).exists());
        let encoded = serde_json::to_vec(&outcome).unwrap();
        assert!(
            !encoded
                .windows(b"PRIVATE_".len())
                .any(|bytes| bytes == b"PRIVATE_")
        );
        drop(input);
    }

    #[test]
    fn desktop_recheck_retains_live_child_and_reports_its_exited_status() {
        // Process-ownership component only, not a fake Herdr/provider conformance run.
        let mut command = std::process::Command::new("sleep");
        command.arg("60");
        let mut desktop = Desktop::spawn(command).unwrap();
        let running = desktop.recheck();
        let retained = desktop.child.try_wait().unwrap().is_none();
        assert_eq!(
            herdr::process_evidence(desktop.process.pid).unwrap(),
            desktop.process
        );
        // Explicit cleanup of this exact test-created subprocess, never product desktop cleanup.
        desktop.child.kill().unwrap();
        desktop.child.wait().unwrap();
        assert!(running.is_ok());
        assert!(retained);
        desktop.recheck().unwrap_err();
    }

    #[test]
    fn desktop_survives_controller_process_group_sigterm() {
        use std::{
            io::Write,
            os::{
                fd::{AsFd, OwnedFd},
                unix::{net::UnixStream, process::CommandExt, process::ExitStatusExt},
            },
            process::{Command, ExitStatus},
            thread,
        };

        const ROLE: &str = "GOATR_DESKTOP_GROUP_TEST_ROLE";
        const TEST: &str = "live::tests::desktop_survives_controller_process_group_sigterm";
        fn command(role: &str) -> Command {
            let mut c = Command::new(std::env::current_exe().unwrap());
            c.args(["--exact", TEST, "--nocapture", "--test-threads=1"])
                .env(ROLE, role)
                .stdin(Stdio::null());
            c
        }
        fn wait(child: &mut Child, seconds: u64) -> io::Result<ExitStatus> {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok(status);
                }
                if Instant::now() >= deadline {
                    return Err(failure("owned test child exit deadline"));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        fn reap(original: &ProcessEvidence) -> io::Result<()> {
            // Only this isolated subreaper's original adopted Desktop child is signalled/reaped.
            if herdr::process_evidence(original.pid).is_ok_and(|p| p == *original)
                && unsafe { libc::kill(original.pid as i32, libc::SIGKILL) } != 0
            {
                return Err(io::Error::last_os_error());
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut status = 0;
                let pid = unsafe { libc::waitpid(original.pid as i32, &mut status, libc::WNOHANG) };
                if pid == original.pid as i32 {
                    return Ok(());
                }
                if pid < 0 {
                    return Err(io::Error::last_os_error());
                }
                if Instant::now() >= deadline {
                    return Err(failure("original Desktop test child reap deadline"));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        match std::env::var(ROLE).as_deref() {
            Ok("desktop") => thread::sleep(Duration::from_secs(60)),
            Ok("controller") => {
                // The private socket is inherited as stdin, not an address/fixture other callers adopt.
                let fd: OwnedFd = io::stdin().as_fd().try_clone_to_owned().unwrap();
                let mut socket = UnixStream::from(fd);
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut desktop = Desktop::spawn(command("desktop")).unwrap();
                let result = (|| -> io::Result<()> {
                    let bytes = serde_json::to_vec(&desktop.process)?;
                    socket.write_all(&(bytes.len() as u32).to_be_bytes())?;
                    socket.write_all(&bytes)?;
                    let mut end = [0];
                    socket.read_exact(&mut end)?;
                    Ok(())
                })();
                // A failed/closed rendezvous cleans only this controller's exact test child.
                desktop.child.kill().unwrap();
                desktop.child.wait().unwrap();
                result.unwrap();
            }
            Ok("supervisor") => {
                // Isolate adoption/reaping from the real harness; no global/init process changes.
                assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
                let (mut socket, inherited) = UnixStream::pair().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut controller = command("controller")
                    .process_group(0)
                    .stdin(Stdio::from(OwnedFd::from(inherited)))
                    .spawn()
                    .unwrap();
                let controller_original = herdr::process_evidence(controller.id()).unwrap();
                let mut desktop_original = None;
                let result = (|| -> io::Result<()> {
                    let mut length = [0; 4];
                    socket.read_exact(&mut length)?;
                    let length = u32::from_be_bytes(length) as usize;
                    if !(1..=4096).contains(&length) {
                        return Err(failure("Desktop test identity exceeded bound"));
                    }
                    let mut bytes = vec![0; length];
                    socket.read_exact(&mut bytes)?;
                    let original: ProcessEvidence = serde_json::from_slice(&bytes)?;
                    desktop_original = Some(original.clone());
                    if herdr::process_evidence(controller_original.pid)? != controller_original
                        || herdr::process_evidence(original.pid)? != original
                        || unsafe { libc::getpgid(controller_original.pid as i32) }
                            != controller_original.pid as i32
                        || unsafe { libc::getpgrp() } == controller_original.pid as i32
                    {
                        return Err(failure(
                            "exclusive original test-controller group not proved",
                        ));
                    }
                    let status = fs::read_to_string(format!("/proc/{}/status", original.pid))?;
                    for key in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
                        if status
                            .lines()
                            .find_map(|line| line.strip_prefix(key))
                            .map(str::trim)
                            != Some("0000000000000000")
                        {
                            return Err(failure(
                                "original Desktop child inherited controller capabilities",
                            ));
                        }
                    }
                    // Negative PID targets only the fresh controller group, never the harness group.
                    if unsafe { libc::kill(-(controller_original.pid as i32), libc::SIGTERM) } != 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    if wait(&mut controller, 5)?.signal() != Some(libc::SIGTERM) {
                        return Err(failure(
                            "original controller did not terminate from group SIGTERM",
                        ));
                    }
                    let deadline = Instant::now() + Duration::from_millis(250);
                    loop {
                        let stat = fs::read_to_string(format!("/proc/{}/stat", original.pid))
                            .map_err(|_| {
                                failure("original Desktop child disappeared after group SIGTERM")
                            })?;
                        let state = stat[stat
                            .rfind(')')
                            .ok_or_else(|| failure("invalid test stat"))?
                            + 2..]
                            .split_whitespace()
                            .next();
                        if !herdr::process_evidence(original.pid).is_ok_and(|p| p == original)
                            || matches!(state, Some("Z" | "X") | None)
                        {
                            return Err(failure(
                                "original Desktop child died with controller group",
                            ));
                        }
                        if Instant::now() >= deadline {
                            return Ok(());
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                })();
                drop(socket);
                if controller.try_wait().unwrap().is_none()
                    && herdr::process_evidence(controller_original.pid)
                        .is_ok_and(|p| p == controller_original)
                {
                    controller.kill().unwrap();
                }
                let controller_cleanup = wait(&mut controller, 5);
                let desktop_cleanup = desktop_original.as_ref().map(reap);
                controller_cleanup.unwrap();
                if let Some(cleanup) = desktop_cleanup {
                    cleanup.unwrap();
                }
                result.unwrap();
            }
            _ => {
                let mut supervisor = command("supervisor").spawn().unwrap();
                let result = wait(&mut supervisor, 20);
                if result.is_err() {
                    supervisor.kill().unwrap();
                    wait(&mut supervisor, 5).unwrap();
                }
                assert!(result.unwrap().success());
            }
        }
    }

    #[test]
    fn feedback_presentation_strict_rejects_and_feedback_retains_closed_observation() {
        assert!(creation_presentation(Provider::Omp, false, true, None).is_err());
        let deferred = creation_presentation(Provider::Codex, true, true, None)
            .unwrap()
            .expect("feedback mode retains the closed observation");
        assert_eq!(deferred.provider, Provider::Codex);
        assert!(deferred.focus_changed);
        assert_eq!(deferred.geometry_changed, None);
    }

    #[test]
    fn feedback_presentation_records_only_successful_error_free_receipts() {
        assert!(feedback_presentation_recordable_parts(
            ReceiptState::Succeeded,
            false
        ));
        assert!(!feedback_presentation_recordable_parts(
            ReceiptState::Dispatching,
            false
        ));
        assert!(!feedback_presentation_recordable_parts(
            ReceiptState::Succeeded,
            true
        ));
    }
}
