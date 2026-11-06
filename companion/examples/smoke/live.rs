// SPDX-License-Identifier: GPL-3.0-or-later
//! Actual packaged/provider acceptance. Missing native prerequisites are blocked, never fixtures.
use super::{
    binary_sha256, boot, failure, identity, prerequisite, start, terminal_pair, write_json,
};
use clap::{Arg, ArgAction, ArgGroup, Command, value_parser};
use goatr::{
    config::{CodexOwner, Config, ConfiguredProject},
    herdr::{self, Client, ProcessEvidence, Snapshot},
    pairing::{self, PairInvite, hex, private_dir, private_file, random32, uid},
    protocol::*,
    storage::ServerEvidence,
    transport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
#[path = "physical.rs"]
mod physical;
const APP: &str = "io.github.trevarj.goatr.debug";
const NTFY_APK_SHA256:&str="b4baa6f668bd57ad5df3b4e0e769165dcf25c5ee19fdb04f78077ee819616f2a";
const NTFY_SIGNER_SHA256:&str="76e94d8e04706c5b99064c23210d8ca9910608abae8f3e972b41e388391646e0";
const OWNED_STOCK_CHECKPOINT_PAIR_LABEL:&str="Goatr owned stock checkpoint";
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
    adb_client_home: PathBuf,
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
/// One recorded console-auth home, used only by ADB/emulator children. No credential bytes
/// are opened, retained or reported; the ordinary exact console-name proof is unchanged.
pub(super) fn validate_adb_client_home(root:&Path,home:&Path)->io::Result<()> {
    if home==root||!home.starts_with(root)||!home.is_absolute()||fs::canonicalize(home)?!=home{return Err(failure("ADB client HOME is not the canonical recorded owned directory"));}
    private_dir(home,false)?;
    let metadata=fs::symlink_metadata(home.join(".emulator_console_auth_token"))?;
    if !metadata.file_type().is_file()||metadata.uid()!=uid()||metadata.mode()&0o7777!=0o600||metadata.nlink()!=1{return Err(failure("ADB console credential metadata is not a private original regular file"));}
    Ok(())
}
pub(super) fn configure_adb_client_home(command:&mut tokio::process::Command,root:&Path,home:&Path)->io::Result<()> {
    validate_adb_client_home(root,home)?;
    command.env("HOME",home);
    Ok(())
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
    validate_adb_client_home(&l.root,&l.adb_client_home)?;
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
    #[cfg(target_os = "linux")]
    // SAFETY: the hook only clears capabilities with async-signal-safe syscalls.
    unsafe {
        c.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
    }
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
    configure_adb_client_home(&mut c,&l.root,&l.adb_client_home)?;
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
        json!({"runId":l.run_id,"serial":l.serial,"adbClientHome":l.adb_client_home,"api":api.trim(),"abi":abi.trim(),"pageSize":page.trim(),"process":{"pid":l.process_identity.pid,"start":l.process_identity.start_ticks,"boot":l.process_identity.linux_boot_id,"executable":l.process_identity.executable},"limit":"4096-byte runtime is NOT a real16KiB pass"}),
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
    configure_adb_client_home(&mut command,&l.root,&l.adb_client_home)?;
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
    .env("HOME",&l.adb_client_home)
    .env("ANDROID_USER_HOME", &l.android_user_home)
    .env("ANDROID_EMULATOR_HOME", &l.android_user_home)
    .env("XDG_RUNTIME_DIR", &runtime)
    .env("TMPDIR", l.root.join("tmp"))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(target_os = "linux")]
    // SAFETY: the hook only clears capabilities with async-signal-safe syscalls.
    unsafe {
        use std::os::unix::process::CommandExt;
        c.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
    }
    let mut child = c.spawn()?;
    let parent = herdr::process_evidence(child.id())?;
    write_json(
        &evidence.join(format!("emulator-launch-{}.json", Uuid::random().0)),
        &json!({"gate":"owned-emulator-prepare","stage":"exact owned launcher before readiness","lease":path,"root":identity(&l.root)?,"launcher":executable,"process":parent,"expectedQemu":l.process_identity.executable,"AVD":l.avd_path,"serial":l.serial,"adbClientHome":l.adb_client_home}),
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GridQualification {
    version: u32,
    native_executable: PathBuf,
    native_sha256: String,
    native_source_manifest: PathBuf,
    native_source_sha256: String,
    native_schema: PathBuf,
    native_schema_sha256: String,
    native_lock: PathBuf,
    native_lock_sha256: String,
    goatr_source_manifest: PathBuf,
    goatr_source_sha256: String,
    goatr_sha256: String,
    companion_lock_sha256: String,
    helper_executable: PathBuf,
    helper_sha256: String,
    config_sha256: String,
    resource_parent: PathBuf,
    providers: Vec<Provider>,
    omp_sha256: Option<String>,
    codex_sha256: Option<String>,
    shell_grant: String,
    environment: std::collections::BTreeMap<String, String>,
}
impl GridQualification {
    fn immutable_bytes(path: &Path) -> io::Result<Vec<u8>> {
        if !path.is_absolute()
            || fs::canonicalize(path).map_err(|_| grid_failure("qualification_path"))? != path
        {
            return Err(grid_failure("qualification_path"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| grid_failure("qualification_parent"))?;
        private_dir(parent, false).map_err(|_| grid_failure("qualification_parent"))?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| grid_failure("qualification_open"))?;
        let before = file
            .metadata()
            .map_err(|_| grid_failure("qualification_metadata"))?;
        let stamp = |m: &fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mode(),
                m.uid(),
                m.nlink(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if !before.is_file()
            || before.uid() != uid()
            || before.nlink() != 1
            || before.mode() & 0o7777 != 0o400
            || before.len() > 65536
        {
            return Err(grid_failure("qualification_immutable"));
        }
        if stamp(&before)
            != stamp(
                &fs::symlink_metadata(path).map_err(|_| grid_failure("qualification_metadata"))?,
            )
        {
            return Err(grid_failure("qualification_changed"));
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        (&mut file)
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| grid_failure("qualification_read"))?;
        let after = file
            .metadata()
            .map_err(|_| grid_failure("qualification_metadata"))?;
        let named =
            fs::symlink_metadata(path).map_err(|_| grid_failure("qualification_metadata"))?;
        if bytes.len() > 65536 || stamp(&before) != stamp(&after) || stamp(&after) != stamp(&named)
        {
            return Err(grid_failure("qualification_changed"));
        }
        Ok(bytes)
    }
    fn read(args: &clap::ArgMatches, evidence: &Path) -> io::Result<(Self, Config)> {
        let path = args
            .get_one::<PathBuf>("grid-lease")
            .ok_or_else(|| grid_failure("qualification_missing"))?;
        let bytes = Self::immutable_bytes(path)?;
        let pin: Self =
            serde_json::from_slice(&bytes).map_err(|_| grid_failure("qualification_shape"))?;
        let selected = args
            .get_many::<String>("backend")
            .ok_or_else(|| grid_failure("providers_missing"))?
            .map(|p| {
                if p == "omp" {
                    Provider::Omp
                } else {
                    Provider::Codex
                }
            })
            .collect::<Vec<_>>();
        let unique = selected.iter().copied().collect::<HashSet<_>>();
        if pin.version != 1
            || selected.is_empty()
            || unique.len() != selected.len()
            || selected != pin.providers
            || pin.shell_grant != "read_create_input"
        {
            return Err(grid_failure("qualification_scope"));
        }
        if owned_omp_capture_requested()
            || owned_hello_capture_requested()
            || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").is_some()
        {
            return Err(grid_failure("diagnostic_environment"));
        }
        let workspace = args
            .get_one::<PathBuf>("workspace-root")
            .ok_or_else(|| grid_failure("resource_parent"))?;
        if workspace != &pin.resource_parent
            || fs::canonicalize(workspace).map_err(|_| grid_failure("resource_parent"))?
                != *workspace
        {
            return Err(grid_failure("resource_parent"));
        }
        private_dir(workspace, false).map_err(|_| grid_failure("resource_parent"))?;
        if !evidence.starts_with(workspace) || evidence == workspace {
            return Err(grid_failure("evidence_parent"));
        }
        let config = args
            .get_one::<PathBuf>("config")
            .ok_or_else(|| grid_failure("configuration"))?;
        let deployment = Self::configuration(config, &pin.config_sha256)?;
        if deployment.executables.herdr != pin.native_executable {
            return Err(grid_failure("native_executable"));
        }
        for (path, sha) in [
            (&pin.native_executable, &pin.native_sha256),
            (&pin.native_source_manifest, &pin.native_source_sha256),
            (&pin.native_schema, &pin.native_schema_sha256),
            (&pin.native_lock, &pin.native_lock_sha256),
            (&pin.goatr_source_manifest, &pin.goatr_source_sha256),
            (&pin.helper_executable, &pin.helper_sha256),
        ] {
            Self::file(path, sha)?;
        }
        if std::env::current_exe().map_err(|_| grid_failure("helper_executable"))?
            != pin.helper_executable
            || pin.native_lock_sha256
                != "a2991470164eadb9d8e4704d7599c5cace436714eef5a8ed69cee501e897207f"
            || pin.companion_lock_sha256 != transport::lock_sha256()
            || args.get_one::<String>("expected-companion-lock-sha256")
                != Some(&pin.companion_lock_sha256)
            || args.get_one::<String>("expected-goatr-sha256") != Some(&pin.goatr_sha256)
        {
            return Err(grid_failure("artifact_join"));
        }
        let binary =
            dev_continuity::frozen_package(args).map_err(|_| grid_failure("goatr_package"))?;
        Self::file(&binary, &pin.goatr_sha256)?;
        for (provider, path, sha) in [
            (
                Provider::Omp,
                deployment.executables.omp.as_ref(),
                pin.omp_sha256.as_ref(),
            ),
            (
                Provider::Codex,
                deployment.executables.codex.as_ref(),
                pin.codex_sha256.as_ref(),
            ),
        ] {
            if selected.contains(&provider) {
                Self::file(
                    path.ok_or_else(|| grid_failure("provider_wrapper"))?,
                    sha.ok_or_else(|| grid_failure("provider_wrapper"))?,
                )?;
            } else if sha.is_some() {
                return Err(grid_failure("unselected_wrapper"));
            }
        }
        const ENV: &[&str] = &[
            "HOME",
            "PATH",
            "USER",
            "LOGNAME",
            "LANG",
            "LC_ALL",
            "XDG_RUNTIME_DIR",
            "TERM",
            "COLORTERM",
            "SHELL",
            "NIX_SSL_CERT_FILE",
            "SSL_CERT_FILE",
        ];
        let actual = std::env::vars_os().collect::<Vec<_>>();
        if actual.len() != pin.environment.len() {
            return Err(grid_failure("environment_scope"));
        }
        for (key, value) in actual {
            use sha2::{Digest, Sha256};
            use std::os::unix::ffi::OsStrExt;
            let key = key
                .to_str()
                .ok_or_else(|| grid_failure("environment_scope"))?;
            let hash = hex(&Sha256::digest(value.as_bytes()));
            if !ENV.contains(&key) || pin.environment.get(key) != Some(&hash) {
                return Err(grid_failure("environment_scope"));
            }
        }
        if !pin.environment.contains_key("HOME")
            || !pin.environment.contains_key("PATH")
            || !pin.environment.contains_key("XDG_RUNTIME_DIR")
        {
            return Err(grid_failure("environment_scope"));
        }
        Ok((pin, deployment))
    }
    fn configuration(path: &Path, sha: &str) -> io::Result<Config> {
        use sha2::{Digest, Sha256};
        if sha.len() != 64
            || !sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(grid_failure("configuration_pin"));
        }
        goatr::config::absolute(path).map_err(|_| grid_failure("configuration_path"))?;
        if fs::canonicalize(path).map_err(|_| grid_failure("configuration_path"))? != path {
            return Err(grid_failure("configuration_path"));
        }
        let parent = path.parent().ok_or_else(|| grid_failure("configuration_parent"))?;
        private_dir(parent, false).map_err(|_| grid_failure("configuration_parent"))?;
        let parent_before = fs::symlink_metadata(parent)
            .map_err(|_| grid_failure("configuration_parent"))?;
        let named_before = fs::symlink_metadata(path)
            .map_err(|_| grid_failure("configuration_metadata"))?;
        let owner = uid();
        let permitted = |m: &fs::Metadata| m.is_file() && m.uid() == owner && m.nlink() == 1
            && m.mode() & 0o7777 == 0o400 && m.len() <= goatr::config::CONFIG_BYTES as u64;
        if !permitted(&named_before) {
            return Err(grid_failure("configuration_immutable"));
        }
        let mut file = OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path).map_err(|_| grid_failure("configuration_open"))?;
        let before = file.metadata().map_err(|_| grid_failure("configuration_metadata"))?;
        let stamp = |m: &fs::Metadata| (
            m.dev(), m.ino(), m.len(), m.mode(), m.uid(), m.gid(), m.nlink(),
            m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec(),
        );
        if !permitted(&before) || stamp(&before) != stamp(&named_before)
        {
            return Err(grid_failure("configuration_immutable"));
        }
        let mut text = String::with_capacity(before.len() as usize);
        (&mut file).take((goatr::config::CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut text).map_err(|_| grid_failure("configuration_read"))?;
        if text.len() > goatr::config::CONFIG_BYTES {
            return Err(grid_failure("configuration_bound"));
        }
        let hex = b"0123456789abcdef";
        if Sha256::digest(text.as_bytes()).iter().zip(sha.as_bytes().chunks_exact(2))
            .any(|(byte, pair)| pair[0] != hex[usize::from(*byte >> 4)]
                || pair[1] != hex[usize::from(*byte & 15)])
        {
            return Err(grid_failure("configuration_sha256"));
        }
        let deployment = Config::parse(&text).map_err(|_| grid_failure("configuration"))?;
        deployment.preflight().map_err(|_| grid_failure("configuration"))?;
        let after = file.metadata().map_err(|_| grid_failure("configuration_metadata"))?;
        let named_after = fs::symlink_metadata(path)
            .map_err(|_| grid_failure("configuration_metadata"))?;
        let parent_after = fs::symlink_metadata(parent)
            .map_err(|_| grid_failure("configuration_parent"))?;
        private_dir(parent, false).map_err(|_| grid_failure("configuration_parent"))?;
        if !permitted(&after) || !permitted(&named_after)
            || stamp(&before) != stamp(&after) || stamp(&after) != stamp(&named_after)
            || stamp(&parent_before) != stamp(&parent_after)
            || fs::canonicalize(path).map_err(|_| grid_failure("configuration_path"))? != path
        {
            return Err(grid_failure("configuration_changed"));
        }
        Ok(deployment)
    }
    fn file(path: &Path, sha: &str) -> io::Result<()> {
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !path.is_absolute()
            || fs::canonicalize(path).map_err(|_| grid_failure("artifact_path"))? != path
        {
            return Err(grid_failure("artifact_pin"));
        }
        let before = fs::symlink_metadata(path).map_err(|_| grid_failure("artifact_metadata"))?;
        if !before.is_file() || before.mode() & 0o222 != 0 {
            return Err(grid_failure("artifact_immutable"));
        }
        if binary_sha256(path).map_err(|_| grid_failure("artifact_read"))? != sha {
            return Err(grid_failure("artifact_sha256"));
        }
        let after = fs::symlink_metadata(path).map_err(|_| grid_failure("artifact_metadata"))?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.mtime() != after.mtime()
            || before.ctime() != after.ctime()
        {
            return Err(grid_failure("artifact_changed"));
        }
        Ok(())
    }
}
// No stop authority: the grid runner holds original pidfds and its supervisor after every outcome.
struct GridHold {
    originals: Vec<std::os::fd::OwnedFd>,
    spawned: usize,
    relays: Vec<Arc<std::net::TcpListener>>,
    source_control: Option<tokio::io::BufReader<McpStdin>>,
    source_loss_observed: bool,
}
impl GridHold {
    fn pin_child(&mut self, child: &Child) -> io::Result<()> {
        use std::os::fd::FromRawFd;
        self.spawned += 1;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id(), 0) } as i32;
        if fd < 0 {
            return Err(grid_failure("original_pidfd"));
        }
        self.originals
            .push(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
        Ok(())
    }
}
struct Desktop {
    child: Child,
    process: ProcessEvidence,
}
impl Desktop {
    fn spawn(c: std::process::Command) -> io::Result<Self> {
        Self::spawn_held(c, None, None)
    }
    fn spawn_held(mut c: std::process::Command, hold: Option<&mut GridHold>, admission_manifest: Option<&Path>) -> io::Result<Self> {
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
        if let Some(manifest) = admission_manifest {
            write_json(manifest, &json!({"pid":child.id(),"processProof":"unproved",
                "cleanupAuthority":false,"proofManifest":manifest.with_extension("proof.json"),
                "scope":"fresh retained child spawn only; PID alone grants no cleanup authority"}))?;
            println!("{}",json!({"gate":"admission-owned-herdr-spawn","manifest":manifest}));
        }
        if let Some(hold) = hold {
            hold.pin_child(&child)?;
        }
        let process = herdr::process_evidence(child.id())?;
        if let Some(manifest) = admission_manifest {
            write_json(&manifest.with_extension("proof.json"),
                &json!({"process":process,"spawnManifest":manifest,
                    "scope":"captured original child proof; cleanup still requires current identity review"}))?;
        }
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
/// Ordinary checkpoints stop only their companion. Admission retains both originals
/// for identity review; desktop resources are never rolled back or lifetime-coupled.
struct ServeLaunch<'a> {
    diagnostic_capture: bool,
    herdr_report_capture: Option<&'a herdr_report_capture::Capture>,
    preserve_xdg_config_home: bool,
    retain_process: bool,
    hello_capture: bool,
    grid_hold: Option<&'a mut GridHold>,
    project_read: Option<&'a ProjectReadFixture>,
    startup_deadline: Option<Instant>,
    unit_authority: Option<&'a goatr::codex_relay::UnitWakePin>,
}
struct Companion {
    child: Child,
    process: ProcessEvidence,
    guard_drain: Option<GuardDrain>,
    omp_capture_budget: Option<herdr_report_capture::Budget>,
    retain_process: bool,
}
const COMPANION_CLEANUP_BUDGET: Duration = Duration::from_secs(15);
enum GuardDrain {
    Codex {
        worker: std::thread::JoinHandle<()>,
        progress: Arc<CodexDrainProgress>,
    },
    Other(std::thread::JoinHandle<()>),
}
#[derive(Clone, Default)]
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
    owner_progress_mask: u8,
    owner_progress_guard: Option<&'static str>,
    pending_at_observation: bool,
    first_nonprogress_phase: Option<Arc<str>>,
    first_nonprogress_guard: Option<Arc<str>>,
}
#[derive(Clone, Copy, Default)]
enum CodexDrainTermination {
    #[default]
    Reading,
    Eof,
    ReadError,
}
#[derive(Default)]
struct CodexDrainProgress(Mutex<CodexDrainSummary>);
impl CodexDrainProgress {
    fn update(&self, update: impl FnOnce(&mut CodexDrainSummary)) {
        let mut summary = self.0.lock().unwrap_or_else(|error| error.into_inner());
        update(&mut summary);
    }
    fn snapshot(&self) -> CodexDrainSummary {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }
    fn accepted_guard(&self, value: &Value) {
        let Some(phase)=value["phase"].as_str() else { return; };
        let Some(guard)=value["guard"].as_str() else { return; };
        self.update(|summary| {
            if phase == "owner_progress" {
                match (summary.owner_progress_mask, guard) {
                    (0, "owner_start_settled") => summary.owner_progress_mask = 1,
                    (1, "owner_child_status_returned") => summary.owner_progress_mask = 3,
                    (3, "owner_native_initialize_entered") => {
                        summary.owner_progress_mask = 7;
                        summary.owner_progress_guard = Some("owner_native_initialize_entered");
                    }
                    (3, "owner_pending_authorization")
                    | (3, "owner_pending_service_metadata")
                    | (3, "owner_pending_service_proof")
                    | (3, "owner_pending_native_process")
                    | (3, "owner_pending_socket_epoch")
                    | (3, "owner_pending_owner_record_proof") => {
                        summary.owner_progress_mask = 7;
                        summary.owner_progress_guard = Some(match guard {
                            "owner_pending_authorization" => "owner_pending_authorization",
                            "owner_pending_service_metadata" => "owner_pending_service_metadata",
                            "owner_pending_service_proof" => "owner_pending_service_proof",
                            "owner_pending_native_process" => "owner_pending_native_process",
                            "owner_pending_socket_epoch" => "owner_pending_socket_epoch",
                            "owner_pending_owner_record_proof" => "owner_pending_owner_record_proof",
                            _ => unreachable!(),
                        });
                        summary.pending_at_observation = true;
                    }
                    _ => {}
                }
            } else if summary.first_nonprogress_phase.is_none() {
                summary.first_nonprogress_phase = Some(Arc::from(phase));
                summary.first_nonprogress_guard = Some(Arc::from(guard));
            }
        });
    }
}
fn codex_drain_is_pristine(summary: &CodexDrainSummary, worker_finished: bool) -> bool {
    summary.matching_prefix == 0
        && summary.accepted == 0
        && summary.rejected == 0
        && summary.oversized == 0
        && !summary.output_failed
        && !summary.eof
        && !summary.completed
        && summary.read_error.is_none()
        && matches!(summary.termination, CodexDrainTermination::Reading)
        && !worker_finished
}
impl Companion {
    fn recheck(&self) -> io::Result<()> {
        if herdr::process_evidence(self.process.pid)? != self.process {
            return Err(failure("owned companion process epoch changed"));
        }
        Ok(())
    }
    fn codex_drain_snapshot(&self) -> io::Result<CodexDrainSummary> {
        match self.guard_drain.as_ref() {
            Some(GuardDrain::Codex { progress, .. }) => Ok(progress.snapshot()),
            _ => Err(failure("owned Codex diagnostic capture unavailable")),
        }
    }
    fn codex_drain_pristine(&self) -> io::Result<()> {
        let (summary, worker_finished) = match self.guard_drain.as_ref() {
            Some(GuardDrain::Codex { worker, progress }) => (progress.snapshot(), worker.is_finished()),
            _ => return Err(failure("owned Codex diagnostic capture unavailable")),
        };
        if !codex_drain_is_pristine(&summary, worker_finished) {
            return Err(failure("Codex diagnostic readiness emitted a guard; no create submitted"));
        }
        Ok(())
    }
    async fn codex_creation_guard_by(&self, deadline: Instant) -> io::Result<CodexDrainSummary> {
        loop {
            let summary = self.codex_drain_snapshot()?;
            if summary.matching_prefix > 8
                || summary.accepted > 8
                || summary.rejected != 0
                || summary.oversized != 0
                || summary.output_failed
                || summary.eof
                || summary.completed
                || summary.read_error.is_some()
                || !matches!(summary.termination, CodexDrainTermination::Reading)
            {
                return Err(failure("Codex diagnostic guard capture malformed or unavailable"));
            }
            if summary.first_nonprogress_phase.as_deref() == Some("creation") {
                return Ok(summary);
            }
            if Instant::now() >= deadline {
                return Err(failure("Codex diagnostic creation guard unavailable before deadline"));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    async fn stop_process_until(&mut self, deadline: Instant) -> io::Result<()> {
        if self.retain_process {
            return Err(grid_failure("cleanup_not_authorized"));
        }
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }
        self.recheck()?;
        let _ = unsafe { libc::kill(self.process.pid as i32, libc::SIGTERM) };
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
        stopped?;
        self.finish_guard_drain();
        Ok(())
    }
    async fn finish_retained_omp_capture(&self) -> bool {
        let Some(budget) = &self.omp_capture_budget else {
            return false;
        };
        // A terminal receipt follows the producer's synchronous first-guard write.
        // Wait for that already-emitted line, not EOF from the retained original.
        let deadline = Instant::now() + COMPANION_CLEANUP_BUDGET;
        loop {
            if budget.shell_probe_captured() {
                return true;
            }
            if Instant::now() >= deadline
                || self.guard_drain.as_ref().is_none_or(|drain| match drain {
                    GuardDrain::Codex { worker, .. } | GuardDrain::Other(worker) => worker.is_finished(),
                })
            {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
        if !self.retain_process
            && status.is_ok_and(|s| s.is_none())
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
    stream_floor: Option<u64>,
    streamed_text: bool,
    streamed_tool: bool,
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
        let stream_floor=snapshot.history.iter().map(|record|record.order.0).max();
        let records = std::mem::take(&mut snapshot.history);
        let mut view = Self {
            sequence,
            snapshot,
            record_bytes: HashMap::new(),
            bytes: 0,
            failed: false,
            stream_floor,
            streamed_text: false,
            streamed_tool: false,
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
            EventBody::HistoryUpsert(value) => {
                for record in &value.records {
                    if self.stream_floor.is_none_or(|floor|record.order.0>floor) {
                        self.streamed_text|=record.role==Role::Assistant
                            && matches!(&record.text,Some(Content::Text(text)) if !text.text.0.is_empty());
                        self.streamed_tool|=record.tool.is_some();
                    }
                }
                self.upsert(value.records)?;
            }
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

// Private, count-only witness. Production owns coverage, wide-cell validation and pixels.
#[derive(PartialEq, Eq)]
enum GridSubject {
    Pane(Target),
    Shell(ShellTarget),
}
const TERMINAL_MARKER:&str="GOATRSTOCKBEGIN";
const TERMINAL_DELAYED:&str="GOATRSTOCKDELAYED";
const TERMINAL_DISCLOSURE:&str="Native control may resize every tab in this Herdr session and let Herdr resume deferred agents. Goatr does not send terminal input to known agent panes. With the sole active frontend this can affect every workspace/tab on this server. Queued native bytes cannot be recalled. Release reapplies surviving controllers, not an assumed saved size.";
#[derive(Clone,Copy,Default)]
struct TerminalFixtureFacts {unicode:u8,colors:u8,wide_tails:u32,marker:bool,popup:bool,graphics:bool}
impl TerminalFixtureFacts {
    fn complete(self)->bool{self.unicode==31&&self.colors==15&&self.wide_tails>=5&&self.marker&&!self.popup&&!self.graphics}
    fn evidence(self)->Value {json!({"unicodeCases":self.unicode.count_ones(),"colorCases":self.colors.count_ones(),
        "cjk":self.unicode&1!=0,"kana":self.unicode&2!=0,"zwj":self.unicode&4!=0,"vs16":self.unicode&8!=0,"flag":self.unicode&16!=0,
        "namedSgrPalette":self.colors&1!=0,"indexed202":self.colors&2!=0,"rgb112233":self.colors&4!=0,"defaultColor":self.colors&8!=0,
        "canonicalWideTails":self.wide_tails,"publicMarker":self.marker,"popupBlocked":self.popup,
        "graphicsOmitted":self.graphics,"fixtureComplete":self.complete()})}
}
struct TerminalFixtureWitness {stage:TerminalFixtureFacts,committed:Option<TerminalFixtureFacts>,opening:Option<(u64,bool)>,marker_match:usize,wide_fixture:bool}
impl TerminalFixtureWitness {
    fn new()->Self{Self{stage:TerminalFixtureFacts::default(),committed:None,opening:None,marker_match:0,wide_fixture:false}}
    fn begin(&mut self,frame:&GridBegin){self.stage=TerminalFixtureFacts{popup:frame.popup_blocked,graphics:frame.graphics_omitted,..Default::default()};self.marker_match=0;self.wide_fixture=false;}
    fn cell(&mut self,cell:&GridCell,row_start:bool) {
        if row_start{self.marker_match=0;}
        if cell.width.0==0 {if self.wide_fixture{self.stage.wide_tails+=1;}self.wide_fixture=false;return;}
        self.wide_fixture=false;
        if cell.attributes.concealed{return;}
        for byte in cell.text.bytes(){
            if byte==TERMINAL_MARKER.as_bytes()[self.marker_match]{self.marker_match+=1;}
            else{self.marker_match=usize::from(byte==TERMINAL_MARKER.as_bytes()[0]);}
            if self.marker_match==TERMINAL_MARKER.len(){self.stage.marker=true;self.marker_match=0;}
        }
        let unicode=match cell.text.as_str(){"界"=>1,"ｶﾞ"=>2,"👩‍💻"=>4,"❤️"=>8,"🇺🇸"=>16,_=>0};
        if unicode!=0&&cell.width.0==2{self.stage.unicode|=unicode;self.wide_fixture=true;}
        if cell.background!=GridColor::Default||cell.attributes.inverse{return;}
        self.stage.colors|=match (cell.text.as_str(),&cell.foreground){
            ("R",GridColor::Indexed{index})if index.0==1=>1,
            ("I",GridColor::Indexed{index})if index.0==202=>2,
            ("T",GridColor::Rgb{value})if value.0==0x112233=>4,
            ("D",GridColor::Default)=>8,_=>0,
        };
    }
}
#[derive(Clone,Copy)]
enum GridGoal {Commit,Token,TerminalFixture{after:u64},Resized{after:u64,dimensions:(u64,u64)}}
struct GridWitness {
    opening: Uuid,
    subject: GridSubject,
    subscription: Option<Uuid>,
    next_seq: u64,
    revision: u64,
    parts: u64,
    seen: u64,
    cells: usize,
    cell_limit: usize,
    bytes: usize,
    deadline: Option<Instant>,
    committed: Option<u64>,
    token: Vec<u8>,
    prefix: [usize; 64],
    matched: usize,
    tentative: bool,
    token_commit: Option<u64>,
    previous_cell: Option<(u64, u64)>,
    retired: bool,
    columns:u64,
    rows:u64,
    committed_dimensions:Option<(u64,u64)>,
    wide_pending:bool,
    fixture:Option<TerminalFixtureWitness>,
}
#[track_caller]
fn grid_failure(phase: &'static str) -> io::Error {
    // Only authored phase labels cross the private terminal boundary.
    failure(&format!("grid smoke boundary: phase={phase}"))
}
#[track_caller]
fn grid_context(phase: &'static str, error: io::Error) -> io::Error {
    super::located_message(
        error.kind(),
        &format!("grid smoke boundary: phase={phase}"),
        super::helper_error_location(&error),
    )
}
impl GridWitness {
    fn pending(opening: Uuid, subject: GridSubject) -> Self {
        Self {
            opening,
            subject,
            subscription: None,
            next_seq: 1,
            revision: 0,
            parts: 0,
            seen: 0,
            cells: 0,
            cell_limit: 0,
            bytes: 0,
            deadline: None,
            committed: None,
            token: Vec::new(),
            prefix: [0; 64],
            matched: 0,
            tentative: false,
            token_commit: None,
            previous_cell: None,
            retired: false,
            columns:0,rows:0,committed_dimensions:None,wide_pending:false,fixture:None,
        }
    }
    fn budget(&mut self, bytes: usize) -> io::Result<()> {
        if bytes == 0 || self.deadline.is_none_or(|end| Instant::now() >= end) {
            return Err(grid_failure("transaction_deadline"));
        }
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| grid_failure("transaction_bytes"))?;
        if self.bytes > 8 * 1024 * 1024 {
            return Err(grid_failure("transaction_bytes"));
        }
        Ok(())
    }
    fn begin(&mut self, frame: &GridBegin, bytes: usize, initial: bool) -> io::Result<()> {
        frame.validate().map_err(|_| grid_failure("descriptor"))?;
        if matches!(self.subject,GridSubject::Pane(_)) && (frame.columns.0*frame.rows.0>8192 || !frame.graphics_omitted || frame.alternate_screen || frame.popup_blocked) {
            return Err(grid_failure("ansi_observer_descriptor"));
        }
        let expected = if initial {
            0
        } else {
            self.committed
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| grid_failure("revision"))?
        };
        if frame.revision.0 != expected || (!initial && self.deadline.is_some()) {
            return Err(grid_failure("revision"));
        }
        self.deadline = Some(Instant::now() + Duration::from_secs(15));
        self.revision = expected;
        self.parts = frame.part_count.0;
        self.seen = 0;
        self.cells = 0;
        self.cell_limit = (frame.columns.0 * frame.rows.0) as usize;
        self.columns=frame.columns.0;self.rows=frame.rows.0;self.wide_pending=false;
        if let Some(fixture)=&mut self.fixture{fixture.begin(frame);}
        self.bytes = 0;
        self.matched = 0;
        self.tentative = false;
        self.previous_cell = None;
        self.budget(bytes)
    }
    fn response(
        &mut self,
        id: &Uuid,
        outcome: &Result<ResultBody, WireError>,
        bytes: usize,
    ) -> io::Result<()> {
        if id != &self.opening {
            return Ok(());
        }
        if self.subscription.is_some() {
            return Err(grid_failure("opening_duplicate"));
        }
        let (subscription, frame) = match (&self.subject, outcome) {
            (GridSubject::Pane(expected), Ok(ResultBody::TerminalGridSubscribe(r)))
                if &r.target == expected && r.seq.0 == 0 =>
            {
                (&r.subscription, &r.frame)
            }
            (GridSubject::Shell(expected), Ok(ResultBody::ShellGridSubscribe(r)))
                if &r.target == expected && r.seq.0 == 0 =>
            {
                (&r.subscription, &r.frame)
            }
            _ => return Err(grid_failure("opening_response")),
        };
        self.subscription = Some(subscription.clone());
        if self.retired {
            return Err(grid_failure("opening_retired"));
        }
        self.begin(frame, bytes, true)
    }
    fn arm(&mut self, token: &str) -> io::Result<()> {
        if token.is_empty() || token.len() > 64 || !token.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(grid_failure("token_contract"));
        }
        self.token = token.as_bytes().to_vec();
        self.prefix.fill(0);
        let mut n = 0;
        for i in 1..self.token.len() {
            while n > 0 && self.token[i] != self.token[n] {
                n = self.prefix[n - 1];
            }
            if self.token[i] == self.token[n] {
                n += 1;
            }
            self.prefix[i] = n;
        }
        self.matched = 0;
        self.tentative = false;
        self.token_commit = None;
        self.previous_cell = None;
        Ok(())
    }
    fn terminal_opening_commit(&self)->io::Result<u64> {
        match self.fixture.as_ref().and_then(|fixture|fixture.opening) {
            Some((revision,false))=>Ok(revision),
            Some((_,true))=>Err(grid_failure("terminal_delayed_marker_already_in_opening")),
            None=>Err(grid_failure("terminal_opening_not_committed")),
        }
    }
    fn terminal_fixture_ready(&self,after:u64)->io::Result<bool> {
        let opening=self.terminal_opening_commit()?;
        if opening!=after{return Err(grid_failure("terminal_opening_revision_changed"));}
        Ok(self.token_commit.is_some_and(|revision|revision>opening)
            &&self.fixture.as_ref().and_then(|fixture|fixture.committed).is_some_and(TerminalFixtureFacts::complete))
    }
    fn text(&mut self, text: &str) {
        if self.token.is_empty() {
            return;
        }
        for byte in text.bytes() {
            while self.matched > 0 && byte != self.token[self.matched] {
                self.matched = self.prefix[self.matched - 1];
            }
            if byte == self.token[self.matched] {
                self.matched += 1;
            }
            if self.matched == self.token.len() {
                self.tentative = true;
                self.matched = self.prefix[self.matched - 1];
            }
        }
    }
    fn event(&mut self, event: Event, bytes: usize) -> io::Result<()> {
        let result = self.event_checked(event, bytes);
        if result.is_err() {
            self.retired = true;
            self.token.fill(0);
            self.token.clear();
        }
        result
    }
    fn event_checked(&mut self, event: Event, bytes: usize) -> io::Result<()> {
        if self.retired
            || self.subscription.as_ref() != Some(&event.subscription)
            || event.seq.0 != self.next_seq
        {
            return Err(grid_failure("subscription_sequence"));
        }
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| grid_failure("sequence_limit"))?;
        match event.body {
            EventBody::GridBegin(data) => self.begin(&data.frame, bytes, false),
            EventBody::GridPart(part) => {
                self.budget(bytes)?;
                if part.revision.0 != self.revision
                    || part.index.0 != self.seen
                    || self.seen >= self.parts
                {
                    return Err(grid_failure("part_order"));
                }
                for span in part.spans {
                    for (offset, cell) in span.cells.into_iter().enumerate() {
                        cell.validate().map_err(|_|grid_failure("selected_cell"))?;
                        if matches!(self.subject,GridSubject::Shell(_)) && cell.text.len()>128 {return Err(grid_failure("selected_cell"));}
                        let column=span.column.0+offset as u64;
                        if span.row.0!=self.cells as u64/self.columns||column!=self.cells as u64%self.columns
                            || (cell.width.0==0)!=self.wide_pending
                            || cell.width.0==2&&column+1>=self.columns
                        {return Err(grid_failure("canonical_selected_coverage"));}
                        self.wide_pending=cell.width.0==2;
                        if let Some(fixture)=&mut self.fixture{fixture.cell(&cell,column==0);}
                        self.cells = self
                            .cells
                            .checked_add(1)
                            .ok_or_else(|| grid_failure("cell_limit"))?;
                        if self.cells > self.cell_limit {
                            return Err(grid_failure("cell_limit"));
                        }
                        if self.previous_cell != Some((span.row.0, column)) {
                            self.matched = 0;
                        }
                        if cell.width.0 != 0 {
                            self.text(&cell.text);
                        }
                        self.previous_cell = Some((span.row.0, column + 1));
                    }
                }
                self.seen += 1;
                Ok(())
            }
            EventBody::GridCommit(commit) => {
                self.budget(bytes)?;
                if commit.revision.0 != self.revision
                    || self.seen != self.parts
                    || self.cells != self.cell_limit || self.wide_pending
                {
                    return Err(grid_failure("commit_order"));
                }
                self.committed = Some(self.revision);
                self.committed_dimensions=Some((self.columns,self.rows));
                if let Some(fixture)=&mut self.fixture {
                    fixture.committed=Some(fixture.stage);
                    if fixture.opening.is_none(){fixture.opening=Some((self.revision,self.tentative));}
                }
                if self.tentative {
                    self.token_commit = Some(self.revision);
                }
                self.deadline = None;
                self.matched = 0;
                self.tentative = false;
                self.previous_cell = None;
                Ok(())
            }
            EventBody::SyncRequired(_) => Err(grid_failure("source_sync_required")),
            _ => Err(grid_failure("event_family")),
        }
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
    grid: Option<GridWitness>,
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
        self.event_sized(event,0)
    }
    fn event_sized(&mut self,event:Event,bytes:usize)->io::Result<()> {
        Self::route_event(&mut self.grid,&mut self.events,&mut self.generation,event,bytes)
    }
    fn route_event(grid:&mut Option<GridWitness>,events:&mut u64,generation:&mut Option<GenerationView>,event:Event,bytes:usize)->io::Result<()> {
        let private=matches!(&event.body,EventBody::GridBegin(_)|EventBody::GridPart(_)|EventBody::GridCommit(_))
            || matches!(&event.body,EventBody::SyncRequired(_))&&grid.as_ref().is_some_and(|g|g.subscription.as_ref()==Some(&event.subscription));
        if private {
            return grid.as_mut().ok_or_else(||grid_failure("unowned_grid_event"))?.event(event,bytes);
        }
        *events+=1;
        if let Some(view)=generation {view.routed_event(event)?;}
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
                self.report_frame_read_close();
                request_failure(method, phase, "frame_read", error)
            })?;
            match decode_payload(&bytes, Some(method)).map_err(|error| {
                request_failure(method, phase, "payload_decode", io::Error::other(error))
            })? {
                Envelope::Response(r) if r.id == id => {
                    if let Some(grid) = &mut self.grid {
                        grid.response(&r.id, &r.outcome, bytes.len())?;
                    }
                    self.request_phase(method, phase, "response_received");
                    return Ok(r.outcome);
                }
                Envelope::Event(event) => self.event_sized(event, bytes.len())?,
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
    let body = RequestBody::SessionCreate(SessionCreateParams { directory_ref: None, placement: None, boot_id: phone.hello.boot_id.clone(),
    server_id: server.clone(),
    project_id: config.projects[0].id.clone(),
    provider,
    name: Label::new(format!("partial-{provider:?}-{}", &id.0[..8]))
        .map_err(io::Error::other)?, });
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
            features: HelloFeatures::default(),
        },
        events: 0,
        generation: None,
        history_command: None,
        omp_capture_budget: None,
        phone_close_reported: false,
        grid: None,
    };
    if let Some(i) = invite {
        p.rpc(
            RequestBody::PairConfirm(PairConfirmParams {
                token: i.token,
                device_name: Label::new(OWNED_STOCK_CHECKPOINT_PAIR_LABEL)
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
                device_name: Label::new(OWNED_STOCK_CHECKPOINT_PAIR_LABEL)
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
    let original=Client::connect(socket).await?.qualify_executable(&config.executables.herdr)?;
    proof_selected(config,socket,s,None,&original).await
}
pub(super) async fn created_codex_proof(
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
    let original=Client::connect(socket).await?.qualify_executable(&config.executables.herdr)?;
    let proof = tokio::time::timeout_at(
        deadline.into(),
        proof_selected(config,socket,s,Some((binding,deadline)),&original),
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
    original:&Client,
) -> io::Result<NativeProof> {
    original.recheck()?;
    let client=Client::connect(socket).await?;original.recheck()?;
    let client=client.qualify_executable(&config.executables.herdr)?;
    if client.evidence!=original.evidence{return Err(failure("native proof snapshot peer differs from original retained source"));}
    original.recheck()?;
    let (e,snapshot)=client.snapshot().await?;original.recheck()?;
    if e!=original.evidence{return Err(failure("native proof snapshot returned a replacement source"));}
    let pane = snapshot.pane(&s.pane_id)?;
    if pane.terminal_id != s.terminal_id || pane.workspace_id != s.workspace_id {
        return Err(failure(
            "actual native session routing differs from current Herdr surface",
        ));
    }
    original.recheck()?;
    let client=Client::connect(socket).await?;original.recheck()?;
    let client=client.qualify_executable(&config.executables.herdr)?;
    if client.evidence!=original.evidence{return Err(failure("native proof process-info peer differs from original retained source"));}
    original.recheck()?;
    let info=client.process_info(&s.pane_id).await?;original.recheck()?;
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
            original.recheck()?;
            let client=Client::connect(socket).await?;original.recheck()?;
            let client=client.qualify_executable(&config.executables.herdr)?;
            if client.evidence!=original.evidence{return Err(failure("OMP post-FINAL snapshot peer differs from original retained source"));}
            original.recheck()?;
            let (current_herdr,current_surface)=client.snapshot().await?;original.recheck()?;
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
            original.recheck()?;
            let client=Client::connect(socket).await?;original.recheck()?;
            let client=client.qualify_executable(&config.executables.herdr)?;
            if client.evidence!=original.evidence{return Err(failure("OMP post-FINAL process-info peer differs from original retained source"));}
            original.recheck()?;
            let current_processes=client.process_info(&s.pane_id).await?;original.recheck()?;
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
fn owned_herdr_report_binary(capture: bool, workspace: &Path) -> io::Result<Option<PathBuf>> {
    let binary = std::env::var_os("GOATR_OWNED_HERDR_REPORT_BIN");
    let expected = std::env::var("GOATR_OWNED_HERDR_REPORT_SHA256").ok();
    if !capture {
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

fn codex_single_creation_diagnostic_environment(args: &clap::ArgMatches) -> io::Result<()> {
    if !args.get_flag("codex-invalidation-diagnostic") {
        return Ok(());
    }
    if [
        "GOATR_OWNED_CODEX_DIAGNOSTIC",
        "GOATR_OWNED_OMP_CREATION_CAPTURE",
        "GOATR_OWNED_OMP_OFFLINE_CAPTURE",
        "GOATR_OWNED_OMP_BACKEND_CAPTURE",
        "GOATR_OWNED_HELLO_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256",
        "GOATR_OWNED_OMP_PRODUCER_BUN",
        "GOATR_OWNED_OMP_PRODUCER_BUN_SHA256",
        "GOATR_OWNED_HERDR_REPORT_BIN",
        "GOATR_OWNED_HERDR_REPORT_SHA256",
        "GOATR_OWNED_GRID_OPEN_CAPTURE",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some())
    {
        return Err(failure("Codex single-creation diagnostic rejects inherited diagnostic/private capture environment"));
    }
    Ok(())
}
fn require_shared_backends(args: &clap::ArgMatches) -> io::Result<()> {
    let mut count=0;
    let mut omp=0;
    let mut codex=0;
    for provider in args.get_many::<String>("backend").into_iter().flatten() {
        count+=1;
        omp+=usize::from(provider=="omp");
        codex+=usize::from(provider=="codex");
    }
    if count!=2 || omp!=1 || codex!=1 {
        return Err(failure("shared-agent-only requires exactly one OMP and one Codex backend"));
    }
    Ok(())
}
fn retained_pairing_provider_create(args:&clap::ArgMatches)->io::Result<Option<Provider>> {
    let selected=args.get_many::<String>("backend").into_iter().flatten().map(String::as_str).collect::<Vec<_>>();
    if args.get_flag("physical-owned-retirement") || args.get_flag("physical-pairing-shell-checkpoint") || args.get_flag("physical-pairing-shell-interaction-checkpoint") {
        return if selected.is_empty()&&!args.get_flag("consent-owned-providers") {Ok(None)} else {Err(failure("retained pairing shell checkpoint rejects provider backend and consent"))};
    }
    match (selected.as_slice(),args.get_flag("consent-owned-providers")) {
        ([],false)=>Ok(None),
        (["omp"],true)=>Ok(Some(Provider::Omp)),
        (["codex"],true)=>Ok(Some(Provider::Codex)),
        _=>Err(failure("retained pairing creation requires exactly one --backend omp|codex with --consent-owned-providers")),
    }
}
fn project_read_mode_environment(args:&clap::ArgMatches)->io::Result<()> {
    if !args.get_flag("project-read-only"){return Ok(());}
    if ["GOATR_OWNED_CODEX_DIAGNOSTIC", "GOATR_OWNED_OMP_CREATION_CAPTURE",
        "GOATR_OWNED_OMP_OFFLINE_CAPTURE","GOATR_OWNED_OMP_BACKEND_CAPTURE","GOATR_OWNED_HELLO_CAPTURE","GOATR_OWNED_OMP_PRODUCER_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER","GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256","GOATR_OWNED_OMP_PRODUCER_BUN",
        "GOATR_OWNED_OMP_PRODUCER_BUN_SHA256","GOATR_OWNED_HERDR_REPORT_BIN","GOATR_OWNED_HERDR_REPORT_SHA256","GOATR_OWNED_GRID_OPEN_CAPTURE"]
        .iter().any(|key|std::env::var_os(key).is_some()) {return Err(failure("project-read-only conflicts with inherited private/provider diagnostic capture"));}
    Ok(())
}

fn fresh_shared_evidence(path: &Path) -> io::Result<()> {
    let parent=path.parent().ok_or_else(||failure("shared-agent evidence parent absent"))?;
    if fs::canonicalize(parent)?!=parent {
        return Err(failure("shared-agent evidence parent must be canonical"));
    }
    fs::DirBuilder::new().mode(0o700).create(path)?;
    private_dir(path,false)
}
fn terminal_frozen_selection(args:&clap::ArgMatches)->io::Result<bool> {
    match (args.get_one::<String>("expected-goatr-sha256"),args.get_one::<String>("expected-companion-lock-sha256")) {
        (None,None)=>Ok(false),(Some(_),Some(_))=>Ok(true),
        _=>Err(failure("terminal-only frozen executable requires both explicit binary and lock hashes")),
    }
}
fn terminal_mode_environment(args:&clap::ArgMatches)->io::Result<()> {
    if !args.get_flag("terminal-only"){return Ok(());}
    terminal_frozen_selection(args)?;
    if ["GOATR_OWNED_CODEX_DIAGNOSTIC",
        "GOATR_OWNED_OMP_CREATION_CAPTURE","GOATR_OWNED_OMP_OFFLINE_CAPTURE","GOATR_OWNED_OMP_BACKEND_CAPTURE",
        "GOATR_OWNED_HELLO_CAPTURE","GOATR_OWNED_OMP_PRODUCER_CAPTURE","GOATR_OWNED_OMP_PRODUCER_METADATA_READER",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256","GOATR_OWNED_OMP_PRODUCER_BUN",
        "GOATR_OWNED_OMP_PRODUCER_BUN_SHA256","GOATR_OWNED_HERDR_REPORT_BIN","GOATR_OWNED_HERDR_REPORT_SHA256"]
        .iter().any(|key|std::env::var_os(key).is_some()) {
        return Err(failure("terminal-only conflicts with inherited provider/private diagnostic capture"));
    }
    Ok(())
}
fn background_mode_environment(args:&clap::ArgMatches)->io::Result<()> {
    let Some(mode)=args.get_one::<String>("background") else{return Ok(())};
    require_shared_backends(args).map_err(|_|failure("background requires exactly one OMP and one Codex backend"))?;
    if args.get_one::<u32>("api")!=Some(&34){return Err(failure("background requires the selected original API34 owned emulator; never a paired physical phone"));}
    if ["GOATR_OWNED_CODEX_DIAGNOSTIC", "GOATR_OWNED_OMP_CREATION_CAPTURE",
        "GOATR_OWNED_OMP_OFFLINE_CAPTURE","GOATR_OWNED_OMP_BACKEND_CAPTURE","GOATR_OWNED_HELLO_CAPTURE","GOATR_OWNED_OMP_PRODUCER_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER","GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256","GOATR_OWNED_OMP_PRODUCER_BUN",
        "GOATR_OWNED_OMP_PRODUCER_BUN_SHA256","GOATR_OWNED_HERDR_REPORT_BIN","GOATR_OWNED_HERDR_REPORT_SHA256","GOATR_OWNED_GRID_OPEN_CAPTURE"]
        .iter().any(|key|std::env::var_os(key).is_some()){return Err(failure("background conflicts with inherited private/provider diagnostics"));}
    for (pin,expected) in [("expected-ntfy-apk-sha256",NTFY_APK_SHA256),("expected-ntfy-signer-sha256",NTFY_SIGNER_SHA256)] {
        match args.get_one::<String>(pin) {
            Some(value) if mode=="ntfy"&&value==expected=>{},
            None if mode=="persistent"=>{},
            _=>return Err(failure("background ntfy requires the exact qualified official F-Droid1.25.2 APK/signer pair; persistent accepts neither")),
        }
    }
    Ok(())
}
fn physical_mode_environment(args:&clap::ArgMatches)->io::Result<()> {
    let full=args.get_flag("physical-mvp");let pairing=args.get_flag("physical-pairing-checkpoint");let retirement=args.get_flag("physical-owned-retirement");
    let process_inventory=args.get_flag("physical-owned-process-inventory");let process_stop=args.get_flag("physical-owned-process-stop");
    if !process_inventory&&!process_stop&&matches!((args.get_one::<String>("expected-goatr-sha256"),args.get_one::<String>("expected-companion-lock-sha256")),(None,Some(_))) {return Err(failure("frozen development companion lock requires its exact binary hash"));}
    if !full&&!pairing&&!retirement&&!process_inventory&&!process_stop {
        if args.get_one::<String>("expected-config-sha256").is_some(){return Err(failure("independent seed config pin is physical-only"));}
        if args.get_one::<String>("expected-controller-sha256").is_some(){return Err(failure("controller hash is physical-only"));}
        if args.get_one::<PathBuf>("prepared-fresh-install").is_some()||args.get_one::<String>("expected-prepared-install-sha256").is_some(){return Err(failure("prepared install inputs are physical-mvp only"));}
        return Ok(());
    }
    physical::validate_controller_unit(args.get_one::<String>("expected-controller-unit").ok_or_else(||failure("explicit physical controller unit missing"))?)?;
    if args.get_one::<String>("expected-controller-sha256").is_none_or(|pin|pin.len()!=64||!pin.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))) {
        return Err(prerequisite("independent_controller_unit_pin_unavailable"));
    }
    if process_inventory||process_stop {
        let allowed=if process_inventory {vec!["physical-owned-process-inventory","owned-process-source-manifest","expected-owned-process-source-manifest-sha256",
            "owned-app-retirement-gate","expected-owned-app-retirement-gate-sha256","expected-controller-unit","expected-controller-sha256",
            "expected-companion-lock-sha256","evidence-dir"]}else {vec!["physical-owned-process-stop","owned-process-stop-approval","expected-owned-process-stop-approval-sha256",
            "owned-process-source-manifest","expected-owned-process-source-manifest-sha256","owned-app-retirement-gate","expected-owned-app-retirement-gate-sha256",
            "expected-controller-unit","expected-controller-sha256","expected-companion-lock-sha256","evidence-dir"]};
        let command=feedback_arguments(arguments_base());
        if command.get_arguments().any(|argument|args.value_source(argument.get_id().as_str())==Some(clap::parser::ValueSource::CommandLine)
            &&!allowed.contains(&argument.get_id().as_str())) {
            return Err(failure("owned process inventory/stop accepts only its closed approval/source/completion/unit/lock/evidence inputs"));
        }
        if args.get_one::<String>("expected-owned-process-source-manifest-sha256").is_none_or(|pin|pin.len()!=64||!pin.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)))
            ||args.get_one::<String>("expected-owned-app-retirement-gate-sha256").is_none_or(|pin|pin.len()!=64||!pin.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)))
            ||process_stop&&args.get_one::<String>("expected-owned-process-stop-approval-sha256").is_none_or(|pin|pin.len()!=64||!pin.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)))
            ||args.get_one::<String>("expected-companion-lock-sha256").map(String::as_str)!=Some(transport::lock_sha256()) {
            return Err(failure("owned process inventory requires its exact inventory/source/completion/lock pins"));
        }
        if ["GOATR_OWNED_CODEX_DIAGNOSTIC","GOATR_OWNED_OMP_CREATION_CAPTURE","GOATR_OWNED_OMP_OFFLINE_CAPTURE",
            "GOATR_OWNED_OMP_BACKEND_CAPTURE","GOATR_OWNED_HELLO_CAPTURE","GOATR_OWNED_OMP_PRODUCER_CAPTURE",
            "GOATR_OWNED_OMP_PRODUCER_METADATA_READER","GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256",
            "GOATR_OWNED_OMP_PRODUCER_BUN","GOATR_OWNED_OMP_PRODUCER_BUN_SHA256","GOATR_OWNED_HERDR_REPORT_BIN",
            "GOATR_OWNED_HERDR_REPORT_SHA256","GOATR_OWNED_GRID_OPEN_CAPTURE"].iter().any(|name|std::env::var_os(name).is_some()) {
            return Err(failure("owned process inventory rejects inherited native/provider/private capture"));
        }
        return Ok(());
    }
    if args.get_one::<String>("expected-config-sha256").is_none_or(|pin|pin.len()!=64||!pin.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte))) {
        return Err(prerequisite("independent_seed_config_pin_unavailable"));
    }
    if pairing||retirement {
        if retirement && (args.get_many::<String>("backend").into_iter().flatten().next().is_some() || args.get_flag("consent-owned-providers")) {return Err(failure("owned retirement rejects provider backend and consent"));}
        retained_pairing_provider_create(args)?;
        physical::validate_retained_subject_pin_arguments(args)?;
        if args.value_source("api")==Some(clap::parser::ValueSource::CommandLine) {return Err(failure("retained pairing discovers the selected physical API/ABI; explicit --api is forbidden"));}
        if ["GOATR_OWNED_CODEX_DIAGNOSTIC","GOATR_OWNED_OMP_CREATION_CAPTURE","GOATR_OWNED_OMP_OFFLINE_CAPTURE",
            "GOATR_OWNED_OMP_BACKEND_CAPTURE","GOATR_OWNED_HELLO_CAPTURE","GOATR_OWNED_OMP_PRODUCER_CAPTURE",
            "GOATR_OWNED_OMP_PRODUCER_METADATA_READER","GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256",
            "GOATR_OWNED_OMP_PRODUCER_BUN","GOATR_OWNED_OMP_PRODUCER_BUN_SHA256","GOATR_OWNED_HERDR_REPORT_BIN",
            "GOATR_OWNED_HERDR_REPORT_SHA256","GOATR_OWNED_GRID_OPEN_CAPTURE"].iter().any(|name|std::env::var_os(name).is_some()) {
            return Err(failure("retained pairing refuses inherited private/provider/Hello/grid capture"));
        }
        return Ok(());
    }
    match (args.get_one::<PathBuf>("prepared-fresh-install"),args.get_one::<String>("expected-prepared-install-sha256")) {
        (None,None)=>{},
        (Some(_),Some(hash)) if hash.len()==64&&hash.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte))=>{},
        _=>return Err(failure("prepared install requires a record and independent lowercase64 SHA256 pair")),
    }
    require_shared_backends(args).map_err(|_|failure("physical-mvp requires exactly one OMP and one Codex backend"))?;
    if args.value_source("api")==Some(clap::parser::ValueSource::CommandLine) {
        return Err(failure("physical-mvp discovers the selected physical API/ABI; explicit --api is forbidden"));
    }
    if ["GOATR_OWNED_CODEX_DIAGNOSTIC","GOATR_OWNED_OMP_CREATION_CAPTURE","GOATR_OWNED_OMP_OFFLINE_CAPTURE",
        "GOATR_OWNED_OMP_BACKEND_CAPTURE","GOATR_OWNED_HELLO_CAPTURE","GOATR_OWNED_OMP_PRODUCER_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER","GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256",
        "GOATR_OWNED_OMP_PRODUCER_BUN","GOATR_OWNED_OMP_PRODUCER_BUN_SHA256","GOATR_OWNED_HERDR_REPORT_BIN",
        "GOATR_OWNED_HERDR_REPORT_SHA256","GOATR_OWNED_GRID_OPEN_CAPTURE"].iter().any(|name|std::env::var_os(name).is_some()) {
        return Err(failure("physical-mvp refuses inherited private/provider/Hello/grid capture"));
    }
    for (name,expected) in [
        ("expected-ntfy-apk-sha256",NTFY_APK_SHA256),
        ("expected-ntfy-signer-sha256",NTFY_SIGNER_SHA256),
    ] {
        if args.get_one::<String>(name).map(String::as_str)!=Some(expected) {
            return Err(failure("physical-mvp requires the exact qualified official F-Droid ntfy1.25.2 APK and signer pins"));
        }
    }
    Ok(())
}
fn physical_host_preflight_selection(args:&clap::ArgMatches)->io::Result<Option<physical::ControllerPolicy<'_>>> {
    let policy=match (args.get_flag("physical-host-preflight"),args.get_flag("physical-host-wake-preflight")) {
        (false,false)=>return Ok(None),
        (true,false)=>physical::ControllerPolicy::Zero,
        (false,true)=>{
            let unit=args.get_one::<String>("expected-controller-unit").ok_or_else(||failure("Wake diagnostic requires the exact controller unit"))?;
            physical::validate_controller_unit(unit)?;physical::ControllerPolicy::WakeDiagnostic {unit}
        }
        (true,true)=>return Err(failure("host preflight policies are mutually exclusive")),
    };
    // ArgMatches also carries derived group IDs with CommandLine provenance; only
    // actual declared arguments can be explicit frontend inputs.
    let command=feedback_arguments(arguments_base());
    if command.get_arguments().any(|argument|args.value_source(argument.get_id().as_str())==Some(clap::parser::ValueSource::CommandLine)
        &&!match policy {
            physical::ControllerPolicy::Zero=>matches!(argument.get_id().as_str(),"physical-host-preflight"|"evidence-dir"),
            physical::ControllerPolicy::WakeDiagnostic {..}=>matches!(argument.get_id().as_str(),"physical-host-wake-preflight"|"expected-controller-unit"|"evidence-dir"),
            physical::ControllerPolicy::UnitWake {..}=>false,
        }) {
        return Err(failure("host preflight accepts only its closed selector/unit/evidence inputs; no runtime/device/artifact/effect arguments"));
    }
    Ok(Some(policy))
}
fn arguments_base() -> Command {
    Command::new("smoke")
        .arg(Arg::new("prepared-fresh-install").long("prepared-fresh-install").value_parser(value_parser!(PathBuf))
            .requires_all(["physical-mvp","expected-prepared-install-sha256"]).help("Explicit sealed0400 owned fresh-install creator record; skips ONLY the already-owned PRODUCT install, never baseline/runtime"))
        .arg(Arg::new("expected-prepared-install-sha256").long("expected-prepared-install-sha256")
            .requires_all(["physical-mvp","prepared-fresh-install"]).help("Independent lowercase64 creator-record pin, not measured candidate authority"))
        .arg(Arg::new("expected-controller-sha256").long("expected-controller-sha256")
            .help("Independent lowercase SHA256 pin of the held controller executable; never measured self-authorization"))
        .arg(Arg::new("physical-host-wake-preflight").long("physical-host-wake-preflight").action(ArgAction::SetTrue)
            .requires_all(["expected-controller-unit","evidence-dir"]).conflicts_with("physical-host-preflight")
            .help("Explicit real-host TTY diagnostic ONLY: exact four-set Wake capability shape plus expected actual controller unit, never full-matrix/runtime authority"))
        .arg(Arg::new("expected-controller-unit").long("expected-controller-unit").requires("controller-unit-mode")
            .help("Explicit expected fresh goatr-host-wake-<canonical-lower-UUID>.service; joined to the original manager, MainPID and full cgroup"))
        .group(ArgGroup::new("controller-unit-mode").args(["physical-mvp","physical-pairing-checkpoint","physical-owned-retirement","physical-owned-process-inventory","physical-owned-process-stop","physical-host-wake-preflight"]))
        .group(ArgGroup::new("owned-process-mode").args(["physical-owned-process-inventory","physical-owned-process-stop"]))
        .group(ArgGroup::new("retained-subject-mode").args(["physical-pairing-checkpoint","physical-owned-retirement"]))
        .arg(Arg::new("physical-host-preflight").long("physical-host-preflight").action(ArgAction::SetTrue)
            .requires("evidence-dir")
            .help("Human real-host TTY only: qualify the unchanged original-host guard and record closed permission diagnostics; evidence only, no native/provider/ADB/product effect or full-MVP claim"))
        .arg(Arg::new("physical-mvp").long("physical-mvp").action(ArgAction::SetTrue)
            .requires_all(["backend","goatr","config","workspace-root","evidence-dir","consent-owned-providers",
                "apk","test-apk","expected-apk-sha256","expected-test-apk-sha256","expected-goatr-sha256",
                "expected-companion-lock-sha256","expected-config-sha256","serial","expected-ntfy-apk-sha256","expected-ntfy-signer-sha256","expected-controller-unit","expected-controller-sha256"])
            .conflicts_with_all(["background","admission-only","shared-agent-only","terminal-only","project-read-only",
                "grid","grid-lease","ui","camera","ime","setup","prepare-emulator","emulator-owner","paging-proof",
                "codex-invalidation-diagnostic","omp-creation-diagnostic","physical-hello-capture","physical-relay-ephemeral",
                "development-continuity","development-owner-feedback","development-android-only",
                "keep-foreground-for-feedback","defer-retention-crossing","admin-socket","disposable-uid","automate-native-inputs","physical-pairing-checkpoint"])
            .help("Stock BOTH-provider physical MVP: explicit genuine PRODUCT absence OR pinned owned-prepared fresh install; same-signer TEST update and full camera/baseline/observer chain, no arbitrary installed-app adoption"))
        .arg(Arg::new("physical-pairing-checkpoint").long("physical-pairing-checkpoint").action(ArgAction::SetTrue)
            .requires_all(["goatr","config","workspace-root","evidence-dir","apk","test-apk","expected-apk-sha256","expected-test-apk-sha256","expected-current-test-apk-sha256","expected-product-uid","expected-product-first-install-time","expected-product-data-dir","expected-test-uid","expected-test-first-install-time","expected-test-data-dir","expected-goatr-sha256","expected-companion-lock-sha256","expected-config-sha256","serial","expected-controller-unit","expected-controller-sha256"])
            .conflicts_with_all(["physical-mvp","physical-host-preflight","physical-host-wake-preflight","background","admission-only","shared-agent-only","terminal-only","project-read-only","grid","grid-lease","ui","camera","ime","setup","prepare-emulator","emulator-owner","paging-proof","codex-invalidation-diagnostic","omp-creation-diagnostic","physical-hello-capture","physical-relay-ephemeral","development-continuity","development-owner-feedback","development-android-only","keep-foreground-for-feedback","defer-retention-crossing","admin-socket","disposable-uid","automate-native-inputs","expected-ntfy-apk-sha256","expected-ntfy-signer-sha256","prepared-fresh-install","expected-prepared-install-sha256"])
            .help("One retained installed-subject private Paste/Preview/Confirm pairing checkpoint: PRODUCT is qualified only, TEST may receive one same-signer in-place update, and all stock resources remain retained"))
        .arg(Arg::new("physical-owned-retirement").long("physical-owned-retirement").action(ArgAction::SetTrue)
            .requires_all(["goatr","config","workspace-root","evidence-dir","apk","test-apk","expected-apk-sha256","expected-test-apk-sha256","expected-current-test-apk-sha256","expected-product-uid","expected-product-first-install-time","expected-product-data-dir","expected-test-uid","expected-test-first-install-time","expected-test-data-dir","expected-goatr-sha256","expected-companion-lock-sha256","expected-config-sha256","serial","expected-controller-unit","expected-controller-sha256","retirement-target-manifest","expected-retirement-manifest-sha256"])
            .conflicts_with_all(["physical-mvp","physical-pairing-checkpoint","physical-pairing-shell-checkpoint","physical-pairing-shell-interaction-checkpoint","physical-host-preflight","physical-host-wake-preflight","background","admission-only","shared-agent-only","terminal-only","project-read-only","grid","grid-lease","ui","camera","ime","setup","prepare-emulator","emulator-owner","paging-proof","codex-invalidation-diagnostic","omp-creation-diagnostic","physical-hello-capture","physical-relay-ephemeral","development-continuity","development-owner-feedback","development-android-only","keep-foreground-for-feedback","defer-retention-crossing","admin-socket","disposable-uid","automate-native-inputs","expected-ntfy-apk-sha256","expected-ntfy-signer-sha256","prepared-fresh-install","expected-prepared-install-sha256","backend","consent-owned-providers"])
            .help("Remove only root-reviewed locally owned old host rows through the installed product Forget UI; target names remain private to authenticated control"))
        .arg(Arg::new("retirement-target-manifest").long("retirement-target-manifest").value_parser(value_parser!(PathBuf)).requires("physical-owned-retirement"))
        .arg(Arg::new("expected-retirement-manifest-sha256").long("expected-retirement-manifest-sha256").value_parser(value_parser!(String)).requires("physical-owned-retirement"))
        .arg(Arg::new("physical-owned-process-inventory").long("physical-owned-process-inventory").action(ArgAction::SetTrue)
            .requires_all(["owned-process-source-manifest","expected-owned-process-source-manifest-sha256","owned-app-retirement-gate","expected-owned-app-retirement-gate-sha256","expected-controller-unit","expected-controller-sha256","expected-companion-lock-sha256","evidence-dir"])
            .conflicts_with_all(["physical-mvp","physical-pairing-checkpoint","physical-owned-retirement","physical-pairing-shell-checkpoint","physical-pairing-shell-interaction-checkpoint","physical-host-preflight","physical-host-wake-preflight","background","admission-only","shared-agent-only","terminal-only","project-read-only","grid","grid-lease","ui","camera","ime","setup","prepare-emulator","emulator-owner","paging-proof","codex-invalidation-diagnostic","omp-creation-diagnostic","physical-hello-capture","physical-relay-ephemeral","development-continuity","development-owner-feedback","development-android-only","keep-foreground-for-feedback","defer-retention-crossing","admin-socket","disposable-uid","automate-native-inputs","expected-ntfy-apk-sha256","expected-ntfy-signer-sha256","prepared-fresh-install","expected-prepared-install-sha256","backend","consent-owned-providers","goatr","config","workspace-root","serial","apk","test-apk","expected-apk-sha256","expected-test-apk-sha256","expected-current-test-apk-sha256","expected-product-uid","expected-product-first-install-time","expected-product-data-dir","expected-test-uid","expected-test-first-install-time","expected-test-data-dir","expected-goatr-sha256","expected-config-sha256","retirement-target-manifest","expected-retirement-manifest-sha256"])
            .help("Read-only root-reviewed old-process inventory only; no stop, pidfd, native or device path"))
        .arg(Arg::new("physical-owned-process-stop").long("physical-owned-process-stop").action(ArgAction::SetTrue)
            .requires_all(["owned-process-stop-approval","expected-owned-process-stop-approval-sha256","owned-process-source-manifest","expected-owned-process-source-manifest-sha256","owned-app-retirement-gate","expected-owned-app-retirement-gate-sha256","expected-controller-unit","expected-controller-sha256","expected-companion-lock-sha256","evidence-dir"])
            .conflicts_with_all(["physical-mvp","physical-pairing-checkpoint","physical-owned-retirement","physical-owned-process-inventory","physical-pairing-shell-checkpoint","physical-pairing-shell-interaction-checkpoint","physical-host-preflight","physical-host-wake-preflight","background","admission-only","shared-agent-only","terminal-only","project-read-only","grid","grid-lease","ui","camera","ime","setup","prepare-emulator","emulator-owner","paging-proof","codex-invalidation-diagnostic","omp-creation-diagnostic","physical-hello-capture","physical-relay-ephemeral","development-continuity","development-owner-feedback","development-android-only","keep-foreground-for-feedback","defer-retention-crossing","admin-socket","disposable-uid","automate-native-inputs","expected-ntfy-apk-sha256","expected-ntfy-signer-sha256","prepared-fresh-install","expected-prepared-install-sha256","backend","consent-owned-providers","goatr","config","workspace-root","serial","apk","test-apk","expected-apk-sha256","expected-test-apk-sha256","expected-current-test-apk-sha256","expected-product-uid","expected-product-first-install-time","expected-product-data-dir","expected-test-uid","expected-test-first-install-time","expected-test-data-dir","expected-goatr-sha256","expected-config-sha256","retirement-target-manifest","expected-retirement-manifest-sha256"])
            .help("Root-reviewed pidfd-only stop of an exact wholly proved inventory subset; no systemd, cgroup, native, or device fallback"))
        .arg(Arg::new("owned-process-stop-approval").long("owned-process-stop-approval").value_parser(value_parser!(PathBuf)).requires("physical-owned-process-stop"))
        .arg(Arg::new("expected-owned-process-stop-approval-sha256").long("expected-owned-process-stop-approval-sha256").requires("physical-owned-process-stop"))
        .arg(Arg::new("owned-process-source-manifest").long("owned-process-source-manifest").value_parser(value_parser!(PathBuf)).requires("owned-process-mode"))
        .arg(Arg::new("expected-owned-process-source-manifest-sha256").long("expected-owned-process-source-manifest-sha256").requires("owned-process-mode"))
        .arg(Arg::new("owned-app-retirement-gate").long("owned-app-retirement-gate").value_parser(value_parser!(PathBuf)).requires("owned-process-mode"))
        .arg(Arg::new("expected-owned-app-retirement-gate-sha256").long("expected-owned-app-retirement-gate-sha256").requires("owned-process-mode"))
        .arg(Arg::new("physical-pairing-shell-checkpoint").long("physical-pairing-shell-checkpoint").action(ArgAction::SetTrue)
            .requires("physical-pairing-checkpoint").conflicts_with_all(["backend","consent-owned-providers"])
            .help("Extend the retained pairing checkpoint with one fresh stock ordinary-shell Begin, one Enter, Release, and viewer-only close; no provider creation"))
        .arg(Arg::new("physical-pairing-shell-interaction-checkpoint").long("physical-pairing-shell-interaction-checkpoint").action(ArgAction::SetTrue)
            .requires("physical-pairing-shell-checkpoint").conflicts_with_all(["backend","consent-owned-providers"])
            .help("Extend the retained ordinary-shell checkpoint with one owned Draft/Live input sequence and measured history controls; no provider creation"))
        .arg(Arg::new("background").long("background").value_parser(["persistent","ntfy"])
            .requires_all([
                "backend",
                "goatr",
                "config",
                "workspace-root",
                "evidence-dir",
                "consent-owned-providers",
                "automate-native-inputs",
                "apk",
                "test-apk",
                "expected-apk-sha256",
                "expected-test-apk-sha256",
                "expected-goatr-sha256",
                "expected-companion-lock-sha256",
                "serial",
                "emulator-owner",
            ])
            .conflicts_with_all([
                "admission-only",
                "shared-agent-only",
                "terminal-only",
                "project-read-only",
                "grid",
                "grid-lease",
                "ui",
                "camera",
                "ime",
                "setup",
                "prepare-emulator",
                "paging-proof",
                "codex-invalidation-diagnostic",
                "omp-creation-diagnostic",
                "physical-hello-capture",
                "physical-relay-ephemeral",
                "development-continuity",
                "development-owner-feedback",
                "development-android-only",
                "keep-foreground-for-feedback",
                "defer-retention-crossing",
                "admin-socket",
                "disposable-uid",
            ])
            .help("Finite owned-emulator persistent or official F-Droid ntfy1.25.2 registration/probe/ACK/native attention/dedup/tap gate for BOTH original providers; retains native resources, no fault stop or replay"))
        .arg(Arg::new("expected-ntfy-apk-sha256").long("expected-ntfy-apk-sha256").requires("notification-artifact-mode")
            .help("Independent exact APK pin for official F-Droid https://f-droid.org/repo/io.heckel.ntfy_63.apk (1.25.2/code63); not the upstream GitHub fdroid-release build"))
        .arg(Arg::new("expected-ntfy-signer-sha256").long("expected-ntfy-signer-sha256").requires("notification-artifact-mode")
            .help("Independent exact official F-Droid APK-signer pin; physical-mvp/background ntfy only, never persistent"))
        .arg(Arg::new("expected-config-sha256").long("expected-config-sha256").requires("config").help("Independent caller SHA-256 of the canonical0600 neutral seed; lowercase64, physical-only, never measured by the runner to authorize its candidate"))
        .arg(Arg::new("terminal-only").long("terminal-only").action(ArgAction::SetTrue)
            .requires_all([
                "goatr",
                "config",
                "workspace-root",
                "evidence-dir",
                "consent-owned-providers",
                "automate-native-inputs",
            ])
            .conflicts_with_all([
                "backend",
                "admission-only",
                "shared-agent-only",
                "grid",
                "grid-lease",
                "ui",
                "camera",
                "ime",
                "setup",
                "prepare-emulator",
                "paging-proof",
                "codex-invalidation-diagnostic",
                "omp-creation-diagnostic",
                "physical-hello-capture",
                "physical-relay-ephemeral",
                "development-continuity",
                "development-owner-feedback",
                "development-android-only",
                "keep-foreground-for-feedback",
                "defer-retention-crossing",
                "apk",
                "test-apk",
                "expected-apk-sha256",
                "expected-test-apk-sha256",
                "serial",
                "emulator-owner",
                "admin-socket",
                "disposable-uid",
                "api",
            ])
            .help("Finite stock cells/resize/release, Goatr text/key frame handoff and configured-project terminal creation; one guarded native printf fixture, no providers; originals retained"))
        .arg(Arg::new("shared-agent-only").long("shared-agent-only").action(ArgAction::SetTrue)
            .requires_all([
                "backend",
                "goatr",
                "config",
                "workspace-root",
                "consent-owned-providers",
                "automate-native-inputs",
            ])
            .conflicts_with_all([
                "admission-only",
                "grid",
                "ui",
                "camera",
                "ime",
                "setup",
                "prepare-emulator",
                "paging-proof",
                "codex-invalidation-diagnostic",
                "omp-creation-diagnostic",
                "physical-hello-capture",
                "physical-relay-ephemeral",
                "development-continuity",
                "development-owner-feedback",
                "development-android-only",
                "keep-foreground-for-feedback",
                "defer-retention-crossing",
                "apk",
                "test-apk",
                "serial",
                "emulator-owner",
                "admin-socket",
                "disposable-uid",
            ])
            .help("Finite fresh BOTH-provider shared conversation/requests/Stop/reconnect/native-history gate; retains all native resources"))
        .arg(Arg::new("admission-only").long("admission-only").action(ArgAction::SetTrue)
            .requires_all([
                "backend",
                "goatr",
                "config",
                "workspace-root",
                "consent-owned-providers",
            ])
            .conflicts_with_all([
                "grid",
                "ui",
                "camera",
                "ime",
                "setup",
                "prepare-emulator",
                "paging-proof",
                "codex-invalidation-diagnostic",
                "omp-creation-diagnostic",
                "physical-hello-capture",
                "physical-relay-ephemeral",
                "development-continuity",
                "development-owner-feedback",
                "development-android-only",
                "keep-foreground-for-feedback",
                "defer-retention-crossing",
                "automate-native-inputs",
                "apk",
                "test-apk",
                "serial",
                "emulator-owner",
                "admin-socket",
                "disposable-uid",
            ])
            .help("Finite owned Hello/current-source/creation eligibility only; no provider creation or input; resources retained"))
        .arg(Arg::new("project-read-only").long("project-read-only").action(ArgAction::SetTrue)
            .requires_all([
                "backend",
                "goatr",
                "config",
                "workspace-root",
                "evidence-dir",
                "consent-owned-providers",
            ])
            .conflicts_with_all([
                "admission-only",
                "shared-agent-only",
                "terminal-only",
                "grid",
                "grid-lease",
                "ui",
                "camera",
                "ime",
                "setup",
                "prepare-emulator",
                "paging-proof",
                "codex-invalidation-diagnostic",
                "omp-creation-diagnostic",
                "physical-hello-capture",
                "physical-relay-ephemeral",
                "development-continuity",
                "development-owner-feedback",
                "development-android-only",
                "keep-foreground-for-feedback",
                "defer-retention-crossing",
                "automate-native-inputs",
                "apk",
                "test-apk",
                "expected-apk-sha256",
                "expected-test-apk-sha256",
                "serial",
                "emulator-owner",
                "admin-socket",
                "disposable-uid",
                "api",
            ])
            .help("Finite BOTH-provider/read-only Files/Git fixture gate after ordinary admission; existing qualified signed public seed only, no input or new commits; originals retained"))
        .arg(Arg::new("project-read-seed-pack").long("project-read-seed-pack").value_parser(value_parser!(PathBuf))
            .requires_all(["project-read-only","project-read-seed-pack-sha256","project-read-seed-commit","project-read-seed-tree","project-read-seed-signature-sha256"]))
        .arg(Arg::new("project-read-seed-pack-sha256").long("project-read-seed-pack-sha256")
            .requires_all(["project-read-only","project-read-seed-pack","project-read-seed-commit","project-read-seed-tree","project-read-seed-signature-sha256"]))
        .arg(Arg::new("project-read-seed-commit").long("project-read-seed-commit")
            .requires_all(["project-read-only","project-read-seed-pack","project-read-seed-pack-sha256","project-read-seed-tree","project-read-seed-signature-sha256"]))
        .arg(Arg::new("project-read-seed-tree").long("project-read-seed-tree")
            .requires_all(["project-read-only","project-read-seed-pack","project-read-seed-pack-sha256","project-read-seed-commit","project-read-seed-signature-sha256"]))
        .arg(Arg::new("project-read-seed-signature-sha256").long("project-read-seed-signature-sha256")
            .requires_all(["project-read-only","project-read-seed-pack","project-read-seed-pack-sha256","project-read-seed-commit","project-read-seed-tree"]))

        .group(
            ArgGroup::new("frozen-development-package")
                .args([
                    "development-continuity",
                    "codex-invalidation-diagnostic",
                    "physical-hello-capture",
                    "omp-creation-diagnostic",
                    "grid",
                    "terminal-only",
                    "project-read-only",
                    "background",
                    "physical-mvp",
                    "physical-pairing-checkpoint",
                    "physical-owned-retirement",
                ]),
        )
        .group(ArgGroup::new("android-artifact-mode").args(["development-continuity","background","physical-mvp","physical-pairing-checkpoint","physical-owned-retirement"]))
        .group(ArgGroup::new("notification-artifact-mode").args(["background","physical-mvp"]))
        .arg(Arg::new("grid").long("grid").action(ArgAction::SetTrue)
            .requires_all([
                "grid-lease",
                "backend",
                "goatr",
                "config",
                "workspace-root",
                "evidence-dir",
                "expected-goatr-sha256",
                "expected-companion-lock-sha256",
                "consent-owned-providers",
                "automate-native-inputs",
            ])
            .conflicts_with_all([
                "ui",
                "camera",
                "ime",
                "setup",
                "prepare-emulator",
                "paging-proof",
                "codex-invalidation-diagnostic",
                "omp-creation-diagnostic",
                "physical-hello-capture",
                "physical-relay-ephemeral",
                "development-continuity",
                "development-owner-feedback",
                "development-android-only",
                "keep-foreground-for-feedback",
                "defer-retention-crossing",
                "apk",
                "test-apk",
                "expected-apk-sha256",
                "expected-test-apk-sha256",
                "serial",
                "emulator-owner",
                "admin-socket",
            ])
            .help("Qualified fresh native-to-framed-grid witness only; retains supervisor/resources, never phone or cleanup authority"))
        .arg(Arg::new("grid-lease").long("grid-lease").value_parser(value_parser!(PathBuf)).requires("grid"))
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
                    "automate-native-inputs",
                    "development-continuity",
                    "defer-retention-crossing",
                    "keep-foreground-for-feedback",
                ])
                .help("Fresh Codex one-action status-only creation diagnostic; retains resources and performs no native input, proof, cleanup, or Android flow"),
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
            Arg::new("physical-relay-ephemeral")
                .long("physical-relay-ephemeral")
                .action(ArgAction::SetTrue)
                .requires("physical-hello-capture")
                .help("Backend-only owned pre-Create capture on one kernel-selected loopback relay port; leaves the paired controller untouched"),
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
                .requires("expected-companion-lock-sha256")
                .help("Required frozen SHA-256 for the exact development companion binary"),
        )
        .arg(
            Arg::new("expected-companion-lock-sha256")
                .long("expected-companion-lock-sha256")
                .value_parser(value_parser!(String))
                .help("Required frozen source lock SHA-256 for the development companion"),
        )
        .arg(
            Arg::new("expected-apk-sha256")
                .long("expected-apk-sha256")
                .value_parser(value_parser!(String))
                .requires("android-artifact-mode")
                .help("Required frozen SHA-256 for the selected development/background debug APK"),
        )
        .arg(
            Arg::new("expected-test-apk-sha256")
                .long("expected-test-apk-sha256")
                .value_parser(value_parser!(String))
                .requires("android-artifact-mode")
                .help("Required frozen SHA-256 for the selected development/background debug test APK"),
        )
        .arg(Arg::new("expected-current-test-apk-sha256").long("expected-current-test-apk-sha256").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent SHA-256 pin for the presently installed retained TEST APK before its one permitted update"))
        .arg(Arg::new("expected-product-uid").long("expected-product-uid").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent retained PRODUCT UID pin"))
        .arg(Arg::new("expected-product-first-install-time").long("expected-product-first-install-time").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent retained PRODUCT first-install epoch pin"))
        .arg(Arg::new("expected-product-data-dir").long("expected-product-data-dir").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent retained PRODUCT data-directory pin"))
        .arg(Arg::new("expected-test-uid").long("expected-test-uid").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent retained TEST UID pin"))
        .arg(Arg::new("expected-test-first-install-time").long("expected-test-first-install-time").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent retained TEST first-install epoch pin"))
        .arg(Arg::new("expected-test-data-dir").long("expected-test-data-dir").value_parser(value_parser!(String))
            .requires("retained-subject-mode").help("Independent retained TEST data-directory pin"))
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
                .help("Explicitly create retained fresh named Herdr/project resources; terminal-only creates no provider and permits its guarded public fixture, staged Goatr text/keys and one configured-project tab"),
        )
        .arg(
            Arg::new("automate-native-inputs")
                .long("automate-native-inputs")
                .requires("consent-owned-providers")
                .conflicts_with_all(["camera", "ime", "setup", "prepare-emulator"])
                .action(ArgAction::SetTrue)
                .help("Consent to once-only guarded owned TUI inputs, or the terminal-only public printf fixture plus Goatr staged text/key handoff; never replay uncertain input"),
        )
        .arg(Arg::new("apk").long("apk").value_parser(value_parser!(PathBuf)))
        .arg(
            Arg::new("test-apk")
                .long("test-apk")
                .value_parser(value_parser!(PathBuf)),
        )
}
fn packaged(args: &clap::ArgMatches) -> io::Result<PathBuf> {
    if args.get_flag("terminal-only")&&terminal_frozen_selection(args)? {
        return dev_continuity::frozen_package(args);
    }
    if args.get_flag("codex-invalidation-diagnostic")
        || args.get_flag("omp-creation-diagnostic")
        || args.get_flag("grid")
        || args.get_one::<String>("background").is_some()
        || args.get_flag("physical-mvp")
        || args.get_flag("physical-pairing-checkpoint") || args.get_flag("physical-owned-retirement")
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
        if !matches!(
            guard,
            "owner_start_settled"
                | "owner_child_status_returned"
                | "owner_native_initialize_entered"
                | "owner_pending_authorization"
                | "owner_pending_service_metadata"
                | "owner_pending_service_proof"
                | "owner_pending_native_process"
                | "owner_pending_socket_epoch"
                | "owner_pending_owner_record_proof"
        )
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
            | "workspace_confirm_tab"
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
            | "workspace_confirm_tab"
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
        | "workspace_confirm_focus" | "workspace_confirm_tab" => {
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
                            progress.accepted_guard(&value);
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
            | "coarse_shell"
    ) {
        return None;
    }
    let guard = fields.next()?.strip_prefix("guard=")?;
    let coarse_shell_guard = matches!(guard,
        "consent_absent" | "source_lookup" | "reporter_process" | "connect"
            | "peer_identity" | "process_info" | "shell_absent" | "reported_process"
            | "shell_executable" | "source_recheck" | "no_candidate" | "record_absent" | "installed"
    );
    if phase == "coarse_shell" && !coarse_shell_guard {
        return None;
    }
    let launch_preflight_guard = matches!(
        guard,
        "project_lookup" | "project_path" | "creation_overlay" | "creation_overlay_path"
            | "diagnostic_extension_args" | "source_lookup" | "topology_source"
            | "topology_source_changed" | "topology_project" | "topology_focus"
            | "creation_placement"
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
            | "creation_placement"
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
            | "creation_directory"
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
    ) && !(phase == "coarse_shell" && coarse_shell_guard) {
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
    if phase == "coarse_shell" {
        let shape = match guard {
            "installed" => code == "none" && io == "none",
            "consent_absent" | "peer_identity" | "shell_absent" | "no_candidate" | "record_absent" => code == "NotReady" && io == "none",
            "source_lookup" => matches!(code, "NotFound" | "NotReady" | "ResourceLimit") && io == "none",
            _ => matches!(code, "NotReady" | "ResourceLimit") && io != "none",
        };
        if !shape || frame != "none" {
            return None;
        }
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
    let deadline_detail=if phase=="shell_probe"&&guard=="result_deadline" {
        match fields.next() {
            None=>None,
            Some(site)=>Some((site.strip_prefix("deadlineSite=")?,
                fields.next()?.strip_prefix("observation=")?,fields.next()?.strip_prefix("entryBudget=")?)),
        }
    } else {None};
    if fields.next().is_some() {
        return None;
    }
    let mut value=json!({"gate":"owned-omp-creation-capture","phase":phase,"guard":guard,"code":code,"io":io,"frame":frame,"ack":ack});
    if let Some((site,observation,budget))=deadline_detail {
        value["deadlineSite"]=json!(site);value["observation"]=json!(observation);value["entryBudget"]=json!(budget);
    }
    herdr_report_capture::valid_probe_deadline(&value).then_some(value)
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
    )) || (phase == "codex_manager_hello" && code == "NotReady" && matches!(
        guard,
        "runtime_missing" | "runtime_path" | "runtime_directory" | "socket_before"
            | "socket_create" | "socket_path" | "connect" | "peer_credentials" | "peer_uid"
            | "socket_after" | "socket_changed" | "peer_pid" | "process_evidence"
            | "process_namespace" | "process_stat" | "process_pid" | "process_uid"
            | "process_boot" | "process_changed"
    ) && matches!(
        io,
        "none" | "not_found" | "permission_denied" | "connection_refused" | "connection_reset"
            | "connection_aborted" | "not_connected" | "addr_in_use" | "addr_not_available"
            | "broken_pipe" | "already_exists" | "would_block" | "invalid_input" | "invalid_data"
            | "timed_out" | "interrupted" | "unexpected_eof" | "unsupported" | "out_of_memory"
            | "write_zero" | "other"
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
    start_serve_with_codex_capture(binary, config, root, socket, listener, false, None)
}
fn start_serve_with_codex_capture(
    binary: &Path,
    config: &Path,
    root: &Path,
    socket: &Path,
    listener: Option<&std::net::TcpListener>,
    diagnostic_capture: bool,
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
            herdr_report_capture,
            preserve_xdg_config_home: false,
            retain_process: false,
            hello_capture: false,
            grid_hold: None,
            project_read: None,
            startup_deadline: None,
            unit_authority: None,
        },
    )
}
fn admit_first_child_evidence(process:ProcessEvidence,deadline:Instant,observed_at:Instant)->io::Result<ProcessEvidence> {
    if observed_at>=deadline{return Err(io::Error::new(io::ErrorKind::TimedOut,"original child proof completed after its absolute deadline"));}
    Ok(process)
}
fn first_child_evidence(child:&mut Child,original:&goatr::codex_relay::ProcessStart,executable:(u64,u64),deadline:Instant)->io::Result<ProcessEvidence> {
    loop {
        if child.try_wait()?.is_some(){return Err(failure("original child exited before its first protected identity proof"));}
        if Instant::now()>=deadline{return Err(io::Error::new(io::ErrorKind::TimedOut,"original child inspectability deadline expired"));}
        match original.evidence(child.id()) {
            Ok(process)=>{
                if child.try_wait()?.is_some(){return Err(failure("original child exited during its first protected identity proof"));}
                original.check(child.id())?;
                let object=fs::metadata(format!("/proc/{}/exe",child.id()))?;
                if (object.dev(),object.ino())!=executable{return Err(failure("first child proof is not the original executable object"));}
                return admit_first_child_evidence(process,deadline,Instant::now());
            }
            Err(error) if error.kind()==io::ErrorKind::PermissionDenied=>{
                if child.try_wait()?.is_some(){return Err(failure("original child exited while awaiting inspectability"));}
                original.check(child.id())?;
                std::thread::sleep(Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())));
            }
            Err(error)=>return Err(error),
        }
    }
}
fn start_serve_with_capture(
    binary: &Path,
    config: &Path,
    root: &Path,
    socket: &Path,
    listener: Option<&std::net::TcpListener>,
    mut launch: ServeLaunch<'_>,
) -> io::Result<Companion> {
    let startup_deadline=launch.startup_deadline.unwrap_or_else(||Instant::now()+Duration::from_secs(20));
    let source=fs::metadata(binary)?;let executable=(source.dev(),source.ino());
    let codex_capture = launch.diagnostic_capture
        || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let omp_creation_capture = owned_omp_creation_capture_requested();
    let omp_offline_capture = owned_omp_offline_capture_requested();
    let omp_backend_capture = owned_omp_backend_capture_requested();
    let hello_capture = owned_hello_capture_requested() || launch.hello_capture;
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
    if let Some(pin) = launch.unit_authority {
        pin.serving_controller(Some(startup_deadline))?;
        c.arg("--expected-unit").arg(pin.unit());
    }
    if !launch.preserve_xdg_config_home {
        c.env(
            "XDG_CONFIG_HOME",
            socket
                .parent()
                .ok_or_else(|| failure("owned admin parent missing"))?
                .join("config"),
        );
    }
    if let Some(fixture)=launch.project_read {fixture.current()?;fixture.environment(&mut c);}
    c.env("XDG_STATE_HOME", root.join("state"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    c.env_remove("GOATR_OWNED_CODEX_DIAGNOSTIC");
    c.env_remove("GOATR_OWNED_OMP_CREATION_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_OFFLINE_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_BACKEND_CAPTURE");
    c.env_remove("GOATR_OWNED_HELLO_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_PRODUCER_CAPTURE");
    c.env_remove("GOATR_OWNED_OMP_PRODUCER_OVERLAY_SHA256");
    c.env_remove("GOATR_OWNED_OMP_PRODUCER_HOOK_SHA256");
    if codex_capture {
        c.env("GOATR_OWNED_CODEX_DIAGNOSTIC", "1");
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
    if Instant::now()>=startup_deadline{return Err(io::Error::new(io::ErrorKind::TimedOut,"original startup budget expired before spawn"));}
    if let Some(pin) = launch.unit_authority { pin.serving_controller(Some(startup_deadline))?; }
    if Instant::now()>=startup_deadline{return Err(io::Error::new(io::ErrorKind::TimedOut,"original startup budget expired during unit qualification"));}
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
    let retain_process = launch.grid_hold.is_some() || launch.retain_process;
    if retain_process {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(target_os="linux")]
    if launch.unit_authority.is_none() {
        unsafe {
            use std::os::unix::process::CommandExt;
            c.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
    }
    let spawned = if Instant::now()>=startup_deadline {
        Err(io::Error::new(io::ErrorKind::TimedOut,"original startup budget expired immediately before spawn"))
    } else { c.spawn() };
    let pin = if let (Some(hold), Ok(child)) = (launch.grid_hold.as_deref_mut(), &spawned) {
        hold.pin_child(child)
    } else {
        Ok(())
    };
    let restored =
        inherited.is_none_or(|(fd, flags)| unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } >= 0);
    pin?;
    if !restored {
        return Err(io::Error::last_os_error());
    }
    let mut child = spawned?;
    if launch.retain_process {
        write_json(&root.join("admission-companion-spawn.json"),
            &json!({"pid":child.id(),"package":binary,"adminSocket":socket,
                "scope":"fresh owned child; process proof pending; no automatic teardown"}))?;
    }
    let original=goatr::codex_relay::ProcessStart::capture(child.id())?;
    let process=first_child_evidence(&mut child,&original,executable,startup_deadline.min(Instant::now()+Duration::from_secs(2)))?;
    if launch.retain_process {
        let manifest = root.join("admission-companion.json");
        write_json(&manifest, &json!({"process":process,"package":binary,"adminSocket":socket,
            "scope":"retained fresh owned companion; cleanup requires original identity review"}))?;
        println!("{}", json!({"gate":"admission-owned-companion","manifest":manifest}));
    }
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
        guard_drain,
        omp_capture_budget: omp_budget,
        retain_process,
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
fn physical_relay_bind_port(ephemeral: bool, physical: bool, mode: OwnedOmpCaptureMode, other_capture: bool) -> io::Result<u16> {
    if ephemeral && (!physical || mode != OwnedOmpCaptureMode::Backend || other_capture) {
        return Err(failure("ephemeral physical relay requires only the owned OMP Backend capture"));
    }
    Ok(if ephemeral { 0 } else { 7466 })
}
fn physical_relay_request(args: &clap::ArgMatches) -> io::Result<u16> {
    physical_relay_bind_port(
        args.get_flag("physical-relay-ephemeral"),
        args.get_flag("physical-hello-capture"),
        selected_omp_capture_mode(),
        owned_hello_capture_requested()
            || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref() == Some(std::ffi::OsStr::new("1"))
            || std::env::var_os("GOATR_OWNED_OMP_PRODUCER_CAPTURE").as_deref() == Some(std::ffi::OsStr::new("1")),
    )
}
async fn physical_hello_capture(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    let bind_port = physical_relay_request(args)?;
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
    if omp_backend_capture && config.herdr_sockets.len() != 1 {
        return Err(failure("owned OMP coarse capture requires exactly one configured Herdr socket"));
    }
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
    // All selector/config/socket admission precedes the first new owned artifact.
    private_dir(evidence, true)?;
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, bind_port))?;
    let address = listener.local_addr()?;
    if address.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        || address.port() == 0
        || (bind_port != 0 && address.port() != bind_port)
    {
        return Err(failure("physical Hello relay listener address changed"));
    }
    listener.set_nonblocking(true)?;
    let metadata = fs::metadata(format!("/proc/self/fd/{}", listener.as_raw_fd()))?;
    let record = goatr::backends::omp::OwnedRelayListener {
        v: 1,
        fd: listener.as_raw_fd(),
        device: metadata.dev(),
        inode: metadata.ino(),
        port: address.port(),
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
            herdr_report_capture: None,
            // This controller observes the existing paired desktop configuration.
            preserve_xdg_config_home: true,
            retain_process: false,
            hello_capture: false,
            grid_hold: None,
            project_read: None,
            startup_deadline: None,
            unit_authority: None,
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
    deadline: Option<Instant>,
) -> io::Result<Phone> {
    if deadline.is_some_and(|deadline|Instant::now()>=deadline){return Err(io::Error::new(io::ErrorKind::TimedOut,"original pairing deadline expired before worker spawn"));}
    let b = binary.to_owned();
    let r = root.to_owned();
    let s = socket.to_owned();
    let mut uri = tokio::task::spawn_blocking(move || terminal_pair(&b, Some(&r), Some(&s),deadline))
        .await
        .map_err(|_| {
            failure("phone setup failed: phase=phone_pair; guard=task_join; no retry")
        })??;
    let invite =
        PairInvite::parse(&uri).map_err(|_| failure("actual private pair URI rejected"))?;
    unsafe { uri.as_bytes_mut().fill(0) };
    uri.clear();
    let host = invite.host.clone();
    if let Some(deadline)=deadline {
        if Instant::now()>=deadline{return Err(io::Error::new(io::ErrorKind::TimedOut,"original pairing deadline expired before connection"));}
        tokio::time::timeout_at(deadline.into(),connect(endpoint,&host,Some(invite))).await
            .map_err(|_|io::Error::new(io::ErrorKind::TimedOut,"original pairing connection deadline"))?
    } else { connect(endpoint, &host, Some(invite)).await }
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
fn codex_single_creation_record(receipt: &Receipt, drain: Option<CodexDrainSummary>) -> Value {
    let (stage, stage_state, resources) = receipt.creation.as_deref().map_or(
        ("none".to_owned(), "none".to_owned(), json!({
            "workspace":false,"pane":false,"terminal":false,"native":false,
            "binding":false,"session":false,"generation":false,"ownerStarted":false,"relayStarted":false
        })),
        |creation| (
            format!("{:?}", creation.stage),
            format!("{:?}", creation.stage_state),
            json!({
                "workspace":creation.resources.workspace_id.is_some(),
                "pane":creation.resources.pane_id.is_some(),
                "terminal":creation.resources.terminal_id.is_some(),
                "native":creation.resources.native_id.is_some(),
                "binding":creation.resources.binding_id.is_some(),
                "session":creation.resources.session_id.is_some(),
                "generation":creation.resources.generation.is_some(),
                "ownerStarted":creation.resources.owner_started == Some(true),
                "relayStarted":creation.resources.relay_started == Some(true),
            }),
        ),
    );
    json!({
        "outcome":if receipt.state == ReceiptState::Succeeded { "unexpected_succeeded" } else { "captured_failed_or_uncertain" },
        "state":format!("{:?}", receipt.state),
        "stage":stage,
        "stageState":stage_state,
        "code":receipt.error.as_ref().map_or("none".to_owned(), |error| format!("{:?}", error.code)),
        "resources":resources,
        "ownerProgressMask":drain.as_ref().map(|summary| summary.owner_progress_mask),
        "ownerProgressGuard":drain.as_ref().and_then(|summary| summary.owner_progress_guard),
        "pendingAtObservation":drain.as_ref().map(|summary| summary.pending_at_observation),
        "firstNonprogressPhase":drain.as_ref().and_then(|summary| summary.first_nonprogress_phase.as_deref()),
        "firstNonprogressGuard":drain.as_ref().and_then(|summary| summary.first_nonprogress_guard.as_deref()),
    })
}
async fn create_codex_single_creation_diagnostic(
    phone: &mut Phone,
    config: &Config,
    server: &Uuid,
    serve: &Companion,
    deadline: Instant,
) -> io::Result<Receipt> {
    serve.codex_drain_pristine()?;
    if Instant::now() >= deadline {
        return Err(failure("Codex diagnostic deadline expired before one creation"));
    }
    let id = Uuid::random();
    let params = SessionCreateParams {
        directory_ref: None,
        placement: None,
        boot_id: phone.hello.boot_id.clone(),
        server_id: server.clone(),
        project_id: config.projects[0].id.clone(),
        provider: Provider::Codex,
        name: Label::new(format!("owned-Codex-{}", &id.0[..8])).map_err(io::Error::other)?,
    };
    let initial = tokio::time::timeout_at(
        deadline.into(),
        phone.request_at(id.clone(), RequestBody::SessionCreate(params), "create_initial"),
    )
    .await
    .map_err(|_| failure("Codex single creation response exceeded deadline"))?
    .map_err(|_| failure("Codex single creation request unavailable"))?
    .map_err(|_| failure("Codex single creation request rejected"))?;
    let initial = receipt_result(initial)?;
    let receipt = if matches!(initial.state, ReceiptState::Reserved | ReceiptState::Dispatching) {
        tokio::time::timeout_at(deadline.into(), phone.settled_by(&id, deadline))
            .await
            .map_err(|_| failure("Codex single creation settlement exceeded deadline"))?
            .map_err(|_| failure("Codex single creation settlement unavailable"))?
    } else {
        initial
    };
    let creation = receipt.creation.as_deref().ok_or_else(|| {
        failure("Codex single creation terminal receipt lacked creation facts")
    })?;
    if receipt.action_id != id
        || receipt.method != ActionMethod::SessionCreate
        || receipt.boot_id != phone.hello.boot_id
        || creation.provider != Provider::Codex
        || creation.server_id != *server
        || creation.project_id != config.projects[0].id
    {
        return Err(failure("Codex single creation receipt identity mismatch"));
    }
    Ok(receipt)
}

async fn create_omp_creation_diagnostic(
    phone: &mut Phone,
    config: &Config,
    server: &Uuid,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<Receipt> {
    let id = Uuid::random();
    let params = SessionCreateParams { directory_ref: None, placement: None, boot_id: phone.hello.boot_id.clone(),
    server_id: server.clone(),
    project_id: config.projects[0].id.clone(),
    provider: Provider::Omp,
    name: Label::new(format!("owned-Omp-{}", &id.0[..8])).map_err(io::Error::other)?, };
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
    admitted: Option<&AdmittedSource>,
) -> io::Result<(Option<SessionSummary>, Receipt, Option<FeedbackPresentation>)> {
    if let Some(admitted)=admitted {
        if &admitted.identity.0!=server {
            return Err(prerequisite("creation server differs from the admitted original"));
        }
        admitted.recheck()?;
    }
    let (before_source, before) = surface(socket).await?;
    if let Some(admitted)=admitted {
        if before_source!=admitted.client.evidence {
            return Err(prerequisite("original creation socket/process source changed"));
        }
        admitted.recheck()?;
    }
    before.no_focus()?;
    let anchor = before
        .focused_pane_id
        .clone()
        .ok_or_else(|| failure("focused native pane absent"))?;
    let before_geometry = geometry(socket, &anchor, GeometryPhase::CreateBefore)
        .await
        .map_err(GeometryFailure::into_io)?;
    let id = Uuid::random();
    let params = SessionCreateParams { directory_ref: None, placement: None, boot_id: phone.hello.boot_id.clone(),
    server_id: server.clone(),
    project_id: config.projects[0].id.clone(),
    provider,
    name: Label::new(format!("owned-{provider:?}-{}", &id.0[..8])).map_err(io::Error::other)?, };
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
        let original_current=if let Some(admitted)=admitted {
            admitted.observation(&before_source,&before,&inventory)?
        } else {true};
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
        if original_current && observed.is_some_and(|(connectivity, can_create)| {
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
            "available":available,"snapshotTimedOut":snapshot_timed_out,"originalSourcePinned":admitted.is_some()}),
    )?;
    if !available {
        return Err(prerequisite(
            "original owned server/provider creation prerequisites did not become available; no create submitted",
        ));
    }
    if let Some(admitted)=admitted {admitted.recheck()?;}
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
                RequestBody::PromptSubmit(PromptSubmitParams { skill: None, boot_id: phone.hello.boot_id.clone(),
                delivery: None, native_turn_id: None, images: None,
                target: p.target.clone(),
                text: PromptText(text), }),
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
    let stop=stop_current_work(phone,p).await?;
    let Value::Object(stop)=stop else {return Err(failure("Stop evidence object absent"));};
    let mut result=json!({"prompt":id,"receipt":receipt,"mobileMarker":marker,"desktopMarker":desktop,
        "historyIds":history.history.iter().map(|r|&r.id).collect::<Vec<_>>(),
        "toolObserved":history.history.iter().any(|r|r.tool.is_some()),
        "scope":"real native operations, payloads not retained; no OMP acknowledgment/winner fabricated"});
    result.as_object_mut().expect("prompt evidence").extend(stop);
    Ok(result)
}
async fn stop_current_work(phone: &mut Phone,p: &NativeProof) -> io::Result<Value> {
    let stop_prompt = Uuid::random();
    phone
        .request(
            stop_prompt.clone(),
            RequestBody::PromptSubmit(PromptSubmitParams { skill: None, boot_id: phone.hello.boot_id.clone(),
            delivery: None, native_turn_id: None, images: None,
            target: p.target.clone(),
            text: PromptText(
                "Run the native shell tool sleep 30, then report done; no file changes.".into(),
            ), }),
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
        json!({"stopPrompt":stop_prompt,"stopAction":cancel,"stopReceipt":stop,"stopIdleObserved":true,"nativeTurn":state.native_turn_id}),
    )
}

const SHARED_HISTORY_LIMIT:u64=2;
const SHARED_HISTORY_ADVANCES:usize=31;
struct SharedAgentContext<'a> {
    inputs: NativeInputs<'a>,
    companion: &'a Companion,
    endpoint: &'a iroh::Endpoint,
    source: &'a AdmittedSource,
}
fn shared_summary_matches(summary: &SessionSummary,original: &NativeProof) -> bool {
    target(summary)==original.target && summary.provider==original.provider
        && summary.native_id==original.native && summary.workspace_id==original.workspace
        && summary.pane_id==original.pane && summary.terminal_id==original.terminal
}
pub(super) fn shared_user_record<'a>(records: &'a [HistoryRecord],marker: &str) -> io::Result<Option<&'a HistoryRecord>> {
    let mut matching=records.iter().filter(|record|record.role==Role::User && record.complete
        && matches!(&record.text,Some(Content::Text(text)) if text.text.0.contains(marker)));
    let first=matching.next();
    if matching.next().is_some() {
        return Err(failure("shared-agent original prompt appeared more than once; no replay"));
    }
    Ok(first)
}
pub(super) fn shared_assistant_record<'a>(records: &'a [HistoryRecord],marker: &str) -> io::Result<Option<&'a HistoryRecord>> {
    let mut matching=records.iter().filter(|record|record.role==Role::Assistant && record.complete
        && matches!(&record.text,Some(Content::Text(text)) if text.text.0.contains(marker)));
    let first=matching.next();
    if matching.next().is_some() {
        return Err(failure("shared-agent assistant reply appeared more than once; no replay"));
    }
    Ok(first)
}
async fn shared_prompt(
    phone: &mut Phone,p: &NativeProof,inputs: &NativeInputs<'_>,marker: &str,
) -> io::Result<(Receipt,Id,Value)> {
    inputs.idle(phone,p).await?;
    inputs.current(phone,p).await?;
    let initial=phone.start_generation(&p.target).await?;
    if !shared_summary_matches(&initial.state.summary,p)
        || initial.state.summary.connectivity!=SessionConnectivity::Ready
        || initial.state.summary.status!=SessionStatus::Idle || !initial.pending_requests.is_empty() {
        return Err(prerequisite("shared-agent original is not Ready/Idle before its fresh prompt"));
    }
    let deadline=tokio::time::Instant::now()+Duration::from_secs(180);
    let action=Uuid::random();
    let receipt=receipt_result(phone.request(action.clone(),RequestBody::PromptSubmit(PromptSubmitParams {
        delivery: None, native_turn_id: None, images: None,
        skill:None,boot_id:phone.hello.boot_id.clone(),target:p.target.clone(),
        text:PromptText(format!(
            "In this exact existing session, run the native shell tool once to execute printf '%s\\n' '{marker}'. Do not write files or create/switch sessions. Then repeat {marker} in your reply."
        )),
    })).await?.map_err(|_|failure("shared-agent original prompt rejected; no retry"))?)?;
    let receipt=if matches!(receipt.state,ReceiptState::Reserved|ReceiptState::Dispatching) {
        tokio::time::timeout_at(deadline,phone.settled(&action)).await
            .map_err(|_|prerequisite("shared-agent original prompt receipt did not settle"))??
    } else {receipt};
    if receipt.action_id!=action || receipt.target.as_ref()!=Some(&p.target)
        || receipt.method!=ActionMethod::PromptSubmit || receipt.error.is_some()
        || !matches!(receipt.state,ReceiptState::Dispatched|ReceiptState::Succeeded) {
        return Err(prerequisite("shared-agent original prompt did not dispatch"));
    }
    let user=loop {
        p.recheck_processes()?;
        let view=phone.generation.as_ref().ok_or_else(||failure("shared-agent original stream absent"))?;
        if view.failed || !shared_summary_matches(&view.snapshot.state.summary,p) {
            return Err(failure("shared-agent original stream changed; no resnapshot"));
        }
        let tool=dev_continuity::completed_tool_output_state(&view.snapshot.history,marker);
        if tool==dev_continuity::ToolOutputMarkerState::Rejected {
            return Err(prerequisite("shared-agent completed native tool output failed or was ambiguous"));
        }
        let assistant=view.snapshot.history.iter().any(|record|record.role==Role::Assistant&&record.complete
            && matches!(&record.text,Some(Content::Text(text)) if text.text.0.contains(marker)));
        if view.streamed_text && view.streamed_tool && assistant
            && tool==dev_continuity::ToolOutputMarkerState::Ready
            && view.snapshot.state.summary.connectivity==SessionConnectivity::Ready
            && view.snapshot.state.summary.status==SessionStatus::Idle
            && view.snapshot.pending_requests.is_empty()
            && let Some(user)=shared_user_record(&view.snapshot.history,marker)? {
            break user.id.clone();
        }
        tokio::time::timeout_at(deadline,phone.generation_event()).await
            .map_err(|_|prerequisite("shared-agent real text/tool stream did not complete within its observation budget"))??;
    };
    let latest=phone.finish_generation(&p.target).await?;
    if !shared_summary_matches(&latest.state.summary,p)
        || shared_user_record(&latest.history,marker)?.map(|record|&record.id)!=Some(&user) {
        return Err(failure("shared-agent latest snapshot lost or duplicated its original prompt"));
    }
    inputs.current(phone,p).await?;
    let evidence=json!({"action":action,"receiptState":receipt.state,"originalUserRecord":user,
        "textEventsObserved":true,"toolEventsObserved":true,"completedNativeToolOutputObserved":true});
    Ok((receipt,user,evidence))
}
async fn shared_history_witness(phone: &mut Phone,p: &NativeProof,inputs: &NativeInputs<'_>) -> io::Result<Id> {
    inputs.idle(phone,p).await?;
    let mut witness=if p.provider==Provider::Codex {
        let mut witness=paging_proof::Witness::connect(p,inputs).await?;
        witness.admit(p,false).await?;
        Some(witness)
    } else {None};
    let initial=phone.start_generation(&p.target).await?;
    if !shared_summary_matches(&initial.state.summary,p)
        || initial.state.summary.connectivity!=SessionConnectivity::Ready
        || initial.state.summary.status!=SessionStatus::Idle || !initial.pending_requests.is_empty() {
        return Err(prerequisite("shared-agent history witness lacks original idle state"));
    }
    let marker=format!("goatr-shared-earlier-{}-{:?}",&inputs.run.0[..8],p.provider);
    let command=format!("printf '%s\\n' '{marker}'");
    let started=Instant::now();
    let deadline=history_command_deadline(false,p.provider,started+Duration::from_secs(30),started);
    phone.history_command=Some(HistoryCommandTrace::default());
    if let Some(observer)=&p.omp_observer {
        observer.healthy(&p.target)?;
        observer.state.lock().register(&command,&marker,deadline)?;
    }
    let completion=tokio::time::timeout_at(deadline.into(),async {
        inputs.send(phone,p,NativeInput::Prompt(&format!("!{command}")),None,"shared_earlier_witness").await?;
        if let Some(witness)=witness.as_mut() {
            let ((snapshot,id),(native,turn))=tokio::try_join!(
                inputs.shell_completed(phone,p,&command,&marker),
                witness.completed(p,&command,&marker,deadline.into())
            )?;
            if id!=native || snapshot.history.iter().find(|record|record.id==id)
                .and_then(|record|record.turn_id.as_ref())!=Some(&turn) {
                return Err(failure("shared-agent independent native completion IDs disagree"));
            }
            Ok(id)
        } else {
            if p.omp_observer.is_none() {return Err(prerequisite("shared-agent independent OMP observer absent"));}
            Ok(inputs.shell_completed(phone,p,&command,&marker).await?.1)
        }
    }).await;
    phone.history_command=None;
    let id=completion.map_err(|_|prerequisite("shared-agent earlier native witness deadline; no command replay"))??;
    phone.finish_generation(&p.target).await?;
    p.recheck_processes()?;
    Ok(id)
}
fn shared_history_page(
    page: &SessionHistoryResult,view: &Uuid,source: RecordSource,
    previous: Option<u64>,seen: &mut HashSet<Id>,
) -> io::Result<Option<u64>> {
    if page.view_id!=*view || page.records.len()>SHARED_HISTORY_LIMIT as usize
        || page.records.windows(2).any(|pair|pair[0].order.0>=pair[1].order.0)
        || previous.is_some_and(|oldest|page.records.last().is_some_and(|row|row.order.0>=oldest)) {
        return Err(failure("shared-agent native browse view/order/page bound changed"));
    }
    for row in &page.records {
        if !row.complete || row.source!=source || !seen.insert(row.id.clone()) {
            return Err(failure("shared-agent older page was incomplete, duplicated or not native"));
        }
    }
    Ok(page.records.first().map(|row|row.order.0).or(previous))
}
async fn shared_history(
    phone: &mut Phone,p: &NativeProof,inputs: &NativeInputs<'_>,witness: &Id,
) -> io::Result<Value> {
    inputs.current(phone,p).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(120);
    let ResultBody::SessionHistory(head)=tokio::time::timeout_at(deadline,phone.rpc(
        RequestBody::SessionHistory(SessionHistoryParams{target:p.target.clone(),view_id:None,cursor:None,
            limit:Some(Uint(SHARED_HISTORY_LIMIT))}),"shared_history_head"
    )).await.map_err(|_|prerequisite("shared-agent native browse deadline"))?? else {
        return Err(failure("shared-agent history head result mismatch"));
    };
    let source=if p.provider==Provider::Omp {RecordSource::Persisted} else {RecordSource::NativePage};
    let mut seen=HashSet::new();
    let mut boundary=shared_history_page(&head,&head.view_id,source,None,&mut seen)?;
    let mut cursor=head.next_cursor;
    let mut cursors=HashSet::new();
    let mut older_pages=0;
    let mut witnessed=false;
    for _ in 0..SHARED_HISTORY_ADVANCES {
        let Some(next)=cursor else {break};
        if !cursors.insert(next.clone()) {return Err(failure("shared-agent native cursor cycle"));}
        p.recheck_processes()?;
        let ResultBody::SessionHistory(page)=tokio::time::timeout_at(deadline,phone.rpc(
            RequestBody::SessionHistory(SessionHistoryParams{target:p.target.clone(),view_id:Some(head.view_id.clone()),
                cursor:Some(next),limit:Some(Uint(SHARED_HISTORY_LIMIT))}),"shared_history_older"
        )).await.map_err(|_|prerequisite("shared-agent native browse deadline"))?? else {
            return Err(failure("shared-agent older history result mismatch"));
        };
        boundary=shared_history_page(&page,&head.view_id,source,boundary,&mut seen)?;
        if !page.records.is_empty() {
            older_pages+=1;
            witnessed|=page.records.iter().any(|row|&row.id==witness);
        }
        cursor=page.next_cursor;
        if older_pages>=2 && witnessed {
            inputs.current(phone,p).await?;
            return Ok(json!({"viewId":head.view_id,"genuineOlderPages":older_pages,
                "earlierNativeRecord":witness,"source":source,"boundedRecords":seen.len(),
                "nativeFixtureCommands":1,"retentionBoundaryClaimed":false}));
        }
    }
    Err(prerequisite("shared-agent two genuine older pages and independently witnessed earlier record unavailable within bound"))
}
async fn shared_agent_flow(mut phone: Phone,selected: &[Provider],context: SharedAgentContext<'_>) -> io::Result<bool> {
    let inputs=&context.inputs;
    let host=phone.hello.host_id.clone();
    let boot=phone.hello.boot_id.clone();
    let device=phone.hello.device_id.clone();
    let mut originals=Vec::new();
    let mut observations=Vec::new();
    let mut completed=HashSet::new();
    let mut active=None;
    let mut phase="creation";
    let result:io::Result<()>=async {
        dev_continuity::zero_native_caps(inputs.herdr)?;
        for provider in selected.iter().copied() {
            active=Some(provider);
            let (session,receipt,_)=create_session_presented(&mut phone,inputs.config,inputs.socket,provider,
                &context.source.identity.0,inputs.evidence,inputs.run,false,Some(context.source)).await?;
            write_json(&inputs.evidence.join(format!("shared-create-{}-{provider:?}.json",inputs.run.0)),
                &json!({"runId":inputs.run,"provider":provider,"receipt":receipt}))?;
            let session=session.ok_or_else(||non_succeeded_creation_error(provider,&receipt))?;
            let original=proof(inputs.config,inputs.socket,&session,None).await?;
            for process in std::iter::once(&original.owner).chain(original.tui.iter()).chain(original.relay.iter()) {
                dev_continuity::zero_native_caps(process)?;
            }
            original.recheck_processes()?;
            write_json(&inputs.evidence.join(format!("shared-native-{}-{provider:?}.json",inputs.run.0)),
                &json!({"runId":inputs.run,"proof":original.evidence(),"creationReceipt":receipt}))?;
            originals.push(original);
        }
        let mut witnesses=Vec::new();
        let mut prompts=Vec::new();
        for original in &originals {
            active=Some(original.provider);
            phase="earlier_native_witness";
            let witness=shared_history_witness(&mut phone,original,inputs).await?;
            witnesses.push(witness);
            phase="phone_stream";
            let marker=format!("goatr-shared-mobile-{}-{:?}",&inputs.run.0[..8],original.provider);
            let (receipt,user,evidence)=shared_prompt(&mut phone,original,inputs,&marker).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"evidence":evidence}));
            prompts.push((marker,receipt,user));
            phase="desktop_continuation";
            let desktop=format!("goatr-shared-desktop-{}-{:?}",&inputs.run.0[..8],original.provider);
            inputs.marker(&mut phone,original,&desktop).await?;
            wait_history(&mut phone,&original.target,&desktop,false).await?;
            inputs.idle(&mut phone,original).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"observed":true}));
            let kind=if original.provider==Provider::Omp {PresentationKind::OmpSelect} else {PresentationKind::CodexApproval};
            phase="phone_native_request";
            let answered=dialog(&mut phone,original,kind,inputs).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"evidence":answered}));
            phase="competing_desktop_resolution";
            let competing=desktop_dialog(&mut phone,original,kind,inputs).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"evidence":competing}));
            inputs.idle(&mut phone,original).await?;
            phase="actual_stop";
            let stopped=stop_current_work(&mut phone,original).await?;
            inputs.current(&mut phone,original).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"evidence":stopped}));
        }
        active=None;
        phase="same_original_reconnect";
        phone=dev_continuity::reconnect_original(phone,context.endpoint,&host,inputs.config,inputs.socket,&originals).await?;
        if phone.hello.boot_id!=boot || phone.hello.device_id!=device {
            return Err(failure("shared-agent reconnect changed original companion boot or paired identity"));
        }
        for ((original,witness),(marker,receipt,user)) in originals.iter().zip(&witnesses).zip(&prompts) {
            active=Some(original.provider);
            phase="native_older_history";
            let history=shared_history(&mut phone,original,inputs,witness).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"evidence":history}));
            phase="return_to_latest";
            let latest=phone.open(&original.target,"shared_return_latest").await?;
            if !shared_summary_matches(&latest.state.summary,original)
                || shared_user_record(&latest.history,marker)?.map(|row|&row.id)!=Some(user) {
                return Err(failure("shared-agent return to latest lost or duplicated the original native prompt"));
            }
            let retained=phone.receipt(&receipt.action_id).await?;
            if retained.action_id!=receipt.action_id || retained.target!=receipt.target
                || retained.method!=receipt.method || retained.error.is_some()
                || !matches!(retained.state,ReceiptState::Dispatched|ReceiptState::Succeeded) {
                return Err(failure("shared-agent original receipt did not reconcile; no resend"));
            }
            phase="fresh_phone_after_reconnect";
            let final_marker=format!("goatr-shared-return-{}-{:?}",&inputs.run.0[..8],original.provider);
            let (_,_,sent)=shared_prompt(&mut phone,original,inputs,&final_marker).await?;
            let latest=phone.open(&original.target,"shared_final_latest").await?;
            if !shared_summary_matches(&latest.state.summary,original)
                || shared_user_record(&latest.history,marker)?.map(|row|&row.id)!=Some(user) {
                return Err(failure("shared-agent original prompt duplicated or disappeared after fresh input"));
            }
            inputs.current(&mut phone,original).await?;
            observations.push(json!({"provider":original.provider,"phase":phase,"evidence":sent,
                "originalPromptStillExactlyOnce":true,"originalReceiptReconciledWithoutResend":true}));
            completed.insert(original.provider);
        }
        active=None;
        phase="final_original_proof";
        if completed.len()!=selected.len() {
            return Err(failure("shared-agent selected provider was not fully exercised"));
        }
        context.companion.recheck()?;
        if herdr::process_evidence(inputs.herdr.pid)?!=*inputs.herdr {
            return Err(failure("shared-agent original Herdr process changed"));
        }
        phone.connection.close(0u32.into(),b"finite shared-agent scenario complete; no action replay");
        Ok(())
    }.await;
    let available=result.is_ok();
    let results=selected.iter().map(|provider|json!({"gate":"shared-agent-only","runId":inputs.run,
        "provider":provider,"status":if completed.contains(provider)&&available {"passed"} else {"unavailable"},
        "reason":if available {"completed"} else if active.is_none()||active==Some(*provider) {
            "original_native_boundary_unavailable"
        } else {"scenario_stopped_after_other_boundary"},
        "phase":if available {"complete"} else {phase},
    })).collect::<Vec<_>>();
    let boundary=result.as_ref().err().map(super::helper_error_metadata);
    write_json(&inputs.evidence.join(format!("shared-agent-{}.json",inputs.run.0)),&json!({
        "runId":inputs.run,"results":results,"boundary":boundary,"observations":observations,
        "originals":originals.iter().map(NativeProof::evidence).collect::<Vec<_>>(),
        "resourcesRetained":true,"androidAcceptance":false,"retentionBoundaryClaimed":false,
        "scope":"fresh normally created originals only; no replay, native cleanup, cold substitute or manufactured history",
    }))?;
    for result in results {println!("{result}");}
    Ok(available)
}
async fn grid_run(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    let (qualification, deployment) = GridQualification::read(args, evidence)?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|error| grid_context("retain_term", error))?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|error| grid_context("retain_interrupt", error))?;
    let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .map_err(|error| grid_context("retain_hangup", error))?;
    let control = McpStdin::open().map_err(|error| grid_context("source_control_pipe", error))?;
    let metadata = control
        .0
        .get_ref()
        .metadata()
        .map_err(|error| grid_context("source_control_pipe", error))?;
    if metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
        return Err(grid_failure("source_control_private"));
    }
    let mut hold = GridHold {
        originals: Vec::new(),
        spawned: 0,
        relays: Vec::new(),
        source_control: Some(tokio::io::BufReader::new(control)),
        source_loss_observed: false,
    };
    write_json(
        &evidence.join(format!("grid-qualification-{}.json", Uuid::random().0)),
        &json!({
            "gate":"grid","phase":"qualified_inputs","nativeSha256":qualification.native_sha256,
            "nativeSourceSha256":qualification.native_source_sha256,"nativeSchemaSha256":qualification.native_schema_sha256,
            "nativeLockSha256":qualification.native_lock_sha256,"goatrSha256":qualification.goatr_sha256,
            "goatrSourceSha256":qualification.goatr_source_sha256,"helperSha256":qualification.helper_sha256,
            "companionLockSha256":qualification.companion_lock_sha256,"providers":qualification.providers,
            "explicitShellInputGrant":true,"cleanupAuthorized":false
        }),
    )?;
    let result = checkpoint_held(args, evidence, Some(&mut hold), Some(deployment)).await;
    let location = result.as_ref().err().and_then(super::helper_error_location);
    // A failure's first authored file/line survives, but its potentially private text never does.
    let record = json!({"gate":"grid","phase":"retained_outcome","scopedFlowCompleted":result.as_ref().is_ok_and(|v|*v),
        "outcomeKnown":result.is_ok(),"spawnedOriginals":hold.spawned,"heldOriginalPidfds":hold.originals.len(),
        "heldRelayListeners":hold.relays.len(),"firstFailureFile":location.map(|l|l.file),
        "firstFailureLine":location.map(|l|l.line),"cleanupAuthorized":false,
        "failureMetadata":result.as_ref().err().map(super::helper_error_metadata),
        "nativeSourceEofGate":if hold.source_loss_observed{"observed"}else{"not_reached"},"quantitativeMemoryProof":false});
    let recorded = write_json(
        &evidence.join(format!("grid-retained-{}.json", Uuid::random().0)),
        &record,
    );
    let _ = writeln!(std::io::stdout(), "{record}");
    if hold.spawned == 0 {
        recorded.map_err(|_| grid_failure("outcome_evidence"))?;
        return result.map_err(|_| grid_failure("prelaunch_prerequisite"));
    }
    // Native owns its PTY masters; both children use null stdio and distinct process groups.
    // Preserve the original relay fd, pidfds and supervisor even on failure/evidence failure.
    // Only a separately qualified fresh candidate can authorize a later external stop.
    loop {
        tokio::select! {_ = term.recv()=>{},_ = interrupt.recv()=>{},_ = hangup.recv()=>{}}
    }
}
impl Phone {
    async fn grid_open(&mut self, subject: GridSubject) -> io::Result<()> {
        self.grid_subscribe(GridWitness::pending(Uuid::random(),subject)).await
    }
    async fn grid_subscribe(&mut self,witness:GridWitness)->io::Result<()> {
        if self.grid.is_some() || self.generation.is_some() {
            return Err(grid_failure("viewer_busy"));
        }
        let id=witness.opening.clone();
        let body = match &witness.subject {
            GridSubject::Pane(target) => {
                if !self.hello.features.agent_ansi{return Err(grid_failure("ansi_observer_unavailable"));}
                RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams {
                    target: target.clone(),
                    ansi_observer: true,
                })
            }
            GridSubject::Shell(target) => {
                RequestBody::ShellGridSubscribe(ShellGridSubscribeParams { target: target.clone(), history: false })
            }
        };
        self.grid = Some(witness);
        self.request_at(id, body, "grid_open")
            .await?
            .map_err(|_| grid_failure("opening_rejected"))?;
        self.grid_wait(GridGoal::Commit).await
    }
    async fn grid_wait(&mut self, goal:GridGoal) -> io::Result<()> {
        let phase_end = Instant::now() + Duration::from_secs(45);
        loop {
            let witness = self
                .grid
                .as_ref()
                .ok_or_else(|| grid_failure("viewer_missing"))?;
            if witness.retired {
                return Err(grid_failure("viewer_retired"));
            }
            if match goal {
                GridGoal::Commit=>witness.committed.is_some(),
                GridGoal::Token=>witness.token_commit.is_some_and(|revision|revision>0),
                GridGoal::TerminalFixture{after}=>witness.terminal_fixture_ready(after)?,
                GridGoal::Resized{after,dimensions}=>witness.committed.is_some_and(|revision|revision>after)
                    &&witness.committed_dimensions.is_some_and(|actual|actual!=dimensions),
            } {
                return Ok(());
            }
            let end = witness.deadline.map_or(phase_end, |end| end.min(phase_end));
            let bytes = tokio::time::timeout_at(
                tokio::time::Instant::from_std(end),
                transport::read_frame(&mut self.recv, FRAME_MAX),
            )
            .await
            .map_err(|_| grid_failure("observation_deadline"))?
            .map_err(|error| {
                let close=closed_connection_reason(self.connection.close_reason());
                let method=match &witness.subject {
                    GridSubject::Shell(_)=>Method::ShellGridSubscribe,
                    GridSubject::Pane(_)=>Method::TerminalGridSubscribe,
                };
                request_failure_with_close(method,"grid_wait","frame_read",error,Some(close))
            })?;
            match decode_payload(&bytes, None).map_err(|_| grid_failure("grid_codec"))? {
                Envelope::Event(event) => self.event_sized(event, bytes.len())?,
                _ => return Err(grid_failure("unexpected_response")),
            }
        }
    }
    async fn grid_unsubscribe(&mut self) -> io::Result<()> {
        let subscription = self
            .grid
            .as_ref()
            .and_then(|g| g.subscription.clone())
            .ok_or_else(|| grid_failure("unsubscribe_scope"))?;
        let result = self
            .rpc(
                RequestBody::SessionUnsubscribe(SessionUnsubscribeParams { subscription }),
                "grid_unsubscribe",
            )
            .await
            .map_err(|_| grid_failure("unsubscribe_response"))?;
        if !matches!(result, ResultBody::SessionUnsubscribe(_)) {
            return Err(grid_failure("unsubscribe_response"));
        }
        let witness = self
            .grid
            .as_mut()
            .ok_or_else(|| grid_failure("unsubscribe_scope"))?;
        witness.retired = true;
        witness.deadline = None;
        // This correlated response drains all earlier frames. Any late retired data is rejected
        // at the private route before it can enter generic event/history handling.
        self.host()
            .await
            .map_err(|_| grid_failure("unsubscribe_drain"))?;
        self.grid = None;
        Ok(())
    }
    async fn grid_action(
        &mut self,
        id: Uuid,
        body: RequestBody,
        slow: bool,
    ) -> io::Result<Option<Receipt>> {
        let method = body.method();
        let outcome = if slow {
            let mut bytes = encode_payload(&Envelope::Request(Request {
                id: id.clone(),
                body,
            }))
            .map_err(|_| grid_failure("action_codec"))?;
            transport::write_frame(&mut self.send, &bytes)
                .await
                .map_err(|_| grid_failure("action_write"))?;
            bytes.fill(0);
            // Predetermined bounded pause of this sole receiver, not a parallel reader.
            tokio::time::sleep(Duration::from_secs(6)).await;
            let end = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                let read =
                    tokio::time::timeout_at(end, transport::read_frame(&mut self.recv, FRAME_MAX))
                        .await
                        .map_err(|_| grid_failure("slow_response_deadline"))?;
                let bytes = match read {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        use iroh::endpoint::ConnectionError as C;
                        if matches!(
                            self.connection.close_reason(),
                            Some(
                                C::ApplicationClosed(_)
                                    | C::ConnectionClosed(_)
                                    | C::TransportError(_)
                                    | C::Reset
                                    | C::TimedOut
                            )
                        ) {
                            return Ok(None);
                        }
                        return Err(grid_context("slow_transport_unproved", error));
                    }
                };
                match decode_payload(&bytes, Some(method))
                    .map_err(|_| grid_failure("slow_codec"))?
                {
                    Envelope::Event(event) => self.event_sized(event, bytes.len())?,
                    Envelope::Response(response) if response.id == id => break response.outcome,
                    _ => return Err(grid_failure("slow_correlation")),
                }
            }
        } else {
            // A fatal routing/codec/correlation error is the original failure, not a lost
            // response to recover on this reader. The recorded action remains status-only.
            self.request_at(id.clone(), body, "grid_dirty_action")
                .await?
        };
        let result = outcome.map_err(|_| grid_failure("action_rejected"))?;
        let receipt = match result {
            ResultBody::PromptSubmit(r) => r.receipt,
            ResultBody::ShellInput(r) => r.receipt,
            _ => return Err(grid_failure("action_response")),
        };
        if matches!(
            receipt.state,
            ReceiptState::Reserved | ReceiptState::Dispatching
        ) {
            self.settled_by(&id, Instant::now() + Duration::from_secs(30))
                .await
                .map(Some)
                .map_err(|error| grid_context("original_action_status", error))
        } else {
            Ok(Some(receipt))
        }
    }
}
fn grid_receipt(
    receipt: &Receipt,
    id: &Uuid,
    boot: &Uuid,
    subject: &GridSubject,
) -> io::Result<()> {
    let scope = match subject {
        GridSubject::Pane(target) => {
            receipt.method == ActionMethod::PromptSubmit
                && receipt.target.as_ref() == Some(target)
                && receipt.shell_target.is_none()
        }
        GridSubject::Shell(target) => {
            receipt.method == ActionMethod::ShellInput
                && receipt.shell_target.as_ref() == Some(target)
                && receipt.target.is_none()
                && receipt.state==ReceiptState::Dispatched
                && matches!(&receipt.result,Some(ActionResult::ShellDispatch(result))if result.progress.complete())
        }
    };
    if !scope
        || receipt.action_id != *id
        || receipt.boot_id != *boot
        || receipt.error.is_some()
        || !matches!(
            receipt.state,
            ReceiptState::Dispatched | ReceiptState::Succeeded
        )
    {
        return Err(grid_failure("original_action_receipt"));
    }
    Ok(())
}
// Capacity only: Client pins the peer; its duplicate is the sole I/O adapter.
// Neither an assembler nor another Goatr viewer. Both descriptors close together.
async fn grid_native_line(
    reader: &mut tokio::io::BufReader<tokio::net::UnixStream>,
    deadline: tokio::time::Instant,
    total: &mut usize,
) -> io::Result<Option<Value>> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let mut bytes = Vec::new();
    tokio::time::timeout_at(
        deadline,
        (&mut *reader)
            .take(128 * 1024 + 1)
            .read_until(b'\n', &mut bytes),
    )
    .await
    .map_err(|_| grid_failure("native_transaction_deadline"))?
    .map_err(|error| grid_context("native_stream_read", error))?;
    if bytes.len() > 128 * 1024 {
        return Err(grid_failure("native_line_bound"));
    }
    *total = total
        .checked_add(bytes.len())
        .ok_or_else(|| grid_failure("native_transaction_bound"))?;
    if *total > 8 * 1024 * 1024 {
        return Err(grid_failure("native_transaction_bound"));
    }
    // read_until only ends without LF at actual EOF (the size-limit case was rejected).
    // This is native-FD observation, never a caller declaration or an Iroh-close inference.
    if bytes.last() != Some(&b'\n') {
        return Ok(None);
    }
    goatr::native_json::parse(&bytes, goatr::native_json::NATIVE)
        .map(Some)
        .map_err(|_| grid_failure("native_stream_json"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GridSourceIntent {
    version: u8,
    run: Uuid,
    ready: Uuid,
    trigger: Uuid,
}
struct GridSourceLoss {
    run: Uuid,
    ready: Uuid,
    trigger: Option<Uuid>,
    native_eof: bool,
    iroh_closed: bool,
    sync_required: bool,
    deadline: Option<tokio::time::Instant>,
}
impl GridSourceLoss {
    fn arm(&mut self, intent: GridSourceIntent) -> io::Result<()> {
        if intent.version != 1
            || intent.run != self.run
            || intent.ready != self.ready
            || self.trigger.is_some()
            || intent.trigger == self.run
            || intent.trigger == self.ready
            || intent.trigger.validate().is_err()
        {
            return Err(grid_failure("source_trigger_join"));
        }
        self.trigger = Some(intent.trigger);
        Ok(())
    }
    fn upstream_eof(&mut self) -> io::Result<()> {
        if self.trigger.is_none() || self.native_eof {
            return Err(grid_failure("source_eof_without_armed_intent"));
        }
        self.native_eof = true;
        self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(15));
        Ok(())
    }
    fn downstream(&mut self, sync: bool) -> io::Result<()> {
        if self.trigger.is_none() {
            return Err(grid_failure("source_downstream_before_intent"));
        }
        if sync {
            self.sync_required = true;
        } else {
            self.iroh_closed = true;
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.trigger.is_some()
            && self.native_eof
            && (self.iroh_closed || self.sync_required)
            && self
                .deadline
                .is_some_and(|deadline| tokio::time::Instant::now() < deadline)
    }
}
// One readiness seam for the SAME retained futures. Failure/data wins over intent;
// no branch recreates a partial read_frame or grants process-stop authority.
enum GridSourceWake<N, C> {
    Native(N),
    Closed,
    ObservationDeadline,
    TransactionDeadline,
    Frame(io::Result<Vec<u8>>),
    Control(C),
}
async fn grid_source_wake<N, C, D>(
    native: impl std::future::Future<Output = N>,
    closed: impl std::future::Future<Output = D>,
    observation: impl std::future::Future<Output = ()>,
    transaction: impl std::future::Future<Output = ()>,
    frame: impl std::future::Future<Output = io::Result<Vec<u8>>>,
    control: impl std::future::Future<Output = C>,
) -> GridSourceWake<N, C> {
    tokio::select! {biased;
        _=observation=>GridSourceWake::ObservationDeadline,
        _=transaction=>GridSourceWake::TransactionDeadline,
        result=frame=>GridSourceWake::Frame(result),
        result=native=>GridSourceWake::Native(result),
        _=closed=>GridSourceWake::Closed,
        result=control=>GridSourceWake::Control(result),
    }
}
enum GridSourceHandoff {
    Frame(io::Result<Vec<u8>>),
    Pending { settled: bool },
}
fn grid_source_deadlines(loss: &GridSourceLoss, grid: &Option<GridWitness>) -> io::Result<()> {
    if loss
        .deadline
        .is_some_and(|end| tokio::time::Instant::now() >= end)
    {
        return Err(grid_failure("source_downstream_deadline"));
    }
    if grid
        .as_ref()
        .and_then(|g| g.deadline)
        .is_some_and(|end| Instant::now() >= end)
    {
        return Err(grid_failure("source_grid_transaction_deadline"));
    }
    Ok(())
}
fn grid_source_handoff(
    frame: std::pin::Pin<&mut impl std::future::Future<Output = io::Result<Vec<u8>>>>,
    fin_seen: bool,
    loss: &GridSourceLoss,
    grid: &Option<GridWitness>,
    subscription: &Uuid,
    subject: &GridSubject,
) -> io::Result<GridSourceHandoff> {
    use futures_util::FutureExt;
    // Consume only a temporary Pin reborrow. Pending preserves the exact original reader.
    if !fin_seen {
        if let Some(result) = frame.now_or_never() {
            return Ok(GridSourceHandoff::Frame(result));
        }
    }
    grid_source_deadlines(loss, grid)?;
    if !grid
        .as_ref()
        .is_some_and(|g| g.subscription.as_ref() == Some(subscription) && &g.subject == subject)
    {
        return Err(grid_failure("source_original_scope"));
    }
    // Settlement is a point observation at this Pending/actual-FIN rejoin, not a claim
    // that no failure can ever become ready after this decision.
    Ok(GridSourceHandoff::Pending {
        settled: loss.complete(),
    })
}
fn grid_source_fin(armed: bool, error: io::Error) -> io::Result<()> {
    let eof = matches!(
        transport::FrameFailure::from_error(&error),
        Some(transport::FrameFailure {
            stage: transport::FrameStage::FrameHeader | transport::FrameStage::FramePayload,
            io: Some(transport::FrameIoKind::UnexpectedEof),
            code: None,
        })
    );
    if armed && eof { Ok(()) } else { Err(error) }
}
async fn grid_source_intent(
    mut input: tokio::io::BufReader<McpStdin>,
) -> (tokio::io::BufReader<McpStdin>, io::Result<GridSourceIntent>) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let result = async {
        let mut bytes = Vec::new();
        (&mut input)
            .take(513)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|error| grid_context("source_control_read", error))?;
        if bytes.is_empty() || bytes.len() > 512 || bytes.last() != Some(&b'\n') {
            return Err(grid_failure("source_control_record"));
        }
        let value = goatr::native_json::parse(
            &bytes,
            goatr::native_json::Limits {
                bytes: 512,
                depth: 4,
                members: 4,
            },
        )
        .map_err(|_| grid_failure("source_control_record"))?;
        serde_json::from_value(value).map_err(|_| grid_failure("source_control_shape"))
    }
    .await;
    (input, result)
}
async fn grid_source_native(
    mut native: NativeCapacityReader,
) -> (NativeCapacityReader, io::Result<bool>) {
    let result = native.source_next().await;
    (native, result)
}
async fn grid_source_blocked(evidence: &Path, run: &Uuid, error: io::Error) -> io::Result<()> {
    let frame_failure=transport::FrameFailure::from_error(&error).map(|f|json!({
        "stage":format!("{:?}",f.stage),"io":f.io.map(|v|format!("{v:?}")),"code":f.code.map(|v|format!("{v:?}"))
    }));
    let record = json!({"gate":"grid","phase":"source_loss_blocked","run":run,
        "failure":super::helper_error_metadata(&error),"frameFailure":frame_failure,"cleanupAuthorized":false,"sameRunHeld":true});
    let _ = write_json(
        &evidence.join(format!(
            "grid-source-loss-blocked-{}.json",
            Uuid::random().0
        )),
        &record,
    );
    let _ = writeln!(std::io::stdout(), "{record}");
    // A fatal reader/authority boundary cannot be repaired by lookup, retarget or replay.
    // Caller locals still own the exact Phone/endpoint/native reader while Main reviews cleanup.
    std::future::pending().await
}
struct NativeCapacityReader {
    client: Client,
    reader: tokio::io::BufReader<tokio::net::UnixStream>,
    scope: Value,
    sequence: u64,
    parts: u64,
    revision: u64,
    deadline: tokio::time::Instant,
    bytes: usize,
}
impl NativeCapacityReader {
    async fn line(&mut self) -> io::Result<Value> {
        self.client
            .recheck()
            .map_err(|_| grid_failure("capacity_original_peer"))?;
        let value = grid_native_line(&mut self.reader, self.deadline, &mut self.bytes)
            .await?
            .ok_or_else(|| grid_failure("capacity_native_eof"))?;
        self.client
            .recheck()
            .map_err(|error| grid_context("capacity_original_peer", error))?;
        Ok(value)
    }
    async fn open(
        socket: &Path,
        source: &ServerEvidence,
        method: &'static str,
        params: Value,
        scope: Value,
    ) -> io::Result<Option<Self>> {
        use tokio::io::AsyncWriteExt;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let client = tokio::time::timeout_at(deadline, Client::connect(socket))
            .await
            .map_err(|_| grid_failure("capacity_connect"))?
            .map_err(|_| grid_failure("capacity_connect"))?;
        if &client.evidence != source {
            return Err(grid_failure("capacity_peer"));
        }
        let duplicate = client
            .fence_socket()
            .map_err(|_| grid_failure("capacity_fence"))?;
        let stream = std::os::unix::net::UnixStream::from(duplicate);
        stream
            .set_nonblocking(true)
            .map_err(|_| grid_failure("capacity_adapter"))?;
        let stream = tokio::net::UnixStream::from_std(stream)
            .map_err(|_| grid_failure("capacity_adapter"))?;
        let mut reader = Self {
            client,
            reader: tokio::io::BufReader::new(stream),
            scope,
            sequence: 1,
            parts: 0,
            revision: 0,
            deadline,
            bytes: 0,
        };
        let id = Uuid::random();
        let mut bytes = serde_json::to_vec(&json!({"id":id.0,"method":method,"params":params}))
            .map_err(|_| grid_failure("capacity_request"))?;
        bytes.push(b'\n');
        reader
            .client
            .recheck()
            .map_err(|_| grid_failure("capacity_original_peer"))?;
        tokio::time::timeout_at(deadline, reader.reader.get_mut().write_all(&bytes))
            .await
            .map_err(|_| grid_failure("capacity_write"))?
            .map_err(|_| grid_failure("capacity_write"))?;
        let opening = reader.line().await?;
        if opening.get("id").and_then(Value::as_str) != Some(id.0.as_str()) {
            return Err(grid_failure("capacity_correlation"));
        }
        if let Some(error) = opening.get("error") {
            if error.get("code").and_then(Value::as_str) == Some("grid_resource_limit") {
                return Ok(None);
            }
            return Err(grid_failure("capacity_open_rejected"));
        }
        let result = opening
            .get("result")
            .ok_or_else(|| grid_failure("capacity_opening"))?;
        if result["type"] != "grid_subscribed"
            || result["scope"] != reader.scope
            || result["sequence"].as_u64() != Some(0)
            || result["frame"]["revision"].as_u64() != Some(0)
        {
            return Err(grid_failure("capacity_opening"));
        }
        reader.parts = result["frame"]["part_count"]
            .as_u64()
            .filter(|n| (1..=128).contains(n))
            .ok_or_else(|| grid_failure("capacity_descriptor"))?;
        Ok(Some(reader))
    }
    async fn initial_commit(&mut self) -> io::Result<()> {
        for index in 0..self.parts {
            let part = self.line().await?;
            if part["scope"] != self.scope
                || part["sequence"].as_u64() != Some(self.sequence)
                || part["type"] != "grid_part"
                || part["revision"].as_u64() != Some(0)
                || part["index"].as_u64() != Some(index)
            {
                return Err(grid_failure("capacity_part_order"));
            }
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or_else(|| grid_failure("capacity_sequence"))?;
        }
        let commit = self.line().await?;
        if commit["scope"] != self.scope
            || commit["sequence"].as_u64() != Some(self.sequence)
            || commit["type"] != "grid_commit"
            || commit["revision"].as_u64() != Some(0)
        {
            return Err(grid_failure("capacity_commit"));
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| grid_failure("capacity_sequence"))?;
        self.client
            .recheck()
            .map_err(|_| grid_failure("capacity_original_peer"))
    }
    async fn source_next(&mut self) -> io::Result<bool> {
        use tokio::io::AsyncBufReadExt;
        // Quiet native source waits for data/actual EOF, never a renewing/idle timer.
        if self
            .reader
            .fill_buf()
            .await
            .map_err(|error| grid_context("source_native_read", error))?
            .is_empty()
        {
            return Ok(true);
        }
        self.deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        self.bytes = 0;
        let Some(begin) =
            grid_native_line(&mut self.reader, self.deadline, &mut self.bytes).await?
        else {
            return Ok(true);
        };
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| grid_failure("source_native_revision"))?;
        if begin["type"] != "grid_begin"
            || begin["scope"] != self.scope
            || begin["sequence"].as_u64() != Some(self.sequence)
            || begin["frame"]["revision"].as_u64() != Some(revision)
        {
            return Err(grid_failure("source_native_begin"));
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| grid_failure("source_native_sequence"))?;
        let parts = begin["frame"]["part_count"]
            .as_u64()
            .filter(|n| (1..=128).contains(n))
            .ok_or_else(|| grid_failure("source_native_parts"))?;
        for index in 0..parts {
            let Some(part) =
                grid_native_line(&mut self.reader, self.deadline, &mut self.bytes).await?
            else {
                return Ok(true);
            };
            if part["type"] != "grid_part"
                || part["scope"] != self.scope
                || part["sequence"].as_u64() != Some(self.sequence)
                || part["revision"].as_u64() != Some(revision)
                || part["index"].as_u64() != Some(index)
            {
                return Err(grid_failure("source_native_part"));
            }
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or_else(|| grid_failure("source_native_sequence"))?;
        }
        let Some(commit) =
            grid_native_line(&mut self.reader, self.deadline, &mut self.bytes).await?
        else {
            return Ok(true);
        };
        if commit["type"] != "grid_commit"
            || commit["scope"] != self.scope
            || commit["sequence"].as_u64() != Some(self.sequence)
            || commit["revision"].as_u64() != Some(revision)
        {
            return Err(grid_failure("source_native_commit"));
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| grid_failure("source_native_sequence"))?;
        self.revision = revision;
        Ok(false)
    }
    fn release(self) {
        let Self { client, reader, .. } = self;
        drop(reader); // duplicated I/O fd
        drop(client); // original validated fd: only this final close can establish upstream EOF
    }
}
async fn native_grid_capacity(
    socket: &Path,
    source: &ServerEvidence,
    provider_scope: (Value, Value),
    shell_scope: Value,
    evidence: &Path,
    run: &Uuid,
) -> io::Result<()> {
    let (pane_params, pane_scope) = provider_scope;
    let shell_params = json!({"scope":shell_scope});
    let shell_scope = json!({"kind":"shell","scope":shell_scope});
    let first = NativeCapacityReader::open(
        socket,
        source,
        "pane.grid_subscribe_bound",
        pane_params.clone(),
        pane_scope.clone(),
    )
    .await?
    .ok_or_else(|| grid_failure("capacity_first_unavailable"))?;
    let mut survivor = NativeCapacityReader::open(
        socket,
        source,
        "shell.grid_subscribe_bound",
        shell_params,
        shell_scope,
    )
    .await?
    .ok_or_else(|| grid_failure("capacity_second_unavailable"))?;
    if let Some(unexpected) = NativeCapacityReader::open(
        socket,
        source,
        "pane.grid_subscribe_bound",
        pane_params.clone(),
        pane_scope.clone(),
    )
    .await?
    {
        unexpected.release();
        return Err(grid_failure("capacity_third_not_refused"));
    }
    first.release();
    // ONE fresh admission, never capacity retry/eviction. A final-Drop scheduling race is blocked.
    let mut fresh = NativeCapacityReader::open(
        socket,
        source,
        "pane.grid_subscribe_bound",
        pane_params,
        pane_scope,
    )
    .await?
    .ok_or_else(|| grid_failure("native_final_drop_not_observed"))?;
    survivor.initial_commit().await?;
    fresh.initial_commit().await?;
    survivor.release();
    fresh.release();
    write_json(
        &evidence.join(format!("grid-capacity-{}.json", run.0)),
        &json!({
            "phase":"native_capacity","sources":2,"thirdResourceLimit":true,"closedOriginalAndDuplicateFds":true,
            "freshAdmission":true,"survivingReaderCommitted":true,"freshReaderCommitted":true,
            "capacityRetry":false,"producerStopClaim":false
        }),
    )
}
fn grid_source<'a>(
    inventory: &'a HostSnapshotResult,
    native: &Snapshot,
    root: &herdr::Pane,
    expected: Option<(&Uuid, &Uuid)>,
) -> io::Result<(&'a Server, &'a Uuid)> {
    let [graph] = inventory.hierarchy.as_slice() else {
        return Err(grid_failure("sole_owned_hierarchy"));
    };
    let mut associated = None;
    for server in &inventory.servers {
        if server.server_id == graph.server_id {
            if associated.replace(server).is_some() {
                return Err(grid_failure("duplicate_owned_server"));
            }
        } else if server.connectivity != ServerConnectivity::Unavailable {
            return Err(grid_failure("additional_proved_server"));
        }
    }
    let server = associated.ok_or_else(|| grid_failure("missing_owned_server"))?;
    if server.connectivity != ServerConnectivity::Online
        || graph.freshness != NativeFreshness::Current
        || graph.reason.is_some()
        || !graph.panes.iter().any(|p| {
            p.pane_id == root.pane_id
                && p.terminal_id == root.terminal_id
                && p.workspace_id == root.workspace_id
                && p.tab_id == root.tab_id
        })
        || native
            .pane(&root.pane_id)
            .map_err(|_| grid_failure("root_binding"))?
            .read_identity
            != root.read_identity
        || expected.is_some_and(|(id, incarnation)| {
            id != &server.server_id || incarnation != &graph.server_incarnation
        })
    {
        return Err(grid_failure("original_owned_source"));
    }
    Ok((server, &graph.server_incarnation))
}
#[allow(clippy::too_many_arguments)]
async fn grid_source_loss(
    phone: &mut Phone,
    socket: &Path,
    source: &ServerEvidence,
    desktop: &ProcessEvidence,
    target: ShellTarget,
    native_scope: Value,
    server: &Uuid,
    incarnation: &Uuid,
    evidence: &Path,
    run: &Uuid,
    control: tokio::io::BufReader<McpStdin>,
) -> io::Result<()> {
    use futures_util::FutureExt;
    let setup = async {
        if herdr::process_evidence(desktop.pid)
            .map_err(|error| grid_context("source_ready_original", error))?
            != *desktop
        {
            return Err(grid_failure("source_ready_original"));
        }
        phone.grid_open(GridSubject::Shell(target.clone())).await?;
        let mut native = NativeCapacityReader::open(
            socket,
            source,
            "shell.grid_subscribe_bound",
            json!({"scope":native_scope}),
            json!({"kind":"shell","scope":native_scope}),
        )
        .await?
        .ok_or_else(|| grid_failure("source_witness_resource_limit"))?;
        native.initial_commit().await?;
        Ok::<_,io::Error>(native)
    }.await;
    let native=match setup{Ok(native)=>native,Err(error)=>return grid_source_blocked(evidence,run,error).await};
    let subscription=match phone.grid.as_ref().and_then(|g|g.subscription.clone()){
        Some(subscription)=>subscription,None=>return grid_source_blocked(evidence,run,grid_failure("source_viewer_original")).await,
    };
    let ready=Uuid::random();
    let mut loss=GridSourceLoss {run:run.clone(),ready:ready.clone(),trigger:None,native_eof:false,
        iroh_closed:false,sync_required:false,deadline:None};
    let pipe=match control.get_ref().0.get_ref().metadata(){
        Ok(pipe)=>pipe,Err(error)=>return grid_source_blocked(evidence,run,grid_context("source_control_original",error)).await,
    };
    let record=json!({"gate":"grid","phase":"source_loss_ready","run":run,"ready":ready,
        "serverId":server,"serverIncarnation":incarnation,"shellTarget":target,"subscription":subscription,
        "nativeSource":source,"nativeProcess":desktop,"nativeScope":native_scope,
        "control":"original_private_stdin_pipe","pipeDevice":pipe.dev(),"pipeInode":pipe.ino(),
        "armRecordVersion":1,"mustWaitForArmedRecordBeforeExternalTrigger":true,
        "cleanupAuthorized":false,"quietIdleTimeout":false,"sourceLossObserved":false});
    if let Err(error) = write_json(
        &evidence.join(format!("grid-source-loss-ready-{}.json", run.0)),
        &record,
    ) {
        return grid_source_blocked(evidence, run, grid_context("source_ready_evidence", error))
            .await;
    }
    let _ = writeln!(std::io::stdout(), "{record}");
    let mut native_wait = Box::pin(grid_source_native(native).fuse());
    let mut control_wait = Box::pin(grid_source_intent(control).fuse());
    let mut held_native = None;
    let mut held_control = None;
    let mut fin_seen = false;
    let mut pending_closed = false;
    let mut pending_intent = None;
    let original_subject = GridSubject::Shell(target.clone());
    let Phone {
        connection,
        recv,
        grid,
        events,
        generation,
        ..
    } = phone;
    let closed = connection.closed().fuse();
    tokio::pin!(closed);
    let observation = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(observation);
    let transaction = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(transaction);
    loop {
        // Preserve this exact read_frame future across native/control readiness. Cancelling
        // and recreating it on another branch could consume a header then retry a suffix.
        let received = {
            // Actual FIN never starts another read_frame, nor polls its completed Fuse.
            let read = if fin_seen {
                futures_util::future::Either::Left(std::future::pending::<io::Result<Vec<u8>>>())
            } else {
                futures_util::future::Either::Right(transport::read_frame(recv, FRAME_MAX))
            };
            let frame = read.fuse();
            tokio::pin!(frame);
            loop {
                if let Err(error) = grid_source_deadlines(&loss, grid) {
                    return grid_source_blocked(evidence, run, error).await;
                }
                if pending_closed {
                    match grid_source_handoff(
                        frame.as_mut(),
                        fin_seen,
                        &loss,
                        grid,
                        &subscription,
                        &original_subject,
                    ) {
                        Ok(GridSourceHandoff::Frame(result)) => break result,
                        Ok(GridSourceHandoff::Pending { .. }) => {
                            if let Err(error) = loss.downstream(false) {
                                return grid_source_blocked(evidence, run, error).await;
                            }
                            pending_closed = false;
                        }
                        Err(error) => return grid_source_blocked(evidence, run, error).await,
                    }
                }
                if pending_intent.is_some() {
                    match grid_source_handoff(
                        frame.as_mut(),
                        fin_seen,
                        &loss,
                        grid,
                        &subscription,
                        &original_subject,
                    ) {
                        Ok(GridSourceHandoff::Frame(result)) => break result,
                        Ok(GridSourceHandoff::Pending { .. }) => {
                            let Some(intent) = pending_intent.take() else {
                                return grid_source_blocked(
                                    evidence,
                                    run,
                                    grid_failure("source_intent_missing"),
                                )
                                .await;
                            };
                            if let Err(error) = loss.arm(intent) {
                                return grid_source_blocked(evidence, run, error).await;
                            }
                            let record = json!({"gate":"grid","phase":"source_loss_armed","run":run,"ready":loss.ready,
                                "trigger":loss.trigger,"sourceLossObserved":false,"cleanupAuthorized":false});
                            if let Err(error) = write_json(
                                &evidence.join(format!("grid-source-loss-armed-{}.json", run.0)),
                                &record,
                            ) {
                                return grid_source_blocked(
                                    evidence,
                                    run,
                                    grid_context("source_arm_evidence", error),
                                )
                                .await;
                            }
                        }
                        Err(error) => return grid_source_blocked(evidence, run, error).await,
                    }
                }
                if loss.complete() {
                    match grid_source_handoff(
                        frame.as_mut(),
                        fin_seen,
                        &loss,
                        grid,
                        &subscription,
                        &original_subject,
                    ) {
                        Ok(GridSourceHandoff::Frame(result)) => break result,
                        Ok(GridSourceHandoff::Pending { settled: true }) => {
                            // No await between final Pending/FIN+scope+deadline rejoin and decision.
                            let record = json!({"gate":"grid","phase":"source_loss_observed","run":run,"ready":loss.ready,
                                "trigger":loss.trigger,"subscription":subscription,"nativeOriginalFdEof":true,
                                "downstreamIrohClosed":loss.iroh_closed,"orderedSyncRequired":loss.sync_required,
                                "observationBudgetSeconds":15,"cleanupAuthorized":false,"fullAcceptance":false});
                            if let Err(error) = write_json(
                                &evidence.join(format!("grid-source-loss-result-{}.json", run.0)),
                                &record,
                            ) {
                                return grid_source_blocked(
                                    evidence,
                                    run,
                                    grid_context("source_result_evidence", error),
                                )
                                .await;
                            }
                            drop(held_native);
                            drop(held_control);
                            return Ok(());
                        }
                        Ok(GridSourceHandoff::Pending { settled: false }) => {}
                        Err(error) => return grid_source_blocked(evidence, run, error).await,
                    }
                }
                let observe_active = loss.native_eof;
                let transaction_active = grid.as_ref().is_some_and(|g| g.deadline.is_some());
                let read_active = !fin_seen;
                let wake = grid_source_wake(
                    native_wait.as_mut(),
                    closed.as_mut(),
                    async {
                        if observe_active {
                            observation.as_mut().await
                        } else {
                            std::future::pending::<()>().await
                        }
                    },
                    async {
                        if transaction_active {
                            transaction.as_mut().await
                        } else {
                            std::future::pending::<()>().await
                        }
                    },
                    async {
                        if read_active {
                            frame.as_mut().await
                        } else {
                            std::future::pending::<io::Result<Vec<u8>>>().await
                        }
                    },
                    control_wait.as_mut(),
                )
                .await;
                match wake {
                    GridSourceWake::Native((native,result))=>match result {
                        Ok(true)=>{
                            held_native=Some(native);
                            if let Err(error)=loss.upstream_eof(){return grid_source_blocked(evidence,run,error).await;}
                            if let Some(deadline)=loss.deadline{observation.as_mut().reset(deadline);}
                            else{return grid_source_blocked(evidence,run,grid_failure("source_observation_cutoff")).await;}
                        }
                        Ok(false)=>native_wait=Box::pin(grid_source_native(native).fuse()),
                        Err(error)=>{
                            let result=grid_source_blocked(evidence,run,error).await;
                            drop(native);
                            return result;
                        }
                    },
                    GridSourceWake::Closed => pending_closed = true,
                    GridSourceWake::ObservationDeadline => {
                        return grid_source_blocked(
                            evidence,
                            run,
                            grid_failure("source_downstream_deadline"),
                        )
                        .await;
                    }
                    GridSourceWake::TransactionDeadline => {
                        return grid_source_blocked(
                            evidence,
                            run,
                            grid_failure("source_grid_transaction_deadline"),
                        )
                        .await;
                    }
                    GridSourceWake::Frame(result) => break result,
                    GridSourceWake::Control((input, result)) => {
                        held_control = Some(input);
                        let intent = match result {
                            Ok(intent) => intent,
                            Err(error) => return grid_source_blocked(evidence, run, error).await,
                        };
                        pending_intent = Some(intent);
                    }
                }
            }
        };
        let bytes = match received {
            Ok(bytes) => bytes,
            Err(error) => {
                // read_frame's OUTER kind is InvalidData. Only its closed FrameFailure
                // metadata distinguishes actual FIN; FIN is not Iroh connection closure.
                if let Err(error) = grid_source_fin(loss.trigger.is_some(), error) {
                    return grid_source_blocked(evidence, run, error).await;
                }
                fin_seen = true;
                continue;
            }
        };
        let event=match decode_payload(&bytes,None).map_err(|_|grid_failure("source_phone_codec")){
            Ok(Envelope::Event(event))=>event,
            Ok(_)=>return grid_source_blocked(evidence,run,grid_failure("source_phone_correlation")).await,
            Err(error)=>return grid_source_blocked(evidence,run,error).await,
        };
        if event.subscription==subscription&&matches!(&event.body,EventBody::SyncRequired(_)){
            let Some(witness)=grid.as_mut()else{return grid_source_blocked(evidence,run,grid_failure("source_viewer_missing")).await;};
            if witness.retired||event.seq.0!=witness.next_seq{return grid_source_blocked(evidence,run,grid_failure("source_sync_order")).await;}
            witness.retired=true;witness.deadline=None;
            if let Err(error)=loss.downstream(true){return grid_source_blocked(evidence,run,error).await;}
        }else if let Err(error)=Phone::route_event(grid,events,generation,event,bytes.len()){
            return grid_source_blocked(evidence,run,error).await;
        }
        if let Some(deadline)=grid.as_ref().and_then(|g|g.deadline){transaction.as_mut().reset(tokio::time::Instant::from_std(deadline));}
    }
}
#[allow(clippy::too_many_arguments)]
async fn grid_flow(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    desktop: &ProcessEvidence,
    serve: &mut Companion,
    evidence: &Path,
    run: &Uuid,
    selected: &[Provider],
    hold: &mut GridHold,
) -> io::Result<bool> {
    if config.herdr_sockets.len() != 1 || config.herdr_sockets[0] != socket {
        return Err(grid_failure("sole_configured_socket"));
    }
    let (source, initial) = surface(socket)
        .await
        .map_err(|_| grid_failure("native_source"))?;
    if source.pid != desktop.pid
        || source.pid_start_ticks != desktop.start_ticks
        || source.linux_boot_id != desktop.linux_boot_id
    {
        return Err(grid_failure("native_peer"));
    }
    dev_continuity::zero_native_caps(desktop).map_err(|_| grid_failure("native_capabilities"))?;
    let [root] = initial.panes.as_slice() else {
        return Err(grid_failure("fresh_root_pane"));
    };
    let native_shell = root
        .shell
        .as_ref()
        .ok_or_else(|| grid_failure("root_shell_grant"))?;
    if root.agent.is_some()
        || root.agent_session.is_some()
        || !initial.agents.is_empty()
        || !initial.pane_grid_subscribe_bound
        || !initial.shell_grid_subscribe_bound
        || !initial.shell_read_bound
        || !native_shell.read
        || !native_shell.create
        || !native_shell.input
        || native_shell.close
        || root.read_identity.as_ref() != Some(&native_shell.scope.read_identity)
        || native_shell.scope.pane_id != root.pane_id
        || native_shell.scope.terminal_id != root.terminal_id
        || native_shell.scope.workspace_id != root.workspace_id
        || native_shell.scope.tab_id != root.tab_id
    {
        return Err(grid_failure("root_shell_grant"));
    }
    let admission = Instant::now() + Duration::from_secs(30);
    let (server, incarnation) = loop {
        let inventory = phone
            .host()
            .await
            .map_err(|_| grid_failure("source_inventory"))?;
        if let Ok((server, incarnation)) = grid_source(&inventory, &initial, &root, None) {
            if selected.iter().all(|p| match p {
                Provider::Omp => server.can_create.omp,
                Provider::Codex => server.can_create.codex,
            }) {
                break (server.server_id.clone(), incarnation.clone());
            }
        }
        if Instant::now() >= admission {
            return Err(grid_failure("selected_provider_unavailable"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let inputs = NativeInputs {
        enabled: true,
        paging_proof: false,
        config,
        socket,
        herdr: desktop,
        evidence,
        run,
        mcp: None,
        plan: RefCell::new(None),
        mcp_dialogs: RefCell::new(HashMap::new()),
    };
    let mut capacity_scope = None;
    for provider in selected.iter().copied() {
        serve
            .recheck()
            .map_err(|_| grid_failure("original_companion"))?;
        let (proof, created) = create(phone, config, socket, provider, &server, evidence, run)
            .await
            .map_err(|error| grid_context("provider_create", error))?;
        let (current, fresh) = inputs
            .current_checked(
                phone,
                &proof,
                Instant::now() + Duration::from_secs(30),
                &Cell::new("grid_guard"),
            )
            .await
            .map_err(|error| grid_context("original_ready", error))?;
        if !proof.same_input_binding(&fresh) {
            return Err(grid_failure("original_binding"));
        }
        let pane = current
            .pane(&proof.pane)
            .map_err(|_| grid_failure("provider_pane"))?;
        let read_pin = pane
            .read_identity
            .clone()
            .ok_or_else(|| grid_failure("provider_read_pin"))?;
        if capacity_scope.is_none() {
            capacity_scope = Some((
                json!({"pane_id":pane.pane_id,"expected_terminal_id":pane.terminal_id,"expected_read_identity":read_pin}),
                json!({"kind":"pane","pane_id":pane.pane_id,"terminal_id":pane.terminal_id,
                    "workspace_id":pane.workspace_id,"tab_id":pane.tab_id,"read_identity":read_pin}),
            ));
        }
        for process in std::iter::once(&proof.owner)
            .chain(proof.tui.iter())
            .chain(proof.relay.iter())
        {
            use std::os::fd::FromRawFd;
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, process.pid, 0) } as i32;
            if fd < 0 {
                return Err(grid_failure("provider_pidfd"));
            }
            hold.originals
                .push(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
            if herdr::process_evidence(process.pid).map_err(|_| grid_failure("provider_process"))?
                != *process
            {
                return Err(grid_failure("provider_process"));
            }
        }
        write_json(
            &evidence.join(format!("grid-provider-{}-{provider:?}.json", run.0)),
            &json!({
                "phase":"original_ready","provider":provider,"creationAction":created.action_id,"target":proof.target,
                "nativeId":proof.native,"paneId":proof.pane,"terminalId":proof.terminal,"workspaceId":proof.workspace,
                "nativeSource":source,"readPin":read_pin,"owner":proof.owner,"tui":proof.tui,"relay":proof.relay,
                "serverId":server,"serverIncarnation":incarnation
            }),
        )?;
        phone
            .grid_open(GridSubject::Pane(proof.target.clone()))
            .await?;
        inputs
            .current_checked(
                phone,
                &proof,
                Instant::now() + Duration::from_secs(30),
                &Cell::new("grid_guard"),
            )
            .await
            .map_err(|error| grid_context("original_ready_before_input", error))?;
        let token = hex(&random32()?[..16]);
        phone
            .grid
            .as_mut()
            .ok_or_else(|| grid_failure("viewer_missing"))?
            .arm(&token)?;
        let action = Uuid::random();
        let boot = phone.hello.boot_id.clone();
        write_json(
            &evidence.join(format!("grid-action-{}.json", action.0)),
            &json!({
                "run":run,"action":action,"boot":boot,"method":"prompt.submit","target":proof.target,
                "uncertainOutcomePolicy":"original_receipt_status_only_no_failed_reader_reuse","replayAuthorized":false
            }),
        )?;
        let receipt = phone
            .grid_action(
                action.clone(),
                RequestBody::PromptSubmit(PromptSubmitParams { skill: None, boot_id: boot.clone(),
                delivery: None, native_turn_id: None, images: None,
                target: proof.target.clone(),
                text: PromptText(format!("Reply only with {token}; no tools.")), }),
                false,
            )
            .await?;
        let receipt = receipt.ok_or_else(|| grid_failure("provider_transport_closed"))?;
        grid_receipt(
            &receipt,
            &action,
            &boot,
            &GridSubject::Pane(proof.target.clone()),
        )?;
        // Same framed stream: private grid traffic is routed while this ordinary RPC correlates.
        let inventory = phone
            .host()
            .await
            .map_err(|_| grid_failure("interleaved_rpc"))?;
        let (now_source, now) = surface(socket)
            .await
            .map_err(|_| grid_failure("native_rejoin"))?;
        if now_source != source {
            return Err(grid_failure("native_rejoin"));
        }
        grid_source(&inventory, &now, &root, Some((&server, &incarnation)))?;
        phone.grid_wait(GridGoal::Token).await?;
        write_json(
            &evidence.join(format!("grid-provider-result-{}-{provider:?}.json", run.0)),
            &json!({
                "phase":"committed_dirty","provider":provider,"action":action,"quietInitialCommitted":true,
                "laterTokenCommitted":true,"ordinaryRpcCorrelated":true,"providerCompletionClaim":false
            }),
        )?;
        phone.grid_unsubscribe().await?;
    }
    let (same_source, now) = surface(socket)
        .await
        .map_err(|_| grid_failure("shell_source"))?;
    if same_source != source
        || now
            .pane(&root.pane_id)
            .map_err(|_| grid_failure("root_binding"))?
            .shell
            != root.shell
    {
        return Err(grid_failure("root_binding"));
    }
    let inventory = phone
        .host()
        .await
        .map_err(|_| grid_failure("shell_inventory"))?;
    grid_source(&inventory, &now, &root, Some((&server, &incarnation)))?;
    let mut shells = inventory.shells.iter().filter(|s| {
        s.server_id == server
            && s.pane_id == root.pane_id
            && s.terminal_id == root.terminal_id
            && s.workspace_id == root.workspace_id
            && s.tab_id == root.tab_id
    });
    let shell = shells
        .next()
        .ok_or_else(|| grid_failure("exact_root_shell"))?;
    if shells.next().is_some() {
        return Err(grid_failure("exact_root_shell"));
    }
    if shell.connectivity != SessionConnectivity::Ready
        || !shell.capabilities.read
        || !shell.capabilities.grid
        || !shell.capabilities.input
        || shell.capabilities.close
    {
        return Err(grid_failure("shell_authority"));
    }
    let shell_target = shell.target.clone();
    phone
        .grid_open(GridSubject::Shell(shell_target.clone()))
        .await?;
    let (current_source, current_native) = surface(socket)
        .await
        .map_err(|error| grid_context("shell_input_rejoin", error))?;
    let current_inventory = phone
        .host()
        .await
        .map_err(|error| grid_context("shell_input_inventory", error))?;
    grid_source(
        &current_inventory,
        &current_native,
        &root,
        Some((&server, &incarnation)),
    )?;
    if current_source != source
        || current_native
            .pane(&root.pane_id)
            .map_err(|error| grid_context("shell_input_root", error))?
            .shell
            != root.shell
        || herdr::process_evidence(desktop.pid)
            .map_err(|error| grid_context("shell_input_process", error))?
            != *desktop
        || !current_inventory.shells.iter().any(|s| {
            s.target == shell_target
                && s.server_id == server
                && s.pane_id == root.pane_id
                && s.terminal_id == root.terminal_id
                && s.workspace_id == root.workspace_id
                && s.tab_id == root.tab_id
                && s.connectivity == SessionConnectivity::Ready
                && s.capabilities.read
                && s.capabilities.grid
                && s.capabilities.input
                && !s.capabilities.close
        })
    {
        return Err(grid_failure("shell_input_original_authority"));
    }
    let token = hex(&random32()?[..16]);
    phone
        .grid
        .as_mut()
        .ok_or_else(|| grid_failure("viewer_missing"))?
        .arm(&token)?;
    let action = Uuid::random();
    let boot = phone.hello.boot_id.clone();
    let subscription=phone.grid.as_ref().and_then(|grid|grid.subscription.clone()).ok_or_else(||grid_failure("shell_input_original_subscription"))?;
    write_json(
        &evidence.join(format!("grid-action-{}.json", action.0)),
        &json!({
            "run":run,"action":action,"boot":boot,"method":"shell.input","shellTarget":shell_target,
            "uncertainOutcomePolicy":"original_receipt_status_only_no_failed_reader_reuse","replayAuthorized":false
        }),
    )?;
    let receipt = phone
        .grid_action(
            action.clone(),
            RequestBody::ShellInput(ShellInputParams {
                boot_id: boot.clone(),
                target: shell_target.clone(),
                subscription,
                input: ShellInput::Text { text: token },
            }),
            true,
        )
        .await?;
    let Some(receipt) = receipt else {
        write_json(
            &evidence.join(format!("grid-shell-result-{}.json", run.0)),
            &json!({
                "phase":"slow_consumer","target":shell_target,"action":action,"quietInitialCommitted":true,
                "slowConsumerPauseSeconds":6,"slowConsumerOutcome":"closed_transport",
                "laterTokenCommitted":false,"shellDirtyGate":"blocked","fullAcceptance":false
            }),
        )?;
        return Ok(false);
    };
    grid_receipt(
        &receipt,
        &action,
        &boot,
        &GridSubject::Shell(shell_target.clone()),
    )?;
    phone.grid_wait(GridGoal::Token).await?;
    phone.grid_unsubscribe().await?;
    write_json(
        &evidence.join(format!("grid-shell-result-{}.json", run.0)),
        &json!({
            "phase":"committed_dirty","target":shell_target,"action":action,"rootPaneId":root.pane_id,
            "quietInitialCommitted":true,"laterTokenCommitted":true,"inputReceiptAccepted":true,
            "slowConsumerPauseSeconds":6,"slowConsumerOutcome":"valid_bounded_progress","postUnsubscribeRpc":true,
            "serverId":server,"serverIncarnation":incarnation,"nativeSource":source,"nativeScope":native_shell.scope
        }),
    )?;
    let shell_scope = serde_json::to_value(&native_shell.scope)
        .map_err(|_| grid_failure("capacity_shell_scope"))?;
    native_grid_capacity(
        socket,
        &source,
        capacity_scope.ok_or_else(|| grid_failure("capacity_provider"))?,
        shell_scope,
        evidence,
        run,
    )
    .await?;
    let control = hold
        .source_control
        .take()
        .ok_or_else(|| grid_failure("source_control_missing"))?;
    let scope =
        serde_json::to_value(&native_shell.scope).map_err(|_| grid_failure("source_scope"))?;
    grid_source_loss(
        phone,
        socket,
        &source,
        desktop,
        shell_target,
        scope,
        &server,
        &incarnation,
        evidence,
        run,
        control,
    )
    .await?;
    hold.source_loss_observed = true;
    write_json(
        &evidence.join(format!("grid-scope-{}.json", run.0)),
        &json!({
            "gate":"grid","selectedProvidersCompleted":selected.len(),"separateShellCompleted":true,"nativeCapacityCompleted":true,
            "nativeSourceEofGate":"observed_original_native_fd_and_downstream","quantitativeMemoryProof":false,
            "phoneUiQualification":false,"scaleQualification":false,"fullAcceptance":false
        }),
    )?;
    Ok(true)
}
fn admission_reason(provider: Provider, backends: &[Backend], server: Option<&Server>, hierarchy: &[NativeHierarchy]) -> &'static str {
    if backends.iter().filter(|backend| backend.provider == provider).count() != 1 {
        return "backend_missing_or_ambiguous";
    }
    if backends.iter().any(|backend| backend.provider == provider
        && backend.availability == Availability::Unavailable) {
        return "backend_unavailable";
    }
    let Some(server) = server else { return "source_missing_or_ambiguous" };
    if server.connectivity != ServerConnectivity::Online { return "source_not_online" }
    if hierarchy.iter().filter(|source| source.server_id == server.server_id
        && source.freshness == NativeFreshness::Current).count() != 1 {
        return "source_not_current";
    }
    if match provider { Provider::Omp => server.can_create.omp, Provider::Codex => server.can_create.codex } {
        "eligible"
    } else {
        "creation_unavailable"
    }
}
struct AdmittedSource {
    identity: (Uuid,Uuid),
    client: Client,
    process: ProcessEvidence,
    native: Snapshot,
}
impl AdmittedSource {
    fn recheck(&self) -> io::Result<()> {
        self.client.recheck()?;
        if herdr::process_evidence(self.process.pid)?!=self.process {
            return Err(prerequisite("original admitted Herdr process/namespace changed"));
        }
        Ok(())
    }
    fn observation(&self,actual: &ServerEvidence,native: &Snapshot,inventory: &HostSnapshotResult) -> io::Result<bool> {
        self.recheck()?;
        creation_source_observation(&self.identity,&self.client.evidence,&self.native,actual,native,inventory)
            .map_err(prerequisite)
    }
}
fn creation_source_observation(
    identity: &(Uuid,Uuid),original: &ServerEvidence,initial: &Snapshot,
    actual: &ServerEvidence,native: &Snapshot,inventory: &HostSnapshotResult,
) -> Result<bool,&'static str> {
    if actual!=original {return Err("original creation socket/process source changed");}
    if !initial.workspaces.iter().all(|row|native.workspaces.iter().any(|current|current.workspace_id==row.workspace_id))
        || !initial.tabs.iter().all(|row|native.tabs.iter().any(|current|current.tab_id==row.tab_id&&current.workspace_id==row.workspace_id))
        || !initial.panes.iter().all(|row|native.panes.iter().any(|current|current.pane_id==row.pane_id
            &&current.terminal_id==row.terminal_id&&current.workspace_id==row.workspace_id&&current.tab_id==row.tab_id)) {
        return Err("original creation snapshot binding changed");
    }
    let mut servers=inventory.servers.iter().filter(|server|server.server_id==identity.0);
    let Some(server)=servers.next() else {return Ok(false)};
    if servers.next().is_some() {return Err("original creation server is ambiguous");}
    match admission_source(inventory,&server.name.0,native,Some(identity)) {
        Ok(_)=>Ok(true),
        Err("source_replaced")=>Err("original creation server incarnation changed"),
        Err(_)=>Ok(false),
    }
}
struct AdmissionSource<'a> {
    socket: &'a Path,
    desktop: &'a mut Desktop,
    original: &'a ServerEvidence,
    native: Snapshot,
}
impl AdmissionSource<'_> {
    async fn connect(&mut self) -> io::Result<Client> {
        self.desktop.recheck()?;
        let client = Client::connect(self.socket).await?;
        if client.evidence != *self.original {
            return Err(prerequisite("original admission socket/process source changed"));
        }
        self.recheck(&client)?;
        Ok(client)
    }
    fn recheck(&mut self, client: &Client) -> io::Result<()> {
        client.recheck()?;
        self.desktop.recheck()
    }
}
fn admission_source<'a>(
    inventory: &'a HostSnapshotResult,
    name: &str,
    native: &Snapshot,
    pinned: Option<&(Uuid, Uuid)>,
) -> Result<(&'a Server, &'a Uuid), &'static str> {
    let mut servers = inventory.servers.iter().filter(|server| server.name.0 == name);
    let server = servers.next().ok_or("source_missing_or_ambiguous")?;
    if servers.next().is_some()
        || inventory.servers.iter().filter(|candidate|candidate.server_id==server.server_id).count()!=1 {
        return Err("source_missing_or_ambiguous");
    }
    let mut graphs = inventory.hierarchy.iter().filter(|graph|graph.server_id==server.server_id);
    let graph = graphs.next().ok_or("source_not_current")?;
    if graphs.next().is_some() { return Err("source_not_current"); }
    if pinned.is_some_and(|(id, incarnation)| *id!=server.server_id || *incarnation!=graph.server_incarnation) {
        return Err("source_replaced");
    }
    if graph.freshness!=NativeFreshness::Current || graph.reason.is_some()
        || graph.workspaces.len()!=native.workspaces.len()
        || graph.tabs.len()!=native.tabs.len() || graph.panes.len()!=native.panes.len()
        || !native.workspaces.iter().all(|row|graph.workspaces.iter().any(|shown|shown.workspace_id==row.workspace_id))
        || !native.tabs.iter().all(|row|graph.tabs.iter().any(|shown|shown.tab_id==row.tab_id&&shown.workspace_id==row.workspace_id))
        || !native.panes.iter().all(|row|graph.panes.iter().any(|shown|shown.pane_id==row.pane_id
            &&shown.terminal_id==row.terminal_id&&shown.tab_id==row.tab_id&&shown.workspace_id==row.workspace_id)) {
        return Err("source_not_current");
    }
    Ok((server,&graph.server_incarnation))
}
async fn creation_admission(phone: &mut Phone, selected: &[Provider], name: &str, evidence: &Path, run: &Uuid, mut owned: AdmissionSource<'_>) -> io::Result<Option<AdmittedSource>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut pinned = None;
    loop {
        let original = owned.connect().await?;
        let inventory = phone.host().await?;
        owned.recheck(&original)?;
        let (server, source_reason) = match admission_source(&inventory,name,&owned.native,pinned.as_ref()) {
            Ok((server, incarnation)) => {
                if pinned.is_none() {
                    let manifest=evidence.join(format!("admission-binding-{}.json",run.0));
                    write_json(&manifest,&json!({"runId":run,"serverId":server.server_id,
                        "serverIncarnation":incarnation,"originalSource":owned.original,
                        "originalProcess":owned.desktop.process,"socket":owned.socket,
                        "scope":"first coherent inventory joined across original kernel process/socket proof; never retargeted"}))?;
                    println!("{}",json!({"gate":"admission-binding","manifest":manifest}));
                    pinned=Some((server.server_id.clone(),incarnation.clone()));
                }
                (Some(server),None)
            }
            Err(reason) => (None,Some(reason)),
        };
        let results = selected.iter().map(|provider| {
            let reason = source_reason.unwrap_or_else(||admission_reason(*provider, &phone.hello.backends, server, &inventory.hierarchy));
            json!({"gate":"admission-only","runId":run,"provider":provider,
                "status":if reason=="eligible" {"eligible"} else {"unavailable"},"reason":reason,
                "serverId":pinned.as_ref().map(|(id,_)|id),
                "serverIncarnation":pinned.as_ref().map(|(_,incarnation)|incarnation),
                "scope":"Hello/current source/creation eligibility only; no provider created or input sent"})
        }).collect::<Vec<_>>();
        let eligible = results.iter().all(|result|result["status"]=="eligible");
        if eligible || source_reason==Some("source_replaced") || Instant::now() >= deadline {
            write_json(&evidence.join(format!("admission-{}.json",run.0)), &json!({"runId":run,"results":results}))?;
            for result in results { println!("{result}"); }
            return Ok(if eligible {
                pinned.map(|identity|AdmittedSource{identity,client:original,process:owned.desktop.process.clone(),native:owned.native})
            } else {None});
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
fn terminal_native_pane<'a>(native:&'a Snapshot,route:&ShellSummary)->io::Result<&'a herdr::Pane> {
    let pane=native.pane(&route.pane_id)?;
    if pane.terminal_id!=route.terminal_id||pane.workspace_id!=route.workspace_id||pane.tab_id!=route.tab_id
        ||pane.agent.is_some()||pane.display_agent.is_some()||pane.agent_session.is_some()||pane.restore_error.is_some()
        ||native.agents.iter().any(|agent|agent.pane_id==pane.pane_id||agent.terminal_id==pane.terminal_id)
        ||native.agents.iter().any(|agent|agent.launch_pending)||native.panes.iter().any(|pane|pane.restore_error.is_some())
    {return Err(prerequisite("terminal-only original non-agent route or full unsafe-state screen changed"));}
    Ok(pane)
}
fn terminal_select(inventory:&HostSnapshotResult,native:&Snapshot,name:&str,pinned:Option<&(Uuid,Uuid)>)->io::Result<Option<(ShellSummary,Uuid)>> {
    let (server,incarnation)=match admission_source(inventory,name,native,pinned){
        Ok(found)=>found,Err("source_replaced")=>return Err(prerequisite("terminal-only original server incarnation changed")),
        Err(_)=>return Ok(None),
    };
    if server.connectivity!=ServerConnectivity::Online{return Ok(None);}
    if native.workspaces.len()!=1||native.tabs.len()!=1||native.panes.len()!=1||!native.agents.is_empty(){
        return Err(prerequisite("terminal-only fresh startup must contain exactly one ordinary original root"));
    }
    let pane=&native.panes[0];
    let mut rows=inventory.shells.iter().filter(|shell|shell.server_id==server.server_id
        &&shell.pane_id==pane.pane_id&&shell.terminal_id==pane.terminal_id
        &&shell.workspace_id==pane.workspace_id&&shell.tab_id==pane.tab_id);
    let Some(shell)=rows.next() else{return Ok(None)};
    if rows.next().is_some(){return Err(prerequisite("terminal-only shell route is ambiguous"));}
    terminal_native_pane(native,shell)?;
    let graph=inventory.hierarchy.iter().find(|graph|graph.server_id==server.server_id).ok_or_else(||prerequisite("terminal-only current hierarchy absent"))?;
    if !graph.panes.iter().any(|shown|shown.pane_id==pane.pane_id&&shown.shell_target.as_ref()==Some(&shell.target)
        &&shown.target.is_none()&&shown.unbound_row_id.is_none())
        ||shell.connectivity!=SessionConnectivity::Ready||!shell.capabilities.read||!shell.capabilities.grid
    {return Ok(None);}
    Ok(Some((shell.clone(),incarnation.clone())))
}
async fn terminal_current(phone:&mut Phone,source:&AdmittedSource,socket:&Path,desktop:&mut Desktop,route:&ShellSummary)->io::Result<Snapshot> {
    desktop.recheck()?;source.recheck()?;
    let (actual,native)=surface(socket).await?;
    let inventory=phone.host().await?;
    if !source.observation(&actual,&native,&inventory)?{return Err(prerequisite("terminal-only original current source unavailable"));}
    let selected=terminal_select(&inventory,&native,&inventory.servers.iter().find(|server|server.server_id==source.identity.0).ok_or_else(||prerequisite("terminal-only original server disappeared"))?.name.0,Some(&source.identity))?
        .ok_or_else(||prerequisite("terminal-only original logical viewer authority unavailable"))?.0;
    if selected.target!=route.target{return Err(prerequisite("terminal-only original shell generation changed"));}
    terminal_native_pane(&native,route)?;desktop.recheck()?;source.recheck()?;Ok(native)
}
fn terminal_idle_pid(info:&herdr::PaneProcessInfo)->Option<u32> {
    let pid=info.shell_pid?;
    (info.foreground_process_group_id==Some(pid)&&!info.foreground_processes.is_empty()
        &&info.foreground_processes.iter().all(|process|process.pid==pid)).then_some(pid)
}
struct TerminalShellProcess {process:ProcessEvidence,reported_pid:u32,executable:File,path:PathBuf,cwd:(u64,u64)}
impl TerminalShellProcess {
    fn capture(reporter:&ProcessEvidence,info:&herdr::PaneProcessInfo,project:&Path)->io::Result<Option<Self>> {
        let Some(pid)=terminal_idle_pid(info) else{return Ok(None)};
        let process=herdr::reported_process(reporter,pid)?;
        dev_continuity::zero_native_caps(&process)?;
        let proc_path=PathBuf::from(format!("/proc/{}/exe",process.pid));
        let path=fs::read_link(&proc_path)?;
        if !path.starts_with("/nix/store")||!matches!(path.file_name().and_then(|value|value.to_str()),Some("bash"|"zsh")){
            return Err(prerequisite("terminal-only fixture requires a proved installed bash/zsh ELF"));
        }
        let mut executable=File::open(&proc_path)?;let mut magic=[0;4];executable.read_exact(&mut magic)?;
        if magic!=[0x7f,b'E',b'L',b'F']{return Err(prerequisite("terminal-only shell executable is not an ELF"));}
        let held=executable.metadata()?;let selected=fs::metadata(&path)?;
        if !held.is_file()||(held.dev(),held.ino())!=(selected.dev(),selected.ino()){
            return Err(prerequisite("terminal-only selected shell executable object unavailable"));
        }
        let approved=fs::metadata(project)?;let cwd=fs::metadata(format!("/proc/{}/cwd",process.pid))?;
        if !approved.is_dir()||(cwd.dev(),cwd.ino())!=(approved.dev(),approved.ino()){
            return Err(prerequisite("terminal-only fresh shell actual cwd differs from owned project"));
        }
        let proof=Self{process,reported_pid:pid,executable,path,cwd:(cwd.dev(),cwd.ino())};proof.recheck()?;
        if herdr::process_evidence(reporter.pid)?!=*reporter{return Err(prerequisite("terminal-only original reporting process changed"));}
        Ok(Some(proof))
    }
    fn recheck_idle(&self,reporter:&ProcessEvidence,info:&herdr::PaneProcessInfo,pane:&Id)->io::Result<()> {
        if &info.pane_id!=pane||terminal_idle_pid(info)!=Some(self.reported_pid)
            ||herdr::reported_process(reporter,self.reported_pid)?!=self.process
        {return Err(prerequisite("terminal-only original shell foreground/membership changed before input"));}
        self.recheck()?;
        if herdr::process_evidence(reporter.pid)?!=*reporter{return Err(prerequisite("terminal-only original idle reporter changed"));}
        Ok(())
    }
    fn recheck(&self)->io::Result<()> {
        if herdr::process_evidence(self.process.pid)?!=self.process{return Err(prerequisite("terminal-only original shell process changed"));}
        let current=fs::metadata(format!("/proc/{}/exe",self.process.pid))?;let held=self.executable.metadata()?;
        let cwd=fs::metadata(format!("/proc/{}/cwd",self.process.pid))?;
        if (current.dev(),current.ino())!=(held.dev(),held.ino())||(cwd.dev(),cwd.ino())!=self.cwd{
            return Err(prerequisite("terminal-only shell executable/cwd object changed"));
        }
        dev_continuity::zero_native_caps(&self.process)
    }
}
fn terminal_fixture_command()->String {
    fn octal(value:&str)->String {use std::fmt::Write;let mut encoded=String::with_capacity(value.len()*4);for byte in value.bytes(){write!(&mut encoded,"\\{byte:03o}").expect("String write");}encoded}
    // Public fixture text is encoded even in the command, so echoed input cannot prove output.
    let first="\u{1b}[2J\u{1b}[HGOATRSTOCKBEGIN\nGOATRUC: 界|ｶﾞ|👩‍💻|❤️|🇺🇸\nGOATRCOLOR: \u{1b}[31mR\u{1b}[0m|\u{1b}[38;5;202mI\u{1b}[0m|\u{1b}[38;2;17;34;51mT\u{1b}[0m|D\n";
    format!("builtin printf '{}'; builtin command sleep 20; builtin printf '{}'",octal(first),octal("\nGOATRSTOCKDELAYED\n"))
}
async fn terminal_fixture(phone:&mut Phone,config:&Config,socket:&Path,desktop:&mut Desktop,source:&AdmittedSource,route:&ShellSummary,evidence:&Path,run:&Uuid)->io::Result<TerminalShellProcess> {
    let deadline=Instant::now()+Duration::from_secs(30);
    let shell=loop {
        let native=terminal_current(phone,source,socket,desktop,route).await?;
        let pane=terminal_native_pane(&native,route)?;
        if native.focused_workspace_id.as_ref()!=Some(&pane.workspace_id)||native.focused_tab_id.as_ref()!=Some(&pane.tab_id)||native.focused_pane_id.as_ref()!=Some(&pane.pane_id){
            return Err(prerequisite("terminal-only startup root is not the original foreground selection"));
        }
        let client=Client::connect(socket).await?;
        if client.evidence!=source.client.evidence{return Err(prerequisite("terminal-only process-info original source changed"));}
        let info=client.process_info(&route.pane_id).await?;
        if let Some(shell)=TerminalShellProcess::capture(&source.process,&info,&config.projects[0].path)?{break shell;}
        if Instant::now()>=deadline{return Err(prerequisite("terminal-only fresh original shell did not become provably idle"));}
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let native=terminal_current(phone,source,socket,desktop,route).await?;
    terminal_native_pane(&native,route)?;shell.recheck()?;
    let client=Client::connect(socket).await?;
    if client.evidence!=source.client.evidence{return Err(prerequisite("terminal-only fixture input original source changed"));}
    let input=Uuid::random();let intent=evidence.join(format!("terminal-fixture-{}-{}.json",run.0,input.0));
    let metadata=shell.executable.metadata()?;
    write_json(&intent,&json!({"runId":run,"inputId":input,"method":"pane.send_input","state":"dispatching",
        "shellTarget":route.target,"serverId":route.server_id,"workspaceId":route.workspace_id,"tabId":route.tab_id,
        "paneId":route.pane_id,"terminalId":route.terminal_id,"originalHerdr":source.process,"source":source.client.evidence,
        "shell":shell.process,"shellExecutable":{"path":shell.path,"device":metadata.dev(),"inode":metadata.ino(),"bytes":metadata.len()},
        "fixture":"fixed public printf Unicode/color/delayed-output only; payload not retained","ackMeans":"queued, not execution",
        "uncertainRule":"never repeat or correct input; retain resources"}))?;
    // All readiness RPCs/connections precede this last foreground query. Its returned original
    // process/group/membership proof and synchronous source/object rechecks directly precede input.
    let final_guard:io::Result<()>=async {
        let foreground=Client::connect(socket).await?;
        if foreground.evidence!=source.client.evidence{return Err(prerequisite("terminal-only final foreground source changed"));}
        let info=foreground.process_info(&route.pane_id).await?;
        terminal_native_pane(&native,route)?;desktop.recheck()?;source.recheck()?;client.recheck()?;
        shell.recheck_idle(&source.process,&info,&route.pane_id)?;
        if Instant::now()>=deadline{return Err(prerequisite("terminal-only original fixture deadline passed before input"));}
        Ok(())
    }.await;
    if let Err(error)=final_guard {
        write_native_outcome(&intent,&json!({"runId":run,"inputId":input,"state":"not_dispatched","category":"final_idle_guard",
            "queueAcknowledged":false,"executionProved":false,"replayAllowed":false}))?;
        return Err(error);
    }
    let outcome=client.request("pane.send_input",json!({"pane_id":route.pane_id,"text":terminal_fixture_command(),"keys":["Enter"]}),"ok").await;
    let ack=outcome.is_ok();
    let post=if ack{terminal_current(phone,source,socket,desktop,route).await.and_then(|_|shell.recheck())}else{Err(failure("terminal-only fixture input entered outcome unavailable"))};
    write_native_outcome(&intent,&json!({"runId":run,"inputId":input,"state":if ack&&post.is_ok(){"acknowledged"}else{"uncertain"},
        "queueAcknowledged":ack,"sameOriginalAfter":post.is_ok(),"executionProved":false,"payloadRetained":false,"replayAllowed":false}))?;
    if !ack||post.is_err(){return Err(failure("terminal-only fixture outcome uncertain; no correction/replay, originals retained"));}
    Ok(shell)
}
fn terminal_resize_receipt(receipt:&Receipt,id:&Uuid,boot:&Uuid,target:&ShellTarget,columns:u64,rows:u64)->io::Result<()> {
    receipt.validate().map_err(|_|grid_failure("terminal_resize_receipt"))?;
    if &receipt.action_id!=id||&receipt.boot_id!=boot||receipt.method!=ActionMethod::ShellResize
        ||receipt.shell_target.as_ref()!=Some(target)||receipt.target.is_some()||receipt.creation.is_some()
        ||receipt.state!=ReceiptState::Succeeded||receipt.error.is_some()
        ||!matches!(&receipt.result,Some(ActionResult::ShellResize(result))if result.columns.0==columns&&result.rows.0==rows)
    {return Err(grid_failure("terminal_resize_authoritative_dimensions"));}
    Ok(())
}
fn terminal_input_scope(receipt:&Receipt,id:&Uuid,boot:&Uuid,target:&ShellTarget)->io::Result<()> {
    receipt.validate().map_err(|_|grid_failure("terminal_input_receipt"))?;
    if receipt.action_id!=*id||receipt.boot_id!=*boot||receipt.method!=ActionMethod::ShellInput
        ||receipt.shell_target.as_ref()!=Some(target)||receipt.target.is_some()||receipt.creation.is_some()
    {return Err(grid_failure("terminal_input_original_receipt"));}Ok(())
}
async fn terminal_goatr_input(phone:&mut Phone,route:&ShellSummary,subscription:&Uuid,input:ShellInput,evidence:&Path,run:&Uuid,reader_safe:&mut bool)->io::Result<Receipt> {
    let action=Uuid::random();let boot=phone.hello.boot_id.clone();
    let kind=match &input{ShellInput::Text{..}=>"text_commit",ShellInput::Keys{..}=>"keys"};
    let intent=evidence.join(format!("terminal-input-{}-{}.json",run.0,action.0));
    write_json(&intent,&json!({"runId":run,"actionId":action,"bootId":boot,"method":"shell.input","shellTarget":route.target,
        "subscription":subscription,"inputKind":kind,"state":"dispatching","payloadRetained":false,"replayAllowed":false,
        "outcomeMeaning":"complete original native socket frame only; no PTY acceptance/flush/consume acknowledgment"}))?;
    *reader_safe=false;
    let outcome=phone.request_at(action.clone(),RequestBody::ShellInput(ShellInputParams{boot_id:boot.clone(),target:route.target.clone(),subscription:subscription.clone(),input}),"terminal_input").await;
    let receipt=match outcome {
        Ok(Ok(ResultBody::ShellInput(result)))=>result.receipt,
        Ok(Err(error))=>{
            *reader_safe=true;
            write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"state":"request_rejected","code":error.code,"replayAllowed":false}))?;
            return Err(grid_failure("terminal_input_request_rejected"));
        }
        _=>{
            write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"state":"uncertain","replayAllowed":false}))?;
            return Err(grid_failure("terminal_input_entered_unknown"));
        }
    };
    terminal_input_scope(&receipt,&action,&boot,&route.target)?;
    let receipt=if matches!(receipt.state,ReceiptState::Reserved|ReceiptState::Dispatching) {
        phone.settled(&action).await?
    }else{receipt};
    terminal_input_scope(&receipt,&action,&boot,&route.target)?;*reader_safe=true;
    write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"receipt":receipt,"payloadRetained":false,"replayAllowed":false}))?;
    grid_receipt(&receipt,&action,&boot,&GridSubject::Shell(route.target.clone()))?;Ok(receipt)
}
fn terminal_creation_scope(receipt:&Receipt,id:&Uuid,boot:&Uuid)->io::Result<()> {
    receipt.validate().map_err(|_|grid_failure("terminal_create_receipt"))?;
    if receipt.action_id!=*id||receipt.boot_id!=*boot||receipt.method!=ActionMethod::TerminalCreate
        ||receipt.target.is_some()||receipt.shell_target.is_some()||receipt.creation.is_some()
    {return Err(grid_failure("terminal_create_original_receipt"));}Ok(())
}
async fn terminal_goatr_create(phone:&mut Phone,config:&Config,socket:&Path,desktop:&mut Desktop,source:&AdmittedSource,route:&ShellSummary,evidence:&Path,run:&Uuid)->io::Result<Value> {
    let before=terminal_current(phone,source,socket,desktop,route).await?;
    let action=Uuid::random();let boot=phone.hello.boot_id.clone();let project=&config.projects[0];
    let intent=evidence.join(format!("terminal-create-{}-{}.json",run.0,action.0));
    write_json(&intent,&json!({"runId":run,"actionId":action,"bootId":boot,"method":"terminal.create","serverId":route.server_id,
        "workspaceId":route.workspace_id,"projectId":project.id,"source":ArtifactSource::Shell{target:route.target.clone()},"directory":"configured project root",
        "state":"dispatching","focus":false,"replayAllowed":false,"resourceCleanup":"retain originals for coordinator review"}))?;
    let outcome=phone.request_at(action.clone(),RequestBody::TerminalCreate(TerminalCreateParams{boot_id:boot.clone(),server_id:route.server_id.clone(),
        workspace_id:route.workspace_id.clone(),project_id:project.id.clone(),directory_ref:None,source:Some(ArtifactSource::Shell{target:route.target.clone()})}),"terminal_create").await;
    let receipt=match outcome {
        Ok(Ok(ResultBody::TerminalCreate(result)))=>result.receipt,
        Ok(Err(error))=>{
            write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"state":"request_rejected","code":error.code,"replayAllowed":false}))?;
            return Err(grid_failure("terminal_create_request_rejected"));
        }
        _=>{
            write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"state":"uncertain","replayAllowed":false}))?;
            return Err(grid_failure("terminal_create_entered_unknown"));
        }
    };
    terminal_creation_scope(&receipt,&action,&boot)?;
    let receipt=if matches!(receipt.state,ReceiptState::Reserved|ReceiptState::Dispatching){phone.settled(&action).await?}else{receipt};
    terminal_creation_scope(&receipt,&action,&boot)?;
    // Preserve known resource IDs BEFORE any independent read can fail. Nothing is corrected or closed.
    write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"receipt":receipt,"replayAllowed":false}))?;
    let Some(ActionResult::ShellCreation(result))=&receipt.result else{return Err(grid_failure("terminal_create_resource_facts"));};
    let resources=&result.resources;
    if receipt.state!=ReceiptState::Succeeded||receipt.error.is_some()||!resources.complete()
        ||resources.server_id.as_ref()!=Some(&route.server_id)||resources.workspace_id.as_ref()!=Some(&route.workspace_id)
        ||resources.pane_id.as_ref()==Some(&route.pane_id)||resources.terminal_id.as_ref()==Some(&route.terminal_id)||resources.tab_id.as_ref()==Some(&route.tab_id)
    {return Err(grid_failure("terminal_create_actual_cwd_or_original_workspace_unproved"));}
    let target=resources.created_target.as_ref().ok_or_else(||grid_failure("terminal_create_target"))?;
    let end=Instant::now()+Duration::from_secs(30);
    let created=loop {
        desktop.recheck()?;source.recheck()?;
        let (actual,native)=surface(socket).await?;let inventory=phone.host().await?;
        if !source.observation(&actual,&native,&inventory)?{return Err(prerequisite("terminal-only created resource current source unavailable"));}
        terminal_native_pane(&native,route)?;
        if native.focused_workspace_id!=before.focused_workspace_id||native.focused_tab_id!=before.focused_tab_id||native.focused_pane_id!=before.focused_pane_id {
            return Err(grid_failure("terminal_create_native_focus_changed"));
        }
        if let Some(created)=inventory.shells.iter().find(|shell|&shell.target==target&&shell.connectivity==SessionConnectivity::Ready&&shell.capabilities.read&&shell.capabilities.input) {
            if resources.pane_id.as_ref()!=Some(&created.pane_id)||resources.terminal_id.as_ref()!=Some(&created.terminal_id)
                ||resources.tab_id.as_ref()!=Some(&created.tab_id)||created.workspace_id!=route.workspace_id||created.server_id!=route.server_id
            {return Err(grid_failure("terminal_create_inventory_resource_join"));}
            terminal_native_pane(&native,created)?;
            let client=Client::connect(socket).await?;
            if client.evidence!=source.client.evidence{return Err(grid_failure("terminal_create_process_info_source"));}
            let info=client.process_info(&created.pane_id).await?;
            if let Some(shell)=TerminalShellProcess::capture(&source.process,&info,&project.path)? {break shell;}
        }
        if Instant::now()>=end{return Err(prerequisite("terminal-only created original shell process/cwd inventory join unavailable"));}
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let executable=created.executable.metadata()?;
    let manifest=evidence.join(format!("terminal-created-ownership-{}-{}.json",run.0,action.0));
    write_json(&manifest,&json!({"runId":run,"actionId":action,"receipt":receipt,"serverId":source.identity.0,"serverIncarnation":source.identity.1,
        "originalHerdr":source.process,"socketSource":source.client.evidence,"createdShell":created.process,
        "cwdObject":{"device":created.cwd.0,"inode":created.cwd.1},"shellExecutable":{"device":executable.dev(),"inode":executable.ino(),"bytes":executable.len()},
        "nativeFocusUnchanged":true,"providerLaunched":false,"inputSentToNewShell":false,"cleanup":"retained; coordinator original-identity review required"}))?;
    Ok(json!({"receipt":receipt,"ownershipManifest":manifest,"sameOriginalWorkspace":true,"actualKernelCwdMatchedConfiguredRoot":true,
        "nativeFocusUnchanged":true,"inputSentToNewShell":false}))
}

async fn terminal_flow(phone:&mut Phone,config:&Config,socket:&Path,desktop:&mut Desktop,original:&ServerEvidence,initial:Snapshot,name:&str,evidence:&Path,run:&Uuid)->io::Result<bool> {
    let end=Instant::now()+Duration::from_secs(30);
    let (source,route)=loop {
        desktop.recheck()?;dev_continuity::zero_native_caps(&desktop.process)?;
        let client=Client::connect(socket).await?;
        if client.evidence!=*original{return Err(prerequisite("terminal-only original startup source changed"));}
        let (actual,native)=surface(socket).await?;let inventory=phone.host().await?;
        if actual!=*original{return Err(prerequisite("terminal-only original source observation changed"));}
        if let Some((route,incarnation))=terminal_select(&inventory,&native,name,None)? {
            let identity=(route.server_id.clone(),incarnation);
            creation_source_observation(&identity,original,&initial,&actual,&native,&inventory).map_err(prerequisite)?.then_some(()).ok_or_else(||prerequisite("terminal-only original source not current"))?;
            let source=AdmittedSource{identity,client,process:desktop.process.clone(),native:initial};
            write_json(&evidence.join(format!("terminal-binding-{}.json",run.0)),&json!({"runId":run,"serverId":source.identity.0,"serverIncarnation":source.identity.1,
                "shellTarget":route.target,"workspaceId":route.workspace_id,"tabId":route.tab_id,"paneId":route.pane_id,"terminalId":route.terminal_id,
                "source":original,"herdr":source.process,"scope":"actual Hello/current original source and logical shell; no provider eligibility dependency"}))?;
            break (source,route);
        }
        if Instant::now()>=end{return Err(prerequisite("terminal-only ordinary current shell admission unavailable"));}
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let mut reader_safe=false;let mut delayed_marker_observed=false;let mut input_echo_observed=false;
    let outcome:io::Result<Value>=async {
        let shell=terminal_fixture(phone,config,socket,desktop,&source,&route,evidence,run).await?;
        terminal_current(phone,&source,socket,desktop,&route).await?;shell.recheck()?;
        write_json(&evidence.join(format!("terminal-begin-consent-{}.json",run.0)),&json!({"runId":run,"consented":true,
            "explicitFlags":["consent-owned-providers","automate-native-inputs"],"disclosure":TERMINAL_DISCLOSURE,"beforeSubscribe":true,
            "shellTarget":route.target,"serverId":route.server_id,"source":source.client.evidence}))?;
        let mut witness=GridWitness::pending(Uuid::random(),GridSubject::Shell(route.target.clone()));
        witness.fixture=Some(TerminalFixtureWitness::new());witness.arm(TERMINAL_DELAYED)?;
        phone.grid_subscribe(witness).await?;
        let initial_commit=phone.grid.as_ref().ok_or_else(||grid_failure("terminal_initial_commit"))?.terminal_opening_commit()?;
        phone.grid_wait(GridGoal::TerminalFixture{after:initial_commit}).await?;
        delayed_marker_observed=true;
        let witness=phone.grid.as_ref().ok_or_else(||grid_failure("terminal_fixture_viewer"))?;
        let facts=witness.fixture.as_ref().and_then(|fixture|fixture.committed).ok_or_else(||grid_failure("terminal_fixture_facts"))?;
        let before=witness.committed.ok_or_else(||grid_failure("terminal_fixture_commit"))?;
        let crop=witness.committed_dimensions.ok_or_else(||grid_failure("terminal_fixture_dimensions"))?;
        let subscription=witness.subscription.clone().ok_or_else(||grid_failure("terminal_fixture_subscription"))?;
        let native=terminal_current(phone,&source,socket,desktop,&route).await?;shell.recheck()?;
        let seed=native.layouts.iter().find(|layout|layout.workspace_id==route.workspace_id&&layout.tab_id==route.tab_id)
            .ok_or_else(||prerequisite("terminal-only current native layout metadata absent"))?;
        let (columns,rows)=if (seed.area.width,seed.area.height)==(96,32){(112,36)}else{(96,32)};
        let action=Uuid::random();let boot=phone.hello.boot_id.clone();
        write_json(&evidence.join(format!("terminal-resize-consent-{}.json",run.0)),&json!({"runId":run,"actionId":action,"consented":true,
            "disclosure":TERMINAL_DISCLOSURE,"beforeResize":true,"columns":columns,"rows":rows,"dimensionMeaning":"native frontend surface; not selected inner_rect crop"}))?;
        let intent=evidence.join(format!("terminal-resize-{}-{}.json",run.0,action.0));
        write_json(&intent,&json!({"runId":run,"actionId":action,"bootId":boot,"method":"shell.resize","shellTarget":route.target,"subscription":subscription,
            "columns":columns,"rows":rows,"state":"dispatching","replayAllowed":false}))?;
        let result=phone.request_at(action.clone(),RequestBody::ShellResize(ShellResizeParams{boot_id:boot.clone(),target:route.target.clone(),subscription:subscription.clone(),columns:Uint(columns),rows:Uint(rows)}),"terminal_resize").await;
        let receipt=match result {
            Ok(Ok(ResultBody::ShellResize(result)))=>{
                if matches!(result.receipt.state,ReceiptState::Reserved|ReceiptState::Dispatching){
                    match phone.settled_by(&action,Instant::now()+Duration::from_secs(125)).await {
                        Ok(receipt)=>receipt,
                        Err(error)=>{write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"state":"uncertain","replayAllowed":false}))?;return Err(error);}
                    }
                }else{result.receipt}
            }
            _=>{write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"state":"uncertain","replayAllowed":false}))?;return Err(grid_failure("terminal_resize_entered_unknown"));}
        };
        write_native_outcome(&intent,&json!({"runId":run,"actionId":action,"receipt":receipt,"replayAllowed":false}))?;
        terminal_resize_receipt(&receipt,&action,&boot,&route.target,columns,rows)?;
        phone.grid_wait(GridGoal::Resized{after:before,dimensions:crop}).await?;
        let resized=phone.grid.as_ref().ok_or_else(||grid_failure("terminal_resize_viewer"))?;
        if resized.subscription.as_ref()!=Some(&subscription)||resized.subject!=GridSubject::Shell(route.target.clone()){return Err(grid_failure("terminal_resize_original_viewer"));}
        let observed=resized.committed_dimensions.ok_or_else(||grid_failure("terminal_resize_commit"))?;
        if observed.0>columns||observed.1>rows{return Err(grid_failure("terminal_resize_crop_bounds"));}
        terminal_current(phone,&source,socket,desktop,&route).await?;shell.recheck()?;reader_safe=true;
        let marker=format!("GOATRSTOCKRPC{}",Uuid::random().0.replace('-',""));
        phone.grid.as_mut().ok_or_else(||grid_failure("terminal_input_viewer"))?.arm(&marker)?;
        let text=terminal_goatr_input(phone,&route,&subscription,ShellInput::Text{text:marker},evidence,run,&mut reader_safe).await?;
        reader_safe=false;phone.grid_wait(GridGoal::Token).await?;input_echo_observed=true;
        terminal_current(phone,&source,socket,desktop,&route).await?;shell.recheck()?;reader_safe=true;
        let modifiers=ShellKeyModifiers{shift:false,ctrl:false,alt:false,super_key:false};
        let keys=terminal_goatr_input(phone,&route,&subscription,ShellInput::Keys{events:vec![
            ShellKeyEvent{key:ShellKey::Left,modifiers:modifiers.clone(),phase:ShellKeyPhase::Press,physical_key_id:None},
            ShellKeyEvent{key:ShellKey::Right,modifiers,phase:ShellKeyPhase::Press,physical_key_id:None},
        ]},evidence,run,&mut reader_safe).await?;
        Ok(json!({"gate":"terminal-only","runId":run,"status":"pass","scope":"single-controller stock cells/resize/Goatr input/public release/configured-project creation",
            "shellTarget":route.target,"serverId":source.identity.0,"serverIncarnation":source.identity.1,
            "fixture":facts.evidence(),"firstCommit":initial_commit,"openingDelayedMarkerAbsent":true,"fixtureCommit":before,"frontendDimensions":{"columns":columns,"rows":rows},
            "selectedCropBefore":{"columns":crop.0,"rows":crop.1},"selectedCropAfter":{"columns":observed.0,"rows":observed.1},
            "authoritativeResizeReceipt":true,"publicReplacementCommit":true,"nativeFixtureInputs":1,
            "goatrInputActions":[{"receipt":text},{"receipt":keys}],"publicCommittedTextObserved":input_echo_observed,
            "inputMeaning":"full native socket frames and staged public text echo only; no Enter sent; no execution, PTY acceptance or hardware/IME claim",
            "nativeTag19":"unproved","multiFrontendGeometry":"unproved","allWorkspacePtyGeometry":"unproved",
            "sourceLossAndRevoke":"unproved","physicalPhone":"unproved","phase4InputImeHardware":"unproved",
            "defaultOrRawLogsRead":false,"providerSessionsCreated":0}))
    }.await;
    let observation_saved=write_json(&evidence.join(format!("terminal-observation-{}.json",run.0)),&json!({"runId":run,
        "openingAccepted":phone.grid.as_ref().is_some_and(|grid|grid.subscription.is_some()),
        "committedRevision":phone.grid.as_ref().and_then(|grid|grid.committed),
        "openingCommit":phone.grid.as_ref().and_then(|grid|grid.fixture.as_ref()).and_then(|fixture|fixture.opening).map(|(revision,_)|revision),
        "openingDelayedMarkerPresent":phone.grid.as_ref().and_then(|grid|grid.fixture.as_ref()).and_then(|fixture|fixture.opening).map(|(_,present)|present),
        "selectedCrop":phone.grid.as_ref().and_then(|grid|grid.committed_dimensions),
        "delayedMarkerCommitted":delayed_marker_observed,"goatrInputEchoObserved":input_echo_observed,
        "streamRetired":phone.grid.as_ref().is_some_and(|grid|grid.retired),
        "committedFixtureFacts":phone.grid.as_ref().and_then(|grid|grid.fixture.as_ref()).and_then(|fixture|fixture.committed).map(TerminalFixtureFacts::evidence),
        "nativeTag19":"unproved","contentRetained":false}));
    let had_subscription=phone.grid.as_ref().and_then(|grid|grid.subscription.as_ref()).is_some();
    let release_requested=had_subscription&&reader_safe;
    let release=if release_requested {phone.grid_unsubscribe().await}else{
        if had_subscription{phone.connection.close(0u32.into(),b"original terminal viewer retired");}Ok(())
    };
    write_json(&evidence.join(format!("terminal-release-{}.json",run.0)),&json!({"runId":run,"protocolUnsubscribeRequested":release_requested,"protocolUnsubscribeConfirmed":release_requested&&release.is_ok(),
        "originalReaderTrustedForCleanup":reader_safe,
        "nativeDeactivationAck":"not_provided","nativeResources":"retained; no pane close/signals/stop",
        "scope":"temporary local client viewer release only; no claim of a native deactivation or size-restoration acknowledgment"}))?;
    let mut record=outcome?;observation_saved?;release?;
    record["terminalCreate"]=terminal_goatr_create(phone,config,socket,desktop,&source,&route,evidence,run).await?;
    write_json(&evidence.join(format!("terminal-result-{}.json",run.0)),&record)?;println!("{record}");Ok(true)
}
async fn checkpoint(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    checkpoint_held(args, evidence, None, None).await
}
// This fixture input is public and independently crypto-qualified by the coordinator.
// The helper checks bytes/objects only; it never signs, unlocks or reads a source repository.
const PROJECT_PUBLIC_COMMIT:&str="a55cd979cab6a0a47d4c757ac97b97d8535ee9eb";
const PROJECT_PUBLIC_TREE:&str="fb2fbdd55eed2884f345b23ab25bdfa0f6ce31f8";
const PROJECT_PACK_LIMIT:usize=16*1024*1024;
fn project_hash(text:&str)->bool {text.len()==64&&text.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte))}
fn project_stamp(metadata:&fs::Metadata)->(u64,u64,u64,i64,i64,i64,i64,u32,u32) {
    (metadata.dev(),metadata.ino(),metadata.len(),metadata.mtime(),metadata.mtime_nsec(),metadata.ctime(),metadata.ctime_nsec(),metadata.mode(),metadata.uid())
}
fn project_input(path:&Path,maximum:usize,pack:bool)->io::Result<Vec<u8>> {
    let before=fs::symlink_metadata(path)?;
    if !path.is_absolute()||fs::canonicalize(path)?!=path||!before.is_file()||before.file_type().is_symlink()
        ||![0,uid()].contains(&before.uid())||before.mode()&0o222!=0||before.len()>maximum as u64
        ||pack&&(before.nlink()!=1||before.mode()&0o7777!=0o400){return Err(prerequisite("owned project-read immutable input identity unavailable"));}
    if pack {private_dir(path.parent().ok_or_else(||failure("public seed parent absent"))?,false)?;}
    let mut file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(path)?;
    if project_stamp(&file.metadata()?)!=project_stamp(&before){return Err(prerequisite("owned project-read immutable input replaced"));}
    let mut bytes=vec![0u8;before.len() as usize];file.read_exact(&mut bytes)?;let mut extra=[0u8;1];
    if file.read(&mut extra)?!=0||project_stamp(&file.metadata()?)!=project_stamp(&before)||project_stamp(&fs::symlink_metadata(path)?)!=project_stamp(&before){return Err(prerequisite("owned project-read immutable input changed"));}
    Ok(bytes)
}
fn project_file_hash(path:&Path)->io::Result<String> {
    use sha2::{Digest,Sha256};
    let before=fs::symlink_metadata(path)?;
    if !before.is_file()||before.file_type().is_symlink()||before.len()>128*1024*1024||before.mode()&0o222!=0||![0,uid()].contains(&before.uid()){return Err(prerequisite("immutable selected Git image unavailable"));}
    let mut file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path)?;let mut hash=Sha256::new();let mut buffer=[0u8;65536];
    loop {let count=file.read(&mut buffer)?;if count==0{break;}hash.update(&buffer[..count]);}
    if project_stamp(&file.metadata()?)!=project_stamp(&before)||project_stamp(&fs::symlink_metadata(path)?)!=project_stamp(&before){return Err(prerequisite("immutable selected Git image changed"));}Ok(hex(&hash.finalize()))
}
struct ProjectFixtureGit {executable:PathBuf,exec_path:PathBuf,manifest_sha:String,image_sha:String,stamp:(u64,u64,u64,i64,i64,i64,i64,u32,u32)}
impl ProjectFixtureGit {
    fn qualify(binary:&Path)->io::Result<Self> {
        let binary=fs::canonicalize(binary)?;
        if !binary.starts_with("/nix/store"){return Err(prerequisite("project-read-only requires the selected immutable packaged companion manifest"));}
        let path=binary.parent().and_then(Path::parent).ok_or_else(||failure("selected package root absent"))?.join("share/goatr/manifest.json");
        let bytes=project_input(&path,1024*1024,false)?;let manifest:Value=serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let compiled=option_env!("GOATR_READ_GIT_MANIFEST").ok_or_else(||prerequisite("project-read-only helper requires the same locked Nix Git manifest"))?;
        let selected:Value=serde_json::from_slice(&project_input(Path::new(compiled),1024*1024,false)?).map_err(io::Error::other)?;
        if manifest["kind"]!="direct-rust"||manifest["lockSha256"]!=transport::lock_sha256()||manifest["readGit"]!=selected["readGit"]||manifest["readGit"]["available"]!=true{return Err(prerequisite("selected companion/helper immutable Git identity join unavailable"));}
        let git=&manifest["readGit"];
        let executable=PathBuf::from(git["executable"].as_str().ok_or_else(||failure("selected Git image absent"))?);
        let exec_path=PathBuf::from(git["execPath"].as_str().ok_or_else(||failure("selected Git exec root absent"))?);
        let image_sha=git["executableSha256"].as_str().filter(|value|project_hash(value)).ok_or_else(||failure("selected Git digest absent"))?.to_owned();
        if !executable.starts_with("/nix/store")||fs::canonicalize(&executable)?!=executable||project_file_hash(&executable)?!=image_sha{return Err(prerequisite("selected fixture Git is not the exact manifest image"));}
        let mut image=File::open(&executable)?;let mut magic=[0u8;4];image.read_exact(&mut magic)?;if &magic!=b"\x7fELF"{return Err(prerequisite("selected fixture Git must be a direct ELF, never a wrapper or PATH fallback"));}
        let stamp=project_stamp(&fs::symlink_metadata(&executable)?);
        Ok(Self {executable,exec_path,manifest_sha:hex(&sha256(&bytes)),image_sha,stamp})
    }
    async fn run(&self,fixture:&ProjectReadFixture,cwd:&Path,args:&[&str],input:Option<&[u8]>,deadline:tokio::time::Instant,limit:usize)->io::Result<Vec<u8>> {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};use std::os::unix::process::CommandExt as _;
        fixture.current()?;
        if !cwd.starts_with(&fixture.read)||fs::canonicalize(cwd)?!=cwd||project_stamp(&fs::symlink_metadata(&self.executable)?)!=self.stamp{return Err(prerequisite("owned fixture cwd or immutable Git image changed"));}
        let directory=OpenOptions::new().read(true).custom_flags(libc::O_PATH|libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(cwd)?;
        let raw=directory.as_raw_fd();let mut command=tokio::process::Command::new(&self.executable);
        command.args(["--no-pager","-c","core.fsmonitor=false","-c","core.untrackedCache=false","-c","core.preloadIndex=false","-c","gc.auto=0","-c","maintenance.auto=false","-c","pack.threads=1","-c","protocol.allow=never"])
            .arg("-c").arg(format!("core.hooksPath={}",fixture.hooks.display())).args(args)
            .env_clear().env("HOME",&fixture.home).env("XDG_CONFIG_HOME",&fixture.home).env("LC_ALL","C")
            .env("GIT_CONFIG_GLOBAL",&fixture.global).env("GIT_CONFIG_NOSYSTEM","1").env("GIT_ATTR_NOSYSTEM","1")
            .env("GIT_OPTIONAL_LOCKS","0").env("GIT_NO_LAZY_FETCH","1").env("GIT_TERMINAL_PROMPT","0").env("GIT_EXEC_PATH",&self.exec_path)
            .stdin(if input.is_some(){Stdio::piped()}else{Stdio::null()}).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        command.as_std_mut().process_group(0);
        unsafe{command.pre_exec(move||{goatr::codex_relay::drop_inherited_capabilities()?;if libc::fchdir(raw)!=0{return Err(io::Error::last_os_error());}Ok(())});}
        let mut child=command.spawn()?;let stdout=child.stdout.take().ok_or_else(||failure("owned Git output pipe absent"))?;
        let mut stdin=child.stdin.take();
        let transfer=async {
            let write=async {if let (Some(mut stdin),Some(input))=(stdin.take(),input){stdin.write_all(input).await?;stdin.shutdown().await?;drop(stdin);}Ok::<(),io::Error>(())};
            let read=async {let mut bytes=Vec::new();stdout.take(limit as u64+1).read_to_end(&mut bytes).await?;if bytes.len()>limit{return Err(failure("owned fixture Git output exceeded its bound"));}Ok::<Vec<u8>,io::Error>(bytes)};
            let (_,bytes,status)=tokio::try_join!(write,read,child.wait())?;
            if !status.success(){return Err(prerequisite("owned public seed or fixture Git command refused; no fallback"));}Ok(bytes)
        };
        let bytes=tokio::time::timeout_at(deadline,transfer).await.map_err(|_|prerequisite("owned fixture Git aggregate preparation deadline"))??;
        fixture.current()?;Ok(bytes)
    }
}
struct ProjectSeedPin {pack:PathBuf,pack_sha:String,commit:String,tree:String,signature_sha:String}
impl ProjectSeedPin {
    fn parse(args:&clap::ArgMatches)->io::Result<Option<Self>> {
        let Some(pack)=args.get_one::<PathBuf>("project-read-seed-pack")else{return Ok(None)};
        let get=|key|args.get_one::<String>(key).cloned().ok_or_else(||prerequisite("complete prequalified public signed seed pins required"));
        let pin=Self {pack:pack.clone(),pack_sha:get("project-read-seed-pack-sha256")?,commit:get("project-read-seed-commit")?,tree:get("project-read-seed-tree")?,signature_sha:get("project-read-seed-signature-sha256")?};
        if !project_hash(&pin.pack_sha)||!project_hash(&pin.signature_sha)||pin.commit!=PROJECT_PUBLIC_COMMIT||pin.tree!=PROJECT_PUBLIC_TREE{return Err(prerequisite("fixture seed must be the explicitly qualified existing public Goatr commit/tree"));}Ok(Some(pin))
    }
}
struct ProjectReadFixture {read:PathBuf,creation_only:PathBuf,global:PathBuf,home:PathBuf,hooks:PathBuf,folder:String,binary:PathBuf,pin:Option<ProjectSeedPin>,root_object:(u64,u64),creation_object:(u64,u64)}
struct ProjectReadFacts {renamed:String,original:String,conflict:String,binary:String,submodule:String,linked:PathBuf}
impl ProjectReadFixture {
    fn prepare(args:&clap::ArgMatches,root:&Path,binary:&Path,run:&Uuid)->io::Result<Self> {
        let read=root.join("project");let creation_only=root.join("creation-only");fs::DirBuilder::new().mode(0o700).create(&creation_only)?;
        let home=read.join(format!(".goatr-fixture-home-{}",&run.0[..8]));let hooks=home.join("hooks");
        fs::DirBuilder::new().mode(0o700).create(&home)?;fs::DirBuilder::new().mode(0o700).create(&hooks)?;
        let global=home.join("global.config");OpenOptions::new().write(true).create_new(true).mode(0o600).open(&global)?.sync_all()?;
        let a=fs::symlink_metadata(&read)?;let b=fs::symlink_metadata(&creation_only)?;
        Ok(Self {read,creation_only,global,home,hooks,folder:format!("goatr-owned-review-{}",&run.0[..8]),binary:binary.to_owned(),pin:ProjectSeedPin::parse(args)?,root_object:(a.dev(),a.ino()),creation_object:(b.dev(),b.ino())})
    }
    fn current(&self)->io::Result<()> {
        for (path,object) in [(&self.read,self.root_object),(&self.creation_only,self.creation_object)] {
            private_dir(path,false)?;let metadata=fs::symlink_metadata(path)?;
            if fs::canonicalize(path)?!=*path||(metadata.dev(),metadata.ino())!=object{return Err(prerequisite("owned project fixture root replaced"));}
        }Ok(())
    }
    fn environment(&self,command:&mut std::process::Command) {command.env("GIT_CONFIG_GLOBAL",&self.global).env("GIT_CONFIG_NOSYSTEM","1").env("GIT_ATTR_NOSYSTEM","1");}
    async fn seed(&self,git:&ProjectFixtureGit,cwd:&Path,pack:&[u8],pin:&ProjectSeedPin,deadline:tokio::time::Instant)->io::Result<()> {
        git.run(self,cwd,&["init","--quiet","--object-format=sha1"],None,deadline,4096).await?;
        git.run(self,cwd,&["unpack-objects","-q"],Some(pack),deadline,4096).await?;
        git.run(self,cwd,&["fsck","--strict","--no-reflogs","--no-dangling",&pin.commit],None,deadline,65536).await?;
        git.run(self,cwd,&["update-ref","refs/heads/goatr-fixture",&pin.commit],None,deadline,4096).await?;
        git.run(self,cwd,&["symbolic-ref","HEAD","refs/heads/goatr-fixture"],None,deadline,4096).await?;
        git.run(self,cwd,&["read-tree","--reset","-u",&pin.commit],None,deadline,4096).await?;Ok(())
    }
    async fn build(&self,evidence:&Path,run:&Uuid)->io::Result<ProjectReadFacts> {
        self.current()?;let pin=self.pin.as_ref().ok_or_else(||prerequisite("project-read-only requires the independently crypto-qualified existing public signed object pack and complete byte/object pins; no provider created"))?;
        let git=ProjectFixtureGit::qualify(&self.binary)?;
        let pack=project_input(&pin.pack,PROJECT_PACK_LIMIT,true)?;
        if hex(&sha256(&pack))!=pin.pack_sha||pack.len()<12||&pack[..4]!=b"PACK"{return Err(prerequisite("qualified complete public seed pack byte pin mismatch"));}
        let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
        self.seed(&git,&self.read,&pack,pin,deadline).await?;
        let commit=git.run(self,&self.read,&["cat-file","commit",&pin.commit],None,deadline,128*1024).await?;
        let (tree,signature)=project_commit_fields(&commit)?;
        if tree!=pin.tree||hex(&sha256(signature.as_bytes()))!=pin.signature_sha{return Err(prerequisite("existing public commit tree or signature-block bytes mismatch"));}
        let rows=git.run(self,&self.read,&["ls-tree","-r","-z",&pin.tree],None,deadline,256*1024).await?;
        let files=project_seed_files(&rows)?;
        if files.len()<3{return Err(prerequisite("qualified existing public seed needs three distinct regular blob fixtures"));}
        let folder=self.read.join(&self.folder);fs::DirBuilder::new().mode(0o700).create(&folder)?;
        let pages=folder.join("pages");fs::DirBuilder::new().mode(0o700).create(&pages)?;
        for index in 0..105 {fs::write(pages.join(format!("row-{index:03}")),b"public bounded page\n")?;}
        fs::write(folder.join("complete.txt"),vec![b'x';65536+7])?;fs::write(folder.join("binary.bin"),b"binary\0first")?;
        fs::write(folder.join("empty.txt"),b"")?;File::create(folder.join("large.bin"))?.set_len(1024*1024+1)?;
        let renamed=format!("{}.goatr-renamed",files[0].0);fs::rename(self.read.join(&files[0].0),self.read.join(&renamed))?;
        let binary=format!("{}/binary.bin",self.folder);
        git.run(self,&self.read,&["add","--force","--",&files[0].0,&renamed,&binary],None,deadline,4096).await?;
        fs::write(self.read.join(&binary),b"binary\0second")?;
        OpenOptions::new().append(true).open(self.read.join(&files[1].0))?.write_all(b"\nGOATR owned unstaged reference fixture\n")?;
        let zero="0".repeat(pin.commit.len());let conflict=files[2].0.clone();
        let index=format!("0 {zero}\t{conflict}\n100644 {} 1\t{conflict}\n100644 {} 2\t{conflict}\n100644 {} 3\t{conflict}\n",files[2].1,files[0].1,files[1].1);
        git.run(self,&self.read,&["update-index","--index-info"],Some(index.as_bytes()),deadline,4096).await?;
        let nested=folder.join("submodule");fs::DirBuilder::new().mode(0o700).create(&nested)?;self.seed(&git,&nested,&pack,pin,deadline).await?;
        fs::write(nested.join("GOATR_UNTRACKED_NATIVE"),b"owned submodule dirtiness\n")?;
        let submodule=format!("{}/submodule",self.folder);let cache=format!("160000,{},{}",pin.commit,submodule);
        git.run(self,&self.read,&["update-index","--add","--cacheinfo",&cache],None,deadline,4096).await?;
        let linked=folder.join("linked-worktree");let linked_text=linked.to_str().ok_or_else(||failure("owned linked fixture path unrepresentable"))?;
        git.run(self,&self.read,&["worktree","add","--detach","--no-checkout",linked_text,&pin.commit],None,deadline,4096).await?;
        fs::write(self.creation_only.join("outside.config"),b"[core]\nquotePath=false\n")?;
        write_json(&evidence.join(format!("project-read-seed-{}.json",run.0)),&json!({"runId":run,"packSha256":pin.pack_sha,"existingCommit":pin.commit,"existingTree":pin.tree,"signatureBlockSha256":pin.signature_sha,
            "byteObjectPinsChecked":true,"cryptoVerification":"separate coordinator-qualified existing public evidence; not performed or fabricated by helper","newCommitsOrTags":false,"sourceRepositoryOrCredentialsRead":false,
            "fixtureGitManifestSha256":git.manifest_sha,"fixtureGitImageSha256":git.image_sha,"fixtureRoot":self.read,"creationOnlySibling":self.creation_only,"scope":"owned blob/index/working-tree/gitlink/worktree setup only; no phone producer mutations; retained, no automatic cleanup"}))?;
        Ok(ProjectReadFacts {renamed,original:files[0].0.clone(),conflict,binary,submodule,linked})
    }
}
fn project_commit_fields(bytes:&[u8])->io::Result<(String,String)> {
    let text=std::str::from_utf8(bytes).map_err(io::Error::other)?;let header=text.split_once("\n\n").ok_or_else(||failure("complete existing commit header absent"))?.0;
    let tree=header.lines().next().and_then(|line|line.strip_prefix("tree ")).ok_or_else(||failure("existing commit tree absent"))?.to_owned();
    let mut signature=String::new();let mut capturing=false;
    for line in header.lines().skip(1) {
        if let Some(first)=line.strip_prefix("gpgsig "){if capturing||!signature.is_empty(){return Err(failure("ambiguous existing signature block"));}signature.push_str(first);capturing=true;}
        else if capturing {if let Some(next)=line.strip_prefix(' '){signature.push('\n');signature.push_str(next);}else{capturing=false;}}
    }
    if !signature.starts_with("-----BEGIN PGP SIGNATURE-----\n")||!signature.ends_with("-----END PGP SIGNATURE-----"){return Err(prerequisite("qualified existing PGP signature block unavailable"));}
    Ok((tree,signature))
}
fn project_seed_files(bytes:&[u8])->io::Result<Vec<(String,String)>> {
    if bytes.len()>256*1024||bytes.last()!=Some(&0){return Err(failure("bounded complete public seed tree absent"));}
    let text=std::str::from_utf8(bytes).map_err(io::Error::other)?;let mut files=Vec::new();let mut blobs=HashSet::new();
    for row in text.split_terminator('\0') {
        let (metadata,path)=row.split_once('\t').ok_or_else(||failure("public seed row invalid"))?;let mut fields=metadata.split(' ');
        let mode=fields.next().ok_or_else(||failure("public seed mode absent"))?;let kind=fields.next().ok_or_else(||failure("public seed type absent"))?;let oid=fields.next().ok_or_else(||failure("public seed object absent"))?;
        if fields.next().is_some()||path.is_empty()||path.len()>4096||Path::new(path).is_absolute()||path.chars().any(char::is_control)||Path::new(path).components().any(|part|!matches!(part,std::path::Component::Normal(_)))
            ||!matches!(mode,"100644"|"100755")||kind!="blob"||oid.len()!=40||!oid.bytes().all(|byte|byte.is_ascii_hexdigit()){return Err(prerequisite("public seed requires bounded ordinary regular tree objects; no unsafe alias or submodule adoption"));}
        if files.len()<3&&mode=="100644"&&blobs.insert(oid){files.push((path.to_owned(),oid.to_owned()));}
    }Ok(files)
}
struct ProjectReadRun<'a> {fixture:&'a ProjectReadFixture,config:&'a Config,socket:&'a Path,desktop:&'a Desktop,source:&'a AdmittedSource,evidence:&'a Path,run:&'a Uuid}
async fn project_create(phone:&mut Phone,context:&ProjectReadRun<'_>,provider:Provider,project:&ConfiguredProject)->io::Result<NativeProof> {
    context.fixture.current()?;context.source.recheck()?;dev_continuity::zero_native_caps(&context.desktop.process)?;
    let action=Uuid::random();let params=SessionCreateParams {boot_id:phone.hello.boot_id.clone(),server_id:context.source.identity.0.clone(),project_id:project.id.clone(),provider,
        name:Label::new(format!("owned-read-{provider:?}-{}",&action.0[..8])).map_err(io::Error::other)?,directory_ref:None,placement:None};
    write_json(&context.evidence.join(format!("project-read-create-intent-{}-{}.json",context.run.0,action.0)),&json!({"runId":context.run,"actionId":action,"provider":provider,"projectId":project.id,"readGrant":project.allow_read_tools,"scope":"one first native creation; entered results status-only; never mutation replay or automatic close"}))?;
    let initial=receipt_result(phone.request_at(action.clone(),RequestBody::SessionCreate(params),"project_read_create").await?.map_err(|error|failure(&format!("project-read creation refused: {:?}",error.code)))?)?;
    let receipt=if matches!(initial.state,ReceiptState::Reserved|ReceiptState::Dispatching){phone.creation_settled(&action,provider).await?}else{initial};
    write_json(&context.evidence.join(format!("project-read-create-receipt-{}-{}.json",context.run.0,action.0)),&json!({"runId":context.run,"receipt":receipt,"resourcesRetained":true}))?;
    if receipt.state!=ReceiptState::Succeeded{return Err(non_succeeded_creation_error(provider,&receipt));}
    context.source.recheck()?;let inventory=phone.host().await?;
    let session=inventory.sessions.iter().find(|session|receipt.creation.as_ref().is_some_and(|creation|created_session_matches(&creation.resources,session,provider))).ok_or_else(||prerequisite("exact owned read original is not NativeReady"))?;
    let original=proof(context.config,context.socket,session,None).await?;original.recheck_processes()?;
    write_json(&context.evidence.join(format!("project-read-original-{}-{}.json",context.run.0,action.0)),&json!({"runId":context.run,"proof":original.evidence(),"creationAction":action,"scope":"original process/session or shared owner/thread only; no prompt, PTY or replay"}))?;
    Ok(original)
}
async fn project_refused(phone:&mut Phone,body:RequestBody)->io::Result<()> {
    match phone.request_at(Uuid::random(),body,"project_read_refusal").await? {
        Err(error)if matches!(error.code,ErrorCode::NotReady|ErrorCode::StaleSession|ErrorCode::CursorExpired|ErrorCode::NotFound)=>Ok(()),
        _=>Err(failure("owned project negative did not refuse at its actual supported boundary")),
    }
}
async fn project_list(phone:&mut Phone,source:&ArtifactSource,parent:Option<Uuid>)->io::Result<Vec<ArtifactEntry>> {
    let mut cursor=None;let mut rows=Vec::new();let mut seen=HashSet::new();
    loop {
        let body=RequestBody::ArtifactsList(ArtifactsListParams {source:source.clone(),parent:if cursor.is_none(){parent.clone()}else{None},cursor:cursor.clone(),limit:Uint(100)});
        let ResultBody::ArtifactsList(page)=phone.rpc(body,"project_read_list").await?else{return Err(failure("actual artifact page result absent"))};
        if &page.source!=source{return Err(failure("artifact original source changed"));}
        for row in page.entries {if rows.len()==1000||!seen.insert(uuid::Uuid::parse_str(&row.reference.0).map_err(io::Error::other)?){return Err(failure("artifact complete listing exceeded or repeated its bound"));}rows.push(row);}
        match page.next_cursor {None=>return Ok(rows),Some(next)if Some(&next)!=cursor.as_ref()=>cursor=Some(next),_=>return Err(failure("artifact cursor made no progress"))}
    }
}
async fn project_open(phone:&mut Phone,source:&ArtifactSource,row:&ArtifactEntry)->io::Result<ArtifactsOpenResult> {
    let ResultBody::ArtifactsOpen(open)=phone.rpc(RequestBody::ArtifactsOpen(ArtifactsOpenParams {source:source.clone(),file:row.reference.clone()}),"project_read_open").await?else{return Err(failure("actual artifact snapshot absent"))};
    if &open.source!=source||open.name!=row.name||Some(open.size)!=row.size{return Err(failure("artifact snapshot differs from the selected immutable row"));}Ok(open)
}
async fn project_close(phone:&mut Phone,source:&ArtifactSource,read:&Uuid)->io::Result<()> {
    let ResultBody::ArtifactsClose(closed)=phone.rpc(RequestBody::ArtifactsClose(ArtifactsCloseParams {source:source.clone(),read_id:read.clone()}),"project_read_close").await?else{return Err(failure("artifact memory close result absent"))};
    if closed.source!=*source||closed.read_id!=*read||!closed.closed{return Err(failure("selected artifact read owner was not released"));}Ok(())
}
async fn project_artifacts(phone:&mut Phone,source:&ArtifactSource,fixture:&ProjectReadFixture)->io::Result<()> {
    use base64::Engine as _;use sha2::{Digest,Sha256};
    let root=project_list(phone,source,None).await?;let folder=root.iter().find(|row|row.name==fixture.folder&&row.kind==ArtifactKind::Directory).ok_or_else(||failure("owned bounded artifact directory absent"))?;
    let rows=project_list(phone,source,Some(folder.reference.clone())).await?;
    let pages=rows.iter().find(|row|row.name=="pages"&&row.kind==ArtifactKind::Directory).ok_or_else(||failure("owned paginated artifact directory absent"))?;
    if project_list(phone,source,Some(pages.reference.clone())).await?.len()!=105{return Err(failure("real native artifact cursor did not return the complete two-page directory"));}
    for (name,kind) in [("complete.txt",ArtifactPreviewKind::Text),("empty.txt",ArtifactPreviewKind::Text),("binary.bin",ArtifactPreviewKind::Binary),("large.bin",ArtifactPreviewKind::Unavailable)] {
        let row=rows.iter().find(|row|row.name==name&&row.kind==ArtifactKind::File).ok_or_else(||failure("owned artifact fixture row absent"))?;
        let open=project_open(phone,source,row).await?;if open.preview_kind!=kind{return Err(failure("text, binary and unavailable complete snapshots were conflated"));}
        let mut offset=0u64;let mut hash=Sha256::new();
        loop {
            let ResultBody::ArtifactsRead(chunk)=phone.rpc(RequestBody::ArtifactsRead(ArtifactsReadParams {source:source.clone(),read_id:open.read_id.clone(),offset:Uint(offset),limit:Uint(65536)}),"project_read_chunk").await?else{return Err(failure("artifact chunk absent"))};
            let mut bytes=base64::engine::general_purpose::STANDARD.decode(&chunk.data.0).map_err(io::Error::other)?;
            if chunk.source!=*source||chunk.read_id!=open.read_id||chunk.offset.0!=offset||bytes.len()>65536||offset+bytes.len() as u64>open.size.0||bytes.is_empty()&&!chunk.eof{return Err(failure("artifact exact chunk offset/size ownership mismatch"));}
            offset+=bytes.len() as u64;hash.update(&bytes);bytes.fill(0);if chunk.eof{break;}
        }
        let mut local=File::open(fixture.read.join(&fixture.folder).join(name))?;let mut local_hash=Sha256::new();let mut buffer=[0u8;65536];let size=local.metadata()?.len();
        loop {let count=local.read(&mut buffer)?;if count==0{break;}local_hash.update(&buffer[..count]);}
        if offset!=open.size.0||size!=offset||hex(&hash.finalize())!=open.sha256||hex(&local_hash.finalize())!=open.sha256{return Err(failure("complete artifact host export-equivalent size/digest mismatch"));}
        project_close(phone,source,&open.read_id).await?;
    }Ok(())
}
fn project_changes_facts(result:&ChangesReadResult,original:&NativeProof,facts:&ProjectReadFacts,pin:&ProjectSeedPin)->io::Result<()> {
    if result.target!=original.target||result.changes.repository_context.as_ref().and_then(|context|context.latest_commit.as_ref()).map(|commit|&commit.oid)!=Some(&pin.commit)
        ||!result.changes.entries.iter().any(|entry|matches!(entry,ChangeEntry::Tracked {path,original_path:Some(old),index:ChangeState::Renamed,..} if path==&facts.renamed&&old==&facts.original))
        ||!result.changes.entries.iter().any(|entry|matches!(entry,ChangeEntry::Tracked {path,index:ChangeState::Unmerged,..} if path==&facts.conflict))
        ||!result.changes.entries.iter().any(|entry|matches!(entry,ChangeEntry::Tracked {path,staged_binary:true,unstaged_binary:true,..} if path==&facts.binary))
        ||!result.changes.entries.iter().any(|entry|matches!(entry,ChangeEntry::Tracked {path,submodule:Some(submodule),..} if path==&facts.submodule&&submodule.untracked_dirty))
        ||result.changes.staged_patch.is_empty()||result.changes.unstaged_patch.is_empty(){return Err(failure("actual native staged/unstaged/rename/binary/conflict/submodule/reference facts incomplete"));}
    Ok(())
}
async fn project_read_flow(mut phone:Phone,selected:&[Provider],context:ProjectReadRun<'_>)->io::Result<bool> {
    let mut completed=HashSet::new();let mut observations=Vec::new();let mut proved_read_originals=0;let mut proved_creation_originals=0;let mut pinned_files_panes=0;
    let outcome:io::Result<()>=async {
        if context.config.allow_terminal_control||context.config.projects.len()!=2||!context.config.projects[0].allow_read_tools||context.config.projects[1].allow_read_tools{return Err(failure("owned project read and creation-only grants were not independent"));}
        let facts=context.fixture.build(context.evidence,context.run).await?;let pin=context.fixture.pin.as_ref().ok_or_else(||failure("qualified public seed pin absent"))?;
        let inventory=phone.host().await?;let native=context.source.native.panes.first().filter(|_|context.source.native.panes.len()==1&&context.source.native.agents.is_empty()).ok_or_else(||prerequisite("one original startup Files pane required"))?;
        let shell=inventory.shells.iter().find(|shell|shell.server_id==context.source.identity.0&&shell.pane_id==native.pane_id&&shell.terminal_id==native.terminal_id&&shell.workspace_id==native.workspace_id&&shell.tab_id==native.tab_id).ok_or_else(||prerequisite("exact original startup Files source absent"))?;
        if !shell.capabilities.artifacts||shell.capabilities.input||shell.capabilities.grid||shell.capabilities.create{return Err(failure("Files source borrowed native terminal input/grid authority"));}
        let shell_source=ArtifactSource::Shell {target:shell.target.clone()};
        pinned_files_panes=1;
        let mut originals=Vec::new();
        for provider in selected.iter().copied() {originals.push(project_create(&mut phone,&context,provider,&context.config.projects[0]).await?);proved_read_originals+=1;}
        for original in &originals {
            original.recheck_processes()?;let source=ArtifactSource::Session {target:original.target.clone()};project_artifacts(&mut phone,&source,context.fixture).await?;
            let ResultBody::ChangesRead(changes)=phone.rpc(RequestBody::ChangesRead(ChangesReadParams {target:original.target.clone()}),"project_read_changes").await?else{return Err(failure("actual Changes response absent"))};
            project_changes_facts(&changes,original,&facts,pin)?;
            let ResultBody::WorktreesList(worktrees)=phone.rpc(RequestBody::WorktreesList(WorktreesListParams {target:original.target.clone()}),"project_read_worktrees").await?else{return Err(failure("actual worktree response absent"))};
            if worktrees.target!=original.target||!worktrees.worktrees.iter().any(|row|Path::new(&row.path)==context.fixture.read&&row.is_linked_worktree==Some(false))
                ||!worktrees.worktrees.iter().any(|row|Path::new(&row.path)==facts.linked&&row.is_linked_worktree==Some(true)){return Err(failure("complete native worktree/admin inventory facts absent"));}
            observations.push(json!({"provider":original.provider,"completeArtifacts":true,"hostExportEquivalentOnly":true,"stagedPatchSha256":hex(&sha256(changes.changes.staged_patch.as_bytes())),"unstagedPatchSha256":hex(&sha256(changes.changes.unstaged_patch.as_bytes())),"changesEntries":changes.changes.entries.len(),"worktrees":worktrees.worktrees.len(),"referenceFacts":"immutable native path/stage/binary/conflict/gitlink metadata; Android draft/UI separately required"}));completed.insert(original.provider);
        }
        project_artifacts(&mut phone,&shell_source,context.fixture).await?;
        let creation=project_create(&mut phone,&context,Provider::Omp,&context.config.projects[1]).await?;
        proved_creation_originals=1;
        project_refused(&mut phone,RequestBody::ArtifactsList(ArtifactsListParams {source:ArtifactSource::Session {target:creation.target.clone()},parent:None,cursor:None,limit:Uint(100)})).await?;
        // Existing mutable Git configuration is data, not a grant. An outside
        // include must refuse the COMPLETE observation for each original provider.
        let include=format!("[include]\npath = {}\n",context.fixture.creation_only.join("outside.config").display());fs::write(&context.fixture.global,include)?;
        for original in &originals {project_refused(&mut phone,RequestBody::ChangesRead(ChangesReadParams {target:original.target.clone()})).await?;}
        fs::write(&context.fixture.global,b"")?;
        let root=project_list(&mut phone,&shell_source,None).await?;let directory=root.iter().find(|row|row.name==context.fixture.folder).ok_or_else(||failure("owned artifact directory absent"))?.reference.clone();
        let folder=context.fixture.read.join(&context.fixture.folder);
        for special in ["outside-alias","special-fifo"] {
            let path=folder.join(special);
            if special=="outside-alias" {std::os::unix::fs::symlink(context.fixture.creation_only.join("outside.config"),&path)?;}else{
                let path=std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;if unsafe{libc::mkfifo(path.as_ptr(),0o600)}!=0{return Err(io::Error::last_os_error());}
            }
            project_refused(&mut phone,RequestBody::ArtifactsList(ArtifactsListParams {source:shell_source.clone(),parent:Some(directory.clone()),cursor:None,limit:Uint(100)})).await?;
            fs::remove_file(path)?;
        }
        let rows=project_list(&mut phone,&shell_source,Some(directory.clone())).await?;let old=rows.iter().find(|row|row.name=="complete.txt").ok_or_else(||failure("owned changed-file row absent"))?;
        fs::rename(folder.join("complete.txt"),folder.join("retired-complete.txt"))?;fs::write(folder.join("complete.txt"),vec![b'y';65536+7])?;
        project_refused(&mut phone,RequestBody::ArtifactsOpen(ArtifactsOpenParams {source:shell_source.clone(),file:old.reference.clone()})).await?;
        let rows=project_list(&mut phone,&shell_source,Some(directory)).await?;let row=rows.iter().find(|row|row.name=="complete.txt").ok_or_else(||failure("replacement snapshot row absent"))?;
        let open=project_open(&mut phone,&shell_source,row).await?;
        // The already-open snapshot remains charged, but no data may be published
        // after its original approved root is retired. Never stop originals or restore it.
        let retired=context.fixture.read.parent().ok_or_else(||failure("owned root parent absent"))?.join("retired-read-root");fs::rename(&context.fixture.read,&retired)?;fs::DirBuilder::new().mode(0o700).create(&context.fixture.read)?;
        write_json(&context.evidence.join(format!("project-read-retired-root-{}.json",context.run.0)),&json!({"runId":context.run,"originalObject":context.fixture.root_object,"retiredRoot":retired,"replacementRoot":context.fixture.read,"scope":"owned root-object invalidation only; original provider/shell processes retained; no restore, signal, input, pane close or replay"}))?;
        project_refused(&mut phone,RequestBody::ArtifactsRead(ArtifactsReadParams {source:shell_source.clone(),read_id:open.read_id.clone(),offset:Uint(0),limit:Uint(65536)})).await?;
        project_close(&mut phone,&shell_source,&open.read_id).await?;
        for original in &originals {original.recheck_processes()?;project_refused(&mut phone,RequestBody::ChangesRead(ChangesReadParams {target:original.target.clone()})).await?;}
        creation.recheck_processes()?;context.source.recheck()?;
        Ok(())
    }.await;
    let available=outcome.is_ok();let results=selected.iter().map(|provider|json!({"gate":"project-read-only","provider":provider,"status":if available&&completed.contains(provider){"passed"}else{"unavailable"},"reason":if available{"completed"}else{"original_read_boundary_or_fixture_prerequisite_unavailable"},"originalsRetained":true,"terminalInputOrGrid":false,"androidSafOrUiProof":false})).collect::<Vec<_>>();
    write_json(&context.evidence.join(format!("project-read-results-{}.json",context.run.0)),&json!({"runId":context.run,"results":results,"observations":observations,"provedCounts":{"readGrantedOriginals":proved_read_originals,"creationOnlyOriginals":proved_creation_originals,"startupFilesPanes":pinned_files_panes},"expectedCounts":{"readGrantedProviders":2,"creationOnlyOmpOriginal":1,"ordinaryStartupFilesPane":1},"scope":"host framed workflows and immutable native reference facts only; no Android SAF/UI/native-input acceptance claimed; all entered results retained/status-only; entered unproved resources are recorded separately, never inferred absent"}))?;
    for result in results {println!("{result}");}phone.connection.close(0u32.into(),b"finite project read complete");outcome.map(|()|true)
}


async fn checkpoint_held(
    args: &clap::ArgMatches,
    evidence: &Path,
    mut grid_hold: Option<&mut GridHold>,
    qualified_config: Option<Config>,
) -> io::Result<bool> {
    let project_read_only=args.get_flag("project-read-only");
    let finite_admission=args.get_flag("admission-only")||args.get_flag("shared-agent-only")||project_read_only||args.get_one::<String>("background").is_some();
    let terminal_only=args.get_flag("terminal-only");
    terminal_mode_environment(args)?;
    project_read_mode_environment(args)?;
    background_mode_environment(args)?;
    codex_single_creation_diagnostic_environment(args)?;
    if args.get_one::<String>("background").is_some(){ui::background_preflight(args).await?;}
    let retain_process=finite_admission||terminal_only||args.get_flag("omp-creation-diagnostic")||args.get_flag("codex-invalidation-diagnostic");
    if args.get_flag("grid") != grid_hold.is_some()
        || args.get_flag("grid") != qualified_config.is_some()
    {
        return Err(grid_failure("retention_construction"));
    }
    if !args.get_flag("consent-owned-providers") {
        return Err(failure(
            "explicit --consent-owned-providers required before owned desktop/provider effects",
        ));
    }
    let selected=if terminal_only {Vec::new()}else{args
        .get_many::<String>("backend")
        .ok_or_else(||failure("select --backend omp --backend codex; missing backend is not silently skipped"))?
        .map(|provider|if provider=="omp"{Provider::Omp}else{Provider::Codex}).collect::<Vec<_>>()};
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
    if omp_creation_diagnostic
        && (selected_omp_capture_mode() != OwnedOmpCaptureMode::Backend
            || owned_hello_capture_requested())
    {
        return Err(failure(
            "OMP creation diagnostic requires only GOATR_OWNED_OMP_BACKEND_CAPTURE=1",
        ));
    }
    let mut unique = HashSet::new();
    if selected.iter().any(|p| !unique.insert(*p)) {
        return Err(failure("duplicate selected backend"));
    }
    require_both_checkpoint_providers(args, &selected)?;
    if finite_admission
        && (owned_omp_capture_requested()
            || std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").is_some()
            || std::env::var_os("GOATR_OWNED_OMP_PRODUCER_CAPTURE").is_some()
            || std::env::var_os("GOATR_OWNED_HERDR_REPORT_BIN").is_some()
            || std::env::var_os("GOATR_OWNED_HERDR_REPORT_SHA256").is_some())
    {
        return Err(failure("finite owned mode conflicts with provider/private diagnostic capture"));
    }
    let deployment = match qualified_config {
        Some(deployment) => deployment,
        None => Config::load(args.get_one::<PathBuf>("config").ok_or_else(||failure("actual deployment --config with preserved installed outer wrappers/source roots required"))?)?,
    };
    for p in &selected {
        if !finite_admission
            && let Some(reason) = deployment.backend_reason(*p)? {
            return Err(prerequisite(reason));
        }
    }
    if !terminal_only&&!deployment.allow_scoped_shell_probe {
        return Err(failure(
            "explicit local scoped probe consent must be saved by actual setup before stock creation gate",
        ));
    }
    if terminal_only&&!deployment.allow_terminal_control {
        return Err(prerequisite("terminal-only requires explicit saved local allow_terminal_control=true"));
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
            "exhaustivePaging":if args.get_one::<String>("background").is_some(){"not_part_of_finite_background_workflow"}else{exhaustive_paging(args)},
            "diagnosticOnly":selected.len()==1,"terminalOnly":terminal_only,"finalBothProviderAndroidAcceptanceRequired":true,
            "scope":if terminal_only{"one fixed public printf fixture in this fresh owned idle shell only; no provider or existing work adoption"}else{"only this fresh run's explicitly owned native TUIs; no existing work adoption"},
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
    let private_herdr = owned_herdr_report_binary(omp_receiver_capture, &workspace)?;
    let project_fixture=if project_read_only {Some(ProjectReadFixture::prepare(args,&root,&binary,&run)?)}else{None};
    let mut config = deployment.clone();
    config.runtime_dir = root.join("runtime");
    config.runtime_dir_agent = config.runtime_dir.clone();
    config.herdr_sockets = vec![herdr_socket.clone()];
    if project_read_only {config.allow_terminal_control=false;config.allow_attachments=false;}
    if terminal_only {
        config.allow_scoped_shell_probe=false;config.allow_attachments=false;
        config.omp_registry_dir=None;config.omp_blob_root=None;config.omp_session_roots.clear();
        config.omp_skills_package_root=None;config.executables.omp=None;config.executables.codex=None;
    }
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
        allow_read_tools: false,
        id: Uuid::random(),
        label: Label::new("Owned checkpoint project").map_err(io::Error::other)?,
        path: root.join("project"),
    }];
    if let Some(fixture)=&project_fixture {
        config.projects[0].allow_read_tools=true;
        config.projects.push(ConfiguredProject {id:Uuid::random(),label:Label::new("Owned creation-only sibling").map_err(io::Error::other)?,path:fixture.creation_only.clone(),allow_read_tools:false});
    }
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
    let native_mcp = if !args.get_flag("grid")
        && !args.get_flag("shared-agent-only")
        && !args.get_flag("paging-proof")
        && args.get_flag("automate-native-inputs")
        && selected.contains(&Provider::Codex)
        && !args.get_flag("codex-invalidation-diagnostic")
    {
        Some(NativeMcp::prepare(&root, &run, &mut config, evidence)?)
    } else {
        None
    };
    if args.get_flag("grid") {
        let native_config = admin.join("config/herdr/config.toml");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&native_config)?;
        file.write_all(b"[terminal]\nmobile_shell_access = \"read_create_input\"\n")?;
        file.sync_all()?;
    }
    let config_path = root.join("config.toml");
    let config_lock = goatr::setup::SetupLock::acquire(&root)?;
    config_lock.publish(
        &config_path,
        &goatr::setup::ExpectedFile::Absent,
        config.serialized()?.as_bytes(),
    )?;
    drop(config_lock);
    if args.get_flag("shared-agent-only") {
        config=Config::load(&config_path)?;
        for provider in &selected {
            if let Some(reason)=config.backend_reason(*provider)? {
                return Err(prerequisite(reason));
            }
        }
    }
    let listener = if args.get_flag("paging-proof")||terminal_only {
        None
    } else {
        Some(Arc::new(std::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        ))?))
    };
    if let (Some(hold), Some(listener)) = (grid_hold.as_deref_mut(), listener.as_ref()) {
        hold.relays.push(listener.clone());
    }
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
    if let Some(fixture)=&project_fixture {fixture.environment(&mut command);}
    if terminal_only{command.env_remove("HERDR_RENDER_PROF");}
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
    let spawn_manifest = retain_process.then(||evidence.join(format!("herdr-spawn-{}.json",run.0)));
    let mut desktop = Desktop::spawn_held(command, grid_hold.as_deref_mut(), spawn_manifest.as_deref())?;
    if let Some(capture) = &herdr_report_capture {
        capture.pin(&desktop.process);
    }
    write_json(
        &evidence.join(format!("herdr-{}.json", run.0)),
        &json!({"runId":run,"process":desktop.process,"executable":config.executables.herdr,"outerSha256":binary_sha256(&config.executables.herdr)?,"socket":herdr_socket,"cwd":config.projects[0].path,"XDG_CONFIG_HOME":admin.join("config"),"XDG_RUNTIME_DIR":runtime}),
    )?;
    if retain_process {
        println!("{}",json!({"gate":"admission-owned-herdr",
            "manifest":evidence.join(format!("herdr-{}.json",run.0))}));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let (source_evidence, admission_native) = loop {
        if let Ok(source) = surface(&herdr_socket).await {
            break source;
        }
        if Instant::now() > deadline {
            return Err(prerequisite(
                "owned actual Herdr source/socket/PID namespace prerequisites unavailable; recorded desktop retained",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    if retain_process {
        let manifest = evidence.join(format!("admission-source-{}.json",run.0));
        write_json(&manifest, &json!({"runId":run,"herdr":desktop.process,
            "source":source_evidence,"socket":identity(&herdr_socket)?,
            "scope":"retained fresh owned Herdr; original kernel process and validated socket source"}))?;
        println!("{}",json!({"gate":"admission-source","manifest":manifest}));
        if source_evidence.linux_boot_id != desktop.process.linux_boot_id
            || source_evidence.pid != desktop.process.pid
            || source_evidence.pid_start_ticks != desktop.process.start_ticks
        {
            return Err(prerequisite("owned Herdr source does not match original child"));
        }
    }
    let mut serve = if args.get_flag("grid") || retain_process {
        start_serve_with_capture(
            &binary,
            &config_path,
            &root,
            &socket,
            listener.as_deref(),
            ServeLaunch {
                diagnostic_capture: args.get_flag("codex-invalidation-diagnostic"),
                herdr_report_capture: None,
                preserve_xdg_config_home: false,
                retain_process,
                hello_capture: finite_admission,
                grid_hold: grid_hold.as_deref_mut(),
                project_read: project_fixture.as_ref(),
                startup_deadline: Some(deadline),
                unit_authority: None,
            },
        )?
    } else {
        start_serve_with_codex_capture(
            &binary,
            &config_path,
            &root,
            &socket,
            listener.as_deref(),
            args.get_flag("codex-invalidation-diagnostic"),
            herdr_report_capture.as_ref(),
        )?
    };
    healthy(&mut serve, &socket).await?;
    write_json(
        &evidence.join(format!("serve-{}.json", run.0)),
        &json!({"runId":run,"process":serve.process,"adminSocket":identity(&socket)?,"absoluteConfig":config_path,"OMPownedPort":port,"serveArgument":"--admin-socket identical on pair/list/revoke/restart","scope":"ordinary configured packaged serve, not hidden transport smoke"}),
    )?;
    if retain_process {
        let manifest = evidence.join(format!("admission-ownership-{}.json",run.0));
        write_json(&manifest,
            &json!({"runId":run,"herdr":desktop.process,"source":source_evidence,
                "herdrSocket":identity(&herdr_socket)?,"companion":serve.process,
                "adminSocket":identity(&socket)?,"root":root,"config":config_path,
                "scope":"retained fresh originals; no automatic teardown; review exact identities before cleanup"}))?;
        println!("{}",json!({"gate":"admission-ownership","manifest":manifest}));
    }
    let ready = json!({"gate":"owned-runtime-ready","runId":run,"root":root,"herdrSocket":herdr_socket,"OMPownedPort":port,"scope":if terminal_only{"retained provider-independent owned stock source; no provider relay or readiness dependency"}else{"sanitized resource rendezvous; relay is live, no provider readiness claimed"}});
    if args.get_flag("grid") {
        let _ = writeln!(std::io::stdout(), "{ready}");
    } else {
        println!("{ready}");
    }
    let endpoint = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?))
        .bind()
        .await
        .map_err(io::Error::other)?;
    let mut phone = pair(&binary, &root, &socket, &endpoint,None).await?;
    if terminal_only {
        let result=terminal_flow(&mut phone,&config,&herdr_socket,&mut desktop,&source_evidence,admission_native,&name,evidence,&run).await;
        phone.connection.close(0u32.into(),b"finite terminal workflow complete");
        return result;
    }
    if args.get_flag("grid") {
        return grid_flow(
            &mut phone,
            &config,
            &herdr_socket,
            &desktop.process,
            &mut serve,
            evidence,
            &run,
            &selected,
            grid_hold
                .as_deref_mut()
                .ok_or_else(|| grid_failure("retention_missing"))?,
        )
        .await;
    }
    if finite_admission {
        let server=creation_admission(&mut phone, &selected, &name, evidence, &run,
            AdmissionSource{socket:&herdr_socket,desktop:&mut desktop,original:&source_evidence,native:admission_native}).await?;
        if args.get_flag("admission-only") {
            phone.connection.close(0u32.into(), b"finite admission complete");
            return Ok(server.is_some());
        }
        if args.get_one::<String>("background").is_some(){
            let server=server.ok_or_else(||prerequisite("background finite both-provider admission unavailable"))?;
            let mut originals=Vec::with_capacity(2);
            for provider in selected.iter().copied(){
                server.recheck()?;
                let (original,receipt)=create(&mut phone,&config,&herdr_socket,provider,&server.identity.0,evidence,&run).await?;
                if provider==Provider::Codex&&let Some(mcp)=&native_mcp {
                    mcp.bind_owner(&original)?;
                }
                write_json(&evidence.join(format!("background-original-{}-{provider:?}.json",run.0)),&json!({"runId":run,"mode":args.get_one::<String>("background"),"creationReceipt":receipt,"proof":original.evidence(),"resourcesRetained":true,"cleanupAuthorized":false}))?;
                originals.push(original);
            }
            ui::background(args,evidence,ui::Context {binary:&binary,root:&root,socket:&socket,config:&config,herdr_socket:&herdr_socket,herdr_process:&desktop.process,
                mcp:native_mcp.as_ref(),serve:&mut serve,listener:listener.as_deref().ok_or_else(||failure("background owned listener absent"))?,endpoint:&endpoint,phone:&mut phone,native:&mut originals,history_witnesses:&[],feedback_presentation:&[]},&run).await?;
            return Ok(true);
        }
        if let Some(fixture)=&project_fixture {
            let server=server.ok_or_else(||prerequisite("project-read-only finite creation admission unavailable"))?;
            return project_read_flow(phone,&selected,ProjectReadRun {fixture,config:&config,socket:&herdr_socket,desktop:&desktop,source:&server,evidence,run:&run}).await;
        }
        let server=server.ok_or_else(||prerequisite("shared-agent creation admission unavailable"))?;
        return shared_agent_flow(phone, &selected, SharedAgentContext {
            inputs:NativeInputs{enabled:true,paging_proof:false,config:&config,socket:&herdr_socket,
                herdr:&desktop.process,evidence,run:&run,mcp:None,plan:RefCell::new(None),
                mcp_dialogs:RefCell::new(HashMap::new())},
            companion:&serve,endpoint:&endpoint,source:&server,
        }).await;
    }
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
            let probe_disposition = if receipt.creation.as_ref().is_some_and(|creation| {
                creation.stage == CreationStage::Workspace && receipt.state != ReceiptState::Succeeded
            }) {
                if serve.finish_retained_omp_capture().await { "observed" } else { "unavailable" }
            } else {
                "not_required_by_receipt"
            };
            serve.finish_guard_drain();
            let succeeded = receipt.state == ReceiptState::Succeeded;
            println!(
                "{}",
                json!({"gate":"omp-creation-diagnostic","runId":run,
                    "action":receipt.action_id,"state":format!("{:?}", receipt.state),
                    "stage":stage,"stageState":stage_state,"errorCode":error_code,
                    "status":if succeeded {"succeeded"} else {"not_succeeded"},
                    "probeDisposition":probe_disposition,"resourcesRetained":true,
                    "cleanupAuthorized":false}),
            );
            return Ok(succeeded);
        }
        if p == Provider::Codex && args.get_flag("codex-invalidation-diagnostic") {
            let deadline = Instant::now() + Duration::from_secs(120);
            let receipt = create_codex_single_creation_diagnostic(
                &mut phone,
                &config,
                &server,
                &serve,
                deadline,
            )
            .await?;
            match receipt.state {
                ReceiptState::Failed | ReceiptState::Uncertain => {
                    let drain = serve.codex_creation_guard_by(deadline).await?;
                    write_json(
                        &evidence.join(format!("codex-single-creation-diagnostic-{}.json", run.0)),
                        &codex_single_creation_record(&receipt, Some(drain)),
                    )?;
                    return Ok(false);
                }
                ReceiptState::Succeeded => {
                    write_json(
                        &evidence.join(format!("codex-single-creation-diagnostic-{}.json", run.0)),
                        &codex_single_creation_record(&receipt, None),
                    )?;
                    return Ok(false);
                }
                ReceiptState::Reserved | ReceiptState::Dispatching | ReceiptState::Dispatched => {
                    return Err(failure("Codex single creation had no terminal receipt; resources retained"));
                }
            }
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
        let req = RequestBody::PromptSubmit(PromptSubmitParams { skill: None, boot_id: phone.hello.boot_id.clone(),
        delivery: None, native_turn_id: None, images: None,
        target: p.target.clone(),
        text: PromptText(format!(
            "Reply only goatr-once-{}-{:?}",
            &run.0[..8],
            p.provider
        )), });
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
    serve = start_serve(&binary, &config_path, &root, &socket, listener.as_deref())?;
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
    serve = start_serve(&binary, &config_path, &root, &socket, listener.as_deref())?;
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
            serve = start_serve(&binary, &config_path, &root, &socket, listener.as_deref())?;
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
            serve = start_serve(&binary, &config_path, &root, &socket, listener.as_deref())?;
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
                let mut guard = pair(&binary, &root, &socket, &guard_endpoint,None).await?;
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
pub(super) struct LiveAdmission {
    args: clap::ArgMatches,
    unit: Option<goatr::codex_relay::UnitWakePin>,
}
pub(super) fn admit_arguments() -> io::Result<LiveAdmission> {
    let args = feedback_arguments(arguments_base()).try_get_matches_from(std::env::args_os())
        .map_err(|error|io::Error::new(io::ErrorKind::InvalidInput,error.to_string()))?;
    let diagnostic = physical_host_preflight_selection(&args)?;
    physical_mode_environment(&args)?;
    let expected = if args.get_flag("physical-mvp") || args.get_flag("physical-pairing-checkpoint") || args.get_flag("physical-owned-retirement") || args.get_flag("physical-owned-process-inventory") || args.get_flag("physical-owned-process-stop") || matches!(diagnostic,Some(physical::ControllerPolicy::WakeDiagnostic {..})) {
        args.get_one::<String>("expected-controller-unit").map(String::as_str)
    } else { None };
    let unit = goatr::codex_relay::prepare_current_executable(true,expected)?;
    Ok(LiveAdmission {args,unit})
}
pub(super) async fn run(admitted: Option<LiveAdmission>) -> io::Result<bool> {
    let LiveAdmission {args,unit}=match admitted {
        Some(admitted)=>admitted,
        None=>LiveAdmission {args:arguments(),unit:None},
    };
    if let Some(policy)=physical_host_preflight_selection(&args)? {
        let evidence=std::path::absolute(args.get_one::<PathBuf>("evidence-dir").expect("required host preflight evidence"))?;
        fresh_shared_evidence(&evidence)?;
        return physical::host_preflight(&evidence,policy,unit);
    }
    physical_mode_environment(&args)?;
    if args.get_flag("physical-mvp") {
        let evidence=std::path::absolute(args.get_one::<PathBuf>("evidence-dir").expect("required physical evidence"))?;
        fresh_shared_evidence(&evidence)?;
        physical::run(&args,&evidence,unit.ok_or_else(||failure("full mode lost its synchronous admitted unit context"))?).await?;
        return Ok(true);
    }
    if args.get_flag("physical-pairing-checkpoint") {
        let evidence=std::path::absolute(args.get_one::<PathBuf>("evidence-dir").expect("required retained pairing evidence"))?;
        fresh_shared_evidence(&evidence)?;
        physical::pairing_checkpoint(&args,&evidence,unit.ok_or_else(||failure("retained pairing lost its synchronous admitted unit context"))?).await?;
        return Ok(true);
    }
    if args.get_flag("physical-owned-retirement") {
        let evidence=std::path::absolute(args.get_one::<PathBuf>("evidence-dir").expect("required owned retirement evidence"))?;
        fresh_shared_evidence(&evidence)?;
        physical::owned_retirement(&args,&evidence,unit.ok_or_else(||failure("owned retirement lost its synchronous admitted unit context"))?).await?;
        return Ok(true);
    }
    if args.get_flag("physical-owned-process-inventory") {
        let evidence=std::path::absolute(args.get_one::<PathBuf>("evidence-dir").expect("required owned process inventory evidence"))?;
        fresh_shared_evidence(&evidence)?;
        physical::owned_process_inventory(&args,&evidence,unit.ok_or_else(||failure("owned process inventory lost its synchronous admitted unit context"))?)?;
        return Ok(true);
    }
    if args.get_flag("physical-owned-process-stop") {
        let evidence=std::path::absolute(args.get_one::<PathBuf>("evidence-dir").expect("required owned process stop evidence"))?;
        fresh_shared_evidence(&evidence)?;
        physical::owned_process_stop(&args,&evidence,unit.ok_or_else(||failure("owned process stop lost its synchronous admitted unit context"))?)?;
        return Ok(true);
    }
    terminal_mode_environment(&args)?;
    project_read_mode_environment(&args)?;
    background_mode_environment(&args)?;
    codex_single_creation_diagnostic_environment(&args)?;
    // Reject an invalid ephemeral selector before even creating its evidence directory.
    physical_relay_request(&args)?;
    let evidence = std::path::absolute(
        args.get_one::<PathBuf>("evidence-dir")
            .expect("required evidence"),
    )?;
    if args.get_flag("shared-agent-only")||args.get_flag("terminal-only")||args.get_flag("project-read-only")||args.get_one::<String>("background").is_some() {
        if args.get_flag("shared-agent-only"){require_shared_backends(&args)?;}
        if args.get_flag("project-read-only"){require_shared_backends(&args).map_err(|_|failure("project-read-only requires exactly one OMP and one Codex backend"))?;}
        fresh_shared_evidence(&evidence)?;
    }
    if args.get_flag("physical-hello-capture") {
        return physical_hello_capture(&args, &evidence).await;
    }
    private_dir(&evidence, true)?;
    if args.get_one::<String>("background").is_some(){
        let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result=tokio::select!{
            result=checkpoint(&args,&evidence)=>result,
            _=tokio::signal::ctrl_c()=>Err(failure("background cancelled; original providers/receipts retained; no replay or automatic native cleanup")),
            _=terminate.recv()=>Err(failure("background cancelled; original providers/receipts retained; no replay or automatic native cleanup")),
        };
        if let Err(error)=&result {
            let unavailable=args.get_many::<String>("backend").into_iter().flatten().map(|provider|json!({"gate":"background","mode":args.get_one::<String>("background"),"provider":provider,"status":"unavailable","reason":"preparation_or_real_delivery_boundary","boundary":super::helper_error_metadata(error),"resourcesRetained":true,"cleanupAuthorized":false})).collect::<Vec<_>>();
            write_json(&evidence.join(format!("background-unavailable-{}.json",Uuid::random().0)),&json!({"results":unavailable}))?;
            for row in unavailable{println!("{row}");}
        }
        return result;
    }
    if args.get_flag("project-read-only") {
        let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result=tokio::select! {
            result=checkpoint(&args,&evidence)=>result,
            _=tokio::signal::ctrl_c()=>Err(failure("project-read-only cancelled; originals and entered receipts retained, never replayed")),
            _=terminate.recv()=>Err(failure("project-read-only cancelled; originals and entered receipts retained, never replayed")),
        };
        if let Err(error)=&result {
            let results=args.get_many::<String>("backend").into_iter().flatten().map(|provider|json!({"gate":"project-read-only","provider":provider,"status":"unavailable","reason":"preparation_or_original_read_boundary","boundary":super::helper_error_metadata(error),"scope":"retained originals/receipts; no input, replay, native closure or cleanup; not Android SAF/UI proof"})).collect::<Vec<_>>();
            write_json(&evidence.join(format!("project-read-unavailable-{}.json",Uuid::random().0)),&json!({"results":results}))?;
            for result in results {println!("{result}");}
        }
        return result;
    }

    if args.get_flag("terminal-only") {
        let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result=tokio::select!{
            result=checkpoint(&args,&evidence)=>result,
            _=tokio::signal::ctrl_c()=>Err(failure("terminal-only cancelled; original resources retained, no input replay")),
            _=terminate.recv()=>Err(failure("terminal-only cancelled; original resources retained, no input replay")),
        };
        if let Err(error)=&result {
            let record=json!({"gate":"terminal-only","status":"unavailable","reason":"preparation_or_observation",
                "boundary":super::helper_error_metadata(error),"resources":"retained originals; no stop/closure/rollback",
                "nativeTag19":"unproved","phase4Input":"unproved","scope":"finite single-controller stock interoperability only"});
            write_json(&evidence.join(format!("terminal-unavailable-{}.json",Uuid::random().0)),&record)?;
            println!("{record}");
        }
        return result;
    }
    if args.get_flag("admission-only") {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result = tokio::select! {
            result=checkpoint(&args,&evidence)=>result,
            _=tokio::signal::ctrl_c()=>Err(failure("admission cancelled; fresh owned resources retained")),
            _=terminate.recv()=>Err(failure("admission cancelled; fresh owned resources retained")),
        };
        if result.is_err() {
            let results = args.get_many::<String>("backend").into_iter().flatten().map(|provider|
                json!({"gate":"admission-only","provider":provider,"status":"unavailable",
                    "reason":"preparation_unavailable","scope":"no provider creation or input admitted"})
            ).collect::<Vec<_>>();
            write_json(&evidence.join(format!("admission-unavailable-{}.json",Uuid::random().0)),
                &json!({"results":results}))?;
            for result in results { println!("{result}"); }
            return Err(prerequisite("finite admission preparation unavailable; fresh ownership evidence retained"));
        }
        return result;
    }
    if args.get_flag("shared-agent-only") {
        let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result=tokio::select! {
            result=checkpoint(&args,&evidence)=>result,
            _=tokio::signal::ctrl_c()=>Err(failure("shared-agent cancelled; original receipts/resources retained")),
            _=terminate.recv()=>Err(failure("shared-agent cancelled; original receipts/resources retained")),
        };
        if let Err(error)=result {
            let results=args.get_many::<String>("backend").into_iter().flatten().map(|provider|
                json!({"gate":"shared-agent-only","provider":provider,"status":"unavailable",
                    "reason":"preparation_or_cancellation","boundary":super::helper_error_metadata(&error),
                    "scope":"retained original jobs/receipts; inspect only, never replay or automatic cleanup"})
            ).collect::<Vec<_>>();
            write_json(&evidence.join(format!("shared-agent-unavailable-{}.json",Uuid::random().0)),
                &json!({"results":results}))?;
            for result in results {println!("{result}");}
            return Ok(false);
        }
        return result;
    }
    if args.get_flag("grid") {
        return grid_run(&args, &evidence).await;
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
    use std::os::unix::fs::PermissionsExt;
    use std::os::{fd::{AsRawFd,FromRawFd},unix::{net::UnixStream as FileWakeChannel,process::CommandExt}};
    struct FileWakeFixtureChild(Child);
    impl FileWakeFixtureChild {
        fn exit(&mut self)->std::process::ExitStatus {
            let deadline=Instant::now()+Duration::from_secs(5);
            loop{if let Some(status)=self.0.try_wait().unwrap(){return status;}assert!(Instant::now()<deadline,"owned fixture exit timed out");std::thread::sleep(Duration::from_millis(10));}
        }
    }
    impl Drop for FileWakeFixtureChild {fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
    fn file_wake_endpoint(key:&str)->Option<FileWakeChannel> {
        let fd=std::env::var(key).ok()?.parse::<i32>().unwrap();assert!(fd>2);let channel=unsafe{FileWakeChannel::from_raw_fd(fd)};
        channel.set_read_timeout(Some(Duration::from_secs(5))).unwrap();channel.set_write_timeout(Some(Duration::from_secs(5))).unwrap();Some(channel)
    }
    fn file_wake_state(channel:&mut FileWakeChannel,expected:u8){let mut state=[0];channel.read_exact(&mut state).unwrap();assert_eq!(state,[expected]);}
    #[test]
    fn terminal_pair_deadline_reaps_the_original_cli_before_returning() {
        if let Some(mut channel)=file_wake_endpoint("GOATR_PAIR_DEADLINE_TEST_FD") {
            let parent=std::env::var("GOATR_PAIR_DEADLINE_DRIVER_PARENT").unwrap().parse::<u32>().unwrap();
            if unsafe{libc::getppid()} as u32!=parent {
                // This is the actual terminal helper subprocess; its PID travels only over fixed IPC.
                assert!(goatr::codex_relay::current_capabilities().unwrap().zero());
                channel.write_all(&std::process::id().to_be_bytes()).unwrap();
                let mut hold=[0];let _=channel.read_exact(&mut hold);return;
            }
            let binary=std::env::current_exe().unwrap();
            let deadline=Instant::now()+Duration::from_secs(3);
            let error=terminal_pair(&binary,None,None,Some(deadline)).unwrap_err();
            assert_eq!(error.kind(),io::ErrorKind::TimedOut);
            assert_eq!(terminal_pair(Path::new("/nonexistent-goatr-pair-deadline-helper"),None,None,Some(Instant::now())).unwrap_err().kind(),io::ErrorKind::TimedOut);
            channel.write_all(b"T").unwrap();return;
        }
        let test=concat!(module_path!(),"::terminal_pair_deadline_reaps_the_original_cli_before_returning");
        let test=test.strip_prefix(concat!(env!("CARGO_CRATE_NAME"),"::")).unwrap();
        let (mut channel,endpoint)=FileWakeChannel::pair().unwrap();
        channel.set_read_timeout(Some(Duration::from_secs(5))).unwrap();channel.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let fd=endpoint.as_raw_fd();assert!(fd>2);let mut command=std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact",test,"--test-threads=1"]).env("GOATR_PAIR_DEADLINE_TEST_FD",fd.to_string())
            .env("GOATR_PAIR_DEADLINE_DRIVER_PARENT",std::process::id().to_string())
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe{command.pre_exec(move||{goatr::codex_relay::drop_inherited_capabilities()?;let flags=libc::fcntl(fd,libc::F_GETFD);if flags<0||libc::fcntl(fd,libc::F_SETFD,flags&!libc::FD_CLOEXEC)<0{return Err(io::Error::last_os_error());}Ok(())});}
        let mut driver=FileWakeFixtureChild(command.spawn().unwrap());drop(endpoint);
        let mut bytes=[0;4];channel.read_exact(&mut bytes).unwrap();let pid=u32::from_be_bytes(bytes);
        let original=goatr::codex_relay::ProcessStart::capture(pid).unwrap();original.evidence(pid).unwrap();
        file_wake_state(&mut channel,b'T');
        assert_eq!(fs::symlink_metadata(format!("/proc/{pid}")).unwrap_err().kind(),io::ErrorKind::NotFound);
        assert!(driver.exit().success());
    }
    #[test]
    fn first_child_final_admission_refuses_exact_or_late_absolute_deadline_with_real_evidence() {
        let process=herdr::process_evidence(std::process::id()).unwrap();let start=Instant::now();let deadline=start+Duration::from_secs(1);
        assert_eq!(admit_first_child_evidence(process.clone(),deadline,start).unwrap(),process);
        for observed in [deadline,deadline+Duration::from_secs(2)] {
            assert_eq!(admit_first_child_evidence(process.clone(),deadline,observed).unwrap_err().kind(),io::ErrorKind::TimedOut);
        }
    }
    #[test]
    fn privileged_frontend_routes_malformed_and_mixed_selectors_before_shortcuts() {
        for selector in ["--physical-mvp","--physical-mvp=false","--physical-mvp=malformed",
            "--expected-controller-unit=goatr-host-wake-invalid.service","--expected-controller-sha256=bad"] {
            for shortcut in ["--owned-mcp-stdio","--review-fixture","--terminal-only"] {
                let args=["smoke",shortcut,selector];
                assert!(args.iter().any(|arg|crate::early_live_argument(std::ffi::OsStr::new(arg))));
                assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).is_err());
            }
        }
    }
    #[test]
    fn first_child_identity_wait_is_read_only_bounded_and_pins_dumpability_transition() {
        if let Some(mut channel)=file_wake_endpoint("GOATR_SERVE_DUMP_TEST_FD") {
            goatr::codex_relay::drop_inherited_capabilities().unwrap();assert_eq!(unsafe{libc::prctl(libc::PR_SET_DUMPABLE,0,0,0,0)},0);
            channel.write_all(b"0").unwrap();file_wake_state(&mut channel,b'1');
            assert_eq!(unsafe{libc::prctl(libc::PR_SET_DUMPABLE,1,0,0,0)},0);channel.write_all(b"1").unwrap();file_wake_state(&mut channel,b'2');return;
        }
        let test=concat!(module_path!(),"::first_child_identity_wait_is_read_only_bounded_and_pins_dumpability_transition");
        let test=test.strip_prefix(concat!(env!("CARGO_CRATE_NAME"),"::")).unwrap();let executable=std::env::current_exe().unwrap();let metadata=fs::metadata(&executable).unwrap();
        let (mut channel,endpoint)=FileWakeChannel::pair().unwrap();channel.set_read_timeout(Some(Duration::from_secs(5))).unwrap();channel.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let fd=endpoint.as_raw_fd();assert!(fd>2);let object=(metadata.dev(),metadata.ino());let mut command=std::process::Command::new(executable);
        command.args(["--exact",test,"--test-threads=1"]).env("GOATR_SERVE_DUMP_TEST_FD",fd.to_string()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe{command.pre_exec(move||{goatr::codex_relay::drop_inherited_capabilities()?;let flags=libc::fcntl(fd,libc::F_GETFD);if flags<0||libc::fcntl(fd,libc::F_SETFD,flags&!libc::FD_CLOEXEC)<0{return Err(io::Error::last_os_error());}Ok(())});}
        let mut child=FileWakeFixtureChild(command.spawn().unwrap());drop(endpoint);let pid=child.0.id();file_wake_state(&mut channel,b'0');
        let original=goatr::codex_relay::ProcessStart::capture(pid).unwrap();assert_eq!(original.evidence(pid).unwrap_err().kind(),io::ErrorKind::PermissionDenied);
        let other=goatr::codex_relay::ProcessStart::capture(std::process::id()).unwrap();
        assert!(first_child_evidence(&mut child.0,&other,object,Instant::now()+Duration::from_secs(1)).is_err());
        assert_eq!(first_child_evidence(&mut child.0,&original,object,Instant::now()).unwrap_err().kind(),io::ErrorKind::TimedOut);
        std::thread::scope(|scope| {
            let (send,receive)=std::sync::mpsc::sync_channel(1);let child=&mut child.0;let original=&original;
            scope.spawn(move||{send.send(first_child_evidence(child,original,object,Instant::now()+Duration::from_secs(3))).unwrap();});
            assert!(matches!(receive.recv_timeout(Duration::from_millis(50)),Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
            channel.write_all(b"1").unwrap();file_wake_state(&mut channel,b'1');
            let proof=receive.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();assert_eq!(proof.pid,pid);
        });
        assert!(first_child_evidence(&mut child.0,&original,(object.0,object.1+1),Instant::now()+Duration::from_secs(1)).is_err());
        channel.write_all(b"2").unwrap();assert!(child.exit().success());
        assert!(first_child_evidence(&mut child.0,&original,object,Instant::now()+Duration::from_secs(1)).is_err());
    }
    #[test]
    fn unavailable_codex_terminal_capture_is_not_a_smoke_option() {
        let error = feedback_arguments(arguments_base())
            .try_get_matches_from(["smoke", "--codex-startup-terminal-capture"])
            .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }
    fn background_args(mode:&str)->Vec<String> {
        let mut args=vec!["smoke".into(),format!("--background={mode}"),"--backend=omp".into(),"--backend=codex".into(),
            "--goatr=/nix/store/example/bin/goatr".into(),"--config=/private/config.toml".into(),"--workspace-root=/workspace".into(),
            "--evidence-dir=/private/fresh".into(),"--consent-owned-providers".into(),"--automate-native-inputs".into(),
            "--serial=emulator-5594".into(),"--emulator-owner=/private/run.json".into(),"--apk=/private/app.apk".into(),"--test-apk=/private/test.apk".into()];
        for field in ["expected-goatr-sha256","expected-companion-lock-sha256","expected-apk-sha256","expected-test-apk-sha256"]{args.push(format!("--{field}={}","1".repeat(64)));}
        if mode=="ntfy" {
            args.push(format!("--expected-ntfy-apk-sha256={NTFY_APK_SHA256}"));
            args.push(format!("--expected-ntfy-signer-sha256={NTFY_SIGNER_SHA256}"));
        }
        args
    }
    fn physical_mvp_args()->Vec<String> {
        let mut args=vec!["smoke".into(),"--physical-mvp".into(),"--backend=omp".into(),"--backend=codex".into(),
            "--goatr=/private/frozen-goatr".into(),"--config=/private/config.toml".into(),"--workspace-root=/private/workspace".into(),
            "--evidence-dir=/private/fresh".into(),"--consent-owned-providers".into(),"--serial=owned-physical-serial".into(),
            "--apk=/private/app.apk".into(),"--test-apk=/private/test.apk".into()];
        for field in ["expected-goatr-sha256","expected-companion-lock-sha256","expected-apk-sha256","expected-test-apk-sha256"]{args.push(format!("--{field}={}","1".repeat(64)));}
        args.push(format!("--expected-config-sha256={}","3".repeat(64)));
        args.push(format!("--expected-controller-unit=goatr-host-wake-{}.service",Uuid::random().0));args.push(format!("--expected-controller-sha256={}","4".repeat(64)));
        args.push(format!("--expected-ntfy-apk-sha256={NTFY_APK_SHA256}"));
        args.push(format!("--expected-ntfy-signer-sha256={NTFY_SIGNER_SHA256}"));
        args
    }
    fn retained_pairing_args()->Vec<String> {
        let mut args=vec!["smoke".into(),"--physical-pairing-checkpoint".into(),
            "--goatr=/private/frozen-goatr".into(),"--config=/private/config.toml".into(),"--workspace-root=/private/workspace".into(),
            "--evidence-dir=/private/fresh".into(),"--serial=owned-physical-serial".into(),
            "--apk=/private/app.apk".into(),"--test-apk=/private/test.apk".into()];
        for field in ["expected-goatr-sha256","expected-companion-lock-sha256","expected-apk-sha256","expected-test-apk-sha256"]{args.push(format!("--{field}={}","1".repeat(64)));}
        args.push(format!("--expected-current-test-apk-sha256={}","2".repeat(64)));
        args.extend(["--expected-product-uid=10528","--expected-product-first-install-time=2026-10-27 04:39:23","--expected-product-data-dir=/data/user/0/io.github.trevarj.goatr.debug","--expected-test-uid=10101","--expected-test-first-install-time=2026-10-27 04:39:23","--expected-test-data-dir=/data/user/0/io.github.trevarj.goatr.debug.test"].into_iter().map(str::to_owned));
        args.push(format!("--expected-config-sha256={}","3".repeat(64)));
        args.push(format!("--expected-controller-unit=goatr-host-wake-{}.service",Uuid::random().0));args.push(format!("--expected-controller-sha256={}","4".repeat(64)));
        args
    }
    fn owned_process_inventory_args()->Vec<String> {vec!["smoke".to_owned(),"--physical-owned-process-inventory".to_owned(),"--owned-process-source-manifest=/private/sources.json".to_owned(),format!("--expected-owned-process-source-manifest-sha256={}","1".repeat(64)),"--owned-app-retirement-gate=/private/gate.json".to_owned(),format!("--expected-owned-app-retirement-gate-sha256={}","2".repeat(64)),format!("--expected-controller-unit=goatr-host-wake-{}.service",Uuid::random().0),format!("--expected-controller-sha256={}","3".repeat(64)),format!("--expected-companion-lock-sha256={}",transport::lock_sha256()),"--evidence-dir=/private/fresh".to_owned()]}
    fn owned_process_stop_args()->Vec<String> {vec!["smoke".to_owned(),"--physical-owned-process-stop".to_owned(),"--owned-process-stop-approval=/private/approval.json".to_owned(),format!("--expected-owned-process-stop-approval-sha256={}","0".repeat(64)),"--owned-process-source-manifest=/private/sources.json".to_owned(),format!("--expected-owned-process-source-manifest-sha256={}","1".repeat(64)),"--owned-app-retirement-gate=/private/gate.json".to_owned(),format!("--expected-owned-app-retirement-gate-sha256={}","2".repeat(64)),format!("--expected-controller-unit=goatr-host-wake-{}.service",Uuid::random().0),format!("--expected-controller-sha256={}","3".repeat(64)),format!("--expected-companion-lock-sha256={}",transport::lock_sha256()),"--evidence-dir=/private/fresh".to_owned()]}
    fn host_preflight_args(wake:bool)->Vec<String> {
        let mut args=vec!["smoke".to_owned(),"--evidence-dir=/private/fresh".to_owned()];
        if wake {
            args.push("--physical-host-wake-preflight".to_owned());args.push(format!("--expected-controller-unit=goatr-host-wake-{}.service",Uuid::random().0));
        }else{args.push("--physical-host-preflight".to_owned());}
        args
    }
    #[test]
    fn physical_host_wake_preflight_requires_canonical_unit_and_never_selects_zero_or_runtime() {
        let args=host_preflight_args(true);let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();
        let unit=parsed.get_one::<String>("expected-controller-unit").unwrap();
        assert_eq!(physical_host_preflight_selection(&parsed).unwrap(),Some(physical::ControllerPolicy::WakeDiagnostic {unit}));
        for prefix in ["--expected-controller-unit=","--evidence-dir="] {
            let missing=args.iter().filter(|argument|!argument.starts_with(prefix)).cloned().collect::<Vec<_>>();
            assert_eq!(feedback_arguments(arguments_base()).try_get_matches_from(missing).unwrap_err().kind(),clap::error::ErrorKind::MissingRequiredArgument);
        }
        for invalid in [unit.to_uppercase(),format!("/private/{unit}"),unit.replace(".service",".scope"),"goatr-host-wake-invalid.service".to_owned()] {
            let mut bad=args.iter().filter(|argument|!argument.starts_with("--expected-controller-unit=")).cloned().collect::<Vec<_>>();
            bad.push(format!("--expected-controller-unit={invalid}"));let parsed=feedback_arguments(arguments_base()).try_get_matches_from(bad).unwrap();
            assert!(physical_host_preflight_selection(&parsed).is_err());
        }
        let mut old=host_preflight_args(false);old.push(format!("--expected-controller-unit={unit}"));
        assert!(feedback_arguments(arguments_base()).try_get_matches_from(old).map_or(true,|parsed|physical_host_preflight_selection(&parsed).is_err()));
        let no_selector=args.iter().filter(|argument|argument.as_str()!="--physical-host-wake-preflight").cloned().collect::<Vec<_>>();
        assert!(feedback_arguments(arguments_base()).try_get_matches_from(no_selector).map_or(true,|parsed|physical_host_preflight_selection(&parsed).is_err()));
    }
    #[test]
    fn physical_host_preflight_policy_accepts_only_selector_and_fresh_evidence() {
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(["smoke","--physical-host-preflight","--evidence-dir=/private/fresh"]).unwrap();
        assert_eq!(physical_host_preflight_selection(&parsed).unwrap(),Some(physical::ControllerPolicy::Zero));
        assert_eq!(parsed.value_source("api"),Some(clap::parser::ValueSource::DefaultValue));
        assert_eq!(feedback_arguments(arguments_base()).try_get_matches_from(["smoke","--physical-host-preflight"]).unwrap_err().kind(),clap::error::ErrorKind::MissingRequiredArgument);
        let ordinary=feedback_arguments(arguments_base()).try_get_matches_from(["smoke","--evidence-dir=/private/fresh"]).unwrap();
        assert!(physical_host_preflight_selection(&ordinary).unwrap().is_none());
        for ordinary in [physical_mvp_args(),background_args("persistent"),background_args("ntfy")] {
            let parsed=feedback_arguments(arguments_base()).try_get_matches_from(ordinary).unwrap();
            assert!(physical_host_preflight_selection(&parsed).unwrap().is_none());
        }
    }
    #[test]
    fn physical_host_preflight_policy_refuses_effect_inputs_and_complete_runtime_modes() {
        for selector in ["--api=30","--api=34","--backend=omp","--config=/private/config","--goatr=/private/goatr",
            "--workspace-root=/private/workspace","--serial=74110723","--apk=/private/app.apk","--test-apk=/private/test.apk",
            "--expected-goatr-sha256=pin","--expected-companion-lock-sha256=pin","--expected-apk-sha256=pin",
            "--expected-test-apk-sha256=pin","--expected-config-sha256=pin","--expected-ntfy-apk-sha256=pin","--expected-ntfy-signer-sha256=pin",
            "--consent-owned-providers","--automate-native-inputs","--physical-mvp","--background=persistent","--background=ntfy",
            "--admission-only","--shared-agent-only","--terminal-only","--project-read-only","--grid","--grid-lease=/private/lease",
            "--ui","--camera","--ime","--setup","--prepare-emulator","--emulator-owner=/private/owner",
            "--paging-proof","--codex-invalidation-diagnostic","--omp-creation-diagnostic","--physical-hello-capture",
            "--physical-relay-ephemeral","--development-continuity","--development-owner-feedback","--development-android-only",
            "--keep-foreground-for-feedback","--defer-retention-crossing","--admin-socket=/private/socket","--disposable-uid=1901",
            "--project-read-seed-pack=/private/pack","--review-fixture","--owned-mcp-stdio=/private/lease",
            "--prepared-fresh-install=/private/creator.json","--expected-prepared-install-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--physical-host-preflight","--physical-host-wake-preflight",
            "--physical-host-preflight=true","--physical-host-preflight=false","--physical-host-preflight=1",
            "--physical-host-wake-preflight=true","--physical-host-wake-preflight=false","--physical-host-wake-preflight=1"] {
            for wake in [false,true] {
                let mut args=host_preflight_args(wake);args.push(selector.to_owned());
                let admitted=feedback_arguments(arguments_base()).try_get_matches_from(args).is_ok_and(|parsed|physical_host_preflight_selection(&parsed).is_ok_and(|selected|selected.is_some()));
                assert!(!admitted,"{selector}/{wake}");
            }
        }
        for complete in [physical_mvp_args(),background_args("persistent"),background_args("ntfy")] {
            for wake in [false,true] {
                let mut args=complete.clone();args.extend(host_preflight_args(wake).into_iter().skip(2));
                assert!(feedback_arguments(arguments_base()).try_get_matches_from(args)
                    .map_or(true,|parsed|physical_host_preflight_selection(&parsed).is_err()));
            }
        }
    }
    #[test]
    fn physical_host_preflight_early_frontend_routes_malformed_and_mixed_selectors_to_rejection() {
        for selector in ["--physical-host-preflight","--physical-host-preflight=true","--physical-host-preflight=false","--physical-host-preflight=garbage",
            "--physical-host-wake-preflight","--physical-host-wake-preflight=true","--physical-host-wake-preflight=false","--physical-host-wake-preflight=garbage"] {
            for shortcut in [vec!["--owned-mcp-stdio","/private/lease"],vec!["--review-fixture"],vec!["--terminal-only"],vec!["--project-read-only"]] {
                for first in [true,false] {
                    let mut args=vec!["smoke","--evidence-dir=/private/fresh"];
                    if selector.starts_with("--physical-host-wake-preflight"){args.push("--expected-controller-unit=goatr-host-wake-00000000-0000-4000-8000-000000000000.service");}
                    if first{args.push(selector);}args.extend(shortcut.iter().copied());if !first{args.push(selector);}
                    assert!(args.iter().skip(1).any(|argument|super::super::early_live_argument(std::ffi::OsStr::new(argument))));
                    let admitted=feedback_arguments(arguments_base()).try_get_matches_from(args).is_ok_and(|parsed|physical_host_preflight_selection(&parsed).is_ok_and(|selected|selected.is_some()));
                    assert!(!admitted);
                }
            }
        }
        for existing in ["--physical-mvp","--physical-mvp=false","--background=ntfy"] {
            assert!(super::super::early_live_argument(std::ffi::OsStr::new(existing)));
        }
        assert!(!super::super::early_live_argument(std::ffi::OsStr::new("--review-fixture")));
    }
    #[test]
    fn physical_mvp_cli_requires_all_frozen_inputs_and_accepts_both_backend_orders() {
        let args=physical_mvp_args();
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();
        physical_mode_environment(&parsed).unwrap();
        assert!(parsed.get_flag("physical-mvp"));assert!(!parsed.get_flag("automate-native-inputs"));
        assert_eq!(parsed.value_source("api"),Some(clap::parser::ValueSource::DefaultValue));
        for required in ["--goatr=","--config=","--workspace-root=","--evidence-dir=","--serial=","--apk=","--test-apk=",
            "--expected-goatr-sha256=","--expected-companion-lock-sha256=","--expected-apk-sha256=","--expected-test-apk-sha256=",
            "--expected-config-sha256=","--expected-ntfy-apk-sha256=","--expected-ntfy-signer-sha256=","--consent-owned-providers","--expected-controller-unit=","--expected-controller-sha256="] {
            let missing=args.iter().filter(|arg|!arg.starts_with(required)).cloned().collect::<Vec<_>>();
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(missing).is_err(),"{required}");
        }
        let mut reversed=args.clone();reversed.swap(2,3);
        physical_mode_environment(&feedback_arguments(arguments_base()).try_get_matches_from(reversed).unwrap()).unwrap();
        for providers in [vec!["--backend=omp"],vec!["--backend=omp","--backend=omp"],vec!["--backend=codex","--backend=omp","--backend=codex"]] {
            let mut invalid=args.iter().filter(|arg|!arg.starts_with("--backend=")).cloned().collect::<Vec<_>>();
            invalid.extend(providers.into_iter().map(str::to_owned));
            let parsed=feedback_arguments(arguments_base()).try_get_matches_from(invalid).unwrap();assert!(require_shared_backends(&parsed).is_err());
        }
    }
    #[test]
    fn retained_pairing_owned_retirement_requires_subject_and_manifest_pins_and_refuses_provider_inputs() {
        let mut args=retained_pairing_args();args[1]="--physical-owned-retirement".to_owned();args.push("--retirement-target-manifest=/private/retirement.json".to_owned());args.push(format!("--expected-retirement-manifest-sha256={}","5".repeat(64)));
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();physical_mode_environment(&parsed).unwrap();assert!(parsed.get_flag("physical-owned-retirement"));
        for required in ["--retirement-target-manifest=","--expected-retirement-manifest-sha256=","--expected-current-test-apk-sha256="] {let reduced=args.iter().filter(|arg|!arg.starts_with(required)).cloned().collect::<Vec<_>>();assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err(),"{required}");}
        for rejected in ["--backend=omp","--consent-owned-providers","--physical-mvp","--physical-pairing-checkpoint","--physical-pairing-shell-checkpoint","--physical-pairing-shell-interaction-checkpoint","--physical-host-preflight","--physical-host-wake-preflight","--background=persistent","--admission-only","--shared-agent-only","--terminal-only","--project-read-only","--grid","--grid-lease=/private/lease","--ui","--camera","--ime","--setup","--prepare-emulator","--emulator-owner=/private/owner","--paging-proof","--codex-invalidation-diagnostic","--omp-creation-diagnostic","--physical-hello-capture","--physical-relay-ephemeral","--development-continuity","--development-owner-feedback","--development-android-only","--keep-foreground-for-feedback","--defer-retention-crossing","--admin-socket=/private/admin.sock","--disposable-uid=12345","--automate-native-inputs","--expected-ntfy-apk-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","--expected-ntfy-signer-sha256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","--prepared-fresh-install=/private/install","--expected-prepared-install-sha256=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"] {let mut mixed=args.clone();mixed.push(rejected.to_owned());assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).is_err(),"{rejected}");}
    }
    #[test]
    fn owned_process_inventory_cli_is_inventory_only_and_refuses_every_effect_surface() {
        let args=owned_process_inventory_args();let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();physical_mode_environment(&parsed).unwrap();assert!(parsed.get_flag("physical-owned-process-inventory"));
        for required in ["--physical-owned-process-inventory","--owned-process-source-manifest=","--expected-owned-process-source-manifest-sha256=","--owned-app-retirement-gate=","--expected-owned-app-retirement-gate-sha256=","--expected-controller-unit=","--expected-controller-sha256=","--expected-companion-lock-sha256=","--evidence-dir="] {let reduced=args.iter().filter(|argument|!argument.starts_with(required)).cloned().collect::<Vec<_>>();assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err(),"{required}");}
        for rejected in ["--physical-mvp","--physical-pairing-checkpoint","--physical-owned-retirement","--physical-host-preflight","--physical-host-wake-preflight","--background=persistent","--terminal-only","--project-read-only","--backend=omp","--consent-owned-providers","--goatr=/private/goatr","--config=/private/config","--workspace-root=/private/workspace","--serial=device","--apk=/private/app","--test-apk=/private/test","--admin-socket=/private/admin","--automate-native-inputs"] {let mut mixed=args.clone();mixed.push(rejected.to_owned());assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()),"{rejected}");}
        for unknown in ["--owned-process-retirement-phase=inventory","--owned-process-retirement-sources=/private/sources","--expected-owned-process-retirement-sources-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","--completed-owned-retirement=/private/completed","--expected-completed-owned-retirement-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","--owned-process-retirement-approve=/private/review","--owned-process-retirement-pid=7"] {let mut mixed=args.clone();mixed.push(unknown.to_owned());assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).is_err(),"{unknown}");}
    }
    #[test]
    fn owned_process_stop_cli_requires_reviewed_pins_and_refuses_inventory_and_effect_inputs() {
        let args=owned_process_stop_args();let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();physical_mode_environment(&parsed).unwrap();assert!(parsed.get_flag("physical-owned-process-stop"));
        for required in ["--physical-owned-process-stop","--owned-process-stop-approval=","--expected-owned-process-stop-approval-sha256=","--owned-process-source-manifest=","--expected-owned-process-source-manifest-sha256=","--owned-app-retirement-gate=","--expected-owned-app-retirement-gate-sha256=","--expected-controller-unit=","--expected-controller-sha256=","--expected-companion-lock-sha256=","--evidence-dir="] {let reduced=args.iter().filter(|argument|!argument.starts_with(required)).cloned().collect::<Vec<_>>();assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err(),"{required}");}
        for rejected in ["--physical-owned-process-inventory","--physical-mvp","--physical-pairing-checkpoint","--physical-owned-retirement","--backend=omp","--consent-owned-providers","--goatr=/private/goatr","--config=/private/config","--serial=device","--admin-socket=/private/admin","--automate-native-inputs","--signal=TERM","--pid=7"] {let mut mixed=args.clone();mixed.push(rejected.to_owned());assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()),"{rejected}");}
        for unselected in ["--owned-process-source-manifest=/private/sources.json","--expected-owned-process-source-manifest-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","--owned-app-retirement-gate=/private/gate.json","--expected-owned-app-retirement-gate-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {assert!(feedback_arguments(arguments_base()).try_get_matches_from(["smoke",unselected]).is_err(),"{unselected}");}
    }

    #[test]
    fn retained_pairing_checkpoint_requires_current_subject_pins_and_rejects_other_runtime_modes() {
        let args=retained_pairing_args();let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();
        physical_mode_environment(&parsed).unwrap();assert!(parsed.get_flag("physical-pairing-checkpoint"));assert!(!parsed.get_flag("physical-mvp"));
        for required in ["--goatr=","--config=","--workspace-root=","--evidence-dir=","--serial=","--apk=","--test-apk=",
            "--expected-goatr-sha256=","--expected-companion-lock-sha256=","--expected-apk-sha256=","--expected-test-apk-sha256=","--expected-current-test-apk-sha256=",
            "--expected-product-uid=","--expected-product-first-install-time=","--expected-product-data-dir=","--expected-test-uid=","--expected-test-first-install-time=","--expected-test-data-dir=",
            "--expected-config-sha256=","--expected-controller-unit=","--expected-controller-sha256="] {
            let reduced=args.iter().filter(|arg|!arg.starts_with(required)).cloned().collect::<Vec<_>>();
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err(),"{required}");
        }
        assert_eq!(retained_pairing_provider_create(&parsed).unwrap(),None);
        let mut shell=args.clone();shell.push("--physical-pairing-shell-checkpoint".to_owned());
        let shell=feedback_arguments(arguments_base()).try_get_matches_from(shell).unwrap();physical_mode_environment(&shell).unwrap();assert_eq!(retained_pairing_provider_create(&shell).unwrap(),None);
        for rejected in [vec!["--backend=omp"],vec!["--backend=codex"],vec!["--consent-owned-providers"]] {
            let mut mixed=args.clone();mixed.push("--physical-pairing-shell-checkpoint".to_owned());mixed.extend(rejected.into_iter().map(str::to_owned));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()));
        }
        for (backend,expected) in [("omp",Provider::Omp),("codex",Provider::Codex)] {
            let mut optional=args.clone();optional.extend([format!("--backend={backend}"),"--consent-owned-providers".to_owned()]);
            let optional=feedback_arguments(arguments_base()).try_get_matches_from(optional).unwrap();physical_mode_environment(&optional).unwrap();assert_eq!(retained_pairing_provider_create(&optional).unwrap(),Some(expected));
        }
        for rejected in [vec!["--backend=omp"],vec!["--backend=codex"],vec!["--consent-owned-providers"],vec!["--backend=omp","--backend=omp","--consent-owned-providers"],vec!["--backend=codex","--backend=codex","--consent-owned-providers"],vec!["--backend=omp","--backend=codex","--consent-owned-providers"]] {
            let mut mixed=args.clone();mixed.extend(rejected.into_iter().map(str::to_owned));assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()));
        }
        for conflict in ["--physical-mvp","--background=persistent","--prepare-emulator","--emulator-owner=/private/owner.json",
            "--development-continuity","--physical-host-preflight","--physical-host-wake-preflight","--automate-native-inputs",
            "--expected-ntfy-apk-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","--expected-ntfy-signer-sha256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"] {
            let mut mixed=args.clone();mixed.push(conflict.to_owned());assert!(feedback_arguments(arguments_base()).try_get_matches_from(mixed).is_err(),"{conflict}");
        }
        for invalid in [String::new(),"A".repeat(64),"a".repeat(63),"g".repeat(64)] {
            let mut changed=args.iter().filter(|arg|!arg.starts_with("--expected-controller-sha256=")).cloned().collect::<Vec<_>>();
            changed.push(format!("--expected-controller-sha256={invalid}"));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(changed).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()));
        }
        for (pin,value) in [("--expected-current-test-apk-sha256=","g".repeat(64)),("--expected-product-uid=","9".to_owned()),("--expected-product-first-install-time=",String::new()),("--expected-product-data-dir=","/private/product".to_owned()),("--expected-test-uid=","0".to_owned()),("--expected-test-first-install-time=",String::new()),("--expected-test-data-dir=","/private/test".to_owned())] {
            let mut changed=args.iter().filter(|arg|!arg.starts_with(pin)).cloned().collect::<Vec<_>>();changed.push(format!("{pin}{value}"));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(changed).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()),"{pin}");
        }
    }
    #[test]
    fn retained_pairing_shell_interaction_checkpoint_requires_shell_parent_and_refuses_provider_inputs() {
        let args=retained_pairing_args();
        let mut valid=args.clone();valid.push("--physical-pairing-shell-checkpoint".to_owned());valid.push("--physical-pairing-shell-interaction-checkpoint".to_owned());
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(valid).unwrap();
        assert!(parsed.get_flag("physical-pairing-checkpoint"));assert!(parsed.get_flag("physical-pairing-shell-checkpoint"));assert!(parsed.get_flag("physical-pairing-shell-interaction-checkpoint"));
        assert_eq!(retained_pairing_provider_create(&parsed).unwrap(),None);
        for rejected in [vec!["--backend=omp"],vec!["--backend=codex"],vec!["--consent-owned-providers"],vec!["--backend=omp","--consent-owned-providers"]] {
            let mut invalid=args.clone();invalid.push("--physical-pairing-shell-checkpoint".to_owned());invalid.push("--physical-pairing-shell-interaction-checkpoint".to_owned());invalid.extend(rejected.into_iter().map(str::to_owned));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(invalid).is_err());
        }
        assert!(feedback_arguments(arguments_base()).try_get_matches_from(["smoke","--physical-pairing-shell-interaction-checkpoint"]).is_err());
    }
    #[test]
    fn retained_pairing_shell_checkpoint_requires_parent_and_refuses_provider_inputs() {
        let args=retained_pairing_args();
        let mut shell=args.clone();shell.push("--physical-pairing-shell-checkpoint".to_owned());
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(shell).unwrap();
        assert!(parsed.get_flag("physical-pairing-checkpoint"));assert!(parsed.get_flag("physical-pairing-shell-checkpoint"));
        assert_eq!(retained_pairing_provider_create(&parsed).unwrap(),None);
        for rejected in [vec!["--backend=omp"],vec!["--backend=codex"],vec!["--consent-owned-providers"],vec!["--backend=omp","--consent-owned-providers"]] {
            let mut invalid=args.clone();invalid.push("--physical-pairing-shell-checkpoint".to_owned());invalid.extend(rejected.into_iter().map(str::to_owned));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(invalid).is_err());
        }
        let orphan=feedback_arguments(arguments_base()).try_get_matches_from(["smoke","--physical-pairing-shell-checkpoint"]);
        assert!(orphan.is_err());
    }
    #[test]
    fn prepared_install_cli_is_an_explicit_physical_only_pair_not_an_adoption_default() {
        let base=physical_mvp_args();let mut valid=base.clone();valid.extend(["--prepared-fresh-install=/private/creator.json".to_owned(),format!("--expected-prepared-install-sha256={}","a".repeat(64))]);
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&valid).unwrap();physical_mode_environment(&parsed).unwrap();
        assert!(parsed.get_one::<PathBuf>("prepared-fresh-install").is_some());
        let absent=feedback_arguments(arguments_base()).try_get_matches_from(&base).unwrap();physical_mode_environment(&absent).unwrap();assert!(absent.get_one::<PathBuf>("prepared-fresh-install").is_none());
        for extra in [vec!["--prepared-fresh-install=/private/creator.json".to_owned()],vec![format!("--expected-prepared-install-sha256={}","a".repeat(64))],
            vec!["--prepared-fresh-install=/private/creator.json".to_owned(),format!("--expected-prepared-install-sha256={}","A".repeat(64))],
            vec!["--prepared-fresh-install=/private/creator.json".to_owned(),"--expected-prepared-install-sha256=short".to_owned()]] {
            let mut bad=base.clone();bad.extend(extra);assert!(feedback_arguments(arguments_base()).try_get_matches_from(bad).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()));
        }
        for mut other in [host_preflight_args(false),host_preflight_args(true),background_args("persistent"),background_args("ntfy"),
            vec!["smoke".to_owned(),"--evidence-dir=/private/fresh".to_owned()]] {
            other.extend(["--prepared-fresh-install=/private/creator.json".to_owned(),format!("--expected-prepared-install-sha256={}","a".repeat(64))]);
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(other).map_or(true,|parsed| {
                match physical_host_preflight_selection(&parsed){Err(_)=>true,Ok(Some(_))=>false,Ok(None)=>physical_mode_environment(&parsed).is_err()}
            }));
        }
        for selector in ["--prepared-fresh-install=/private/creator.json","--expected-prepared-install-sha256=bad"] {
            for shortcut in ["--owned-mcp-stdio","--review-fixture","--terminal-only"] {
                let args=["smoke",shortcut,selector];assert!(args.iter().any(|arg|crate::early_live_argument(std::ffi::OsStr::new(arg))));
                assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).is_err());
            }
        }
    }
    #[test]
    fn full_physical_unit_wake_requires_independent_controller_pin_and_rejects_all_other_modes() {
        let valid=physical_mvp_args();physical_mode_environment(&feedback_arguments(arguments_base()).try_get_matches_from(&valid).unwrap()).unwrap();
        for value in ["".to_owned(),"A".repeat(64),"a".repeat(63),"g".repeat(64)] {
            let mut args=valid.iter().filter(|argument|!argument.starts_with("--expected-controller-sha256=")).cloned().collect::<Vec<_>>();
            args.push(format!("--expected-controller-sha256={value}"));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).map_or(true,|parsed|physical_mode_environment(&parsed).is_err()));
        }
        for mut args in [host_preflight_args(false),host_preflight_args(true),background_args("persistent"),background_args("ntfy"),vec!["smoke".to_owned(),"--evidence-dir=/private/fresh".to_owned()]] {
            args.push(format!("--expected-controller-sha256={}","4".repeat(64)));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).map_or(true,|parsed| {
                match physical_host_preflight_selection(&parsed) {Err(_)=>true,Ok(Some(_))=>false,Ok(None)=>physical_mode_environment(&parsed).is_err()}
            }));
        }
        for selector in ["--expected-controller-unit=malformed","--expected-controller-sha256=bad"] {
            let args=["smoke","--owned-mcp-stdio","/private/lease",selector];
            assert!(args.iter().any(|argument|crate::early_live_argument(std::ffi::OsStr::new(argument))));
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).is_err());
        }
    }
    #[test]
    fn physical_mvp_rejects_removed_old_product_audit_interface() {
        for removed in ["--old-product-audit=/private/obsolete.json","--expected-old-product-audit-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {
            let mut args=physical_mvp_args();args.push(removed.to_owned());
            assert_eq!(feedback_arguments(arguments_base()).try_get_matches_from(args).unwrap_err().kind(),clap::error::ErrorKind::UnknownArgument);
        }
    }
    #[test]
    fn physical_mvp_seed_pin_accepts_independent_lowercase_claims_but_rejects_invalid_and_unrelated_inputs() {
        for pin in ["1".repeat(64),"abcdef0123456789".repeat(4)] {
            let mut args=physical_mvp_args();args.retain(|arg|!arg.starts_with("--expected-config-sha256="));args.push(format!("--expected-config-sha256={pin}"));
            let parsed=feedback_arguments(arguments_base()).try_get_matches_from(args).unwrap();physical_mode_environment(&parsed).unwrap();
        }
        for pin in [String::new(),"a".repeat(63),"a".repeat(65),"A".repeat(64),"g".repeat(64)] {
            let mut args=physical_mvp_args();args.retain(|arg|!arg.starts_with("--expected-config-sha256="));args.push(format!("--expected-config-sha256={pin}"));
            let admitted=match feedback_arguments(arguments_base()).try_get_matches_from(args) {
                Ok(parsed)=>physical_mode_environment(&parsed).is_ok(),
                Err(_)=>false,
            };
            assert!(!admitted);
        }
        let missing=physical_mvp_args().into_iter().filter(|arg|!arg.starts_with("--expected-config-sha256=")).collect::<Vec<_>>();
        assert_eq!(feedback_arguments(arguments_base()).try_get_matches_from(missing).unwrap_err().kind(),clap::error::ErrorKind::MissingRequiredArgument);
        for mut unrelated in [vec!["smoke".to_owned()],background_args("persistent"),background_args("ntfy")] {
            unrelated.push(format!("--expected-config-sha256={}","1".repeat(64)));
            let admitted=match feedback_arguments(arguments_base()).try_get_matches_from(unrelated) {
                Ok(parsed)=>physical_mode_environment(&parsed).is_ok(),
                Err(_)=>false,
            };
            assert!(!admitted);
        }
    }
    #[test]
    fn physical_mvp_rejects_explicit_api30_and_explicit_default34_by_value_source() {
        for api in ["30","34"] {
            let mut args=physical_mvp_args();args.push(format!("--api={api}"));
            let parsed=feedback_arguments(arguments_base()).try_get_matches_from(args).unwrap();
            assert_eq!(parsed.value_source("api"),Some(clap::parser::ValueSource::CommandLine));
            let error=physical_mode_environment(&parsed).unwrap_err();
            assert!(error.to_string().contains("explicit --api is forbidden"));
        }
    }
    #[test]
    fn physical_mvp_rejects_all_legacy_owners_private_native_modes_and_direct_input() {
        for selector in ["--background=persistent","--admission-only","--shared-agent-only","--terminal-only","--project-read-only",
            "--grid","--grid-lease=/private/lease.json","--ui","--camera","--ime","--setup","--prepare-emulator",
            "--emulator-owner=/private/emulator.json","--paging-proof","--codex-invalidation-diagnostic","--omp-creation-diagnostic",
            "--physical-hello-capture","--physical-relay-ephemeral","--development-continuity","--development-owner-feedback",
            "--development-android-only","--keep-foreground-for-feedback","--defer-retention-crossing",
            "--admin-socket=/private/admin.sock","--disposable-uid=1901","--automate-native-inputs"] {
            let mut args=physical_mvp_args();args.push(selector.into());
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).is_err(),"{selector}");
        }
    }
    #[test]
    fn physical_mvp_requires_exact_official_ntfy_pins_without_leaking_them_to_other_modes() {
        let apk="b4baa6f668bd57ad5df3b4e0e769165dcf25c5ee19fdb04f78077ee819616f2a";
        let signer="76e94d8e04706c5b99064c23210d8ca9910608abae8f3e972b41e388391646e0";
        // Former upstream release and actual older installed F-Droid artifact are reject inputs only.
        let upstream_apk="8a1075c0963caa1e0dc404996531bc641cb2b982f92a4bc692d4999f9eb53f39";
        let upstream_signer="6e145d7ae685eff75468e5067e03a6c3645453343e4e181dac8b6b17ff67489d";
        let older_fdroid_apk="be6ea2f315fa86bc9c7d977dece15882e9db4a8fc12d6dd2fe7f655540a3ec43";
        let arbitrary="0".repeat(64);
        for physical in [true,false] {
            let mut args=if physical{physical_mvp_args()}else{background_args("ntfy")};
            args.retain(|arg|!arg.starts_with("--expected-ntfy-"));
            args.extend([format!("--expected-ntfy-apk-sha256={apk}"),format!("--expected-ntfy-signer-sha256={signer}")]);
            let check:fn(&clap::ArgMatches)->io::Result<()>=if physical{physical_mode_environment}else{background_mode_environment};
            check(&feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap()).unwrap();
            for (candidate_apk,candidate_signer) in [
                (upstream_apk,upstream_signer),(apk,upstream_signer),(upstream_apk,signer),(older_fdroid_apk,signer),
                (arbitrary.as_str(),signer),(apk,arbitrary.as_str()),("not-a-sha",signer),(apk,"not-a-sha"),(signer,apk),
            ] {
                let mut rejected=args.clone();rejected.retain(|arg|!arg.starts_with("--expected-ntfy-"));
                rejected.extend([format!("--expected-ntfy-apk-sha256={candidate_apk}"),format!("--expected-ntfy-signer-sha256={candidate_signer}")]);
                let parsed=feedback_arguments(arguments_base()).try_get_matches_from(rejected).unwrap();
                assert!(check(&parsed).is_err(),"physical={physical} must reject a nonfixture or mixed identity");
            }
            for missing in ["--expected-ntfy-apk-sha256=","--expected-ntfy-signer-sha256="] {
                let mut rejected=args.clone();rejected.retain(|arg|!arg.starts_with(missing));
                match feedback_arguments(arguments_base()).try_get_matches_from(rejected) {
                    Ok(parsed)=>assert!(check(&parsed).is_err()),
                    Err(error)=>assert_eq!(error.kind(),clap::error::ErrorKind::MissingRequiredArgument),
                }
            }
        }
        for (candidate_apk,candidate_signer) in [(apk,signer),(upstream_apk,upstream_signer),(apk,upstream_signer)] {
            let pins=[format!("--expected-ntfy-apk-sha256={candidate_apk}"),format!("--expected-ntfy-signer-sha256={candidate_signer}")];
            for supplied in [vec![pins[0].clone()],vec![pins[1].clone()],pins.to_vec()] {
                let mut unrelated=vec!["smoke".to_owned()];unrelated.extend(supplied.clone());
                assert!(feedback_arguments(arguments_base()).try_get_matches_from(unrelated).is_err());
                let mut persistent=background_args("persistent");persistent.extend(supplied);
                let parsed=feedback_arguments(arguments_base()).try_get_matches_from(persistent).unwrap();
                assert!(background_mode_environment(&parsed).is_err());
            }
        }
    }
    #[test]
    fn finite_background_parser_accepts_both_explicit_modes_and_refuses_effectful_mixed_routes() {
        for mode in ["persistent","ntfy"] {
            let args=background_args(mode);
            let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();
            assert_eq!(parsed.get_one::<String>("background").map(String::as_str),Some(mode));
            background_mode_environment(&parsed).unwrap();
            for required in ["--goatr=","--config=","--workspace-root=","--evidence-dir=","--emulator-owner=","--serial=","--apk=","--test-apk=","--expected-apk-sha256=","--expected-test-apk-sha256="] {
                let reduced=args.iter().filter(|arg|!arg.starts_with(required)).cloned().collect::<Vec<_>>();
                assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err());
            }
            for conflict in [
                "--review-fixture",
                "--transport=direct",
                "--transport=relay",
                "--owned-mcp-stdio",
                "--admission-only",
                "--shared-agent-only",
                "--terminal-only",
                "--project-read-only",
                "--grid",
                "--ui",
                "--camera",
                "--ime",
                "--setup",
                "--prepare-emulator",
                "--paging-proof",
                "--physical-hello-capture",
                "--physical-relay-ephemeral",
                "--codex-invalidation-diagnostic",
                "--omp-creation-diagnostic",
                "--development-continuity",
                "--development-owner-feedback",
                "--development-android-only",
                "--keep-foreground-for-feedback",
                "--defer-retention-crossing",
            ] {
                let mut mixed = args.clone();
                mixed.push(conflict.into());
                assert!(
                    feedback_arguments(arguments_base())
                        .try_get_matches_from(mixed)
                        .is_err()
                );
            }
            for backends in [vec!["--backend=omp"],vec!["--backend=codex"],vec!["--backend=omp","--backend=omp"]] {
                let mut invalid=args.iter().filter(|arg|!arg.starts_with("--backend=")).cloned().collect::<Vec<_>>();
                invalid.extend(backends.into_iter().map(str::to_owned));
                let parsed=feedback_arguments(arguments_base()).try_get_matches_from(invalid).unwrap();
                assert!(background_mode_environment(&parsed).is_err());
            }
        }
        let missing=background_args("ntfy").into_iter().filter(|arg|!arg.starts_with("--expected-ntfy-")).collect::<Vec<_>>();
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(missing).unwrap();
        assert!(background_mode_environment(&parsed).is_err());
        let mut unwanted=background_args("persistent");unwanted.extend(["--expected-ntfy-apk-sha256=bad".into(),"--expected-ntfy-signer-sha256=bad".into()]);
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(unwanted).unwrap();
        assert!(background_mode_environment(&parsed).is_err());
    }

    fn project_read_args()->Vec<String> {
        ["smoke","--project-read-only","--backend","omp","--backend","codex","--goatr","/nix/store/example/bin/goatr","--config","/private/config.toml","--workspace-root","/workspace","--evidence-dir","/private/fresh","--consent-owned-providers"].into_iter().map(str::to_owned).collect()
    }
    #[test]
    fn project_read_cli_requires_both_original_backends_and_explicit_fresh_owned_scope_without_input() {
        let args=project_read_args();let matches=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();require_shared_backends(&matches).unwrap();assert!(!matches.get_flag("automate-native-inputs"));assert!(ProjectSeedPin::parse(&matches).unwrap().is_none());
        for required in ["--goatr","--config","--workspace-root","--evidence-dir","--consent-owned-providers"] {
            let mut reduced=args.clone();let index=reduced.iter().position(|arg|arg==required).unwrap();reduced.drain(index..index+if required=="--consent-owned-providers"{1}else{2});
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err());
        }
        let mut single=args.clone();let index=single.iter().position(|arg|arg=="codex").unwrap();single.drain(index-1..=index);
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(single).unwrap();assert!(require_shared_backends(&parsed).is_err());
        let mut duplicate=args.clone();*duplicate.iter_mut().find(|arg|arg.as_str()=="codex").unwrap()="omp".into();let parsed=feedback_arguments(arguments_base()).try_get_matches_from(duplicate).unwrap();assert!(require_shared_backends(&parsed).is_err());
        for conflict in ["--admission-only","--shared-agent-only","--terminal-only","--grid","--ui","--camera","--ime","--setup","--prepare-emulator","--paging-proof","--automate-native-inputs","--codex-invalidation-diagnostic","--omp-creation-diagnostic","--physical-hello-capture","--physical-relay-ephemeral","--development-continuity","--development-owner-feedback","--development-android-only","--keep-foreground-for-feedback","--defer-retention-crossing"] {
            let mut args=args.clone();args.push(conflict.into());assert!(feedback_arguments(arguments_base()).try_get_matches_from(args).is_err());
        }
    }
    #[test]
    fn project_read_seed_pins_are_complete_and_never_claim_signature_verification() {
        let mut args=project_read_args();args.extend(["--project-read-seed-pack".into(),"/private/existing-public.pack".into()]);assert!(feedback_arguments(arguments_base()).try_get_matches_from(&args).is_err());
        args.extend(["--project-read-seed-pack-sha256".into(),"1".repeat(64),"--project-read-seed-commit".into(),PROJECT_PUBLIC_COMMIT.into(),"--project-read-seed-tree".into(),PROJECT_PUBLIC_TREE.into(),"--project-read-seed-signature-sha256".into(),"2".repeat(64)]);
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();assert!(ProjectSeedPin::parse(&parsed).unwrap().is_some());
        let index=args.iter().position(|arg|arg=="--project-read-seed-commit").unwrap()+1;args[index]="3".repeat(40);
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(args).unwrap();assert!(ProjectSeedPin::parse(&parsed).is_err());
        // Byte-field parsing is deliberately NOT a fake cryptographic verifier.
        let signature="-----BEGIN PGP SIGNATURE-----\npublic parser fixture\n-----END PGP SIGNATURE-----";
        let commit=format!("tree {PROJECT_PUBLIC_TREE}\nauthor public fixture\ngpgsig {}\n\npublic subject\n",signature.replace('\n',"\n "));
        let (tree,block)=project_commit_fields(commit.as_bytes()).unwrap();assert_eq!(tree,PROJECT_PUBLIC_TREE);assert_eq!(block,signature);
        assert!(project_commit_fields(b"tree missing\n\nunsigned subject\n").is_err());
    }
    #[test]
    fn project_read_input_type_alias_mode_and_byte_bounds_are_checked_before_read() {
        struct Fixture(PathBuf);impl Drop for Fixture{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
        let parent=fs::canonicalize(std::env::temp_dir()).unwrap();let fixture=Fixture(parent.join(format!("goatr-public-seed-input-{}",Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&fixture.0).unwrap();let path=fixture.0.join("existing-public.pack");fs::write(&path,b"public immutable bytes").unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(project_input(&path,64,true).unwrap(),b"public immutable bytes");assert!(project_input(&path,4,true).is_err());
        let alias=fixture.0.join("alias");std::os::unix::fs::symlink(&path,&alias).unwrap();assert!(project_input(&alias,64,true).is_err());
        fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();assert!(project_input(&path,64,true).is_err());
        let fifo=fixture.0.join("fifo");let name=std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o400)},0);assert!(project_input(&fifo,64,true).is_err());
    }
    #[test]
    fn project_read_seed_tree_requires_real_distinct_regular_rows_without_path_escape() {
        let valid=format!("100644 blob {}\tfile-a\x00100644 blob {}\tdir/file-b\x00100644 blob {}\tfile-c\x00","1".repeat(40),"2".repeat(40),"3".repeat(40));
        let rows=project_seed_files(valid.as_bytes()).unwrap();assert_eq!(rows.len(),3);
        for invalid in [valid.replace("file-a","../outside"),valid.replace("100644","120000"),valid.replace("file-a","bad\nname"),valid.trim_end_matches('\0').to_owned()] {assert!(project_seed_files(invalid.as_bytes()).is_err());}
    }


    fn terminal_args()->Vec<&'static str> {
        vec!["smoke","--terminal-only","--goatr","/nix/store/example/bin/goatr",
            "--config","/private/config.toml","--workspace-root","/workspace",
            "--evidence-dir","/private/fresh","--consent-owned-providers","--automate-native-inputs"]
    }
    #[test]
    fn terminal_cli_requires_owned_fixture_consents_rejects_backends_and_pairs_frozen_hashes() {
        let args=terminal_args();
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();
        assert!(!terminal_frozen_selection(&parsed).unwrap());
        for required in ["--goatr","--config","--workspace-root","--evidence-dir","--consent-owned-providers","--automate-native-inputs"] {
            let mut reduced=args.clone();let index=reduced.iter().position(|arg|*arg==required).unwrap();
            reduced.drain(index..index+if matches!(required,"--consent-owned-providers"|"--automate-native-inputs"){1}else{2});
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err(),"{required}");
        }
        for backend in ["omp","codex"]{let mut conflict=args.clone();conflict.extend(["--backend",backend]);assert!(feedback_arguments(arguments_base()).try_get_matches_from(conflict).is_err());}
        for mode in [
            "--admission-only",
            "--shared-agent-only",
            "--grid",
            "--ui",
            "--camera",
            "--ime",
            "--setup",
            "--prepare-emulator",
            "--paging-proof",
            "--codex-invalidation-diagnostic",
            "--omp-creation-diagnostic",
            "--physical-hello-capture",
            "--physical-relay-ephemeral",
            "--development-continuity",
            "--development-owner-feedback",
            "--development-android-only",
            "--keep-foreground-for-feedback",
            "--defer-retention-crossing",
        ] {
            let mut conflict = args.clone();
            conflict.push(mode);
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(conflict)
                    .is_err(),
                "{mode}"
            );
        }
        let mut api=args.clone();api.extend(["--api","35"]);assert!(feedback_arguments(arguments_base()).try_get_matches_from(api).is_err());
        for flag in ["--expected-goatr-sha256","--expected-companion-lock-sha256"] {
            let mut incomplete=args.clone();incomplete.extend([flag,"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
            if flag=="--expected-goatr-sha256" {assert!(feedback_arguments(arguments_base()).try_get_matches_from(incomplete).is_err());}else{let parsed=feedback_arguments(arguments_base()).try_get_matches_from(incomplete).unwrap();assert!(terminal_mode_environment(&parsed).is_err());}
        }
        let mut frozen=args;frozen.extend(["--expected-goatr-sha256","aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--expected-companion-lock-sha256","bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]);
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(frozen).unwrap();
        assert!(terminal_frozen_selection(&parsed).unwrap());
    }
    fn terminal_test_cells()->Vec<GridCell> {
        fn cell(text:&str,width:u64,foreground:GridColor)->GridCell {
            GridCell{text:text.into(),width:Uint(width),foreground,background:GridColor::Default,
                attributes:GridAttributes{bold:false,faint:false,italic:false,blink:false,strike:false,inverse:false,concealed:false,underline:GridUnderline::None,underline_color:None}}
        }
        let mut cells=TERMINAL_MARKER.chars().map(|value|cell(&value.to_string(),1,GridColor::Default)).collect::<Vec<_>>();
        for text in ["界","ｶﾞ","👩‍💻","❤️","🇺🇸"] {cells.push(cell(text,2,GridColor::Default));cells.push(cell("",0,GridColor::Default));}
        for (text,color) in [("R",GridColor::Indexed{index:Uint(1)}),("I",GridColor::Indexed{index:Uint(202)}),("T",GridColor::Rgb{value:Uint(0x112233)}),("D",GridColor::Default)]{cells.push(cell(text,1,color));}
        cells
    }
    fn terminal_test_witness(columns:usize)->(GridWitness,Uuid) {
        let opening=Uuid::random();let subscription=Uuid::random();let target=ShellTarget{shell_id:Uuid::random(),generation:Uuid::random()};
        let mut witness=GridWitness::pending(opening.clone(),GridSubject::Shell(target.clone()));witness.fixture=Some(TerminalFixtureWitness::new());
        witness.response(&opening,&Ok(ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{target,subscription:subscription.clone(),seq:ZeroValue(0),
            frame:GridBegin { scroll: None, mode:GridMode::Replace,revision:Uint(0),columns:Uint(columns as u64),rows:Uint(1),cursor:None,
                alternate_screen:false,graphics_omitted:false,popup_blocked:false,part_count:Uint(2) }})),128).unwrap();
        (witness,subscription)
    }
    #[test]
    fn grid_transaction_clock_starts_at_each_accepted_descriptor_and_never_renews_on_progress() {
        fn route(grid:&mut Option<GridWitness>,events:&mut u64,generation:&mut Option<GenerationView>,event:Event)->io::Result<()> {
            let envelope=Envelope::Event(event);
            let bytes=goatr::protocol::encode_payload(&envelope).unwrap().len();
            let Envelope::Event(event)=envelope else {unreachable!()};
            Phone::route_event(grid,events,generation,event,bytes)
        }
        for duplicate in [true,false] {
            let opening=Uuid::random();let subscription=Uuid::random();
            let target=ShellTarget{shell_id:Uuid::random(),generation:Uuid::random()};
            let mut witness=GridWitness::pending(opening.clone(),GridSubject::Shell(target.clone()));
            witness.fixture=Some(TerminalFixtureWitness::new());
            assert!(witness.deadline.is_none());witness.arm("TransactionA").unwrap();assert!(witness.deadline.is_none());
            let mut cells=terminal_test_cells();let columns=cells.len();let first=cells[0].clone();
            let split=TERMINAL_MARKER.len()+1;let tail=cells.split_off(split);
            let mut frame=GridBegin { scroll: None, mode:GridMode::Replace,revision:Uint(0),columns:Uint(columns as u64),rows:Uint(1),cursor:None,
                alternate_screen:false,graphics_omitted:false,popup_blocked:false,part_count:Uint(2) };
            let envelope=Envelope::Response(Response{id:opening,outcome:Ok(ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{
                target,subscription:subscription.clone(),seq:ZeroValue(0),frame:frame.clone()}))});
            let bytes=goatr::protocol::encode_payload(&envelope).unwrap().len();
            let Envelope::Response(response)=envelope else {unreachable!()};
            let before=Instant::now();witness.response(&response.id,&response.outcome,bytes).unwrap();let after=Instant::now();
            let initial=witness.deadline.unwrap();
            assert!(initial>=before+Duration::from_secs(15)&&initial<=after+Duration::from_secs(15));
            let mut grid=Some(witness);let mut events=0;let mut generation=None;
            let part=|sequence,revision,index,column,cells|Event{subscription:subscription.clone(),seq:Uint(sequence),
                body:EventBody::GridPart(GridPart{revision:Uint(revision),index:Uint(index),spans:vec![GridSpan{row:Uint(0),column:Uint(column),cells}]})};
            route(&mut grid,&mut events,&mut generation,part(1,0,0,0,cells)).unwrap();
            assert_eq!(grid.as_ref().unwrap().deadline,Some(initial));assert!(grid.as_ref().unwrap().committed.is_none());
            grid.as_mut().unwrap().arm("TransactionB").unwrap();assert_eq!(grid.as_ref().unwrap().deadline,Some(initial));
            route(&mut grid,&mut events,&mut generation,part(2,0,1,split as u64,tail)).unwrap();
            assert_eq!(grid.as_ref().unwrap().deadline,Some(initial));
            assert!(grid.as_ref().unwrap().fixture.as_ref().unwrap().committed.is_none());
            route(&mut grid,&mut events,&mut generation,Event{subscription:subscription.clone(),seq:Uint(3),
                body:EventBody::GridCommit(GridCommit{revision:Uint(0)})}).unwrap();
            assert_eq!(grid.as_ref().unwrap().committed,Some(0));assert!(grid.as_ref().unwrap().deadline.is_none());
            assert!(grid.as_ref().unwrap().fixture.as_ref().unwrap().committed.is_some());
            frame.revision=Uint(1);
            let next=Event{subscription:subscription.clone(),seq:Uint(4),body:EventBody::GridBegin(GridBeginEvent{frame:frame.clone()})};
            let before=Instant::now();route(&mut grid,&mut events,&mut generation,next).unwrap();let after=Instant::now();
            let deadline=grid.as_ref().unwrap().deadline.unwrap();
            assert!(deadline>=before+Duration::from_secs(15)&&deadline<=after+Duration::from_secs(15));
            let unchanged=if duplicate {
                let duplicate=Event{subscription:subscription.clone(),seq:Uint(5),body:EventBody::GridBegin(GridBeginEvent{frame})};
                assert!(route(&mut grid,&mut events,&mut generation,duplicate).is_err());deadline
            }else{
                let expired=Instant::now();grid.as_mut().unwrap().deadline=Some(expired);
                let error=route(&mut grid,&mut events,&mut generation,part(5,1,0,0,vec![first])).unwrap_err();
                assert!(error.to_string().contains("phase=transaction_deadline"));expired
            };
            let witness=grid.as_ref().unwrap();
            assert!(witness.retired);assert_eq!(witness.deadline,Some(unchanged));assert_eq!(witness.committed,Some(0));
            assert!(witness.token_commit.is_none());assert_eq!(events,0);assert!(generation.is_none());
        }
    }
    #[test]
    fn terminal_fixture_facts_require_complete_selected_coverage_and_cross_part_wide_tails() {
        let cells=terminal_test_cells();let (mut witness,subscription)=terminal_test_witness(cells.len());let split=TERMINAL_MARKER.len()+1;
        let part=|sequence,index,column,cells|Event{subscription:subscription.clone(),seq:Uint(sequence),body:EventBody::GridPart(GridPart{revision:Uint(0),index:Uint(index),spans:vec![GridSpan{row:Uint(0),column:Uint(column),cells}]})};
        witness.event(part(1,0,0,cells[..split].to_vec()),256).unwrap();
        assert!(witness.fixture.as_ref().unwrap().committed.is_none());
        witness.event(part(2,1,split as u64,cells[split..].to_vec()),256).unwrap();
        witness.event(Event{subscription:subscription.clone(),seq:Uint(3),body:EventBody::GridCommit(GridCommit{revision:Uint(0)})},32).unwrap();
        assert!(witness.fixture.as_ref().unwrap().committed.unwrap().complete());
        assert_eq!(witness.committed_dimensions,Some((cells.len() as u64,1)));
        let (mut wrong,subscription)=terminal_test_witness(cells.len());
        assert!(wrong.event(Event{subscription,seq:Uint(1),body:EventBody::GridPart(GridPart{revision:Uint(0),index:Uint(0),spans:vec![GridSpan{row:Uint(0),column:Uint(1),cells:cells.clone()}]})},256).is_err());
        assert!(wrong.retired&&wrong.committed.is_none());
        let (mut wrong,subscription)=terminal_test_witness(cells.len());
        let mut malformed=cells;malformed[TERMINAL_MARKER.len()+1].text="not a tail".into();malformed[TERMINAL_MARKER.len()+1].width=Uint(1);
        assert!(wrong.event(Event{subscription,seq:Uint(1),body:EventBody::GridPart(GridPart{revision:Uint(0),index:Uint(0),spans:vec![GridSpan{row:Uint(0),column:Uint(0),cells:malformed}]})},256).is_err());
        assert!(wrong.retired&&wrong.committed.is_none());
    }
    #[test]
    fn terminal_idle_snapshot_refuses_changed_foreground_group_or_membership() {
        let mut info:herdr::PaneProcessInfo=serde_json::from_value(json!({"pane_id":"p1","shell_pid":17,
            "foreground_process_group_id":17,"foreground_processes":[{"pid":17,"name":"zsh"}]})).unwrap();
        assert_eq!(terminal_idle_pid(&info),Some(17));
        info.foreground_process_group_id=Some(18);assert_eq!(terminal_idle_pid(&info),None);
        info.foreground_process_group_id=Some(17);
        let mut child=info.foreground_processes[0].clone();child.pid=18;info.foreground_processes.push(child);
        assert_eq!(terminal_idle_pid(&info),None);
        info.foreground_processes.clear();assert_eq!(terminal_idle_pid(&info),None);
        info.shell_pid=None;assert_eq!(terminal_idle_pid(&info),None);
    }
    #[test]
    fn terminal_delayed_marker_requires_absent_opening_and_later_complete_commit() {
        let cells=|present:bool| {
            let mut cells=terminal_test_cells();
            for value in TERMINAL_DELAYED.chars() {
                let mut cell=cells[0].clone();cell.text=if present{value.to_string()}else{" ".into()};cells.push(cell);
            }
            cells
        };
        for opening_contains_marker in [true,false] {
            let opening_cells=cells(opening_contains_marker);let later_cells=cells(true);
            let (mut witness,subscription)=terminal_test_witness(opening_cells.len());witness.arm(TERMINAL_DELAYED).unwrap();
            let split=TERMINAL_MARKER.len()+1;
            let part=|sequence,revision,index,column,cells|Event{subscription:subscription.clone(),seq:Uint(sequence),
                body:EventBody::GridPart(GridPart{revision:Uint(revision),index:Uint(index),spans:vec![GridSpan{row:Uint(0),column:Uint(column),cells}]})};
            witness.event(part(1,0,0,0,opening_cells[..split].to_vec()),256).unwrap();
            witness.event(part(2,0,1,split as u64,opening_cells[split..].to_vec()),256).unwrap();
            assert!(witness.fixture.as_ref().unwrap().opening.is_none());
            assert!(witness.fixture.as_ref().unwrap().committed.is_none());
            assert!(witness.terminal_opening_commit().is_err());
            witness.event(Event{subscription:subscription.clone(),seq:Uint(3),body:EventBody::GridCommit(GridCommit{revision:Uint(0)})},32).unwrap();
            assert_eq!(witness.fixture.as_ref().unwrap().opening,Some((0,opening_contains_marker)));
            if opening_contains_marker {
                assert!(witness.terminal_fixture_ready(0).is_err());
            }else{
                assert_eq!(witness.terminal_opening_commit().unwrap(),0);
                assert!(!witness.terminal_fixture_ready(0).unwrap());
            }
            witness.event(Event{subscription:subscription.clone(),seq:Uint(4),body:EventBody::GridBegin(GridBeginEvent{frame:GridBegin { scroll: None, mode:GridMode::Replace,revision:Uint(1),columns:Uint(later_cells.len() as u64),rows:Uint(1),cursor:None,
            alternate_screen:false,graphics_omitted:false,popup_blocked:false,part_count:Uint(2) }})},128).unwrap();
            witness.event(part(5,1,0,0,later_cells[..split].to_vec()),256).unwrap();
            witness.event(part(6,1,1,split as u64,later_cells[split..].to_vec()),256).unwrap();
            if !opening_contains_marker{assert!(!witness.terminal_fixture_ready(0).unwrap());}
            witness.event(Event{subscription:subscription.clone(),seq:Uint(7),body:EventBody::GridCommit(GridCommit{revision:Uint(1)})},32).unwrap();
            assert_eq!(witness.token_commit,Some(1));
            assert_eq!(witness.fixture.as_ref().unwrap().opening,Some((0,opening_contains_marker)));
            if opening_contains_marker {
                assert!(witness.terminal_fixture_ready(0).is_err());
            }else{
                assert!(witness.terminal_fixture_ready(0).unwrap());
            }
        }
    }
    #[test]
    fn terminal_resize_result_binds_action_boot_shell_and_native_surface_not_crop() {
        let action=Uuid::random();let boot=Uuid::random();let target=ShellTarget{shell_id:Uuid::random(),generation:Uuid::random()};
        let receipt:Receipt=serde_json::from_value(json!({"actionId":action,"bootId":boot,"method":"shell.resize","shellTarget":target,
            "createdAt":0,"updatedAt":1,"state":"succeeded","result":{"kind":"shell_resize","columns":96,"rows":32}})).unwrap();
        terminal_resize_receipt(&receipt,&action,&boot,&target,96,32).unwrap();
        assert!(terminal_resize_receipt(&receipt,&Uuid::random(),&boot,&target,96,32).is_err());
        assert!(terminal_resize_receipt(&receipt,&action,&Uuid::random(),&target,96,32).is_err());
        assert!(terminal_resize_receipt(&receipt,&action,&boot,&target,96,30).is_err());
        let mut unknown=receipt.clone();unknown.state=ReceiptState::Uncertain;
        assert!(terminal_resize_receipt(&unknown,&action,&boot,&target,96,32).is_err());
        let mut foreign=receipt;foreign.shell_target=Some(ShellTarget{shell_id:Uuid::random(),generation:target.generation.clone()});
        assert!(terminal_resize_receipt(&foreign,&action,&boot,&target,96,32).is_err());
    }
    #[test]
    fn terminal_input_gate_accepts_only_complete_original_frame_dispatch_not_historical_pty_success() {
        let action=Uuid::random();let boot=Uuid::random();let target=ShellTarget{shell_id:Uuid::random(),generation:Uuid::random()};
        let receipt:Receipt=serde_json::from_value(json!({"actionId":action,"bootId":boot,"method":"shell.input","shellTarget":target,
            "createdAt":0,"updatedAt":1,"state":"dispatched","result":{"kind":"shell_dispatch","progress":{"frameBytes":19,"socketBytesWritten":19,"frameComplete":true}}})).unwrap();
        terminal_input_scope(&receipt,&action,&boot,&target).unwrap();
        grid_receipt(&receipt,&action,&boot,&GridSubject::Shell(target.clone())).unwrap();
        assert!(terminal_input_scope(&receipt,&Uuid::random(),&boot,&target).is_err());
        assert!(terminal_input_scope(&receipt,&action,&Uuid::random(),&target).is_err());
        assert!(terminal_input_scope(&receipt,&action,&boot,&ShellTarget{shell_id:Uuid::random(),generation:target.generation.clone()}).is_err());
        let mut partial=receipt.clone();partial.state=ReceiptState::Uncertain;
        partial.result=Some(ActionResult::ShellDispatch(ActionResultShellDispatch{progress:ShellDispatchProgress{frame_bytes:Some(19),socket_bytes_written:Some(7),frame_complete:Some(false)}}));
        terminal_input_scope(&partial,&action,&boot,&target).unwrap();
        assert!(grid_receipt(&partial,&action,&boot,&GridSubject::Shell(target.clone())).is_err());
        let historical:Receipt=serde_json::from_value(json!({"actionId":action,"bootId":boot,"method":"shell.input","shellTarget":target,
            "createdAt":0,"updatedAt":1,"state":"succeeded","result":{"kind":"shell_input","progress":{"encodedBytes":13,"writtenBytes":13,"flushCompleted":true}}})).unwrap();
        historical.validate().unwrap();
        assert!(grid_receipt(&historical,&action,&boot,&GridSubject::Shell(target)).is_err());
    }
    #[test]
    fn configured_terminal_gate_never_adopts_a_provider_target_from_a_historical_factory_receipt() {
        let action=Uuid::random();let boot=Uuid::random();
        let receipt:Receipt=serde_json::from_value(json!({"actionId":action,"bootId":boot,"method":"terminal.create",
            "createdAt":0,"updatedAt":1,"state":"reserved"})).unwrap();
        terminal_creation_scope(&receipt,&action,&boot).unwrap();
        assert!(terminal_creation_scope(&receipt,&Uuid::random(),&boot).is_err());
        assert!(terminal_creation_scope(&receipt,&action,&Uuid::random()).is_err());
        let mut historical=receipt;historical.target=Some(Target{session_id:Uuid::random(),generation:Uuid::random()});
        historical.validate().unwrap();
        assert!(terminal_creation_scope(&historical,&action,&boot).is_err());
    }

    fn admission_args() -> Vec<&'static str> {
        vec!["smoke","--admission-only","--backend","omp","--backend","codex",
            "--goatr","/nix/store/example/bin/goatr","--config","/private/config.toml",
            "--workspace-root","/workspace","--consent-owned-providers","--evidence-dir","/private/evidence"]
    }
    fn shared_args() -> Vec<&'static str> {
        let mut args=admission_args();
        args[1]="--shared-agent-only";
        args.push("--automate-native-inputs");
        args
    }
    #[test]
    fn shared_agent_cli_requires_both_unique_backends_and_explicit_owned_inputs() {
        let args=shared_args();
        let parsed=feedback_arguments(arguments_base()).try_get_matches_from(&args).unwrap();
        require_shared_backends(&parsed).unwrap();
        for backends in [vec!["omp"],vec!["codex"],vec!["omp","omp"],vec!["codex","codex"],vec!["omp","codex","omp"]] {
            let mut reduced=args.clone();
            let first=reduced.iter().position(|arg|*arg=="--backend").unwrap();
            reduced.drain(first..first+4);
            for backend in backends {reduced.extend(["--backend",backend]);}
            let parsed=feedback_arguments(arguments_base()).try_get_matches_from(reduced).unwrap();
            assert!(require_shared_backends(&parsed).is_err());
        }
        for required in ["--backend","--goatr","--config","--workspace-root","--consent-owned-providers","--automate-native-inputs","--evidence-dir"] {
            let mut reduced=args.clone();
            let at=reduced.iter().position(|arg|*arg==required).unwrap();
            let count=if required=="--backend" {4}
                else if matches!(required,"--consent-owned-providers"|"--automate-native-inputs") {1} else {2};
            reduced.drain(at..at+count);
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(reduced).is_err(),"{required}");
        }
        for conflict in [
            "--admission-only",
            "--grid",
            "--ui",
            "--camera",
            "--ime",
            "--setup",
            "--prepare-emulator",
            "--paging-proof",
            "--codex-invalidation-diagnostic",
            "--omp-creation-diagnostic",
            "--physical-hello-capture",
            "--physical-relay-ephemeral",
            "--development-continuity",
            "--development-owner-feedback",
            "--development-android-only",
            "--keep-foreground-for-feedback",
            "--defer-retention-crossing",
        ] {
            let mut conflicting = args.clone();
            conflicting.push(conflict);
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(conflicting)
                    .is_err(),
                "{conflict}"
            );
        }
    }
    #[test]
    fn shared_agent_evidence_is_exclusive_and_never_adopts_an_existing_scope() {
        let root=fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("goatr-shared-evidence-{}",Uuid::random().0));
        struct Root(PathBuf);
        impl Drop for Root {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
        let _root=Root(root.clone());
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path=root.join("fresh");
        fresh_shared_evidence(&path).unwrap();
        private_dir(&path,false).unwrap();
        assert_eq!(fresh_shared_evidence(&path).unwrap_err().kind(),io::ErrorKind::AlreadyExists);
    }
    #[test]
    fn shared_effective_config_uses_real_owned_scope_without_adopting_template_owner() {
        use std::os::unix::fs::PermissionsExt;
        fn published(path: &Path,config: &Config) -> io::Result<Config> {
            let lock=goatr::setup::SetupLock::acquire(path.parent().unwrap())?;
            lock.publish(path,&goatr::setup::ExpectedFile::Absent,config.serialized()?.as_bytes())?;
            drop(lock);
            Config::load(path)
        }
        struct Root(PathBuf);
        impl Drop for Root {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
        let home=fs::canonicalize(PathBuf::from(std::env::var_os("HOME").unwrap())).unwrap();
        let root=Root(home.join(format!(".scfg-{}",&Uuid::random().0[..12])));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        for directory in ["template-runtime","registry","sessions","blobs","owned","owned/runtime","owned/runtime/b","owned/project"] {
            fs::DirBuilder::new().mode(0o700).create(root.0.join(directory)).unwrap();
        }
        let wrapper=root.0.join("wrapper");
        let mut file=OpenOptions::new().write(true).create_new(true).mode(0o700).open(&wrapper).unwrap();
        file.write_all(b"#!/bin/sh\nexit 99\n").unwrap();
        drop(file);
        let template=Config::parse(&format!(
            "version=1\nallow_scoped_shell_probe=true\nruntime_dir={:?}\nruntime_dir_agent={:?}\nomp_registry_dir={:?}\nomp_blob_root={:?}\nomp_session_roots=[{:?}]\n[executables]\nherdr={wrapper:?}\nomp={wrapper:?}\ncodex={wrapper:?}\n",
            root.0.join("template-runtime"),root.0.join("template-runtime"),
            root.0.join("registry"),root.0.join("blobs"),root.0.join("sessions"),
        )).unwrap();
        let template=published(&root.0.join("deployment.toml"),&template).unwrap();
        assert!(template.codex_owners.is_empty());
        assert!(template.backend_reason(Provider::Codex).unwrap().is_some());
        let mut effective=template.clone();
        effective.runtime_dir=root.0.join("owned/runtime");
        effective.runtime_dir_agent=effective.runtime_dir.clone();
        effective.herdr_sockets=vec![root.0.join("owned/h.sock")];
        effective.projects=vec![ConfiguredProject{id:Uuid::random(),
            allow_read_tools:false,
            label:Label::new("Owned fixture project").unwrap(),path:root.0.join("owned/project")}];
        let owner=CodexOwner{id:Uuid::random(),socket_host:effective.runtime_dir.join("c.sock"),
            socket_agent:effective.runtime_dir.join("c.sock"),bindings_dir:effective.runtime_dir.join("b"),
            unit:"goatr-config-fixture.service".into()};
        effective.default_codex_owner_id=Some(owner.id.clone());
        effective.codex_owners=vec![owner];
        let effective=published(&root.0.join("owned/config.toml"),&effective).unwrap();
        for provider in [Provider::Omp,Provider::Codex] {
            assert!(effective.backend_reason(provider).unwrap().is_none());
        }
        // Configuration trust/attempt eligibility is not a fabricated native owner.
        assert!(!effective.codex_owners[0].socket_host.try_exists().unwrap());
        assert!(fs::read_dir(&effective.codex_owners[0].bindings_dir).unwrap().next().is_none());
        let mut missing=effective.clone();
        missing.codex_owners[0].bindings_dir=effective.runtime_dir.join("missing-bindings");
        let missing=published(&root.0.join("owned/missing-bindings.toml"),&missing).unwrap();
        assert!(missing.backend_reason(Provider::Codex).unwrap().is_some());
        let mut missing=effective.clone();
        missing.executables.omp=Some(root.0.join("missing-outer-wrapper"));
        let missing=published(&root.0.join("owned/missing-wrapper.toml"),&missing).unwrap();
        assert!(missing.backend_reason(Provider::Omp).unwrap().is_some());
        let mut missing=effective.clone();
        missing.omp_registry_dir=Some(root.0.join("missing-registry"));
        let missing=published(&root.0.join("owned/missing-registry.toml"),&missing).unwrap();
        assert!(missing.backend_reason(Provider::Omp).unwrap().is_some());
        let mut missing=effective.clone();
        missing.runtime_dir=root.0.join("missing-runtime");
        assert!(published(&root.0.join("owned/missing-runtime.toml"),&missing).is_err());
        let unsafe_bindings=root.0.join("unsafe-bindings");
        fs::DirBuilder::new().mode(0o700).create(&unsafe_bindings).unwrap();
        fs::set_permissions(&unsafe_bindings,fs::Permissions::from_mode(0o755)).unwrap();
        let mut unsafe_scope=effective.clone();
        unsafe_scope.codex_owners[0].bindings_dir=unsafe_bindings;
        assert!(published(&root.0.join("owned/unsafe-bindings.toml"),&unsafe_scope).is_err());
        let unsafe_wrapper=root.0.join("unsafe-wrapper");
        fs::write(&unsafe_wrapper,b"#!/bin/sh\nexit 99\n").unwrap();
        fs::set_permissions(&unsafe_wrapper,fs::Permissions::from_mode(0o777)).unwrap();
        let mut unsafe_scope=effective.clone();
        unsafe_scope.executables.codex=Some(unsafe_wrapper);
        assert!(published(&root.0.join("owned/unsafe-wrapper.toml"),&unsafe_scope).is_err());
    }
    #[test]
    fn shared_agent_native_page_admission_rejects_changed_views_duplicates_and_live_cache_rows() {
        let view=Uuid::random();
        let mut first=generation_record(5);first.source=RecordSource::NativePage;
        let page=SessionHistoryResult{view_id:view.clone(),records:vec![first],next_cursor:None};
        let mut seen=HashSet::new();
        assert_eq!(shared_history_page(&page,&view,RecordSource::NativePage,Some(6),&mut seen).unwrap(),Some(5));
        assert!(shared_history_page(&page,&view,RecordSource::NativePage,Some(6),&mut seen).is_err());
        assert!(shared_history_page(&page,&Uuid::random(),RecordSource::NativePage,None,&mut HashSet::new()).is_err());
        let mut cached=page.clone();cached.records[0].source=RecordSource::Live;
        assert!(shared_history_page(&cached,&view,RecordSource::NativePage,None,&mut HashSet::new()).is_err());
        assert!(shared_history_page(&page,&view,RecordSource::NativePage,Some(5),&mut HashSet::new()).is_err());
    }
    #[test]
    fn shared_agent_stream_admission_requires_new_events_not_the_open_snapshot() {
        let mut snapshot=generation_snapshot();
        snapshot.history.push(generation_record(1));
        let expected=target(&snapshot.state.summary);
        let mut view=GenerationView::new(snapshot,&expected).unwrap();
        assert!(!view.streamed_text&&!view.streamed_tool);
        view.routed_event(generation_event(&view,1,EventBody::HistoryUpsert(HistoryUpsertEvent{
            records:vec![generation_record(1)],
        }))).unwrap();
        assert!(!view.streamed_tool);
        let assistant:HistoryRecord=serde_json::from_value(json!({"id":"assistant","order":2,
            "role":"assistant","kind":"message","text":{"kind":"text","text":"fixture"},
            "source":"live","complete":true,"sourceTruncated":false})).unwrap();
        view.routed_event(generation_event(&view,2,EventBody::HistoryUpsert(HistoryUpsertEvent{
            records:vec![assistant],
        }))).unwrap();
        assert!(view.streamed_text&&!view.streamed_tool);
        view.routed_event(generation_event(&view,3,EventBody::HistoryUpsert(HistoryUpsertEvent{
            records:vec![generation_record(3)],
        }))).unwrap();
        assert!(view.streamed_text&&view.streamed_tool);
    }
    #[test]
    fn shared_agent_prompt_admission_distinguishes_assistant_echo_from_duplicate_native_input() {
        let user:HistoryRecord=serde_json::from_value(json!({"id":"user","order":1,
            "role":"user","kind":"message","text":{"kind":"text","text":"owned-marker"},
            "source":"live","complete":true,"sourceTruncated":false})).unwrap();
        let mut assistant=user.clone();
        assistant.id=Id("assistant".into());
        assistant.role=Role::Assistant;
        assert!(shared_user_record(&[user.clone(),assistant.clone()],"owned-marker").unwrap().is_some());
        assert!(shared_assistant_record(&[user.clone(),assistant.clone()],"owned-marker").unwrap().is_some());
        let mut duplicate=user.clone();duplicate.id=Id("duplicate".into());
        assert!(shared_user_record(&[user.clone(),duplicate],"owned-marker").is_err());
        let mut duplicate_assistant=assistant.clone();duplicate_assistant.id=Id("assistant-duplicate".into());
        assert!(shared_assistant_record(&[user,assistant,duplicate_assistant],"owned-marker").is_err());
    }
    #[test]
    fn retained_pairing_marker_history_requires_one_complete_user_and_assistant() {
        let user:HistoryRecord=serde_json::from_value(json!({"id":"pairing-user","order":1,
            "role":"user","kind":"message","text":{"kind":"text","text":"pairing-marker"},
            "source":"live","complete":true,"sourceTruncated":false})).unwrap();
        let mut assistant=user.clone();assistant.id=Id("pairing-assistant".into());assistant.role=Role::Assistant;
        assert!(shared_user_record(&[user.clone(),assistant.clone()],"pairing-marker").unwrap().is_some());
        assert!(shared_assistant_record(&[user.clone(),assistant.clone()],"pairing-marker").unwrap().is_some());
        let mut incomplete=assistant.clone();incomplete.complete=false;
        assert!(shared_assistant_record(&[user.clone(),incomplete],"pairing-marker").unwrap().is_none());
        let mut duplicate=assistant.clone();duplicate.id=Id("pairing-assistant-duplicate".into());
        assert!(shared_assistant_record(&[user,assistant,duplicate],"pairing-marker").is_err());
    }
    #[test]
    fn finite_admission_cli_requires_explicit_inputs_and_rejects_effectful_modes() {
        let args = admission_args();
        assert!(feedback_arguments(arguments_base()).try_get_matches_from(&args).is_ok());
        for required in ["--backend","--goatr","--config","--workspace-root","--consent-owned-providers"] {
            let mut missing = Vec::new();
            let mut skip = false;
            for arg in &args {
                if skip { skip = false; continue; }
                if *arg == required {
                    skip = required != "--consent-owned-providers";
                } else { missing.push(*arg); }
            }
            assert!(feedback_arguments(arguments_base()).try_get_matches_from(missing).is_err(),"{required}");
        }
        for conflict in [
            "--grid",
            "--ui",
            "--camera",
            "--ime",
            "--setup",
            "--prepare-emulator",
            "--paging-proof",
            "--codex-invalidation-diagnostic",
            "--omp-creation-diagnostic",
            "--physical-hello-capture",
            "--physical-relay-ephemeral",
            "--development-continuity",
            "--development-owner-feedback",
            "--development-android-only",
            "--keep-foreground-for-feedback",
            "--defer-retention-crossing",
            "--automate-native-inputs",
        ] {
            let mut conflicting = args.clone();
            conflicting.push(conflict);
            assert!(
                feedback_arguments(arguments_base())
                    .try_get_matches_from(conflicting)
                    .is_err(),
                "{conflict}"
            );
        }
    }
    #[test]
    fn finite_admission_needs_current_source_and_each_selected_provider_eligibility() {
        let server = Server { server_id:Uuid::random(), name:Label::new("owned").unwrap(),
            connectivity:ServerConnectivity::Online, can_create:CanCreate{omp:true,codex:false}, reason:None };
        let mut source = NativeHierarchy {server_id:server.server_id.clone(),server_incarnation:Uuid::random(),
            freshness:NativeFreshness::Current,reason:None,workspaces:vec![],tabs:vec![],panes:vec![]};
        let backends = [Provider::Omp,Provider::Codex].map(|provider|Backend{
            provider,version:"source fixture".into(),availability:Availability::SetupRequired,reason:None});
        assert_eq!(admission_reason(Provider::Omp,&backends,Some(&server),&[source.clone()]),"eligible");
        assert_eq!(admission_reason(Provider::Codex,&backends,Some(&server),&[source.clone()]),"creation_unavailable");
        let mut unavailable=backends.clone();
        unavailable[0].availability=Availability::Unavailable;
        assert_eq!(admission_reason(Provider::Omp,&unavailable,Some(&server),&[source.clone()]),"backend_unavailable");
        source.freshness=NativeFreshness::Stale;
        assert_eq!(admission_reason(Provider::Omp,&backends,Some(&server),&[source]),"source_not_current");
        assert_eq!(admission_reason(Provider::Omp,&[],Some(&server),&[]),"backend_missing_or_ambiguous");
        assert_eq!(admission_reason(Provider::Omp,&backends,None,&[]),"source_missing_or_ambiguous");
    }
    #[test]
    fn shared_creation_admission_keeps_original_source_before_first_and_second_provider() {
        let identity=(Uuid::random(),Uuid::random());
        let original=ServerEvidence{linux_boot_id:Uuid::random(),pid:17,pid_start_ticks:3,
            socket_device:1,socket_inode:10};
        let mut initial=None;
        for created in 0..=1 {
            let mut workspaces=Vec::new();
            let mut tabs=Vec::new();
            let mut panes=Vec::new();
            let mut shown_workspaces=Vec::new();
            let mut shown_tabs=Vec::new();
            let mut shown_panes=Vec::new();
            for index in 0..=created {
                let workspace=format!("workspace-{index}");
                let tab=format!("tab-{index}");
                let pane=format!("pane-{index}");
                let terminal=format!("terminal-{index}");
                workspaces.push(json!({"workspace_id":workspace,"number":index+1,"label":"fixture",
                    "focused":index==0,"active_tab_id":tab,"agent_status":"idle"}));
                tabs.push(json!({"tab_id":tab,"workspace_id":workspace,"number":1,
                    "label":"fixture","focused":index==0,"agent_status":"idle"}));
                panes.push(json!({"pane_id":pane,"terminal_id":terminal,"workspace_id":workspace,
                    "tab_id":tab,"focused":index==0,"agent_status":"idle"}));
                shown_workspaces.push(json!({"workspaceId":workspace,"number":index+1,"label":"fixture",
                    "focused":index==0,"activeTabId":tab,"agentStatus":"idle"}));
                shown_tabs.push(json!({"tabId":tab,"workspaceId":workspace,"number":1,
                    "label":"fixture","focused":index==0,"agentStatus":"idle"}));
                shown_panes.push(json!({"paneId":pane,"terminalId":terminal,"workspaceId":workspace,
                    "tabId":tab,"focused":index==0,"agentStatus":"idle"}));
            }
            let native:Snapshot=serde_json::from_value(json!({"version":"0.9.1","protocol":22,
                "focused_workspace_id":"workspace-0","focused_tab_id":"tab-0","focused_pane_id":"pane-0",
                "workspaces":workspaces,"tabs":tabs,"panes":panes,"agents":[]})).unwrap();
            let inventory:HostSnapshotResult=serde_json::from_value(json!({"subscription":Uuid::random(),"seq":0,
                "servers":[{"serverId":identity.0,"name":"owned","connectivity":"online","canCreate":{"omp":true,"codex":true}}],
                "projects":[],"sessions":[],"unbound":[],
                "hierarchy":[{"serverId":identity.0,"serverIncarnation":identity.1,"freshness":"current",
                    "workspaces":shown_workspaces,"tabs":shown_tabs,"panes":shown_panes}]})).unwrap();
            native.validate().unwrap();
            inventory.validate().unwrap();
            let initial=initial.get_or_insert_with(||native.clone());
            assert_eq!(creation_source_observation(&identity,&original,initial,&original,&native,&inventory),Ok(true));
            let mut replaced=original.clone();replaced.socket_inode+=1;
            assert!(creation_source_observation(&identity,&original,initial,&replaced,&native,&inventory).is_err());
            replaced=original.clone();replaced.pid+=1;replaced.pid_start_ticks+=1;
            assert!(creation_source_observation(&identity,&original,initial,&replaced,&native,&inventory).is_err());
            let mut successor=inventory.clone();
            successor.hierarchy[0].server_incarnation=Uuid::random();
            assert!(creation_source_observation(&identity,&original,initial,&original,&native,&successor).is_err());
            let mut rebound=native.clone();
            rebound.panes[0].pane_id=Id("replacement".into());
            rebound.panes[0].terminal_id=Id("replacement-terminal".into());
            rebound.focused_pane_id=Some(Id("replacement".into()));
            let mut rebound_inventory=inventory.clone();
            rebound_inventory.hierarchy[0].panes[0].pane_id=Id("replacement".into());
            rebound_inventory.hierarchy[0].panes[0].terminal_id=Id("replacement-terminal".into());
            rebound.validate().unwrap();
            rebound_inventory.validate().unwrap();
            assert!(creation_source_observation(&identity,&original,initial,&original,&rebound,&rebound_inventory).is_err());
        }
    }
    #[test]
    fn admission_refuses_a_current_same_name_successor_instead_of_retargeting() {
        let server=Uuid::random();
        let incarnation=Uuid::random();
        let native:Snapshot=serde_json::from_value(json!({"version":"0.9.1","protocol":22,
            "workspaces":[],"tabs":[],"panes":[],"agents":[]})).unwrap();
        let mut inventory:HostSnapshotResult=serde_json::from_value(json!({
            "subscription":Uuid::random(),"seq":0,
            "servers":[{"serverId":server,"name":"owned","connectivity":"online","canCreate":{"omp":true,"codex":true}}],
            "projects":[],"sessions":[],"unbound":[],
            "hierarchy":[{"serverId":server,"serverIncarnation":incarnation,"freshness":"current",
                "workspaces":[],"tabs":[],"panes":[]}]})).unwrap();
        native.validate().unwrap();
        inventory.validate().unwrap();
        let pinned=(server,incarnation);
        assert!(admission_source(&inventory,"owned",&native,Some(&pinned)).is_ok());
        inventory.hierarchy[0].server_incarnation=Uuid::random();
        assert_eq!(admission_source(&inventory,"owned",&native,Some(&pinned)).unwrap_err(),"source_replaced");
        inventory.servers[0].server_id=Uuid::random();
        inventory.hierarchy[0].server_id=inventory.servers[0].server_id.clone();
        assert_eq!(admission_source(&inventory,"owned",&native,Some(&pinned)).unwrap_err(),"source_replaced");
    }
    #[test]
    fn hello_manager_capture_accepts_only_closed_first_guard_fields() {
        let line = b"goatr_hello_capture phase=codex_manager_hello guard=runtime_path code=NotReady io=none";
        let value = parse_hello_guard_line(line).unwrap();
        assert_eq!(value["guard"],"runtime_path");
        for line in [
            b"goatr_hello_capture phase=codex_manager_hello guard=/private/path code=NotReady io=none".as_slice(),
            b"goatr_hello_capture phase=codex_manager_hello guard=runtime_path code=NotReady io=private_error",
            b"goatr_hello_capture phase=codex_manager_hello guard=runtime_path code=NotReady io=none secret=payload",
            b"goatr_hello_capture phase=codex_manager_hello guard=runtime_path code=Unauthorized io=none",
        ] { assert!(parse_hello_guard_line(line).is_none()); }
    }

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
        for guard in [
            "owner_pending_authorization",
            "owner_pending_service_metadata",
            "owner_pending_service_proof",
            "owner_pending_native_process",
            "owner_pending_socket_epoch",
            "owner_pending_owner_record_proof",
        ] {
            let line = format!(
                "goatr_codex_first_guard phase=owner_progress guard={guard} code=none io=none errno=0"
            );
            assert!(parse_codex_guard_line(line.as_bytes()).is_some());
        }
        assert!(parse_codex_guard_line(
            b"goatr_codex_first_guard phase=owner_progress guard=owner_pending_unknown code=none io=none errno=0"
        )
        .is_none());
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
    fn codex_single_creation_diagnostic_cli_needs_no_native_input_and_refuses_it() {
        let args = [
            "smoke",
            "--codex-invalidation-diagnostic",
            "--backend=codex",
            "--consent-owned-providers",
            "--goatr=/nix/store/example/bin/goatr",
            "--expected-goatr-sha256=1111111111111111111111111111111111111111111111111111111111111111",
            "--expected-companion-lock-sha256=2222222222222222222222222222222222222222222222222222222222222222",
            "--evidence-dir=/private/evidence",
        ];
        let parsed = feedback_arguments(arguments_base()).try_get_matches_from(args).unwrap();
        assert!(parsed.get_flag("codex-invalidation-diagnostic"));
        assert!(!parsed.get_flag("automate-native-inputs"));
        assert!(feedback_arguments(arguments_base())
            .try_get_matches_from([
                "smoke",
                "--codex-invalidation-diagnostic",
                "--backend=codex",
                "--consent-owned-providers",
                "--automate-native-inputs",
                "--goatr=/nix/store/example/bin/goatr",
                "--expected-goatr-sha256=1111111111111111111111111111111111111111111111111111111111111111",
                "--expected-companion-lock-sha256=2222222222222222222222222222222222222222222222222222222222222222",
                "--evidence-dir=/private/evidence",
            ])
            .is_err());
    }

    #[test]
    fn codex_single_creation_drain_projects_only_ordered_owner_and_first_guard() {
        let progress = CodexDrainProgress::default();
        for guard in [
            "owner_start_settled",
            "owner_child_status_returned",
            "owner_native_initialize_entered",
        ] {
            progress.accepted_guard(&json!({"phase":"owner_progress","guard":guard}));
        }
        progress.accepted_guard(&json!({"phase":"creation","guard":"owner_timeout"}));
        progress.accepted_guard(&json!({"phase":"connection","guard":"ignored_later"}));
        let summary = progress.snapshot();
        assert_eq!(summary.owner_progress_mask, 7);
        assert_eq!(summary.first_nonprogress_phase.as_deref(), Some("creation"));
        assert_eq!(summary.first_nonprogress_guard.as_deref(), Some("owner_timeout"));
    }

    #[test]
    fn owner_pending_drain_projects_one_third_slot_with_explicit_observation() {
        let progress = CodexDrainProgress::default();
        for guard in [
            "owner_start_settled",
            "owner_child_status_returned",
            "owner_pending_service_proof",
            "owner_native_initialize_entered",
        ] {
            progress.accepted_guard(&json!({"phase":"owner_progress","guard":guard}));
        }
        let summary = progress.snapshot();
        assert_eq!(summary.owner_progress_mask, 7);
        assert_eq!(summary.owner_progress_guard, Some("owner_pending_service_proof"));
        assert!(summary.pending_at_observation);
    }

    #[test]
    fn codex_single_creation_drain_refuses_zero_count_dead_or_errored_workers() {
        let clean = CodexDrainSummary::default();
        assert!(codex_drain_is_pristine(&clean, false));
        assert!(!codex_drain_is_pristine(&clean, true));
        let mut eof = clean.clone();
        eof.eof = true;
        eof.termination = CodexDrainTermination::Eof;
        assert!(!codex_drain_is_pristine(&eof, false));
        let mut errored = clean;
        errored.read_error = Some(("other", None));
        errored.termination = CodexDrainTermination::ReadError;
        assert!(!codex_drain_is_pristine(&errored, false));
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
    fn physical_ephemeral_relay_requires_explicit_backend_only_context() {
        assert_eq!(physical_relay_bind_port(false, true, OwnedOmpCaptureMode::Backend, false).unwrap(), 7466);
        assert_eq!(physical_relay_bind_port(false, true, OwnedOmpCaptureMode::None, true).unwrap(), 7466);
        assert_eq!(physical_relay_bind_port(true, true, OwnedOmpCaptureMode::Backend, false).unwrap(), 0);
        assert!(physical_relay_bind_port(true, false, OwnedOmpCaptureMode::Backend, false).is_err());
        assert!(physical_relay_bind_port(true, true, OwnedOmpCaptureMode::Backend, true).is_err());
        for mode in [OwnedOmpCaptureMode::None, OwnedOmpCaptureMode::Creation, OwnedOmpCaptureMode::Offline, OwnedOmpCaptureMode::Conflict] {
            assert!(physical_relay_bind_port(true, true, mode, false).is_err());
        }
    }

    #[test]
    fn physical_ephemeral_relay_argument_requires_physical_capture() {
        let accepted = ["smoke", "--physical-hello-capture", "--physical-relay-ephemeral",
            "--config", "/owned/config.toml", "--goatr", "/nix/store/owned-goatr/bin/goatr",
            "--admin-socket", "/owned/runtime/admin.sock", "--expected-goatr-sha256", "00",
            "--expected-companion-lock-sha256", "00", "--evidence-dir", "/owned/evidence"];
        assert!(feedback_arguments(arguments_base()).try_get_matches_from(accepted).is_ok());
        assert!(feedback_arguments(arguments_base()).try_get_matches_from(accepted.into_iter().filter(|arg| *arg != "--physical-hello-capture")).is_err());
    }

    #[test]
    fn coarse_shell_parser_accepts_only_closed_settlement_shapes() {
        for line in [
            "goatr_omp_first_guard phase=coarse_shell guard=installed code=none io=none frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=process_info code=NotReady io=permission_denied frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=source_lookup code=NotFound io=none frame=none",
        ] {
            assert!(parse_omp_guard_line(line.as_bytes()).is_some());
        }
        for line in [
            "goatr_omp_first_guard phase=coarse_shell guard=unknown code=NotReady io=none frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=installed code=NotReady io=none frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=process_info code=none io=permission_denied frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=source_lookup code=Unauthorized io=none frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=shell_absent code=NotReady io=invalid_data frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=process_info code=NotReady io=raw_native_error frame=none",
            "goatr_omp_first_guard phase=coarse_shell guard=installed code=none io=none frame=frame_cap",
            "goatr_omp_first_guard phase=coarse_shell guard=installed code=none io=none frame=none pid=1",
        ] {
            assert!(parse_omp_guard_line(line.as_bytes()).is_none());
        }
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
    fn omp_result_deadline_detail_is_closed_and_keeps_one_backend_budget_slot() {
        let prefix="goatr_omp_first_guard phase=shell_probe guard=result_deadline code=Timeout io=none frame=none";
        let budget=herdr_report_capture::Budget::new_backend();
        for detail in [
            "deadlineSite=loop_check observation=not_read entryBudget=expired",
            "deadlineSite=loop_check observation=empty_only entryBudget=le_1s",
            "deadlineSite=after_authorization observation=partial_seen entryBudget=le_5s",
            "deadlineSite=complete_before_parse observation=complete_seen entryBudget=gt_5s",
            "deadlineSite=parsed_success observation=complete_seen entryBudget=gt_5s",
        ] {
            let parsed=parse_omp_guard_line(format!("{prefix} {detail}").as_bytes()).unwrap();
            assert!(parsed["deadlineSite"].is_string()&&parsed["observation"].is_string()&&parsed["entryBudget"].is_string());
            budget.omp(parsed);
        }
        assert_eq!(budget.creation_counts().0,1);
        assert!(budget.shell_probe_captured());
        for detail in [
            "deadlineSite=unknown observation=not_read entryBudget=expired",
            "deadlineSite=loop_check observation=raw entryBudget=le_1s",
            "deadlineSite=loop_check observation=partial_seen entryBudget=119634",
            "deadlineSite=loop_check observation=not_read",
            "deadlineSite=parsed_success observation=empty_only entryBudget=gt_5s",
            "deadlineSite=after_authorization observation=not_read entryBudget=expired",
            "deadlineSite=loop_check observation=not_read entryBudget=expired extra=x",
            "deadlineSite=loop_check observation=not_read entryBudget=expired name=1 kind=1 pane=1 terminal=1 workspace=1 argv=1",
        ] {assert!(parse_omp_guard_line(format!("{prefix} {detail}").as_bytes()).is_none());}
        assert!(parse_omp_guard_line(format!("{} deadlineSite=loop_check observation=partial_seen entryBudget=le_1s",
            prefix.replace("result_deadline","result_read_failed")).as_bytes()).is_none());
        let plain=parse_omp_guard_line(prefix.as_bytes()).unwrap();
        let legacy=herdr_report_capture::Budget::new(false);legacy.omp(plain.clone());
        assert_eq!(legacy.creation_counts().0,1,"ordinary Creation reporting stays unchanged");
        let backend=herdr_report_capture::Budget::new_backend();backend.omp(plain);
        assert_eq!(backend.creation_counts().0,0,"Backend deadline requires the complete closed detail");
        let mut extra=parse_omp_guard_line(format!("{prefix} deadlineSite=loop_check observation=partial_seen entryBudget=le_1s").as_bytes()).unwrap();
        let creation=herdr_report_capture::Budget::new(false);creation.omp(extra.clone());
        assert_eq!(creation.creation_counts().0,0,"Backend-only details do not enter ordinary Creation capture");
        extra["unknown"]=json!("not admitted");backend.omp(extra);
        assert_eq!(backend.creation_counts().0,0,"unknown fields cannot bypass the parser through the shared budget");
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
            adb_client_home: root.join("home"),
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
    #[tokio::test]
    async fn recorded_adb_home_is_consumed_only_by_the_child_and_unsafe_credential_metadata_refuses() {
        const CHILD:&str="GOATR_ADB_HOME_METADATA_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert_eq!(std::env::var_os("HOME"),std::env::var_os("GOATR_ADB_HOME_EXPECTED"));
            let runtime=std::env::var_os("XDG_RUNTIME_DIR");
            if std::env::var_os("GOATR_ADB_RUNTIME_ABSENT").is_some(){assert!(runtime.is_none());}else{assert_eq!(runtime,std::env::var_os("GOATR_ADB_RUNTIME_EXPECTED"));}
            println!("owned_adb_child_home_confirmed");
            return;
        }
        struct Root(PathBuf);impl Drop for Root{fn drop(&mut self){fs::remove_dir_all(&self.0).unwrap();}}
        let before_home=std::env::var_os("HOME");let before_runtime=std::env::var_os("XDG_RUNTIME_DIR");
        let root=Root(fs::canonicalize(PathBuf::from(before_home.clone().unwrap())).unwrap().join(format!(".goatr-adb-home-{}",Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let home=root.0.join("home");fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
        let credential=home.join(".emulator_console_auth_token");
        // A nonsecret test fixture: the production validator must use metadata, not contents.
        OpenOptions::new().write(true).create_new(true).mode(0o600).open(&credential).unwrap().write_all(b"public metadata-only fixture").unwrap();
        let lease_path=root.0.join("run.json");
        let mut recorded=json!({"schema":1,"runId":"8bd5376a-b170-4fa3-80c0-d449834fed4d","ownerUid":uid(),"root":root.0,
            "serial":"emulator-5594","avdName":"owned-avd","avdHome":home,"androidUserHome":home,"avdPath":home,
            "processIdentity":{"linuxBootId":Uuid::random(),"pid":1,"startTicks":"1","executable":"/nix/store/owned/qemu"}});
        OpenOptions::new().write(true).create_new(true).mode(0o600).open(&lease_path).unwrap().write_all(&serde_json::to_vec(&recorded).unwrap()).unwrap();
        assert!(lease(&lease_path).is_err());
        recorded["adbClientHome"]=json!(home);
        fs::write(&lease_path,serde_json::to_vec(&recorded).unwrap()).unwrap();
        let selected=lease(&lease_path).unwrap();
        let mut child=tokio::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact","live::tests::recorded_adb_home_is_consumed_only_by_the_child_and_unsafe_credential_metadata_refuses","--nocapture"])
            .env(CHILD,"1").env("GOATR_ADB_HOME_EXPECTED",&home);
        if let Some(runtime)=&before_runtime{child.env("GOATR_ADB_RUNTIME_EXPECTED",runtime);}else{child.env("GOATR_ADB_RUNTIME_ABSENT","1");}
        configure_adb_client_home(&mut child,&selected.root,&selected.adb_client_home).unwrap();
        let consumed=output(child,15,65536).await.unwrap();
        assert!(consumed.windows(b"owned_adb_child_home_confirmed".len()).any(|part|part==b"owned_adb_child_home_confirmed"));
        assert_eq!(std::env::var_os("HOME"),before_home);assert_eq!(std::env::var_os("XDG_RUNTIME_DIR"),before_runtime);
        let alias=root.0.join("alias");std::os::unix::fs::symlink(&home,&alias).unwrap();
        assert!(validate_adb_client_home(&root.0,&alias).is_err());
        fs::set_permissions(&credential,fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_adb_client_home(&root.0,&home).is_err());
        fs::set_permissions(&credential,fs::Permissions::from_mode(0o600)).unwrap();
        let hardlink=home.join("duplicate");fs::hard_link(&credential,&hardlink).unwrap();
        assert!(validate_adb_client_home(&root.0,&home).is_err());fs::remove_file(hardlink).unwrap();
        fs::remove_file(&credential).unwrap();
        let name=std::ffi::CString::new(credential.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);
        assert!(validate_adb_client_home(&root.0,&home).is_err());
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
    #[test]
    fn grid_immutable_configuration_admits_exact_bytes_and_owns_deployment() {
        use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = Root(PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".grid-config-{}", Uuid::random().0)));
        fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let text = format!(
            "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\nherdr_sockets=[]\n[executables]\nherdr={executable:?}\n",
            root.0, root.0
        );
        let path = root.0.join("config.toml");
        let mut file = OpenOptions::new().write(true).create_new(true)
            .mode(0o600).open(&path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.sync_all().unwrap();
        drop(file);
        Config::load(&path).expect("ordinary0600 deployment must parse and pass real preflight");
        let sha = binary_sha256(&path).unwrap();
        assert!(GridQualification::configuration(&path, &sha).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(Config::load(&path).is_err(), "ordinary loader still requires0600");
        let mut wrong = sha.clone();
        wrong.replace_range(..1, if sha.starts_with('0') { "1" } else { "0" });
        assert!(GridQualification::configuration(&path, &wrong).is_err());
        let alias = root.0.join("alias.toml");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(GridQualification::configuration(&alias, &sha).is_err());
        fs::remove_file(&alias).unwrap();
        let linked = root.0.join("linked.toml");
        fs::hard_link(&path, &linked).unwrap();
        assert!(GridQualification::configuration(&path, &sha).is_err());
        fs::remove_file(&linked).unwrap();
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(GridQualification::configuration(&path, &sha).is_err());
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
        let fifo = root.0.join("writer-free.fifo");
        let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: the owned path is NUL-terminated and remains live for mkfifo.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o400) }, 0);
        let (finished, result) = std::sync::mpsc::channel();
        let read_fifo = fifo.clone();
        let read_sha = sha.clone();
        let reader = std::thread::spawn(move || {
            let rejected = GridQualification::configuration(&read_fifo, &read_sha).is_err();
            let _ = finished.send(rejected);
        });
        let rejected = match result.recv_timeout(Duration::from_secs(5)) {
            Ok(rejected) => {
                reader.join().unwrap();
                rejected
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                fs::set_permissions(&fifo, fs::Permissions::from_mode(0o600)).unwrap();
                let writer = OpenOptions::new().read(true).write(true)
                    .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&fifo).expect("owned nonblocking FIFO cleanup writer");
                let released = result.recv_timeout(Duration::from_secs(5));
                drop(writer);
                if released.is_ok() {
                    reader.join().unwrap();
                }
                panic!("writer-free FIFO admission blocked; cleanup released reader: {}",
                    released.is_ok());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                reader.join().unwrap();
                panic!("owned FIFO reader did not return an admission result");
            }
        };
        assert!(rejected, "nonregular configuration admitted");
        fs::remove_file(&fifo).unwrap();
        let deployment = GridQualification::configuration(&path, &sha)
            .expect("valid immutable0400 deployment must be admitted");
        fs::remove_file(&path).unwrap();
        deployment.preflight().unwrap();
    }

    #[test]
    fn grid_immutable_admission_reads_owned_0400_and_rejects_permission_aliases() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("goatr-grid-admission-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("qualification.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"version\":1}\n").unwrap();
        drop(file);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        let bytes = GridQualification::immutable_bytes(&path).unwrap();
        let read: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            read["version"] == 1,
            "owned0400 admission did not read original data"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            GridQualification::immutable_bytes(&path).is_err(),
            "mutable0600 lease admitted"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        let alias = root.join("alias.json");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(
            GridQualification::immutable_bytes(&alias).is_err(),
            "symlink lease admitted"
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            GridQualification::immutable_bytes(&path).is_err(),
            "nonprivate lease parent admitted"
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn grid_action_interleaved_fault_preserves_first_boundary_without_lookup() {
        let host = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[71; 32]))
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        let peer = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[72; 32]))
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        let (client, server) = tokio::join!(peer.connect(host.addr(), goatr::ALPN), async {
            host.accept().await.unwrap().await.unwrap()
        });
        let client = client.unwrap();
        let (mut send, recv) = client.open_bi().await.unwrap();
        send.write_all(&[0]).await.unwrap();
        let (mut host_send, mut host_recv) = server.accept_bi().await.unwrap();
        let mut probe = [0];
        host_recv.read_exact(&mut probe).await.unwrap();
        let expected = target(&generation_snapshot().state.summary);
        let opening = Uuid::random();
        let mut witness =
            GridWitness::pending(opening.clone(), GridSubject::Pane(expected.clone()));
        let descriptor = ResultBody::TerminalGridSubscribe(TerminalGridSubscribeResult {
            target: expected.clone(),
            subscription: Uuid::random(),
            seq: ZeroValue(0),
            frame: GridBegin { scroll: None, mode: GridMode::Replace,
            revision: Uint(0),
            columns: Uint(1),
            rows: Uint(1),
            cursor: None,
            alternate_screen: false,
            graphics_omitted: true,
            popup_blocked: false,
            part_count: Uint(1), },
        });
        let encoded = encode_payload(&Envelope::Response(Response {
            id: opening,
            outcome: Ok(descriptor),
        }))
        .unwrap();
        let Envelope::Response(response) =
            decode_payload(&encoded, Some(Method::TerminalGridSubscribe)).unwrap()
        else {
            panic!("opening family");
        };
        witness
            .response(&response.id, &response.outcome, encoded.len())
            .unwrap();
        let mut phone = Phone {
            connection: client,
            send,
            recv,
            hello: HelloResult {
                host_id: EndpointId(host.id().to_string()),
                device_id: Uuid::random(),
                boot_id: Uuid::random(),
                protocol: ProtocolVersion(2),
                backends: vec![],
                features: HelloFeatures::default(),
            },
            events: 0,
            generation: None,
            history_command: None,
            omp_capture_budget: None,
            phone_close_reported: false,
                grid: Some(witness),
        };
        let body = RequestBody::PromptSubmit(PromptSubmitParams { skill: None, boot_id: phone.hello.boot_id.clone(),
        delivery: None, native_turn_id: None, images: None,
        target: expected,
        text: PromptText("owned boundary fixture".into()), });
        let server_side = async {
            let first = transport::read_frame(&mut host_recv, FRAME_MAX)
                .await
                .unwrap();
            assert!(
                matches!(
                    decode_payload(&first, None).unwrap(),
                    Envelope::Request(Request {
                        body: RequestBody::PromptSubmit(_),
                        ..
                    })
                ),
                "initial action family"
            );
            let cell:GridCell=serde_json::from_value(json!({"text":"PRIVATEBOUNDARY","width":1,"foreground":{"kind":"default"},
                "background":{"kind":"default"},"attributes":{"bold":false,"faint":false,"italic":false,"blink":false,"strike":false,"underline":"none"}})).unwrap();
            let fault = Envelope::Event(Event {
                subscription: Uuid::random(),
                seq: Uint(1),
                body: EventBody::GridPart(GridPart {
                    revision: Uint(0),
                    index: Uint(0),
                    spans: vec![GridSpan {
                        row: Uint(0),
                        column: Uint(0),
                        cells: vec![cell],
                    }],
                }),
            });
            transport::write_frame(&mut host_send, &encode_payload(&fault).unwrap())
                .await
                .unwrap();
            let lookup = tokio::time::timeout(
                Duration::from_secs(1),
                transport::read_frame(&mut host_recv, FRAME_MAX),
            )
            .await;
            let looked_up = matches!(lookup, Ok(Ok(_)));
            server.close(0u32.into(), b"owned regression complete");
            looked_up
        };
        let action = async {
            let error = phone
                .grid_action(Uuid::random(), body, false)
                .await
                .err()
                .expect("private fault must fail");
            let original = error.to_string() == "grid smoke boundary: phase=subscription_sequence"
                && super::super::helper_error_location(&error).is_some()
                && phone.grid.as_ref().is_some_and(|g| g.retired);
            phone
                .connection
                .close(0u32.into(), b"owned regression complete");
            original
        };
        let (original, looked_up) = tokio::join!(action, server_side);
        assert!(original, "original routing boundary was replaced");
        assert!(!looked_up, "fatal action reader issued another request");
        goatr::close_endpoint(&peer).await.unwrap();
        goatr::close_endpoint(&host).await.unwrap();
    }
    #[test]
    fn grid_source_associates_the_original_graph_without_adopting_logical_rows() {
        let server_id = Uuid::random();
        let incarnation = Uuid::random();
        let logical_id = Uuid::random();
        let native: Snapshot = serde_json::from_value(json!({
            "version":"0.9.1","protocol":22,
            "focused_workspace_id":"workspace","focused_tab_id":"tab","focused_pane_id":"pane",
            "workspaces":[{"workspace_id":"workspace","number":1,"label":"fixture","focused":true,"active_tab_id":"tab","agent_status":"idle"}],
            "tabs":[{"tab_id":"tab","workspace_id":"workspace","number":1,"label":"fixture","focused":true,"agent_status":"idle"}],
            "panes":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"workspace","tab_id":"tab","focused":true,"agent_status":"idle",
                "read_identity":{"herdr_incarnation":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","runtime_epoch":1,"binding_epoch":2}}],
            "agents":[]
        })).unwrap();
        native.validate().unwrap();
        let root = native.panes[0].clone();
        let mut inventory: HostSnapshotResult = serde_json::from_value(json!({
            "subscription":Uuid::random(),"seq":0,
            "servers":[
                {"serverId":server_id,"name":"proved","connectivity":"online","canCreate":{"omp":false,"codex":true}},
                {"serverId":logical_id,"name":"logical","connectivity":"unavailable","canCreate":{"omp":true,"codex":false}}
            ],
            "projects":[],"sessions":[],"shells":[],"unbound":[],
            "hierarchy":[{"serverId":server_id,"serverIncarnation":incarnation,"freshness":"current",
                "workspaces":[{"workspaceId":"workspace","number":1,"label":"fixture","focused":true,"activeTabId":"tab","agentStatus":"idle"}],
                "tabs":[{"tabId":"tab","workspaceId":"workspace","number":1,"label":"fixture","focused":true,"agentStatus":"idle"}],
                "panes":[{"paneId":"pane","terminalId":"terminal","workspaceId":"workspace","tabId":"tab","focused":true,"agentStatus":"idle"}]}]
        })).unwrap();
        inventory.validate().unwrap();
        for _ in 0..2 {
            let (associated, actual_incarnation) = grid_source(
                &inventory, &native, &root, Some((&server_id, &incarnation)),
            ).expect("original proved source beside an unavailable logical row");
            assert_eq!((&associated.server_id, actual_incarnation), (&server_id, &incarnation));
            assert!(!associated.can_create.omp);
            assert!(associated.can_create.codex);
            inventory.servers.reverse();
        }
        for connectivity in [ServerConnectivity::Online, ServerConnectivity::Offline] {
            let mut other_source = inventory.clone();
            other_source.servers[1].connectivity = connectivity;
            assert!(grid_source(&other_source, &native, &root, None).is_err());
        }
        let mut duplicate = inventory.clone();
        duplicate.servers.push(duplicate.servers[0].clone());
        assert!(grid_source(&duplicate, &native, &root, None).is_err());
        let mut missing = inventory.clone();
        missing.servers.remove(0);
        assert!(grid_source(&missing, &native, &root, None).is_err());
        assert!(grid_source(&inventory, &native, &root, Some((&logical_id, &incarnation))).is_err());
        let mut other_graph = inventory.clone();
        let mut graph = other_graph.hierarchy[0].clone();
        graph.server_id = logical_id;
        other_graph.hierarchy.push(graph);
        assert!(grid_source(&other_graph, &native, &root, None).is_err());
        for field in 0..4 {
            let mut changed = inventory.clone();
            let pane = &mut changed.hierarchy[0].panes[0];
            match field {
                0 => pane.pane_id = Id("other".into()),
                1 => pane.terminal_id = Id("other".into()),
                2 => pane.workspace_id = Id("other".into()),
                _ => pane.tab_id = Id("other".into()),
            }
            assert!(grid_source(&changed, &native, &root, None).is_err());
        }
        let mut changed_native = native.clone();
        changed_native.panes[0].read_identity.as_mut().unwrap().binding_epoch += 1;
        assert!(grid_source(&inventory, &changed_native, &root, None).is_err());
        changed_native.panes.clear();
        assert!(grid_source(&inventory, &changed_native, &root, None).is_err());
        let mut stale = inventory.clone();
        stale.hierarchy[0].freshness = NativeFreshness::Stale;
        assert!(grid_source(&stale, &native, &root, None).is_err());
        stale.hierarchy[0].freshness = NativeFreshness::Current;
        stale.hierarchy[0].reason = Some(NativeUnavailableReason::SourceUnavailable);
        assert!(grid_source(&stale, &native, &root, None).is_err());
        assert!(grid_source(&inventory, &native, &root, Some((&server_id, &Uuid::random()))).is_err());
    }
    #[tokio::test]
    async fn grid_source_loss_requires_joined_intent_and_independent_native_eof() {
        use std::os::fd::FromRawFd;
        use tokio::io::AsyncWriteExt;
        let run = Uuid::random();
        let ready = Uuid::random();
        let trigger = Uuid::random();
        let mut fds = [0; 2];
        assert!(
            unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } == 0,
            "owned pipe"
        );
        let input = unsafe { File::from_raw_fd(fds[0]) };
        let mut output = unsafe { File::from_raw_fd(fds[1]) };
        let mut bytes =
            serde_json::to_vec(&json!({"version":1,"run":run,"ready":ready,"trigger":trigger}))
                .unwrap();
        bytes.push(b'\n');
        output.write_all(&bytes).unwrap();
        let input =
            tokio::io::BufReader::new(McpStdin(tokio::io::unix::AsyncFd::new(input).unwrap()));
        let (_input, intent) = grid_source_intent(input).await;
        let mut loss = GridSourceLoss {
            run,
            ready,
            trigger: None,
            native_eof: false,
            iroh_closed: false,
            sync_required: false,
            deadline: None,
        };
        loss.arm(intent.unwrap()).unwrap();
        loss.downstream(false).unwrap();
        assert!(
            !loss.complete(),
            "Iroh closure/intent alone claimed native EOF"
        );
        let (reader, mut writer) = tokio::net::UnixStream::pair().unwrap();
        let mut reader = tokio::io::BufReader::new(reader);
        writer.write_all(b"{}\n").await.unwrap();
        let mut total = 0;
        let line = grid_native_line(
            &mut reader,
            tokio::time::Instant::now() + Duration::from_secs(15),
            &mut total,
        )
        .await
        .unwrap();
        assert!(
            line.is_some() && total == 3,
            "actual native LF accounting changed"
        );
        assert!(!loss.complete(), "native metadata data was treated as EOF");
        drop(writer);
        assert!(
            grid_native_line(
                &mut reader,
                tokio::time::Instant::now() + Duration::from_secs(15),
                &mut total
            )
            .await
            .unwrap()
            .is_none(),
            "original native fd did not observe EOF"
        );
        loss.upstream_eof().unwrap();
        let cutoff = loss.deadline;
        assert!(
            loss.complete(),
            "joined independently observed source loss did not settle"
        );
        loss.downstream(true).unwrap();
        assert!(
            loss.deadline == cutoff,
            "downstream progress renewed the source observation bound"
        );
    }
    #[tokio::test]
    async fn grid_source_selector_and_real_fin_cannot_retroactively_arm_or_infer_connection_close()
    {
        let run = Uuid::random();
        let ready = Uuid::random();
        let intent = || GridSourceIntent {
            version: 1,
            run: run.clone(),
            ready: ready.clone(),
            trigger: Uuid::random(),
        };
        let (native, writer) = tokio::net::UnixStream::pair().unwrap();
        drop(writer);
        let mut native = tokio::io::BufReader::new(native);
        let mut total = 0;
        let already_eof = grid_native_line(
            &mut native,
            tokio::time::Instant::now() + Duration::from_secs(15),
            &mut total,
        )
        .await
        .unwrap();
        let wake = grid_source_wake(
            std::future::ready(already_eof),
            std::future::pending::<()>(),
            std::future::pending::<()>(),
            std::future::pending::<()>(),
            std::future::pending::<io::Result<Vec<u8>>>(),
            std::future::ready(intent()),
        )
        .await;
        let mut before_arm = GridSourceLoss {
            run: run.clone(),
            ready: ready.clone(),
            trigger: None,
            native_eof: false,
            iroh_closed: false,
            sync_required: false,
            deadline: None,
        };
        match wake {
            GridSourceWake::Native(None) => assert!(
                before_arm.upstream_eof().is_err(),
                "old EOF admitted an arm"
            ),
            _ => panic!("queued intent won over already-ready original native EOF"),
        }
        assert!(
            before_arm.trigger.is_none() && !before_arm.complete(),
            "old termination retroactively qualified"
        );
        for fin in [false, true] {
            let host = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[73; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .bind()
                .await
                .unwrap();
            let peer = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[74; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .bind()
                .await
                .unwrap();
            let (client, server) = tokio::join!(peer.connect(host.addr(), goatr::ALPN), async {
                host.accept().await.unwrap().await.unwrap()
            });
            let client = client.unwrap();
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(&[0]).await.unwrap();
            let (mut host_send, mut host_recv) = server.accept_bi().await.unwrap();
            let mut probe = [0];
            host_recv.read_exact(&mut probe).await.unwrap();
            let mut loss = GridSourceLoss {
                run: run.clone(),
                ready: ready.clone(),
                trigger: None,
                native_eof: false,
                iroh_closed: false,
                sync_required: false,
                deadline: None,
            };
            if fin {
                loss.arm(intent()).unwrap();
                host_send.finish().unwrap();
                let error = transport::read_frame(&mut recv, FRAME_MAX)
                    .await
                    .err()
                    .expect("FIN must end the frame reader");
                assert!(
                    error.kind() == io::ErrorKind::InvalidData,
                    "outer frame metadata contract changed"
                );
                assert!(
                    client.close_reason().is_none(),
                    "fixture closed connection instead of only FIN"
                );
                grid_source_fin(true, error).unwrap();
                assert!(
                    !loss.iroh_closed && !loss.complete(),
                    "FIN promoted to Iroh closure"
                );
                let (native, writer) = tokio::net::UnixStream::pair().unwrap();
                drop(writer);
                let mut native = tokio::io::BufReader::new(native);
                let mut count = 0;
                assert!(
                    grid_native_line(
                        &mut native,
                        tokio::time::Instant::now() + Duration::from_secs(15),
                        &mut count
                    )
                    .await
                    .unwrap()
                    .is_none(),
                    "independent original native EOF missing"
                );
                loss.upstream_eof().unwrap();
                assert!(
                    !loss.complete(),
                    "native EOF plus FIN settled without actual connection closure"
                );
                server.close(0u32.into(), b"owned FIN regression complete");
                client.closed().await;
                loss.downstream(false).unwrap();
                assert!(loss.complete(), "actual later closure failed to settle");
            } else {
                host_send
                    .write_all(&((FRAME_MAX + 1) as u32).to_be_bytes())
                    .await
                    .unwrap();
                let error = transport::read_frame(&mut recv, FRAME_MAX)
                    .await
                    .err()
                    .expect("oversized header must fail");
                let original = transport::FrameFailure::from_error(&error);
                let wake = grid_source_wake(
                    std::future::pending::<()>(),
                    std::future::pending::<()>(),
                    std::future::pending::<()>(),
                    std::future::pending::<()>(),
                    std::future::ready(Err(error)),
                    std::future::ready(intent()),
                )
                .await;
                let error = match wake {
                    GridSourceWake::Frame(Err(error)) => error,
                    _ => panic!("intent won over already-ready framed failure"),
                };
                let error = grid_source_fin(false, error)
                    .err()
                    .expect("pre-arm failure must remain fatal");
                let error = grid_source_fin(true, error)
                    .err()
                    .expect("non-EOF framed failure must remain fatal");
                assert!(
                    transport::FrameFailure::from_error(&error) == original,
                    "first closed frame metadata replaced"
                );
                assert!(
                    loss.trigger.is_none() && !loss.complete(),
                    "framed fault armed the observer"
                );
                server.close(0u32.into(), b"owned pre-arm regression complete");
                client.closed().await;
                let wake = grid_source_wake(
                    std::future::pending::<()>(),
                    std::future::ready(()),
                    std::future::pending::<()>(),
                    std::future::pending::<()>(),
                    std::future::pending::<io::Result<Vec<u8>>>(),
                    std::future::ready(intent()),
                )
                .await;
                match wake {
                    GridSourceWake::Closed => assert!(
                        loss.downstream(false).is_err(),
                        "old connection closure admitted arm"
                    ),
                    _ => panic!("intent won over already-ready actual downstream closure"),
                }
                assert!(
                    loss.trigger.is_none() && !loss.complete(),
                    "old closure retroactively qualified"
                );
            }
            client.close(0u32.into(), b"owned regression complete");
            goatr::close_endpoint(&peer).await.unwrap();
            goatr::close_endpoint(&host).await.unwrap();
        }
    }
    #[tokio::test]
    async fn grid_source_combined_ready_frame_fault_wins_closure_handoff_and_settlement() {
        use futures_util::FutureExt;
        for final_settlement in [false, true] {
            let host = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[75; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .bind()
                .await
                .unwrap();
            let peer = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[76; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .bind()
                .await
                .unwrap();
            let (client, server) = tokio::join!(peer.connect(host.addr(), goatr::ALPN), async {
                host.accept().await.unwrap().await.unwrap()
            });
            let client = client.unwrap();
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(&[0]).await.unwrap();
            let (mut host_send, mut host_recv) = server.accept_bi().await.unwrap();
            let mut probe = [0];
            host_recv.read_exact(&mut probe).await.unwrap();
            host_send
                .write_all(&(FRAME_MAX as u32).to_be_bytes())
                .await
                .unwrap();
            let actual = transport::read_frame(&mut recv, FRAME_MAX - 1).await;
            let original = actual
                .as_ref()
                .err()
                .and_then(transport::FrameFailure::from_error);
            assert!(
                matches!(
                    original,
                    Some(transport::FrameFailure {
                        stage: transport::FrameStage::FrameCap,
                        ..
                    })
                ),
                "fixture did not retain actual FrameCap failure"
            );
            // This barrier retains the ACTUAL Iroh/read_frame result, not a fabricated error.
            let released = Cell::new(false);
            let mut result = Some(actual);
            let retained = std::future::poll_fn(|_| {
                if released.get() {
                    std::task::Poll::Ready(result.take().expect("one actual reader result"))
                } else {
                    std::task::Poll::Pending
                }
            })
            .fuse();
            tokio::pin!(retained);
            let target = target(&generation_snapshot().state.summary);
            let subject = GridSubject::Pane(target.clone());
            let subscription = Uuid::random();
            let opening = Uuid::random();
            let mut witness =
                GridWitness::pending(opening.clone(), GridSubject::Pane(target.clone()));
            let response = Envelope::Response(Response {
                id: opening,
                outcome: Ok(ResultBody::TerminalGridSubscribe(
                    TerminalGridSubscribeResult {
                        target,
                        subscription: subscription.clone(),
                        seq: ZeroValue(0),
                        frame: GridBegin { scroll: None, mode: GridMode::Replace,
                        revision: Uint(0),
                        columns: Uint(1),
                        rows: Uint(1),
                        cursor: None,
                        alternate_screen: false,
                        graphics_omitted: true,
                        popup_blocked: false,
                        part_count: Uint(1), },
                    },
                )),
            });
            let bytes = encode_payload(&response).unwrap();
            let Envelope::Response(response) =
                decode_payload(&bytes, Some(Method::TerminalGridSubscribe)).unwrap()
            else {
                panic!("opening family");
            };
            witness
                .response(&response.id, &response.outcome, bytes.len())
                .unwrap();
            let cell:GridCell=serde_json::from_value(json!({"text":"x","width":1,"foreground":{"kind":"default"},
                "background":{"kind":"default"},"attributes":{"bold":false,"faint":false,"italic":false,"blink":false,"strike":false,"underline":"none"}})).unwrap();
            for (seq, body) in [
                (
                    1,
                    EventBody::GridPart(GridPart {
                        revision: Uint(0),
                        index: Uint(0),
                        spans: vec![GridSpan {
                            row: Uint(0),
                            column: Uint(0),
                            cells: vec![cell],
                        }],
                    }),
                ),
                (2, EventBody::GridCommit(GridCommit { revision: Uint(0) })),
            ] {
                let bytes = encode_payload(&Envelope::Event(Event {
                    subscription: subscription.clone(),
                    seq: Uint(seq),
                    body,
                }))
                .unwrap();
                let Envelope::Event(event) = decode_payload(&bytes, None).unwrap() else {
                    panic!("grid family");
                };
                witness.event(event, bytes.len()).unwrap();
            }
            let grid = Some(witness);
            let run = Uuid::random();
            let ready = Uuid::random();
            let mut loss = GridSourceLoss {
                run: run.clone(),
                ready: ready.clone(),
                trigger: None,
                native_eof: false,
                iroh_closed: false,
                sync_required: false,
                deadline: None,
            };
            loss.arm(GridSourceIntent {
                version: 1,
                run,
                ready,
                trigger: Uuid::random(),
            })
            .unwrap();
            let (native, writer) = tokio::net::UnixStream::pair().unwrap();
            drop(writer);
            let mut native = tokio::io::BufReader::new(native);
            let mut count = 0;
            assert!(
                grid_native_line(
                    &mut native,
                    tokio::time::Instant::now() + Duration::from_secs(15),
                    &mut count
                )
                .await
                .unwrap()
                .is_none(),
                "actual independent native EOF missing"
            );
            loss.upstream_eof().unwrap();
            server.close(0u32.into(), b"owned combined regression complete");
            let actual_closed = client.closed().await;
            let wake = grid_source_wake(
                std::future::pending::<()>(),
                std::future::ready(actual_closed),
                std::future::pending::<()>(),
                std::future::pending::<()>(),
                retained.as_mut(),
                std::future::pending::<()>(),
            )
            .await;
            assert!(
                matches!(wake, GridSourceWake::Closed),
                "actual closure fact missing"
            );
            if final_settlement {
                match grid_source_handoff(
                    retained.as_mut(),
                    false,
                    &loss,
                    &grid,
                    &subscription,
                    &subject,
                )
                .unwrap()
                {
                    GridSourceHandoff::Pending { settled: false } => {
                        loss.downstream(false).unwrap()
                    }
                    _ => panic!("barrier was not pending at closure acceptance"),
                }
                assert!(loss.complete(), "final-settlement facts not present");
            }
            released.set(true);
            let mut observed = false;
            let mut original_fatal = false;
            match grid_source_handoff(
                retained.as_mut(),
                false,
                &loss,
                &grid,
                &subscription,
                &subject,
            )
            .unwrap()
            {
                GridSourceHandoff::Frame(Err(error)) => {
                    let error = grid_source_fin(true, error)
                        .err()
                        .expect("original FrameCap must remain fatal");
                    assert!(
                        transport::FrameFailure::from_error(&error) == original,
                        "original closed failure replaced"
                    );
                    original_fatal = true;
                }
                GridSourceHandoff::Pending { settled } => observed = settled,
                _ => panic!("actual fatal reader result disappeared"),
            }
            assert!(
                original_fatal && !observed,
                "closure/final settlement erased an already-ready actual frame failure"
            );
            client.close(0u32.into(), b"owned regression complete");
            goatr::close_endpoint(&peer).await.unwrap();
            goatr::close_endpoint(&host).await.unwrap();
        }
    }
    #[test]
    fn grid_private_codec_route_commits_without_history_or_stale_reappearance() {
        const TOKEN: &str = "SAFEGRIDBOUNDARY";
        let snapshot = generation_snapshot();
        let expected = target(&snapshot.state.summary);
        let mut history = Some(GenerationView::new(snapshot, &expected).unwrap());
        let original_history_seq = history.as_ref().unwrap().snapshot.seq.0;
        let opening = Uuid::random();
        let subscription = Uuid::random();
        let frame = GridBegin { scroll: None, mode: GridMode::Replace,
        revision: Uint(0),
        columns: Uint(TOKEN.len() as u64),
        rows: Uint(1),
        cursor: None,
        alternate_screen: false,
        graphics_omitted: true,
        popup_blocked: false,
        part_count: Uint(1), };
        let response = Envelope::Response(Response {
            id: opening.clone(),
            outcome: Ok(ResultBody::TerminalGridSubscribe(
                TerminalGridSubscribeResult {
                    target: expected.clone(),
                    subscription: subscription.clone(),
                    seq: ZeroValue(0),
                    frame: frame.clone(),
                },
            )),
        });
        let encoded = encode_payload(&response).unwrap();
        let Envelope::Response(decoded) =
            decode_payload(&encoded, Some(Method::TerminalGridSubscribe)).unwrap()
        else {
            panic!("response family");
        };
        let mut witness =
            GridWitness::pending(opening.clone(), GridSubject::Pane(expected.clone()));
        witness.arm(TOKEN).unwrap();
        witness
            .response(&decoded.id, &decoded.outcome, encoded.len())
            .unwrap();
        let mut grid = Some(witness);
        let mut events = 0;
        let part = GridPart {
            revision: Uint(0),
            index: Uint(0),
            spans: vec![GridSpan {
                row: Uint(0),
                column: Uint(0),
                cells: TOKEN
                    .chars()
                    .map(|c| GridCell {
                        text: c.to_string(),
                        width: Uint(1),
                        foreground: GridColor::Default,
                        background: GridColor::Default,
                        attributes: GridAttributes {
                            bold: false,
                            faint: false,
                            italic: false,
                            blink: false,
                            strike: false,
                            inverse: false,
                            concealed: false,
                            underline: GridUnderline::None,
                            underline_color: None,
                        },
                    })
                    .collect(),
            }],
        };
        let mut private = Envelope::Event(Event {
            subscription: subscription.clone(),
            seq: Uint(1),
            body: EventBody::GridPart(part),
        });
        let encoded = encode_payload(&private).unwrap();
        let Envelope::Event(decoded) = decode_payload(&encoded, None).unwrap() else {
            panic!("event family");
        };
        Phone::route_event(&mut grid, &mut events, &mut history, decoded, encoded.len()).unwrap();
        assert!(
            events == 0
                && history.as_ref().unwrap().snapshot.seq.0 == original_history_seq
                && history.as_ref().unwrap().snapshot.history.is_empty(),
            "private route changed history"
        );
        assert!(
            grid.as_ref().unwrap().token_commit.is_none(),
            "part published tentative token"
        );
        let commit = Envelope::Event(Event {
            subscription: subscription.clone(),
            seq: Uint(2),
            body: EventBody::GridCommit(GridCommit { revision: Uint(0) }),
        });
        let encoded = encode_payload(&commit).unwrap();
        let Envelope::Event(decoded) = decode_payload(&encoded, None).unwrap() else {
            panic!("commit family");
        };
        Phone::route_event(&mut grid, &mut events, &mut history, decoded, encoded.len()).unwrap();
        assert!(
            grid.as_ref().unwrap().token_commit == Some(0),
            "complete commit did not promote token"
        );
        grid.as_mut().unwrap().retired = true; // production unsubscribe ACK retires before its RPC drain
        if let Envelope::Event(event) = &mut private {
            event.seq = Uint(3);
        }
        let encoded = encode_payload(&private).unwrap();
        let Envelope::Event(decoded) = decode_payload(&encoded, None).unwrap() else {
            panic!("stale event family");
        };
        assert!(
            Phone::route_event(&mut grid, &mut events, &mut history, decoded, encoded.len())
                .is_err(),
            "retired private data accepted"
        );
        assert!(
            events == 0 && history.as_ref().unwrap().snapshot.history.is_empty(),
            "stale data reached history"
        );
        // A fault before opening followed by a queued success retains the original correlation,
        // captures only its subscription for the original connection's teardown, never retargets.
        let mut canceled = GridWitness::pending(opening, GridSubject::Pane(expected));
        let wrong = Envelope::Event(Event {
            subscription: Uuid::random(),
            seq: Uint(1),
            body: EventBody::GridBegin(GridBeginEvent { frame }),
        });
        let bytes = encode_payload(&wrong).unwrap();
        let Envelope::Event(event) = decode_payload(&bytes, None).unwrap() else {
            panic!("fault family");
        };
        assert!(
            canceled.event(event, bytes.len()).is_err(),
            "unowned opening data accepted"
        );
        let bytes = encode_payload(&response).unwrap();
        let Envelope::Response(response) =
            decode_payload(&bytes, Some(Method::TerminalGridSubscribe)).unwrap()
        else {
            panic!("late response family");
        };
        assert!(
            canceled
                .response(&response.id, &response.outcome, bytes.len())
                .is_err(),
            "retired opening revived"
        );
        assert!(
            canceled.retired && canceled.subscription.is_some() && canceled.committed.is_none(),
            "late response changed original ownership"
        );
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
