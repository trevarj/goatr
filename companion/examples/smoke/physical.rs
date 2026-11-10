// SPDX-License-Identifier: GPL-3.0-or-later
//! Stock-only physical acceptance. USB carries signed control, never the Iroh data plane.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{MetadataExt, PermissionsExt},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command as AsyncCommand,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::timeout,
};

const APP: &str = "io.github.trevarj.goatr.debug";
const TEST: &str = "io.github.trevarj.goatr.debug.test";
const VERIFY: &str = "io.github.trevarj.goatr.debug.verify";
const PRODUCT_RUNNER: &str =
    "io.github.trevarj.goatr.debug.test/io.github.trevarj.goatr.OwnedInstrumentationRunner";
const UI_RUNNER: &str =
    "io.github.trevarj.goatr.debug.test/io.github.trevarj.goatr.OwnedUiAutomationRunner";
const MAX_CONTROL: usize = 131_072;
const MVP_AARS: [(&str, &str, &str); 5] = [
    (
        "androidx.startup",
        "startup-runtime/1.1.1",
        "e0a6329a371262fe4c450372b70fdaf33b769ef6917094723787cfce896b1dd3",
    ),
    (
        "androidx.lifecycle",
        "lifecycle-process/2.11.0",
        "9743f0c7baf27d34497b7e9a1cd497d8260251b03124507963abfaad66c3f552",
    ),
    (
        "androidx.emoji2",
        "emoji2/1.4.0",
        "433febd3434a45667176c76a64f3f205ca6335a6b544c5b5d57f25a38a375242",
    ),
    (
        "androidx.profileinstaller",
        "profileinstaller/1.4.0",
        "d502141fcce902430f62b674c32aecd0f75262e7ee2cd15c74adb617cd13130a",
    ),
    (
        "org.unifiedpush.android",
        "connector/3.3.5",
        "fa017cdfabbdc9af021e1f8c8eff2c8098e701d7a2c599937dce0b4ecf1e929b",
    ),
];
const INSTALLED_HERDR: &str = "/nix/store/gpsmifw68ssar7p2gqikvl4c1b0xp9mv-herdr-0.9.1/bin/herdr";
const INSTALLED_HERDR_SHA256: &str =
    "71add50d05b0cf06c04caf72727bfe9fcedac986e9eba78b8a0ef8332269ee0c";
fn complete_stamp(m: &fs::Metadata) -> (u64, u64, u64, u32, u32, u32, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mode(),
        m.uid(),
        m.gid(),
        m.nlink(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
fn seed_source_identities(config: &Config) -> io::Result<()> {
    if fs::canonicalize(&config.executables.herdr)? != Path::new(INSTALLED_HERDR)
        || binary_sha256(Path::new(INSTALLED_HERDR))? != INSTALLED_HERDR_SHA256
        || config.executables.omp.as_deref()
            != Some(Path::new(
                "/nix/store/jnxpnwp4fgia3h02m55921n3rjsfm3g7-omp/bin/omp",
            ))
        || config.executables.codex.as_deref()
            != Some(Path::new(
                "/nix/store/qhb4isrj1vnddb3jm2s0w8383ig1d818-codex/bin/codex",
            ))
        || config.omp_registry_dir.as_deref() != Some(Path::new("/home/trev/.omp/run/collab-hosts"))
        || config.omp_session_roots.len() != 1
        || config.omp_session_roots.first().map(PathBuf::as_path)
            != Some(Path::new("/home/trev/.omp/agent/sessions"))
        || config.omp_blob_root.as_deref() != Some(Path::new("/home/trev/.omp/agent/blobs"))
        || config.omp_skills_package_root.as_deref()
            != Some(Path::new(
                "/home/trev/.local/share/npm/lib/node_modules/@oh-my-pi/pi-coding-agent",
            ))
    {
        return Err(prerequisite(
            "independently_qualified_seed_identities_unavailable",
        ));
    }
    for path in [
        Some(&config.executables.herdr),
        config.executables.omp.as_ref(),
        config.executables.codex.as_ref(),
    ] {
        goatr::config::executable(
            path.ok_or_else(|| prerequisite("seed_provider_wrapper_unavailable"))?,
        )?;
    }
    Ok(())
}
fn read_pinned_config(path: &Path, caller_sha: &str) -> io::Result<(Config, File, fs::Metadata)> {
    if !hex64(caller_sha) {
        return Err(prerequisite("independent_seed_config_pin_unavailable"));
    }
    goatr::config::absolute(path)?;
    if fs::canonicalize(path).map_err(|_| prerequisite("independently_pinned_seed_unavailable"))?
        != path
    {
        return Err(failure("physical seed config is not canonical"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| failure("physical seed config has no private parent"))?;
    private_dir(parent, false)?;
    let parent_before = fs::symlink_metadata(parent)?;
    private_file(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let before = file.metadata()?;
    if before.len() > goatr::config::CONFIG_BYTES as u64
        || complete_stamp(&before) != complete_stamp(&fs::symlink_metadata(path)?)
    {
        return Err(failure("physical seed config changed before read"));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&mut file)
        .take((goatr::config::CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    private_file(path)?;
    private_dir(parent, false)?;
    if bytes.len() > goatr::config::CONFIG_BYTES
        || complete_stamp(&before) != complete_stamp(&file.metadata()?)
        || complete_stamp(&before) != complete_stamp(&fs::symlink_metadata(path)?)
        || complete_stamp(&parent_before) != complete_stamp(&fs::symlink_metadata(parent)?)
        || fs::canonicalize(path)? != path
    {
        return Err(failure("physical seed config changed across its held read"));
    }
    use sha2::{Digest, Sha256};
    if hex(&Sha256::digest(&bytes)) != caller_sha {
        return Err(prerequisite("independently_pinned_seed_hash_mismatch"));
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| failure("physical seed config is not UTF-8"))?;
    Ok((Config::parse(text)?, file, before))
}
fn neutral_seed_policy(config: &Config, parent: &Path) -> io::Result<()> {
    if config.pairing_name.is_some()
        || config.default_codex_owner_id.is_some()
        || !config.herdr_sockets.is_empty()
        || !config.projects.is_empty()
        || !config.codex_owners.is_empty()
        || !config.skill_document_roots.is_empty()
        || config.allow_attachments
        || !config.allow_scoped_shell_probe
        || !config.allow_terminal_control
        || !config.allow_omp_controls
        || config.push_allowed_origins.len() != 1
        || config.push_allowed_origins.first().map(String::as_str) != Some("https://ntfy.sh")
    {
        return Err(failure(
            "physical seed must be neutral with only the required control consents and public push origin",
        ));
    }
    if config.runtime_dir != config.runtime_dir_agent
        || config.runtime_dir == parent
        || !config.runtime_dir.starts_with(parent)
    {
        return Err(failure(
            "physical seed runtime must be identical and strictly beneath its private parent",
        ));
    }
    private_dir(parent, false)?;
    private_dir(&config.runtime_dir, false)?;
    Ok(())
}
fn qualified_configuration(
    path: &Path,
    caller_sha: &str,
) -> io::Result<(Config, File, fs::Metadata)> {
    let (config, file, original) = read_pinned_config(path, caller_sha)?;
    config.preflight()?;
    neutral_seed_policy(
        &config,
        path.parent()
            .ok_or_else(|| failure("physical seed parent is absent"))?,
    )?;
    seed_source_identities(&config)?;
    Ok((config, file, original))
}
fn descriptor_sha256(file: &mut File, length: u64) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 8192];
    let mut total = 0u64;
    let mut reader = file.take(
        length
            .checked_add(1)
            .ok_or_else(|| failure("physical frozen input size is invalid"))?,
    );
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        digest.update(&buffer[..count]);
    }
    if total != length {
        return Err(failure(
            "physical frozen held input changed length while hashing",
        ));
    }
    Ok(hex(&digest.finalize()))
}
fn frozen_input(path: &Path, expected: &str) -> io::Result<(PathBuf, File, fs::Metadata)> {
    let path = canonical_file(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)?;
    let original = file.metadata()?;
    if !original.is_file() {
        return Err(failure("physical frozen descriptor is not a regular file"));
    }
    let sha = descriptor_sha256(&mut file, original.len())?;
    let pin = (path, file, original);
    recheck_inputs(std::slice::from_ref(&pin))?;
    if !hex64(expected) || sha != expected {
        return Err(failure(
            "physical held original input does not match its independent caller SHA-256",
        ));
    }
    Ok(pin)
}

fn object<'a>(value: &'a Value, fields: &[&str]) -> io::Result<&'a serde_json::Map<String, Value>> {
    let map = value
        .as_object()
        .ok_or_else(|| failure("physical report is not an object"))?;
    if map.len() != fields.len() || fields.iter().any(|field| !map.contains_key(*field)) {
        return Err(failure("physical report has an unknown or missing field"));
    }
    Ok(map)
}
fn string<'a>(value: &'a Value, key: &str) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 512)
        .ok_or_else(|| failure("physical report string is missing or unbounded"))
}
fn boolean(value: &Value, key: &str) -> io::Result<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| failure("physical report boolean is missing"))
}
fn unsigned(value: &Value, key: &str) -> io::Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| failure("physical report integer is missing"))
}
fn array<'a>(value: &'a Value, key: &str, bound: usize) -> io::Result<&'a Vec<Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .filter(|values| values.len() <= bound)
        .ok_or_else(|| failure("physical report array is missing or unbounded"))
}
fn hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn uuid_text(value: &str) -> io::Result<()> {
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| failure("physical UUID is invalid"))?;
    let mut canonical = [0u8; 36];
    if parsed.get_variant() != uuid::Variant::RFC4122
        || parsed.hyphenated().encode_lower(&mut canonical) != value
    {
        return Err(failure("physical UUID is not canonical RFC 4122"));
    }
    Ok(())
}
fn id(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    {
        return Err(failure("physical report identifier is invalid"));
    }
    Ok(())
}
fn canonical_file(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if !metadata.is_file() || metadata.mode() & 0o022 != 0 {
        return Err(failure("physical input is not a protected regular file"));
    }
    Ok(canonical)
}
fn canonical_directory(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
        return Err(failure("physical input directory is not protected"));
    }
    Ok(canonical)
}
fn private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
        || !metadata.is_dir()
    {
        return Err(failure("physical private directory ownership changed"));
    }
    Ok(())
}
fn protected_tool(path: &Path) -> io::Result<PathBuf> {
    goatr::config::executable(path)?;
    Ok(path.to_path_buf())
}
fn resolve_tool(name: &str) -> io::Result<PathBuf> {
    for directory in
        std::env::split_paths(&std::env::var_os("PATH").ok_or_else(|| failure("PATH unavailable"))?)
    {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(name);
        if candidate.exists() {
            return protected_tool(&candidate);
        }
    }
    Err(failure("physical required protected tool is unavailable"))
}
async fn output(
    mut command: AsyncCommand,
    limit: usize,
    deadline: Duration,
) -> io::Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    // SAFETY: the hook only clears capabilities with async-signal-safe syscalls.
    unsafe {
        command.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
    }
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failure("physical command has no output pipe"))?;
    let mut bytes = Vec::new();
    timeout(deadline, async {
        stdout
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > limit {
            return Err(failure("physical command output exceeds bound"));
        }
        if !child.wait().await?.success() {
            return Err(failure("physical command failed"));
        }
        Ok(())
    })
    .await
    .map_err(|_| failure("physical command deadline expired"))??;
    Ok(bytes)
}
fn text(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| failure("physical command output is not UTF-8"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ControllerPolicy<'a> {
    Zero,
    WakeDiagnostic { unit: &'a str },
    UnitWake { unit: &'a str },
}
pub(super) fn validate_controller_unit(unit: &str) -> io::Result<()> {
    let id = unit
        .strip_prefix("goatr-host-wake-")
        .and_then(|value| value.strip_suffix(".service"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Wake diagnostic unit is not canonical",
            )
        })?;
    uuid_text(id).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Wake diagnostic unit is not canonical",
        )
    })
}
struct HostAuthority {
    _socket: OwnedFd,
    executable: File,
    caller: ProcessEvidence,
    manager: ProcessEvidence,
    facts: Value,
    controller_unit: Option<goatr::codex_relay::UnitWakePin>,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
struct HostCaps {
    #[serde(rename = "CapInh")]
    inheritable: String,
    #[serde(rename = "CapPrm")]
    permitted: String,
    #[serde(rename = "CapEff")]
    effective: String,
    #[serde(rename = "CapAmb")]
    ambient: String,
}
#[derive(Default, Debug, PartialEq, Eq)]
struct HostDiagnostic {
    guard: &'static str,
    errno: Option<i32>,
    caller_caps: Option<HostCaps>,
    manager_caps: Option<HostCaps>,
    proc_view_validated: bool,
    controller_unit: Option<String>,
    controller_unit_validated: bool,
    manager_command_observed: bool,
}
#[track_caller]
fn host_guard(
    guard: &'static str,
    error: io::Error,
    diagnostic: Option<&mut HostDiagnostic>,
) -> io::Error {
    let errno = error.raw_os_error();
    if let Some(diagnostic) = diagnostic {
        diagnostic.guard = guard;
        diagnostic.errno = errno;
    }
    super::super::located_message(
        error.kind(),
        &format!(
            "guard={guard}; errno={}; no mutation",
            errno.map_or_else(|| "none".to_owned(), |errno| errno.to_string())
        ),
        super::super::helper_error_location(&error),
    )
}
fn host_probe_read(
    path: &Path,
    guard: &'static str,
    diagnostic: &mut HostDiagnostic,
) -> io::Result<()> {
    let file =
        File::open(path).map_err(|error| host_guard(guard, error, Some(&mut *diagnostic)))?;
    let count = io::copy(&mut file.take(65_537), &mut io::sink())
        .map_err(|error| host_guard(guard, error, Some(&mut *diagnostic)))?;
    if count > 65_536 {
        return Err(host_guard(
            guard,
            failure("physical bounded proc read exceeded its limit"),
            Some(diagnostic),
        ));
    }
    Ok(())
}
fn host_probe_bytes(
    path: &Path,
    guard: &'static str,
    diagnostic: &mut HostDiagnostic,
) -> io::Result<Vec<u8>> {
    let file =
        File::open(path).map_err(|error| host_guard(guard, error, Some(&mut *diagnostic)))?;
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|error| host_guard(guard, error, Some(&mut *diagnostic)))?;
    if bytes.len() > 65_536 {
        return Err(host_guard(
            guard,
            failure("physical bounded proc read exceeded its limit"),
            Some(diagnostic),
        ));
    }
    Ok(bytes)
}
fn host_caps(status: &[u8]) -> io::Result<HostCaps> {
    let status = std::str::from_utf8(status).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "physical capability fields are malformed",
        )
    })?;
    let mut masks = [None; 4];
    for line in status.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let index = match key {
            "CapInh" => 0,
            "CapPrm" => 1,
            "CapEff" => 2,
            "CapAmb" => 3,
            _ => continue,
        };
        let value = value.trim_matches(|character| matches!(character, ' ' | '\t'));
        if masks[index].is_some()
            || value.len() != 16
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "physical capability fields are malformed or repeated",
            ));
        }
        masks[index] = Some(value);
    }
    let [
        Some(inheritable),
        Some(permitted),
        Some(effective),
        Some(ambient),
    ] = masks
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "physical capability fields are missing",
        ));
    };
    Ok(HostCaps {
        inheritable: inheritable.to_owned(),
        permitted: permitted.to_owned(),
        effective: effective.to_owned(),
        ambient: ambient.to_owned(),
    })
}
fn host_status(
    path: &Path,
    guard: &'static str,
    manager: bool,
    diagnostic: &mut HostDiagnostic,
) -> io::Result<Vec<u8>> {
    if manager {
        diagnostic.manager_caps = None;
    } else {
        diagnostic.caller_caps = None;
    }
    let bytes = host_probe_bytes(path, guard, diagnostic)?;
    let parse_guard = if manager {
        "manager_cap_fields"
    } else {
        "caller_cap_fields"
    };
    let caps = host_caps(&bytes)
        .map_err(|error| host_guard(parse_guard, error, Some(&mut *diagnostic)))?;
    if manager {
        diagnostic.manager_caps = Some(caps);
    } else {
        diagnostic.caller_caps = Some(caps);
    }
    Ok(bytes)
}
fn host_validate_proc_view(
    stat: &[u8],
    status: &[u8],
    diagnostic: &mut HostDiagnostic,
) -> io::Result<()> {
    diagnostic.proc_view_validated = false;
    let validate = || -> io::Result<()> {
        let actual = std::process::id();
        if u32::try_from(unsafe { libc::getpid() }).ok() != Some(actual) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "physical actual caller PID is unavailable",
            ));
        }
        let stat = std::str::from_utf8(stat).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "physical caller proc view is malformed",
            )
        })?;
        let status = std::str::from_utf8(status).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "physical caller proc view is malformed",
            )
        })?;
        herdr::validate_procfs_view(stat, status, actual)
    };
    validate().map_err(|error| host_guard("caller_procfs_view", error, Some(&mut *diagnostic)))?;
    diagnostic.proc_view_validated = true;
    Ok(())
}
fn host_manager_reads(pid: u32, diagnostic: &mut HostDiagnostic) -> io::Result<()> {
    if !diagnostic.proc_view_validated {
        return Err(host_guard(
            "manager_procfs_view_required",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "physical numeric proc path requires this invocation's checked caller view",
            ),
            Some(diagnostic),
        ));
    }
    // No numeric pathname is even formed until the /proc/self numbering proof above.
    let root = PathBuf::from(format!("/proc/{pid}"));
    fs::metadata(&root)
        .map_err(|error| host_guard("manager_proc_stat", error, Some(&mut *diagnostic)))?;
    host_probe_read(&root.join("stat"), "manager_stat_read", diagnostic)?;
    host_status(
        &root.join("status"),
        "manager_status_read",
        true,
        diagnostic,
    )?;
    for (name, guard) in [
        ("pid", "manager_ns_pid_stat"),
        ("mnt", "manager_ns_mnt_stat"),
        ("user", "manager_ns_user_stat"),
    ] {
        fs::metadata(root.join("ns").join(name))
            .map_err(|error| host_guard(guard, error, Some(&mut *diagnostic)))?;
    }
    Ok(())
}
fn host_proc_reads(pid: u32, manager: bool, diagnostic: &mut HostDiagnostic) -> io::Result<()> {
    // Revalidate each initial/final capture. Failed reads cannot retain an earlier view/sample.
    diagnostic.proc_view_validated = false;
    diagnostic.caller_caps = None;
    if manager {
        diagnostic.manager_caps = None;
    }
    let stat = host_probe_bytes(Path::new("/proc/self/stat"), "self_stat_read", diagnostic)?;
    let status = host_status(
        Path::new("/proc/self/status"),
        "self_status_read",
        false,
        diagnostic,
    )?;
    host_validate_proc_view(&stat, &status, diagnostic)?;
    drop(stat);
    drop(status); // Only the four closed masks survive the numbering check.
    for (path, guard) in [
        ("/proc/self/ns/pid", "self_ns_pid_stat"),
        ("/proc/self/ns/mnt", "self_ns_mnt_stat"),
        ("/proc/self/ns/user", "self_ns_user_stat"),
    ] {
        fs::metadata(path).map_err(|error| host_guard(guard, error, Some(&mut *diagnostic)))?;
    }
    fs::metadata("/proc/self")
        .map_err(|error| host_guard("caller_proc_stat", error, Some(&mut *diagnostic)))?;
    host_probe_read(
        Path::new("/proc/sys/kernel/random/boot_id"),
        "boot_id_read",
        diagnostic,
    )?;
    if manager {
        host_manager_reads(pid, diagnostic)?;
    }
    Ok(())
}
fn host_process_evidence(
    pid: u32,
    manager: bool,
    guard: &'static str,
    mut diagnostic: Option<&mut HostDiagnostic>,
) -> io::Result<ProcessEvidence> {
    if let Some(diagnostic) = diagnostic.as_deref_mut() {
        host_proc_reads(pid, manager, diagnostic)?;
    }
    // Always run the complete original capture after diagnostic reads; never use their results.
    herdr::process_evidence(pid).map_err(|error| host_guard(guard, error, diagnostic))
}
fn controller_environment(unsandboxed: bool, sandboxed: bool) -> io::Result<()> {
    if unsandboxed || sandboxed {
        return Err(prerequisite(
            "physical controller requires the direct authorized host context, not an inherited sandbox/unsandboxed override",
        ));
    }
    Ok(())
}
fn controller_caps_admission(policy: ControllerPolicy<'_>, status: &[u8]) -> io::Result<()> {
    match policy {
        ControllerPolicy::UnitWake { .. } | ControllerPolicy::WakeDiagnostic { .. } => {
            let status = std::str::from_utf8(status)
                .map_err(|_| failure("unit capability status is malformed"))?;
            if !goatr::codex_relay::capability_state(status)?.unit_wake() {
                return Err(failure("unit capability/NNP shape is invalid"));
            }
            Ok(())
        }
        ControllerPolicy::Zero => {
            let caps = host_caps(status)?;
            let expected = if matches!(policy, ControllerPolicy::Zero) {
                "0000000000000000"
            } else {
                "0000000800000000"
            };
            if [
                &caps.inheritable,
                &caps.permitted,
                &caps.effective,
                &caps.ambient,
            ]
            .into_iter()
            .any(|value| value.as_str() != expected)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "controller capability policy refused the actual mask shape",
                ));
            }
            Ok(())
        }
    }
}
fn controller_cgroup_admission(
    expected: &str,
    bytes: &[u8],
    diagnostic: &mut HostDiagnostic,
) -> io::Result<()> {
    diagnostic.controller_unit = None;
    diagnostic.controller_unit_validated = false;
    let validate = || -> io::Result<()> {
        validate_controller_unit(expected)?;
        let text = std::str::from_utf8(bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "controller cgroup is malformed")
        })?;
        goatr::codex_relay::unit_cgroup(text, expected)?;
        Ok(())
    };
    validate()
        .map_err(|error| host_guard("wake_controller_cgroup", error, Some(&mut *diagnostic)))?;
    diagnostic.controller_unit = Some(expected.to_owned());
    diagnostic.controller_unit_validated = true;
    // Historical effect observation is sticky; a failed final check clears CURRENT proof only.
    diagnostic.manager_command_observed = true;
    Ok(())
}
fn controller_cgroup_read(
    unit: &str,
    path: &Path,
    diagnostic: &mut HostDiagnostic,
) -> io::Result<()> {
    diagnostic.controller_unit = None;
    diagnostic.controller_unit_validated = false;
    let bytes = host_probe_bytes(path, "wake_controller_cgroup_read", diagnostic)?;
    controller_cgroup_admission(unit, &bytes, diagnostic)
}
fn controller_admission(
    policy: ControllerPolicy<'_>,
    caller: &ProcessEvidence,
    diagnostic: Option<&mut HostDiagnostic>,
) -> io::Result<()> {
    match policy {
        ControllerPolicy::Zero => dev_continuity::zero_native_caps(caller),
        ControllerPolicy::UnitWake { unit } => {
            if !goatr::codex_relay::current_capabilities()?.unit_wake()
                || goatr::codex_relay::unit_cgroup(&fs::read_to_string("/proc/self/cgroup")?, unit)?
                    .is_empty()
            {
                return Err(failure("unit Wake controller caps/NNP/full cgroup changed"));
            }
            Ok(())
        }
        ControllerPolicy::WakeDiagnostic { unit } => {
            let diagnostic = diagnostic.ok_or_else(|| {
                host_guard(
                    "wake_diagnostic_context",
                    failure("Wake policy is diagnostic only"),
                    None,
                )
            })?;
            diagnostic.controller_unit = None;
            diagnostic.controller_unit_validated = false;
            validate_controller_unit(unit).map_err(|error| {
                host_guard("wake_controller_unit", error, Some(&mut *diagnostic))
            })?;
            if !diagnostic.proc_view_validated {
                return Err(host_guard(
                    "wake_controller_procfs_view",
                    failure("Wake diagnostic requires the current checked caller view"),
                    Some(diagnostic),
                ));
            }
            // Fresh real admission read: never use the earlier diagnostic display samples as proof.
            let status = host_probe_bytes(
                Path::new("/proc/self/status"),
                "wake_controller_status_read",
                diagnostic,
            )?;
            controller_caps_admission(policy, &status).map_err(|error| {
                host_guard(
                    "wake_controller_capabilities",
                    error,
                    Some(&mut *diagnostic),
                )
            })?;
            drop(status);
            controller_cgroup_read(unit, Path::new("/proc/self/cgroup"), diagnostic)
        }
    }
}
fn same_host_namespace_group(caller: &ProcessEvidence, manager: &ProcessEvidence) -> bool {
    caller.linux_boot_id == manager.linux_boot_id
        && caller.namespaces.pid == manager.namespaces.pid
        && caller.namespaces.mount == manager.namespaces.mount
        && caller.namespaces.user == manager.namespaces.user
        && !caller.namespaces.nspids.is_empty()
        && caller.namespaces.nspids.len() == manager.namespaces.nspids.len()
}
fn recheck_host_processes(
    caller: &ProcessEvidence,
    manager: &ProcessEvidence,
    mut diagnostic: Option<&mut HostDiagnostic>,
) -> io::Result<()> {
    if caller.pid != std::process::id()
        || host_process_evidence(
            caller.pid,
            false,
            "final_caller_evidence",
            diagnostic.as_deref_mut(),
        )? != *caller
        || host_process_evidence(
            manager.pid,
            true,
            "final_manager_evidence",
            diagnostic.as_deref_mut(),
        )? != *manager
        || !same_host_namespace_group(caller, manager)
    {
        return Err(host_guard(
            "original_process_identity",
            failure("physical original caller/manager process or namespace authority changed"),
            diagnostic,
        ));
    }
    Ok(())
}
/// Qualification only: no manager command, creation/restart or capability/namespace changes.
fn original_host_authority(
    policy: ControllerPolicy<'_>,
    mut diagnostic: Option<&mut HostDiagnostic>,
    admitted: Option<goatr::codex_relay::UnitWakePin>,
) -> io::Result<HostAuthority> {
    if let Some(diagnostic) = diagnostic.as_deref_mut() {
        *diagnostic = HostDiagnostic::default();
    }
    controller_environment(
        std::env::var_os("OMP_UNSANDBOXED").is_some(),
        std::env::var_os("AGENT_SANDBOXED").is_some(),
    )
    .map_err(|error| host_guard("controller_environment", error, diagnostic.as_deref_mut()))?;
    let controller_unit = match policy {
        ControllerPolicy::UnitWake { unit } | ControllerPolicy::WakeDiagnostic { unit } => {
            let pin =
                admitted.ok_or_else(|| failure("pre-runtime qualified unit context missing"))?;
            if pin.unit() != unit || pin.process().pid != std::process::id() {
                return Err(failure("admitted original unit controller differs"));
            }
            pin.recheck(None)?;
            Some(pin)
        }
        ControllerPolicy::Zero => {
            if admitted.is_some() {
                return Err(failure("zero mode cannot retain unit authority"));
            }
            None
        }
    };
    let caller = host_process_evidence(
        std::process::id(),
        false,
        "caller_evidence",
        diagnostic.as_deref_mut(),
    )?;
    controller_admission(policy, &caller, diagnostic.as_deref_mut()).map_err(|error| {
        if matches!(policy, ControllerPolicy::Zero) {
            host_guard("caller_capabilities", error, diagnostic.as_deref_mut())
        } else {
            error
        }
    })?;
    if unsafe { libc::geteuid() } != 1000
        || std::env::consts::ARCH != "x86_64"
        || std::env::consts::OS != "linux"
    {
        return Err(host_guard(
            "host_platform",
            failure(
                "physical original-host prerequisite requires the qualified Linux UID 1000 host",
            ),
            diagnostic,
        ));
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path == Path::new("/run/user/1000"))
        .ok_or_else(|| {
            host_guard(
                "runtime_selection",
                failure("physical original-host runtime is unavailable"),
                diagnostic.as_deref_mut(),
            )
        })?;
    goatr::pairing::private_dir(&runtime, false)
        .map_err(|error| host_guard("runtime_directory", error, diagnostic.as_deref_mut()))?;
    let socket = runtime.join("systemd/private");
    let before = fs::symlink_metadata(&socket)
        .map_err(|error| host_guard("socket_initial_stat", error, diagnostic.as_deref_mut()))?;
    if !std::os::unix::fs::FileTypeExt::is_socket(&before.file_type())
        || before.uid() != 1000
        || before.mode() & 0o777 != 0o700
    {
        return Err(host_guard(
            "socket_policy",
            failure("physical original-host protected manager socket is unavailable"),
            diagnostic,
        ));
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(host_guard(
            "socket_create",
            io::Error::last_os_error(),
            diagnostic,
        ));
    }
    let stream = unsafe { OwnedFd::from_raw_fd(raw) };
    let bytes = socket.as_os_str().as_encoded_bytes();
    if bytes.len() > 107 {
        return Err(host_guard(
            "socket_path_bound",
            failure("physical original-host manager socket path exceeds bound"),
            diagnostic,
        ));
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (out, byte) in address.sun_path.iter_mut().zip(bytes) {
        *out = *byte as libc::c_char;
    }
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    if unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length,
        )
    } != 0
    {
        return Err(host_guard(
            "socket_connect",
            io::Error::last_os_error(),
            diagnostic,
        ));
    }
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut size,
        )
    } != 0
    {
        return Err(host_guard(
            "socket_peer",
            io::Error::last_os_error(),
            diagnostic,
        ));
    }
    if peer.uid != 1000 || peer.pid <= 0 {
        return Err(host_guard(
            "socket_peer_identity",
            failure("physical original-host manager peer is not the qualified user"),
            diagnostic,
        ));
    }
    let after = fs::symlink_metadata(&socket)
        .map_err(|error| host_guard("socket_post_stat", error, diagnostic.as_deref_mut()))?;
    if (before.dev(), before.ino(), before.uid(), before.mode())
        != (after.dev(), after.ino(), after.uid(), after.mode())
    {
        return Err(host_guard(
            "socket_object_identity",
            failure("physical original-host manager socket changed during qualification"),
            diagnostic,
        ));
    }
    let manager = host_process_evidence(
        peer.pid as u32,
        true,
        "manager_evidence",
        diagnostic.as_deref_mut(),
    )?;
    if !same_host_namespace_group(&caller, &manager) {
        return Err(host_guard(
            "manager_namespace_group",
            failure("physical original-host manager namespace authority is unavailable"),
            diagnostic,
        ));
    }
    let exe_path = fs::read_link(format!("/proc/{}/exe", peer.pid))
        .map_err(|error| host_guard("manager_exe_readlink", error, diagnostic.as_deref_mut()))?;
    let manager_exe = protected_tool(&exe_path)
        .map_err(|error| host_guard("manager_exe_policy", error, diagnostic.as_deref_mut()))?;
    if manager_exe.file_name().and_then(|name| name.to_str()) != Some("systemd") {
        return Err(host_guard(
            "manager_exe_name",
            failure("physical original-host manager executable is not systemd"),
            diagnostic,
        ));
    }
    let manager_file = File::open(format!("/proc/{}/exe", peer.pid))
        .map_err(|error| host_guard("manager_exe_open", error, diagnostic.as_deref_mut()))?;
    let held = manager_file
        .metadata()
        .map_err(|error| host_guard("manager_exe_fstat", error, diagnostic.as_deref_mut()))?;
    let configured = fs::metadata(&manager_exe)
        .map_err(|error| host_guard("manager_exe_stat", error, diagnostic.as_deref_mut()))?;
    if (held.dev(), held.ino()) != (configured.dev(), configured.ino()) {
        return Err(host_guard(
            "manager_exe_identity",
            failure("physical original-host manager executable changed during qualification"),
            diagnostic,
        ));
    }
    let mut tools = BTreeMap::new();
    for (name, resolve_guard, hash_guard) in [
        ("systemd-run", "systemd_run_resolve", "systemd_run_hash"),
        ("systemctl", "systemctl_resolve", "systemctl_hash"),
        ("busctl", "busctl_resolve", "busctl_hash"),
    ] {
        let tool = resolve_tool(name)
            .map_err(|error| host_guard(resolve_guard, error, diagnostic.as_deref_mut()))?;
        let hash = binary_sha256(&tool)
            .map_err(|error| host_guard(hash_guard, error, diagnostic.as_deref_mut()))?;
        tools.insert(name, json!({"path":tool,"sha256":hash}));
    }
    if matches!(policy, ControllerPolicy::WakeDiagnostic { .. })
        && let Some(diagnostic) = diagnostic.as_deref_mut()
    {
        diagnostic.controller_unit = None;
        diagnostic.controller_unit_validated = false;
    }
    recheck_host_processes(&caller, &manager, diagnostic.as_deref_mut())?;
    controller_admission(policy, &caller, diagnostic.as_deref_mut()).map_err(|error| {
        if matches!(policy, ControllerPolicy::Zero) {
            host_guard("final_caller_capabilities", error, diagnostic)
        } else {
            error
        }
    })?;
    if let Some(pin) = &controller_unit {
        if pin.manager() != &manager || pin.process() != &caller {
            return Err(failure(
                "host authority is not the SAME original qualified unit manager/controller",
            ));
        }
        pin.recheck(None)?;
    }
    let facts = json!({"uid":1000,"manager":manager,"managerExecutable":manager_exe,"socketDevice":before.dev(),"socketInode":before.ino(),"tools":tools,"caller":caller,"mutation":false,
        "controllerUnitWake":controller_unit.is_some(),"controllerUnit":controller_unit.as_ref().map(|pin|pin.unit()),"controllerCgroup":controller_unit.as_ref().map(|pin|pin.cgroup()),"NoNewPrivs":controller_unit.is_some()});
    Ok(HostAuthority {
        _socket: stream,
        executable: manager_file,
        caller,
        manager,
        facts,
        controller_unit,
    })
}
fn host_preflight_report(
    run: &Uuid,
    policy: ControllerPolicy<'_>,
    qualified: bool,
    diagnostic: &HostDiagnostic,
    error: Option<&io::Error>,
) -> Value {
    let expected_unit = match policy {
        ControllerPolicy::Zero | ControllerPolicy::UnitWake { .. } => None,
        ControllerPolicy::WakeDiagnostic { unit } => Some(unit),
    };
    let wake = expected_unit.is_some();
    let controller_unit = if wake {
        diagnostic.controller_unit.as_deref()
    } else {
        None
    };
    let mut report = json!({"gate":if wake{"physical-host-wake-preflight"}else{"physical-host-preflight"},"runId":run,"status":if error.is_some(){"blocked"}else{"pass"},
        "qualification":if qualified{"qualified"}else if matches!(diagnostic.guard,"human_tty"|"interactive_tty"){"not_attempted"}else{"refused"},
        "guard":if qualified{"qualified"}else{diagnostic.guard},"errno":diagnostic.errno,"io_kind":"none",
        "callerCaps":diagnostic.caller_caps,"managerCaps":diagnostic.manager_caps,
        "wakeDiagnostic":wake,"expectedControllerUnit":expected_unit,"controllerUnit":controller_unit,
        "controllerUnitValidated":wake&&diagnostic.controller_unit_validated&&controller_unit==expected_unit,
        "managerCommand":wake&&diagnostic.manager_command_observed,"managerCreatedOrRestarted":false,"nativeLaunch":false,"productInstall":false,
        "ADB":false,"provider":false,"runtimeAcceptanceClaim":false});
    if let Some(error) = error {
        report.as_object_mut().expect("object").extend(
            super::super::helper_error_metadata(error)
                .as_object()
                .expect("object")
                .clone(),
        );
    }
    report
}
fn host_terminal_report(output: &mut impl Write, report: &Value, passed: bool) -> bool {
    // Entry owns the single terminal attempt, even if write_fmt emits bytes before failing.
    // Returning false is FAILURE in the existing live/main result owner, not a second error report.
    writeln!(output, "{report}").is_ok() && passed
}
pub(super) fn host_preflight(
    evidence: &Path,
    policy: ControllerPolicy<'_>,
    admitted: Option<goatr::codex_relay::UnitWakePin>,
) -> io::Result<bool> {
    if matches!(policy, ControllerPolicy::UnitWake { .. }) {
        return Err(failure("full unit Wake is never a preflight alias"));
    }
    let run = Uuid::random();
    let mut diagnostic = HostDiagnostic::default();
    let qualification = if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let guard = match policy {
            ControllerPolicy::Zero | ControllerPolicy::UnitWake { .. } => "human_tty",
            ControllerPolicy::WakeDiagnostic { .. } => "interactive_tty",
        };
        Err(host_guard(
            guard,
            prerequisite("physical host preflight requires an actual interactive host TTY"),
            Some(&mut diagnostic),
        ))
    } else {
        original_host_authority(policy, Some(&mut diagnostic), admitted)
    };
    let qualified = qualification.is_ok();
    // err() drops the successful authority/connected socket; preflight never retains a runtime owner.
    let error = qualification.err();
    let mut report = host_preflight_report(&run, policy, qualified, &diagnostic, error.as_ref());
    report["evidenceWritten"] = json!(true);
    let filename = match policy {
        ControllerPolicy::Zero | ControllerPolicy::UnitWake { .. } => {
            "physical-host-preflight.json"
        }
        ControllerPolicy::WakeDiagnostic { .. } => "physical-host-wake-preflight.json",
    };
    let stored = match write_json(&evidence.join(filename), &report) {
        Ok(()) => true,
        Err(error) => {
            let mut write_diagnostic = HostDiagnostic::default();
            let error = host_guard("evidence_write", error, Some(&mut write_diagnostic));
            let mut boundary = super::super::helper_error_metadata(&error);
            boundary["guard"] = json!(write_diagnostic.guard);
            boundary["errno"] = json!(write_diagnostic.errno);
            report["status"] = json!("fail");
            report["evidenceWritten"] = json!(false);
            report["evidenceWrite"] = boundary;
            false
        }
    };
    Ok(host_terminal_report(
        &mut io::stdout(),
        &report,
        qualified && stored,
    ))
}

#[derive(Clone)]
struct PhysicalAdb {
    binary: PathBuf,
    serial: String,
}
impl PhysicalAdb {
    fn command(&self) -> AsyncCommand {
        let mut command = AsyncCommand::new(&self.binary);
        command.args(["-s", &self.serial]);
        command
            .env_remove("ANDROID_ADB_SERVER_PORT")
            .env_remove("ADB_SERVER_PORT")
            .env("ADB_SERVER_SOCKET", "tcp:127.0.0.1:5037");
        unsafe {
            use std::os::unix::process::CommandExt;
            command
                .as_std_mut()
                .pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
        command
    }
    async fn invoke(&self, arguments: &[&str], limit: usize) -> io::Result<Vec<u8>> {
        self.recheck_server().await?;
        let mut command = self.command();
        command.args(arguments);
        output(command, limit, Duration::from_secs(45)).await
    }
    async fn recheck_server(&self) -> io::Result<()> {
        if super::sdk_adb().await? != self.binary {
            return Err(failure("physical selected SDK ADB executable changed"));
        }
        Ok(())
    }
    async fn private_record(&self, path: &str) -> io::Result<Option<Value>> {
        let quoted = super::shell_literal(path)?;
        let script = format!(
            "if 'test' '-f' {quoted}; then 'cat' {quoted}; else 'printf' '%s' 'physical_pending'; fi"
        );
        let bytes = self
            .shell(&["run-as", TEST, "sh", "-c", &script], MAX_CONTROL)
            .await?;
        if bytes == b"physical_pending" {
            return Ok(None);
        }
        let value = goatr::native_json::parse(&bytes, goatr::native_json::REGISTRY)
            .map_err(|_| failure("physical owned UI private producer record is invalid"))?;
        Ok(Some(value))
    }
    async fn tunnels(&self, control: Option<u16>) -> io::Result<()> {
        let forwards = self.invoke(&["forward", "--list"], 65_536).await?;
        if text(&forwards)?
            .lines()
            .any(|line| line.split_whitespace().next() == Some(self.serial.as_str()))
        {
            return Err(failure(
                "physical selected device has an ADB forward; no ADB Iroh data-plane tunnel is allowed",
            ));
        }
        let reverse = self.invoke(&["reverse", "--list"], 65_536).await?;
        let rows: Vec<_> = text(&reverse)?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        match control {
            None if rows.is_empty() => Ok(()),
            Some(port) if rows.len() == 1 => {
                let words: Vec<_> = rows[0].split_whitespace().collect();
                let expected = format!("tcp:{port}");
                if words.len() != 3 || words[1] != expected || words[2] != expected {
                    return Err(failure(
                        "physical ADB reverse is not the one owned signed-control rendezvous",
                    ));
                }
                Ok(())
            }
            _ => Err(failure(
                "physical selected device has an unrelated ADB reverse; no tunnel is adopted or removed",
            )),
        }
    }
    async fn shell(&self, arguments: &[&str], limit: usize) -> io::Result<Vec<u8>> {
        let remote = arguments
            .iter()
            .map(|argument| super::shell_literal(argument))
            .collect::<io::Result<Vec<_>>>()?
            .join(" ");
        self.invoke(&["shell", &remote], limit).await
    }
    async fn write_private(&self, package: &str, path: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_CONTROL {
            return Err(failure("physical private control record exceeds bound"));
        }
        self.recheck_server().await?;
        let script = format!("'umask' '077'; 'cat' > {}", super::shell_literal(path)?);
        let args = ["run-as", package, "sh", "-c", &script];
        let remote = args
            .iter()
            .map(|argument| super::shell_literal(argument))
            .collect::<io::Result<Vec<_>>>()?
            .join(" ");
        let mut command = self.command();
        command
            .args(["shell", &remote])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        timeout(Duration::from_secs(15), async {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| failure("physical private record has no input pipe"))?;
            stdin.write_all(bytes).await?;
            stdin.shutdown().await?;
            drop(stdin);
            if !child.wait().await?.success() {
                return Err(failure("physical private control record write failed"));
            }
            Ok(())
        })
        .await
        .map_err(|_| failure("physical private control record deadline expired"))?
    }
    async fn default_product_process_absent(&self) -> io::Result<()> {
        self.recheck_server().await?;
        let remote = ["pidof", APP]
            .into_iter()
            .map(super::shell_literal)
            .collect::<io::Result<Vec<_>>>()?
            .join(" ");
        let mut child = self
            .command()
            .args(["shell", &remote])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure("product process stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| failure("product process stderr unavailable"))?;
        let out = async {
            let mut bytes = Vec::new();
            stdout.take(1025).read_to_end(&mut bytes).await?;
            if bytes.len() > 1024 {
                return Err(failure("product pidof output is oversized"));
            }
            Ok::<_, io::Error>(bytes)
        };
        let err = async {
            let mut bytes = Vec::new();
            stderr.take(1025).read_to_end(&mut bytes).await?;
            if bytes.len() > 1024 {
                return Err(failure("product pidof error is oversized"));
            }
            Ok::<_, io::Error>(bytes)
        };
        let (status, stdout, stderr) = timeout(Duration::from_secs(45), async {
            tokio::try_join!(child.wait(), out, err)
        })
        .await
        .map_err(|_| failure("product process absence query expired"))??;
        if status.code() != Some(1) || !stdout.is_empty() || !stderr.is_empty() {
            return Err(failure(
                "prepared PRODUCT process is present or absence is unproved",
            ));
        }
        Ok(())
    }
    async fn qualify(args: &clap::ArgMatches) -> io::Result<(Self, PreparedDevice)> {
        let serial = args
            .get_one::<String>("serial")
            .ok_or_else(|| failure("physical exact serial is required"))?
            .clone();
        if serial.is_empty()
            || serial.len() > 128
            || serial.starts_with("emulator-")
            || !serial
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
        {
            return Err(failure(
                "physical serial is not a bounded physical-device selector",
            ));
        }
        let adb = Self {
            binary: super::sdk_adb().await?,
            serial,
        };
        if text(&adb.invoke(&["get-state"], 128).await?)?.trim() != "device" {
            return Err(failure(
                "physical selected serial is not an authorized device",
            ));
        }
        let transports = adb.invoke(&["devices", "-l"], 65_536).await?;
        validate_current_usb_device(text(&transports)?, &adb.serial)?;
        let qemu = text(&adb.shell(&["getprop", "ro.kernel.qemu"], 128).await?)?
            .trim()
            .to_owned();
        if !matches!(qemu.as_str(), "" | "0") {
            return Err(failure("physical mode rejects emulator/unknown devices"));
        }
        let api = text(&adb.shell(&["getprop", "ro.build.version.sdk"], 128).await?)?
            .trim()
            .parse::<u32>()
            .map_err(|_| failure("physical device SDK is invalid"))?;
        if ![30, 34].contains(&api) {
            return Err(failure(
                "physical matrix requires an actual API 30 or API 34 device",
            ));
        }
        let abi = text(&adb.shell(&["getprop", "ro.product.cpu.abi"], 128).await?)?
            .trim()
            .to_owned();
        if !["arm64-v8a", "x86_64"].contains(&abi.as_str()) {
            return Err(failure(
                "physical device ABI is outside the packaged native closure",
            ));
        }
        let model = text(&adb.shell(&["getprop", "ro.product.model"], 256).await?)?
            .trim()
            .to_owned();
        if model.is_empty() || model.len() > 128 || model.chars().any(char::is_control) {
            return Err(failure("physical model is unavailable"));
        }
        let device = PreparedDevice {
            serial: adb.serial.clone(),
            model,
            sdk: api,
            abi,
            qemu,
        };
        Ok((adb, device))
    }
}

#[derive(Clone)]
struct Artifact {
    path: PathBuf,
    sha256: String,
    signer: String,
    package: String,
    debug: bool,
}
#[derive(Debug)]
struct ManifestNode {
    kind: String,
    indent: usize,
    attributes: BTreeMap<String, String>,
}
fn manifest_nodes(value: &str) -> io::Result<Vec<ManifestNode>> {
    let mut nodes: Vec<ManifestNode> = Vec::new();
    for line in value.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("E: ") {
            let kind = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| failure("APK manifest element is invalid"))?;
            nodes.push(ManifestNode {
                kind: kind.to_owned(),
                indent: line.len() - trimmed.len(),
                attributes: BTreeMap::new(),
            });
        } else if let Some(rest) = trimmed.strip_prefix("A: ") {
            let (name, value) = rest
                .split_once('=')
                .ok_or_else(|| failure("APK manifest attribute is invalid"))?;
            let value = if let Some(quoted) = value.strip_prefix('"') {
                let end = quoted
                    .find('"')
                    .ok_or_else(|| failure("APK manifest quoted attribute is invalid"))?;
                &quoted[..end]
            } else {
                value.trim()
            };
            if let Some(node) = nodes.last_mut() {
                let name = name.split('(').next().unwrap_or(name);
                let key = if let Some(local) =
                    name.strip_prefix("http://schemas.android.com/apk/res/android:")
                {
                    format!("android:{local}")
                } else {
                    name.to_owned()
                };
                if node.attributes.insert(key, value.to_owned()).is_some() {
                    return Err(failure("APK manifest has duplicate attributes"));
                }
            }
        }
    }
    if nodes.first().map(|node| node.kind.as_str()) != Some("manifest") {
        return Err(failure("APK manifest root is missing"));
    }
    Ok(nodes)
}
fn dex_classes(bytes: &[u8], wanted: &BTreeSet<String>) -> io::Result<BTreeSet<String>> {
    fn word(bytes: &[u8], offset: usize) -> io::Result<u32> {
        let value = bytes
            .get(
                offset
                    ..offset
                        .checked_add(4)
                        .ok_or_else(|| failure("DEX offset overflow"))?,
            )
            .ok_or_else(|| failure("DEX offset outside file"))?;
        Ok(u32::from_le_bytes(value.try_into().unwrap()))
    }
    if bytes.len() < 112
        || bytes.get(..4) != Some(b"dex\n")
        || bytes.get(7) != Some(&0)
        || word(bytes, 32)? as usize != bytes.len()
        || word(bytes, 36)? != 112
        || word(bytes, 40)? != 0x12345678
    {
        return Err(failure("physical APK DEX header is unsupported"));
    }
    let strings = word(bytes, 56)? as usize;
    let strings_at = word(bytes, 60)? as usize;
    let types = word(bytes, 64)? as usize;
    let types_at = word(bytes, 68)? as usize;
    let classes = word(bytes, 96)? as usize;
    let classes_at = word(bytes, 100)? as usize;
    if strings > 2_000_000 || types > 65_536 || classes > 65_536 {
        return Err(failure("physical APK DEX tables exceed bounds"));
    }
    let mut found = BTreeSet::new();
    for index in 0..classes {
        let class_index = word(
            bytes,
            classes_at
                .checked_add(index * 32)
                .ok_or_else(|| failure("DEX class offset overflow"))?,
        )? as usize;
        if class_index >= types {
            return Err(failure("DEX class type is outside table"));
        }
        let string_index = word(bytes, types_at + class_index * 4)? as usize;
        if string_index >= strings {
            return Err(failure("DEX type string is outside table"));
        }
        let mut offset = word(bytes, strings_at + string_index * 4)? as usize;
        let mut terminal = false;
        for _ in 0..5 {
            let byte = *bytes
                .get(offset)
                .ok_or_else(|| failure("DEX string length outside file"))?;
            offset += 1;
            if byte & 0x80 == 0 {
                terminal = true;
                break;
            }
        }
        if !terminal {
            return Err(failure("DEX string length is unbounded"));
        }
        let tail = bytes
            .get(offset..)
            .ok_or_else(|| failure("DEX string outside file"))?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .filter(|end| *end <= 4096)
            .ok_or_else(|| failure("DEX class descriptor is unbounded"))?;
        if let Ok(descriptor) = std::str::from_utf8(&tail[..end])
            && wanted.contains(descriptor)
            && !found.insert(descriptor.to_owned())
        {
            return Err(failure("APK has duplicate required class definitions"));
        }
    }
    Ok(found)
}
fn packaged_process_manifest(nodes: &[ManifestNode], package: &str) -> io::Result<()> {
    let manifest = nodes
        .first()
        .filter(|node| node.kind == "manifest")
        .ok_or_else(|| failure("physical APK has no manifest root"))?;
    if manifest.attributes.contains_key("android:sharedUserId") {
        return Err(failure("physical product/test APK requests a shared UID"));
    }
    let mut applications = nodes.iter().filter(|node| node.kind == "application");
    let application = applications
        .next()
        .ok_or_else(|| failure("physical APK has no application"))?;
    if applications.next().is_some()
        || application
            .attributes
            .get("android:process")
            .is_some_and(|process| process != package)
    {
        return Err(failure(
            "physical product/test APK does not use its distinct default process",
        ));
    }
    Ok(())
}
fn packaged_instrumentations(nodes: &[ManifestNode]) -> io::Result<()> {
    let instrumentations: Vec<_> = nodes
        .iter()
        .filter(|node| node.kind == "instrumentation")
        .collect();
    let expected = [
        ("io.github.trevarj.goatr.OwnedInstrumentationRunner", APP),
        ("io.github.trevarj.goatr.OwnedUiAutomationRunner", TEST),
    ];
    if instrumentations.len() != 2
        || expected.iter().any(|(name, target)| {
            instrumentations
                .iter()
                .filter(|node| {
                    node.attributes.get("android:name").map(String::as_str) == Some(*name)
                        && node
                            .attributes
                            .get("android:targetPackage")
                            .map(String::as_str)
                            == Some(*target)
                })
                .count()
                != 1
        })
        || instrumentations
            .iter()
            .any(|node| node.attributes.contains_key("android:targetProcesses"))
    {
        return Err(failure(
            "physical packaged instrumentation targets/processes are not the exact two isolated owners",
        ));
    }
    Ok(())
}
fn packaged_startup_provider(nodes: &[ManifestNode]) -> io::Result<()> {
    let providers: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == "provider")
        .collect();
    if providers.len() != 1 {
        return Err(failure(
            "physical product APK has an unaudited pre-Application provider",
        ));
    }
    let (index, provider) = providers[0];
    if provider.attributes.get("android:name").map(String::as_str)
        != Some("androidx.startup.InitializationProvider")
        || provider
            .attributes
            .get("android:authorities")
            .map(String::as_str)
            != Some("io.github.trevarj.goatr.debug.androidx-startup")
        || provider
            .attributes
            .get("android:exported")
            .map(String::as_str)
            != Some("false")
        || provider
            .attributes
            .get("android:enabled")
            .is_some_and(|value| value != "true")
    {
        return Err(failure(
            "physical product APK startup provider is outside the audited closure",
        ));
    }
    let expected = [
        "androidx.emoji2.text.EmojiCompatInitializer",
        "androidx.lifecycle.ProcessLifecycleInitializer",
        "androidx.profileinstaller.ProfileInstallerInitializer",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for node in nodes
        .iter()
        .skip(index + 1)
        .take_while(|node| node.indent > provider.indent)
    {
        if node.kind != "meta-data"
            || node.attributes.get("android:value").map(String::as_str) != Some("androidx.startup")
        {
            return Err(failure(
                "physical product APK has an unaudited startup initializer",
            ));
        }
        actual.insert(
            node.attributes
                .get("android:name")
                .map(String::as_str)
                .ok_or_else(|| failure("startup initializer has no name"))?,
        );
    }
    if actual != expected {
        return Err(failure(
            "physical product APK startup initializer closure changed",
        ));
    }
    Ok(())
}
async fn qualify_artifact(
    path: &Path,
    expected_sha: &str,
    sdk: &Path,
    package: &str,
    test: bool,
) -> io::Result<Artifact> {
    let pin = frozen_input(path, expected_sha)?;
    let pins = std::slice::from_ref(&pin);
    let path = &pin.0;
    let sha256 = expected_sha.to_owned();
    let aapt = protected_tool(&sdk.join("build-tools/36.0.0/aapt2"))?;
    let signer_tool = protected_tool(&sdk.join("build-tools/36.0.0/apksigner"))?;
    let mut signer_command = AsyncCommand::new(signer_tool);
    signer_command
        .args(["verify", "--verbose", "--print-certs"])
        .arg(path);
    recheck_inputs(pins)?;
    let signer_output = output(signer_command, 16_384, Duration::from_secs(30)).await?;
    recheck_inputs(pins)?;
    let signers: Vec<_> = text(&signer_output)?
        .lines()
        .filter_map(|line| line.strip_prefix("Signer #1 certificate SHA-256 digest: "))
        .collect();
    if signers.len() != 1
        || !hex64(signers[0])
        || text(&signer_output)?
            .lines()
            .any(|line| line.starts_with("Signer #2 "))
    {
        return Err(failure("physical APK signer is not singular and qualified"));
    }
    let signer = signers[0].to_owned();
    let mut badging_command = AsyncCommand::new(&aapt);
    badging_command.args(["dump", "badging"]).arg(path);
    recheck_inputs(pins)?;
    let badging_bytes = output(badging_command, 131_072, Duration::from_secs(30)).await?;
    recheck_inputs(pins)?;
    let badging = text(&badging_bytes)?;
    if !badging
        .lines()
        .any(|line| line.starts_with(&format!("package: name='{package}' ")))
    {
        return Err(failure("physical APK package does not match its owner"));
    }
    let debug = badging.lines().any(|line| line == "application-debuggable");
    if matches!(package, APP | TEST) && !debug {
        return Err(failure("physical product and test APKs must be debuggable"));
    }
    let mut manifest_command = AsyncCommand::new(&aapt);
    manifest_command
        .args(["dump", "xmltree", "--file", "AndroidManifest.xml"])
        .arg(path);
    recheck_inputs(pins)?;
    let manifest_bytes = output(manifest_command, 1_048_576, Duration::from_secs(30)).await?;
    recheck_inputs(pins)?;
    let nodes = manifest_nodes(text(&manifest_bytes)?)?;
    if matches!(package, APP | TEST) {
        packaged_process_manifest(&nodes, package)?;
    }
    if test {
        packaged_instrumentations(&nodes)?;
        let required = [
            "Lio/github/trevarj/goatr/OwnedInstrumentationRunner;",
            "Lio/github/trevarj/goatr/OwnedUiAutomationRunner;",
            "Lio/github/trevarj/goatr/PhysicalMvpObserver;",
            "Lio/github/trevarj/goatr/PhysicalMvpPlatform;",
            "Lio/github/trevarj/goatr/GoatrConnectionTest;",
            "Lio/github/trevarj/goatr/SessionStoreTest;",
            "Lio/github/trevarj/goatr/TerminalInputConnectionTest;",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        let unzip = resolve_tool("unzip")?;
        let mut listing = AsyncCommand::new(&unzip);
        listing.args(["-Z1"]).arg(path);
        recheck_inputs(pins)?;
        let entries_bytes = output(listing, 1_048_576, Duration::from_secs(30)).await?;
        recheck_inputs(pins)?;
        let mut found = BTreeSet::new();
        let mut dex_count = 0;
        for member in text(&entries_bytes)?.lines().filter(|member| {
            member.starts_with("classes") && member.ends_with(".dex") && !member.contains('/')
        }) {
            dex_count += 1;
            if dex_count > 64 {
                return Err(failure("physical test APK has too many DEX files"));
            }
            let mut extract = AsyncCommand::new(&unzip);
            extract.arg("-p").arg(path).arg(member);
            recheck_inputs(pins)?;
            let dex = output(extract, 67_108_864, Duration::from_secs(30)).await?;
            recheck_inputs(pins)?;
            for class in dex_classes(&dex, &required)? {
                if !found.insert(class) {
                    return Err(failure("physical test APK repeats a required class"));
                }
            }
        }
        if found != required {
            return Err(failure(
                "physical test APK does not contain the packaged observer and both runner class definitions",
            ));
        }
    } else if package == APP {
        packaged_startup_provider(&nodes)?;
    }
    recheck_inputs(pins)?;
    Ok(Artifact {
        path: pin.0,
        sha256,
        signer,
        package: package.to_owned(),
        debug,
    })
}

fn provider_dependency_proof() -> io::Result<Value> {
    let source = canonical_file(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../gradle/verification-metadata.xml"),
    )?;
    let metadata = fs::read_to_string(&source)?;
    let mut proofs = Vec::with_capacity(MVP_AARS.len());
    for (group, coordinate, expected) in MVP_AARS {
        let (module, version) = coordinate.split_once('/').unwrap();
        let artifact = format!("{module}-{version}.aar");
        let header =
            format!("<component group=\"{group}\" name=\"{module}\" version=\"{version}\">");
        let mut components = metadata.split(&header);
        components.next();
        let component = components
            .next()
            .and_then(|value| value.split_once("</component>").map(|(value, _)| value))
            .ok_or_else(|| failure("physical retained dependency component is missing"))?;
        if components.next().is_some() {
            return Err(failure(
                "physical retained dependency component is ambiguous",
            ));
        }
        let artifact_header = format!("<artifact name=\"{artifact}\">");
        let mut artifacts = component.split(&artifact_header);
        artifacts.next();
        let record = artifacts
            .next()
            .and_then(|value| value.split_once("</artifact>").map(|(value, _)| value))
            .ok_or_else(|| failure("physical retained dependency artifact record is missing"))?;
        if artifacts.next().is_some() || !record.contains(&format!("<sha256 value=\"{expected}\""))
        {
            return Err(failure(
                "physical retained exact dependency artifact SHA-256 pin changed",
            ));
        }
        let path = maven_aar(group, module, version)?;
        if binary_sha256(&path)? != expected {
            return Err(failure(
                "physical actual resolved dependency AAR does not match its exact retained verification record",
            ));
        }
        proofs.push(json!({"group":group,"module":module,"version":version,"aar":path,"sha256":expected,"role":if group=="org.unifiedpush.android"{"registration-storage-contract-only"}else{"pre-Application-AndroidX-provider-closure"}}));
    }
    Ok(
        json!({"verificationMetadataSha256":binary_sha256(&source)?,"artifacts":proofs,"provider":"androidx.startup.InitializationProvider","initializers":["androidx.emoji2.text.EmojiCompatInitializer","androidx.lifecycle.ProcessLifecycleInitializer","androidx.profileinstaller.ProfileInstallerInitializer"],"preApplicationClaim":"only-pinned-AndroidX-providers-before-raw-baseline","connectorStorage":"qualified-3.3.5-SQLite-v1-not-legacy-preferences","excludedFromRetainedBaseline":["androidx-profile-installation-markers","application-file-markers"]}),
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RetainedHost {
    host: String,
    device: Option<String>,
    metadata_sha256: String,
    rows_sha256: String,
    action_count: u64,
    registration_generation: Option<String>,
    registration_sha256: Option<String>,
    mode: String,
    enabled: bool,
    completed: bool,
    key_aliases: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PreferenceFact {
    file: String,
    key_sha256: String,
    #[serde(rename = "type")]
    kind: String,
    value_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RetainedFacts {
    schema: u64,
    database_exists: bool,
    hosts: Vec<RetainedHost>,
    preferences: Vec<PreferenceFact>,
    aliases: Vec<String>,
    alias_sha256: String,
    service_requested: bool,
    service_requested_present: bool,
    run_label_collision: bool,
    connector: Value,
    database: Value,
    orphans: Value,
    key_files: Value,
}
#[derive(Debug, Serialize)]
struct AdmissionFailure {
    site: &'static str,
    guard: &'static str,
    summary: Option<Value>,
}
impl std::fmt::Display for AdmissionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.site, self.guard)
    }
}
impl std::error::Error for AdmissionFailure {}
fn admission_failure(
    site: &'static str,
    guard: &'static str,
    mut error: io::Error,
    summary: Option<Value>,
) -> io::Error {
    if let Some(closed) = error
        .get_mut()
        .and_then(|error| error.downcast_mut::<AdmissionFailure>())
    {
        if summary.is_some() {
            closed.summary = summary;
        }
        return error;
    }
    io::Error::new(
        error.kind(),
        AdmissionFailure {
            site,
            guard,
            summary,
        },
    )
}
fn admission_diagnostic(error: &io::Error) -> Option<&AdmissionFailure> {
    error.get_ref()?.downcast_ref::<AdmissionFailure>()
}
fn review_failure_projection(error: &io::Error) -> Value {
    let metadata = super::super::helper_error_metadata(error);
    let mut projection = json!({"io_kind":metadata["io_kind"]});
    for field in ["source_file", "source_line"] {
        if !metadata[field].is_null() {
            projection[field] = metadata[field].clone();
        }
    }
    if let Some(diagnostic) = admission_diagnostic(error) {
        projection["admissionFailure"] = json!(diagnostic);
    }
    projection
}
fn review_admission_failure(projection: &Value) -> Option<&Value> {
    projection.get("admissionFailure")
}
fn retain_review_failure(error: io::Error, diagnostic: &mut Option<Value>) -> &'static str {
    let code = blocker(&error);
    if diagnostic.is_none() {
        *diagnostic = Some(review_failure_projection(&error));
    }
    code
}
fn first_open_admission_result(
    admission: io::Result<()>,
    publication: io::Result<()>,
    mut summary: Value,
) -> io::Result<()> {
    if let Err(error) = publication {
        summary["publicationFailure"] =
            json!({"site":"first_open_publication","guard":"admission_summary"});
        return Err(admission_failure(
            "first_open_publication",
            "admission_summary",
            admission.err().unwrap_or(error),
            Some(summary),
        ));
    }
    admission
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RetainedDecodeSummary {
    schema_is_one: bool,
    database_exists: Option<bool>,
    hosts: Option<usize>,
    preferences: Option<usize>,
    aliases: Option<usize>,
    orphans: Option<usize>,
    key_files: Option<usize>,
    service_requested: Option<bool>,
    service_requested_present: Option<bool>,
}
fn retained_decode_summary(value: &Value) -> RetainedDecodeSummary {
    // ponytail: capture only scalars before validation; allocate diagnostic JSON only on rejection.
    RetainedDecodeSummary {
        schema_is_one: value["schema"] == 1,
        database_exists: value["databaseExists"].as_bool(),
        hosts: value["hosts"].as_array().map(Vec::len),
        preferences: value["preferences"].as_array().map(Vec::len),
        aliases: value["aliases"].as_array().map(Vec::len),
        orphans: value["orphans"].as_array().map(Vec::len),
        key_files: value["keyFiles"]["files"].as_array().map(Vec::len),
        service_requested: value["serviceRequested"].as_bool(),
        service_requested_present: value["serviceRequestedPresent"].as_bool(),
    }
}
#[derive(Serialize)]
struct FirstOpenAdmission {
    transition: [(&'static str, bool); 12],
    quiescence: [(&'static str, bool); 18],
    key_material: [(&'static str, bool); 5],
}
impl FirstOpenAdmission {
    fn require(site: &'static str, checks: &[(&'static str, bool)]) -> io::Result<()> {
        if let Some((guard, _)) = checks.iter().find(|(_, passed)| !*passed) {
            return Err(admission_failure(
                site,
                guard,
                failure("physical first-open admission rejected"),
                None,
            ));
        }
        Ok(())
    }
}
fn digest_text(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(text.as_bytes()))
}
impl RetainedFacts {
    fn fresh_uninitialized(&self) -> io::Result<()> {
        if self.database_exists
            || !self.hosts.is_empty()
            || !self.orphans.as_array().is_some_and(Vec::is_empty)
            || !self.preferences.is_empty()
            || !self.aliases.is_empty()
            || self.alias_sha256 != digest_text("")
            || self.service_requested
            || self.service_requested_present
            || self.run_label_collision
            || self.connector["databaseExists"] != false
            || self.connector["legacyFilePresent"] != false
            || !self.key_files["files"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            return Err(prerequisite(
                "physical fresh install contains unexpected Goatr state",
            ));
        }
        validate_database(&self.database, false)?;
        validate_connector(&self.connector)?;
        validate_key_files(&self.key_files)?;
        Ok(())
    }
    fn fresh_empty(&self) -> io::Result<()> {
        self.fresh_uninitialized()?;
        if !self.key_files["directory"].is_null() {
            return Err(prerequisite(
                "physical fresh install already has a Goatr key directory",
            ));
        }
        Ok(())
    }
    fn constructor_ready(&self, ready: &Self, uid: u32) -> io::Result<()> {
        self.fresh_empty()?;
        ready.fresh_uninitialized()?;
        if ready.key_files["directory"].is_null()
            || unsigned(&ready.key_files["directory"], "uid")? != u64::from(uid)
        {
            return Err(failure(
                "physical normal Application constructor did not create only its owned empty key directory",
            ));
        }
        Ok(())
    }
    fn first_open_admission(
        &self,
        constructor: &Self,
        opened: &Self,
        uid: u32,
    ) -> FirstOpenAdmission {
        let files = opened.key_files["files"].as_array();
        let endpoint = files.and_then(|files| {
            files
                .iter()
                .find(|file| file["name"] == "goatr.installation.endpoint.gcm")
        });
        let marker = files.and_then(|files| {
            files
                .iter()
                .find(|file| file["name"] == "goatr.installation.endpoint.gcm.initialized")
        });
        FirstOpenAdmission {
            transition: [
                ("database_exists", opened.database_exists),
                ("hosts_empty", opened.hosts.is_empty()),
                (
                    "orphans_empty",
                    opened.orphans.as_array().is_some_and(Vec::is_empty),
                ),
                ("preferences_empty", opened.preferences.is_empty()),
                ("service_not_requested", !opened.service_requested),
                ("service_key_absent", !opened.service_requested_present),
                ("run_label_absent", !opened.run_label_collision),
                ("connector_unchanged", opened.connector == self.connector),
                (
                    "directory_unchanged",
                    opened.key_files["directory"] == constructor.key_files["directory"],
                ),
                ("one_alias", opened.aliases.len() == 1),
                (
                    "endpoint_alias",
                    opened.aliases.first().map(String::as_str)
                        == Some("goatr.installation.endpoint"),
                ),
                (
                    "alias_fingerprint",
                    opened.alias_sha256 == digest_text("goatr.installation.endpoint"),
                ),
            ],
            quiescence: DB8_QUIESCENCE.map(|name| {
                (
                    name,
                    opened.database["quiescence"][name].as_u64() == Some(0),
                )
            }),
            key_material: [
                ("two_files", files.is_some_and(|files| files.len() == 2)),
                ("endpoint_file", endpoint.is_some()),
                ("endpoint_marker", marker.is_some()),
                (
                    "owned_uid",
                    files.is_some_and(|files| {
                        files
                            .iter()
                            .all(|file| file["uid"].as_u64() == Some(u64::from(uid)))
                    }),
                ),
                (
                    "marker_gai1",
                    marker.is_some_and(|file| file["sha256"] == digest_text("GAI1")),
                ),
            ],
        }
    }
    fn admitted_first_open(
        &self,
        constructor: &Self,
        opened: &Self,
        uid: u32,
        admission: &FirstOpenAdmission,
    ) -> io::Result<()> {
        self.constructor_ready(constructor, uid).map_err(|error| {
            admission_failure("first_open_validation", "constructor", error, None)
        })?;
        // The raw reader admits only android_metadata globally; every other row has a validated host/orphan group.
        FirstOpenAdmission::require("first_open_transition", &admission.transition)?;
        FirstOpenAdmission::require("first_open_quiescence", &admission.quiescence)?;
        validate_database(&opened.database, true)
            .map_err(|error| admission_failure("first_open_validation", "database", error, None))?;
        validate_key_files(&opened.key_files).map_err(|error| {
            admission_failure("first_open_validation", "key_files", error, None)
        })?;
        FirstOpenAdmission::require("first_open_key_material", &admission.key_material)
    }
    fn decode(value: &Value) -> io::Result<Self> {
        let mut guard = "schema";
        let result = (|| {
            let facts = Self::deserialize(value)
                .map_err(|_| failure("physical raw retained baseline has an unknown schema"))?;
            guard = "bounds";
            if facts.schema != 1
                || facts.hosts.len() > 129
                || facts.preferences.len() > 8192
                || facts.aliases.len() > 1026
                || !hex64(&facts.alias_sha256)
            {
                return Err(failure("physical retained baseline is unbounded"));
            }
            guard = "hosts";
            let mut hosts = BTreeSet::new();
            for host in &facts.hosts {
                if !hex64(&host.host)
                    || host
                        .device
                        .as_ref()
                        .is_some_and(|device| uuid_text(device).is_err())
                    || !hex64(&host.metadata_sha256)
                    || !hex64(&host.rows_sha256)
                    || !hosts.insert(host.host.as_str())
                    || !matches!(host.mode.as_str(), "Persistent" | "Ntfy")
                    || host
                        .registration_sha256
                        .as_ref()
                        .is_some_and(|hash| !hex64(hash))
                    || host.key_aliases.len() > 8
                    || host
                        .key_aliases
                        .iter()
                        .any(|alias| !facts.aliases.contains(alias))
                {
                    return Err(failure("physical retained host baseline is invalid"));
                }
            }
            guard = "preferences";
            for preference in &facts.preferences {
                if !hex64(&preference.key_sha256)
                    || !hex64(&preference.value_sha256)
                    || preference.file.len() > 128
                {
                    return Err(failure("physical retained preference baseline is invalid"));
                }
            }
            guard = "database";
            validate_database(&facts.database, facts.database_exists)?;
            guard = "key_files";
            validate_key_files(&facts.key_files)?;
            let files = key_file_index(&facts.key_files)?;
            guard = "file_wrapping_alias";
            for name in files.keys() {
                let alias = goatr_key_alias(name)
                    .ok_or_else(|| failure("physical encrypted key namespace is invalid"))?;
                if !facts.aliases.iter().any(|present| present == alias) {
                    return Err(failure(
                        "physical encrypted file has no original wrapping alias",
                    ));
                }
            }
            guard = "alias_material_pair";
            for alias in &facts.aliases {
                if goatr_key_alias(&format!("{alias}.gcm")).is_some()
                    && (!files.contains_key(format!("{alias}.gcm").as_str())
                        || !files.contains_key(format!("{alias}.gcm.initialized").as_str()))
                {
                    return Err(failure(
                        "physical original wrapping alias has missing encrypted material",
                    ));
                }
            }
            guard = "orphans";
            let mut orphans = BTreeSet::new();
            for row in facts
                .orphans
                .as_array()
                .filter(|rows| rows.len() <= 256)
                .ok_or_else(|| failure("physical orphan-row index is unbounded"))?
            {
                object(row, &["host", "rowsSha256", "rowCount"])?;
                let host = string(row, "host")?;
                if !hex64(host)
                    || hosts.contains(host)
                    || !orphans.insert(host)
                    || !hex64(string(row, "rowsSha256")?)
                {
                    return Err(failure(
                        "physical orphan logical rows are invalid or hidden by a host",
                    ));
                }
                unsigned(row, "rowCount")?;
            }
            guard = "service_request";
            let service = facts.service_fact();
            if service.is_some() != facts.service_requested_present
                || service.is_some_and(|fact| {
                    fact.kind != "boolean"
                        || fact.value_sha256
                            != digest_text(if facts.service_requested {
                                "true"
                            } else {
                                "false"
                            })
                })
                || service.is_none() && facts.service_requested
            {
                return Err(failure(
                    "physical service request boolean/type/presence facts disagree",
                ));
            }
            guard = "connector";
            validate_connector(&facts.connector)?;
            guard = "connector_wrapping_alias";
            if ["registrations", "tokens", "keys"]
                .iter()
                .any(|table| !facts.connector[*table].as_array().unwrap().is_empty())
                && !facts.aliases.iter().any(|alias| alias == "UnifiedPush")
            {
                return Err(failure(
                    "physical retained connector material has no original wrapping alias; never generate a replacement",
                ));
            }
            Ok(facts)
        })();
        result.map_err(|error| admission_failure("retained_decode", guard, error, None))
    }
    fn service_fact(&self) -> Option<&PreferenceFact> {
        let key = digest_text("service_requested");
        self.preferences
            .iter()
            .find(|fact| fact.file == "delivery_preferences" && fact.key_sha256 == key)
    }
    fn before_private(&self, after: &Self, private: &str) -> io::Result<()> {
        if self.database_exists != after.database_exists
            || self.database["schemaSha256"] != after.database["schemaSha256"]
            || self.database["globalSha256"] != after.database["globalSha256"]
            || self.orphans != after.orphans
            || self.key_files != after.key_files
            || self.hosts != after.hosts
            || self.preferences != after.preferences
            || self.aliases != after.aliases
            || self.service_requested != after.service_requested
            || self.service_requested_present != after.service_requested_present
            || after.run_label_collision
            || self.connector["databaseExists"] != after.connector["databaseExists"]
            || connector_preserved(&self.connector, &after.connector, private)?
        {
            return Err(failure(
                "physical product startup changed retained user state before private pairing",
            ));
        }
        Ok(())
    }
    fn preserved_with_private_host(
        &self,
        after: &Self,
        private: &str,
        service_requested: bool,
        normalize_owned_service_key: bool,
    ) -> io::Result<()> {
        if !self.database_exists {
            return Err(failure(
                "physical private comparison requires the separately admitted immutable first-open reference",
            ));
        }
        let retained: Vec<_> = after
            .hosts
            .iter()
            .filter(|host| host.host != private)
            .cloned()
            .collect();
        let selected = after
            .hosts
            .iter()
            .find(|host| host.host == private)
            .ok_or_else(|| failure("physical private host is absent from retained comparison"))?;
        if !after.database_exists
            || self.database["schemaSha256"] != after.database["schemaSha256"]
            || self.database["globalSha256"] != after.database["globalSha256"]
            || self.orphans != after.orphans
        {
            return Err(failure(
                "physical retained DB8 existence/schema/global/orphan rows changed",
            ));
        }
        key_files_preserved(self, after, private)?;
        let private_connector = connector_preserved(&self.connector, &after.connector, private)?;
        let mut expected_aliases: BTreeSet<_> = self
            .aliases
            .iter()
            .chain(selected.key_aliases.iter())
            .map(String::as_str)
            .collect();
        if private_connector
            && selected.registration_generation.is_some()
            && selected.registration_sha256.is_some()
        {
            expected_aliases.insert("UnifiedPush");
        }
        let current_aliases: BTreeSet<_> = after.aliases.iter().map(String::as_str).collect();
        let suffix = digest_text(private);
        let owned_keys =
            ["mode", "enabled", "completed"].map(|key| digest_text(&format!("{key}.{suffix}")));
        let request_key = digest_text("service_requested");
        if self.preferences.iter().any(|fact| {
            fact.file == "delivery_preferences" && owned_keys.contains(&fact.key_sha256)
        }) {
            return Err(failure(
                "physical new private host collides with retained delivery preference keys",
            ));
        }
        let before_service = self.service_fact();
        let after_service = after.service_fact();
        let service_exact = before_service == after_service;
        let owned_true = normalize_owned_service_key
            && !self.service_requested
            && service_requested
            && after_service.is_some_and(|fact| {
                fact.kind == "boolean" && fact.value_sha256 == digest_text("true")
            });
        let owned_absent_restored = normalize_owned_service_key
            && !self.service_requested_present
            && !self.service_requested
            && !service_requested
            && after_service.is_some_and(|fact| {
                fact.kind == "boolean" && fact.value_sha256 == digest_text("false")
            });
        if !service_exact && !owned_true && !owned_absent_restored {
            return Err(failure(
                "physical existing service request key/type/value/presence was not preserved",
            ));
        }
        let before: Vec<_> = self
            .preferences
            .iter()
            .filter(|fact| !(fact.file == "delivery_preferences" && fact.key_sha256 == request_key))
            .collect();
        let current: Vec<_> = after
            .preferences
            .iter()
            .filter(|fact| {
                !(fact.file == "delivery_preferences"
                    && (owned_keys.contains(&fact.key_sha256) || fact.key_sha256 == request_key))
            })
            .collect();
        if retained != self.hosts
            || after.hosts.len() != self.hosts.len() + 1
            || current != before
            || current_aliases != expected_aliases
            || after.service_requested != service_requested
        {
            return Err(failure(
                "physical retained hosts, actions, presentation, preferences, key aliases, or service request changed",
            ));
        }
        Ok(())
    }
}
const DB8_QUIESCENCE: [&str; 18] = [
    "viewed",
    "pendingCancellation",
    "pendingKeyDeletion",
    "expiredRecovery",
    "staleInventory",
    "staleRecords",
    "expiredContent",
    "staleHints",
    "staleNotifications",
    "notificationCapacity",
    "stalePresentation",
    "presentationCapacity",
    "staleReadWater",
    "oldTerminalAction",
    "retainedFloor",
    "presentationFloor",
    "cacheBudget",
    "locale",
];
fn validate_database(value: &Value, exists: bool) -> io::Result<()> {
    object(
        value,
        &["schemaSha256", "globalSha256", "measuredAt", "quiescence"],
    )?;
    unsigned(value, "measuredAt")?;
    for key in ["schemaSha256", "globalSha256"] {
        if exists {
            if !hex64(string(value, key)?) {
                return Err(failure(
                    "physical DB8 complete schema/global fingerprint is invalid",
                ));
            }
        } else if !value[key].is_null() {
            return Err(failure(
                "physical absent DB8 has fabricated schema/global facts",
            ));
        }
    }
    let checks = value["quiescence"]
        .as_object()
        .ok_or_else(|| failure("physical DB8 fixed-point facts are not an object"))?;
    if exists {
        object(&value["quiescence"], &DB8_QUIESCENCE)?;
        for name in DB8_QUIESCENCE {
            unsigned(&value["quiescence"], name)?;
        }
    } else if !checks.is_empty() {
        return Err(failure("physical absent DB8 reports first-open row facts"));
    }
    Ok(())
}
fn goatr_key_alias(name: &str) -> Option<&str> {
    let base = name.strip_suffix(".initialized").unwrap_or(name);
    let alias = base.strip_suffix(".gcm")?;
    if alias == "goatr.installation.endpoint" {
        return Some(alias);
    }
    let rest = alias.strip_prefix("goatr.host.")?;
    let digest = rest
        .strip_suffix(".receipt")
        .or_else(|| rest.strip_suffix(".push"))?;
    hex64(digest).then_some(alias)
}
fn key_file_index(value: &Value) -> io::Result<BTreeMap<&str, &Value>> {
    array(value, "files", 2050)?
        .iter()
        .map(|file| Ok((string(file, "name")?, file)))
        .collect()
}
fn validate_key_files(value: &Value) -> io::Result<()> {
    object(value, &["directory", "files"])?;
    let files = key_file_index(value)?;
    if files.len() != array(value, "files", 2050)?.len() {
        return Err(failure("physical key file identity is duplicated"));
    }
    if value["directory"].is_null() {
        if !files.is_empty() {
            return Err(failure("physical absent key directory reports files"));
        }
    } else {
        let dir = &value["directory"];
        object(dir, &["device", "inode", "mode", "uid"])?;
        if unsigned(dir, "mode")? != 0o700 || unsigned(dir, "inode")? == 0 {
            return Err(failure("physical key directory metadata is unsafe"));
        }
        unsigned(dir, "device")?;
        unsigned(dir, "uid")?;
    }
    for (name, file) in &files {
        object(
            file,
            &[
                "name", "sha256", "device", "inode", "size", "mode", "uid", "gid", "links",
                "mtime", "ctime",
            ],
        )?;
        let alias = goatr_key_alias(name).ok_or_else(|| {
            failure("physical key inventory contains unknown/back-up/partial names")
        })?;
        let marker = name.ends_with(".initialized");
        if !hex64(string(file, "sha256")?)
            || unsigned(file, "mode")? != 0o600
            || unsigned(file, "links")? != 1
            || unsigned(file, "size")? != if marker { 4 } else { 61 }
            || unsigned(file, "inode")? == 0
        {
            return Err(failure(
                "physical encrypted key file/marker metadata is unsafe",
            ));
        }
        for key in ["device", "uid", "gid", "mtime", "ctime"] {
            unsigned(file, key)?;
        }
        if !files.contains_key(format!("{alias}.gcm").as_str())
            || !files.contains_key(format!("{alias}.gcm.initialized").as_str())
        {
            return Err(failure("physical encrypted material pair is partial"));
        }
    }
    Ok(())
}
fn key_files_preserved(
    before: &RetainedFacts,
    after: &RetainedFacts,
    private: &str,
) -> io::Result<()> {
    validate_key_files(&before.key_files)?;
    validate_key_files(&after.key_files)?;
    if !before.key_files["directory"].is_null()
        && before.key_files["directory"] != after.key_files["directory"]
    {
        return Err(failure(
            "physical original key directory was removed or replaced",
        ));
    }
    let old = key_file_index(&before.key_files)?;
    let current = key_file_index(&after.key_files)?;
    for (name, facts) in &old {
        if current.get(name) != Some(facts) {
            return Err(failure(
                "physical retained encrypted endpoint/receipt/push file or marker changed/deleted/replaced",
            ));
        }
    }
    let receipt = format!("goatr.host.{}.receipt", digest_text(private));
    let push = format!("goatr.host.{}.push", digest_text(private));
    let first = before.hosts.is_empty()
        && !before
            .aliases
            .iter()
            .any(|alias| alias == "goatr.installation.endpoint")
        && !old
            .keys()
            .any(|name| name.starts_with("goatr.installation.endpoint."));
    for name in current.keys().filter(|name| !old.contains_key(**name)) {
        let alias = goatr_key_alias(name)
            .ok_or_else(|| failure("physical added encrypted key filename is invalid"))?;
        if alias != receipt && alias != push && !(first && alias == "goatr.installation.endpoint") {
            return Err(failure(
                "physical unrelated encrypted key material was added",
            ));
        }
    }
    let selected = after
        .hosts
        .iter()
        .find(|host| host.host == private)
        .ok_or_else(|| failure("physical private host disappeared while comparing key material"))?;
    if selected
        .key_aliases
        .iter()
        .any(|alias| alias != &receipt && alias != &push && alias != "goatr.installation.endpoint")
    {
        return Err(failure(
            "physical private host records another key namespace",
        ));
    }
    Ok(())
}
fn validate_connector(value: &Value) -> io::Result<()> {
    object(
        value,
        &[
            "databaseExists",
            "version",
            "schemaSha256",
            "platformMetadataSha256",
            "legacyFilePresent",
            "legacyKeyCount",
            "localeChanges",
            "distributors",
            "registrations",
            "tokens",
            "keys",
        ],
    )?;
    let exists = boolean(value, "databaseExists")?;
    boolean(value, "legacyFilePresent")?;
    if unsigned(value, "legacyKeyCount")? != 0
        || unsigned(value, "version")? != if exists { 1 } else { 0 }
    {
        return Err(failure(
            "physical connector legacy material/schema is not safe before Runtime",
        ));
    }
    for key in ["schemaSha256", "platformMetadataSha256"] {
        if !value[key].is_null() && !hex64(string(value, key)?) {
            return Err(failure("physical connector schema fingerprint is invalid"));
        }
    }
    if exists == value["schemaSha256"].is_null()
        || exists == value["platformMetadataSha256"].is_null()
        || unsigned(value, "localeChanges")? != 0
    {
        return Err(prerequisite(
            "physical_connector_first_open_not_quiescent_or_schema_presence_changed",
        ));
    }
    let mut total = 0usize;
    for table in ["distributors", "registrations", "tokens", "keys"] {
        let rows = array(value, table, 4096)?;
        total += rows.len();
        if !exists && !rows.is_empty() {
            return Err(failure("physical absent connector database reports rows"));
        }
        let mut ids = BTreeSet::new();
        for row in rows {
            let fields: &[&str] = match table {
                "distributors" => &[
                    "rowId",
                    "sha256",
                    "distributor",
                    "fallbackFrom",
                    "ack",
                    "dateInsertion",
                ],
                "registrations" => &[
                    "rowId",
                    "sha256",
                    "instanceSha256",
                    "messageMatchesPrivate",
                    "vapidNull",
                ],
                "tokens" => &["rowId", "sha256", "instanceSha256", "distributor"],
                _ => &["rowId", "sha256", "instanceSha256", "sealed"],
            };
            object(row, fields)?;
            if unsigned(row, "rowId")? == 0
                || !ids.insert(unsigned(row, "rowId")?)
                || !hex64(string(row, "sha256")?)
            {
                return Err(failure(
                    "physical connector row ID/hash is invalid or duplicated",
                ));
            }
            if table == "distributors" {
                id(string(row, "distributor")?)?;
                boolean(row, "ack")?;
                unsigned(row, "dateInsertion")?;
                if !row["fallbackFrom"].is_null() {
                    id(string(row, "fallbackFrom")?)?;
                }
            } else {
                if !hex64(string(row, "instanceSha256")?) {
                    return Err(failure(
                        "physical connector instance fingerprint is invalid",
                    ));
                }
                if table == "tokens" {
                    id(string(row, "distributor")?)?;
                }
                if table == "registrations" {
                    boolean(row, "messageMatchesPrivate")?;
                    boolean(row, "vapidNull")?;
                }
                if table == "keys" {
                    boolean(row, "sealed")?;
                }
            }
        }
    }
    if total > 4096 {
        return Err(failure("physical connector logical baseline exceeds bound"));
    }
    Ok(())
}
fn connector_admission(facts: &RetainedFacts, private: Option<&str>) -> io::Result<()> {
    validate_connector(&facts.connector)?;
    let distributors = array(&facts.connector, "distributors", 4096)?;
    if distributors.len() > 1
        || distributors.first().is_some_and(|row| {
            row["distributor"] != "io.heckel.ntfy"
                || !row["fallbackFrom"].is_null()
                || row["ack"] != true
        })
    {
        return Err(failure(
            "physical retained connector has another/fallback/unacknowledged distributor; never switch or normalize it",
        ));
    }
    if distributors.is_empty()
        && ["registrations", "tokens", "keys"]
            .iter()
            .any(|table| !facts.connector[*table].as_array().unwrap().is_empty())
    {
        return Err(failure(
            "physical retained connector has registration material without a qualified primary distributor",
        ));
    }
    if let Some(private) = private {
        let expected = digest_text(&format!("goatr.{APP}.{private}"));
        if ["registrations", "tokens", "keys"].iter().any(|table| {
            facts.connector[*table]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["instanceSha256"] == expected)
        }) {
            return Err(failure(
                "physical new connector instance collides with retained registration/token/key material",
            ));
        }
    }
    Ok(())
}
/// Returns true only for the exact genuinely new private registration/key/token delta.
fn connector_preserved(before: &Value, after: &Value, private: &str) -> io::Result<bool> {
    validate_connector(before)?;
    validate_connector(after)?;
    if before["databaseExists"] == true
        && (after["databaseExists"] != true
            || before["schemaSha256"] != after["schemaSha256"]
            || before["platformMetadataSha256"] != after["platformMetadataSha256"])
    {
        return Err(failure(
            "physical retained connector schema/platform metadata was removed or changed",
        ));
    }
    let instance = digest_text(&format!("goatr.{APP}.{private}"));
    let mut private_counts = [0usize; 3];
    for (index, table) in ["registrations", "tokens", "keys"].into_iter().enumerate() {
        let baseline = array(before, table, 4096)?;
        if baseline.iter().any(|row| row["instanceSha256"] == instance) {
            return Err(failure(
                "physical private connector instance existed in the original baseline",
            ));
        }
        let rows = array(after, table, 4096)?;
        let retained: Vec<_> = rows
            .iter()
            .filter(|row| row["instanceSha256"] != instance)
            .collect();
        if retained.len() != baseline.len()
            || retained
                .iter()
                .zip(baseline)
                .any(|(current, original)| *current != original)
        {
            return Err(failure(
                "physical retained connector registration/token/key rows changed",
            ));
        }
        let own: Vec<_> = rows
            .iter()
            .filter(|row| row["instanceSha256"] == instance)
            .collect();
        private_counts[index] = own.len();
        for row in own {
            if table == "registrations"
                && (row["messageMatchesPrivate"] != true || row["vapidNull"] != true)
                || table == "tokens" && row["distributor"] != "io.heckel.ntfy"
                || table == "keys" && row["sealed"] != true
            {
                return Err(failure(
                    "physical new private connector rows do not match exact official-ntfy registration/key facts",
                ));
            }
        }
    }
    let new_private = private_counts == [1, 1, 1];
    if private_counts != [0, 0, 0] && !new_private {
        return Err(failure(
            "physical private connector registration/token/key creation is partial or ambiguous",
        ));
    }
    if before["databaseExists"] == false && after["databaseExists"] == true && !new_private {
        return Err(failure(
            "physical connector database was created without the genuine owned private registration",
        ));
    }
    let old = array(before, "distributors", 4096)?;
    let current = array(after, "distributors", 4096)?;
    if !old.is_empty() {
        if current != old {
            return Err(failure(
                "physical original official distributor row changed, including ACK/date/fallback",
            ));
        }
    } else if !current.is_empty() {
        if !new_private
            || current.len() != 1
            || current[0]["distributor"] != "io.heckel.ntfy"
            || current[0]["ack"] != true
            || !current[0]["fallbackFrom"].is_null()
        {
            return Err(failure(
                "physical new global distributor is not solely the owned fresh official-ntfy setup",
            ));
        }
    } else if new_private {
        return Err(failure(
            "physical private connector material has no official distributor",
        ));
    }
    Ok(new_private)
}

#[derive(Clone, Debug)]
struct ProductEpoch {
    pid: u32,
    start: u64,
    uid: u32,
    public: [u8; 65],
}
impl ProductEpoch {
    fn verify(&self, value: &Value) -> io::Result<()> {
        object(value, &["pid", "start", "uid", "name", "caps"])?;
        if unsigned(value, "pid")? != u64::from(self.pid)
            || string(value, "start")?.parse::<u64>().ok() != Some(self.start)
            || unsigned(value, "uid")? != u64::from(self.uid)
            || string(value, "name")? != APP
        {
            return Err(failure(
                "physical original product PID/start/UID/default-process changed",
            ));
        }
        let caps = value.get("caps").unwrap();
        object(caps, &["CapInh", "CapPrm", "CapEff", "CapAmb"])?;
        for key in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
            if string(caps, key)? != "0000000000000000" {
                return Err(failure("physical product acquired a capability"));
            }
        }
        Ok(())
    }
}
struct ProductInstrumentation {
    epoch: watch::Receiver<Option<ProductEpoch>>,
    task: JoinHandle<io::Result<ProductEpoch>>,
}
impl ProductInstrumentation {
    async fn read_status(
        stdout: impl tokio::io::AsyncRead + Unpin,
        sender: watch::Sender<Option<ProductEpoch>>,
        mode: &'static str,
    ) -> io::Result<ProductEpoch> {
        let mut reader = tokio::io::BufReader::new(stdout.take(2_097_153));
        let mut line = String::new();
        let mut total = 0usize;
        let mut status = BTreeMap::<String, String>::new();
        let mut pinned = None;
        let mut finished = false;
        loop {
            line.clear();
            let count = reader.read_line(&mut line).await?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count)
                .ok_or_else(|| failure("physical instrumentation status overflow"))?;
            if count > 65536 || total > 2_097_152 {
                return Err(failure("physical instrumentation status exceeds bound"));
            }
            let record = line.trim_end_matches(['\r', '\n']);
            if let Some(value) = record.strip_prefix("INSTRUMENTATION_STATUS: ") {
                if let Some((key, value)) = value.split_once('=')
                    && [
                        "goatrOwner",
                        "goatrPid",
                        "goatrStart",
                        "goatrUid",
                        "goatrControlPublic",
                    ]
                    .contains(&key)
                    && status.insert(key.to_owned(), value.to_owned()).is_some()
                {
                    return Err(failure("physical instrumentation repeated an owner field"));
                }
            } else if record.starts_with("INSTRUMENTATION_STATUS_CODE:") && !status.is_empty() {
                if pinned.is_some() || status.len() != 5 {
                    return Err(failure(
                        "physical instrumentation owner status is not singular",
                    ));
                }
                let expected = if mode == "baseline" {
                    "physical-baseline-temporary"
                } else {
                    "physical-observer-instrumented"
                };
                if status.get("goatrOwner").map(String::as_str) != Some(expected) {
                    return Err(failure(
                        "physical instrumentation did not publish the qualified owner phase",
                    ));
                }
                let pid = status["goatrPid"]
                    .parse::<u32>()
                    .ok()
                    .filter(|pid| *pid > 0)
                    .ok_or_else(|| failure("physical instrumentation PID is invalid"))?;
                let start = status["goatrStart"]
                    .parse::<u64>()
                    .ok()
                    .filter(|start| *start > 0)
                    .ok_or_else(|| failure("physical instrumentation start is invalid"))?;
                let uid = status["goatrUid"]
                    .parse::<u32>()
                    .ok()
                    .filter(|uid| *uid >= 10000)
                    .ok_or_else(|| failure("physical instrumentation UID is invalid"))?;
                let public_text = &status["goatrControlPublic"];
                if public_text.len() != 130
                    || !public_text
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                {
                    return Err(failure("physical instrumentation public key is invalid"));
                }
                let mut public = [0u8; 65];
                for (index, byte) in public.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&public_text[index * 2..index * 2 + 2], 16)
                        .map_err(io::Error::other)?;
                }
                if public[0] != 4 {
                    return Err(failure(
                        "physical instrumentation public key is not uncompressed P-256",
                    ));
                }
                let value = ProductEpoch {
                    pid,
                    start,
                    uid,
                    public,
                };
                sender
                    .send(Some(value.clone()))
                    .map_err(|_| failure("physical instrumentation owner receiver ended"))?;
                pinned = Some(value);
                status.clear();
            } else if let Some(value) =
                record.strip_prefix("INSTRUMENTATION_RESULT: goatrPhysicalOutcome=")
            {
                if finished {
                    return Err(failure(
                        "physical instrumentation repeated its final outcome",
                    ));
                }
                let value: Value = serde_json::from_str(value)
                    .map_err(|_| failure("physical instrumentation final outcome is invalid"))?;
                if value["status"] == "blocked" {
                    object(&value, &["status", "code"])?;
                    let code = string(&value, "code")?;
                    if !matches!(
                        code,
                        "physical_preflight"
                            | "physical_observer"
                            | "retained_state_snapshot_unavailable"
                    ) {
                        return Err(failure(
                            "physical instrumentation returned an unknown prerequisite",
                        ));
                    }
                    return Err(prerequisite(code));
                }
                object(&value, &["status", "mode", "pid", "start"])?;
                let original = pinned.as_ref().ok_or_else(|| {
                    failure("physical instrumentation finished before owner publication")
                })?;
                if string(&value, "status")? != "finished"
                    || string(&value, "mode")? != mode
                    || unsigned(&value, "pid")? != u64::from(original.pid)
                    || string(&value, "start")?.parse::<u64>().ok() != Some(original.start)
                {
                    return Err(failure(
                        "physical instrumentation did not finish the original owner",
                    ));
                }
                finished = true;
            }
        }
        if !finished {
            return Err(failure(
                "physical instrumentation ended without a qualified finish",
            ));
        }
        pinned.ok_or_else(|| failure("physical instrumentation owner was never published"))
    }
    async fn start(
        adb: &PhysicalAdb,
        arguments: &BTreeMap<String, String>,
        mode: &'static str,
    ) -> io::Result<Self> {
        adb.recheck_server().await?;
        let mut words = vec![
            "am".to_owned(),
            "instrument".to_owned(),
            "-w".to_owned(),
            "-r".to_owned(),
        ];
        for (key, value) in arguments {
            words.extend(["-e".to_owned(), key.clone(), value.clone()]);
        }
        words.push(PRODUCT_RUNNER.to_owned());
        let remote = words
            .iter()
            .map(|word| super::shell_literal(word))
            .collect::<io::Result<Vec<_>>>()?
            .join(" ");
        let mut command = adb.command();
        command
            .args(["shell", &remote])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure("physical product instrumentation has no status pipe"))?;
        let (sender, epoch) = watch::channel(None);
        let task = tokio::spawn(async move {
            let original = Self::read_status(stdout, sender, mode).await?;
            if !child.wait().await?.success() {
                return Err(failure(
                    "physical instrumentation ended without a qualified finish",
                ));
            }
            Ok(original)
        });
        Ok(Self { epoch, task })
    }
    async fn original(&mut self) -> io::Result<ProductEpoch> {
        timeout(Duration::from_secs(35), async {
            loop {
                if let Some(epoch) = self.epoch.borrow().clone() {
                    return Ok(epoch);
                }
                if self.epoch.changed().await.is_err() {
                    return match (&mut self.task).await {
                        Ok(Err(error)) => Err(error),
                        Ok(Ok(_)) => {
                            Err(failure("physical instrumentation owner publication failed"))
                        }
                        Err(_) => Err(failure("physical instrumentation status owner failed")),
                    };
                }
            }
        })
        .await
        .map_err(|_| failure("physical instrumentation owner publication expired"))?
    }
    async fn finished(self) -> io::Result<ProductEpoch> {
        timeout(Duration::from_secs(35), self.task)
            .await
            .map_err(|_| failure("physical instrumentation finish expired"))?
            .map_err(|_| failure("physical instrumentation status owner failed"))?
    }
}

fn validate_keeper(value: &Value) -> io::Result<()> {
    object(
        value,
        &[
            "running",
            "reconnecting",
            "foregroundNotification",
            "endpointOwned",
            "networkCallback",
            "serviceRequested",
            "privateEnabled",
            "privateMode",
            "privateVerified",
            "privateRequiresKeeper",
            "privateCompleted",
            "retained",
            "privateRegistrationGeneration",
            "privateRegistrationSha256",
            "health",
        ],
    )?;
    for key in [
        "running",
        "foregroundNotification",
        "endpointOwned",
        "networkCallback",
        "serviceRequested",
        "privateEnabled",
        "privateVerified",
        "privateRequiresKeeper",
        "privateCompleted",
    ] {
        boolean(value, key)?;
    }
    if unsigned(value, "reconnecting")? > 128
        || !matches!(string(value, "privateMode")?, "Persistent" | "Ntfy")
    {
        return Err(failure("physical keeper facts are invalid"));
    }
    let mut hosts = BTreeSet::new();
    for host in array(value, "retained", 128)? {
        object(host, &["host", "requiresKeeper"])?;
        let host_id = string(host, "host")?;
        if !hex64(host_id) || !hosts.insert(host_id) {
            return Err(failure("physical retained keeper facts repeat a host"));
        }
        boolean(host, "requiresKeeper")?;
    }
    for key in ["privateRegistrationGeneration", "privateRegistrationSha256"] {
        if !value[key].is_null() {
            let pin = string(value, key)?;
            if key.ends_with("Generation") {
                uuid_text(pin)?;
            } else if !hex64(pin) {
                return Err(failure("physical registration identity is invalid"));
            }
        }
    }
    let health = &value["health"];
    object(
        health,
        &[
            "globalAvailable",
            "runtimePermissionRequired",
            "attentionChannelEnabled",
            "deliveryChannelEnabled",
        ],
    )?;
    for key in [
        "globalAvailable",
        "runtimePermissionRequired",
        "attentionChannelEnabled",
        "deliveryChannelEnabled",
    ] {
        boolean(health, key)?;
    }
    Ok(())
}
fn validate_snapshot(value: &Value, epoch: &ProductEpoch, host: &str) -> io::Result<()> {
    object(
        value,
        &[
            "kind",
            "process",
            "host",
            "device",
            "pairing",
            "foreground",
            "screen",
            "selectedHost",
            "selectedSession",
            "selectedGeneration",
            "selectedShell",
            "selectedShellGeneration",
            "hello",
            "inventoryReady",
            "mutationReady",
            "route",
            "sessions",
            "shells",
            "pending",
            "receipts",
            "actionIds",
            "controlBegun",
            "grid",
            "columns",
            "rows",
            "revision",
            "alternate",
            "scrollOffset",
            "scrollMaximum",
            "scrollViewport",
            "historyMode",
            "markerOutput",
            "oldInputRetired",
            "agentTerminal",
            "agentDisplay",
            "keeper",
            "conversation",
            "network",
            "ui",
            "shellInput",
            "creationReady",
        ],
    )?;
    boolean(value, "creationReady")?;
    let input = &value["shellInput"];
    object(input, &["offered", "draftBytes", "composition", "direct"])?;
    boolean(input, "offered")?;
    if !input["draftBytes"].is_null() && unsigned(input, "draftBytes")? > 65536 {
        return Err(failure(
            "physical local shell draft/recovery exceeds its real UTF-8 bound",
        ));
    }
    for key in ["composition", "direct"] {
        if !input[key].is_null() {
            boolean(input, key)?;
        }
    }
    let mut actions = BTreeSet::new();
    for action in array(value, "actionIds", 4096)? {
        let action = action
            .as_str()
            .ok_or_else(|| failure("physical durable action UUID is not a string"))?;
        uuid_text(action)?;
        if !actions.insert(action) {
            return Err(failure("physical durable action UUID is duplicated"));
        }
    }
    epoch.verify(&value["process"])?;
    if string(value, "kind")? != "snapshot" || string(value, "host")? != host {
        return Err(failure(
            "physical resident snapshot has the wrong host or kind",
        ));
    }
    for key in [
        "foreground",
        "hello",
        "inventoryReady",
        "mutationReady",
        "controlBegun",
        "grid",
        "alternate",
        "markerOutput",
        "oldInputRetired",
        "agentTerminal",
    ] {
        boolean(value, key)?;
    }
    for key in ["screen", "route", "historyMode"] {
        string(value, key)?;
    }
    for key in [
        "selectedHost",
        "selectedSession",
        "selectedGeneration",
        "selectedShell",
        "selectedShellGeneration",
    ] {
        if !value[key].is_null() {
            id(string(value, key)?)?;
        }
    }
    if !value["device"].is_null() {
        uuid_text(string(value, "device")?)?;
    }
    let pairing = &value["pairing"];
    object(
        pairing,
        &[
            "screenPairing",
            "previewPresent",
            "hostMatches",
            "nameMatches",
            "fingerprintMatches",
        ],
    )?;
    for key in [
        "screenPairing",
        "previewPresent",
        "hostMatches",
        "nameMatches",
        "fingerprintMatches",
    ] {
        boolean(pairing, key)?;
    }
    let columns = unsigned(value, "columns")?;
    let rows = unsigned(value, "rows")?;
    if columns > 512
        || rows > 256
        || columns * rows > 32768
        || boolean(value, "grid")? && (columns == 0 || rows == 0)
        || unsigned(value, "revision")? > 9_007_199_254_740_991
    {
        return Err(failure("physical resident Shell grid bounds changed"));
    }
    for key in ["scrollOffset", "scrollMaximum", "scrollViewport"] {
        if !value[key]
            .as_i64()
            .is_some_and(|value| (-1..=9_007_199_254_740_991).contains(&value))
        {
            return Err(failure("physical resident history metrics are invalid"));
        }
    }
    for session in array(value, "sessions", 8192)? {
        object(
            session,
            &[
                "session",
                "generation",
                "provider",
                "native",
                "server",
                "workspace",
                "pane",
                "terminal",
                "ready",
            ],
        )?;
        for key in [
            "session",
            "generation",
            "native",
            "server",
            "workspace",
            "pane",
            "terminal",
        ] {
            id(string(session, key)?)?;
        }
        if !matches!(string(session, "provider")?, "Omp" | "Codex") {
            return Err(failure(
                "physical resident provider is outside BOTH-provider scope",
            ));
        }
        boolean(session, "ready")?;
    }
    for shell in array(value, "shells", 8192)? {
        object(
            shell,
            &[
                "shell",
                "generation",
                "server",
                "workspace",
                "tab",
                "pane",
                "terminal",
                "ready",
            ],
        )?;
        for key in [
            "shell",
            "generation",
            "server",
            "workspace",
            "tab",
            "pane",
            "terminal",
        ] {
            id(string(shell, key)?)?;
        }
        boolean(shell, "ready")?;
    }
    for pending in array(value, "pending", 4096)? {
        object(pending, &["request", "kind"])?;
        uuid_text(string(pending, "request")?)?;
        string(pending, "kind")?;
    }
    for receipt in array(value, "receipts", 4096)? {
        validate_receipt(receipt)?;
    }
    let conversation = &value["conversation"];
    object(
        conversation,
        &[
            "historyCount",
            "historyIds",
            "browsingOlder",
            "hasOlder",
            "viewId",
            "activity",
            "controlsRevision",
            "optionsSha256",
            "optionsLoading",
            "optionsPending",
            "userMarkerCount",
            "assistantMarkerCount",
            "queueAvailable",
            "queueLoaded",
            "queueCount",
            "queueSha256",
            "delivery",
            "offeredDeliveries",
        ],
    )?;
    let records = array(conversation, "historyIds", 4096)?;
    if unsigned(conversation, "historyCount")? != records.len() as u64
        || unsigned(conversation, "userMarkerCount")? > records.len() as u64
        || unsigned(conversation, "assistantMarkerCount")? > records.len() as u64
    {
        return Err(failure("physical resident conversation counts are invalid"));
    }
    for record in records {
        id(record
            .as_str()
            .ok_or_else(|| failure("physical resident history identifier is invalid"))?)?;
    }
    for key in [
        "browsingOlder",
        "hasOlder",
        "optionsLoading",
        "optionsPending",
        "queueAvailable",
        "queueLoaded",
    ] {
        boolean(conversation, key)?;
    }
    string(conversation, "activity")?;
    unsigned(conversation, "controlsRevision")?;
    if !conversation["viewId"].is_null() {
        uuid_text(string(conversation, "viewId")?)?;
    }
    if !conversation["optionsSha256"].is_null() && !hex64(string(conversation, "optionsSha256")?) {
        return Err(failure("physical native options fingerprint is invalid"));
    }
    if unsigned(conversation, "queueCount")? > 4096
        || !matches!(
            string(conversation, "delivery")?,
            "None" | "Native" | "Steer" | "FollowUp" | "Aside" | "Queue"
        )
    {
        return Err(failure(
            "physical native queue/delivery metadata is invalid",
        ));
    }
    let mut offered = BTreeSet::new();
    for delivery in array(conversation, "offeredDeliveries", 5)? {
        let delivery = delivery
            .as_str()
            .filter(|delivery| {
                matches!(
                    *delivery,
                    "Native" | "Steer" | "FollowUp" | "Aside" | "Queue"
                )
            })
            .ok_or_else(|| failure("physical offered delivery is invalid"))?;
        if !offered.insert(delivery) {
            return Err(failure("physical offered delivery is duplicated"));
        }
    }
    if !conversation["queueSha256"].is_null() && !hex64(string(conversation, "queueSha256")?) {
        return Err(failure("physical native queue fingerprint is invalid"));
    }
    let display = &value["agentDisplay"];
    object(display, &["present", "sha256", "styled", "revision"])?;
    boolean(display, "present")?;
    boolean(display, "styled")?;
    unsigned(display, "revision")?;
    if !display["sha256"].is_null() && !hex64(string(display, "sha256")?) {
        return Err(failure(
            "physical decoded ANSI display fingerprint is invalid",
        ));
    }
    let ui = &value["ui"];
    object(
        ui,
        &[
            "orientation",
            "fontScale",
            "imeVisible",
            "interactive",
            "measuredAt",
        ],
    )?;
    boolean(ui, "imeVisible")?;
    boolean(ui, "interactive")?;
    unsigned(ui, "measuredAt")?;
    if unsigned(ui, "orientation")? > 2
        || !ui["fontScale"]
            .as_f64()
            .is_some_and(|scale| scale.is_finite() && scale > 0.0 && scale <= 5.0)
    {
        return Err(failure("physical native display/IME metadata is invalid"));
    }
    let network = &value["network"];
    object(
        network,
        &[
            "internet",
            "validated",
            "cellular",
            "wifi",
            "ethernet",
            "vpn",
            "usb",
        ],
    )?;
    for key in [
        "internet",
        "validated",
        "cellular",
        "wifi",
        "ethernet",
        "vpn",
        "usb",
    ] {
        boolean(network, key)?;
    }
    validate_keeper(&value["keeper"])
}
fn validate_receipt(value: &Value) -> io::Result<()> {
    object(
        value,
        &[
            "action",
            "method",
            "state",
            "target",
            "generation",
            "shell",
            "shellGeneration",
            "sha256",
            "createdSession",
            "createdGeneration",
            "createdShell",
            "createdShellGeneration",
        ],
    )?;
    uuid_text(string(value, "action")?)?;
    string(value, "method")?;
    if !hex64(string(value, "sha256")?) {
        return Err(failure("physical receipt fingerprint is invalid"));
    }
    if !matches!(
        string(value, "state")?,
        "Reserved" | "Dispatching" | "Dispatched" | "Succeeded" | "Failed" | "Uncertain"
    ) {
        return Err(failure("physical receipt state is invalid"));
    }
    for key in [
        "target",
        "generation",
        "shell",
        "shellGeneration",
        "createdSession",
        "createdGeneration",
        "createdShell",
        "createdShellGeneration",
    ] {
        if !value[key].is_null() {
            uuid_text(string(value, key)?)?;
        }
    }
    Ok(())
}
fn validate_report(value: &Value, epoch: &ProductEpoch, host: &str) -> io::Result<()> {
    match string(value, "kind")? {
        "ready" => {
            object(value, &["kind", "process", "retained"])?;
            epoch.verify(&value["process"])?;
            RetainedFacts::decode(&value["retained"])?;
        }
        "snapshot" => validate_snapshot(value, epoch, host)?,
        "epoch" => {
            object(
                value,
                &[
                    "kind",
                    "actionRunId",
                    "process",
                    "observedAt",
                    "apkSha256",
                    "snapshot",
                ],
            )?;
            epoch.verify(&value["process"])?;
            uuid_text(string(value, "actionRunId")?)?;
            unsigned(value, "observedAt")?;
            if !hex64(string(value, "apkSha256")?) {
                return Err(failure("physical epoch APK hash is invalid"));
            }
            validate_snapshot(&value["snapshot"], epoch, host)?;
        }
        "retained" => {
            object(value, &["kind", "facts"])?;
            RetainedFacts::decode(&value["facts"])?;
        }
        "keeper" => {
            object(value, &["kind", "facts"])?;
            validate_keeper(&value["facts"])?;
        }
        "notification" => {
            object(
                value,
                &["kind", "host", "event", "present", "qualified", "ledger"],
            )?;
            if string(value, "host")? != host {
                return Err(failure("physical notification observer changed host"));
            }
            uuid_text(string(value, "event")?)?;
            boolean(value, "present")?;
            boolean(value, "qualified")?;
            let ledger = &value["ledger"];
            object(
                ledger,
                &[
                    "present",
                    "sequence",
                    "session",
                    "dispositionSha256",
                    "active",
                    "attemptedAt",
                    "readThrough",
                ],
            )?;
            boolean(ledger, "present")?;
            unsigned(ledger, "readThrough")?;
            if !ledger["sequence"].is_null() {
                unsigned(ledger, "sequence")?;
            }
            if !ledger["attemptedAt"].is_null() {
                unsigned(ledger, "attemptedAt")?;
            }
            if !ledger["session"].is_null() {
                uuid_text(string(ledger, "session")?)?;
            }
            if !ledger["active"].is_null() {
                boolean(ledger, "active")?;
            }
            if !ledger["dispositionSha256"].is_null()
                && !hex64(string(ledger, "dispositionSha256")?)
            {
                return Err(failure(
                    "physical notification disposition fingerprint is invalid",
                ));
            }
        }
        "idle" | "finished" => {
            object(value, &["kind", "process"])?;
            epoch.verify(&value["process"])?;
        }
        _ => return Err(failure("physical observer returned an unknown report kind")),
    }
    Ok(())
}
fn validate_retained_report(value: &Value, epoch: &ProductEpoch, host: &str) -> io::Result<()> {
    let summary = retained_decode_summary(&value["facts"]);
    let result = if value["kind"] != "retained" {
        Err(failure("physical retained query/result kind mismatch"))
    } else {
        validate_report(value, epoch, host)
    };
    result.map_err(|error| {
        admission_failure(
            "retained_report",
            "envelope_or_kind",
            error,
            Some(json!(summary)),
        )
    })
}

struct Query {
    fields: Value,
    expected: &'static str,
    answer: oneshot::Sender<io::Result<(Value, Value)>>,
}
struct Observer {
    epoch: ProductEpoch,
    sender: mpsc::Sender<Query>,
    task: JoinHandle<io::Result<()>>,
    instrumentation: ProductInstrumentation,
}
struct ControlReplyBytes(Vec<u8>);
fn erase_control_reply(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
impl Drop for ControlReplyBytes {
    fn drop(&mut self) {
        erase_control_reply(&mut self.0);
    }
}
impl io::Write for ControlReplyBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > 65536)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "physical control response exceeds bound",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
async fn control_reply<W: tokio::io::AsyncWrite + Unpin>(
    stream: &mut W,
    value: &Value,
) -> io::Result<()> {
    // Single bounded serialization, no reallocation/copy of an earlier secret prefix; erase on EVERY exit.
    let mut bytes = ControlReplyBytes(Vec::with_capacity(65536));
    serde_json::to_writer(&mut bytes, value).map_err(io::Error::other)?;
    if !(2..=65536).contains(&bytes.0.len()) {
        return Err(failure("physical control response length is invalid"));
    }
    let header = (bytes.0.len() as u32).to_be_bytes();
    timeout(Duration::from_secs(5), async {
        stream.write_all(&header).await?;
        stream.write_all(&bytes.0).await
    })
    .await
    .map_err(|_| failure("physical control response expired"))??;
    Ok(())
}
async fn authenticated_report(
    stream: &mut TcpStream,
    epoch: &ProductEpoch,
    run: &Uuid,
    sequence: u64,
) -> io::Result<(Value, Value)> {
    let mut witness = Value::Null;
    let control =
        super::ui::authenticated_control_record(stream, &epoch.public, Some(&mut witness)).await?;
    match control {
        super::ui::Control::PhysicalReport {
            run_id,
            sequence: observed,
            report,
        } if run_id == *run && observed.0 == sequence => Ok((report, witness)),
        _ => Err(failure(
            "physical signed observer run/sequence/correlation changed",
        )),
    }
}
impl Observer {
    async fn start(
        listener: TcpListener,
        mut instrumentation: ProductInstrumentation,
        run: Uuid,
        host: String,
    ) -> io::Result<(Self, RetainedFacts)> {
        let epoch = instrumentation.original().await?;
        let (mut stream, peer) = timeout(Duration::from_secs(35), listener.accept())
            .await
            .map_err(|_| failure("physical product observer ready expired"))??;
        if !peer.ip().is_loopback() {
            return Err(failure("physical control peer is not loopback"));
        }
        let (ready, _) = authenticated_report(&mut stream, &epoch, &run, 0).await?;
        validate_report(&ready, &epoch, &host)?;
        if string(&ready, "kind")? != "ready" {
            return Err(failure(
                "physical product observer did not start with ready",
            ));
        }
        let facts = RetainedFacts::decode(&ready["retained"])?;
        control_reply(
            &mut stream,
            &json!({"kind":"wait","runId":run,"sequence":0}),
        )
        .await?;
        drop(stream);
        let (sender, mut receiver) = mpsc::channel::<Query>(1);
        let original = epoch.clone();
        let task = tokio::spawn(async move {
            let mut sequence = 1u64;
            let mut pending: Option<Query> = None;
            loop {
                let (mut stream, peer) = timeout(Duration::from_secs(20), listener.accept())
                    .await
                    .map_err(|_| failure("physical long observer stopped reporting"))??;
                if !peer.ip().is_loopback() {
                    return Err(failure("physical control peer is not loopback"));
                }
                let (report, witness) =
                    authenticated_report(&mut stream, &original, &run, sequence).await?;
                if pending
                    .as_ref()
                    .is_some_and(|query| query.expected == "retained")
                {
                    if let Err(error) = validate_retained_report(&report, &original, &host) {
                        // Preserve the closed rejection before the same original owner ends; never send its raw reply.
                        if let Some(query) = pending.take() {
                            let _ = query.answer.send(Err(error));
                        }
                        return Err(failure("physical retained report was rejected"));
                    }
                } else {
                    validate_report(&report, &original, &host)?;
                }
                let kind = string(&report, "kind")?;
                if let Some(query) = pending.take() {
                    if kind != query.expected {
                        return Err(failure("physical observer query/result kind mismatch"));
                    }
                    if kind == "epoch" && query.fields["actionRunId"] != report["actionRunId"]
                        || kind == "notification" && query.fields["event"] != report["event"]
                    {
                        return Err(failure(
                            "physical observer query argument correlation changed",
                        ));
                    }
                    let finishing = kind == "finished";
                    let _ = query.answer.send(Ok((report, witness)));
                    if finishing {
                        control_reply(
                            &mut stream,
                            &json!({"kind":"finished","runId":run,"sequence":sequence}),
                        )
                        .await?;
                        return Ok(());
                    }
                } else if kind != "idle" {
                    return Err(failure("physical observer emitted an unrequested report"));
                }
                let mut next = match receiver.try_recv() {
                    Ok(query) => {
                        let fields = query.fields.clone();
                        pending = Some(query);
                        fields
                    }
                    Err(mpsc::error::TryRecvError::Empty) => json!({"kind":"wait"}),
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        return Err(failure(
                            "physical observer coordinator ended before qualified finish",
                        ));
                    }
                };
                let fields = next
                    .as_object_mut()
                    .ok_or_else(|| failure("physical query is not an object"))?;
                fields.insert(
                    "runId".to_owned(),
                    serde_json::to_value(&run).map_err(io::Error::other)?,
                );
                fields.insert("sequence".to_owned(), json!(sequence));
                control_reply(&mut stream, &next).await?;
                sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| failure("physical observer sequence overflow"))?;
            }
        });
        Ok((
            Self {
                epoch,
                sender,
                task,
                instrumentation,
            },
            facts,
        ))
    }
    async fn query(&self, fields: Value, expected: &'static str) -> io::Result<(Value, Value)> {
        let (answer, result) = oneshot::channel();
        self.sender
            .send(Query {
                fields,
                expected,
                answer,
            })
            .await
            .map_err(|_| failure("physical original observer control owner ended"))?;
        timeout(Duration::from_secs(20), result)
            .await
            .map_err(|_| failure("physical observer query expired"))?
            .map_err(|_| failure("physical observer result owner ended"))?
    }
    async fn snapshot(&self, marker: Option<&str>) -> io::Result<Value> {
        self.query(
            json!({"kind":"snapshot","marker":marker.unwrap_or("")}),
            "snapshot",
        )
        .await
        .map(|(value, _)| value)
    }
    async fn keeper(&self) -> io::Result<Value> {
        self.query(json!({"kind":"keeper"}), "keeper")
            .await
            .map(|(value, _)| value["facts"].clone())
    }
    async fn retained(&self) -> io::Result<RetainedFacts> {
        let (value, _) = self.query(json!({"kind":"retained"}), "retained").await?;
        RetainedFacts::decode(&value["facts"])
    }
    async fn finished(self) -> io::Result<ProductEpoch> {
        self.query(json!({"kind":"finish"}), "finished").await?;
        timeout(Duration::from_secs(10), self.task)
            .await
            .map_err(|_| failure("physical control finish expired"))?
            .map_err(|_| failure("physical control owner failed"))??;
        self.instrumentation.finished().await
    }
}

fn recheck_original_host(authority: &HostAuthority) -> io::Result<()> {
    let observation = || {
        if let Some(pin) = &authority.controller_unit {
            pin.recheck(None)?;
        } else {
            dev_continuity::zero_native_caps(&authority.caller)?;
        }
        recheck_host_processes(&authority.caller, &authority.manager, None)?;
        let facts = &authority.facts;
        let current = fs::metadata(format!("/proc/{}/exe", authority.manager.pid))?;
        let original = authority.executable.metadata()?;
        let socket = fs::symlink_metadata("/run/user/1000/systemd/private")?;
        if (current.dev(), current.ino()) != (original.dev(), original.ino())
            || socket.dev() != unsigned(facts, "socketDevice")?
            || socket.ino() != unsigned(facts, "socketInode")?
            || socket.uid() != 1000
            || socket.mode() & 0o777 != 0o700
        {
            return Err(failure(
                "physical original protected manager authority changed",
            ));
        }
        Ok(())
    };
    let result = observation();
    if let Some(pin) = &authority.controller_unit {
        pin.observe(result)
    } else {
        result
    }
}
fn pin_process(process: &ProcessEvidence) -> io::Result<OwnedFd> {
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, process.pid, 0) } as i32;
    if raw < 0 {
        return Err(failure(
            "physical original child pidfd authority is unavailable",
        ));
    }
    let held = unsafe { OwnedFd::from_raw_fd(raw) };
    if herdr::process_evidence(process.pid)? != *process {
        return Err(failure(
            "physical original child changed while retaining its pidfd",
        ));
    }
    Ok(held)
}
fn companion_caps(
    process: &ProcessEvidence,
    pin: &goatr::codex_relay::UnitWakePin,
    deadline: Option<Instant>,
) -> io::Result<Value> {
    let state = pin.recheck(deadline)?;
    pin.observe(if pin.process() == process {
        Ok(())
    } else {
        Err(failure(
            "companion profile is not the original serving epoch",
        ))
    })?;
    Ok(
        json!({"masks":{"CapInh":format!("{:016x}",state.inheritable),"CapPrm":format!("{:016x}",state.permitted),
        "CapEff":format!("{:016x}",state.effective),"CapAmb":format!("{:016x}",state.ambient)},"NoNewPrivs":1,"actualOwnedUnitWakeQualified":true,"unit":pin.unit(),"cgroup":pin.cgroup()}),
    )
}
fn native_desktop_join(client: &Client, original: &ProcessEvidence) -> io::Result<()> {
    client.recheck()?;
    let source = &client.evidence;
    if source.pid != original.pid
        || source.pid_start_ticks != original.start_ticks
        || source.linux_boot_id != original.linux_boot_id
        || herdr::process_evidence(source.pid)? != *original
    {
        return Err(failure(
            "physical stock socket is not the original launched desktop process/namespace",
        ));
    }
    client.recheck()
}
struct StockRun {
    root: PathBuf,
    admin: PathBuf,
    config_path: PathBuf,
    config: Config,
    binary: PathBuf,
    socket: PathBuf,
    native_socket: PathBuf,
    client_socket: PathBuf,
    name: String,
    desktop: Desktop,
    serve: Companion,
    native: Client,
    binary_peer: Client,
    _native_pidfd: OwnedFd,
    _companion_pidfd: OwnedFd,
    _relay: Arc<std::net::TcpListener>,
    roots: (Value, Value),
    inputs: Vec<(PathBuf, File, fs::Metadata)>,
    admin_peer: tokio::net::UnixStream,
    admin_identity: Value,
    _endpoint: iroh::Endpoint,
    phone: Phone,
    serve_unit: goatr::codex_relay::UnitWakePin,
}
struct StockLaunchPins<'a> {
    template_sha: &'a str,
    expected_binary_sha: &'a str,
    controller: &'a goatr::codex_relay::UnitWakePin,
    original_inputs: &'a [(PathBuf, File, fs::Metadata)],
}
impl StockRun {
    async fn start(
        args: &clap::ArgMatches,
        evidence: &Path,
        run: Uuid,
        label: &str,
        deployment: Config,
        binary: PathBuf,
        pins: StockLaunchPins<'_>,
    ) -> io::Result<Self> {
        let StockLaunchPins {
            template_sha,
            expected_binary_sha,
            controller,
            original_inputs,
        } = pins;
        controller.recheck(None)?;
        recheck_inputs(original_inputs)?;
        let template = args
            .get_one::<PathBuf>("config")
            .ok_or_else(|| failure("physical frozen configuration is required"))?;
        if binary_sha256(template)? != template_sha {
            return Err(failure(
                "physical frozen original deployment changed before native launch; no new configuration is adopted",
            ));
        }
        if !deployment.allow_scoped_shell_probe
            || !deployment.allow_terminal_control
            || !deployment.allow_omp_controls
        {
            return Err(prerequisite(
                "physical matrix requires saved explicit scoped-probe, terminal-control, and OMP-control consent",
            ));
        }
        let workspace = fs::canonicalize(
            args.get_one::<PathBuf>("workspace-root")
                .ok_or_else(|| failure("physical workspace root is required"))?,
        )?;
        private_dir(&workspace, false)?;
        let runtime = PathBuf::from(
            std::env::var_os("XDG_RUNTIME_DIR")
                .ok_or_else(|| failure("physical unchanged real runtime is required"))?,
        );
        private_dir(&runtime, false)?;
        let suffix = &run.0[..8];
        let root = workspace.join(format!(".gmvp-{suffix}"));
        let admin = runtime.join(format!("gmvp-{suffix}"));
        private_directory(&root)?;
        private_directory(&admin)?;
        for path in [
            root.join("state"),
            root.join("runtime"),
            root.join("project"),
            root.join("runtime/b"),
            admin.join("config"),
            admin.join("config/herdr"),
            admin.join("config/herdr/sessions"),
        ] {
            private_directory(&path)?;
        }
        let name = format!("gmvp-{suffix}");
        private_directory(&admin.join(format!("config/herdr/sessions/{name}")))?;
        let socket = admin.join("admin.sock");
        let native_socket = admin.join(format!("config/herdr/sessions/{name}/herdr.sock"));
        goatr::config::socket_path(&native_socket)?;
        let mut config = deployment;
        config.runtime_dir = root.join("runtime");
        config.runtime_dir_agent = config.runtime_dir.clone();
        config.herdr_sockets = vec![native_socket.clone()];
        config.pairing_name = Some(Label::new(label).map_err(io::Error::other)?);
        config.allow_attachments = false;
        config.skill_document_roots.clear();
        config.projects = vec![ConfiguredProject {
            id: Uuid::random(),
            label: Label::new(format!("Goatr feature repo {suffix}")).map_err(io::Error::other)?,
            path: root.join("project"),
            allow_read_tools: false,
        }];
        let owner = CodexOwner {
            id: Uuid::random(),
            socket_host: root.join("runtime/c.sock"),
            socket_agent: root.join("runtime/c.sock"),
            bindings_dir: root.join("runtime/b"),
            unit: format!("goatr-physical-{suffix}.service"),
        };
        config.default_codex_owner_id = Some(owner.id.clone());
        config.codex_owners = vec![owner];
        let config_path = root.join("config.toml");
        let lock = goatr::setup::SetupLock::acquire(&root)?;
        lock.publish(
            &config_path,
            &goatr::setup::ExpectedFile::Absent,
            config.serialized()?.as_bytes(),
        )?;
        drop(lock);
        let native_config = admin.join("config/physical-herdr.toml");
        let lock = goatr::setup::SetupLock::acquire(&admin.join("config"))?;
        lock.publish(
            &native_config,
            &goatr::setup::ExpectedFile::Absent,
            b"[update]\nversion_check = false\n",
        )?;
        drop(lock);
        let relay = Arc::new(std::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        ))?);
        relay.set_nonblocking(true)?;
        let port = relay.local_addr()?.port();
        let fd = fs::metadata(format!("/proc/self/fd/{}", relay.as_raw_fd()))?;
        let listener = goatr::backends::omp::OwnedRelayListener {
            v: 1,
            fd: relay.as_raw_fd(),
            device: fd.dev(),
            inode: fd.ino(),
            port,
            parent: herdr::process_evidence(std::process::id())?,
            runtime_dir: config.runtime_dir.clone(),
        };
        let lock = goatr::setup::SetupLock::acquire(&config.runtime_dir)?;
        lock.publish(
            &config.runtime_dir.join("omp-relay.owner"),
            &goatr::setup::ExpectedFile::Absent,
            &serde_json::to_vec(&listener).map_err(io::Error::other)?,
        )?;
        drop(lock);
        goatr::backends::omp::overlay(&config, port)?;
        let roots = (identity(&root)?, identity(&admin)?);
        let config_sha256 = binary_sha256(&config_path)?;
        let package_sha = expected_binary_sha;
        let mut inputs = Vec::with_capacity(4);
        for path in [&binary, &config_path, &native_config, template] {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || complete_stamp(&metadata) != complete_stamp(&fs::symlink_metadata(path)?)
            {
                return Err(failure(
                    "physical original input changed while its descriptor was retained",
                ));
            }
            if path == &binary || path == template {
                let original = original_inputs
                    .iter()
                    .find(|(name, _, _)| name == path)
                    .ok_or_else(|| {
                        failure("physical startup lost its original caller-qualified input")
                    })?;
                if complete_stamp(&metadata) != complete_stamp(&original.2) {
                    return Err(failure(
                        "physical startup input differs from its original pre-await descriptor",
                    ));
                }
            }
            inputs.push((path.to_path_buf(), file, metadata));
        }
        recheck_inputs(original_inputs)?;
        recheck_inputs(&inputs)?;
        write_json(
            &evidence.join("physical-resources-before-launch.json"),
            &json!({"runId":run,"root":roots.0,"admin":roots.1,"template":identity(template)?,"templateSha256":template_sha,"config":identity(&config_path)?,"configSha256":config_sha256,"nativeConfig":identity(&native_config)?,"nativeConfigSha256":binary_sha256(&native_config)?,"nativeConfigPathVariable":"HERDR_CONFIG_PATH","companion":binary,"companionSha256":package_sha,"companionLockSha256":transport::lock_sha256(),"OMPownedListener":listener,"nativeInputAutomation":false,"privateCaptures":false,"nativeClose":false,"resourcesRetainedOnFailure":true}),
        )?;
        let mut command = std::process::Command::new(&config.executables.herdr);
        command
            .args(["--session", &name, "server"])
            .current_dir(&config.projects[0].path)
            .env("HERDR_STARTUP_CWD", &config.projects[0].path)
            .env("HERDR_CONFIG_PATH", &native_config)
            .env("XDG_CONFIG_HOME", admin.join("config"))
            .env("XDG_STATE_HOME", root.join("state"));
        for key in [
            "HERDR_SOCKET_PATH",
            "HERDR_CLIENT_SOCKET_PATH",
            "HERDR_SESSION",
            "HERDR_SESSION_NAME",
            "HERDR_WORKSPACE_ID",
            "HERDR_TAB_ID",
            "HERDR_PANE_ID",
            "HERDR_RENDER_PROF",
        ] {
            command.env_remove(key);
        }
        recheck_inputs(original_inputs)?;
        recheck_inputs(&inputs)?;
        let mut desktop = Desktop::spawn_held(
            command,
            None,
            Some(&evidence.join("physical-herdr-spawn.json")),
        )?;
        let native_pidfd = pin_process(&desktop.process)?;
        dev_continuity::zero_native_caps(&desktop.process)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let native = loop {
            desktop.recheck()?;
            recheck_inputs(original_inputs)?;
            recheck_inputs(&inputs)?;
            let connected = Client::connect(&native_socket).await;
            desktop.recheck()?;
            recheck_inputs(original_inputs)?;
            recheck_inputs(&inputs)?;
            match connected {
                Ok(client) => {
                    let client = client.qualify_executable(&config.executables.herdr)?;
                    native_desktop_join(&client, &desktop.process)?;
                    break client;
                }
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    recheck_inputs(original_inputs)?;
                    recheck_inputs(&inputs)?;
                }
                Err(_) => {
                    return Err(prerequisite(
                        "physical original stock public socket is unavailable; desktop retained",
                    ));
                }
            }
        };
        native.recheck()?;
        desktop.recheck()?;
        recheck_inputs(&inputs)?;
        let initial_client = Client::connect(&native_socket).await?;
        native.recheck()?;
        desktop.recheck()?;
        recheck_inputs(&inputs)?;
        let initial_client = initial_client.qualify_executable(&config.executables.herdr)?;
        if initial_client.evidence != native.evidence {
            return Err(failure(
                "physical initial snapshot peer differs from the retained original source",
            ));
        }
        native.recheck()?;
        recheck_inputs(&inputs)?;
        let (initial_evidence, initial) = initial_client.snapshot().await?;
        native.recheck()?;
        desktop.recheck()?;
        recheck_inputs(&inputs)?;
        if initial_evidence != native.evidence {
            return Err(failure(
                "physical initial snapshot returned another source epoch",
            ));
        }
        let client_socket = native_socket.with_file_name("herdr-client.sock");
        let binary_peer = Client::connect(&client_socket)
            .await?
            .qualify_executable(&config.executables.herdr)?;
        native.recheck()?;
        desktop.recheck()?;
        recheck_inputs(&inputs)?;
        for client in [&native, &binary_peer] {
            native_desktop_join(client, &desktop.process)?;
        }
        recheck_inputs(original_inputs)?;
        recheck_inputs(&inputs)?;
        controller.recheck(Some(deadline))?;
        let mut serve = start_serve_with_capture(
            &binary,
            &config_path,
            &root,
            &socket,
            Some(&relay),
            ServeLaunch {
                diagnostic_capture: false,
                herdr_report_capture: None,
                preserve_xdg_config_home: false,
                retain_process: true,
                hello_capture: false,
                grid_hold: None,
                project_read: None,
                startup_deadline: Some(deadline),
                unit_authority: Some(controller),
            },
        )?;
        // Capture the ORIGINAL child profile before any admin/readiness/pairing effect.
        let held = original_inputs
            .iter()
            .find(|(path, _, _)| path == &binary)
            .ok_or_else(|| failure("serving held artifact missing"))?;
        let serve_unit = controller.capture_child(serve.process.clone(), &held.1, deadline)?;
        startup_profile_deadline(controller, &serve_unit, deadline)?;
        recheck_inputs(&inputs)?;
        tokio::time::timeout_at(deadline.into(), healthy(&mut serve, &socket))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "original serving startup deadline")
            })??;
        startup_profile_deadline(controller, &serve_unit, deadline)?;
        recheck_inputs(&inputs)?;
        let companion_pidfd = pin_process(&serve.process)?;
        let caps = companion_caps(&serve.process, &serve_unit, Some(deadline))?;
        let admin_peer =
            tokio::time::timeout_at(deadline.into(), tokio::net::UnixStream::connect(&socket))
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "original admin startup deadline")
                })??;
        startup_profile_deadline(controller, &serve_unit, deadline)?;
        recheck_inputs(&inputs)?;
        let peer = admin_peer.peer_cred()?;
        if peer.uid() != uid() || peer.pid() != Some(serve.process.pid as i32) {
            return Err(failure(
                "physical exact private admin peer is not the original companion",
            ));
        }
        let admin_identity = identity(&socket)?;
        let executable = fs::metadata(format!("/proc/{}/exe", serve.process.pid))?;
        if (executable.dev(), executable.ino()) != (inputs[0].2.dev(), inputs[0].2.ino()) {
            return Err(failure(
                "physical original companion executable differs from the held frozen input",
            ));
        }
        recheck_inputs(&inputs)?;
        let endpoint = tokio::time::timeout_at(
            deadline.into(),
            goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?)).bind(),
        )
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "original endpoint startup deadline",
            )
        })?
        .map_err(io::Error::other)?;
        startup_profile_deadline(controller, &serve_unit, deadline)?;
        recheck_inputs(&inputs)?;
        // Await the deadline-aware blocking owner itself: dropping its join handle would detach the CLI.
        let phone = pair(&binary, &root, &socket, &endpoint, Some(deadline)).await?;
        startup_profile_deadline(controller, &serve_unit, deadline)?;
        recheck_inputs(&inputs)?;
        recheck_inputs(original_inputs)?;
        write_json(
            &evidence.join("physical-originals.json"),
            &json!({"runId":run,"native":desktop.process,"publicSocket":native.evidence,"binarySocket":binary_peer.evidence,"companion":serve.process,"companionCaps":caps,"nativeInitial":{"version":initial.version,"protocol":initial.protocol,"panes":initial.panes.len(),"workspaces":initial.workspaces.len()},"nativeSnapshotIsNotPhoneOutput":true,"helperPairIsNotCameraPairingProof":true,"retained":true}),
        )?;
        startup_profile_deadline(controller, &serve_unit, deadline)?;
        Ok(Self {
            root,
            admin,
            config_path,
            config,
            binary,
            socket,
            native_socket,
            client_socket,
            name,
            desktop,
            serve,
            native,
            binary_peer,
            _native_pidfd: native_pidfd,
            _companion_pidfd: companion_pidfd,
            _relay: relay,
            roots,
            inputs,
            admin_peer,
            admin_identity,
            _endpoint: endpoint,
            phone,
            serve_unit,
        })
    }
    fn recheck(&mut self) -> io::Result<()> {
        self.desktop.recheck()?;
        self.native.recheck()?;
        self.binary_peer.recheck()?;
        dev_continuity::zero_native_caps(&self.desktop.process)?;
        companion_caps(&self.serve.process, &self.serve_unit, None)?;
        recheck_inputs(&self.inputs)?;
        for (path, source) in [
            (&self.native_socket, &self.native.evidence),
            (&self.client_socket, &self.binary_peer.evidence),
        ] {
            let socket = fs::symlink_metadata(path)?;
            if !socket.file_type().is_socket()
                || socket.uid() != uid()
                || socket.mode() & 0o777 != 0o600
                || socket.dev() != source.socket_device
                || socket.ino() != source.socket_inode
            {
                return Err(failure(
                    "physical original public/client socket object changed",
                ));
            }
        }
        let peer = self.admin_peer.peer_cred()?;
        if peer.uid() != uid()
            || peer.pid() != Some(self.serve.process.pid as i32)
            || identity(&self.socket)? != self.admin_identity
            || identity(&self.root)? != self.roots.0
            || identity(&self.admin)? != self.roots.1
        {
            return Err(failure(
                "physical original resource/package/config identity changed",
            ));
        }
        Ok(())
    }
    async fn inventory(&mut self) -> io::Result<HostSnapshotResult> {
        self.recheck()?;
        let inventory = self.phone.host().await?;
        self.recheck()?;
        Ok(inventory)
    }
    async fn native_snapshot(&mut self) -> io::Result<Snapshot> {
        self.recheck()?;
        let client = Client::connect(&self.native_socket).await?;
        self.recheck()?;
        let client = client.qualify_executable(&self.config.executables.herdr)?;
        if client.evidence != self.native.evidence {
            return Err(failure(
                "physical snapshot request is not the original retained source",
            ));
        }
        self.recheck()?;
        let (evidence, snapshot) = client.snapshot().await?;
        self.recheck()?;
        if evidence != self.native.evidence {
            return Err(failure(
                "physical snapshot response changed its original source",
            ));
        }
        Ok(snapshot)
    }
    async fn native_process_info(&mut self, pane: &Id) -> io::Result<herdr::PaneProcessInfo> {
        self.recheck()?;
        let client = Client::connect(&self.native_socket).await?;
        self.recheck()?;
        let client = client.qualify_executable(&self.config.executables.herdr)?;
        if client.evidence != self.native.evidence {
            return Err(failure(
                "physical process-info request is not the original retained source",
            ));
        }
        self.recheck()?;
        let info = client.process_info(pane).await?;
        self.recheck()?;
        Ok(info)
    }
    async fn provider_proof(&mut self, session: &SessionSummary) -> io::Result<NativeProof> {
        self.recheck()?;
        let proof = proof_selected(
            &self.config,
            &self.native_socket,
            session,
            None,
            &self.native,
        )
        .await?;
        self.recheck()?;
        proof.recheck_processes()?;
        for process in std::iter::once(&proof.owner)
            .chain(proof.tui.iter())
            .chain(proof.relay.iter())
        {
            if process.pid == self.serve.process.pid {
                return Err(failure(
                    "physical provider proof aliases the retained serving companion",
                ));
            }
            dev_continuity::zero_native_caps(process)?;
        }
        Ok(proof)
    }
}
fn startup_profile_deadline(
    controller: &goatr::codex_relay::UnitWakePin,
    serve: &goatr::codex_relay::UnitWakePin,
    deadline: Instant,
) -> io::Result<()> {
    controller.recheck(Some(deadline))?;
    serve.recheck(Some(deadline))?;
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "original absolute startup deadline expired after unit/profile proof",
        ));
    }
    Ok(())
}

fn argument_hash(arguments: &BTreeMap<String, String>) -> io::Result<String> {
    let mut material = String::new();
    for (key, value) in arguments {
        if key.len() > 128
            || value.len() > 4096
            || !key
                .bytes()
                .chain(value.bytes())
                .all(|byte| (32..127).contains(&byte))
        {
            return Err(failure(
                "physical UI arguments are not bounded authored ASCII",
            ));
        }
        use std::fmt::Write as _;
        write!(
            &mut material,
            "{}:{}{}:{}",
            key.len(),
            key,
            value.len(),
            value
        )
        .map_err(io::Error::other)?;
    }
    if material.len() > 65536 {
        return Err(failure("physical UI argument fingerprint exceeds bound"));
    }
    Ok(digest_text(&material))
}
fn proof_facts(proof: &NativeProof) -> io::Result<Value> {
    Ok(
        json!({"target":proof.target,"provider":proof.provider,"native":proof.native,"pane":proof.pane,"terminal":proof.terminal,"workspace":proof.workspace,
        "owner":proof.owner,"tui":proof.tui,"relay":proof.relay,"bindingFactsSha256":digest_text(&serde_json::to_string(&proof.objects).map_err(io::Error::other)?)}),
    )
}
fn target_arguments(target: &Target) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("ownedUiSessionId".to_owned(), target.session_id.0.clone()),
        (
            "ownedUiSessionGeneration".to_owned(),
            target.generation.0.clone(),
        ),
    ])
}
fn shell_arguments(target: &ShellTarget) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("ownedUiShellId".to_owned(), target.shell_id.0.clone()),
        (
            "ownedUiShellGeneration".to_owned(),
            target.generation.0.clone(),
        ),
    ])
}
fn extend(arguments: &mut BTreeMap<String, String>, fields: &[(&str, &str)]) {
    for (key, value) in fields {
        arguments.insert((*key).to_owned(), (*value).to_owned());
    }
}
/// Durable mutation/receipt action UUIDs, including reserved rows without a hydrated UI receipt.
fn receipt_ids(snapshot: &Value) -> io::Result<BTreeSet<String>> {
    array(snapshot, "actionIds", 4096)?
        .iter()
        .map(|action| {
            action
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| failure("physical durable action UUID is invalid"))
        })
        .collect()
}
fn original_session<'a>(snapshot: &'a Value, target: &Target) -> io::Result<&'a Value> {
    let mut matching = array(snapshot, "sessions", 8192)?.iter().filter(|session| {
        session["session"] == target.session_id.0 && session["generation"] == target.generation.0
    });
    let session = matching
        .next()
        .ok_or_else(|| failure("physical original provider target disappeared"))?;
    if matching.next().is_some() {
        return Err(failure("physical original provider target is ambiguous"));
    }
    Ok(session)
}
fn keeper_retained_current(keeper: &Value, baseline: &RetainedFacts) -> io::Result<bool> {
    validate_keeper(keeper)?;
    let actual: BTreeSet<_> = array(keeper, "retained", 128)?
        .iter()
        .map(|row| string(row, "host").map(str::to_owned))
        .collect::<io::Result<_>>()?;
    let expected: BTreeSet<_> = baseline
        .hosts
        .iter()
        .map(|host| host.host.clone())
        .collect();
    if actual != expected {
        return Err(failure(
            "physical retained host set changed; no owned global delivery request may be started or stopped",
        ));
    }
    Ok(array(keeper, "retained", 128)?
        .iter()
        .any(|row| row["requiresKeeper"] == true))
}
#[derive(Debug, PartialEq, Eq)]
enum ShellAdmission {
    Metadata,
    Pending,
    Committed,
}
fn shell_admission(snapshot: &Value, target: &ShellTarget) -> Option<ShellAdmission> {
    if snapshot["selectedShell"] != target.shell_id.0
        || snapshot["selectedShellGeneration"] != target.generation.0
    {
        return None;
    }
    match (
        snapshot["controlBegun"].as_bool()?,
        snapshot["grid"].as_bool()?,
        snapshot["shellInput"]["offered"].as_bool()?,
    ) {
        (false, false, false) => Some(ShellAdmission::Metadata),
        (true, false, false) => Some(ShellAdmission::Pending),
        (true, true, _) if snapshot["oldInputRetired"] == false => Some(ShellAdmission::Committed),
        _ => None,
    }
}
fn nonzero_newer(snapshot: &Value) -> Option<u64> {
    let offset = snapshot["scrollOffset"].as_u64()?;
    let rows = snapshot["scrollViewport"].as_u64()?;
    (rows > 0 && offset > rows).then(|| offset - rows)
}
fn shell_history_ack(snapshot: &Value, target: &ShellTarget, action: &str, offset: u64) -> bool {
    shell_admission(snapshot, target) == Some(ShellAdmission::Committed)
        && snapshot["scrollOffset"] == offset
        && snapshot["historyMode"] == if offset == 0 { "Live" } else { "Browsing" }
        && snapshot["shellInput"]["offered"] == (offset == 0)
        && snapshot["receipts"].as_array().is_some_and(|receipts| {
            receipts.iter().any(|receipt| {
                receipt["action"] == action
                    && receipt["method"] == "ShellScroll"
                    && receipt["state"] == "Succeeded"
                    && receipt["shell"] == target.shell_id.0
                    && receipt["shellGeneration"] == target.generation.0
            })
        })
}
fn provider_controls_admission(provider: Provider, conversation: &Value) -> io::Result<bool> {
    let offered = array(conversation, "offeredDeliveries", 5)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| failure("physical offered delivery is not a string"))
        })
        .collect::<io::Result<BTreeSet<_>>>()?;
    let queue = boolean(conversation, "queueAvailable")?;
    match provider {
        Provider::Omp
            if !queue && offered == BTreeSet::from(["Native", "Steer", "FollowUp", "Aside"]) =>
        {
            Ok(false)
        }
        Provider::Codex
            if queue
                && offered.contains("Native")
                && offered.contains("Queue")
                && !offered.contains("FollowUp")
                && !offered.contains("Aside") =>
        {
            Ok(true)
        }
        _ => Err(failure(
            "physical provider catalogue does not admit its actual stock delivery/queue capabilities",
        )),
    }
}

#[derive(Clone, Copy)]
enum UiExpectation {
    Completed,
    StaleRefusal,
    NotificationTap { api: u32 },
}
fn notification_human_fallback(api: u32, outcome: &Value) -> io::Result<bool> {
    Ok(api == 30
        && string(outcome, "outcome")? == "blocked"
        && string(outcome, "guard")? == "physical_notification_row_ambiguous"
        && string(outcome, "stage")? == "selection"
        && !boolean(outcome, "acknowledged")?)
}
fn closed_ui_token(outcome: &Value, key: &str) -> Value {
    outcome
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 96
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
        .map_or_else(|| json!("unavailable"), |value| json!(value))
}
struct UiOutcome<'a> {
    run: &'a Uuid,
    action: &'a str,
    argument_hash: &'a str,
    original: &'a Value,
    process: &'a Value,
    outcome: &'a Value,
    api: u32,
    command_succeeded: bool,
}
fn ui_outcome_projection(input: &UiOutcome<'_>, pre_input_refusal: Option<bool>) -> Value {
    json!({"runId":input.run,"action":input.action,"argumentHash":input.argument_hash,"process":input.process,"outcome":closed_ui_token(input.outcome,"outcome"),"guard":closed_ui_token(input.outcome,"guard"),
        "stage":closed_ui_token(input.outcome,"stage"),"qualifiedApi":input.api,"commandExit":if input.command_succeeded{"success"}else{"nonzero_or_signal"},
        "preInputRefusal":pre_input_refusal,"acknowledged":input.outcome.get("acknowledged").and_then(Value::as_bool),"expectedScreenObserved":input.outcome.get("expectedScreenObserved").and_then(Value::as_bool),"acknowledgementIsNotNativeEffect":true})
}
fn publish_ui_outcome(
    evidence: &Path,
    input: &UiOutcome<'_>,
    projection: &Value,
) -> io::Result<()> {
    write_json(
        &evidence.join(format!("physical-ui-{}.outcome.json", input.run.0)),
        projection,
    )
}
fn correlated_ui_outcome(
    original: &Value,
    process: &Value,
    outcome: &Value,
    run: &Uuid,
    action: &str,
) -> io::Result<()> {
    if original != process
        || string(outcome, "runId")? != run.0
        || string(outcome, "action")? != action
        || outcome["runnerPid"] != process["runnerPid"]
        || outcome["runnerStart"] != process["runnerStart"]
    {
        return Err(failure(
            "physical owned UI original process/action correlation changed",
        ));
    }
    Ok(())
}
fn ui_expectation_admission(
    expectation: UiExpectation,
    outcome: &Value,
) -> io::Result<(bool, bool)> {
    let result = string(outcome, "outcome")?;
    let guard = string(outcome, "guard")?;
    let _stage = string(outcome, "stage")?;
    let acknowledged = boolean(outcome, "acknowledged")?;
    let observed = boolean(outcome, "expectedScreenObserved")?;
    let ambiguous = match expectation {
        UiExpectation::NotificationTap { api } => notification_human_fallback(api, outcome)?,
        _ => false,
    };
    let admitted = match expectation {
        UiExpectation::Completed => result == "completed" && guard == "none" && observed,
        UiExpectation::StaleRefusal => {
            result == "blocked" && guard == "ui_tag_missing" && !acknowledged
        }
        UiExpectation::NotificationTap { .. } => {
            (result == "completed" && guard == "none" && acknowledged && observed) || ambiguous
        }
    };
    Ok((admitted, ambiguous))
}
fn admit_published_ui_outcome(
    command_succeeded: bool,
    classification: io::Result<(bool, bool)>,
) -> io::Result<bool> {
    if !command_succeeded {
        return Err(failure(
            "physical owned UI runner command failed after a correlated outcome",
        ));
    }
    let (admitted, ambiguous) = classification?;
    if !admitted {
        return Err(failure(
            "physical owned UI original action did not produce its exact expected completion/refusal; no retry",
        ));
    }
    Ok(ambiguous)
}
fn publish_and_admit_ui_outcome(
    evidence: &Path,
    expectation: UiExpectation,
    input: UiOutcome<'_>,
) -> io::Result<(Value, bool)> {
    correlated_ui_outcome(
        input.original,
        input.process,
        input.outcome,
        input.run,
        input.action,
    )?;
    let classification = ui_expectation_admission(expectation, input.outcome);
    let pre_input_refusal = classification
        .as_ref()
        .ok()
        .map(|(_, ambiguous)| *ambiguous);
    let projection = ui_outcome_projection(&input, pre_input_refusal);
    publish_ui_outcome(evidence, &input, &projection)?;
    let ambiguous = admit_published_ui_outcome(input.command_succeeded, classification)?;
    Ok((projection, ambiguous))
}
fn background_noninteractive(snapshot: &Value) -> bool {
    snapshot["foreground"] == false
        && snapshot["ui"]["interactive"] == false
        && snapshot["ui"]["measuredAt"]
            .as_u64()
            .is_some_and(|time| time > 0)
}
fn retain_profile_observation<T>(
    result: io::Result<T>,
    profile_failed: bool,
    failed: &mut bool,
) -> io::Result<T> {
    if profile_failed {
        *failed = true;
    }
    match result {
        Ok(value) if !*failed => Ok(value),
        Ok(_) => Err(failure("original profile observation failure is permanent")),
        Err(error) => Err(error),
    }
}
fn recheck_unit_profiles(
    authority: &HostAuthority,
    stock: &StockRun,
    failed: &mut bool,
) -> io::Result<()> {
    if *failed {
        return Err(failure(
            "original physical input/profile failure is permanent",
        ));
    }
    let check = || {
        authority
            .controller_unit
            .as_ref()
            .ok_or_else(|| failure("original controller profile absent"))?
            .recheck(None)?;
        stock.serve_unit.recheck(None)?;
        Ok(())
    };
    let result = check();
    let profile_failed = result.is_err();
    retain_profile_observation(result, profile_failed, failed)
}
struct Matrix {
    adb: PhysicalAdb,
    observer: Observer,
    stock: StockRun,
    authority: HostAuthority,
    inputs: Vec<(PathBuf, File, fs::Metadata)>,
    input_pin_failed: bool,
    run: Uuid,
    host: String,
    label: String,
    evidence: PathBuf,
    product_sha: String,
    public_sha: String,
    api: u32,
    control_port: u16,
    fresh_baseline: RetainedFacts,
    first_open: Option<RetainedFacts>,
    paired: Option<RetainedFacts>,
    last_stage: &'static str,
    baseline_ui: Option<Value>,
    baseline_network: Option<Value>,
    retained_keeper_baseline: Option<Value>,
    owned_start: Option<Uuid>,
    service_key_restored: bool,
    product_epoch: InstalledProductEpoch,
    prepared_record_sha256: Option<String>,
}
impl Matrix {
    fn recheck(&mut self) -> io::Result<()> {
        recheck_retained_inputs(&self.inputs, &mut self.input_pin_failed)?;
        let result = recheck_original_host(&self.authority);
        retain_profile_observation(
            result,
            self.authority
                .controller_unit
                .as_ref()
                .is_some_and(|pin| pin.failed()),
            &mut self.input_pin_failed,
        )?;
        let result = self.stock.recheck();
        retain_profile_observation(
            result,
            self.stock.serve_unit.failed(),
            &mut self.input_pin_failed,
        )?;
        Ok(())
    }
    async fn recheck_product(&mut self) -> io::Result<()> {
        self.recheck()?;
        require_product_epoch(
            &self.adb,
            &self.product_epoch,
            &self.product_sha,
            &self.inputs,
            Some(&mut self.input_pin_failed),
        )
        .await?;
        self.recheck()
    }
    fn first_open_ref(&self) -> io::Result<&RetainedFacts> {
        self.first_open
            .as_ref()
            .ok_or_else(|| failure("physical immutable first-open reference has not been admitted"))
    }
    fn record(&self, name: &str, facts: &Value) -> io::Result<()> {
        write_json(
            &self
                .evidence
                .join(format!("physical-{name}-{}.json", Uuid::random().0)),
            &json!({"runId":self.run,"host":self.host,"productPid":self.observer.epoch.pid,
            "productStart":self.observer.epoch.start,"productApkSha256":self.product_sha,"facts":facts,"instrumentedLiveness":true,
            "preparedRecordSha256":self.prepared_record_sha256,
            "coldSurvivalProved":false,"foregroundServiceSoleLivenessProved":false,"humanAcknowledgementIsNotEffectProof":true}),
        )
    }
    async fn human_stage(&mut self, stage: &'static str, instruction: &str) -> io::Result<()> {
        self.last_stage = stage;
        self.recheck_product().await?;
        self.adb.tunnels(Some(self.control_port)).await?;
        timeout(Duration::from_secs(1800), super::human(instruction))
            .await
            .map_err(|_| {
                failure("physical real human stage deadline expired; no action is replayed")
            })??;
        self.recheck_product().await?;
        let snapshot = self.observer.snapshot(None).await?;
        self.record(
            &format!("{stage}-human-barrier"),
            &json!({"acknowledged":true,"effectProved":false,"snapshot":snapshot}),
        )?;
        Ok(())
    }
    async fn wait_snapshot(
        &mut self,
        stage: &'static str,
        marker: Option<&str>,
        deadline: Duration,
        predicate: impl Fn(&Value) -> bool,
    ) -> io::Result<Value> {
        self.last_stage = stage;
        let until = Instant::now() + deadline;
        self.recheck_product().await?;
        loop {
            self.recheck()?;
            let snapshot = self.observer.snapshot(marker).await?;
            self.recheck()?;
            if predicate(&snapshot) {
                self.recheck_product().await?;
                if Instant::now() >= until {
                    return Err(failure(
                        "physical effect proof exceeded its original deadline",
                    ));
                }
                self.record(stage, &snapshot)?;
                return Ok(snapshot);
            }
            if Instant::now() >= until {
                return Err(failure(
                    "physical required original-product effect was not observed before its deadline; no replay",
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    async fn action(
        &mut self,
        stage: &'static str,
        action: &str,
        arguments: BTreeMap<String, String>,
    ) -> io::Result<(Uuid, Value)> {
        self.invoke_action(stage, action, arguments, UiExpectation::Completed)
            .await
            .map(|(run, after, _)| (run, after))
    }
    async fn invoke_action(
        &mut self,
        stage: &'static str,
        action: &str,
        mut arguments: BTreeMap<String, String>,
        expectation: UiExpectation,
    ) -> io::Result<(Uuid, Value, bool)> {
        self.last_stage = stage;
        self.recheck_product().await?;
        self.adb.tunnels(Some(self.control_port)).await?;
        let run = Uuid::random();
        extend(
            &mut arguments,
            &[
                ("ownedUiRunId", &run.0),
                ("ownedUiAction", action),
                ("ownedUiHostId", &self.host),
                ("ownedUiMvpRunId", &self.run.0),
                ("ownedUiProductPid", &self.observer.epoch.pid.to_string()),
                (
                    "ownedUiProductStart",
                    &self.observer.epoch.start.to_string(),
                ),
                ("ownedUiObserverPublicSha256", &self.public_sha),
                ("ownedUiExpectedProductSha256", &self.product_sha),
            ],
        );
        let expected_hash = argument_hash(&arguments)?;
        let mut words = vec![
            "am".to_owned(),
            "instrument".to_owned(),
            "-w".to_owned(),
            "-r".to_owned(),
        ];
        for (key, value) in &arguments {
            words.extend(["-e".to_owned(), key.clone(), value.clone()]);
        }
        words.push(UI_RUNNER.to_owned());
        let remote = words
            .iter()
            .map(|word| super::shell_literal(word))
            .collect::<io::Result<Vec<_>>>()?
            .join(" ");
        self.adb.recheck_server().await?;
        let mut command = self.adb.command();
        command
            .args(["shell", &remote])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure("physical owned UI runner status pipe is unavailable"))?;
        let (owner_sender, mut owner_receiver) = watch::channel::<Option<Value>>(None);
        let result = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout.take(2_097_153));
            let mut line = String::new();
            let mut bytes = 0usize;
            let mut owner = None;
            let mut outcome = None;
            loop {
                line.clear();
                let count = reader.read_line(&mut line).await?;
                if count == 0 {
                    break;
                }
                bytes += count;
                if count > 65536 || bytes > 2_097_152 {
                    return Err(failure("physical owned UI runner status exceeds bound"));
                }
                let record = line.trim_end_matches(['\r', '\n']);
                if let Some(json) =
                    record.strip_prefix("INSTRUMENTATION_STATUS: goatrOwnedUiProcess=")
                {
                    if owner.is_some() {
                        return Err(failure(
                            "physical owned UI runner repeated its original process",
                        ));
                    }
                    let value =
                        goatr::native_json::parse(json.as_bytes(), goatr::native_json::REGISTRY)
                            .map_err(|_| failure("physical owned UI process status is invalid"))?;
                    object(
                        &value,
                        &["phase", "runId", "action", "runnerPid", "runnerStart"],
                    )?;
                    if string(&value, "phase")? != "process"
                        || unsigned(&value, "runnerPid")? == 0
                        || unsigned(&value, "runnerStart")? == 0
                    {
                        return Err(failure(
                            "physical owned UI runner process status is unproved",
                        ));
                    }
                    owner_sender
                        .send(Some(value.clone()))
                        .map_err(|_| failure("physical owned UI owner receiver ended"))?;
                    owner = Some(value);
                } else if let Some(json) =
                    record.strip_prefix("INSTRUMENTATION_RESULT: goatrOwnedUiAutomation=")
                {
                    if outcome.is_some() {
                        return Err(failure(
                            "physical owned UI runner repeated its final outcome",
                        ));
                    }
                    outcome = Some(
                        goatr::native_json::parse(json.as_bytes(), goatr::native_json::REGISTRY)
                            .map_err(|_| failure("physical owned UI outcome is invalid"))?,
                    );
                }
            }
            let command_succeeded = child.wait().await?.success();
            Ok((
                owner.ok_or_else(|| failure("physical owned UI process status is absent"))?,
                outcome.ok_or_else(|| failure("physical owned UI final outcome is absent"))?,
                command_succeeded,
            ))
        });
        let process = timeout(Duration::from_secs(25), async {
            loop {
                if let Some(value) = owner_receiver.borrow().clone() {
                    return Ok::<_, io::Error>(value);
                }
                owner_receiver
                    .changed()
                    .await
                    .map_err(|_| failure("physical owned UI process status owner ended"))?;
            }
        })
        .await
        .map_err(|_| failure("physical owned UI process status expired"))??;
        if string(&process, "runId")? != run.0 || string(&process, "action")? != action {
            return Err(failure(
                "physical owned UI process belongs to another action",
            ));
        }
        let intent_path = format!("cache/owned-ui-automation-{}.intent.json", run.0);
        let until = Instant::now() + Duration::from_secs(10);
        let intent = loop {
            if let Some(value) = self.adb.private_record(&intent_path).await? {
                break value;
            }
            if Instant::now() >= until {
                return Err(failure(
                    "physical owned UI intent was not durably published",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        object(
            &intent,
            &[
                "phase",
                "runId",
                "action",
                "creationName",
                "creationServerId",
                "physicalArgumentHash",
                "excludedReceiptCount",
                "runnerPid",
                "runnerStart",
            ],
        )?;
        if string(&intent, "phase")? != "intent"
            || intent["runId"] != process["runId"]
            || intent["action"] != process["action"]
            || intent["runnerPid"] != process["runnerPid"]
            || intent["runnerStart"] != process["runnerStart"]
            || string(&intent, "physicalArgumentHash")? != expected_hash
        {
            return Err(failure(
                "physical owned UI producer intent/complete-argument/process correlation changed",
            ));
        }
        write_json(
            &self
                .evidence
                .join(format!("physical-ui-{}.intent.json", run.0)),
            &intent,
        )?;
        // Complete public epoch/hash admission BEFORE requesting the five-second signed witness.
        self.recheck_product().await?;
        let (epoch, witness) = self
            .observer
            .query(json!({"kind":"epoch","actionRunId":run}), "epoch")
            .await?;
        if string(&epoch, "actionRunId")? != run.0
            || string(&epoch, "apkSha256")? != self.product_sha
        {
            return Err(failure(
                "physical pre-input product epoch is not the original action/package",
            ));
        }
        let bytes = serde_json::to_vec(&witness).map_err(io::Error::other)?;
        if bytes.len() > MAX_CONTROL {
            return Err(failure(
                "physical pre-input product epoch witness exceeds bound",
            ));
        }
        let staging = format!("cache/owned-ui-stage-{}", run.0);
        let release = format!("cache/owned-ui-release-{}", run.0);
        self.adb
            .shell(&["run-as", TEST, "mkdir", "-m", "700", &staging], 1024)
            .await?;
        self.adb
            .write_private(TEST, &format!("{staging}/product-epoch.json"), &bytes)
            .await?;
        self.recheck()?;
        self.adb
            .shell(
                &["run-as", TEST, "mv", "-T", "-n", &staging, &release],
                1024,
            )
            .await?;
        let (original, outcome, command_succeeded) = timeout(Duration::from_secs(30), result)
            .await
            .map_err(|_| {
                failure(
                    "physical owned UI outcome expired; action is uncertain and is never replayed",
                )
            })?
            .map_err(|_| failure("physical owned UI status owner failed; no replay"))??;
        let (projection, ambiguous) = publish_and_admit_ui_outcome(
            &self.evidence,
            expectation,
            UiOutcome {
                run: &run,
                action,
                argument_hash: &expected_hash,
                original: &original,
                process: &process,
                outcome: &outcome,
                api: self.api,
                command_succeeded,
            },
        )?;
        self.recheck_product().await?;
        let after = self.observer.snapshot(None).await?;
        self.record(
            &format!("{stage}-ui-owner"),
            &json!({"ui":projection,"before":epoch["snapshot"],"after":after}),
        )?;
        Ok((run, after, ambiguous))
    }
    async fn new_receipt(
        &mut self,
        stage: &'static str,
        before: &BTreeSet<String>,
        method: &str,
        target: Option<&Target>,
        shell: Option<&ShellTarget>,
    ) -> io::Result<Value> {
        let snapshot = self
            .wait_snapshot(stage, None, Duration::from_secs(125), |snapshot| {
                snapshot["receipts"].as_array().is_some_and(|receipts| {
                    receipts.iter().any(|receipt| {
                        receipt["action"]
                            .as_str()
                            .is_some_and(|action| !before.contains(action))
                            && receipt["method"] == method
                            && target.is_none_or(|target| {
                                receipt["target"] == target.session_id.0
                                    && receipt["generation"] == target.generation.0
                            })
                            && shell.is_none_or(|target| {
                                receipt["shell"] == target.shell_id.0
                                    && receipt["shellGeneration"] == target.generation.0
                            })
                            && receipt["state"].as_str().is_some_and(|state| {
                                matches!(state, "Dispatched" | "Succeeded" | "Failed" | "Uncertain")
                            })
                    })
                })
            })
            .await?;
        let mut matching = array(&snapshot, "receipts", 4096)?
            .iter()
            .filter(|receipt| {
                receipt["action"]
                    .as_str()
                    .is_some_and(|action| !before.contains(action))
                    && receipt["method"] == method
                    && target.is_none_or(|target| {
                        receipt["target"] == target.session_id.0
                            && receipt["generation"] == target.generation.0
                    })
                    && shell.is_none_or(|target| {
                        receipt["shell"] == target.shell_id.0
                            && receipt["shellGeneration"] == target.generation.0
                    })
            });
        let receipt = matching
            .next()
            .ok_or_else(|| failure("physical original action receipt is absent"))?;
        let expected = if method == "ShellInput" {
            "Dispatched"
        } else {
            "Succeeded"
        };
        if matching.next().is_some() || string(receipt, "state")? != expected {
            return Err(failure(
                "physical native action failed, was uncertain, or has ambiguous attribution; no replay",
            ));
        }
        self.record(&format!("{stage}-receipt"), receipt)?;
        Ok(receipt.clone())
    }
    async fn verify_provider(&mut self, original: &NativeProof) -> io::Result<NativeProof> {
        let inventory = self.stock.inventory().await?;
        let current = inventory
            .sessions
            .iter()
            .find(|session| {
                session.session_id == original.target.session_id
                    && session.generation == original.target.generation
            })
            .ok_or_else(|| failure("physical original provider generation disappeared"))?;
        let proof = self.stock.provider_proof(current).await?;
        if !original.same_input_binding(&proof) {
            return Err(failure(
                "physical original native provider/owner/room/relay binding changed; never adopt a replacement",
            ));
        }
        Ok(proof)
    }
}

fn maven_aar(group: &str, module: &str, version: &str) -> io::Result<PathBuf> {
    let home = std::env::var_os("GRADLE_USER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".gradle")))
        .ok_or_else(|| failure("physical retained Gradle cache root is unavailable"))?;
    let root = canonical_directory(
        &home
            .join("caches/modules-2/files-2.1")
            .join(group)
            .join(module)
            .join(version),
    )?;
    let mut matches = Vec::new();
    for directory in fs::read_dir(&root)? {
        let directory = directory?;
        if !directory.file_type()?.is_dir() {
            continue;
        }
        let file = directory.path().join(format!("{module}-{version}.aar"));
        if file.exists() {
            let canonical = canonical_file(&file)?;
            if !canonical.starts_with(&root) {
                return Err(failure(
                    "physical resolved Maven AAR escaped its canonical component root",
                ));
            }
            matches.push(canonical);
        }
    }
    if matches.len() != 1 {
        return Err(failure(
            "physical exact resolved Maven AAR is missing or ambiguous",
        ));
    }
    Ok(matches.remove(0))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PackageRecord {
    package: String,
    uid: u32,
    path: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PackageOwnership {
    package: String,
    uid: u32,
    first_install_time: String,
    data_dir: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct InstalledProductEpoch {
    record: PackageRecord,
    ownership: PackageOwnership,
    version_code: String,
    version_name: String,
    user: BTreeMap<String, String>,
    default_process: String,
    launcher: String,
}
#[derive(Clone, Debug)]
struct ProductObservation {
    epoch: InstalledProductEpoch,
    stopped: bool,
    not_launched: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PreparedDevice {
    serial: String,
    model: String,
    sdk: u32,
    abi: String,
    qemu: String,
}
#[derive(Clone, Debug)]
struct PreparedInstall {
    run: String,
    device: PreparedDevice,
    epoch: InstalledProductEpoch,
    apk_sha256: String,
    signer: String,
    apk_bytes: u64,
    references: Value,
}
fn admit_product_inventory(
    start: &Option<PreparedInstall>,
    packages: &BTreeMap<String, PackageRecord>,
) -> io::Result<()> {
    match start {
        None => fresh_product_admission(packages),
        Some(prepared) => {
            if packages.get(APP) == Some(&prepared.epoch.record) {
                Ok(())
            } else {
                Err(failure(
                    "prepared PRODUCT is absent or its original package/path/UID changed; no fallback install",
                ))
            }
        }
    }
}
fn record_text<'a>(value: &'a Value, key: &str, max: usize) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| text.len() <= max)
        .ok_or_else(|| failure("prepared record string is missing or oversized"))
}
fn record_reference(value: &Value, key: &str) -> io::Result<()> {
    let text = record_text(value, key, 4096)?;
    if !text.starts_with('/') || text.chars().any(char::is_control) {
        return Err(failure("prepared evidence reference is malformed"));
    }
    Ok(()) // Evidence string ONLY: never open, canonicalize or derive authority from this path.
}
fn record_decimal(value: &Value, key: &str) -> io::Result<u64> {
    let text = record_text(value, key, 32)?;
    if text.is_empty()
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(failure("prepared decimal identity is malformed"));
    }
    text.parse()
        .map_err(|_| failure("prepared decimal identity overflows"))
}
fn package_user_state(text: &str) -> io::Result<(BTreeMap<String, String>, bool, bool)> {
    if text.len() > 8192 {
        return Err(failure("product user state is oversized"));
    }
    let text = text
        .strip_prefix("User 0: ")
        .ok_or_else(|| failure("product primary user state is missing"))?;
    let mut fields = BTreeMap::new();
    for token in text.split_whitespace() {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| failure("product user state is malformed"))?;
        if !matches!(
            key,
            "ceDataInode"
                | "installed"
                | "hidden"
                | "suspended"
                | "distractionFlags"
                | "stopped"
                | "notLaunched"
                | "enabled"
                | "instant"
                | "virtual"
        ) || fields.insert(key.to_owned(), value.to_owned()).is_some()
        {
            return Err(failure(
                "product user state has unknown or duplicate fields",
            ));
        }
    }
    for key in [
        "installed",
        "hidden",
        "suspended",
        "stopped",
        "notLaunched",
        "instant",
        "virtual",
    ] {
        if !fields
            .get(key)
            .is_some_and(|value| matches!(value.as_str(), "true" | "false"))
        {
            return Err(failure("product user boolean is unavailable"));
        }
    }
    for key in ["enabled", "distractionFlags"] {
        if !fields.get(key).is_some_and(|value| {
            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            return Err(failure("product user policy field is unavailable"));
        }
    }
    if let Some(inode) = fields.get("ceDataInode")
        && (inode
            .parse::<u64>()
            .ok()
            .filter(|inode| *inode > 0)
            .is_none()
            || inode.starts_with('0'))
    {
        return Err(failure("product CE inode is malformed"));
    }
    if fields["installed"] != "true"
        || fields["hidden"] != "false"
        || fields["suspended"] != "false"
        || fields["instant"] != "false"
        || fields["virtual"] != "false"
    {
        return Err(failure("product primary user installation is not admitted"));
    }
    let stopped = fields.remove("stopped").as_deref() == Some("true");
    let not_launched = fields.remove("notLaunched").as_deref() == Some("true");
    Ok((fields, stopped, not_launched))
}
fn checkpoint_package(
    value: &Value,
    package: &str,
    product: bool,
) -> io::Result<ProductObservation> {
    if product {
        object(
            value,
            &[
                "package",
                "uid",
                "versionCode",
                "versionName",
                "firstInstallTime",
                "dataDir",
                "userState",
                "apkPath",
                "apkSha256",
                "signerByIdenticalIndependentlySdkVerifiedBytes",
                "defaultProcess",
                "registeredLauncher",
                "defaultProcessAbsent",
                "processObservation",
            ],
        )?;
    } else {
        object(
            value,
            &[
                "package",
                "uid",
                "versionCode",
                "versionName",
                "firstInstallTime",
                "dataDir",
                "userState",
                "apkPath",
                "apkSha256",
            ],
        )?;
    }
    if string(value, "package")? != package || !hex64(string(value, "apkSha256")?) {
        return Err(failure("prepared package identity is invalid"));
    }
    let uid = u32::try_from(record_decimal(value, "uid")?)
        .ok()
        .filter(|uid| *uid >= 10000)
        .ok_or_else(|| failure("prepared package UID is invalid"))?;
    let path = record_text(value, "apkPath", 4096)?;
    if !valid_apk_path(path) {
        return Err(failure("prepared APK path is malformed"));
    }
    let data = record_text(value, "dataDir", 4096)?;
    if !data.starts_with("/data/")
        || !data.ends_with(&format!("/{package}"))
        || data.chars().any(char::is_whitespace)
        || data
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(failure("prepared package data identity is malformed"));
    }
    record_decimal(value, "versionCode")?;
    let first = record_text(value, "firstInstallTime", 128)?;
    if first.is_empty() || first == "null" || first.chars().any(char::is_control) {
        return Err(failure("prepared first-install identity is missing"));
    }
    let (user, stopped, not_launched) = package_user_state(record_text(value, "userState", 8192)?)?;
    let default_process = if product {
        string(value, "defaultProcess")?.to_owned()
    } else {
        package.to_owned()
    };
    let launcher = if product {
        string(value, "registeredLauncher")?.to_owned()
    } else {
        String::new()
    };
    if product && (default_process != APP || !valid_launcher(&launcher)) {
        return Err(failure("prepared default process/launcher is invalid"));
    }
    Ok(ProductObservation {
        epoch: InstalledProductEpoch {
            record: PackageRecord {
                package: package.to_owned(),
                uid,
                path: Some(path.to_owned()),
            },
            ownership: PackageOwnership {
                package: package.to_owned(),
                uid,
                first_install_time: first.to_owned(),
                data_dir: data.to_owned(),
            },
            version_code: string(value, "versionCode")?.to_owned(),
            version_name: string(value, "versionName")?.to_owned(),
            user,
            default_process,
            launcher,
        },
        stopped,
        not_launched,
    })
}
fn record_command(value: &Value, exit: i64) -> io::Result<&str> {
    object(value, &["exit", "signal", "stdout", "stderr"])?;
    if value["exit"].as_i64() != Some(exit)
        || !value["signal"].is_null()
        || !record_text(value, "stderr", 65536)?.is_empty()
    {
        return Err(failure(
            "prepared command receipt is not successful and unambiguous",
        ));
    }
    record_text(value, "stdout", 65536)
}
fn validate_current_usb_device(output: &str, selected: &str) -> io::Result<()> {
    if output.len() > 65536 {
        return Err(failure("physical device inventory exceeds its bound"));
    }
    let mut lines = output.lines();
    if lines.next().map(str::trim) != Some("List of devices attached") {
        return Err(failure("physical device inventory header is invalid"));
    }
    let mut serials = BTreeSet::new();
    let mut authorized = None;
    for line in lines.filter(|line| !line.trim().is_empty()) {
        let mut words = line.split_whitespace();
        let serial = words
            .next()
            .ok_or_else(|| failure("physical device row has no serial"))?;
        let state = words
            .next()
            .ok_or_else(|| failure("physical device row has no state"))?;
        if serial.len() > 128
            || !serial
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
            || !serials.insert(serial)
        {
            return Err(failure(
                "physical device inventory has an invalid or repeated serial",
            ));
        }
        match state {
            "device" => {
                if authorized.replace((serial, line)).is_some() {
                    return Err(failure(
                        "physical mode requires globally one authorized device",
                    ));
                }
            }
            "offline" | "unauthorized" => {}
            _ => return Err(failure("physical device inventory has an unknown state")),
        }
    }
    let (serial, line) =
        authorized.ok_or_else(|| failure("physical authorized USB device is absent"))?;
    if serial != selected || serial.starts_with("emulator-") {
        return Err(failure(
            "physical sole authorized device differs from the selected physical serial",
        ));
    }
    validate_historical_usb(line, selected)
}
fn validate_historical_usb(line: &str, serial: &str) -> io::Result<()> {
    let words = line.split_whitespace().collect::<Vec<_>>();
    if words.len() < 3
        || words[0] != serial
        || words[1] != "device"
        || !words[2..]
            .iter()
            .any(|word| word.starts_with("usb:") && word.len() > 4)
    {
        return Err(failure("prepared historical USB witness is invalid"));
    }
    let mut keys = BTreeSet::new();
    for word in &words[2..] {
        let (key, value) = word
            .split_once(':')
            .ok_or_else(|| failure("prepared USB token is malformed"))?;
        if !matches!(key, "usb" | "product" | "model" | "device" | "transport_id")
            || value.is_empty()
            || value.len() > 128
            || value.chars().any(char::is_control)
            || !keys.insert(key)
            || key == "transport_id" && !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(failure(
                "prepared USB witness has malformed, unknown or duplicate tokens",
            ));
        }
    }
    Ok(()) // transport_id, display padding and bus location are historical, not installation identity.
}
fn parse_prepared_install(value: &Value) -> io::Result<PreparedInstall> {
    object(
        value,
        &[
            "version",
            "kind",
            "runId",
            "createdAt",
            "package",
            "device",
            "source",
            "artifact",
            "preinstall",
            "install",
            "installed",
            "preservedPackages",
            "noReplace",
            "noDowngrade",
            "noGrant",
            "noUninstall",
            "noClear",
            "noLaunch",
            "testHelperUpdated",
            "suppressedApplicationBaselineExecuted",
            "privateAppDataRead",
            "providerOrHerdrLaunch",
            "fullMvpAcceptanceClaim",
        ],
    )?;
    if value["version"] != 1
        || string(value, "kind")? != "owned-prepared-fresh-product-install"
        || string(value, "package")? != APP
    {
        return Err(failure(
            "prepared creator record version/kind/package is unsupported",
        ));
    }
    let run = string(value, "runId")?.to_owned();
    uuid_text(&run)?;
    if string(value, "createdAt")?.is_empty() {
        return Err(failure("prepared creation time is missing"));
    }
    for key in [
        "noReplace",
        "noDowngrade",
        "noGrant",
        "noUninstall",
        "noClear",
        "noLaunch",
    ] {
        if !boolean(value, key)? {
            return Err(failure("prepared install authorized a forbidden effect"));
        }
    }
    for key in [
        "testHelperUpdated",
        "suppressedApplicationBaselineExecuted",
        "privateAppDataRead",
        "providerOrHerdrLaunch",
        "fullMvpAcceptanceClaim",
    ] {
        if boolean(value, key)? {
            return Err(failure(
                "prepared record already consumed a lifecycle/effect boundary",
            ));
        }
    }
    let device = &value["device"];
    object(
        device,
        &["serial", "usbTransport", "model", "sdk", "abi", "qemu"],
    )?;
    let serial = string(device, "serial")?.to_owned();
    id(&serial)?;
    if serial.starts_with("emulator-") {
        return Err(failure("prepared device is an emulator"));
    }
    validate_historical_usb(record_text(device, "usbTransport", 4096)?, &serial)?;
    let sdk = u32::try_from(record_decimal(device, "sdk")?)
        .map_err(|_| failure("prepared SDK is invalid"))?;
    let model = string(device, "model")?.to_owned();
    let abi = string(device, "abi")?.to_owned();
    let qemu = string(device, "qemu")?.to_owned();
    if model.is_empty()
        || ![30, 34].contains(&sdk)
        || !matches!(abi.as_str(), "arm64-v8a" | "x86_64")
        || !matches!(qemu.as_str(), "" | "0")
    {
        return Err(failure("prepared physical device properties are invalid"));
    }
    let source = &value["source"];
    object(
        source,
        &[
            "apkBuildSource",
            "currentAndroidInputsMatched",
            "qualification",
        ],
    )?;
    record_reference(source, "apkBuildSource")?;
    record_reference(source, "qualification")?;
    if !(1..=4096).contains(&unsigned(source, "currentAndroidInputsMatched")?) {
        return Err(failure("prepared Android source closure is unproved"));
    }
    let artifact = &value["artifact"];
    object(
        artifact,
        &[
            "path",
            "sha256",
            "bytes",
            "uid",
            "mode",
            "nlink",
            "device",
            "inode",
            "mtimeNs",
            "ctimeNs",
            "signerSha256",
        ],
    )?;
    record_reference(artifact, "path")?;
    if unsigned(artifact, "uid")? != 1000
        || string(artifact, "mode")? != "0400"
        || unsigned(artifact, "nlink")? != 1
        || !hex64(string(artifact, "sha256")?)
        || !hex64(string(artifact, "signerSha256")?)
        || unsigned(artifact, "bytes")? == 0
    {
        return Err(failure(
            "prepared original APK held-object evidence is invalid",
        ));
    }
    for key in ["device", "inode", "mtimeNs", "ctimeNs"] {
        if record_decimal(artifact, key)? == 0 {
            return Err(failure("prepared original APK stamp is missing"));
        }
    }
    let pre = &value["preinstall"];
    object(
        pre,
        &[
            "intent",
            "intentSha256",
            "active",
            "includingUninstalled",
            "exactProductAbsent",
        ],
    )?;
    record_reference(pre, "intent")?;
    if !hex64(string(pre, "intentSha256")?) || !boolean(pre, "exactProductAbsent")? {
        return Err(failure("prepared pre-install intent is unproved"));
    }
    let before = coherent_packages(
        record_command(&pre["active"], 0)?,
        record_command(&pre["includingUninstalled"], 0)?,
    )?;
    fresh_product_admission(&before)?;
    let install = &value["install"];
    object(
        install,
        &[
            "binary",
            "argv",
            "receipt",
            "receiptSha256",
            "result",
            "attempts",
        ],
    )?;
    record_reference(install, "binary")?;
    record_reference(install, "receipt")?;
    if !hex64(string(install, "receiptSha256")?) || unsigned(install, "attempts")? != 1 {
        return Err(failure("prepared install was not one owned attempt"));
    }
    let argv = array(install, "argv", 5)?;
    let expected = [
        "-s",
        serial.as_str(),
        "install",
        "-t",
        record_text(artifact, "path", 4096)?,
    ];
    if argv.len() != expected.len()
        || argv
            .iter()
            .zip(expected)
            .any(|(actual, expected)| actual.as_str() != Some(expected))
        || record_command(&install["result"], 0)?
            .lines()
            .filter(|line| line.trim() == "Success")
            .count()
            != 1
    {
        return Err(failure(
            "prepared receipt is not a sole fresh install-t success",
        ));
    }
    let installed = &value["installed"];
    let observed = checkpoint_package(installed, APP, true)?;
    if !observed.stopped
        || !observed.not_launched
        || !boolean(installed, "defaultProcessAbsent")?
        || !record_command(&installed["processObservation"], 1)?.is_empty()
        || string(installed, "apkSha256")? != string(artifact, "sha256")?
        || string(installed, "signerByIdenticalIndependentlySdkVerifiedBytes")?
            != string(artifact, "signerSha256")?
    {
        return Err(failure(
            "prepared PRODUCT is not its unopened independently verified fresh image",
        ));
    }
    let preserved = &value["preservedPackages"];
    object(
        preserved,
        &[
            "before",
            "after",
            "publicIdentityAndBytesUnchanged",
            "privateBytePreservationClaim",
        ],
    )?;
    if !boolean(preserved, "publicIdentityAndBytesUnchanged")?
        || boolean(preserved, "privateBytePreservationClaim")?
        || preserved["before"] != preserved["after"]
    {
        return Err(failure(
            "prepared install did not preserve other public packages",
        ));
    }
    object(&preserved["before"], &[TEST, VERIFY, "io.heckel.ntfy"])?;
    object(&preserved["after"], &[TEST, VERIFY, "io.heckel.ntfy"])?;
    for package in [TEST, VERIFY, "io.heckel.ntfy"] {
        let prior = checkpoint_package(&preserved["before"][package], package, false)?;
        if package != "io.heckel.ntfy" && before.get(package) != Some(&prior.epoch.record) {
            return Err(failure(
                "prepared absence/public-preservation evidence disagrees",
            ));
        }
    }
    Ok(PreparedInstall {
        run,
        device: PreparedDevice {
            serial,
            model,
            sdk,
            abi,
            qemu,
        },
        epoch: observed.epoch,
        apk_sha256: string(artifact, "sha256")?.to_owned(),
        signer: string(artifact, "signerSha256")?.to_owned(),
        apk_bytes: unsigned(artifact, "bytes")?,
        references: json!({"source":source,"intent":pre["intent"],"receipt":install["receipt"],"originalApkPath":artifact["path"]}),
    })
}
fn parse_pinned_prepared_bytes(bytes: &[u8], expected: &str) -> io::Result<PreparedInstall> {
    use sha2::{Digest, Sha256};
    if !hex64(expected) || bytes.len() > 65536 || hex(&Sha256::digest(bytes)) != expected {
        return Err(failure(
            "prepared record differs from independent caller hash",
        ));
    }
    parse_prepared_install(&goatr::native_json::parse(
        bytes,
        goatr::native_json::REGISTRY,
    )?)
}
fn read_prepared_install(
    path: &Path,
    expected: &str,
) -> io::Result<(PreparedInstall, (PathBuf, File, fs::Metadata))> {
    if !hex64(expected) {
        return Err(failure(
            "prepared install requires its independent lowercase SHA256 pin",
        ));
    }
    goatr::config::absolute(path)?;
    if fs::canonicalize(path)? != path {
        return Err(failure("prepared record must be canonical without aliases"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| failure("prepared record parent missing"))?;
    private_dir(parent, false)?;
    let parent_before = fs::symlink_metadata(parent)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let original = file.metadata()?;
    if parent_before.uid() != 1000
        || !original.is_file()
        || original.uid() != 1000
        || original.nlink() != 1
        || original.mode() & 0o7777 != 0o400
        || original.len() > 65536
    {
        return Err(failure(
            "prepared record is not sealed owned0400/nlink1/bounded",
        ));
    }
    let mut bytes = Vec::with_capacity(original.len() as usize);
    (&mut file).take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536
        || complete_stamp(&original) != complete_stamp(&file.metadata()?)
        || complete_stamp(&original) != complete_stamp(&fs::symlink_metadata(path)?)
        || complete_stamp(&parent_before) != complete_stamp(&fs::symlink_metadata(parent)?)
        || fs::canonicalize(path)? != path
    {
        return Err(failure(
            "prepared record changed during its original held read",
        ));
    }
    let prepared = parse_pinned_prepared_bytes(&bytes, expected)?;
    let pin = (path.to_owned(), file, original);
    recheck_inputs(std::slice::from_ref(&pin))?;
    Ok((prepared, pin))
}
fn valid_apk_path(path: &str) -> bool {
    path.starts_with("/data/app/")
        && path.ends_with("/base.apk")
        && path.len() <= 4096
        && !path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && !path
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
}
fn package_records(output: &str) -> io::Result<BTreeMap<String, PackageRecord>> {
    if output.len() > 65536 {
        return Err(failure("physical package inventory exceeds its bound"));
    }
    let mut records = BTreeMap::new();
    let mut uids = BTreeSet::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let body = line
            .strip_prefix("package:")
            .ok_or_else(|| failure("physical package inventory has an unknown record"))?;
        let (subject, uid_text) = body
            .rsplit_once(" uid:")
            .ok_or_else(|| failure("physical package inventory has no exact UID"))?;
        if uid_text.is_empty()
            || uid_text.starts_with('0')
            || !uid_text.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(failure("physical package UID is malformed"));
        }
        let uid = uid_text
            .parse::<u32>()
            .ok()
            .filter(|uid| *uid >= 10000)
            .ok_or_else(|| failure("physical package UID is malformed"))?;
        let (path, package) = match subject.rsplit_once('=') {
            Some((path, package)) => {
                if !valid_apk_path(path) {
                    return Err(failure("physical package path is malformed or split"));
                }
                (Some(path.to_owned()), package)
            }
            None => (None, subject),
        };
        if !matches!(package, APP | TEST | VERIFY) || !uids.insert(uid) {
            return Err(failure(
                "physical package inventory has an unexpected subject or shared UID",
            ));
        }
        let record = PackageRecord {
            package: package.to_owned(),
            uid,
            path,
        };
        if records.insert(package.to_owned(), record).is_some() {
            return Err(failure("physical package inventory repeats a subject"));
        }
    }
    Ok(records)
}
fn coherent_packages(active: &str, retained: &str) -> io::Result<BTreeMap<String, PackageRecord>> {
    let active = package_records(active)?;
    let retained = package_records(retained)?;
    if active != retained || active.values().any(|record| record.path.is_none()) {
        return Err(failure(
            "physical active and retained package/path/UID evidence is ambiguous",
        ));
    }
    Ok(active)
}
fn fresh_product_admission(packages: &BTreeMap<String, PackageRecord>) -> io::Result<()> {
    if packages.contains_key(APP) {
        return Err(prerequisite(
            "physical PRODUCT is present or retained; fresh-only install refuses replacement",
        ));
    }
    Ok(())
}
async fn package_inventory(adb: &PhysicalAdb) -> io::Result<BTreeMap<String, PackageRecord>> {
    let active = adb
        .shell(
            &["pm", "list", "packages", "-f", "-U", "--user", "0", APP],
            65536,
        )
        .await?;
    let retained = adb
        .shell(
            &[
                "pm", "list", "packages", "-u", "-f", "-U", "--user", "0", APP,
            ],
            65536,
        )
        .await?;
    coherent_packages(text(&active)?, text(&retained)?)
}
fn package_detail_fields<'a>(
    output: &'a str,
    record: &PackageRecord,
) -> io::Result<BTreeMap<&'static str, &'a str>> {
    if output.len() > 262144 {
        return Err(failure("physical package ownership exceeds its bound"));
    }
    let header = format!("Package [{}] (", record.package);
    let lines = output.lines().collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim_start().starts_with(&header) && line.ends_with("):"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let start = match starts.as_slice() {
        [start] => *start,
        _ => {
            return Err(failure(
                "physical package ownership subject is missing or ambiguous",
            ));
        }
    };
    let indent = lines[start].len() - lines[start].trim_start().len();
    let mut fields = BTreeMap::new();
    for line in lines
        .iter()
        .skip(start + 1)
        .take_while(|line| line.trim().is_empty() || line.len() - line.trim_start().len() > indent)
    {
        let line = line.trim();
        if line.starts_with("User 0: ") && fields.insert("userState", line).is_some() {
            return Err(failure("product user state is repeated"));
        }
        for key in [
            "userId",
            "appId",
            "firstInstallTime",
            "dataDir",
            "versionCode",
            "versionName",
        ] {
            if let Some(value) = line
                .strip_prefix(key)
                .and_then(|value| value.strip_prefix('='))
            {
                let value = if key == "versionCode" {
                    value.split_whitespace().next().unwrap_or("")
                } else {
                    value
                };
                if value.is_empty()
                    || value.len() > 4096
                    || value.chars().any(char::is_control)
                    || fields.insert(key, value).is_some()
                {
                    return Err(failure(
                        "physical package ownership field is malformed or repeated",
                    ));
                }
            }
        }
    }
    Ok(fields)
}
fn ownership_from_fields(
    record: &PackageRecord,
    fields: &BTreeMap<&str, &str>,
) -> io::Result<PackageOwnership> {
    let uid = match (fields.get("userId"), fields.get("appId")) {
        (Some(uid), None) | (None, Some(uid)) => uid.parse::<u32>().ok(),
        _ => None,
    };
    let first = fields
        .get("firstInstallTime")
        .filter(|value| value.len() <= 128)
        .ok_or_else(|| failure("physical package first-install identity is unavailable"))?;
    let data = fields
        .get("dataDir")
        .ok_or_else(|| failure("physical package data identity is unavailable"))?;
    if uid != Some(record.uid)
        || !data.starts_with("/data/")
        || !data.ends_with(&format!("/{}", record.package))
        || data.chars().any(char::is_whitespace)
        || data
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(failure(
            "physical package ownership does not match the original listed UID/subject",
        ));
    }
    Ok(PackageOwnership {
        package: record.package.clone(),
        uid: record.uid,
        first_install_time: (*first).to_owned(),
        data_dir: (*data).to_owned(),
    })
}
fn package_ownership_facts(output: &str, record: &PackageRecord) -> io::Result<PackageOwnership> {
    ownership_from_fields(record, &package_detail_fields(output, record)?)
}
fn product_observation_fields(
    output: &str,
    record: &PackageRecord,
    launcher: String,
) -> io::Result<ProductObservation> {
    let fields = package_detail_fields(output, record)?;
    let ownership = ownership_from_fields(record, &fields)?;
    let version_code = (*fields
        .get("versionCode")
        .ok_or_else(|| failure("product version code unavailable"))?)
    .to_owned();
    if version_code.is_empty()
        || !version_code.bytes().all(|byte| byte.is_ascii_digit())
        || (version_code.len() > 1 && version_code.starts_with('0'))
    {
        return Err(failure("product version code malformed"));
    }
    let version_name = (*fields
        .get("versionName")
        .ok_or_else(|| failure("product version name unavailable"))?)
    .to_owned();
    let (user, stopped, not_launched) = package_user_state(
        fields
            .get("userState")
            .ok_or_else(|| failure("product user state unavailable"))?,
    )?;
    if !valid_launcher(&launcher) {
        return Err(failure("product launcher malformed"));
    }
    // The independently qualified CURRENT installed APK manifest proves its default process is APP.
    Ok(ProductObservation {
        epoch: InstalledProductEpoch {
            record: record.clone(),
            ownership,
            version_code,
            version_name,
            user,
            default_process: APP.to_owned(),
            launcher,
        },
        stopped,
        not_launched,
    })
}
async fn product_observation(
    adb: &PhysicalAdb,
    record: &PackageRecord,
) -> io::Result<ProductObservation> {
    let dump = adb.shell(&["dumpsys", "package", APP], 262144).await?;
    product_observation_fields(text(&dump)?, record, product_launcher(adb).await?)
}
fn stable_product_epoch_admission(
    expected: &InstalledProductEpoch,
    current: &ProductObservation,
) -> io::Result<()> {
    if &current.epoch != expected {
        return Err(failure("original PRODUCT stable public epoch changed"));
    }
    Ok(()) // stopped/notLaunched are deliberately consumed after the owned baseline starts.
}
fn prepared_observation_admission(
    prepared: &PreparedInstall,
    current: &ProductObservation,
    pristine: bool,
) -> io::Result<()> {
    stable_product_epoch_admission(&prepared.epoch, current)?;
    if pristine && (!current.stopped || !current.not_launched) {
        return Err(failure("prepared PRODUCT unconsumed lifecycle changed"));
    }
    Ok(())
}
fn recheck_product_inputs(
    inputs: &[(PathBuf, File, fs::Metadata)],
    failed: Option<&mut bool>,
) -> io::Result<()> {
    match failed {
        Some(failed) => recheck_retained_inputs(inputs, failed),
        None => recheck_inputs(inputs),
    }
}
async fn require_product_epoch(
    adb: &PhysicalAdb,
    expected: &InstalledProductEpoch,
    sha256: &str,
    inputs: &[(PathBuf, File, fs::Metadata)],
    mut failed: Option<&mut bool>,
) -> io::Result<()> {
    recheck_product_inputs(inputs, failed.as_deref_mut())?;
    let packages = package_inventory(adb).await?;
    recheck_product_inputs(inputs, failed.as_deref_mut())?;
    let record = packages
        .get(APP)
        .ok_or_else(|| failure("original PRODUCT disappeared"))?;
    if record != &expected.record {
        return Err(failure("original PRODUCT package/path/UID changed"));
    }
    let current = product_observation(adb, record).await?;
    recheck_product_inputs(inputs, failed.as_deref_mut())?;
    stable_product_epoch_admission(expected, &current)?;
    if installed_hash(adb, APP).await? != sha256 {
        return Err(failure("original PRODUCT qualified bytes changed"));
    }
    recheck_product_inputs(inputs, failed)
}
fn prepared_artifact_admission(
    prepared: &PreparedInstall,
    device: &PreparedDevice,
    artifact: &Artifact,
    bytes: u64,
) -> io::Result<()> {
    if &prepared.device != device
        || artifact.package != APP
        || artifact.sha256 != prepared.apk_sha256
        || artifact.signer != prepared.signer
        || bytes != prepared.apk_bytes
    {
        return Err(failure(
            "prepared creator device/source-APK pins differ from current independently qualified inputs",
        ));
    }
    Ok(())
}
async fn admit_product_start(
    start: &Option<PreparedInstall>,
    adb: &PhysicalAdb,
    packages: &BTreeMap<String, PackageRecord>,
    inputs: &[(PathBuf, File, fs::Metadata)],
) -> io::Result<()> {
    admit_product_inventory(start, packages)?;
    if let Some(prepared) = start {
        recheck_inputs(inputs)?;
        let current = product_observation(
            adb,
            packages
                .get(APP)
                .ok_or_else(|| failure("prepared PRODUCT absent"))?,
        )
        .await?;
        recheck_inputs(inputs)?;
        prepared_observation_admission(prepared, &current, true)?;
        if installed_hash(adb, APP).await? != prepared.apk_sha256 {
            return Err(failure("prepared current PRODUCT bytes changed"));
        }
        recheck_inputs(inputs)?;
        adb.default_product_process_absent().await?;
        recheck_inputs(inputs)?;
    }
    Ok(())
}
async fn package_ownership(
    adb: &PhysicalAdb,
    record: &PackageRecord,
) -> io::Result<PackageOwnership> {
    let output = adb
        .shell(&["dumpsys", "package", &record.package], 262144)
        .await?;
    package_ownership_facts(text(&output)?, record)
}
fn test_update_admission(previous: &Artifact, candidate: &Artifact) -> io::Result<()> {
    if previous.package != TEST || candidate.package != TEST || previous.signer != candidate.signer
    {
        return Err(failure(
            "physical helper update requires the original same-signer TEST; never PRODUCT or VERIFY",
        ));
    }
    Ok(())
}
async fn verify_unchanged(
    adb: &PhysicalAdb,
    record: Option<&PackageRecord>,
    ownership: Option<&PackageOwnership>,
    artifact: Option<&Artifact>,
) -> io::Result<()> {
    let current = package_inventory(adb).await?;
    match (record, current.get(VERIFY), ownership, artifact) {
        (None, None, None, None) => Ok(()),
        (Some(original), Some(current), Some(ownership), Some(artifact)) if original == current => {
            if package_ownership(adb, current).await? != *ownership
                || installed_hash(adb, VERIFY).await? != artifact.sha256
            {
                return Err(failure("physical untouched VERIFY installation changed"));
            }
            Ok(())
        }
        _ => Err(failure(
            "physical VERIFY absence or original installation was not preserved",
        )),
    }
}
async fn installed_artifact(
    adb: &PhysicalAdb,
    sdk: &Path,
    package: &str,
    evidence: &Path,
    leaf: &str,
) -> io::Result<Artifact> {
    let paths = adb.shell(&["pm", "path", package], 16_384).await?;
    let lines: Vec<_> = text(&paths)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let path = match lines.as_slice() {
        [line] => line
            .strip_prefix("package:")
            .filter(|path| {
                path.starts_with("/data/app/")
                    && path.ends_with("/base.apk")
                    && path.len() <= 4096
                    && !path.chars().any(char::is_control)
            })
            .ok_or_else(|| {
                failure("physical installed package has no qualified single base APK")
            })?,
        _ => {
            return Err(failure(
                "physical installed package is absent or split; no package is installed as a substitute",
            ));
        }
    };
    let digest = adb.shell(&["sha256sum", path], 8192).await?;
    let actual = text(&digest)?
        .split_whitespace()
        .next()
        .filter(|hash| hex64(hash))
        .ok_or_else(|| failure("physical installed APK has no bounded SHA-256 witness"))?
        .to_owned();
    let destination = evidence.join(leaf);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(failure(
            "physical installed APK evidence destination is not fresh",
        ));
    }
    adb.invoke(
        &[
            "pull",
            path,
            destination
                .to_str()
                .ok_or_else(|| failure("physical APK evidence path is not UTF-8"))?,
        ],
        65_536,
    )
    .await?;
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o600))?;
    let artifact = qualify_artifact(&destination, &actual, sdk, package, false).await?;
    let after = adb.shell(&["sha256sum", path], 8192).await?;
    if text(&after)?.split_whitespace().next() != Some(actual.as_str()) {
        return Err(failure(
            "physical installed APK changed while its exact artifact was captured",
        ));
    }
    Ok(artifact)
}
async fn installed_hash(adb: &PhysicalAdb, package: &str) -> io::Result<String> {
    let paths = adb.shell(&["pm", "path", package], 16_384).await?;
    let lines: Vec<_> = text(&paths)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let path = match lines.as_slice() {
        [line] => line
            .strip_prefix("package:")
            .filter(|path| {
                path.starts_with("/data/app/")
                    && path.ends_with("/base.apk")
                    && path.len() <= 4096
                    && !path.chars().any(char::is_control)
            })
            .ok_or_else(|| failure("physical installed package path is no longer singular"))?,
        _ => {
            return Err(failure(
                "physical installed package disappeared or changed to split APKs",
            ));
        }
    };
    let bytes = adb.shell(&["sha256sum", path], 8192).await?;
    text(&bytes)?
        .split_whitespace()
        .next()
        .filter(|hash| hex64(hash))
        .map(str::to_owned)
        .ok_or_else(|| failure("physical installed package hash is unavailable"))
}
async fn install_test_update(
    adb: &PhysicalAdb,
    previous: &Artifact,
    ownership: &PackageOwnership,
    artifact: &Artifact,
    start: &Option<PreparedInstall>,
    inputs: &[(PathBuf, File, fs::Metadata)],
) -> io::Result<()> {
    test_update_admission(previous, artifact)?;
    recheck_inputs(inputs)?;
    let packages = package_inventory(adb).await?;
    recheck_inputs(inputs)?;
    admit_product_start(start, adb, &packages, inputs).await?;
    let record = packages.get(TEST).ok_or_else(|| {
        failure("physical original TEST disappeared; do not create a replacement helper")
    })?;
    if package_ownership(adb, record).await? != *ownership
        || installed_hash(adb, TEST).await? != previous.sha256
    {
        return Err(failure(
            "physical original helper installation changed before update",
        ));
    }
    recheck_inputs(inputs)?;
    let path = artifact
        .path
        .to_str()
        .ok_or_else(|| failure("physical APK path is not UTF-8"))?;
    recheck_inputs(inputs)?;
    let result = adb.invoke(&["install", "-r", "-t", path], 65_536).await?;
    recheck_inputs(inputs)?;
    let installed = installed_hash(adb, &artifact.package).await?;
    recheck_inputs(inputs)?;
    if !text(&result)?.lines().any(|line| line.trim() == "Success") || installed != artifact.sha256
    {
        return Err(failure(
            "physical same-signer APK update did not install the frozen artifact; no downgrade, uninstall, clearing, or retry",
        ));
    }
    Ok(())
}
async fn install_fresh_product(
    adb: &PhysicalAdb,
    artifact: &Artifact,
    inputs: &[(PathBuf, File, fs::Metadata)],
) -> io::Result<()> {
    if artifact.package != APP {
        return Err(failure("physical fresh install subject is not PRODUCT"));
    }
    recheck_inputs(inputs)?;
    let packages = package_inventory(adb).await?;
    recheck_inputs(inputs)?;
    fresh_product_admission(&packages)?;
    let path = artifact
        .path
        .to_str()
        .ok_or_else(|| failure("physical PRODUCT APK path is not UTF-8"))?;
    let result = adb.invoke(&["install", "-t", path], 65536).await?;
    recheck_inputs(inputs)?;
    let installed = installed_hash(adb, APP).await?;
    recheck_inputs(inputs)?;
    if !text(&result)?.lines().any(|line| line.trim() == "Success") || installed != artifact.sha256
    {
        return Err(failure(
            "physical fresh PRODUCT install failed; no replace, downgrade, clear or retry",
        ));
    }
    Ok(())
}
struct ObserverArtifacts<'a> {
    product: &'a Artifact,
    product_uid: u32,
    test: &'a Artifact,
    verify: Option<&'a Artifact>,
}
fn observer_arguments(
    mode: &str,
    run: &Uuid,
    label: &str,
    host: Option<&str>,
    artifacts: &ObserverArtifacts<'_>,
    dependency: &str,
    port: u16,
) -> BTreeMap<String, String> {
    let ObserverArtifacts {
        product,
        product_uid,
        test,
        verify,
    } = artifacts;
    let mut arguments = BTreeMap::from([
        ("physicalMvpMode".to_owned(), mode.to_owned()),
        ("physicalMvpFreshInstall".to_owned(), "true".to_owned()),
        ("physicalMvpRunId".to_owned(), run.0.clone()),
        ("physicalMvpHostLabel".to_owned(), label.to_owned()),
        (
            "physicalMvpExpectedSignerSha256".to_owned(),
            product.signer.clone(),
        ),
        (
            "physicalMvpExpectedProductSha256".to_owned(),
            product.sha256.clone(),
        ),
        (
            "physicalMvpExpectedProductUid".to_owned(),
            product_uid.to_string(),
        ),
        (
            "physicalMvpExpectedTestSha256".to_owned(),
            test.sha256.clone(),
        ),
        (
            "physicalMvpDependencyProof".to_owned(),
            dependency.to_owned(),
        ),
        ("stockControlPort".to_owned(), port.to_string()),
    ]);
    if let Some(verify) = verify {
        arguments.insert(
            "physicalMvpExpectedVerifySha256".to_owned(),
            verify.sha256.clone(),
        );
        arguments.insert(
            "physicalMvpExpectedVerifySignerSha256".to_owned(),
            verify.signer.clone(),
        );
    }
    if let Some(host) = host {
        arguments.insert("physicalMvpHost".to_owned(), host.to_owned());
    }
    arguments
}
async fn fresh_baseline(
    adb: &PhysicalAdb,
    listener: &TcpListener,
    arguments: &BTreeMap<String, String>,
    run: &Uuid,
    product_uid: u32,
    dependency: &str,
    evidence: &Path,
) -> io::Result<RetainedFacts> {
    let mut instrumentation = ProductInstrumentation::start(adb, arguments, "baseline").await?;
    let epoch = instrumentation.original().await?;
    if epoch.uid != product_uid {
        return Err(failure(
            "physical fresh baseline UID is not the installed PRODUCT",
        ));
    }
    let (mut stream, peer) = timeout(Duration::from_secs(35), listener.accept())
        .await
        .map_err(|_| failure("physical fresh-product raw baseline expired"))??;
    if !peer.ip().is_loopback() {
        return Err(failure("physical baseline control peer is not loopback"));
    }
    let (report, _) = authenticated_report(&mut stream, &epoch, run, 0).await?;
    object(&report, &["kind", "process", "facts", "dependencyProof"])?;
    epoch.verify(&report["process"])?;
    if string(&report, "kind")? != "baseline" || string(&report, "dependencyProof")? != dependency {
        return Err(failure(
            "physical fresh-product raw baseline has the wrong admission/source proof",
        ));
    }
    let baseline = RetainedFacts::decode(&report["facts"])?;
    baseline.fresh_empty()?;
    write_json(
        &evidence.join("physical-fresh-empty-baseline.json"),
        &json!({"runId":run,"freshProductTemporaryProcess":report["process"],
        "facts":baseline,"rawReadOnlyDatabase":true,"rawPreferencesReadOnly":true,"applicationOnCreateInvoked":false,"goatrRuntimeInitialized":false,
        "sessionStoreReadMethodsInvoked":false,"dependencyProof":dependency,"profileAndApplicationFileMarkersExcluded":true}),
    )?;
    control_reply(
        &mut stream,
        &json!({"kind":"finished","runId":run,"sequence":0}),
    )
    .await?;
    drop(stream);
    let finished = instrumentation.finished().await?;
    if finished.pid != epoch.pid || finished.start != epoch.start {
        return Err(failure(
            "physical fresh-product baseline finished another process",
        ));
    }
    // Finishing this temporary instrumentation is an explicit lifetime boundary, not continuity.
    wait_original_process_absent(adb, &epoch).await?;
    write_json(
        &evidence.join("physical-fresh-baseline-finished.json"),
        &json!({"runId":run,"pid":epoch.pid,"start":epoch.start,"finished":true,"processAbsent":true,"continuityClaim":false}),
    )?;
    Ok(baseline)
}
fn valid_launcher(component: &str) -> bool {
    component.split_once('/').is_some_and(|(package, class)| {
        package == APP
            && !class.is_empty()
            && !class.contains('/')
            && class
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._$".contains(&byte))
    })
}
async fn product_launcher(adb: &PhysicalAdb) -> io::Result<String> {
    let bytes = adb
        .shell(
            &[
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.LAUNCHER",
                "-p",
                APP,
            ],
            16_384,
        )
        .await?;
    let components: Vec<_> = text(&bytes)?
        .lines()
        .filter(|line| line.starts_with(&format!("{APP}/")))
        .collect();
    if components.len() != 1 || components[0].len() > 512 || !valid_launcher(components[0]) {
        return Err(failure(
            "physical exact installed product launcher could not be resolved",
        ));
    }
    Ok(components[0].to_owned())
}
async fn launch_product(adb: &PhysicalAdb) -> io::Result<()> {
    let launcher = product_launcher(adb).await?;
    let result = adb
        .shell(&["am", "start", "-W", "-n", &launcher], 16_384)
        .await?;
    if !text(&result)?
        .lines()
        .any(|line| line.trim() == "Status: ok")
    {
        return Err(failure("physical resolved product launcher did not start"));
    }
    Ok(())
}

impl Matrix {
    async fn camera_pairing(&mut self) -> io::Result<()> {
        let unpaired = self.observer.retained().await?;
        self.first_open_ref()?
            .before_private(&unpaired, &self.host)?;
        self.human_stage("camera-ready","On this retained phone, open Add desktop and allow the real camera. Keep private Paste empty. Point away from any invitation; the runner has not displayed its new QR yet. Do not paste/type a pairing URI.").await?;
        let camera = self.observer.snapshot(None).await?;
        if camera["pairing"]["screenPairing"] != true
            || camera["pairing"]["previewPresent"] != false
        {
            return Err(failure(
                "physical camera stage did not start on an empty current Pairing screen",
            ));
        }
        self.action("camera-native-frame", "Observe", BTreeMap::new())
            .await?;
        if !io::stdout().is_terminal() {
            return Err(failure(
                "physical real QR may be shown only on the local private human TTY, never captured evidence",
            ));
        }
        self.recheck()?;
        let mut command = std::process::Command::new(&self.stock.binary);
        command.arg("pair");
        super::super::scoped(&mut command, &self.stock.root, &self.stock.socket);
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null());
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
        let mut child = AsyncCommand::from(command).kill_on_drop(false).spawn()?;
        if !timeout(Duration::from_secs(60), child.wait())
            .await
            .map_err(|_| failure("physical real QR command expired; invitation is not replayed"))??
            .success()
        {
            return Err(failure("physical real terminal-only QR command failed"));
        }
        let preview = self
            .wait_snapshot(
                "camera-real-preview",
                None,
                Duration::from_secs(180),
                |snapshot| {
                    let pairing = &snapshot["pairing"];
                    pairing["screenPairing"] == true
                        && pairing["previewPresent"] == true
                        && pairing["hostMatches"] == true
                        && pairing["nameMatches"] == true
                        && pairing["fingerprintMatches"] == true
                },
            )
            .await?;
        self.action("camera-confirm-control", "Observe", BTreeMap::new())
            .await?;
        self.record("camera-source-disclosure",&json!({"cameraFrameStatusObservedBeforeQr":true,"realTerminalQrDisplayed":true,
            "privatePasteEmptyBeforeQr":true,"pasteAbsentOnPreview":true,"preview":preview["pairing"],"scannerCallbackTraceClaim":false,
            "provenance":"ordered actual camera UI, local human presentation, resident trust preview and authoritative pairing; not injected URI or a synthetic decoder trace"}))?;
        self.human_stage("camera-confirm-once","Review the private Goatr MVP host name and full fingerprint on the phone. Tap its actual Confirm host-wide pairing ONCE. Do not retry or paste another invitation.").await?;
        let snapshot = self
            .wait_snapshot(
                "camera-authoritative-phone-pairing",
                None,
                Duration::from_secs(45),
                |snapshot| {
                    snapshot["hello"] == true
                        && snapshot["inventoryReady"] == true
                        && snapshot["mutationReady"] == true
                        && !snapshot["device"].is_null()
                },
            )
            .await?;
        let retained = self.observer.retained().await?;
        self.first_open_ref()?.preserved_with_private_host(
            &retained,
            &self.host,
            self.first_open_ref()?.service_requested,
            false,
        )?;
        let private = retained
            .hosts
            .iter()
            .find(|host| host.host == self.host)
            .ok_or_else(|| failure("physical camera pairing did not add the exact private host"))?;
        if private.device.is_none()
            || snapshot["device"].as_str() != private.device.as_deref()
            || private.device.as_deref() == Some(self.stock.phone.hello.device_id.0.as_str())
            || !retained.run_label_collision
        {
            return Err(failure(
                "physical new phone device/host/name/Hello identity is not independent of the helper pairing",
            ));
        }
        self.record("camera-paired-retained-state",&json!({"facts":retained,"phoneDevice":private.device,"helperDeviceIsDifferent":true,"phonePairingProof":"real preview, explicit native confirmation, new retained private host/device and authenticated Hello"}))?;
        self.paired = Some(retained);
        Ok(())
    }
    async fn ordinary_shell(
        &mut self,
    ) -> io::Result<(ShellSummary, TerminalShellProcess, ShellSummary)> {
        let inventory = self.stock.inventory().await?;
        if !inventory.sessions.is_empty() || inventory.shells.len() != 1 {
            return Err(failure(
                "physical fresh stock fixture is not exactly its new ordinary startup shell before provider creation",
            ));
        }
        let sibling = inventory.shells[0].clone();
        self.action("shell-dashboard", "NavSessions", BTreeMap::new())
            .await?;
        self.human_stage("shell-private-card-visible","Bring only the new private Goatr MVP host card into view on Sessions. Do not select or change a retained daily host.").await?;
        self.action("shell-list", "OpenShells", BTreeMap::new())
            .await?;
        self.action(
            "shell-create-workspace-picker",
            "OpenTerminalCreation",
            BTreeMap::new(),
        )
        .await?;
        let mut placement = BTreeMap::new();
        extend(
            &mut placement,
            &[
                ("ownedUiServerId", &sibling.server_id.0),
                ("ownedUiWorkspaceId", &sibling.workspace_id.0),
            ],
        );
        self.action(
            "shell-create-workspace",
            "SelectTerminalWorkspace",
            placement.clone(),
        )
        .await?;
        placement.insert(
            "ownedUiProjectId".to_owned(),
            self.stock.config.projects[0].id.0.clone(),
        );
        self.action(
            "shell-create-project",
            "SelectTerminalProject",
            placement.clone(),
        )
        .await?;
        self.wait_snapshot(
            "shell-create-real-directory-admission",
            None,
            Duration::from_secs(30),
            |snapshot| snapshot["creationReady"] == true,
        )
        .await?;
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.action(
            "shell-create-submit-once",
            "SubmitTerminalCreation",
            placement,
        )
        .await?;
        let receipt = self
            .new_receipt(
                "shell-create-native-receipt",
                &before,
                "TerminalCreate",
                None,
                None,
            )
            .await?;
        let target = ShellTarget {
            shell_id: Uuid::new(string(&receipt, "createdShell")?).map_err(io::Error::other)?,
            generation: Uuid::new(string(&receipt, "createdShellGeneration")?)
                .map_err(io::Error::other)?,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let created = loop {
            let inventory = self.stock.inventory().await?;
            if let Some(shell) = inventory.shells.into_iter().find(|shell| {
                shell.target == target && shell.connectivity == SessionConnectivity::Ready
            }) {
                break shell;
            }
            if Instant::now() >= deadline {
                return Err(failure(
                    "physical successful terminal receipt never appeared as its exact ready native shell",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let native = self.stock.native_snapshot().await?;
        let pane = native.pane(&created.pane_id)?;
        if pane.terminal_id != created.terminal_id
            || pane.workspace_id != created.workspace_id
            || pane.tab_id != created.tab_id
            || created.server_id != sibling.server_id
        {
            return Err(failure(
                "physical created shell receipt/inventory/native routing differs",
            ));
        }
        let info = self.stock.native_process_info(&created.pane_id).await?;
        let process=TerminalShellProcess::capture(&self.stock.desktop.process,&info,&self.stock.config.projects[0].path)?
            .ok_or_else(||failure("physical created shell is not its actual owned idle shell in the configured project"))?;
        self.record("shell-created-native",&json!({"target":created.target,"server":created.server_id,"workspace":created.workspace_id,"pane":created.pane_id,"terminal":created.terminal_id,"process":process.process,"sibling":sibling.target}))?;
        self.human_stage("shell-creation-dismiss","Dismiss ONLY the completed private terminal-creation dialog with Cancel. This must leave the newly created native terminal running; do not create another terminal.").await?;
        self.action(
            "shell-open-created",
            "OpenShell",
            shell_arguments(&created.target),
        )
        .await?;
        self.wait_snapshot(
            "shell-created-metadata-before-begin",
            None,
            Duration::from_secs(30),
            |snapshot| shell_admission(snapshot, &created.target) == Some(ShellAdmission::Metadata),
        )
        .await?;
        self.action(
            "shell-begin",
            "BeginShellControl",
            shell_arguments(&created.target),
        )
        .await?;
        self.action(
            "shell-confirm",
            "ConfirmShellControl",
            shell_arguments(&created.target),
        )
        .await?;
        self.wait_snapshot(
            "shell-first-committed-grid-after-explicit-begin",
            None,
            Duration::from_secs(30),
            |snapshot| {
                shell_admission(snapshot, &created.target) == Some(ShellAdmission::Committed)
            },
        )
        .await?;
        Ok((created, process, sibling))
    }
    async fn shell_key(
        &mut self,
        stage: &'static str,
        shell: &ShellSummary,
        key: &str,
    ) -> io::Result<()> {
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        let mut arguments = shell_arguments(&shell.target);
        arguments.insert("ownedUiKey".to_owned(), key.to_owned());
        self.action(stage, "SendShellKey", arguments).await?;
        self.new_receipt(stage, &before, "ShellInput", None, Some(&shell.target))
            .await?;
        Ok(())
    }
    async fn foreground_tool(
        &mut self,
        stage: &'static str,
        shell: &ShellSummary,
        name: &str,
    ) -> io::Result<ProcessEvidence> {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            self.recheck()?;
            let info = self.stock.native_process_info(&shell.pane_id).await?;
            self.recheck()?;
            for foreground in &info.foreground_processes {
                let executable = fs::read_link(format!("/proc/{}/exe", foreground.pid))?;
                let basename = executable
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                let matches = if name == "vim" {
                    basename == "vim" || basename.starts_with("vim.") || basename == "nvim"
                } else {
                    basename == name
                };
                if matches {
                    goatr::config::executable(&executable)?;
                    let process =
                        herdr::reported_process(&self.stock.desktop.process, foreground.pid)?;
                    dev_continuity::zero_native_caps(&process)?;
                    let cwd = fs::metadata(format!("/proc/{}/cwd", process.pid))?;
                    let project = fs::metadata(&self.stock.config.projects[0].path)?;
                    if (cwd.dev(), cwd.ino()) != (project.dev(), project.ino()) {
                        return Err(failure(
                            "physical foreground TUI is outside the exact owned project",
                        ));
                    }
                    self.wait_snapshot(stage, None, Duration::from_secs(30), |snapshot| {
                        snapshot["selectedShell"] == shell.target.shell_id.0
                            && snapshot["alternate"] == true
                            && snapshot["grid"] == true
                    })
                    .await?;
                    self.record(&format!("{stage}-native"),&json!({"process":process,"executable":executable,"sameOwnedProject":true,"alternateAnsiDisplayObserved":true}))?;
                    return Ok(process);
                }
            }
            if Instant::now() >= deadline {
                return Err(failure(
                    "physical actual vim/top foreground process was not observed; human acknowledgement is insufficient",
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    async fn shell_idle(
        &mut self,
        shell: &ShellSummary,
        original: &TerminalShellProcess,
    ) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.recheck()?;
            original.recheck()?;
            let info = self.stock.native_process_info(&shell.pane_id).await?;
            if terminal_idle_pid(&info) == Some(original.process.pid) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(failure(
                    "physical same original ordinary shell did not return idle",
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    async fn shell_workflow(&mut self) -> io::Result<(ShellSummary, ShellSummary)> {
        let (shell, process, sibling) = self.ordinary_shell().await?;
        self.action(
            "shell-harmless-draft",
            "SetShellDraft",
            shell_arguments(&shell.target),
        )
        .await?;
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.action(
            "shell-harmless-send",
            "SendShellDraft",
            shell_arguments(&shell.target),
        )
        .await?;
        self.new_receipt(
            "shell-send-frame",
            &before,
            "ShellInput",
            None,
            Some(&shell.target),
        )
        .await?;
        let marker = format!("GOATR_MVP_{}", self.run.0);
        if self.observer.snapshot(Some(&marker)).await?["markerOutput"] == true {
            return Err(failure(
                "physical Draft Send added Enter; echoed command text is not actual output proof",
            ));
        }
        self.shell_key("shell-left", &shell, "Left").await?;
        self.shell_key("shell-right", &shell, "Right").await?;
        self.shell_key("shell-separate-enter", &shell, "Enter")
            .await?;
        self.wait_snapshot(
            "shell-actual-echo-output",
            Some(&marker),
            Duration::from_secs(30),
            |snapshot| snapshot["markerOutput"] == true,
        )
        .await?;
        self.shell_idle(&shell, &process).await?;
        let unsent = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.human_stage("shell-real-ime-composition","Select Goatr Draft input, then start a harmless real phone IME composition and LEAVE it composing without Send/Enter. Enter yes here on the separate host keyboard while phone composition remains active.").await?;
        let composing = self
            .wait_snapshot(
                "shell-composition-local-only",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["shellInput"]["composition"] == true
                        && snapshot["shellInput"]["direct"] == false
                },
            )
            .await?;
        if receipt_ids(&composing)? != unsent {
            return Err(failure(
                "physical unfinished IME composition allocated a durable native-input action",
            ));
        }
        self.human_stage("shell-compose-commit-clear-local","Commit/cancel that composition LOCALLY, then clear only this owned unsent Draft with the phone IME. Do NOT tap Send or Enter.").await?;
        let cleared = self
            .wait_snapshot(
                "shell-local-composition-cleared",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["shellInput"]["composition"] == false
                        && snapshot["shellInput"]["draftBytes"] == 0
                },
            )
            .await?;
        if receipt_ids(&cleared)? != unsent {
            return Err(failure("physical local compose/clear sent native input"));
        }
        self.human_stage("shell-oversize-public-paste-rejection","Using only normal phone clipboard UI, attempt an intentionally oversized PUBLIC ASCII Paste exceeding16KiB into this EMPTY Draft. Do not send/Enter. Observe the limit and keep all capability/private clipboard content out of this test.").await?;
        let rejected = self.observer.snapshot(None).await?;
        if receipt_ids(&rejected)? != unsent || rejected["shellInput"]["draftBytes"] != 0 {
            return Err(failure(
                "physical oversized local Draft attempt changed accepted text or allocated native input",
            ));
        }
        self.record("shell-oversize-consumer-disclosure",&json!({"noDurableInputAction":true,"emptyDraftUnchanged":true,"exactClipboardLengthCaptured":false,"clipboardExported":false,"precise16KiBBoundaryIsAlsoOwnedSdkRegression":true}))?;
        let paste = format!("GOATR_MVP_PASTE_{}", self.run.0);
        self.human_stage("shell-real-public-paste-send",&format!("Use Goatr's explicit Paste with PUBLIC text `echo {paste}` and tap Draft Send ONCE. Do NOT press Enter yet. Paste/Send must not add Enter; no desktop input or private clipboard substitute.")).await?;
        self.new_receipt(
            "shell-real-paste-native-frame",
            &unsent,
            "ShellInput",
            None,
            Some(&shell.target),
        )
        .await?;
        if self.observer.snapshot(Some(&paste)).await?["markerOutput"] == true {
            return Err(failure("physical Paste/Send added an implicit Enter"));
        }
        self.shell_key("shell-paste-separate-enter", &shell, "Enter")
            .await?;
        self.wait_snapshot(
            "shell-real-paste-output",
            Some(&paste),
            Duration::from_secs(30),
            |snapshot| snapshot["markerOutput"] == true,
        )
        .await?;
        self.shell_idle(&shell, &process).await?;
        self.human_stage("shell-real-vim","In THIS Goatr ordinary-shell Live input, type `vim -u NONE -n` with the real phone IME and separate Enter. Do not start vim from the desktop. Leave vim open for native-process/display observation.").await?;
        self.foreground_tool("shell-vim-native-and-ansi", &shell, "vim")
            .await?;
        self.shell_key("shell-vim-escape", &shell, "Escape").await?;
        self.human_stage("shell-vim-exit","Use this Goatr shell's real IME to enter `:q!` and separate Enter, exiting only the owned empty vim.").await?;
        self.shell_idle(&shell, &process).await?;
        self.human_stage("shell-real-top","In this same Goatr shell use the real phone IME to run `top` with separate Enter, and leave it running.").await?;
        self.foreground_tool("shell-top-native-and-ansi", &shell, "top")
            .await?;
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        let mut ctrl = shell_arguments(&shell.target);
        ctrl.insert("ownedUiKey".to_owned(), "Ctrl".to_owned());
        self.action("shell-control-next", "SetShellModifier", ctrl)
            .await?;
        self.human_stage("shell-control-c","With Goatr Ctrl now armed for ONE key, enter `c` in the shell key bar's Character field and tap its Send-character control ONCE. Do not use a desktop signal or host keyboard to stop top.").await?;
        self.new_receipt(
            "shell-control-c-native-frame",
            &before,
            "ShellInput",
            None,
            Some(&shell.target),
        )
        .await?;
        self.shell_idle(&shell, &process).await?;
        let key_marker = format!("GOATR_MVP_KEYS_{}", self.run.0);
        let keys_before = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.human_stage("shell-alt-tab-arrows",&format!("In this SAME Goatr shell use Alt word movement, Tab, Escape and arrow editing controls on harmless PUBLIC line `echo {key_marker}`. Correct the line using only the phone controls, then press the separate Goatr Enter ONCE. Do not use desktop input; leave the same original shell idle.")).await?;
        self.wait_snapshot(
            "shell-real-edited-key-output",
            Some(&key_marker),
            Duration::from_secs(30),
            |snapshot| snapshot["markerOutput"] == true,
        )
        .await?;
        if receipt_ids(&self.observer.snapshot(None).await?)?.len() <= keys_before.len() {
            return Err(failure(
                "physical key exercise has no actual durable shell-input handoff",
            ));
        }
        self.shell_idle(&shell, &process).await?;
        process.recheck()?;
        let prior = self.observer.snapshot(None).await?;
        let columns = if unsigned(&prior, "columns")? == 96 {
            80
        } else {
            96
        };
        let rows = if unsigned(&prior, "rows")? == 28 {
            24
        } else {
            28
        };
        self.action(
            "shell-resize-dialog",
            "OpenShellResize",
            shell_arguments(&shell.target),
        )
        .await?;
        for (field, stage) in [
            ("Columns", "shell-resize-columns"),
            ("Rows", "shell-resize-rows"),
        ] {
            let mut arguments = shell_arguments(&shell.target);
            extend(
                &mut arguments,
                &[
                    ("ownedUiColumns", &columns.to_string()),
                    ("ownedUiRows", &rows.to_string()),
                    ("ownedUiSizeField", field),
                ],
            );
            self.action(stage, "SetShellSize", arguments).await?;
        }
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        let mut arguments = shell_arguments(&shell.target);
        extend(
            &mut arguments,
            &[
                ("ownedUiColumns", &columns.to_string()),
                ("ownedUiRows", &rows.to_string()),
            ],
        );
        self.action("shell-resize-confirm", "ConfirmShellResize", arguments)
            .await?;
        self.new_receipt(
            "shell-resize-native-receipt",
            &before,
            "ShellResize",
            None,
            Some(&shell.target),
        )
        .await?;
        self.wait_snapshot(
            "shell-resize-actual-native-frame",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["columns"] == columns
                    && snapshot["rows"] == rows
                    && snapshot["revision"].as_u64() > prior["revision"].as_u64()
            },
        )
        .await?;
        self.human_stage("shell-numbered-history","In this Goatr shell, use the real IME to run `seq 1 180` with separate Enter. Wait until it returns idle. Do not generate output through a helper/native RPC or another pane.").await?;
        self.shell_idle(&shell, &process).await?;
        let live = self
            .wait_snapshot(
                "shell-native-scrollback-ready",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["scrollMaximum"]
                        .as_i64()
                        .is_some_and(|value| value > 0)
                        && snapshot["scrollOffset"] == 0
                        && !snapshot["alternate"].as_bool().unwrap_or(true)
                },
            )
            .await?;
        let before_native = self.stock.native_snapshot().await?;
        let source = before_native.pane(&shell.pane_id)?;
        let scroll = source
            .scroll
            .as_ref()
            .ok_or_else(|| failure("physical stock pane snapshot has no actual history metrics"))?;
        if scroll.max_offset_from_bottom != unsigned(&live, "scrollMaximum")?
            || scroll.viewport_rows != unsigned(&live, "scrollViewport")?
        {
            return Err(failure(
                "physical phone history metrics do not match the original stock pane",
            ));
        }
        let mut expected = 0;
        let mut found = false;
        for _ in 0..64 {
            let current = self.observer.snapshot(Some(&marker)).await?;
            if current["markerOutput"] == true && expected > 0 {
                found = true;
                break;
            }
            let offset = unsigned(&current, "scrollOffset")?;
            let maximum = unsigned(&current, "scrollMaximum")?;
            let viewport = unsigned(&current, "scrollViewport")?;
            if offset == maximum || viewport == 0 {
                break;
            }
            expected = offset.saturating_add(viewport).min(maximum);
            let mut older = shell_arguments(&shell.target);
            extend(
                &mut older,
                &[
                    ("ownedUiScrollDirection", "Older"),
                    ("ownedUiScrollOffset", &expected.to_string()),
                ],
            );
            let ids = receipt_ids(&current)?;
            self.action("shell-history-older", "ScrollShellHistory", older)
                .await?;
            let receipt = self
                .new_receipt(
                    "shell-history-older-ack",
                    &ids,
                    "ShellScroll",
                    None,
                    Some(&shell.target),
                )
                .await?;
            let action = string(&receipt, "action")?;
            self.wait_snapshot(
                "shell-history-native-older",
                None,
                Duration::from_secs(30),
                |snapshot| shell_history_ack(snapshot, &shell.target, action, expected),
            )
            .await?;
        }
        if !found {
            return Err(failure(
                "physical older shared viewport did not reveal the real earlier public output marker",
            ));
        }
        let blocked = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.human_stage("shell-history-input-paused","While this actual Older/Browsing view is visible, attempt harmless phone typing/a shell key. It must stay paused and unsent. Do NOT tap Bottom yet or use another frontend to restore it.").await?;
        let paused = self.observer.snapshot(None).await?;
        if paused["shellInput"]["offered"] != false || receipt_ids(&paused)? != blocked {
            return Err(failure(
                "physical Browsing view admitted native input before explicit bottom",
            ));
        }
        let older_native = self.stock.native_snapshot().await?;
        let selected = older_native.pane(&shell.pane_id)?;
        if selected
            .scroll
            .as_ref()
            .map(|metrics| metrics.offset_from_bottom)
            != Some(expected)
            || before_native.layouts != older_native.layouts
            || before_native.focused_pane_id != older_native.focused_pane_id
            || before_native.focused_tab_id != older_native.focused_tab_id
            || before_native.focused_workspace_id != older_native.focused_workspace_id
        {
            return Err(failure(
                "physical shared history moved a sibling geometry/focus or failed its exact original native viewport effect",
            ));
        }
        for _ in 0..64 {
            let current = self.observer.snapshot(None).await?;
            let Some(expected) = nonzero_newer(&current) else {
                break;
            };
            let ids = receipt_ids(&current)?;
            let mut newer = shell_arguments(&shell.target);
            extend(
                &mut newer,
                &[
                    ("ownedUiScrollDirection", "Newer"),
                    ("ownedUiScrollOffset", &expected.to_string()),
                ],
            );
            self.action("shell-history-newer", "ScrollShellHistory", newer)
                .await?;
            let receipt = self
                .new_receipt(
                    "shell-history-nonzero-newer-ack",
                    &ids,
                    "ShellScroll",
                    None,
                    Some(&shell.target),
                )
                .await?;
            let action = string(&receipt, "action")?;
            self.wait_snapshot(
                "shell-history-nonzero-newer-still-paused",
                None,
                Duration::from_secs(30),
                |snapshot| shell_history_ack(snapshot, &shell.target, action, expected),
            )
            .await?;
        }
        let before_bottom = self.observer.snapshot(None).await?;
        let ids = receipt_ids(&before_bottom)?;
        if before_bottom["scrollOffset"]
            .as_u64()
            .is_none_or(|offset| offset == 0)
            || before_bottom["historyMode"] != "Browsing"
        {
            return Err(failure(
                "physical explicit Bottom must own the remaining nonzero Browsing transition",
            ));
        }
        self.action(
            "shell-history-bottom",
            "ShellHistoryBottom",
            shell_arguments(&shell.target),
        )
        .await?;
        let receipt = self
            .new_receipt(
                "shell-history-explicit-bottom-receipt",
                &ids,
                "ShellScroll",
                None,
                Some(&shell.target),
            )
            .await?;
        let action = string(&receipt, "action")?;
        self.wait_snapshot(
            "shell-history-explicit-bottom-ack",
            None,
            Duration::from_secs(30),
            |snapshot| shell_history_ack(snapshot, &shell.target, action, 0),
        )
        .await?;
        let ids = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.action(
            "shell-history-already-zero-bottom",
            "ShellHistoryBottom",
            shell_arguments(&shell.target),
        )
        .await?;
        let receipt = self
            .new_receipt(
                "shell-history-already-zero-fresh-receipt",
                &ids,
                "ShellScroll",
                None,
                Some(&shell.target),
            )
            .await?;
        let action = string(&receipt, "action")?;
        self.wait_snapshot(
            "shell-history-already-zero-independent-ack",
            None,
            Duration::from_secs(30),
            |snapshot| shell_history_ack(snapshot, &shell.target, action, 0),
        )
        .await?;
        self.action(
            "shell-release-control",
            "ReleaseShellControl",
            shell_arguments(&shell.target),
        )
        .await?;
        self.wait_snapshot(
            "shell-release-owner-retired",
            None,
            Duration::from_secs(30),
            |snapshot| snapshot["controlBegun"] == false && snapshot["oldInputRetired"] == true,
        )
        .await?;
        self.record("shell-workflow-originals",&json!({"controlled":shell.target,"sibling":sibling.target,"process":process.process,"nativeClose":false,"nativeInputHelper":false,"historyScope":"shared native viewport; no native atomic fence"}))?;
        Ok((shell, sibling))
    }
}

fn recheck_inputs(inputs: &[(PathBuf, File, fs::Metadata)]) -> io::Result<()> {
    for (path, file, original) in inputs {
        if !original.is_file()
            || complete_stamp(&file.metadata()?) != complete_stamp(original)
            || complete_stamp(&fs::symlink_metadata(path)?) != complete_stamp(original)
            || fs::canonicalize(path)? != *path
        {
            return Err(failure(
                "physical frozen original input was replaced or overwritten; no new artifact is adopted",
            ));
        }
    }
    Ok(())
}
fn recheck_retained_inputs(
    inputs: &[(PathBuf, File, fs::Metadata)],
    failed: &mut bool,
) -> io::Result<()> {
    if *failed {
        return Err(failure(
            "physical original input authority already failed; no recovery or Finish admission",
        ));
    }
    if let Err(error) = recheck_inputs(inputs) {
        *failed = true;
        return Err(error);
    }
    Ok(())
}
fn finish_authorization(input_pin_failed: bool, expected: &str) -> Option<&str> {
    (!input_pin_failed).then_some(expected)
}
fn blocker(error: &io::Error) -> &'static str {
    if let Some(diagnostic) = admission_diagnostic(error) {
        return diagnostic.guard;
    }
    match error.kind() {
        io::ErrorKind::NotFound => "required_original_input_unavailable",
        io::ErrorKind::PermissionDenied => "original_authority_unavailable",
        io::ErrorKind::NotConnected | io::ErrorKind::ConnectionRefused => {
            "original_transport_unavailable"
        }
        io::ErrorKind::TimedOut => "owned_deadline_expired",
        _ => "required_original_effect_or_identity_unproved",
    }
}
pub(super) async fn run(
    args: &clap::ArgMatches,
    evidence: &Path,
    admitted: goatr::codex_relay::UnitWakePin,
) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(failure(
            "physical MVP requires the real local private human TTY; no synthetic camera/input acknowledgement",
        ));
    }
    let run = Uuid::random();
    let label = format!("Goatr MVP {}", run.0);
    let (product_start, prepared_pin) = match (
        args.get_one::<PathBuf>("prepared-fresh-install"),
        args.get_one::<String>("expected-prepared-install-sha256"),
    ) {
        (None, None) => (None, None),
        (Some(path), Some(hash)) => {
            let (record, pin) = read_prepared_install(path, hash)?;
            (Some(record), Some(pin))
        }
        _ => {
            return Err(failure(
                "prepared install record and independent hash must be supplied together",
            ));
        }
    };
    let controller_sha = args
        .get_one::<String>("expected-controller-sha256")
        .ok_or_else(|| prerequisite("independent_controller_pin_unavailable"))?;
    if args
        .get_one::<String>("expected-companion-lock-sha256")
        .map(String::as_str)
        != Some(transport::lock_sha256())
    {
        return Err(failure("physical source lock pin differs"));
    }
    let binary = canonical_file(
        args.get_one::<PathBuf>("goatr")
            .ok_or_else(|| failure("physical frozen Goatr path missing"))?,
    )?;
    let expected_binary_sha = args
        .get_one::<String>("expected-goatr-sha256")
        .ok_or_else(|| failure("physical independent serving binary SHA-256 is missing"))?;
    let serve_input = frozen_input(&binary, expected_binary_sha)?;
    let controller_input = frozen_input(&std::env::current_exe()?, controller_sha)?;
    admitted.observe((|| {
        if complete_stamp(&fs::metadata("/proc/self/exe")?) != complete_stamp(&controller_input.2) {
            return Err(failure(
                "held independently hashed controller is not the original executing object",
            ));
        }
        Ok(())
    })())?;
    let unit = args
        .get_one::<String>("expected-controller-unit")
        .ok_or_else(|| failure("explicit controller unit unavailable"))?;
    let authority = match original_host_authority(
        ControllerPolicy::UnitWake { unit },
        None,
        Some(admitted),
    ) {
        Ok(authority) => authority,
        Err(error) => {
            write_json(
                &evidence.join("physical-original-host-blocked.json"),
                &json!({"runId":run,"gate":"physical-mvp","status":"blocked","stage":"original-host-authority",
                "code":blocker(&error),"nativeLaunch":false,"productUpdate":false,"managerCreatedOrRestarted":false,"runtimeAcceptanceClaim":false}),
            )?;
            return Err(error);
        }
    };
    write_json(
        &evidence.join("physical-original-host-authority.json"),
        &authority.facts,
    )?;
    let template = args
        .get_one::<PathBuf>("config")
        .ok_or_else(|| failure("physical independently pinned neutral seed is missing"))?;
    let template_sha = args
        .get_one::<String>("expected-config-sha256")
        .ok_or_else(|| prerequisite("independent_seed_config_pin_unavailable"))?;
    let (deployment, template_file, template_metadata) =
        qualified_configuration(template, template_sha)?;
    let sdk = canonical_directory(&PathBuf::from(
        std::env::var_os("ANDROID_HOME")
            .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
            .ok_or_else(|| failure("physical pinned SDK input is missing"))?,
    ))?;
    let apk = canonical_file(
        args.get_one::<PathBuf>("apk")
            .ok_or_else(|| failure("physical frozen product APK is missing"))?,
    )?;
    let test_apk = canonical_file(
        args.get_one::<PathBuf>("test-apk")
            .ok_or_else(|| failure("physical frozen test APK is missing"))?,
    )?;
    let product_sha = args
        .get_one::<String>("expected-apk-sha256")
        .ok_or_else(|| failure("physical product APK hash is missing"))?;
    let test_sha = args
        .get_one::<String>("expected-test-apk-sha256")
        .ok_or_else(|| failure("physical test APK hash is missing"))?;
    let mut inputs = Vec::with_capacity(6);
    inputs.push(serve_input);
    inputs.push(frozen_input(&apk, product_sha)?);
    inputs.push(frozen_input(&test_apk, test_sha)?);
    inputs.push((template.clone(), template_file, template_metadata));
    inputs.push(controller_input);
    if let Some(pin) = prepared_pin {
        inputs.push(pin);
    }
    recheck_inputs(&inputs)?;
    let (adb, device) = PhysicalAdb::qualify(args).await?;
    recheck_inputs(&inputs)?;
    let api = device.sdk;
    let abi = device.abi.clone();
    adb.tunnels(None).await?;
    recheck_inputs(&inputs)?;
    let user = adb.shell(&["am", "get-current-user"], 128).await?;
    recheck_inputs(&inputs)?;
    if text(&user)?.trim() != "0" {
        return Err(failure(
            "physical retained installation must be the exact primary Android user; no profile/user switching",
        ));
    }
    recheck_inputs(&inputs)?;
    let product = qualify_artifact(&apk, product_sha, &sdk, APP, false).await?;
    recheck_inputs(&inputs)?;
    let test = qualify_artifact(&test_apk, test_sha, &sdk, TEST, true).await?;
    recheck_inputs(&inputs)?;
    if product.signer != test.signer {
        return Err(failure(
            "physical product/test artifacts do not have the same qualified signer",
        ));
    }
    let dependency = provider_dependency_proof()?;
    let dependency_sha =
        digest_text(&serde_json::to_string(&dependency).map_err(io::Error::other)?);
    write_json(
        &evidence.join("physical-pre-application-dependency-closure.json"),
        &dependency,
    )?;
    let native=canonical_directory(&PathBuf::from(std::env::var_os("GOATR_IROH_ANDROID").ok_or_else(||failure("physical actual source-built native output is required for shared all-ten inventory"))?))?;
    let inventory_inputs = super::super::native_inventory::InventoryInputs {
        apk: product.path.clone(),
        native,
        jna: maven_aar("net.java.dev.jna", "jna", "5.19.1")?,
        camera: maven_aar("androidx.camera", "camera-core", "1.6.1")?,
        graphics: maven_aar("androidx.graphics", "graphics-path", "1.0.1")?,
        sdk: sdk.clone(),
        retained_source: canonical_file(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../third_party/iroh-ffi/source.json"),
        )?,
        scratch_parent: canonical_directory(evidence)?,
    };
    recheck_inputs(&inputs)?;
    let inventory = tokio::task::spawn_blocking(move || {
        super::super::native_inventory::inspect(&inventory_inputs)
    })
    .await
    .map_err(|_| failure("physical shared static inventory owner failed"))??;
    recheck_inputs(&inputs)?;
    if inventory["status"] != "pass"
        || inventory["count"] != 10
        || inventory["apkSha256"] != product.sha256
    {
        return Err(failure(
            "physical actual shared all-ten native inventory did not qualify this exact frozen APK",
        ));
    }
    write_json(&evidence.join("physical-native-inventory.json"), &inventory)?;
    adb.tunnels(None).await?;
    recheck_inputs(&inputs)?;
    let user = adb.shell(&["am", "get-current-user"], 128).await?;
    recheck_inputs(&inputs)?;
    if text(&user)?.trim() != "0" {
        return Err(failure(
            "physical retained installation must be the exact primary Android user; no profile/user switching",
        ));
    }
    let packages_before = package_inventory(&adb).await?;
    recheck_inputs(&inputs)?;
    admit_product_inventory(&product_start, &packages_before)?;
    if let Some(prepared) = &product_start {
        prepared_artifact_admission(prepared, &device, &product, inputs[1].2.len())?;
        // CURRENT installed bytes are pulled and independently SDK/signature/manifest qualified.
        let current = installed_artifact(
            &adb,
            &sdk,
            APP,
            evidence,
            "physical-prepared-product-current.apk",
        )
        .await?;
        recheck_inputs(&inputs)?;
        prepared_artifact_admission(
            prepared,
            &device,
            &current,
            fs::metadata(&current.path)?.len(),
        )?;
        admit_product_start(&product_start, &adb, &packages_before, &inputs).await?;
        write_json(
            &evidence.join("physical-prepared-product-intake.json"),
            &json!({"creatorRun":prepared.run,"originalRecordPin":args.get_one::<String>("expected-prepared-install-sha256"),
            "epoch":prepared.epoch,"currentSdkArtifactSha256":current.sha256,"evidenceReferencesNotFollowed":prepared.references,"runtimeOrRawBaselineClaim":false}),
        )?;
    }
    let test_record = packages_before.get(TEST).ok_or_else(|| {
        prerequisite("physical residual same-signer TEST installation is required")
    })?;
    let test_ownership_before = package_ownership(&adb, test_record).await?;
    recheck_inputs(&inputs)?;
    let previous_test =
        installed_artifact(&adb, &sdk, TEST, evidence, "physical-residual-test.apk").await?;
    recheck_inputs(&inputs)?;
    test_update_admission(&previous_test, &test)?;
    let verify_record = packages_before.get(VERIFY);
    let verify_ownership = match verify_record {
        Some(record) => Some(package_ownership(&adb, record).await?),
        None => None,
    };
    recheck_inputs(&inputs)?;
    let verify = match verify_record {
        Some(_) => Some(
            installed_artifact(
                &adb,
                &sdk,
                VERIFY,
                evidence,
                "physical-untouched-verify.apk",
            )
            .await?,
        ),
        None => None,
    };
    recheck_inputs(&inputs)?;
    let ntfy = installed_artifact(
        &adb,
        &sdk,
        "io.heckel.ntfy",
        evidence,
        "physical-official-ntfy.apk",
    )
    .await?;
    recheck_inputs(&inputs)?;
    if Some(&ntfy.sha256) != args.get_one::<String>("expected-ntfy-apk-sha256")
        || Some(&ntfy.signer) != args.get_one::<String>("expected-ntfy-signer-sha256")
    {
        return Err(failure(
            "physical installed official ntfy does not match both independently qualified pins",
        ));
    }
    write_json(
        &evidence.join("physical-frozen-inputs.json"),
        &json!({"runId":run,"serial":adb.serial,"api":api,"abi":abi,
        "goatr":binary,"goatrSha256":expected_binary_sha,"config":template,"configSha256":template_sha,"lockSha256":transport::lock_sha256(),
        "productApk":{"path":product.path,"sha256":product.sha256,"signer":product.signer,"debuggable":product.debug},
        "testApk":{"path":test.path,"sha256":test.sha256,"signer":test.signer,"debuggable":test.debug,"instrumentations":2,"fullTestArtifact":true},
        "startingProductAbsent":product_start.is_none(),"preparedCreator":product_start.as_ref().map(|prepared|prepared.run.as_str()),"previousTest":{"sha256":previous_test.sha256,"ownership":test_ownership_before},
        "verify":verify.as_ref().map(|artifact|json!({"sha256":artifact.sha256,"signer":artifact.signer,"ownership":verify_ownership})),
        "officialNtfySha256":ntfy.sha256,"officialNtfySigner":ntfy.signer,"nativeInventory":"actual shared implementation; static only, not JNI/device acceptance"}),
    )?;
    recheck_inputs(&inputs)?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    recheck_inputs(&inputs)?;
    let port = listener.local_addr()?.port();
    let mapping = format!("tcp:{port}");
    recheck_original_host(&authority)?;
    recheck_inputs(&inputs)?;
    install_test_update(
        &adb,
        &previous_test,
        &test_ownership_before,
        &test,
        &product_start,
        &inputs,
    )
    .await?;
    let after_test = package_inventory(&adb).await?;
    recheck_inputs(&inputs)?;
    admit_product_inventory(&product_start, &after_test)?;
    if package_ownership(
        &adb,
        after_test
            .get(TEST)
            .ok_or_else(|| failure("physical updated TEST disappeared"))?,
    )
    .await?
        != test_ownership_before
    {
        return Err(failure(
            "physical TEST update changed its original UID/install/data ownership",
        ));
    }
    verify_unchanged(
        &adb,
        verify_record,
        verify_ownership.as_ref(),
        verify.as_ref(),
    )
    .await?;
    recheck_inputs(&inputs)?;
    if product_start.is_none() {
        install_fresh_product(&adb, &product, &inputs).await?;
    }
    let installed_packages = package_inventory(&adb).await?;
    recheck_inputs(&inputs)?;
    let product_record = installed_packages
        .get(APP)
        .ok_or_else(|| failure("physical fresh PRODUCT did not acquire an installed identity"))?;
    let current_product = product_observation(&adb, product_record).await?;
    recheck_inputs(&inputs)?;
    if let Some(prepared) = &product_start {
        prepared_observation_admission(prepared, &current_product, true)?;
    }
    if !current_product.stopped || !current_product.not_launched {
        return Err(failure(
            "fresh PRODUCT lifecycle changed before baseline; never normalize it",
        ));
    }
    let product_epoch = current_product.epoch;
    let product_ownership = product_epoch.ownership.clone();
    write_json(
        &evidence.join("physical-fresh-product-installed.json"),
        &json!({"runId":run,"package":product_record,"epoch":product_epoch,"apkSha256":product.sha256,"signerSha256":product.signer,
        "newProductInstall":product_start.is_none(),"preparedProductInstallSkipped":product_start.is_some(),"replacementAuthorized":false}),
    )?;
    adb.invoke(&["reverse", "--no-rebind", &mapping, &mapping], 4096)
        .await?;
    recheck_inputs(&inputs)?;
    adb.tunnels(Some(port)).await?;
    recheck_inputs(&inputs)?;
    let before_baseline = package_inventory(&adb).await?;
    recheck_inputs(&inputs)?;
    let record = before_baseline
        .get(APP)
        .ok_or_else(|| failure("fresh PRODUCT disappeared before baseline"))?;
    if record != &product_epoch.record {
        return Err(failure(
            "fresh PRODUCT package epoch changed before baseline",
        ));
    }
    let current = product_observation(&adb, record).await?;
    recheck_inputs(&inputs)?;
    if current.epoch != product_epoch
        || !current.stopped
        || !current.not_launched
        || installed_hash(&adb, APP).await? != product.sha256
    {
        return Err(failure(
            "fresh/prepared PRODUCT changed immediately before baseline",
        ));
    }
    adb.default_product_process_absent().await?;
    recheck_inputs(&inputs)?;
    // Ownership begins below. ONLY stopped/notLaunched/process-absence are consumed by this baseline.
    let observer_artifacts = ObserverArtifacts {
        product: &product,
        product_uid: product_record.uid,
        test: &test,
        verify: verify.as_ref(),
    };
    let baseline_args = observer_arguments(
        "baseline",
        &run,
        &label,
        None,
        &observer_artifacts,
        &dependency_sha,
        port,
    );
    let baseline = fresh_baseline(
        &adb,
        &listener,
        &baseline_args,
        &run,
        product_record.uid,
        &dependency_sha,
        evidence,
    )
    .await?;
    recheck_inputs(&inputs)?;
    require_product_epoch(&adb, &product_epoch, &product.sha256, &inputs, None).await?;
    connector_admission(&baseline, None)?;
    recheck_original_host(&authority)?;
    recheck_inputs(&inputs)?;
    let stock = StockRun::start(
        args,
        evidence,
        run.clone(),
        &label,
        deployment,
        binary,
        StockLaunchPins {
            template_sha,
            expected_binary_sha,
            controller: authority
                .controller_unit
                .as_ref()
                .ok_or_else(|| failure("original admitted controller pin missing"))?,
            original_inputs: &inputs,
        },
    )
    .await?;
    recheck_inputs(&inputs)?;
    let host = stock.phone.hello.host_id.0.clone();
    if !hex64(&host) {
        return Err(failure(
            "physical new private companion host identity is invalid",
        ));
    }
    connector_admission(&baseline, Some(&host))?;
    recheck_original_host(&authority)?;
    recheck_inputs(&inputs)?;
    require_product_epoch(&adb, &product_epoch, &product.sha256, &inputs, None).await?;
    let observe_args = observer_arguments(
        "observe",
        &run,
        &label,
        Some(&host),
        &observer_artifacts,
        &dependency_sha,
        port,
    );
    let instrumentation = ProductInstrumentation::start(&adb, &observe_args, "observe").await?;
    recheck_inputs(&inputs)?;
    let (observer, ready) =
        Observer::start(listener, instrumentation, run.clone(), host.clone()).await?;
    use sha2::{Digest, Sha256};
    let public_sha = hex(&Sha256::digest(observer.epoch.public));
    // From this point EVERY failure stays with this same owner until explicit identity-reviewed Finish.
    let mut matrix = Matrix {
        adb,
        observer,
        stock,
        authority,
        inputs,
        input_pin_failed: false,
        run,
        host,
        label,
        evidence: evidence.to_path_buf(),
        product_sha: product.sha256.clone(),
        public_sha,
        api,
        control_port: port,
        fresh_baseline: baseline,
        first_open: None,
        paired: None,
        last_stage: "observer-admission",
        baseline_ui: None,
        baseline_network: None,
        retained_keeper_baseline: None,
        owned_start: None,
        service_key_restored: false,
        product_epoch,
        prepared_record_sha256: args
            .get_one::<String>("expected-prepared-install-sha256")
            .cloned(),
    };
    let result=async {
        matrix.recheck()?;
        if matrix.observer.epoch.uid!=product_record.uid{return Err(failure("physical observer UID differs from the pinned fresh PRODUCT"));}
        matrix.fresh_baseline.constructor_ready(&ready,product_record.uid)?;
        matrix.record("constructor-ready",&json!({"originalFreshBaseline":matrix.fresh_baseline,"constructorReady":ready,"normalApplicationCreatedOnlyEmptyKeyDirectory":true}))?;
        matrix.recheck_product().await?;launch_product(&matrix.adb).await?;matrix.recheck()?;
        // endpointOwned is claimed before key creation; callback follows durable keys and bind.
        let initial=matrix.wait_snapshot("product-normal-first-open-complete",None,Duration::from_secs(45),|snapshot|snapshot["foreground"]==true&&snapshot["keeper"]["endpointOwned"]==true&&snapshot["keeper"]["networkCallback"]==true).await?;
        matrix.baseline_ui=Some(initial["ui"].clone());matrix.baseline_network=Some(initial["network"].clone());
        matrix.last_stage="first-open-retained-read";
        let first_open=matrix.observer.retained().await.map_err(|error|admission_failure("first_open_retained","read",error,None))?;
        let admission=matrix.fresh_baseline.first_open_admission(&ready,&first_open,product_record.uid);
        let summary=json!({"beforeValidation":true,"admission":admission});
        matrix.last_stage="first-open-admission";
        let publication=matrix.record("first-open-admission",&summary);
        let admitted=matrix.fresh_baseline.admitted_first_open(&ready,&first_open,product_record.uid,&admission);
        first_open_admission_result(admitted,publication,summary)?;
        if matrix.first_open.is_some(){return Err(admission_failure("first_open_publication","reference_already_set",failure("physical first-open reference cannot be refreshed"),None));}
        matrix.last_stage="first-open-reference-publication";
        matrix.record("first-open-reference",&json!({"originalFreshBaseline":matrix.fresh_baseline,"firstOpen":first_open,"beforeCameraPairing":true,"measuredStartup":initial["keeper"]}))
            .map_err(|error|admission_failure("first_open_publication","reference",error,None))?;
        matrix.first_open=Some(first_open);
        matrix.camera_pairing().await?;
        matrix.retained_keeper_baseline=Some(matrix.observer.keeper().await?);
        let (shell,sibling)=matrix.shell_workflow().await?;
        let providers=matrix.provider_workflow().await?;
        matrix.mixed_workflow(&shell,&sibling,&providers).await?;
        matrix.independent_network(&shell,&providers[0]).await?;
        matrix.delivery_workflow(&providers[..2]).await?;
        for provider in &providers {matrix.verify_provider(provider).await?;}
        matrix.recheck()?;
        let final_facts=matrix.observer.retained().await?;
        matrix.first_open_ref()?.preserved_with_private_host(&final_facts,&matrix.host,matrix.first_open_ref()?.service_requested,matrix.service_key_restored)?;
        matrix.recheck_product().await?;
        let final_packages=package_inventory(&matrix.adb).await?;
        if final_packages.get(APP)!=Some(product_record)||package_ownership(&matrix.adb,product_record).await?!=product_ownership
            ||package_ownership(&matrix.adb,final_packages.get(TEST).ok_or_else(||failure("physical TEST installation disappeared"))?).await?!=test_ownership_before
            ||installed_hash(&matrix.adb,APP).await?!=product.sha256||installed_hash(&matrix.adb,TEST).await?!=test.sha256
            ||installed_hash(&matrix.adb,"io.heckel.ntfy").await?!=ntfy.sha256{return Err(failure("physical final original package/UID/install/data/artifact evidence changed"));}
        verify_unchanged(&matrix.adb,verify_record,verify_ownership.as_ref(),verify.as_ref()).await?;matrix.recheck()?;
        let proofs=providers.iter().map(proof_facts).collect::<io::Result<Vec<_>>>()?;
        matrix.record("final-retained-and-original-resources",&json!({"retained":final_facts,"providers":proofs,"shell":shell.target,"sibling":sibling.target,
            "native":matrix.stock.desktop.process,"companion":matrix.stock.serve.process,"root":matrix.stock.root,"admin":matrix.stock.admin,"config":matrix.stock.config_path,
            "nativeSocket":matrix.stock.native_socket,"adminSocket":matrix.stock.socket,"herdrSession":matrix.stock.name,
            "bothDeliveryMatricesDurablyProved":true,"serviceRequestedEffectiveBaselineRestored":matrix.service_key_restored||matrix.first_open_ref()?.service_requested,
            "noNativeCloseOrRestart":true,"noAutomaticNativeCleanup":true,"allRuntimeRowsRequireTheseActualObservations":true}))?;
        Ok(())
    }.await;
    matrix.review_and_finish(result).await
}

impl Matrix {
    fn marker(&self, provider: Provider, phase: &str) -> String {
        let suffix = digest_text(&format!("{}|{phase}", self.run.0));
        let provider = if provider == Provider::Omp {
            "OMP"
        } else {
            "CODEX"
        };
        format!("GOATR_MOBILE_{provider}_{}", &suffix[..32])
    }
    async fn open_provider(&mut self, proof: &NativeProof) -> io::Result<()> {
        self.verify_provider(proof).await?;
        self.action("provider-dashboard", "NavSessions", BTreeMap::new())
            .await?;
        self.action(
            "provider-private-inventory",
            "ViewSessions",
            BTreeMap::new(),
        )
        .await?;
        self.action(
            "provider-original-conversation",
            "OpenSession",
            target_arguments(&proof.target),
        )
        .await?;
        self.wait_snapshot(
            "provider-original-conversation-ready",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["selectedSession"] == proof.target.session_id.0
                    && snapshot["selectedGeneration"] == proof.target.generation.0
                    && snapshot["mutationReady"] == true
            },
        )
        .await?;
        Ok(())
    }
    async fn create_provider(
        &mut self,
        provider: Provider,
        phase: &str,
    ) -> io::Result<NativeProof> {
        self.action("provider-create-dashboard", "NavSessions", BTreeMap::new())
            .await?;
        self.action(
            "provider-create-private-inventory",
            "ViewSessions",
            BTreeMap::new(),
        )
        .await?;
        let inventory = self.stock.inventory().await?;
        let mut servers = inventory.servers.iter().filter(|server| {
            server.name.0 == self.stock.name && server.connectivity == ServerConnectivity::Online
        });
        let server = servers
            .next()
            .ok_or_else(|| failure("physical exact new stock server is not online"))?;
        if servers.next().is_some()
            || !(if provider == Provider::Omp {
                server.can_create.omp
            } else {
                server.can_create.codex
            })
        {
            return Err(prerequisite(
                "physical actual selected provider creation is not admitted; no create is submitted",
            ));
        }
        let server_id = server.server_id.0.clone();
        let project_label = self.stock.config.projects[0].label.0.clone();
        self.action("provider-create-form", "OpenNewSession", BTreeMap::new())
            .await?;
        if provider == Provider::Codex {
            self.action(
                "provider-create-codex",
                "SelectCodexProvider",
                BTreeMap::new(),
            )
            .await?;
        }
        let mut server = BTreeMap::new();
        server.insert("ownedUiCreationServerId".to_owned(), server_id.clone());
        self.action(
            "provider-server-picker",
            "SelectOwnedCreationServer",
            server.clone(),
        )
        .await?;
        self.action(
            "provider-server-choice",
            "ContinueOwnedCreationServer",
            server,
        )
        .await?;
        let mut project = BTreeMap::new();
        project.insert(
            "ownedUiCreationProjectLabel".to_owned(),
            project_label.clone(),
        );
        self.action(
            "provider-project-picker",
            "SelectOwnedCreationProject",
            project.clone(),
        )
        .await?;
        self.action(
            "provider-project-choice",
            "ContinueOwnedCreationProject",
            project,
        )
        .await?;
        let name_pin = digest_text(&format!("{}|{phase}|{:?}", self.run.0, provider));
        let name = format!(
            "Goatr phone {} {}",
            if provider == Provider::Omp {
                "OMP"
            } else {
                "Codex"
            },
            &name_pin[..8]
        );
        let mut naming = BTreeMap::new();
        naming.insert("ownedUiCreationName".to_owned(), name.clone());
        self.action("provider-create-name", "SetCreationName", naming.clone())
            .await?;
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        extend(
            &mut naming,
            &[
                ("ownedUiCreationServerLabel", "Configured Herdr"),
                ("ownedUiCreationServerId", &server_id),
                ("ownedUiCreationProjectLabel", &project_label),
                (
                    "ownedUiExcludedReceiptIds",
                    &serde_json::to_string(&before).map_err(io::Error::other)?,
                ),
            ],
        );
        self.action("provider-create-submit-once", "SubmitCreation", naming)
            .await?;
        let receipt = self
            .new_receipt(
                "provider-create-native-receipt",
                &before,
                "SessionCreate",
                None,
                None,
            )
            .await?;
        let target = Target {
            session_id: Uuid::new(string(&receipt, "createdSession")?).map_err(io::Error::other)?,
            generation: Uuid::new(string(&receipt, "createdGeneration")?)
                .map_err(io::Error::other)?,
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        let session = loop {
            let inventory = self.stock.inventory().await?;
            if let Some(session) = inventory.sessions.into_iter().find(|session| {
                session.session_id == target.session_id
                    && session.generation == target.generation
                    && session.provider == provider
                    && session.connectivity == SessionConnectivity::Ready
            }) {
                break session;
            }
            if Instant::now() >= deadline {
                return Err(prerequisite(
                    "physical successful create did not reach its exact actual provider/TUI readiness; retained receipt is not retried",
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        };
        let proof = self.stock.provider_proof(&session).await?;
        let process = proof.tui.as_ref().unwrap_or(&proof.owner);
        let cwd = fs::metadata(format!("/proc/{}/cwd", process.pid))?;
        let project = fs::metadata(&self.stock.config.projects[0].path)?;
        if (cwd.dev(), cwd.ino()) != (project.dev(), project.ino()) {
            return Err(failure(
                "physical created actual provider is not in the exact private configured project",
            ));
        }
        self.record(
            &format!("provider-{phase}-{:?}", provider),
            &proof_facts(&proof)?,
        )?;
        self.human_stage("provider-create-dismiss","Dismiss ONLY this completed private Goatr creation/receipt view without resubmitting or starting a separate action. Keep the created actual native agent/TUI running.").await?;
        Ok(proof)
    }
    async fn chat(&mut self, proof: &NativeProof, phase: &str) -> io::Result<()> {
        self.verify_provider(proof).await?;
        let marker = self.marker(proof.provider, phase);
        let text = format!("Reply only with {marker}");
        let prior = self.observer.snapshot(Some(&marker)).await?;
        original_session(&prior, &proof.target)?;
        if prior["conversation"]["userMarkerCount"] != 0 {
            return Err(failure(
                "physical unique public prompt marker already exists; never repeat it",
            ));
        }
        let mut arguments = target_arguments(&proof.target);
        arguments.insert("ownedUiComposerText".to_owned(), text);
        self.action("chat-public-composer", "SetComposer", arguments.clone())
            .await?;
        let before = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.action("chat-public-submit-once", "SubmitPrompt", arguments)
            .await?;
        let receipt = self
            .new_receipt(
                "chat-native-handoff-receipt",
                &before,
                "PromptSubmit",
                Some(&proof.target),
                None,
            )
            .await?;
        self.wait_snapshot(
            "chat-actual-structured-history",
            Some(&marker),
            Duration::from_secs(180),
            |snapshot| {
                snapshot["selectedSession"] == proof.target.session_id.0
                    && snapshot["conversation"]["userMarkerCount"] == 1
                    && snapshot["conversation"]["assistantMarkerCount"]
                        .as_u64()
                        .is_some_and(|count| count > 0)
            },
        )
        .await?;
        self.recheck()?;
        let native = self
            .stock
            .phone
            .open(&proof.target, "physical_ui_prompt_native_history")
            .await?;
        self.recheck()?;
        let user = shared_user_record(&native.history, &marker)?
            .ok_or_else(|| {
                failure("physical UI prompt is absent from actual same-native structured history")
            })?
            .id
            .clone();
        if !native.history.iter().any(|record| {
            record.complete
                && record.role == Role::Assistant
                && matches!(&record.text,Some(Content::Text(text)) if text.text.0.contains(&marker))
        }) {
            return Err(failure(
                "physical actual same-native assistant reply is absent; a local UI echo/receipt is not acceptance",
            ));
        }
        drop(native);
        self.verify_provider(proof).await?;
        self.record(&format!("chat-{phase}-{:?}",proof.provider),&json!({"target":proof.target,"publicMarkerSha256":digest_text(&marker),"nativeUserRecord":user,
            "nativeAssistantReplyObserved":true,"nativeUserCount":1,"receipt":receipt,"originalBinding":proof_facts(proof)?}))?;
        Ok(())
    }
    async fn history(&mut self, proof: &NativeProof) -> io::Result<()> {
        self.human_stage("provider-genuine-history","In this SAME private Goatr Chat, make enough harmless real turns to expose Load older. Page back using the actual Goatr history control and leave that older view visible. Do not populate data through a helper, SQLite or another conversation.").await?;
        let android = self
            .wait_snapshot(
                "provider-android-older-view",
                None,
                Duration::from_secs(45),
                |snapshot| {
                    snapshot["selectedSession"] == proof.target.session_id.0
                        && snapshot["conversation"]["browsingOlder"] == true
                        && !snapshot["conversation"]["viewId"].is_null()
                },
            )
            .await?;
        self.verify_provider(proof).await?;
        let ResultBody::SessionHistory(head) = self
            .stock
            .phone
            .rpc(
                RequestBody::SessionHistory(SessionHistoryParams {
                    target: proof.target.clone(),
                    view_id: None,
                    cursor: None,
                    limit: Some(Uint(8)),
                }),
                "physical_native_history_head",
            )
            .await?
        else {
            return Err(failure(
                "physical actual native history head result is absent",
            ));
        };
        let source = if proof.provider == Provider::Omp {
            RecordSource::Persisted
        } else {
            RecordSource::NativePage
        };
        let mut seen = HashSet::new();
        let mut boundary = shared_history_page(&head, &head.view_id, source, None, &mut seen)?;
        let mut cursor = head.next_cursor;
        let mut pages = 0;
        let mut older = Vec::new();
        for _ in 0..16 {
            let Some(next) = cursor else { break };
            let ResultBody::SessionHistory(page) = self
                .stock
                .phone
                .rpc(
                    RequestBody::SessionHistory(SessionHistoryParams {
                        target: proof.target.clone(),
                        view_id: Some(head.view_id.clone()),
                        cursor: Some(next),
                        limit: Some(Uint(8)),
                    }),
                    "physical_native_history_older",
                )
                .await?
            else {
                return Err(failure(
                    "physical actual native older history result is absent",
                ));
            };
            boundary = shared_history_page(&page, &head.view_id, source, boundary, &mut seen)?;
            if !page.records.is_empty() {
                pages += 1;
                older.extend(page.records.iter().map(|record| record.id.0.clone()));
            }
            cursor = page.next_cursor;
            if pages >= 2 {
                break;
            }
        }
        let displayed = array(&android["conversation"], "historyIds", 4096)?;
        if pages < 2
            || !displayed.iter().any(|id| {
                id.as_str()
                    .is_some_and(|id| older.iter().any(|older| older == id))
            })
        {
            return Err(prerequisite(
                "physical Android older view did not match two genuine same-native older pages; no narrowed mock history claim",
            ));
        }
        self.record(&format!("history-{:?}",proof.provider),&json!({"target":proof.target,"nativeView":head.view_id,"genuineOlderPages":pages,"source":source,"records":seen.len(),"androidOlderIdsMatchNative":true}))?;
        self.human_stage("provider-return-latest","Return THIS Goatr conversation to its actual latest view without resending any prompt or replaying a receipt.").await?;
        self.wait_snapshot(
            "provider-latest-restored",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["selectedSession"] == proof.target.session_id.0
                    && snapshot["conversation"]["browsingOlder"] == false
            },
        )
        .await?;
        Ok(())
    }
    async fn options_queue(&mut self, proof: &NativeProof) -> io::Result<()> {
        self.human_stage("provider-open-native-options","Open this SAME private conversation's real native Options panel; wait for its offered controls to load. Do not change a daily session.").await?;
        let before = self
            .wait_snapshot(
                "provider-native-options-loaded",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    !snapshot["conversation"]["optionsSha256"].is_null()
                        && snapshot["conversation"]["optionsLoading"] == false
                },
            )
            .await?;
        let ids = receipt_ids(&before)?;
        let old = before["conversation"]["optionsSha256"].clone();
        self.human_stage("provider-apply-native-option-once","Select ONE genuinely different offered mode/effort/model for THIS original native agent and tap Apply ONCE. Leave the Options panel open until its actual refreshed current value is visible.").await?;
        self.new_receipt(
            "provider-native-options-receipt",
            &ids,
            "SessionControlsApply",
            Some(&proof.target),
            None,
        )
        .await?;
        let applied = self
            .wait_snapshot(
                "provider-native-option-changed",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    !snapshot["conversation"]["optionsSha256"].is_null()
                        && snapshot["conversation"]["optionsSha256"] != old
                        && snapshot["conversation"]["optionsLoading"] == false
                        && snapshot["conversation"]["optionsPending"] == false
                },
            )
            .await?;
        self.verify_provider(proof).await?;
        if !provider_controls_admission(proof.provider, &applied["conversation"])? {
            let ids = receipt_ids(&applied)?;
            for delivery in ["Steer", "FollowUp", "Aside", "Native"] {
                self.human_stage("omp-offered-delivery-control",&format!("Select the actually offered {delivery} delivery control for ONLY this original OMP conversation. Leave it selected and Options open. Do not submit text, claim queue inspection, or touch another host.")).await?;
                let selected = self
                    .wait_snapshot(
                        "omp-real-offered-delivery-selected",
                        None,
                        Duration::from_secs(30),
                        |snapshot| {
                            snapshot["conversation"]["delivery"] == delivery
                                && !snapshot["conversation"]["optionsSha256"].is_null()
                        },
                    )
                    .await?;
                if provider_controls_admission(proof.provider, &selected["conversation"])?
                    || receipt_ids(&selected)? != ids
                {
                    return Err(failure(
                        "physical OMP local delivery selection changed native actions or claimed unsupported queue inspection",
                    ));
                }
                self.record(&format!("omp-offered-{delivery}"),&json!({"target":proof.target,"offeredDeliveries":selected["conversation"]["offeredDeliveries"],"selected":delivery,"nativeQueueInspectionEditing":"unavailable","localControlOnly":true}))?;
            }
            self.human_stage("omp-options-close","Close ONLY this original OMP Options panel, retaining Native delivery. Queue inspection/editing is genuinely unsupported and is not a passed queue CRUD stage.").await?;
            self.wait_snapshot(
                "omp-native-delivery-restored",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["conversation"]["delivery"] == "Native"
                        && snapshot["conversation"]["optionsSha256"].is_null()
                },
            )
            .await?;
            self.verify_provider(proof).await?;
            return Ok(());
        }
        self.human_stage("provider-open-native-queue","Open the actual native queue from this same Options panel. It must be supported and initially empty; do not adopt an existing queued item.").await?;
        self.wait_snapshot(
            "provider-native-queue-empty",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["conversation"]["queueAvailable"] == true
                    && snapshot["conversation"]["queueLoaded"] == true
                    && snapshot["conversation"]["queueCount"] == 0
            },
        )
        .await?;
        let marker = self.marker(proof.provider, "queue");
        let ids = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.human_stage("provider-queue-public-entry",&format!("Use this Goatr conversation's offered Queue delivery to submit ONE harmless public text `{marker}`. Reopen its real native queue and leave it visible. Do not start or replay the queued item.")).await?;
        self.new_receipt(
            "provider-queue-native-submit",
            &ids,
            "PromptSubmit",
            Some(&proof.target),
            None,
        )
        .await?;
        let queued = self
            .wait_snapshot(
                "provider-real-queued-entry",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["conversation"]["queueLoaded"] == true
                        && snapshot["conversation"]["queueCount"] == 1
                        && !snapshot["conversation"]["queueSha256"].is_null()
                },
            )
            .await?;
        let ids = receipt_ids(&queued)?;
        let hash = queued["conversation"]["queueSha256"].clone();
        self.human_stage("provider-queue-edit-once","Edit ONLY that newly owned public native queue entry through Goatr, adding the public word EDITED, and tap its Save/Update ONCE. Leave the actual native queue visible.").await?;
        self.new_receipt(
            "provider-queue-native-update",
            &ids,
            "SessionQueueUpdate",
            Some(&proof.target),
            None,
        )
        .await?;
        let edited = self
            .wait_snapshot(
                "provider-real-queue-edit",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["conversation"]["queueLoaded"] == true
                        && snapshot["conversation"]["queueCount"] == 1
                        && snapshot["conversation"]["queueSha256"] != hash
                },
            )
            .await?;
        let ids = receipt_ids(&edited)?;
        self.human_stage("provider-queue-delete-owned-once","Delete ONLY that owned new public native queue entry through Goatr ONCE. Do not clear or alter any other queue.").await?;
        self.new_receipt(
            "provider-queue-native-delete",
            &ids,
            "SessionQueueDelete",
            Some(&proof.target),
            None,
        )
        .await?;
        self.wait_snapshot(
            "provider-real-queue-empty-again",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["conversation"]["queueLoaded"] == true
                    && snapshot["conversation"]["queueCount"] == 0
            },
        )
        .await?;
        self.human_stage("provider-options-close","Restore THIS conversation's prompt delivery to Native and close its Options/Queue panel. Do not reset a global preference or another host.").await?;
        self.wait_snapshot(
            "provider-native-delivery-restored",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["conversation"]["delivery"] == "Native"
                    && snapshot["conversation"]["optionsSha256"].is_null()
            },
        )
        .await?;
        self.verify_provider(proof).await?;
        Ok(())
    }
    async fn request_and_stop(&mut self, proof: &NativeProof) -> io::Result<()> {
        self.human_stage("provider-real-request","Through THIS owned actual agent's ordinary Goatr Chat/native TUI, cause ONE genuine supported select/input/editor/confirm/form/permission question. Leave it unresolved for actual Goatr form observation; do not inject a dialog or use a fake tool/provider.").await?;
        let pending = self
            .wait_snapshot(
                "provider-real-pending-request",
                None,
                Duration::from_secs(120),
                |snapshot| {
                    snapshot["selectedSession"] == proof.target.session_id.0
                        && snapshot["pending"]
                            .as_array()
                            .is_some_and(|pending| pending.len() == 1)
                },
            )
            .await?;
        let request = &array(&pending, "pending", 4096)?[0];
        let key = string(request, "request")?.to_owned();
        let kind = string(request, "kind")?.to_owned();
        let mut arguments = target_arguments(&proof.target);
        arguments.insert("ownedUiRequestId".to_owned(), key.clone());
        self.action(
            "provider-select-original-request",
            "SelectRequest",
            arguments.clone(),
        )
        .await?;
        let ids = receipt_ids(&self.observer.snapshot(None).await?)?;
        if kind == "OmpSelect" {
            arguments.insert("ownedUiRequestAnswer".to_owned(), "SelectFirst".to_owned());
            self.action(
                "provider-select-answer-once",
                "SubmitRequestAnswer",
                arguments,
            )
            .await?;
        } else if ["OmpInput", "OmpEditor", "CodexForm"].contains(&kind.as_str()) {
            self.human_stage("provider-prepare-supported-form","Fill ONLY this original supported Goatr form with harmless public owned-fixture answers. Do NOT tap Submit; the next signed action submits it once.").await?;
            arguments.insert(
                "ownedUiRequestAnswer".to_owned(),
                "SubmitPrepared".to_owned(),
            );
            self.action(
                "provider-submit-prepared-form-once",
                "SubmitRequestAnswer",
                arguments,
            )
            .await?;
        } else {
            self.human_stage("provider-answer-offered-form-once","Answer this original Goatr supported confirm/permission question ONCE using one actually offered native form control. Unsupported forms must remain desktop-only; do not invent a choice or secret answer.").await?;
        }
        self.new_receipt(
            "provider-real-request-answer-receipt",
            &ids,
            "RequestRespond",
            Some(&proof.target),
            None,
        )
        .await?;
        self.wait_snapshot(
            "provider-original-request-resolved",
            None,
            Duration::from_secs(60),
            |snapshot| {
                snapshot["pending"]
                    .as_array()
                    .is_some_and(|pending| pending.iter().all(|request| request["request"] != key))
            },
        )
        .await?;
        self.verify_provider(proof).await?;
        self.human_stage("provider-real-working-turn","In this SAME Goatr Chat, start one harmless bounded longer reply/task and leave it actively working long enough to press Goatr Stop. Do not kill a process or use desktop Ctrl-C as a substitute.").await?;
        let working = self
            .wait_snapshot(
                "provider-real-working",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["selectedSession"] == proof.target.session_id.0
                        && snapshot["conversation"]["activity"] == "Working"
                },
            )
            .await?;
        let ids = receipt_ids(&working)?;
        self.action(
            "provider-stop-once",
            "StopSession",
            target_arguments(&proof.target),
        )
        .await?;
        self.new_receipt(
            "provider-native-stop-receipt",
            &ids,
            "TurnCancel",
            Some(&proof.target),
            None,
        )
        .await?;
        self.wait_snapshot(
            "provider-native-stop-settled",
            None,
            Duration::from_secs(60),
            |snapshot| {
                snapshot["conversation"]["activity"] == "Idle"
                    && snapshot["selectedSession"] == proof.target.session_id.0
            },
        )
        .await?;
        self.verify_provider(proof).await?;
        Ok(())
    }
    async fn passive_terminal(&mut self, proof: &NativeProof) -> io::Result<()> {
        let native = self.stock.native_snapshot().await?;
        self.action(
            "agent-open-passive-terminal",
            "OpenTerminal",
            target_arguments(&proof.target),
        )
        .await?;
        let first = self
            .wait_snapshot(
                "agent-real-styled-ansi",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["agentTerminal"] == true
                        && snapshot["agentDisplay"]["present"] == true
                        && snapshot["agentDisplay"]["styled"] == true
                },
            )
            .await?;
        self.action(
            "agent-terminal-readonly-controls",
            "ObserveTerminal",
            target_arguments(&proof.target),
        )
        .await?;
        self.action(
            "agent-close-viewer-only",
            "CloseTerminal",
            target_arguments(&proof.target),
        )
        .await?;
        self.chat(proof, "post-terminal").await?;
        self.action(
            "agent-reopen-passive-terminal",
            "OpenTerminal",
            target_arguments(&proof.target),
        )
        .await?;
        self.wait_snapshot(
            "agent-changing-decoded-ansi",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["agentTerminal"] == true
                    && snapshot["agentDisplay"]["styled"] == true
                    && snapshot["agentDisplay"]["sha256"] != first["agentDisplay"]["sha256"]
            },
        )
        .await?;
        self.action(
            "agent-terminal-still-readonly",
            "ObserveTerminal",
            target_arguments(&proof.target),
        )
        .await?;
        self.action(
            "agent-dismiss-viewer-only",
            "CloseTerminal",
            target_arguments(&proof.target),
        )
        .await?;
        let after = self.stock.native_snapshot().await?;
        if native.layouts != after.layouts
            || native.focused_pane_id != after.focused_pane_id
            || native.focused_tab_id != after.focused_tab_id
            || native.focused_workspace_id != after.focused_workspace_id
        {
            return Err(failure(
                "physical passive Agent Terminal changed actual native focus/layout geometry",
            ));
        }
        self.verify_provider(proof).await?;
        self.record(&format!("passive-agent-terminal-{:?}",proof.provider),&json!({"target":proof.target,"changingStyledDecodedAnsi":true,"inputOrResizeOrFocusActions":0,
            "nativeLayoutsAndFocusUnchanged":true,"rawAnsiOrCellsExported":false,"nativeCellOrAlternateHistoryParityClaim":false}))?;
        Ok(())
    }
    async fn provider_workflow(&mut self) -> io::Result<Vec<NativeProof>> {
        self.human_stage("shell-view-dismiss","Dismiss ONLY the released Goatr shell viewer and return to its list. Keep both created ordinary native shells running.").await?;
        self.wait_snapshot(
            "shell-viewer-dismissed",
            None,
            Duration::from_secs(30),
            |snapshot| snapshot["selectedShell"].is_null() && snapshot["oldInputRetired"] == true,
        )
        .await?;
        // First create two owned product fixtures, then reopen them as existing native agents.
        let mut originals = Vec::with_capacity(4);
        for provider in [Provider::Omp, Provider::Codex] {
            originals.push(self.create_provider(provider, "existing").await?);
        }
        for proof in &originals {
            self.open_provider(proof).await?;
            self.chat(proof, "existing").await?;
            self.history(proof).await?;
            self.options_queue(proof).await?;
            self.request_and_stop(proof).await?;
            self.passive_terminal(proof).await?;
        }
        for provider in [Provider::Omp, Provider::Codex] {
            let proof = self.create_provider(provider, "created").await?;
            self.open_provider(&proof).await?;
            self.chat(&proof, "created").await?;
            self.action(
                "created-agent-passive-terminal",
                "OpenTerminal",
                target_arguments(&proof.target),
            )
            .await?;
            self.wait_snapshot(
                "created-agent-real-ansi",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["agentTerminal"] == true && snapshot["agentDisplay"]["present"] == true
                },
            )
            .await?;
            self.action(
                "created-agent-readonly",
                "ObserveTerminal",
                target_arguments(&proof.target),
            )
            .await?;
            self.action(
                "created-agent-dismiss-viewer",
                "CloseTerminal",
                target_arguments(&proof.target),
            )
            .await?;
            originals.push(proof);
        }
        Ok(originals)
    }
}

impl Matrix {
    async fn open_controlled_shell(&mut self, shell: &ShellSummary) -> io::Result<Value> {
        self.action("mixed-shell-dashboard", "NavSessions", BTreeMap::new())
            .await?;
        self.action("mixed-shell-list", "OpenShells", BTreeMap::new())
            .await?;
        self.action(
            "mixed-shell-original",
            "OpenShell",
            shell_arguments(&shell.target),
        )
        .await?;
        let snapshot = self
            .wait_snapshot(
                "mixed-original-shell-metadata-or-existing-admission",
                None,
                Duration::from_secs(30),
                |snapshot| shell_admission(snapshot, &shell.target).is_some(),
            )
            .await?;
        if shell_admission(&snapshot, &shell.target) == Some(ShellAdmission::Metadata) {
            self.action(
                "mixed-shell-begin",
                "BeginShellControl",
                shell_arguments(&shell.target),
            )
            .await?;
            self.action(
                "mixed-shell-confirm",
                "ConfirmShellControl",
                shell_arguments(&shell.target),
            )
            .await?;
        }
        self.wait_snapshot(
            "mixed-active-original-committed-grid",
            None,
            Duration::from_secs(30),
            |snapshot| shell_admission(snapshot, &shell.target) == Some(ShellAdmission::Committed),
        )
        .await
    }
    async fn mixed_workflow(
        &mut self,
        shell: &ShellSummary,
        sibling: &ShellSummary,
        providers: &[NativeProof],
    ) -> io::Result<()> {
        self.open_controlled_shell(shell).await?;
        self.human_stage("mixed-first-tap-shell",&format!("Tap the mixed strip's OTHER ordinary shell {} ONCE. Do not tap twice to repair selection, send input or select a retained host.",sibling.target.shell_id.0)).await?;
        let switched = self
            .wait_snapshot(
                "mixed-first-tap-shell-effect",
                None,
                Duration::from_secs(30),
                |snapshot| {
                    snapshot["selectedShell"] == sibling.target.shell_id.0
                        && snapshot["oldInputRetired"] == true
                },
            )
            .await?;
        let prior = receipt_ids(&switched)?;
        let mut stale = shell_arguments(&shell.target);
        stale.insert("ownedUiKey".to_owned(), "Left".to_owned());
        self.invoke_action(
            "mixed-genuine-old-shell-refusal",
            "SendShellKey",
            stale,
            UiExpectation::StaleRefusal,
        )
        .await?;
        let refused = self.observer.snapshot(None).await?;
        if receipt_ids(&refused)? != prior || refused["selectedShell"] != sibling.target.shell_id.0
        {
            return Err(failure(
                "physical stale original shell gesture changed current selection or allocated an input action",
            ));
        }
        self.record("mixed-stale-scope-zero-input",&json!({"old":shell.target,"current":sibling.target,"newGoatrInputReceipts":0,"gestureAcknowledged":false,"nativeKernelInstrumentationClaim":false}))?;
        self.human_stage("mixed-first-tap-agent",&format!("Tap the original owned agent {} in the mixed strip ONCE. Do not resubmit, answer or Stop a turn.",providers[0].target.session_id.0)).await?;
        self.wait_snapshot(
            "mixed-first-tap-agent-effect",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["selectedSession"] == providers[0].target.session_id.0
                    && snapshot["selectedGeneration"] == providers[0].target.generation.0
                    && snapshot["selectedShell"].is_null()
                    && snapshot["oldInputRetired"] == true
            },
        )
        .await?;
        self.open_controlled_shell(shell).await?;
        self.human_stage("mixed-real-ime-visible","Open the real phone IME in this controlled Goatr shell without entering/sending bytes. Leave it visible.").await?;
        let keyboard = self
            .wait_snapshot(
                "mixed-real-ime-observed",
                None,
                Duration::from_secs(30),
                |snapshot| snapshot["ui"]["imeVisible"] == true,
            )
            .await?;
        let baseline = self
            .baseline_ui
            .clone()
            .ok_or_else(|| failure("physical original display baseline was not captured"))?;
        self.human_stage("mixed-rotate-larger-text","Rotate the physical phone to the OTHER orientation and temporarily enlarge text through normal Android UI. Return to THIS same Goatr shell with the real IME visible. Visually check the remaining-height canvas/key dock is usable; do not disable FLAG_SECURE or take terminal screenshots.").await?;
        self.wait_snapshot(
            "mixed-actual-rotation-font-ime",
            None,
            Duration::from_secs(45),
            |snapshot| {
                snapshot["ui"]["orientation"] != baseline["orientation"]
                    && snapshot["ui"]["fontScale"].as_f64() > baseline["fontScale"].as_f64()
                    && snapshot["ui"]["imeVisible"] == true
                    && snapshot["oldInputRetired"] == true
                    && snapshot["selectedShell"] == shell.target.shell_id.0
            },
        )
        .await?;
        self.action(
            "mixed-actual-shell-surface",
            "ObserveShell",
            shell_arguments(&shell.target),
        )
        .await?;
        self.human_stage("mixed-restore-display","Restore ONLY the temporary text-size/orientation changes you just made to their recorded original settings, close the real IME, and leave this same Goatr shell visible. Do not change theme, channel, retained-host or delivery choices.").await?;
        self.wait_snapshot(
            "mixed-display-restored",
            None,
            Duration::from_secs(45),
            |snapshot| {
                snapshot["ui"]["orientation"] == baseline["orientation"]
                    && snapshot["ui"]["fontScale"] == baseline["fontScale"]
                    && snapshot["ui"]["imeVisible"] == false
                    && snapshot["oldInputRetired"] == true
            },
        )
        .await?;
        self.record("mixed-native-window-disclosure",&json!({"before":keyboard["ui"],"restored":baseline,"physicalHumanCanvasDockInspection":true,"terminalScreenshots":false,"fullComposeCaptureInterleavingExecutionClaim":false}))?;
        self.human_stage("mixed-dismiss-shell-view","Dismiss ONLY this Goatr viewer back to its list without closing a native terminal or restarting a process.").await?;
        let before = self.stock.native_snapshot().await?;
        self.human_stage("mixed-stock-layout","On the ORIGINAL owned stock Herdr UI, change the existing pane split ratio/layout and focus a DIFFERENT already-owned pane. Do not create/adopt another session, close a pane/workspace, restart Herdr or touch daily native work. Leave all original providers/shells alive.").await?;
        let after = self.stock.native_snapshot().await?;
        if before.layouts == after.layouts || before.focused_pane_id == after.focused_pane_id {
            return Err(failure(
                "physical actual installed stock layout/focus transition was not observed",
            ));
        }
        self.record("mixed-real-native-layout",&json!({"layoutChanged":true,"focusedPaneBefore":before.focused_pane_id,"focusedPaneAfter":after.focused_pane_id,"nativeCloseOrRestart":false}))?;
        for provider in providers {
            self.verify_provider(provider).await?;
        }
        let active = self.open_controlled_shell(shell).await?;
        let receipt=array(&active,"receipts",4096)?.iter().find(|receipt|receipt["method"]=="ShellInput"&&receipt["state"]=="Dispatched").cloned().ok_or_else(||failure("physical original dispatched shell receipt is unavailable for reconnect inspection"))?;
        self.human_stage("mixed-network-off","Temporarily disconnect the PHONE's data networks through normal Android UI while USB signed control remains attached. Do not stop Goatr, kill a service or change the desktop network. Leave data disconnected for observation.").await?;
        self.wait_snapshot(
            "mixed-real-disconnect-owner-retired",
            None,
            Duration::from_secs(45),
            |snapshot| snapshot["hello"] == false && snapshot["oldInputRetired"] == true,
        )
        .await?;
        self.human_stage("mixed-network-return","Restore the phone's previous data network and return to this SAME Goatr app without force-stop/reinstall/re-pair. Do not resend any input.").await?;
        self.wait_snapshot(
            "mixed-real-fresh-hello",
            None,
            Duration::from_secs(60),
            |snapshot| {
                snapshot["foreground"] == true
                    && snapshot["hello"] == true
                    && snapshot["inventoryReady"] == true
                    && snapshot["mutationReady"] == true
            },
        )
        .await?;
        self.human_stage("mixed-retained-receipt-inspection","Open Goatr's actual receipt/details for an original Dispatched shell-input action and inspect it WITHOUT Retry/Separate/Send. Return to the ordinary app scene when done.").await?;
        let current = self.observer.snapshot(None).await?;
        if !array(&current, "receipts", 4096)?.iter().any(|current| {
            current["action"] == receipt["action"]
                && current["sha256"] == receipt["sha256"]
                && current["state"] == "Dispatched"
        }) {
            return Err(failure(
                "physical original receipt/fingerprint was not retained across actual reconnect",
            ));
        }
        self.record("mixed-reconnect-retained-receipt",&json!({"action":receipt["action"],"sha256":receipt["sha256"],"sameReceipt":true,"inputReplay":false,"replacementPidAdopted":false}))?;
        Ok(())
    }
    async fn independent_network(
        &mut self,
        shell: &ShellSummary,
        provider: &NativeProof,
    ) -> io::Result<()> {
        self.human_stage("independent-network-switch","Manually move the PHONE from LAN/Wi-Fi to validated cellular/independent mobile data. Turn off Wi-Fi, VPN, Ethernet/USB tether data paths; retain USB ONLY for signed ADB control. Return to this same Goatr app. Do not create an ADB data-plane tunnel.").await?;
        self.wait_snapshot(
            "independent-actual-iroh-route",
            None,
            Duration::from_secs(90),
            |snapshot| {
                let network = &snapshot["network"];
                network["internet"] == true
                    && network["validated"] == true
                    && network["cellular"] == true
                    && network["wifi"] == false
                    && network["ethernet"] == false
                    && network["vpn"] == false
                    && network["usb"] == false
                    && snapshot["hello"] == true
                    && snapshot["inventoryReady"] == true
                    && snapshot["mutationReady"] == true
                    && matches!(snapshot["route"].as_str(), Some("direct" | "relay"))
            },
        )
        .await?;
        self.adb.tunnels(Some(self.control_port)).await?;
        self.open_controlled_shell(shell).await?;
        let marker = format!("GOATR_MVP_{}", self.run.0);
        let before = self.observer.snapshot(Some(&marker)).await?;
        if before["markerOutput"] == true {
            return Err(failure(
                "physical network shell echo is already visible; cannot use old output as a new remote-control witness",
            ));
        }
        self.action(
            "independent-shell-public-draft",
            "SetShellDraft",
            shell_arguments(&shell.target),
        )
        .await?;
        let ids = receipt_ids(&self.observer.snapshot(None).await?)?;
        self.action(
            "independent-shell-public-send",
            "SendShellDraft",
            shell_arguments(&shell.target),
        )
        .await?;
        self.new_receipt(
            "independent-shell-native-send",
            &ids,
            "ShellInput",
            None,
            Some(&shell.target),
        )
        .await?;
        if self.observer.snapshot(Some(&marker)).await?["markerOutput"] == true {
            return Err(failure(
                "physical Draft/Send added an implicit Enter before the separate gesture",
            ));
        }
        self.shell_key("independent-shell-separate-enter", shell, "Enter")
            .await?;
        self.wait_snapshot(
            "independent-real-shell-output",
            Some(&marker),
            Duration::from_secs(30),
            |snapshot| snapshot["markerOutput"] == true,
        )
        .await?;
        self.action(
            "independent-release-shell",
            "ReleaseShellControl",
            shell_arguments(&shell.target),
        )
        .await?;
        self.human_stage("independent-dismiss-shell-view","Dismiss ONLY the released Goatr shell viewer, keeping the ordinary native shell running on this independent network.").await?;
        self.open_provider(provider).await?;
        self.chat(provider, "network").await?;
        self.record("independent-network-real-effects",&json!({"actualIrohRoute":self.observer.snapshot(None).await?["route"],"phoneCellularObserved":true,
            "freshNativePromptAndShellOutput":true,"adbDataPlaneTunnel":false,"controlOnlyReversePort":self.control_port}))?;
        Ok(())
    }
}

fn health_ready(keeper: &Value) -> bool {
    let health = &keeper["health"];
    health["globalAvailable"] == true
        && health["runtimePermissionRequired"] == false
        && health["attentionChannelEnabled"] == true
        && health["deliveryChannelEnabled"] == true
}
fn socket_free(keeper: &Value) -> bool {
    keeper["running"] == false
        && keeper["reconnecting"] == 0
        && keeper["foregroundNotification"] == false
        && keeper["endpointOwned"] == false
        && keeper["networkCallback"] == false
        && keeper["privateEnabled"] == true
        && keeper["privateMode"] == "Ntfy"
        && keeper["privateVerified"] == true
        && keeper["privateRequiresKeeper"] == false
        && keeper["retained"]
            .as_array()
            .is_some_and(|hosts| hosts.iter().all(|host| host["requiresKeeper"] == false))
}
impl Matrix {
    async fn wait_keeper(
        &mut self,
        stage: &'static str,
        deadline: Duration,
        predicate: impl Fn(&Value) -> bool,
    ) -> io::Result<Value> {
        self.last_stage = stage;
        let until = Instant::now() + deadline;
        loop {
            self.recheck()?;
            let facts = self.observer.keeper().await?;
            self.recheck()?;
            if predicate(&facts) {
                self.record(stage, &facts)?;
                return Ok(facts);
            }
            if Instant::now() >= until {
                return Err(failure(
                    "physical actual keeper/registration/channel effect did not become proved; no forced cleanup or replay",
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    async fn private_settings(&mut self) -> io::Result<()> {
        self.action("delivery-private-dashboard", "NavSessions", BTreeMap::new())
            .await?;
        self.action(
            "delivery-private-settings",
            "OpenHostSettings",
            BTreeMap::new(),
        )
        .await?;
        self.wait_snapshot(
            "delivery-original-settings-scene",
            None,
            Duration::from_secs(30),
            |snapshot| {
                snapshot["selectedHost"] == snapshot["host"] && snapshot["foreground"] == true
            },
        )
        .await?;
        Ok(())
    }
    async fn notifications(&mut self, after: Option<u64>) -> io::Result<NotificationsSyncResult> {
        self.recheck()?;
        let result = self
            .stock
            .phone
            .rpc(
                RequestBody::NotificationsSync(NotificationsSyncParams {
                    after_sequence: after.map(Uint),
                    limit: Some(Uint(16)),
                }),
                "physical_readonly_notification_ledger",
            )
            .await?;
        self.recheck()?;
        match result {
            ResultBody::NotificationsSync(result) => Ok(result),
            _ => Err(failure(
                "physical actual notification ledger result is absent",
            )),
        }
    }
    async fn wait_event(
        &mut self,
        proof: &NativeProof,
        category: AttentionCategory,
        after: u64,
    ) -> io::Result<AttentionEvent> {
        let until = Instant::now() + Duration::from_secs(180);
        loop {
            let result = self.notifications(Some(after)).await?;
            if result.retained_from.0 > after.saturating_add(1) {
                return Err(failure(
                    "physical notification cursor crossed retention; never fabricate a missing event",
                ));
            }
            let mut matching = result.events.into_iter().filter(|event| {
                event.session_id == proof.target.session_id
                    && event.generation == proof.target.generation
                    && event.category == category
                    && event.sequence.0 > after
            });
            if let Some(event) = matching.next() {
                if matching.next().is_some() {
                    return Err(failure("physical native event attribution is ambiguous"));
                }
                return Ok(event);
            }
            if Instant::now() >= until {
                return Err(prerequisite(
                    "physical genuine owned native attention/completion did not arrive; no fake notification or direct helper prompt",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    async fn notification(&self, event: &AttentionEvent, hint: bool) -> io::Result<Value> {
        self.observer.query(json!({"kind":"notification","host":self.host,"event":event.event_id,"category":if event.category==AttentionCategory::Attention{"attention"}else{"completed"},"hint":hint}),"notification").await.map(|(value,_)|value)
    }
    async fn wait_notification(
        &mut self,
        event: &AttentionEvent,
        hint: bool,
        present: bool,
    ) -> io::Result<Value> {
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            let value = self.notification(event, hint).await?;
            let ledger = &value["ledger"];
            let seen = value["present"] == present
                && (!present
                    || value["qualified"] == true
                        && ledger["present"] == true
                        && ledger["sequence"] == event.sequence.0
                        && ledger["session"] == event.session_id.0
                        && !ledger["attemptedAt"].is_null());
            if seen {
                if present {
                    let state = self.observer.snapshot(None).await?;
                    if !background_noninteractive(&state) {
                        return Err(failure(
                            "physical posted delivery acceptance requires measured background and noninteractive screen state",
                        ));
                    }
                    self.record("delivery-posted-screen-state",&json!({"event":event.event_id,"hint":hint,"foreground":state["foreground"],"ui":state["ui"],"immediatelyBeforeAcceptance":true}))?;
                }
                return Ok(value);
            }
            if Instant::now() >= until {
                return Err(failure(
                    "physical exact app-owned notification/ledger metadata was not observed; no PendingIntent or blanket-dump substitute",
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    async fn delivery_matrix(
        &mut self,
        mode: &'static str,
        providers: &[NativeProof],
        generation: Option<&Value>,
    ) -> io::Result<Value> {
        let hint = mode == "ntfy";
        let mut events = Vec::with_capacity(4);
        let mut unique = BTreeSet::new();
        for proof in providers {
            for category in [AttentionCategory::Attention, AttentionCategory::Completed] {
                self.open_provider(proof).await?;
                self.wait_snapshot(
                    "delivery-original-idle",
                    None,
                    Duration::from_secs(60),
                    |snapshot| snapshot["conversation"]["activity"] == "Idle",
                )
                .await?;
                let cursor = self.notifications(None).await?.latest_sequence.0;
                self.action(
                    "delivery-background-original",
                    "BackgroundOwnedApp",
                    target_arguments(&proof.target),
                )
                .await?;
                self.wait_snapshot(
                    "delivery-actual-background",
                    None,
                    Duration::from_secs(30),
                    |snapshot| snapshot["foreground"] == false,
                )
                .await?;
                let keeper = if hint {
                    self.wait_keeper(
                        "ntfy-actual-socket-free-before-event",
                        Duration::from_secs(45),
                        |keeper| socket_free(keeper) && health_ready(keeper),
                    )
                    .await?
                } else {
                    self.wait_keeper(
                        "persistent-actual-shared-keeper",
                        Duration::from_secs(45),
                        |keeper| {
                            keeper["running"] == true
                                && keeper["foregroundNotification"] == true
                                && keeper["endpointOwned"] == true
                                && keeper["serviceRequested"] == true
                                && keeper["privateMode"] == "Persistent"
                                && keeper["privateEnabled"] == true
                                && keeper["privateCompleted"] == true
                                && health_ready(keeper)
                        },
                    )
                    .await?
                };
                keeper_retained_current(&keeper, self.first_open_ref()?)?;
                if let Some(generation) = generation
                    && keeper["privateRegistrationGeneration"] != *generation
                {
                    return Err(failure(
                        "physical socket-free matrix changed the original verified registration generation",
                    ));
                }
                let name = if category == AttentionCategory::Attention {
                    "attention"
                } else {
                    "completion"
                };
                self.human_stage("delivery-real-native-event",&format!("Keep Goatr BACKGROUND and turn the phone screen OFF. On the SAME ORIGINAL owned {:?} native TUI/session {}, cause ONE genuine {name}: {}. Do not open Goatr, inject a notification, use a helper RPC, or answer through a fake provider. Enter yes on this local TTY only after the native effect exists.",proof.provider,proof.native.0,
                    if category==AttentionCategory::Attention{"open a supported real pending question and leave it unresolved"}else{"finish one harmless real native task/turn"})).await?;
                let screen_off = self.observer.snapshot(None).await?;
                if !background_noninteractive(&screen_off) {
                    return Err(failure(
                        "physical screen-off human barrier lacks measured background/noninteractive facts",
                    ));
                }
                self.record("delivery-pre-event-screen-state",&json!({"eventCategory":category,"foreground":screen_off["foreground"],"ui":screen_off["ui"],"afterHumanBarrier":true}))?;
                let event = self.wait_event(proof, category, cursor).await?;
                if !unique.insert(event.event_id.0.clone()) {
                    return Err(failure("physical four-event matrix reused an old event"));
                }
                let posted = self.wait_notification(&event, hint, true).await?;
                self.verify_provider(proof).await?;
                self.record(&format!("{mode}-real-event"),&json!({"event":event,"ownedMetadata":posted,"preEventKeeper":keeper,"screenOffMeasuredAfterHumanBarrier":screen_off["ui"],"nativeBinding":proof_facts(proof)?}))?;
                self.human_stage("delivery-wake-unlock","Wake/unlock the physical phone WITHOUT opening Goatr, tapping/clearing the alert, changing a channel or stopping delivery. Leave the real SystemUI notification available.").await?;
                self.wait_snapshot(
                    "delivery-measured-awake-before-native-tap",
                    None,
                    Duration::from_secs(30),
                    |snapshot| {
                        snapshot["foreground"] == false && snapshot["ui"]["interactive"] == true
                    },
                )
                .await?;
                let before_ids = receipt_ids(&self.observer.snapshot(None).await?)?;
                self.action(
                    "delivery-open-native-shade",
                    "OpenAttentionShade",
                    BTreeMap::new(),
                )
                .await?;
                let mut arguments = target_arguments(&proof.target);
                extend(
                    &mut arguments,
                    &[
                        ("ownedUiEventId", &event.event_id.0),
                        (
                            "ownedUiNotificationCategory",
                            if category == AttentionCategory::Attention {
                                "Attention"
                            } else {
                                "Completed"
                            },
                        ),
                        (
                            "ownedUiNotificationHint",
                            if hint { "true" } else { "false" },
                        ),
                        ("ownedUiHostLabel", &self.label),
                    ],
                );
                let (_, _, ambiguous) = self
                    .invoke_action(
                        "delivery-exact-native-alert-tap",
                        "TapAttentionNotification",
                        arguments,
                        UiExpectation::NotificationTap { api: self.api },
                    )
                    .await?;
                if ambiguous {
                    self.human_stage("delivery-human-native-row-tap",&format!("The native SystemUI row was ambiguous and NO gesture was acknowledged. Tap the ONE actual Goatr alert for private host `{}` and this expected {:?} event {} ONCE. Do not use another row, coordinates, direct PendingIntent or retry an acknowledged tap.",self.label,category,event.event_id.0)).await?;
                }
                let opened = self
                    .wait_snapshot(
                        "delivery-authenticated-exact-post-tap",
                        None,
                        Duration::from_secs(60),
                        |snapshot| {
                            snapshot["foreground"] == true
                                && snapshot["hello"] == true
                                && snapshot["mutationReady"] == true
                                && snapshot["selectedHost"] == snapshot["host"]
                                && snapshot["selectedSession"] == proof.target.session_id.0
                                && snapshot["selectedGeneration"] == proof.target.generation.0
                        },
                    )
                    .await?;
                if receipt_ids(&opened)? != before_ids {
                    return Err(failure(
                        "physical notification tap allocated a submit/answer/Stop mutation; tap is navigation/sync only",
                    ));
                }
                let read = self.wait_notification(&event, hint, false).await?;
                let until = Instant::now() + Duration::from_secs(30);
                let mut read = read;
                while read["ledger"]["readThrough"].as_u64().unwrap_or(0) < event.sequence.0 {
                    if Instant::now() >= until {
                        return Err(failure(
                            "physical exact notification tap was not authoritatively read/suppressed",
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    read = self.notification(&event, hint).await?;
                }
                if read["ledger"]["attemptedAt"] != posted["ledger"]["attemptedAt"] {
                    return Err(failure(
                        "physical notification attempted presentation was repeated during tap/sync",
                    ));
                }
                // Reopen/sync from the normal product lifecycle, never replay a hint or invoke a PendingIntent.
                self.action(
                    "delivery-background-after-read",
                    "BackgroundOwnedApp",
                    target_arguments(&proof.target),
                )
                .await?;
                if hint {
                    self.wait_keeper(
                        "ntfy-socket-free-after-read",
                        Duration::from_secs(45),
                        socket_free,
                    )
                    .await?;
                }
                self.recheck_product().await?;
                launch_product(&self.adb).await?;
                self.wait_snapshot(
                    "delivery-normal-return-after-read",
                    None,
                    Duration::from_secs(60),
                    |snapshot| {
                        snapshot["foreground"] == true
                            && snapshot["hello"] == true
                            && snapshot["inventoryReady"] == true
                    },
                )
                .await?;
                let duplicate = self.notification(&event, hint).await?;
                if duplicate["present"] != false
                    || duplicate["ledger"]["attemptedAt"] != posted["ledger"]["attemptedAt"]
                    || duplicate["ledger"]["readThrough"].as_u64().unwrap_or(0) < event.sequence.0
                {
                    return Err(failure(
                        "physical read event re-alerted or lost authoritative suppression after normal return/reconnect",
                    ));
                }
                self.record(&format!("{mode}-native-tap-dedup"),&json!({"event":event,"read":read,"afterNormalReturn":duplicate,"humanNativeRowFallback":ambiguous,"mutatingTapEffects":0}))?;
                self.open_provider(proof).await?;
                if category == AttentionCategory::Attention {
                    let ids = receipt_ids(&self.observer.snapshot(None).await?)?;
                    self.human_stage("delivery-resolve-owned-question-once","Answer ONLY this original pending question through its supported Goatr form ONCE so this owned native turn can settle. Do not resubmit a prompt or replay a receipt.").await?;
                    self.new_receipt(
                        "delivery-owned-request-resolved",
                        &ids,
                        "RequestRespond",
                        Some(&proof.target),
                        None,
                    )
                    .await?;
                    self.wait_snapshot(
                        "delivery-owned-turn-settled",
                        None,
                        Duration::from_secs(120),
                        |snapshot| {
                            snapshot["conversation"]["activity"] == "Idle"
                                && snapshot["pending"].as_array().is_some_and(Vec::is_empty)
                        },
                    )
                    .await?;
                }
                self.verify_provider(proof).await?;
                events.push(json!({"eventId":event.event_id,"sequence":event.sequence,"target":proof.target,"category":event.category,"actualPostedMetadata":true,"nativeTapAndAuthoritativeRead":true,"dedupAfterNormalReturn":true}));
            }
        }
        if events.len() != 4 || unique.len() != 4 {
            return Err(failure(
                "physical delivery matrix is not exactly both providers' four distinct actual events",
            ));
        }
        let proof = json!({"runId":self.run,"host":self.host,"mode":mode,"events":events,"count":4,"registrationGeneration":generation,"instrumentedProductPid":self.observer.epoch.pid,
            "instrumentedProductStart":self.observer.epoch.start,"controlOnlyReversePort":self.control_port,"coldOrFgsSoleLivenessClaim":false,"nativeHintOrNotificationInjection":false});
        write_json(
            &self
                .evidence
                .join(format!("physical-{mode}-four-event-proof.json")),
            &proof,
        )?;
        Ok(proof)
    }
    async fn delivery_workflow(&mut self, providers: &[NativeProof]) -> io::Result<()> {
        let baseline_requested = self.first_open_ref()?.service_requested;
        self.private_settings().await?;
        let initial = self.observer.keeper().await?;
        let raw = self.observer.retained().await?;
        self.first_open_ref()?.preserved_with_private_host(
            &raw,
            &self.host,
            baseline_requested,
            false,
        )?;
        connector_admission(&raw, Some(&self.host))?;
        let baseline_keeper = self.retained_keeper_baseline.as_ref().ok_or_else(|| {
            failure("physical pre-private-delivery retained keeper baseline is missing")
        })?;
        let baseline_needs = keeper_retained_current(baseline_keeper, self.first_open_ref()?)?;
        let current_needs = keeper_retained_current(&initial, self.first_open_ref()?)?;
        if initial["serviceRequested"] != baseline_requested {
            return Err(failure(
                "physical global service request changed before our one Start; no ownership is adopted",
            ));
        }
        if !baseline_requested && (baseline_needs || current_needs) {
            return Err(prerequisite(
                "physical retained host needs a keeper; do not make an unrestorable owned global request before StartDelivery",
            ));
        }
        if initial["privateMode"] != "Persistent" || initial["privateEnabled"] != false {
            return Err(failure(
                "physical new private host does not have its observed untouched Persistent/off defaults; never switch/unregister to normalize it",
            ));
        }
        self.record("delivery-observed-private-defaults",&json!({"facts":initial,"defaultPersistentObserved":true,"selectPersistentGesture":false,"reason":"no idempotent unregister/global-distributor mutation"}))?;
        self.human_stage("delivery-normal-os-health","Using only normal Goatr/Android UI and explicit consent, ensure global app notifications, the Attention channel and Delivery channel are healthy and any normal runtime notification permission is granted. Do not alter another host's delivery preferences, switch distributors or use ADB permission/channel setters. Return to this PRIVATE host's Settings.").await?;
        self.wait_keeper(
            "delivery-measured-channel-health",
            Duration::from_secs(30),
            health_ready,
        )
        .await?;
        let mut enabled = BTreeMap::new();
        enabled.insert("ownedUiEnabled".to_owned(), "true".to_owned());
        self.action(
            "delivery-enable-private-attention",
            "EnableAttention",
            enabled.clone(),
        )
        .await?;
        self.action(
            "delivery-enable-private-completed",
            "EnableCompleted",
            enabled,
        )
        .await?;
        let before = self
            .wait_keeper(
                "delivery-before-owned-start",
                Duration::from_secs(30),
                |keeper| {
                    keeper["privateEnabled"] == true
                        && keeper["privateCompleted"] == true
                        && keeper["privateMode"] == "Persistent"
                        && health_ready(keeper)
                },
            )
            .await?;
        keeper_retained_current(&before, self.first_open_ref()?)?;
        if before["serviceRequested"] != baseline_requested
            || !baseline_requested && before["running"] == true
        {
            return Err(failure(
                "physical immediate pre-Start global request/running facts do not establish sole owned transition",
            ));
        }
        if before["running"] != true {
            let mut arguments = BTreeMap::new();
            arguments.insert(
                "ownedUiExpectedServiceRequested".to_owned(),
                baseline_requested.to_string(),
            );
            self.record("delivery-start-candidate",&json!({"before":before,"baselineServiceRequested":baseline_requested,"onlyPrivateSettingsStartAuthorized":true}))?;
            let (run, after) = self
                .action("delivery-one-explicit-start", "StartDelivery", arguments)
                .await?;
            if !baseline_requested && after["keeper"]["serviceRequested"] == true {
                self.owned_start = Some(run.clone());
                self.record("delivery-owned-false-to-true",&json!({"startActionRunId":run,"before":before,"after":after["keeper"],"soleObservedFalseToTrue":true,"restored":false}))?;
            }
        } else if !baseline_requested {
            return Err(failure(
                "physical unowned preexisting running keeper cannot justify an owned request",
            ));
        }
        let persistent = self
            .wait_keeper(
                "delivery-real-persistent-started",
                Duration::from_secs(45),
                |keeper| {
                    keeper["running"] == true
                        && keeper["foregroundNotification"] == true
                        && keeper["serviceRequested"] == true
                        && keeper["privateRequiresKeeper"] == true
                        && health_ready(keeper)
                },
            )
            .await?;
        if !baseline_requested && self.owned_start.is_none() {
            return Err(failure(
                "physical running Persistent request has no fully correlated owned false-to-true Start evidence",
            ));
        }
        self.record("delivery-actual-persistent-mode", &persistent)?;
        let persistent_proof = self.delivery_matrix("persistent", providers, None).await?;
        self.private_settings().await?;
        let before_ntfy = self.observer.retained().await?;
        self.first_open_ref()?.preserved_with_private_host(
            &before_ntfy,
            &self.host,
            true,
            self.owned_start.is_some(),
        )?;
        connector_admission(&before_ntfy, Some(&self.host))?;
        let mut mode = BTreeMap::new();
        mode.insert("ownedUiDeliveryMode".to_owned(), "Ntfy".to_owned());
        self.action(
            "delivery-one-official-ntfy-selection",
            "SelectDeliveryMode",
            mode,
        )
        .await?;
        let verified = self
            .wait_keeper(
                "delivery-actual-endpoint-probe-ack-verified",
                Duration::from_secs(120),
                |keeper| {
                    keeper["privateMode"] == "Ntfy"
                        && keeper["privateEnabled"] == true
                        && keeper["privateCompleted"] == true
                        && keeper["privateVerified"] == true
                        && !keeper["privateRegistrationGeneration"].is_null()
                        && !keeper["privateRegistrationSha256"].is_null()
                },
            )
            .await?;
        let snapshot = self.observer.snapshot(None).await?;
        if snapshot["hello"] != true
            || snapshot["device"].as_str()
                != self
                    .paired
                    .as_ref()
                    .ok_or_else(|| failure("physical authoritative pairing facts are absent"))?
                    .hosts
                    .iter()
                    .find(|host| host.host == self.host)
                    .ok_or_else(|| failure("physical paired private host disappeared"))?
                    .device
                    .as_deref()
        {
            return Err(failure(
                "physical verified registration is not tied to the original current phone Hello/device",
            ));
        }
        let generation = verified["privateRegistrationGeneration"].clone();
        write_json(
            &self
                .evidence
                .join("physical-ntfy-verified-registration.json"),
            &json!({"runId":self.run,"host":self.host,"facts":verified,
            "proof":"existing PushCoordinator verified() requires endpoint registration, genuine received probe and authenticated backend ACK/verified generation; not HTTP success",
            "officialDistributor":"io.heckel.ntfy","qualifiedConnectorVersion":"3.3.5","currentOriginalHelloBound":true}),
        )?;
        self.action(
            "delivery-background-private-ntfy-settings",
            "BackgroundPrivateSettings",
            BTreeMap::new(),
        )
        .await?;
        let free = self
            .wait_keeper(
                "delivery-independent-auto-socket-free-proof",
                Duration::from_secs(45),
                socket_free,
            )
            .await?;
        keeper_retained_current(&free, self.first_open_ref()?)?;
        write_json(
            &self
                .evidence
                .join("physical-ntfy-auto-socket-free-proof.json"),
            &json!({"runId":self.run,"facts":free,"beforeAnyStopDelivery":true,
            "productObserverHasNoRuntimeClaim":true,"instrumentedProcessAliveIsNotEndpointOrFgsOwnership":true,"allSocketsOrColdProcessAbsenceClaim":false}),
        )?;
        self.recheck_product().await?;
        launch_product(&self.adb).await?;
        self.wait_snapshot(
            "ntfy-return-for-real-matrix",
            None,
            Duration::from_secs(60),
            |snapshot| {
                snapshot["foreground"] == true
                    && snapshot["hello"] == true
                    && snapshot["mutationReady"] == true
            },
        )
        .await?;
        let ntfy_proof = self
            .delivery_matrix("ntfy", providers, Some(&generation))
            .await?;
        let network = self
            .baseline_network
            .clone()
            .ok_or_else(|| failure("physical original network transport baseline is missing"))?;
        self.human_stage("delivery-restore-owned-network","Restore ONLY the temporary phone radio/network choices changed for this run to their original LAN/network configuration using normal phone UI. Do not change a delivery preference or distributor. Return to Goatr and leave original native work running.").await?;
        self.wait_snapshot(
            "delivery-original-network-transports-restored",
            None,
            Duration::from_secs(90),
            |snapshot| {
                snapshot["foreground"] == true
                    && snapshot["hello"] == true
                    && snapshot["mutationReady"] == true
                    && snapshot["network"] == network
            },
        )
        .await?;
        self.private_settings().await?;
        // Both original proof files exist before cleanup is even considered; Stop cannot manufacture either.
        let persistent_path = self
            .evidence
            .join("physical-persistent-four-event-proof.json");
        let ntfy_path = self.evidence.join("physical-ntfy-four-event-proof.json");
        let proofs = json!({"persistent":binary_sha256(&persistent_path)?,"ntfy":binary_sha256(&ntfy_path)?,"persistentEvents":persistent_proof["count"],"ntfyEvents":ntfy_proof["count"]});
        let candidate = self.observer.keeper().await?;
        let retained = self.observer.retained().await?;
        self.first_open_ref()?.preserved_with_private_host(
            &retained,
            &self.host,
            boolean(&candidate, "serviceRequested")?,
            self.owned_start.is_some(),
        )?;
        if keeper_retained_current(&candidate, self.first_open_ref()?)?
            || candidate["privateMode"] != "Ntfy"
            || candidate["privateEnabled"] != true
            || candidate["privateVerified"] != true
            || candidate["privateRegistrationGeneration"] != generation
        {
            return Err(failure(
                "physical cleanup candidate changed retained keeper needs/private registration; leave ownership and flag intact",
            ));
        }
        let registration = candidate["privateRegistrationSha256"].clone();
        if !baseline_requested {
            if candidate["serviceRequested"] == true {
                let owned = self.owned_start.clone().ok_or_else(|| {
                    failure("physical current true global request is not our proven owned Start")
                })?;
                self.record("delivery-stop-owned-candidate",&json!({"proofFiles":proofs,"startActionRunId":owned,"originalBaselineServiceRequested":false,"candidate":candidate,"retained":retained,"cleanupIsNotSocketFreeProof":true}))?;
                let mut arguments = BTreeMap::new();
                extend(
                    &mut arguments,
                    &[
                        ("ownedUiExpectedServiceRequested", "true"),
                        ("ownedUiOwnedStartRunId", &owned.0),
                    ],
                );
                self.action(
                    "delivery-stop-only-owned-request-once",
                    "StopDelivery",
                    arguments,
                )
                .await?;
            } else {
                self.record("delivery-request-already-false",&json!({"candidate":candidate,"stopGestureIssued":false,"noOpAction":false,"proofFiles":proofs}))?;
            }
            self.wait_keeper(
                "delivery-owned-request-restored",
                Duration::from_secs(45),
                |keeper| {
                    keeper["serviceRequested"] == false
                        && keeper["running"] == false
                        && keeper["foregroundNotification"] == false
                        && keeper["reconnecting"] == 0
                        && keeper["privateMode"] == "Ntfy"
                        && keeper["privateEnabled"] == true
                        && keeper["privateVerified"] == true
                        && keeper["privateRegistrationGeneration"] == generation
                        && keeper["privateRegistrationSha256"] == registration
                },
            )
            .await?;
            self.service_key_restored = self.owned_start.is_some();
        } else {
            if candidate["serviceRequested"] != true {
                return Err(failure(
                    "physical original true global service request changed; never clear or restore it by raw write",
                ));
            }
            self.record("delivery-baseline-true-preserved",&json!({"candidate":candidate,"stopGestureIssued":false,"globalRequestedTruePreserved":true,"proofFiles":proofs}))?;
        }
        self.action(
            "delivery-final-background-private-settings",
            "BackgroundPrivateSettings",
            BTreeMap::new(),
        )
        .await?;
        let expected_requested = baseline_requested;
        let final_keeper = self
            .wait_keeper(
                "delivery-final-real-no-keeper-claim",
                Duration::from_secs(45),
                |keeper| {
                    socket_free(keeper)
                        && keeper["serviceRequested"] == expected_requested
                        && keeper["privateRegistrationGeneration"] == generation
                        && keeper["privateRegistrationSha256"] == registration
                },
            )
            .await?;
        let after = self.observer.retained().await?;
        self.first_open_ref()?.preserved_with_private_host(
            &after,
            &self.host,
            baseline_requested,
            self.service_key_restored,
        )?;
        self.record("delivery-owned-ui-cleanup-outcome",&json!({"proofFiles":proofs,"keeper":final_keeper,"retained":after,"baselineServiceRequested":baseline_requested,
            "ownedStart":self.owned_start,"effectiveServiceBooleanRestored":self.service_key_restored,"literalAbsentXmlKeyRestorationClaim":false,
            "privateHostStillEnabledNtfy":true,"registrationGeneration":generation,"rawPreferenceOrChannelReset":false,"cleanupNotCountedAsSocketFreeProof":true}))?;
        Ok(())
    }
}

async fn wait_original_process_absent(adb: &PhysicalAdb, epoch: &ProductEpoch) -> io::Result<()> {
    let script = format!(
        "if 'test' '-e' '/proc/{}'; then 'printf' '%s' 'present'; else 'printf' '%s' 'absent'; fi",
        epoch.pid
    );
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = adb
            .shell(&["run-as", APP, "sh", "-c", &script], 128)
            .await?;
        if text(&bytes)? == "absent" {
            return Ok(());
        }
        if text(&bytes)? != "present" || Instant::now() >= until {
            return Err(failure(
                "physical finished original instrumentation PID absence is unproved; no replacement is adopted",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
impl Matrix {
    async fn review_and_finish(mut self, result: io::Result<()>) -> io::Result<()> {
        let mut passed = result.is_ok();
        let mut diagnostic = None;
        let mut code = match result {
            Ok(()) => "matrix_effects_proved_pending_owned_finish",
            Err(error) => retain_review_failure(error, &mut diagnostic),
        };
        let expected = format!(
            "FINISH {} {} {}",
            self.run.0, self.observer.epoch.pid, self.observer.epoch.start
        );
        let mut announced = false;
        loop {
            let was_failed = self.input_pin_failed;
            if let Err(error) = recheck_retained_inputs(&self.inputs, &mut self.input_pin_failed) {
                let _ = retain_review_failure(error, &mut diagnostic);
                passed = false;
                code = "original_frozen_input_changed_finish_rejected";
                if !was_failed {
                    announced = false;
                }
            }
            if !self.input_pin_failed
                && let Err(error) =
                    recheck_unit_profiles(&self.authority, &self.stock, &mut self.input_pin_failed)
            {
                let _ = retain_review_failure(error, &mut diagnostic);
                passed = false;
                code = "original_unit_wake_profile_changed_finish_rejected";
                if !was_failed {
                    announced = false;
                }
            }
            if self.observer.task.is_finished() || self.observer.instrumentation.task.is_finished()
            {
                let lost = json!({"runId":self.run,"gate":"physical-mvp","status":"failed","stage":self.last_stage,"code":"original_observer_lost_or_unknown_before_authorized_finish",
                    "originalPid":self.observer.epoch.pid,"originalStart":self.observer.epoch.start,"productApkSha256":self.product_sha,
                    "effectFailureCode":code,"primaryFailure":diagnostic,"admissionFailure":diagnostic.as_ref().and_then(review_admission_failure),
                    "recreatedOrAdopted":false,"nativeResourcesAndFlagsNotReset":true,"runtimeAcceptanceClaim":false});
                if let Err(error) = write_json(
                    &self
                        .evidence
                        .join(format!("physical-observer-lost-{}.json", Uuid::random().0)),
                    &lost,
                ) {
                    let _ = writeln!(
                        io::stderr(),
                        "{}",
                        json!({"gate":"physical-mvp","status":"failed","code":"owner_lost_evidence_write_failed","io":blocker(&error),"primaryFailure":diagnostic,"admissionFailure":diagnostic.as_ref().and_then(review_admission_failure)})
                    );
                }
                return Err(failure(
                    "physical original product observer was lost/unknown before cleanup authorization; permanent failure, no replacement",
                ));
            }
            if !announced {
                let state = json!({"runId":self.run,"gate":"physical-mvp","status":if passed{"effects_proved_owner_retained"}else{"blocked_owner_retained"},"stage":self.last_stage,"code":code,
                    "originalPid":self.observer.epoch.pid,"originalStart":self.observer.epoch.start,"productApkSha256":self.product_sha,
                    "primaryFailure":diagnostic,"admissionFailure":diagnostic.as_ref().and_then(review_admission_failure),
                    "native":self.stock.desktop.process,"companion":self.stock.serve.process,"ownedStart":self.owned_start,"serviceKeyRestored":self.service_key_restored,
                    "cleanupAuthorization":finish_authorization(self.input_pin_failed,&expected),
                    "warning":if self.input_pin_failed{"Original frozen input authority failed. This FINISH is rejected; retain this same observer/resources. Any cleanup requires separate explicit human ownership review, never automatic termination, restart or pin adoption."}else{"Review exact original PID/start/artifacts and retained flags/resources. This command authorizes ONE observer Finish; Android may kill that instrumented product PID. It does NOT StopDelivery, reset preferences, restore an uncertain flag, kill native work or erase receipts."},
                    "coldOrFgsSoleLivenessClaim":false,"ifControllerIsExternallyLostBeforeFinish":"owner retention becomes unknown, never acceptance"});
                if let Err(error) = self.record("owner-retained-awaiting-explicit-review", &state) {
                    let io_code = blocker(&error);
                    let _ = retain_review_failure(error, &mut diagnostic);
                    passed = false;
                    code = "retained_owner_evidence_write_failed";
                    let _ = writeln!(
                        io::stderr(),
                        "{}",
                        json!({"gate":"physical-mvp","status":"blocked_owner_retained","io":io_code,"primaryFailure":diagnostic,"admissionFailure":diagnostic.as_ref().and_then(review_admission_failure)})
                    );
                }
                if let Err(error) = writeln!(io::stdout(), "{state}") {
                    let _ = retain_review_failure(error, &mut diagnostic);
                    passed = false;
                    code = "retained_owner_console_unavailable";
                }
                announced = true;
            }
            let mut poll = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut poll, 1, 0) } > 0 && poll.revents & libc::POLLIN != 0 {
                let mut bytes = [0u8; 256];
                let count = unsafe {
                    libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len())
                };
                let authorized = count > 0
                    && std::str::from_utf8(&bytes[..count as usize])
                        .is_ok_and(|value| value.trim() == expected);
                bytes.fill(0);
                if authorized {
                    if finish_authorization(self.input_pin_failed, &expected).is_none() {
                        announced = false;
                        continue;
                    }
                    if let Err(error) = self.recheck_product().await {
                        let next = blocker(&error);
                        let _ = retain_review_failure(error, &mut diagnostic);
                        passed = false;
                        code = next;
                        announced = false;
                        continue;
                    }
                    let action = Uuid::random();
                    let current = match self
                        .observer
                        .query(json!({"kind":"epoch","actionRunId":action}), "epoch")
                        .await
                    {
                        Ok((current, _)) => current,
                        Err(error) => {
                            let _ = retain_review_failure(error, &mut diagnostic);
                            passed = false;
                            code = "original_observer_cleanup_epoch_unproved";
                            announced = false;
                            continue;
                        }
                    };
                    if string(&current, "apkSha256").ok() != Some(self.product_sha.as_str()) {
                        passed = false;
                        code = "original_observer_cleanup_artifact_changed";
                        announced = false;
                        continue;
                    }
                    if passed {
                        let final_check=async {
                            self.recheck_product().await?;
                            let retained=self.observer.retained().await?;
                            self.first_open_ref()?.preserved_with_private_host(&retained,&self.host,self.first_open_ref()?.service_requested,self.service_key_restored)?;
                            let ui=self.baseline_ui.as_ref().ok_or_else(||failure("physical original UI baseline is missing"))?;
                            let network=self.baseline_network.as_ref().ok_or_else(||failure("physical original network baseline is missing"))?;
                            if current["snapshot"]["ui"]["orientation"]!=ui["orientation"]||current["snapshot"]["ui"]["fontScale"]!=ui["fontScale"]||current["snapshot"]["network"]!=*network {
                                return Err(failure("physical temporary display/network user choices are not restored"));
                            }
                            Ok::<(),io::Error>(())
                        }.await;
                        if let Err(error) = final_check {
                            passed = false;
                            code = retain_review_failure(error, &mut diagnostic);
                            announced = false;
                            continue;
                        }
                    }
                    let original = self.observer.epoch.clone();
                    let intent = json!({"runId":self.run,"originalPid":original.pid,"originalStart":original.start,"productApkSha256":self.product_sha,
                        "explicitHumanAuthorization":true,"freshSignedEpoch":current,"acceptanceEffectsProvedBeforeFinish":passed,"cleanupOnly":true,
                        "nativeOrPreferenceCleanupAuthorized":false,"androidMayKillInstrumentedProductPid":true});
                    let finish_id = Uuid::random();
                    if let Err(error) = write_json(
                        &self.evidence.join(format!(
                            "physical-observer-finish-{}.intent.json",
                            finish_id.0
                        )),
                        &intent,
                    ) {
                        let _ = retain_review_failure(error, &mut diagnostic);
                        passed = false;
                        code = "observer_finish_intent_not_durable";
                        announced = false;
                        continue;
                    }
                    if let Err(error) = self.recheck() {
                        let next = blocker(&error);
                        let _ = retain_review_failure(error, &mut diagnostic);
                        passed = false;
                        code = next;
                        announced = false;
                        continue;
                    }
                    let outcome = self.observer.finished().await.inspect_err(|error| {
                        if diagnostic.is_none() {
                            diagnostic = Some(review_failure_projection(error));
                        }
                    });
                    if let Err(error) =
                        recheck_retained_inputs(&self.inputs, &mut self.input_pin_failed)
                    {
                        let _ = retain_review_failure(error, &mut diagnostic);
                        passed = false;
                        code = "original_frozen_input_changed_during_finish";
                    }
                    if !self.input_pin_failed
                        && let Err(error) = recheck_unit_profiles(
                            &self.authority,
                            &self.stock,
                            &mut self.input_pin_failed,
                        )
                    {
                        let _ = retain_review_failure(error, &mut diagnostic);
                        passed = false;
                        code = "original_unit_wake_profile_changed_during_finish";
                    }
                    if let Err(error) = require_product_epoch(
                        &self.adb,
                        &self.product_epoch,
                        &self.product_sha,
                        &self.inputs,
                        Some(&mut self.input_pin_failed),
                    )
                    .await
                    {
                        let _ = retain_review_failure(error, &mut diagnostic);
                        passed = false;
                        code = "original_product_epoch_changed_during_finish";
                    }
                    let mut acknowledged = false;
                    let mut absent = false;
                    let mut reverse_removed = false;
                    if let Ok(finished) = outcome {
                        acknowledged =
                            finished.pid == original.pid && finished.start == original.start;
                        if acknowledged {
                            match wait_original_process_absent(&self.adb, &original).await {
                                Ok(()) => absent = true,
                                Err(error) => {
                                    let _ = retain_review_failure(error, &mut diagnostic);
                                }
                            }
                        }
                        if acknowledged && absent {
                            match self.adb.tunnels(Some(self.control_port)).await {
                                Ok(()) => {
                                    let mapping = format!("tcp:{}", self.control_port);
                                    match self
                                        .adb
                                        .invoke(&["reverse", "--remove", &mapping], 4096)
                                        .await
                                    {
                                        Ok(_) => match self.adb.tunnels(None).await {
                                            Ok(()) => reverse_removed = true,
                                            Err(error) => {
                                                let _ =
                                                    retain_review_failure(error, &mut diagnostic);
                                            }
                                        },
                                        Err(error) => {
                                            let _ = retain_review_failure(error, &mut diagnostic);
                                        }
                                    }
                                }
                                Err(error) => {
                                    let _ = retain_review_failure(error, &mut diagnostic);
                                }
                            }
                        }
                    }
                    let complete = passed && acknowledged && absent && reverse_removed;
                    let outcome = json!({"runId":self.run,"gate":"physical-mvp","status":if complete{"pass"}else{"failed"},"priorCode":code,
                        "originalPid":original.pid,"originalStart":original.start,"observerFinishAcknowledged":acknowledged,"finishedOriginalPidAbsent":absent,
                        "onlyOwnedControlReverseRemoved":reverse_removed,"primaryFailure":diagnostic,"admissionFailure":diagnostic.as_ref().and_then(review_admission_failure),"cleanupIsNotPreservationOrSocketFreeAcceptance":true,"coldOrFgsSoleLivenessClaim":false,
                        "nativeResourcesRetained":true,"native":self.stock.desktop.process,"companion":self.stock.serve.process,"root":self.stock.root,"admin":self.stock.admin,
                        "preferencesRawReset":false,"desktopOrProviderKilled":false,"uncertainReceiptsErased":false});
                    write_json(
                        &self.evidence.join(format!(
                            "physical-observer-finish-{}.outcome.json",
                            finish_id.0
                        )),
                        &outcome,
                    )?;
                    writeln!(io::stdout(), "{outcome}")?;
                    return if complete {
                        Ok(())
                    } else {
                        Err(failure(
                            "physical MVP remains failed/unpassed; exact observer cleanup outcome recorded without restoring or replaying native/user state",
                        ))
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Main runs this ONLY in a fresh reviewed Wake/NNP user unit, never the physical-device path.
    #[test]
    #[ignore = "requires explicit owned user unit, independent binary/config pins and fresh retained evidence"]
    fn owned_unit_serving_preserves_direct_child_and_inherited_listener() {
        assert_eq!(std::env::var("GOATR_TEST_UNIT_SERVING_PROOF").unwrap(), "1");
        let required =
            |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("missing explicit {name}"));
        let unit = required("GOATR_UNIT_SERVING_PROOF_UNIT");
        validate_controller_unit(&unit).unwrap();
        let binary = PathBuf::from(required("GOATR_UNIT_SERVING_PROOF_GOATR"));
        let template = PathBuf::from(required("GOATR_UNIT_SERVING_PROOF_CONFIG"));
        let workspace = PathBuf::from(required("GOATR_UNIT_SERVING_PROOF_WORKSPACE"));
        let evidence = PathBuf::from(required("GOATR_UNIT_SERVING_PROOF_EVIDENCE"));
        let binary_sha = required("GOATR_UNIT_SERVING_PROOF_GOATR_SHA256");
        let config_sha = required("GOATR_UNIT_SERVING_PROOF_CONFIG_SHA256");
        let controller_sha = required("GOATR_UNIT_SERVING_PROOF_CONTROLLER_SHA256");
        let controller = goatr::codex_relay::prepare_current_executable(true, Some(&unit))
            .unwrap()
            .expect("actual unit authority required");
        controller.serving_controller(None).unwrap();
        let controller_input =
            frozen_input(&std::env::current_exe().unwrap(), &controller_sha).unwrap();
        assert_eq!(
            complete_stamp(&fs::metadata("/proc/self/exe").unwrap()),
            complete_stamp(&controller_input.2)
        );
        let serving_input = frozen_input(&binary, &binary_sha).unwrap();
        let (deployment, template_file, template_stamp) =
            qualified_configuration(&template, &config_sha).unwrap();
        let inputs = vec![
            serving_input,
            (template.clone(), template_file, template_stamp),
            controller_input,
        ];
        private_dir(&workspace, false).unwrap();
        fresh_shared_evidence(&evidence).unwrap();
        let args = Command::new("owned-unit-serving-proof")
            .arg(
                Arg::new("config")
                    .long("config")
                    .value_parser(value_parser!(PathBuf))
                    .required(true),
            )
            .arg(
                Arg::new("workspace-root")
                    .long("workspace-root")
                    .value_parser(value_parser!(PathBuf))
                    .required(true),
            )
            .try_get_matches_from([
                std::ffi::OsStr::new("owned-unit-serving-proof"),
                std::ffi::OsStr::new("--config"),
                template.as_os_str(),
                std::ffi::OsStr::new("--workspace-root"),
                workspace.as_os_str(),
            ])
            .unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut stock=StockRun::start(&args,&evidence,Uuid::random(),"Owned unit serving proof",deployment,binary,
                StockLaunchPins { template_sha: &config_sha, expected_binary_sha: &binary_sha,
                    controller: &controller, original_inputs: &inputs }).await.unwrap();
            stock.recheck().unwrap();controller.recheck(None).unwrap();recheck_inputs(&inputs).unwrap();
            assert_eq!(stock.serve_unit.cgroup(),controller.cgroup());
            assert_eq!(stock.serve_unit.manager(),controller.manager());
            assert_ne!(stock.serve.process.pid,controller.process().pid);
            let argv=goatr::codex_relay::process_arguments(&stock.serve.process).unwrap();
            assert!(argv.windows(2).any(|pair|pair[0]=="--expected-unit"&&pair[1]==unit));
            assert!(argv.windows(2).any(|pair|pair[0]=="--owned-omp-relay-fd"));
            let listener=fs::metadata(format!("/proc/self/fd/{}",stock._relay.as_raw_fd())).unwrap();
            let original_fd=argv.windows(2).find(|pair|pair[0]=="--owned-omp-relay-fd").unwrap()[1].parse::<i32>().unwrap();
            // Product closes the authenticated incoming FD and retains Relay's DUPLICATE before admin binds.
            let retained=fs::read_dir(format!("/proc/{}/fd",stock.serve.process.pid)).unwrap().map(Result::unwrap).any(|entry| {
                if entry.file_name().to_str().and_then(|name|name.parse::<i32>().ok())==Some(original_fd){return false;}
                match fs::metadata(entry.path()) {
                    Ok(m)=>(m.dev(),m.ino())==(listener.dev(),listener.ino()),
                    Err(error) if error.kind()==io::ErrorKind::NotFound=>false,
                    Err(error)=>panic!("original serving FD became uninspectable: {error}"),
                }
            });
            assert!(retained,"original listener duplicate was not consumed by serving Relay");
            match fs::metadata(format!("/proc/{}/fd/{original_fd}",stock.serve.process.pid)) {
                Ok(m)=>assert_ne!((m.dev(),m.ino()),(listener.dev(),listener.ino())),
                Err(error)=>assert_eq!(error.kind(),io::ErrorKind::NotFound),
            }
            stock.recheck().unwrap();controller.recheck(None).unwrap();
            write_json(&evidence.join("owned-unit-serving-proof.json"),&json!({"status":"pass","controller":controller.process(),
                "serving":stock.serve.process,"manager":controller.manager(),"unit":unit,"cgroup":controller.cgroup(),
                "controllerCaps":companion_caps(controller.process(),&controller,None).unwrap(),"servingCaps":companion_caps(&stock.serve.process,&stock.serve_unit,None).unwrap(),
                "native":stock.desktop.process,"inheritedListenerDevice":listener.dev(),"inheritedListenerInode":listener.ino(),
                "authenticatedDuplicateConsumed":retained,"originalStartupDeadlinePreserved":true,"adminResponsive":true,
                "noDeviceOrApk":true,"resourcesRetained":true,"root":stock.root,"adminSocket":stock.socket})).unwrap();
        });
    }
    fn prepared_fixture_package(
        package: &str,
        uid: u32,
        path: &str,
        sha: &str,
        pristine: bool,
    ) -> Value {
        json!({"package":package,"uid":uid.to_string(),"versionCode":"1","versionName":"0.1.0","firstInstallTime":"2026-01-01 00:00:00",
            "dataDir":format!("/data/user/0/{package}"),"userState":format!("User 0: ceDataInode=123 installed=true hidden=false suspended=false distractionFlags=0 stopped={pristine} notLaunched={pristine} enabled=0 instant=false virtual=false"),
            "apkPath":path,"apkSha256":sha})
    }
    fn prepared_fixture() -> Value {
        let sha = "1".repeat(64);
        let signer = "2".repeat(64);
        let serial = "owned-physical";
        let test = prepared_fixture_package(
            TEST,
            10001,
            "/data/app/test/base.apk",
            &"3".repeat(64),
            false,
        );
        let verify = prepared_fixture_package(
            VERIFY,
            10002,
            "/data/app/verify/base.apk",
            &"4".repeat(64),
            false,
        );
        let ntfy = prepared_fixture_package(
            "io.heckel.ntfy",
            10003,
            "/data/app/ntfy/base.apk",
            &"5".repeat(64),
            false,
        );
        let before = json!({TEST:test,VERIFY:verify,"io.heckel.ntfy":ntfy});
        let packages = format!(
            "package:/data/app/test/base.apk={TEST} uid:10001\npackage:/data/app/verify/base.apk={VERIFY} uid:10002\n"
        );
        let mut installed =
            prepared_fixture_package(APP, 10004, "/data/app/product/base.apk", &sha, true);
        let fields = installed.as_object_mut().unwrap();
        fields.insert(
            "signerByIdenticalIndependentlySdkVerifiedBytes".to_owned(),
            json!(signer),
        );
        fields.insert("defaultProcess".to_owned(), json!(APP));
        fields.insert(
            "registeredLauncher".to_owned(),
            json!(format!("{APP}/{APP}.MainActivity")),
        );
        fields.insert("defaultProcessAbsent".to_owned(), json!(true));
        fields.insert(
            "processObservation".to_owned(),
            json!({"exit":1,"signal":null,"stdout":"","stderr":""}),
        );
        json!({"version":1,"kind":"owned-prepared-fresh-product-install","runId":Uuid::random().0,"createdAt":"2026-01-01T00:00:01Z","package":APP,
            "device":{"serial":serial,"usbTransport":format!("{serial} device usb:1-4 product:owned model:Owned device:owned transport_id:15"),"model":"Owned","sdk":"30","abi":"arm64-v8a","qemu":""},
            "source":{"apkBuildSource":"/evidence/unfollowed/source","currentAndroidInputsMatched":176,"qualification":"/evidence/unfollowed/qualification"},
            "artifact":{"path":"/evidence/unfollowed/app.apk","sha256":sha,"bytes":128,"uid":1000,"mode":"0400","nlink":1,"device":"1","inode":"2","mtimeNs":"3","ctimeNs":"4","signerSha256":signer},
            "preinstall":{"intent":"/evidence/unfollowed/intent","intentSha256":"6".repeat(64),"active":{"exit":0,"signal":null,"stdout":packages,"stderr":""},
                "includingUninstalled":{"exit":0,"signal":null,"stdout":packages,"stderr":""},"exactProductAbsent":true},
            "install":{"binary":"/evidence/unfollowed/adb","argv":["-s",serial,"install","-t","/evidence/unfollowed/app.apk"],
                "receipt":"/evidence/unfollowed/receipt","receiptSha256":"7".repeat(64),"result":{"exit":0,"signal":null,"stdout":"Performing Streamed Install\nSuccess\n","stderr":""},"attempts":1},
            "installed":installed,"preservedPackages":{"before":before,"after":before,"publicIdentityAndBytesUnchanged":true,"privateBytePreservationClaim":false},
            "noReplace":true,"noDowngrade":true,"noGrant":true,"noUninstall":true,"noClear":true,"noLaunch":true,"testHelperUpdated":false,
            "suppressedApplicationBaselineExecuted":false,"privateAppDataRead":false,"providerOrHerdrLaunch":false,"fullMvpAcceptanceClaim":false})
    }
    fn prepared_pm_dump(fixture: &Value) -> String {
        let installed = &fixture["installed"];
        format!(
            "Packages:\n  Package [{APP}] (owned):\n    userId=10004\n    dataDir={}\n    firstInstallTime={}\n    versionCode=1 minSdk=26 targetSdk=36\n    versionName=0.1.0\n    {}\nOther:\n",
            installed["dataDir"].as_str().unwrap(),
            installed["firstInstallTime"].as_str().unwrap(),
            installed["userState"].as_str().unwrap()
        )
    }
    #[test]
    fn prepared_creator_record_consumes_closed_owned_receipt_not_reference_path_authority() {
        let valid = prepared_fixture();
        let parsed = parse_prepared_install(&valid).unwrap();
        assert_eq!(parsed.epoch.record.uid, 10004);
        for key in [
            "noReplace",
            "noDowngrade",
            "noGrant",
            "noUninstall",
            "noClear",
            "noLaunch",
            "testHelperUpdated",
            "suppressedApplicationBaselineExecuted",
            "privateAppDataRead",
            "providerOrHerdrLaunch",
            "fullMvpAcceptanceClaim",
        ] {
            let mut changed = valid.clone();
            changed[key] = json!(!changed[key].as_bool().unwrap());
            assert!(parse_prepared_install(&changed).is_err(), "{key}");
        }
        for case in [
            "version",
            "kind",
            "unknown",
            "attempts",
            "replace",
            "failed-command",
            "prior-product",
            "process-present",
            "used-lifecycle",
            "preserved",
            "source-closure",
        ] {
            let mut changed = valid.clone();
            match case {
                "version" => changed["version"] = json!(2),
                "kind" => changed["kind"] = json!("arbitrary-installed-app"),
                "unknown" => changed["extra"] = json!(true),
                "attempts" => changed["install"]["attempts"] = json!(2),
                "replace" => changed["install"]["argv"][3] = json!("-r"),
                "failed-command" => changed["install"]["result"]["exit"] = json!(1),
                "prior-product" => {
                    changed["preinstall"]["active"]["stdout"] = json!(format!(
                        "package:/data/app/product/base.apk={APP} uid:10004\n"
                    ))
                }
                "process-present" => {
                    changed["installed"]["processObservation"]["stdout"] = json!("123\n")
                }
                "used-lifecycle" => {
                    changed["installed"]["userState"] = json!(
                        string(&valid["installed"], "userState")
                            .unwrap()
                            .replace("notLaunched=true", "notLaunched=false")
                    )
                }
                "preserved" => {
                    changed["preservedPackages"]["after"][TEST]["apkSha256"] = json!("0".repeat(64))
                }
                "source-closure" => changed["source"]["currentAndroidInputsMatched"] = json!(0),
                _ => unreachable!(),
            }
            assert!(parse_prepared_install(&changed).is_err(), "{case}");
        }
        let mut reconnected = valid.clone();
        reconnected["device"]["usbTransport"] = json!(
            "owned-physical  device usb:2-7 product:owned model:Owned device:owned transport_id:99"
        );
        assert_eq!(
            parse_prepared_install(&reconnected).unwrap().device,
            parsed.device
        );
    }
    #[test]
    fn prepared_subject_has_no_implicit_adoption_or_absent_fallback_and_binds_current_sdk_metadata()
    {
        let prepared = parse_prepared_install(&prepared_fixture()).unwrap();
        let mut packages = BTreeMap::new();
        admit_product_inventory(&None, &packages).unwrap();
        assert!(admit_product_inventory(&Some(prepared.clone()), &packages).is_err());
        packages.insert(APP.to_owned(), prepared.epoch.record.clone());
        assert!(admit_product_inventory(&None, &packages).is_err());
        admit_product_inventory(&Some(prepared.clone()), &packages).unwrap();
        for field in ["uid", "path"] {
            let mut wrong = packages.clone();
            let record = wrong.get_mut(APP).unwrap();
            if field == "uid" {
                record.uid += 1;
            } else {
                record.path = Some("/data/app/replaced/base.apk".to_owned());
            }
            assert!(admit_product_inventory(&Some(prepared.clone()), &wrong).is_err());
        }
        // Pure predicate input; real CURRENT SDK qualification remains installed_artifact's responsibility.
        let artifact = Artifact {
            path: PathBuf::from("/owned/sdk-qualified.apk"),
            sha256: prepared.apk_sha256.clone(),
            signer: prepared.signer.clone(),
            package: APP.to_owned(),
            debug: true,
        };
        prepared_artifact_admission(&prepared, &prepared.device, &artifact, 128).unwrap();
        let mut wrong = artifact.clone();
        wrong.signer = "0".repeat(64);
        assert!(prepared_artifact_admission(&prepared, &prepared.device, &wrong, 128).is_err());
        wrong = artifact.clone();
        wrong.sha256 = "0".repeat(64);
        assert!(prepared_artifact_admission(&prepared, &prepared.device, &wrong, 128).is_err());
        assert!(prepared_artifact_admission(&prepared, &prepared.device, &artifact, 129).is_err());
        let mut device = prepared.device.clone();
        device.serial = "other-physical".to_owned();
        assert!(prepared_artifact_admission(&prepared, &device, &artifact, 128).is_err());
    }
    #[test]
    fn prepared_lifecycle_consumption_never_rebases_stable_product_epoch() {
        let prepared = parse_prepared_install(&prepared_fixture()).unwrap();
        let pristine = ProductObservation {
            epoch: prepared.epoch.clone(),
            stopped: true,
            not_launched: true,
        };
        prepared_observation_admission(&prepared, &pristine, true).unwrap();
        let consumed = ProductObservation {
            stopped: false,
            not_launched: false,
            ..pristine.clone()
        };
        assert!(prepared_observation_admission(&prepared, &consumed, true).is_err());
        prepared_observation_admission(&prepared, &consumed, false).unwrap();
        for field in [
            "uid",
            "path",
            "first-install",
            "data",
            "ce",
            "version",
            "version-name",
            "default-process",
            "launcher",
            "user-policy",
        ] {
            let mut changed = consumed.clone();
            match field {
                "uid" => changed.epoch.record.uid += 1,
                "path" => {
                    changed.epoch.record.path = Some("/data/app/replaced/base.apk".to_owned())
                }
                "first-install" => changed.epoch.ownership.first_install_time = "later".to_owned(),
                "data" => changed.epoch.ownership.data_dir = "/data/other".to_owned(),
                "ce" => {
                    changed
                        .epoch
                        .user
                        .insert("ceDataInode".to_owned(), "456".to_owned());
                }
                "version" => changed.epoch.version_code = "2".to_owned(),
                "version-name" => changed.epoch.version_name = "other".to_owned(),
                "default-process" => changed.epoch.default_process = "other".to_owned(),
                "launcher" => changed.epoch.launcher = format!("{APP}/Other"),
                "user-policy" => {
                    changed
                        .epoch
                        .user
                        .insert("enabled".to_owned(), "2".to_owned());
                }
                _ => unreachable!(),
            }
            assert!(
                prepared_observation_admission(&prepared, &changed, false).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn prepared_public_pm_epoch_requires_exact_fields_and_only_lifecycle_is_consumed() {
        let fixture = prepared_fixture();
        let prepared = parse_prepared_install(&fixture).unwrap();
        let dump = prepared_pm_dump(&fixture);
        let parsed = product_observation_fields(
            &dump,
            &prepared.epoch.record,
            prepared.epoch.launcher.clone(),
        )
        .unwrap();
        prepared_observation_admission(&prepared, &parsed, true).unwrap();
        let consumed = dump
            .replace("stopped=true", "stopped=false")
            .replace("notLaunched=true", "notLaunched=false");
        let parsed = product_observation_fields(
            &consumed,
            &prepared.epoch.record,
            prepared.epoch.launcher.clone(),
        )
        .unwrap();
        prepared_observation_admission(&prepared, &parsed, false).unwrap();
        for changed in [
            dump.replace("versionCode=1 minSdk=26 targetSdk=36\n", ""),
            dump.replace("    versionName=0.1.0\n", ""),
            dump.replace("ceDataInode=123", "ceDataInode=0"),
            dump.replace("installed=true", "installed=false"),
            dump.replace("enabled=0", "enabled=0 extraUnknown=1"),
        ] {
            assert!(
                product_observation_fields(
                    &changed,
                    &prepared.epoch.record,
                    prepared.epoch.launcher.clone()
                )
                .is_err()
            );
        }
    }
    #[test]
    fn current_usb_admission_requires_globally_one_authorized_device_and_allows_reconnect() {
        let header = "List of devices attached\n";
        let primary =
            "owned-physical  device usb:1-4 product:owned model:Owned device:owned transport_id:15";
        let with_unavailable = format!(
            "{header}{primary}\noffline-physical offline transport_id:16\nuntrusted-physical unauthorized usb:1-5 transport_id:17\n"
        );
        validate_current_usb_device(&with_unavailable, "owned-physical").unwrap();
        let reconnected = format!(
            "{header}owned-physical    device usb:2-7 product:owned model:Owned device:owned transport_id:99\n"
        );
        validate_current_usb_device(&reconnected, "owned-physical").unwrap();
        for extra in [
            "second-physical device usb:1-6 transport_id:18",
            "10.0.0.2:5555 device product:owned transport_id:18",
            "emulator-5554 device product:sdk transport_id:18",
        ] {
            assert!(
                validate_current_usb_device(
                    &format!("{header}{primary}\n{extra}\n"),
                    "owned-physical"
                )
                .is_err()
            );
        }
        for inventory in [
            format!("{header}{primary}\n"),
            with_unavailable,
            reconnected,
        ] {
            assert!(validate_current_usb_device(&inventory, "other-physical").is_err());
        }
        for inventory in [
            format!("{header}owned-physical device product:owned transport_id:15\n"),
            format!("{header}owned-physical device usb: transport_id:15\n"),
            format!("{header}owned-physical offline transport_id:15\n"),
            format!("{header}{primary}\nowned-physical offline transport_id:16\n"),
            format!("{header}emulator-5554 device usb:1-4 transport_id:15\n"),
            "owned-physical device usb:1-4\n".to_owned(),
            "x".repeat(65537),
        ] {
            assert!(validate_current_usb_device(&inventory, "owned-physical").is_err());
        }
    }
    #[test]
    fn post_baseline_current_epoch_admission_refuses_public_pm_changes_but_allows_consumed_flags() {
        let fixture = prepared_fixture();
        let prepared = parse_prepared_install(&fixture).unwrap();
        let original = prepared_pm_dump(&fixture);
        let consumed = original
            .replace("stopped=true", "stopped=false")
            .replace("notLaunched=true", "notLaunched=false");
        let current = product_observation_fields(
            &consumed,
            &prepared.epoch.record,
            prepared.epoch.launcher.clone(),
        )
        .unwrap();
        stable_product_epoch_admission(&prepared.epoch, &current).unwrap();
        assert!(prepared_observation_admission(&prepared, &current, true).is_err());
        // Exercise the same post-wait production predicate, not a simulated runner or Activity.
        for changed in [
            consumed.replace("ceDataInode=123", "ceDataInode=456"),
            consumed.replace("versionCode=1 ", "versionCode=2 "),
            consumed.replace("versionName=0.1.0", "versionName=0.2.0"),
            consumed.replace("2026-01-01 00:00:00", "2026-01-02 00:00:00"),
            consumed.replace("enabled=0", "enabled=2"),
            consumed.replace(
                &format!("/data/user/0/{APP}"),
                &format!("/data/user_de/0/{APP}"),
            ),
        ] {
            let current = product_observation_fields(
                &changed,
                &prepared.epoch.record,
                prepared.epoch.launcher.clone(),
            )
            .unwrap();
            assert!(stable_product_epoch_admission(&prepared.epoch, &current).is_err());
        }
        let mut replaced = prepared.epoch.record.clone();
        replaced.path = Some("/data/app/replaced/base.apk".to_owned());
        let current =
            product_observation_fields(&consumed, &replaced, prepared.epoch.launcher.clone())
                .unwrap();
        assert!(stable_product_epoch_admission(&prepared.epoch, &current).is_err());
        let current = product_observation_fields(
            &consumed,
            &prepared.epoch.record,
            format!("{APP}/OtherActivity"),
        )
        .unwrap();
        assert!(stable_product_epoch_admission(&prepared.epoch, &current).is_err());
    }
    #[test]
    fn prepared_record_reader_is_sealed_hash_pinned_duplicate_strict_and_permanently_latched() {
        let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = parent.join(format!("goatr-prepared-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("record.json");
        let bytes = serde_json::to_vec(&prepared_fixture()).unwrap();
        let expected = digest_text(std::str::from_utf8(&bytes).unwrap());
        parse_pinned_prepared_bytes(&bytes, &expected).unwrap();
        assert!(parse_pinned_prepared_bytes(&bytes, &"0".repeat(64)).is_err());
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(read_prepared_install(&path, &"0".repeat(64)).is_err());
        if uid() == 1000 {
            let (_, pin) = read_prepared_install(&path, &expected).unwrap();
            let inputs = vec![pin];
            let mut failed = false;
            recheck_product_inputs(&inputs, Some(&mut failed)).unwrap();
            let moved = root.with_file_name(format!("goatr-prepared-moved-{}", Uuid::random().0));
            fs::rename(&root, &moved).unwrap();
            assert!(recheck_product_inputs(&inputs, Some(&mut failed)).is_err());
            fs::rename(&moved, &root).unwrap();
            recheck_inputs(&inputs).unwrap();
            assert!(recheck_product_inputs(&inputs, Some(&mut failed)).is_err());
            assert!(finish_authorization(failed, "FINISH ORIGINAL").is_none());
        } else {
            assert!(read_prepared_install(&path, &expected).is_err());
        }
        let duplicate = format!(
            "{{\"version\":1,{}",
            std::str::from_utf8(&bytes)
                .unwrap()
                .strip_prefix('{')
                .unwrap()
        );
        assert!(
            goatr::native_json::parse(duplicate.as_bytes(), goatr::native_json::REGISTRY).is_err()
        );
        assert!(
            parse_pinned_prepared_bytes(duplicate.as_bytes(), &digest_text(&duplicate)).is_err()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_prepared_install(&path, &expected).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[tokio::test]
    async fn private_control_reply_matches_android_big_endian_length_and_exact_json_without_trailer()
     {
        timeout(Duration::from_secs(5), async {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let (connected, accepted) = tokio::join!(
                TcpStream::connect(listener.local_addr().unwrap()),
                listener.accept()
            );
            let mut receiver = connected.unwrap();
            let (mut sender, _) = accepted.unwrap();
            let reply = json!({"kind":"wait","sequence":0});
            let expected = serde_json::to_vec(&reply).unwrap();
            let write = async {
                control_reply(&mut sender, &reply).await.unwrap();
                sender.shutdown().await.unwrap();
            };
            let read = async {
                let mut header = [0u8; 4];
                receiver.read_exact(&mut header).await.unwrap();
                let length = u32::from_be_bytes(header) as usize;
                // Android readInt admission must precede allocation, including with the broken writer.
                assert!(
                    (2..=65536).contains(&length),
                    "private reply is not an Android bounded big-endian frame: {length}"
                );
                assert_eq!(length, expected.len());
                let mut payload = vec![0; length];
                receiver.read_exact(&mut payload).await.unwrap();
                assert_eq!(payload, expected);
                assert_eq!(serde_json::from_slice::<Value>(&payload).unwrap(), reply);
                let mut trailer = [0u8; 1];
                assert_eq!(
                    receiver.read(&mut trailer).await.unwrap(),
                    0,
                    "private reply has a newline or other trailer"
                );
            };
            tokio::join!(write, read);
        })
        .await
        .expect("owned private-control framing exchange exceeded its deadline");
    }
    #[tokio::test]
    async fn private_control_reply_accepts_maximum_and_refuses_invalid_bounds_before_header() {
        timeout(Duration::from_secs(6), async {
            let (mut writer, mut reader) = tokio::io::duplex(65540);
            let value = json!("x".repeat(65534));
            control_reply(&mut writer, &value).await.unwrap();
            writer.shutdown().await.unwrap();
            let mut header = [0; 4];
            reader.read_exact(&mut header).await.unwrap();
            assert_eq!(u32::from_be_bytes(header), 65536);
            let mut payload = vec![0; 65536];
            reader.read_exact(&mut payload).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&payload).unwrap(), value);
            let mut trailer = [0];
            assert_eq!(reader.read(&mut trailer).await.unwrap(), 0);
            for value in [json!(0), json!("x".repeat(65535))] {
                let (mut writer, mut reader) = tokio::io::duplex(16);
                assert!(control_reply(&mut writer, &value).await.is_err());
                writer.shutdown().await.unwrap();
                let mut byte = [0];
                assert_eq!(
                    reader.read(&mut byte).await.unwrap(),
                    0,
                    "invalid response emitted a partial header"
                );
            }
        })
        .await
        .expect("bounded frame test expired");
    }
    #[tokio::test]
    async fn private_control_reply_truncated_peer_is_an_error_and_erasure_uses_owned_bytes() {
        timeout(Duration::from_secs(6), async {
            let (mut writer, mut reader) = tokio::io::duplex(4);
            let reply = json!({"kind":"wait","sequence":0});
            let write = async { control_reply(&mut writer, &reply).await };
            let truncate = async move {
                let mut prefix = [0; 2];
                reader.read_exact(&mut prefix).await.unwrap();
                drop(reader);
                prefix
            };
            let (written, prefix) = tokio::join!(write, truncate);
            assert!(written.is_err());
            assert_eq!(prefix, [0, 0]);
            let mut owned = ControlReplyBytes(Vec::with_capacity(65536));
            io::Write::write_all(&mut owned, b"owned private response").unwrap();
            erase_control_reply(&mut owned.0);
            assert!(owned.0.iter().all(|byte| *byte == 0));
        })
        .await
        .expect("truncated private frame test expired");
    }
    #[tokio::test]
    async fn private_control_reply_backpressure_keeps_the_existing_five_second_timeout() {
        let (mut writer, _unread) = tokio::io::duplex(1);
        let reply = json!({"kind":"wait"});
        let result = timeout(Duration::from_secs(6), control_reply(&mut writer, &reply))
            .await
            .expect("control reply extended its original timeout");
        assert!(result.is_err());
    }
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::{net::UnixStream as FileWakeChannel, process::CommandExt},
    };
    struct FileWakeFixtureChild(std::process::Child);
    impl FileWakeFixtureChild {
        fn exit(&mut self) -> std::process::ExitStatus {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = self.0.try_wait().unwrap() {
                    return status;
                }
                assert!(Instant::now() < deadline, "owned fixture exit timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    impl Drop for FileWakeFixtureChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn file_wake_endpoint(key: &str) -> Option<FileWakeChannel> {
        let fd = std::env::var(key).ok()?.parse::<i32>().unwrap();
        assert!(fd > 2);
        let channel = unsafe { FileWakeChannel::from_raw_fd(fd) };
        channel
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        channel
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Some(channel)
    }
    fn file_wake_state(channel: &mut FileWakeChannel, expected: u8) {
        let mut state = [0];
        channel.read_exact(&mut state).unwrap();
        assert_eq!(state, [expected]);
    }
    #[test]
    fn serve_profile_failure_from_owned_process_stays_latched_after_inspectability_restore_and_refuses_finish()
     {
        if let Some(mut channel) = file_wake_endpoint("GOATR_PROFILE_LATCH_TEST_FD") {
            goatr::codex_relay::drop_inherited_capabilities().unwrap();
            assert_eq!(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) }, 0);
            channel.write_all(b"0").unwrap();
            file_wake_state(&mut channel, b'1');
            assert_eq!(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0) }, 0);
            channel.write_all(b"1").unwrap();
            file_wake_state(&mut channel, b'2');
            return;
        }
        let test = concat!(
            module_path!(),
            "::serve_profile_failure_from_owned_process_stays_latched_after_inspectability_restore_and_refuses_finish"
        );
        let test = test
            .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
            .unwrap();
        let (mut channel, endpoint) = FileWakeChannel::pair().unwrap();
        channel
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        channel
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let fd = endpoint.as_raw_fd();
        assert!(fd > 2);
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", test, "--test-threads=1"])
            .env("GOATR_PROFILE_LATCH_TEST_FD", fd.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                goatr::codex_relay::drop_inherited_capabilities()?;
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = FileWakeFixtureChild(command.spawn().unwrap());
        drop(endpoint);
        file_wake_state(&mut channel, b'0');
        let original = goatr::codex_relay::ProcessStart::capture(child.0.id()).unwrap();
        let denied = original.evidence(child.0.id());
        assert_eq!(
            denied.as_ref().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        // Failure-only consumer of the actual owned subject; no positive unit authority is fabricated.
        let mut failed = false;
        assert!(retain_profile_observation(denied, true, &mut failed).is_err());
        assert!(finish_authorization(failed, "FINISH ORIGINAL").is_none());
        channel.write_all(b"1").unwrap();
        file_wake_state(&mut channel, b'1');
        let restored = original.evidence(child.0.id()).unwrap();
        assert!(retain_profile_observation(Ok(restored), false, &mut failed).is_err());
        assert!(finish_authorization(failed, "FINISH ORIGINAL").is_none());
        let mut unrelated = false;
        assert!(
            retain_profile_observation::<()>(
                Err(io::Error::from_raw_os_error(libc::EACCES)),
                false,
                &mut unrelated
            )
            .is_err()
        );
        assert!(!unrelated);
        channel.write_all(b"2").unwrap();
        assert!(child.exit().success());
    }
    #[test]
    fn host_guard_captures_original_errno_kind_and_location_without_private_reason() {
        let mut diagnostic = HostDiagnostic::default();
        let error = host_guard(
            "manager_ns_pid_stat",
            io::Error::from_raw_os_error(libc::EACCES),
            Some(&mut diagnostic),
        );
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(diagnostic.errno, Some(13));
        assert_eq!(diagnostic.guard, "manager_ns_pid_stat");
        assert_eq!(error.raw_os_error(), None); // located_message is NOT an errno-preserving io::Error.
        assert!(crate::helper_error_location(&error).is_some());
        let original = crate::HelperLocation::from_parts("examples/smoke/physical.rs", 23).unwrap();
        let error = crate::located_message(
            io::ErrorKind::InvalidData,
            "PRIVATE filesystem path/token/error text",
            Some(original),
        );
        let error = host_guard("manager_evidence", error, Some(&mut diagnostic));
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(diagnostic.errno, None);
        assert_eq!(crate::helper_error_location(&error), Some(original));
        assert_eq!(
            error.to_string(),
            "guard=manager_evidence; errno=none; no mutation"
        );
        assert!(
            !crate::helper_error_metadata(&error)
                .to_string()
                .contains("PRIVATE")
        );
        let error = host_guard(
            "caller_evidence",
            io::Error::from_raw_os_error(libc::EACCES),
            None,
        );
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("errno=13"));
    }
    #[test]
    fn host_preflight_report_is_closed_and_never_claims_no_manager_socket_contact() {
        let run = Uuid::random();
        let mut diagnostic = HostDiagnostic::default();
        let error = host_guard(
            "manager_exe_open",
            io::Error::from_raw_os_error(libc::EACCES),
            Some(&mut diagnostic),
        );
        let report = host_preflight_report(
            &run,
            ControllerPolicy::Zero,
            false,
            &diagnostic,
            Some(&error),
        );
        assert_eq!(report["runId"], json!(run));
        assert_eq!(report["status"], "blocked");
        assert_eq!(report["qualification"], "refused");
        assert_eq!(report["guard"], "manager_exe_open");
        assert_eq!(report["errno"], 13);
        assert_eq!(report["io_kind"], "permission_denied");
        for field in [
            "managerCommand",
            "managerCreatedOrRestarted",
            "nativeLaunch",
            "productInstall",
            "ADB",
            "provider",
            "runtimeAcceptanceClaim",
        ] {
            assert_eq!(report[field], false);
        }
        assert!(report.get("managerContact").is_none());
        assert!(report.get("reason").is_none());
        let error = host_guard(
            "human_tty",
            prerequisite("PRIVATE non-TTY detail"),
            Some(&mut diagnostic),
        );
        let refused = host_preflight_report(
            &run,
            ControllerPolicy::Zero,
            false,
            &diagnostic,
            Some(&error),
        );
        assert_eq!(refused["qualification"], "not_attempted");
        assert!(refused["errno"].is_null());
        assert!(!refused.to_string().contains("PRIVATE"));
        // Report formatting is not a mock successful host admission.
        let qualified = host_preflight_report(
            &run,
            ControllerPolicy::Zero,
            true,
            &HostDiagnostic::default(),
            None,
        );
        assert_eq!(qualified["qualification"], "qualified");
        assert_eq!(qualified["guard"], "qualified");
        assert_eq!(qualified["io_kind"], "none");
        assert_eq!(qualified["runtimeAcceptanceClaim"], false);
        assert!(qualified["errno"].is_null());
    }
    #[test]
    fn host_diagnostic_read_bounds_real_owned_files_without_export_or_mutation() {
        let root = std::env::temp_dir().join(format!("goatr-host-diagnostic-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("owned-proc-fixture");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.set_len(65_536).unwrap();
        drop(file);
        let before = binary_sha256(&path).unwrap();
        let mut diagnostic = HostDiagnostic::default();
        host_probe_read(&path, "caller_stat_read", &mut diagnostic).unwrap();
        assert_eq!(binary_sha256(&path).unwrap(), before);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(65_537)
            .unwrap();
        let before = binary_sha256(&path).unwrap();
        let error = host_probe_read(&path, "caller_stat_read", &mut diagnostic).unwrap_err();
        assert_eq!(diagnostic.guard, "caller_stat_read");
        assert_eq!(diagnostic.errno, None);
        assert!(!error.to_string().contains(path.to_str().unwrap()));
        assert_eq!(binary_sha256(&path).unwrap(), before);
        let error = host_probe_read(
            &root.join("PRIVATE-missing"),
            "caller_status_read",
            &mut diagnostic,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(diagnostic.errno, Some(libc::ENOENT));
        assert!(!error.to_string().contains("PRIVATE"));
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    fn host_status_fixture() -> &'static str {
        "Name:\tPRIVATE-process-name\nGroups:\t1000 1234\nCapBnd:\tffffffffffffffff\nCapInh:\t0000000000000000\nCapPrm:\t 0000000800000000 \nCapEff:\t00000000000000af\nCapAmb:\t0000000000000000\n"
    }
    #[test]
    fn host_procfs_numbering_gate_refuses_before_numeric_target_access() {
        let stat = fs::read_to_string("/proc/self/stat").unwrap();
        let status = fs::read_to_string("/proc/self/status").unwrap();
        let actual = std::process::id();
        let wrong = actual.checked_add(1).unwrap();
        let bad_stat = stat.replacen(&format!("{actual} "), &format!("{wrong} "), 1);
        assert_ne!(bad_stat, stat);
        let bad_status = status
            .lines()
            .map(|line| {
                if line.starts_with("NSpid:") {
                    format!("NSpid:\t{wrong}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(bad_status, status);
        let mut diagnostic = HostDiagnostic::default();
        host_validate_proc_view(stat.as_bytes(), status.as_bytes(), &mut diagnostic).unwrap();
        assert!(diagnostic.proc_view_validated);
        for (stat, status) in [
            (bad_stat.as_bytes(), status.as_bytes()),
            (stat.as_bytes(), bad_status.as_bytes()),
        ] {
            // A previous successful capture is deliberately stale; a failed new view must erase it.
            diagnostic.proc_view_validated = true;
            let error = host_validate_proc_view(stat, status, &mut diagnostic).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(diagnostic.guard, "caller_procfs_view");
            assert!(!diagnostic.proc_view_validated);
            // This is the actual production numeric-read entry, not a copied decision or host success.
            // Without the PRE gate, /proc/0 metadata would instead fail with a kernel ENOENT.
            let error = host_manager_reads(0, &mut diagnostic).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(diagnostic.guard, "manager_procfs_view_required");
            assert_eq!(diagnostic.errno, None);
        }
    }
    #[test]
    fn host_caps_parser_exports_exactly_four_canonical_masks_and_refuses_bad_fields() {
        let parsed =
            serde_json::to_value(host_caps(host_status_fixture().as_bytes()).unwrap()).unwrap();
        assert_eq!(
            parsed,
            json!({"CapInh":"0000000000000000","CapPrm":"0000000800000000","CapEff":"00000000000000af","CapAmb":"0000000000000000"})
        );
        assert_eq!(parsed.as_object().unwrap().len(), 4);
        assert!(!parsed.to_string().contains("PRIVATE"));
        assert!(parsed.get("CapBnd").is_none());
        assert!(parsed.get("Groups").is_none());
        for value in parsed.as_object().unwrap().values() {
            let value = value.as_str().unwrap();
            assert_eq!(value.len(), 16);
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
        }
        let valid = host_status_fixture();
        for key in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
            let prefix = format!("{key}:");
            let field = valid
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap();
            let missing = valid
                .lines()
                .filter(|line| !line.starts_with(&prefix))
                .collect::<Vec<_>>()
                .join("\n");
            for malformed in [missing, format!("{valid}{field}\n")] {
                assert!(host_caps(malformed.as_bytes()).is_err(), "{key}");
            }
        }
        for malformed in [
            valid.replace("00000000000000af", "00000000000000AF"),
            valid.replace("00000000000000af", "00000000000000a"),
            valid.replace("00000000000000af", "10000000000000000"),
            valid.replace("00000000000000af", "0000000000000x0f"),
            valid.replace("00000000000000af", "-00000000000000f"),
        ] {
            let error = host_caps(malformed.as_bytes()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }
    #[test]
    fn host_status_same_read_capture_clears_failed_samples_and_closes_report_fields() {
        let root = std::env::temp_dir().join(format!("goatr-host-status-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("owned-status");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(host_status_fixture().as_bytes()).unwrap();
        drop(file);
        let before = binary_sha256(&path).unwrap();
        let mut diagnostic = HostDiagnostic::default();
        host_status(&path, "self_status_read", false, &mut diagnostic).unwrap();
        host_status(&path, "manager_status_read", true, &mut diagnostic).unwrap();
        assert_eq!(binary_sha256(&path).unwrap(), before);
        let error = host_guard(
            "manager_ns_pid_stat",
            io::Error::from_raw_os_error(libc::EACCES),
            Some(&mut diagnostic),
        );
        let report = host_preflight_report(
            &Uuid::random(),
            ControllerPolicy::Zero,
            false,
            &diagnostic,
            Some(&error),
        );
        assert_eq!(report["callerCaps"].as_object().unwrap().len(), 4);
        assert_eq!(report["managerCaps"].as_object().unwrap().len(), 4);
        assert!(!report.to_string().contains("PRIVATE"));
        assert!(report.get("procViewValidated").is_none());
        assert!(report.get("CapBnd").is_none());
        let error = host_status(
            &root.join("PRIVATE-missing"),
            "self_status_read",
            false,
            &mut diagnostic,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(diagnostic.guard, "self_status_read");
        assert_eq!(diagnostic.errno, Some(libc::ENOENT));
        assert!(diagnostic.caller_caps.is_none());
        assert!(diagnostic.manager_caps.is_some());
        let report = host_preflight_report(
            &Uuid::random(),
            ControllerPolicy::Zero,
            false,
            &diagnostic,
            Some(&error),
        );
        assert!(report["callerCaps"].is_null());
        fs::write(
            &path,
            format!("{}CapInh:\t0000000000000000\n", host_status_fixture()),
        )
        .unwrap();
        let error = host_status(&path, "manager_status_read", true, &mut diagnostic).unwrap_err();
        assert_eq!(diagnostic.guard, "manager_cap_fields");
        assert_eq!(diagnostic.errno, None);
        assert!(diagnostic.manager_caps.is_none());
        let report = host_preflight_report(
            &Uuid::random(),
            ControllerPolicy::Zero,
            false,
            &diagnostic,
            Some(&error),
        );
        assert!(report["managerCaps"].is_null());
        fs::write(&path, host_status_fixture()).unwrap();
        host_status(&path, "self_status_read", false, &mut diagnostic).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(65_537)
            .unwrap();
        assert!(host_status(&path, "self_status_read", false, &mut diagnostic).is_err());
        assert!(diagnostic.caller_caps.is_none());
        assert_eq!(diagnostic.guard, "self_status_read");
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn host_terminal_partial_write_failure_has_one_owner_and_failure_exit() {
        struct PartialFailure {
            remaining: usize,
            bytes: Vec<u8>,
            calls: usize,
        }
        impl Write for PartialFailure {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                if self.remaining == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "PRIVATE terminal failure",
                    ));
                }
                let count = bytes.len().min(self.remaining);
                self.bytes.extend_from_slice(&bytes[..count]);
                self.remaining -= count;
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        // Only the actual writer/outcome boundary is under test; no host qualification is mocked.
        let report = json!({"gate":"owned-output-consumer","runtimeAcceptanceClaim":false});
        for budget in [0, 7] {
            let mut output = PartialFailure {
                remaining: budget,
                bytes: Vec::new(),
                calls: 0,
            };
            let passed = host_terminal_report(&mut output, &report, true);
            assert!(!passed);
            if budget > 0 {
                assert!(!output.bytes.is_empty());
            }
            let calls = output.calls;
            let bytes = output.bytes.len();
            assert_eq!(
                crate::mode_exit(Ok(passed), &mut output),
                std::process::ExitCode::FAILURE
            );
            assert_eq!(output.calls, calls);
            assert_eq!(output.bytes.len(), bytes);
            assert!(!String::from_utf8_lossy(&output.bytes).contains("selected-smoke-mode"));
        }
        let mut output = Vec::new();
        let passed = host_terminal_report(&mut output, &report, true);
        assert!(passed);
        let length = output.len();
        assert_eq!(
            crate::mode_exit(Ok(passed), &mut output),
            std::process::ExitCode::SUCCESS
        );
        assert_eq!(output.len(), length);
        output.clear();
        assert_eq!(
            crate::mode_exit(
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "closed selection refused"
                )),
                &mut output
            ),
            std::process::ExitCode::FAILURE
        );
        let generic: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(generic["gate"], "selected-smoke-mode");
        assert_eq!(generic["io_kind"], "invalid_input");
    }
    fn controller_status(mask: &str) -> String {
        let masks: String = ["CapInh", "CapPrm", "CapEff", "CapAmb"]
            .into_iter()
            .map(|key| format!("{key}:\t{mask}\n"))
            .collect();
        format!("{masks}NoNewPrivs:\t1\n")
    }
    #[test]
    fn controller_zero_and_wake_admission_require_distinct_exact_four_mask_shapes() {
        let unit = format!("goatr-host-wake-{}.service", Uuid::random().0);
        let wake = ControllerPolicy::WakeDiagnostic { unit: &unit };
        let zero_status = controller_status("0000000000000000");
        let wake_status = controller_status("0000000800000000");
        controller_caps_admission(ControllerPolicy::Zero, zero_status.as_bytes()).unwrap();
        controller_caps_admission(wake, wake_status.as_bytes()).unwrap();
        assert!(controller_caps_admission(ControllerPolicy::Zero, wake_status.as_bytes()).is_err());
        assert!(controller_caps_admission(wake, zero_status.as_bytes()).is_err());
        for bad in [
            wake_status.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
            wake_status.replace("NoNewPrivs:\t1\n", ""),
            format!("{wake_status}NoNewPrivs:\t1\n"),
        ] {
            assert!(controller_caps_admission(wake, bad.as_bytes()).is_err());
        }
        for key in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
            let field = format!("{key}:\t0000000800000000\n");
            for mask in [
                "0000000000000000",
                "0000000000000001",
                "0000000800000001",
                "0000000000080000",
                "000000080000000A",
                "10000000800000000",
            ] {
                let changed = wake_status.replace(&field, &format!("{key}:\t{mask}\n"));
                assert!(
                    controller_caps_admission(wake, changed.as_bytes()).is_err(),
                    "{key}/{mask}"
                );
            }
            assert!(
                controller_caps_admission(wake, wake_status.replace(&field, "").as_bytes())
                    .is_err(),
                "{key}/missing"
            );
            assert!(
                controller_caps_admission(wake, format!("{wake_status}{field}").as_bytes())
                    .is_err(),
                "{key}/duplicate"
            );
            assert!(
                controller_caps_admission(
                    ControllerPolicy::Zero,
                    zero_status
                        .replace(&format!("{key}:\t0000000000000000\n"), &field)
                        .as_bytes()
                )
                .is_err(),
                "{key}/zero-extra-bit"
            );
        }
    }
    #[test]
    fn controller_unit_and_unified_cgroup_admission_reject_aliases_and_nonleaf_membership() {
        let unit = format!("goatr-host-wake-{}.service", Uuid::random().0);
        validate_controller_unit(&unit).unwrap();
        for bad in [
            unit.to_uppercase(),
            unit.replace("goatr-host-wake-", "daily-"),
            unit.replace(".service", ".scope"),
            format!("/private/{unit}"),
            format!("{unit}.service"),
            "goatr-host-wake-not-a-uuid.service".to_owned(),
        ] {
            assert!(validate_controller_unit(&bad).is_err());
        }
        let good = format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}\n");
        let other = format!("goatr-host-wake-{}.service", Uuid::random().0);
        for bad in [
            String::new(),
            format!("{good}{good}"),
            format!("{good}1:name=systemd:/other\n"),
            format!("{good}\n"),
            good.replace('\n', "\r\n"),
            good.replace("0::", "1::"),
            good.replace("0::", "0:cpu:"),
            good.replace("0::/", "0::"),
            good.replace("/app.slice/", "/app.slice//"),
            good.replace("/app.slice/", "/app.slice/./"),
            good.replace("/app.slice/", "/app.slice/../"),
            good.replace(&unit, &other),
            good.replace(&unit, &format!("{unit}/child")),
            good.replace(&unit, &format!("{unit}/")),
            good.replace("/app.slice/", "/app.slice :/"),
            good.replace("/app.slice/", "/app.slice\t/"),
        ] {
            let mut diagnostic = HostDiagnostic::default();
            let error =
                controller_cgroup_admission(&unit, bad.as_bytes(), &mut diagnostic).unwrap_err();
            assert!(!diagnostic.controller_unit_validated);
            assert!(!diagnostic.manager_command_observed);
            assert!(diagnostic.controller_unit.is_none());
            let report = host_preflight_report(
                &Uuid::random(),
                ControllerPolicy::WakeDiagnostic { unit: &unit },
                false,
                &diagnostic,
                Some(&error),
            );
            assert_eq!(report["managerCommand"], false);
            assert_eq!(report["controllerUnitValidated"], false);
            assert_eq!(report["runtimeAcceptanceClaim"], false);
        }
        let mut diagnostic = HostDiagnostic::default();
        controller_cgroup_admission(&unit, good.as_bytes(), &mut diagnostic).unwrap();
        assert!(diagnostic.controller_unit_validated);
        assert!(diagnostic.manager_command_observed);
        assert_eq!(diagnostic.controller_unit.as_deref(), Some(unit.as_str()));
    }
    #[test]
    fn wake_report_observes_unit_not_selector_and_final_read_failure_keeps_only_historical_effect()
    {
        let unit = format!("goatr-host-wake-{}.service", Uuid::random().0);
        let policy = ControllerPolicy::WakeDiagnostic { unit: &unit };
        let run = Uuid::random();
        let mut diagnostic = HostDiagnostic::default();
        let error = host_guard(
            "wake_controller_capabilities",
            io::Error::new(io::ErrorKind::PermissionDenied, "closed mask refusal"),
            Some(&mut diagnostic),
        );
        let before = host_preflight_report(&run, policy, false, &diagnostic, Some(&error));
        assert_eq!(before["wakeDiagnostic"], true);
        assert_eq!(before["expectedControllerUnit"], unit);
        assert_eq!(before["managerCommand"], false);
        assert_eq!(before["controllerUnitValidated"], false);
        {
            let mut not_attempted = HostDiagnostic::default();
            let error = host_guard(
                "interactive_tty",
                prerequisite("actual host TTY required"),
                Some(&mut not_attempted),
            );
            let report = host_preflight_report(&run, policy, false, &not_attempted, Some(&error));
            assert_eq!(report["qualification"], "not_attempted");
            assert_eq!(report["managerCommand"], false);
            assert_eq!(report["controllerUnitValidated"], false);
        }
        let root = std::env::temp_dir().join(format!("goatr-wake-unit-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("owned-cgroup");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(format!("0::/owned.slice/{unit}\n").as_bytes())
            .unwrap();
        drop(file);
        let hash = binary_sha256(&path).unwrap();
        controller_cgroup_read(&unit, &path, &mut diagnostic).unwrap();
        assert_eq!(binary_sha256(&path).unwrap(), hash);
        let after = host_preflight_report(&run, policy, false, &diagnostic, Some(&error));
        assert_eq!(after["managerCommand"], true);
        assert_eq!(after["controllerUnitValidated"], true);
        assert_eq!(after["controllerUnit"], unit);
        fs::remove_file(&path).unwrap();
        let error = controller_cgroup_read(&unit, &path, &mut diagnostic).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(diagnostic.errno, Some(libc::ENOENT));
        assert_eq!(diagnostic.guard, "wake_controller_cgroup_read");
        let final_report = host_preflight_report(&run, policy, false, &diagnostic, Some(&error));
        assert_eq!(final_report["qualification"], "refused");
        assert_eq!(final_report["managerCommand"], true);
        assert_eq!(final_report["controllerUnitValidated"], false);
        assert!(final_report["controllerUnit"].is_null());
        fs::write(
            &path,
            format!(
                "0::/owned.slice/goatr-host-wake-{}.service\n",
                Uuid::random().0
            ),
        )
        .unwrap();
        let error = controller_cgroup_read(&unit, &path, &mut diagnostic).unwrap_err();
        assert!(diagnostic.manager_command_observed);
        assert!(!diagnostic.controller_unit_validated);
        let ordinary = host_preflight_report(
            &run,
            ControllerPolicy::Zero,
            false,
            &diagnostic,
            Some(&error),
        );
        assert_eq!(ordinary["wakeDiagnostic"], false);
        assert_eq!(ordinary["managerCommand"], false);
        assert_eq!(ordinary["controllerUnitValidated"], false);
        assert!(ordinary["expectedControllerUnit"].is_null());
        assert!(ordinary["controllerUnit"].is_null());
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    fn empty_connector(exists: bool) -> Value {
        json!({"databaseExists":exists,"version":if exists{1}else{0},"schemaSha256":if exists{json!("1".repeat(64))}else{Value::Null},
            "platformMetadataSha256":if exists{json!("f".repeat(64))}else{Value::Null},"localeChanges":0,"legacyFilePresent":false,"legacyKeyCount":0,"distributors":[],"registrations":[],"tokens":[],"keys":[]})
    }
    fn official_connector() -> Value {
        let mut value = empty_connector(true);
        value["distributors"] = json!([{"rowId":1,"sha256":"2".repeat(64),"distributor":"io.heckel.ntfy","fallbackFrom":null,"ack":true,"dateInsertion":17}]);
        value
    }
    fn retained(connector: Value) -> RetainedFacts {
        RetainedFacts {
            schema: 1,
            database_exists: true,
            hosts: Vec::new(),
            preferences: Vec::new(),
            aliases: Vec::new(),
            alias_sha256: digest_text(""),
            service_requested: false,
            service_requested_present: false,
            run_label_collision: false,
            connector,
            database: json!({"schemaSha256":"d".repeat(64),"globalSha256":"e".repeat(64),"measuredAt":1,
                "quiescence":DB8_QUIESCENCE.iter().map(|name|((*name).to_owned(),json!(0))).collect::<serde_json::Map<_,_>>()}),
            orphans: json!([]),
            key_files: json!({"directory":null,"files":[]}),
        }
    }
    fn fresh_transition_facts() -> (RetainedFacts, RetainedFacts, RetainedFacts) {
        let mut empty = retained(empty_connector(false));
        empty.database_exists = false;
        empty.database =
            json!({"schemaSha256":null,"globalSha256":null,"measuredAt":1,"quiescence":{}});
        let mut constructor = empty.clone();
        constructor.key_files["directory"] = json!({"device":1,"inode":2,"mode":0o700,"uid":10001});
        let mut opened = retained(empty_connector(false));
        opened.key_files["directory"] = constructor.key_files["directory"].clone();
        add_key_pair(&mut opened, "goatr.installation.endpoint");
        (empty, constructor, opened)
    }
    fn add_key_pair(facts: &mut RetainedFacts, alias: &str) {
        if facts.key_files["directory"].is_null() {
            facts.key_files["directory"] = json!({"device":1,"inode":2,"mode":0o700,"uid":10001});
        }
        let files = facts.key_files["files"].as_array_mut().unwrap();
        for marker in [false, true] {
            files.push(json!({"name":format!("{alias}.gcm{}",if marker{".initialized"}else{""}),"sha256":digest_text(if marker{"GAI1"}else{"owned-public-ciphertext-fixture"}),
                "device":1,"inode":100+files.len(),"size":if marker{4}else{61},"mode":0o600,"uid":10001,"gid":10001,"links":1,"mtime":1,"ctime":1}));
        }
        if !facts.aliases.iter().any(|present| present == alias) {
            facts.aliases.push(alias.to_owned());
            facts.aliases.sort();
        }
        facts.alias_sha256 = digest_text(&facts.aliases.join("\0"));
    }
    fn with_private(before: &RetainedFacts, private: &str) -> RetainedFacts {
        let mut after = before.clone();
        let alias = format!("goatr.host.{}.receipt", digest_text(private));
        add_key_pair(&mut after, &alias);
        after.hosts.push(RetainedHost {
            host: private.to_owned(),
            device: Some(Uuid::random().0),
            metadata_sha256: "3".repeat(64),
            rows_sha256: "4".repeat(64),
            action_count: 0,
            registration_generation: None,
            registration_sha256: None,
            mode: "Persistent".to_owned(),
            enabled: false,
            completed: false,
            key_aliases: vec![alias],
        });
        after.run_label_collision = true;
        after
    }
    fn register_private(after: &mut RetainedFacts, private: &str) {
        if after.connector["distributors"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            after.connector = official_connector();
        }
        let instance = digest_text(&format!("goatr.{APP}.{private}"));
        for (table, extra) in [
            (
                "registrations",
                json!({"messageMatchesPrivate":true,"vapidNull":true}),
            ),
            ("tokens", json!({"distributor":"io.heckel.ntfy"})),
            ("keys", json!({"sealed":true})),
        ] {
            let rows = after.connector[table].as_array_mut().unwrap();
            let row_id = rows.len() + 1;
            let mut row = json!({"rowId":row_id,"sha256":"5".repeat(64),"instanceSha256":instance});
            row.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            rows.push(row);
        }
        let host = after
            .hosts
            .iter_mut()
            .find(|host| host.host == private)
            .unwrap();
        host.registration_generation = Some(Uuid::random().0);
        host.registration_sha256 = Some("6".repeat(64));
        host.mode = "Ntfy".to_owned();
        host.enabled = true;
        if !after.aliases.iter().any(|alias| alias == "UnifiedPush") {
            after.aliases.push("UnifiedPush".to_owned());
            after.aliases.sort();
        }
    }
    #[test]
    fn retained_optional_legacy_device_metadata_is_preserved_not_adopted_as_private_pairing() {
        let mut before = retained(empty_connector(false));
        before.aliases.push("retained-legacy-key".to_owned());
        before.alias_sha256 = digest_text("retained-legacy-key");
        before.hosts.push(RetainedHost {
            host: "b".repeat(64),
            device: None,
            metadata_sha256: "3".repeat(64),
            rows_sha256: "4".repeat(64),
            action_count: 2,
            registration_generation: None,
            registration_sha256: None,
            mode: "Persistent".to_owned(),
            enabled: false,
            completed: false,
            key_aliases: vec!["retained-legacy-key".to_owned()],
        });
        assert!(RetainedFacts::decode(&serde_json::to_value(&before).unwrap()).is_ok());
        let private = "a".repeat(64);
        let after = with_private(&before, &private);
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_ok()
        );
        assert!(
            after
                .hosts
                .iter()
                .find(|host| host.host == "b".repeat(64))
                .unwrap()
                .device
                .is_none()
        );
    }
    #[test]
    fn connector_admission_keeps_fresh_setup_and_acknowledged_empty_official_selection_reachable() {
        for connector in [
            empty_connector(false),
            empty_connector(true),
            official_connector(),
        ] {
            let facts = retained(connector);
            assert!(connector_admission(&facts, Some(&"a".repeat(64))).is_ok());
            assert!(RetainedFacts::decode(&serde_json::to_value(&facts).unwrap()).is_ok());
        }
    }
    #[test]
    fn connector_admission_refuses_legacy_other_fallback_unacked_and_private_collision() {
        for (field, value) in [
            ("distributor", json!("other.distributor")),
            ("fallbackFrom", json!("other.distributor")),
            ("ack", json!(false)),
        ] {
            let mut facts = retained(official_connector());
            facts.connector["distributors"][0][field] = value;
            assert!(connector_admission(&facts, None).is_err(), "{field}");
        }
        let mut legacy = retained(empty_connector(false));
        legacy.connector["legacyKeyCount"] = json!(1);
        assert!(connector_admission(&legacy, None).is_err());
        let private = "a".repeat(64);
        let mut facts = with_private(&retained(official_connector()), &private);
        register_private(&mut facts, &private);
        assert!(connector_admission(&facts, Some(&private)).is_err());
        facts.aliases.retain(|alias| alias != "UnifiedPush");
        assert!(RetainedFacts::decode(&serde_json::to_value(&facts).unwrap()).is_err());
    }
    #[test]
    fn retained_comparison_allows_only_exact_owned_connector_and_alias_additions() {
        let private = "a".repeat(64);
        let before = retained(official_connector());
        let mut after = with_private(&before, &private);
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_ok()
        );
        register_private(&mut after, &private);
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_ok()
        );
        let valid = after.clone();
        after.aliases.push("unowned-alias".to_owned());
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_err()
        );
        after = valid.clone();
        after.connector["tokens"][0]["distributor"] = json!("other.distributor");
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_err()
        );
        after = valid;
        after.connector["distributors"][0]["dateInsertion"] = json!(18);
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_err()
        );
    }
    #[test]
    fn retained_connector_rows_cannot_be_replaced_or_hidden_by_private_registration() {
        let private = "a".repeat(64);
        let mut before = retained(official_connector());
        let instance = digest_text("retained-native-instance");
        before.aliases.push("UnifiedPush".to_owned());
        before.connector["registrations"] = json!([{"rowId":1,"sha256":"7".repeat(64),"instanceSha256":instance,"messageMatchesPrivate":false,"vapidNull":true}]);
        before.connector["tokens"] = json!([{"rowId":1,"sha256":"8".repeat(64),"instanceSha256":instance,"distributor":"io.heckel.ntfy"}]);
        before.connector["keys"] =
            json!([{"rowId":1,"sha256":"9".repeat(64),"instanceSha256":instance,"sealed":true}]);
        let mut after = with_private(&before, &private);
        register_private(&mut after, &private);
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_ok()
        );
        after.connector["tokens"][0]["sha256"] = json!("b".repeat(64));
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_err()
        );
    }
    #[test]
    fn service_requested_absent_default_requires_explicit_owned_delta_permission() {
        let private = "a".repeat(64);
        let before = retained(empty_connector(false));
        let mut after = with_private(&before, &private);
        after.service_requested_present = true;
        after.preferences.push(PreferenceFact {
            file: "delivery_preferences".to_owned(),
            key_sha256: digest_text("service_requested"),
            kind: "boolean".to_owned(),
            value_sha256: digest_text("false"),
        });
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, false)
                .is_err()
        );
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, true)
                .is_ok()
        );
        after.preferences.push(PreferenceFact {
            file: "delivery_preferences".to_owned(),
            key_sha256: digest_text("daily-choice"),
            kind: "boolean".to_owned(),
            value_sha256: digest_text("false"),
        });
        assert!(
            before
                .preserved_with_private_host(&after, &private, false, true)
                .is_err()
        );
    }
    #[test]
    fn durable_action_identity_survives_foreground_receipt_hydration_changes() {
        let action = Uuid::random().0;
        let before = json!({"actionIds":[action],"receipts":[]});
        let after =
            json!({"actionIds":[action],"receipts":[{"action":action,"state":"Dispatched"}]});
        assert_eq!(receipt_ids(&before).unwrap(), receipt_ids(&after).unwrap());
    }
    #[test]
    fn complete_ascii_action_fingerprint_binds_values_and_epoch_without_order_dependency() {
        let first = BTreeMap::from([
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "2".to_owned()),
        ]);
        let second = BTreeMap::from([
            ("b".to_owned(), "2".to_owned()),
            ("a".to_owned(), "1".to_owned()),
        ]);
        assert_eq!(argument_hash(&first).unwrap(), digest_text("1:a1:11:b1:2"));
        assert_eq!(
            argument_hash(&first).unwrap(),
            argument_hash(&second).unwrap()
        );
        let mut changed = second;
        changed.insert("a".to_owned(), "3".to_owned());
        assert_ne!(
            argument_hash(&first).unwrap(),
            argument_hash(&changed).unwrap()
        );
        changed.insert("epoch".to_owned(), "line\nbreak".to_owned());
        assert!(argument_hash(&changed).is_err());
    }
    fn dex_fixture(descriptor: &str, defined: bool) -> Vec<u8> {
        let at = 152usize;
        let mut bytes = vec![0u8; at + descriptor.len() + 2];
        bytes[..8].copy_from_slice(b"dex\n035\0");
        let size = bytes.len() as u32;
        for (offset, value) in [
            (32, size),
            (36, 112),
            (40, 0x12345678),
            (56, 1),
            (60, 112),
            (64, 1),
            (68, 116),
            (96, u32::from(defined)),
            (100, 120),
            (112, at as u32),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[at] = descriptor.len() as u8;
        bytes[at + 1..at + 1 + descriptor.len()].copy_from_slice(descriptor.as_bytes());
        bytes
    }
    #[test]
    fn actual_dex_class_definitions_not_descriptor_strings_qualify_packaged_runners() {
        let descriptor = "Lio/github/trevarj/goatr/PhysicalMvpObserver;";
        let wanted = BTreeSet::from([descriptor.to_owned()]);
        assert!(
            dex_classes(&dex_fixture(descriptor, false), &wanted)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            dex_classes(&dex_fixture(descriptor, true), &wanted).unwrap(),
            wanted
        );
        let mut broken = dex_fixture(descriptor, true);
        broken[120..124].copy_from_slice(&1u32.to_le_bytes());
        assert!(dex_classes(&broken, &wanted).is_err());
    }
    fn startup_provider_sdk_manifest() -> &'static str {
        // Pinned SDK36 APP dump d8b95254f502db22afae3892ebda82e1bb36c19eb2319933d5793dd6786e4af8:
        // keep the actual URI-qualified provider subtree, literal false and quoted Raw values.
        r#"N: android=http://schemas.android.com/apk/res/android (line=3)
  E: manifest (line=3)
    A: package="io.github.trevarj.goatr.debug" (Raw: "io.github.trevarj.goatr.debug")
      E: application (line=25)
          E: provider (line=104)
            A: http://schemas.android.com/apk/res/android:name(0x01010003)="androidx.startup.InitializationProvider" (Raw: "androidx.startup.InitializationProvider")
            A: http://schemas.android.com/apk/res/android:exported(0x01010010)=false
            A: http://schemas.android.com/apk/res/android:authorities(0x01010018)="io.github.trevarj.goatr.debug.androidx-startup" (Raw: "io.github.trevarj.goatr.debug.androidx-startup")
              E: meta-data (line=108)
                A: http://schemas.android.com/apk/res/android:name(0x01010003)="androidx.emoji2.text.EmojiCompatInitializer" (Raw: "androidx.emoji2.text.EmojiCompatInitializer")
                A: http://schemas.android.com/apk/res/android:value(0x01010024)="androidx.startup" (Raw: "androidx.startup")
              E: meta-data (line=111)
                A: http://schemas.android.com/apk/res/android:name(0x01010003)="androidx.lifecycle.ProcessLifecycleInitializer" (Raw: "androidx.lifecycle.ProcessLifecycleInitializer")
                A: http://schemas.android.com/apk/res/android:value(0x01010024)="androidx.startup" (Raw: "androidx.startup")
              E: meta-data (line=114)
                A: http://schemas.android.com/apk/res/android:name(0x01010003)="androidx.profileinstaller.ProfileInstallerInitializer" (Raw: "androidx.profileinstaller.ProfileInstallerInitializer")
                A: http://schemas.android.com/apk/res/android:value(0x01010024)="androidx.startup" (Raw: "androidx.startup")
          E: uses-library (line=119)
"#
    }
    #[test]
    fn startup_provider_manifest_accepts_captured_sdk_uri_and_literal_booleans() {
        let captured = startup_provider_sdk_manifest();
        let enabled=captured.replace(":exported(0x01010010)=false\n",":exported(0x01010010)=false\n            A: http://schemas.android.com/apk/res/android:enabled(0x0101000e)=true\n");
        for manifest in [captured, enabled.as_str()] {
            let nodes = manifest_nodes(manifest).unwrap();
            packaged_process_manifest(&nodes, APP).unwrap();
            packaged_startup_provider(&nodes)
                .expect("the captured non-exported enabled SDK36 provider closure must qualify");
        }
    }
    #[test]
    fn startup_provider_manifest_requires_literal_booleans_without_obsolete_hex_aliases() {
        // Internal key spelling isolates literal-boolean admission from URI decoding.
        let manifest = startup_provider_sdk_manifest()
            .replace("http://schemas.android.com/apk/res/android:", "android:");
        packaged_startup_provider(&manifest_nodes(&manifest).unwrap()).unwrap();
        for value in ["true", "(type 0x12)0x0", "0", "@bool/exported", "unknown"] {
            let changed = manifest.replace(
                "android:exported(0x01010010)=false",
                &format!("android:exported(0x01010010)={value}"),
            );
            assert!(
                packaged_startup_provider(&manifest_nodes(&changed).unwrap()).is_err(),
                "{value}"
            );
        }
        let missing = manifest
            .lines()
            .filter(|line| !line.contains("android:exported("))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(packaged_startup_provider(&manifest_nodes(&missing).unwrap()).is_err());
        for value in [
            "false",
            "(type 0x12)0xffffffff",
            "1",
            "@bool/enabled",
            "unknown",
        ] {
            let changed=manifest.replace("android:exported(0x01010010)=false\n",&format!("android:exported(0x01010010)=false\n            A: android:enabled(0x0101000e)={value}\n"));
            assert!(
                packaged_startup_provider(&manifest_nodes(&changed).unwrap()).is_err(),
                "{value}"
            );
        }
    }
    #[test]
    fn startup_provider_manifest_rejects_provider_and_initializer_closure_mutations() {
        let manifest = startup_provider_sdk_manifest();
        packaged_startup_provider(&manifest_nodes(manifest).unwrap()).unwrap();
        let first = manifest
            .find("              E: meta-data (line=108)")
            .unwrap();
        let next = manifest
            .find("              E: meta-data (line=111)")
            .unwrap();
        let missing_initializer = format!("{}{}", &manifest[..first], &manifest[next..]);
        let extra_initializer=manifest.replace("          E: uses-library (line=119)",
            "              E: meta-data (line=118)\n                A: http://schemas.android.com/apk/res/android:name(0x01010003)=\"unknown.Initializer\"\n                A: http://schemas.android.com/apk/res/android:value(0x01010024)=\"androidx.startup\"\n          E: uses-library (line=119)");
        for changed in [
            manifest.replace("E: provider (line=104)", "E: receiver (line=104)"),
            format!("{manifest}          E: provider (line=125)\n"),
            manifest.replace(
                "androidx.startup.InitializationProvider",
                "unknown.Provider",
            ),
            manifest.replace(
                "io.github.trevarj.goatr.debug.androidx-startup",
                "other.androidx-startup",
            ),
            manifest.replace(
                "androidx.emoji2.text.EmojiCompatInitializer",
                "unknown.Initializer",
            ),
            manifest.replace("=\"androidx.startup\"", "=\"unknown.value\""),
            manifest.replace("E: meta-data (line=108)", "E: receiver (line=108)"),
            missing_initializer,
            extra_initializer,
        ] {
            assert!(packaged_startup_provider(&manifest_nodes(&changed).unwrap()).is_err());
        }
    }
    #[test]
    fn manifest_android_uri_shared_uid_and_process_overrides_remain_forbidden() {
        let manifest = "N: android=http://schemas.android.com/apk/res/android\n  E: manifest\n      E: application\n";
        packaged_process_manifest(&manifest_nodes(manifest).unwrap(), APP).unwrap();
        let ordinary = format!(
            "{manifest}        A: http://schemas.android.com/apk/res/android:process(0x01010011)=\"{APP}\"\n"
        );
        packaged_process_manifest(&manifest_nodes(&ordinary).unwrap(), APP).unwrap();
        let shared=manifest.replace("  E: manifest\n","  E: manifest\n    A: http://schemas.android.com/apk/res/android:sharedUserId(0x0101000b)=\"shared.owner\"\n");
        assert!(packaged_process_manifest(&manifest_nodes(&shared).unwrap(), APP).is_err());
        for process in [":remote", "unrelated.owner"] {
            let changed = format!(
                "{manifest}        A: http://schemas.android.com/apk/res/android:process(0x01010011)=\"{process}\"\n"
            );
            assert!(packaged_process_manifest(&manifest_nodes(&changed).unwrap(), APP).is_err());
        }
    }
    #[test]
    fn manifest_android_uri_instrumentations_keep_distinct_targets_and_no_target_processes() {
        let manifest = format!(
            r#"N: android=http://schemas.android.com/apk/res/android
  E: manifest
      E: application
      E: instrumentation (line=4)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)="io.github.trevarj.goatr.OwnedInstrumentationRunner" (Raw: "io.github.trevarj.goatr.OwnedInstrumentationRunner")
        A: http://schemas.android.com/apk/res/android:targetPackage(0x01010021)="{APP}" (Raw: "{APP}")
      E: instrumentation (line=8)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)="io.github.trevarj.goatr.OwnedUiAutomationRunner" (Raw: "io.github.trevarj.goatr.OwnedUiAutomationRunner")
        A: http://schemas.android.com/apk/res/android:targetPackage(0x01010021)="{TEST}" (Raw: "{TEST}")
"#
        );
        let nodes = manifest_nodes(&manifest).unwrap();
        packaged_process_manifest(&nodes, TEST).unwrap();
        packaged_instrumentations(&nodes).unwrap();
        for line in [4, 8] {
            let marker = format!("      E: instrumentation (line={line})\n");
            let changed=manifest.replace(&marker,&format!("{marker}        A: http://schemas.android.com/apk/res/android:targetProcesses(0x0101054f)=\"*\"\n"));
            assert!(packaged_instrumentations(&manifest_nodes(&changed).unwrap()).is_err());
        }
        let wrong = manifest.replace(
            &format!("targetPackage(0x01010021)=\"{APP}\""),
            &format!("targetPackage(0x01010021)=\"{TEST}\""),
        );
        assert!(packaged_instrumentations(&manifest_nodes(&wrong).unwrap()).is_err());
    }
    #[test]
    fn manifest_foreign_namespace_lookalikes_do_not_gain_android_attribute_authority() {
        for namespace in [
            "https://schemas.android.com/apk/res/android:",
            "http://schemas.android.com/apk/res/androidish:",
            "urn:foreign:android:",
        ] {
            let manifest = format!(
                "E: manifest\n  A: {namespace}sharedUserId(0x0101000b)=\"shared.owner\"\n  E: application\n    A: {namespace}process(0x01010011)=\"other.owner\"\n"
            );
            let nodes = manifest_nodes(&manifest).unwrap();
            assert!(
                nodes[0]
                    .attributes
                    .contains_key(&format!("{namespace}sharedUserId"))
            );
            assert!(!nodes[0].attributes.contains_key("android:sharedUserId"));
            assert!(
                nodes[1]
                    .attributes
                    .contains_key(&format!("{namespace}process"))
            );
            assert!(!nodes[1].attributes.contains_key("android:process"));
            packaged_process_manifest(&nodes, APP).unwrap();
            let provider = startup_provider_sdk_manifest()
                .replace("http://schemas.android.com/apk/res/android:", namespace);
            assert!(packaged_startup_provider(&manifest_nodes(&provider).unwrap()).is_err());
        }
    }
    #[test]
    fn manifest_canonical_attribute_duplicates_reject_quoted_and_unquoted_collisions() {
        let uri_quoted =
            "http://schemas.android.com/apk/res/android:exported(0x01010010)=\"false\"";
        let uri_unquoted = "http://schemas.android.com/apk/res/android:exported(0x01010010)=false";
        let internal_quoted = "android:exported(0x01010010)=\"false\"";
        let internal_unquoted = "android:exported(0x01010010)=false";
        for (first, second) in [
            (uri_quoted, uri_quoted),
            (uri_unquoted, uri_unquoted),
            (uri_quoted, uri_unquoted),
            (uri_unquoted, uri_quoted),
            (uri_quoted, internal_quoted),
            (uri_unquoted, internal_unquoted),
            (uri_quoted, internal_unquoted),
            (internal_unquoted, uri_quoted),
        ] {
            let manifest = format!("E: manifest\n  A: {first}\n  A: {second}\n  E: application\n");
            assert!(manifest_nodes(&manifest).is_err(), "{first}; {second}");
        }
    }
    #[test]
    fn packaged_instrumentation_manifest_metadata_remains_distinct_and_closed() {
        let manifest = format!(
            "E: manifest (line=1)\n  E: application\n  E: instrumentation\n    A: android:name(0x1)=\"io.github.trevarj.goatr.OwnedInstrumentationRunner\"\n    A: android:targetPackage(0x2)=\"{APP}\"\n  E: instrumentation\n    A: android:name(0x1)=\"io.github.trevarj.goatr.OwnedUiAutomationRunner\"\n    A: android:targetPackage(0x2)=\"{TEST}\"\n"
        );
        let nodes = manifest_nodes(&manifest).unwrap();
        packaged_process_manifest(&nodes, TEST).unwrap();
        packaged_instrumentations(&nodes).unwrap();
        for index in [2, 3] {
            let mut changed = manifest_nodes(&manifest).unwrap();
            changed[index]
                .attributes
                .insert("android:targetProcesses".to_owned(), "*".to_owned());
            assert!(packaged_instrumentations(&changed).is_err());
        }
        let mut shared = manifest_nodes(&manifest).unwrap();
        shared[0]
            .attributes
            .insert("android:sharedUserId".to_owned(), "shared.owner".to_owned());
        assert!(packaged_process_manifest(&shared, TEST).is_err());
        let mut process = manifest_nodes(&manifest).unwrap();
        process[1]
            .attributes
            .insert("android:process".to_owned(), APP.to_owned());
        assert!(packaged_process_manifest(&process, TEST).is_err());
        assert!(
            manifest_nodes("E: manifest\n  A: package=\"one\"\n  A: package=\"two\"\n").is_err()
        );
    }
    fn observed_shell() -> (ProductEpoch, ShellTarget, Value) {
        let epoch = ProductEpoch {
            pid: 123,
            start: 456,
            uid: 10001,
            public: [4; 65],
        };
        let target = ShellTarget {
            shell_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let keeper = json!({"running":false,"reconnecting":0,"foregroundNotification":false,"endpointOwned":false,"networkCallback":false,
            "serviceRequested":false,"privateEnabled":false,"privateMode":"Persistent","privateVerified":false,"privateRequiresKeeper":false,"privateCompleted":false,
            "retained":[],"privateRegistrationGeneration":null,"privateRegistrationSha256":null,
            "health":{"globalAvailable":true,"runtimePermissionRequired":false,"attentionChannelEnabled":true,"deliveryChannelEnabled":true}});
        let conversation = json!({"historyCount":0,"historyIds":[],"browsingOlder":false,"hasOlder":false,"viewId":null,"activity":"Idle","controlsRevision":0,
            "optionsSha256":null,"optionsLoading":false,"optionsPending":false,"userMarkerCount":0,"assistantMarkerCount":0,
            "queueAvailable":false,"queueLoaded":false,"queueCount":0,"queueSha256":null,"delivery":"Native","offeredDeliveries":[]});
        let process = json!({"pid":123,"start":"456","uid":10001,"name":APP,"caps":{"CapInh":"0000000000000000","CapPrm":"0000000000000000","CapEff":"0000000000000000","CapAmb":"0000000000000000"}});
        let mut fields = serde_json::Map::new();
        for mut part in [
            json!({"kind":"snapshot","process":process,"host":"a".repeat(64),"device":null,
                "pairing":{"screenPairing":false,"previewPresent":false,"hostMatches":false,"nameMatches":false,"fingerprintMatches":false},
                "foreground":true,"screen":"Shell","selectedHost":"a".repeat(64),"selectedSession":null,"selectedGeneration":null,
                "selectedShell":target.shell_id,"selectedShellGeneration":target.generation,"hello":true,"inventoryReady":true,"mutationReady":true,"route":"Direct"}),
            json!({"sessions":[],"shells":[],"pending":[],"receipts":[],"actionIds":[],"controlBegun":false,"grid":false,"columns":0,"rows":0,"revision":0,"alternate":false,
                "scrollOffset":-1,"scrollMaximum":-1,"scrollViewport":-1,"historyMode":"None","markerOutput":false,"oldInputRetired":true,"agentTerminal":false}),
            json!({"agentDisplay":{"present":false,"sha256":null,"styled":false,"revision":0},"keeper":keeper,"conversation":conversation,
                "network":{"internet":false,"validated":false,"cellular":false,"wifi":false,"ethernet":false,"vpn":false,"usb":false},
                "ui":{"orientation":1,"fontScale":1.0,"imeVisible":false,"interactive":false,"measuredAt":1},
                "shellInput":{"offered":false,"draftBytes":null,"composition":null,"direct":null},"creationReady":false}),
        ] {
            fields.extend(std::mem::take(part.as_object_mut().unwrap()));
        }
        let value = Value::Object(fields);
        (epoch, target, value)
    }
    fn committed_shell(value: &mut Value) {
        value["controlBegun"] = json!(true);
        value["grid"] = json!(true);
        value["columns"] = json!(80);
        value["rows"] = json!(24);
        value["revision"] = json!(1);
        value["oldInputRetired"] = json!(false);
        value["historyMode"] = json!("Live");
        value["scrollOffset"] = json!(0);
        value["scrollMaximum"] = json!(96);
        value["scrollViewport"] = json!(24);
        value["shellInput"]["offered"] = json!(true);
    }
    fn scroll_receipt(target: &ShellTarget, action: &str) -> Value {
        json!({"action":action,"method":"ShellScroll","state":"Succeeded","target":null,"generation":null,"shell":target.shell_id,"shellGeneration":target.generation,
            "sha256":"f".repeat(64),"createdSession":null,"createdGeneration":null,"createdShell":null,"createdShellGeneration":null})
    }
    #[test]
    fn fresh_and_released_shell_metadata_admit_begin_without_waiting_for_grid_and_pending_never_rebegins()
     {
        let (epoch, target, mut value) = observed_shell();
        for retired in [false, true] {
            value["oldInputRetired"] = json!(retired);
            validate_snapshot(&value, &epoch, &"a".repeat(64)).unwrap();
            assert_eq!(
                shell_admission(&value, &target),
                Some(ShellAdmission::Metadata)
            );
            value["controlBegun"] = json!(true);
            assert_eq!(
                shell_admission(&value, &target),
                Some(ShellAdmission::Pending)
            );
            assert_ne!(
                shell_admission(&value, &target),
                Some(ShellAdmission::Metadata)
            );
            value["controlBegun"] = json!(false);
        }
        committed_shell(&mut value);
        assert_eq!(
            shell_admission(&value, &target),
            Some(ShellAdmission::Committed)
        );
        value["selectedShellGeneration"] = json!(Uuid::random());
        assert_eq!(shell_admission(&value, &target), None);
    }
    #[test]
    fn actual_browsing_observations_require_nonzero_newer_then_fresh_bottom_receipt_and_committed_live()
     {
        let (_, target, mut value) = observed_shell();
        committed_shell(&mut value);
        let newer = Uuid::random().0;
        value["receipts"] = json!([scroll_receipt(&target, &newer)]);
        value["scrollOffset"] = json!(48);
        value["historyMode"] = json!("Browsing");
        value["shellInput"]["offered"] = json!(false);
        assert_eq!(nonzero_newer(&value), Some(24));
        assert!(shell_history_ack(&value, &target, &newer, 48));
        value["historyMode"] = json!("Browse");
        assert!(!shell_history_ack(&value, &target, &newer, 48));
        value["historyMode"] = json!("Browsing");
        value["scrollOffset"] = json!(24);
        assert_eq!(nonzero_newer(&value), None);
        let bottom = Uuid::random().0;
        assert!(!shell_history_ack(&value, &target, &bottom, 0));
        value["scrollOffset"] = json!(0);
        value["receipts"] = json!([scroll_receipt(&target, &bottom)]);
        assert!(!shell_history_ack(&value, &target, &bottom, 0));
        value["historyMode"] = json!("Live");
        value["shellInput"]["offered"] = json!(true);
        assert!(shell_history_ack(&value, &target, &bottom, 0));
        let noop = Uuid::random().0;
        assert!(!shell_history_ack(&value, &target, &noop, 0));
        value["receipts"] = json!([scroll_receipt(&target, &noop)]);
        assert!(shell_history_ack(&value, &target, &noop, 0));
        for (field, changed) in [
            ("state", json!("Failed")),
            ("shellGeneration", json!(Uuid::random())),
            ("shell", json!(Uuid::random())),
        ] {
            let mut stale = value.clone();
            stale["receipts"][0][field] = changed;
            assert!(!shell_history_ack(&stale, &target, &noop, 0), "{field}");
        }
        value["oldInputRetired"] = json!(true);
        assert!(!shell_history_ack(&value, &target, &noop, 0));
    }
    #[test]
    fn real_snapshot_validator_accepts_shell_512_by_64_and_rejects_oversize_or_empty_committed_grid()
     {
        let (epoch, _, mut value) = observed_shell();
        committed_shell(&mut value);
        value["columns"] = json!(512);
        value["rows"] = json!(64);
        validate_snapshot(&value, &epoch, &"a".repeat(64)).unwrap();
        for (columns, rows) in [(512, 65), (513, 64), (80, 257), (0, 64), (512, 0)] {
            value["columns"] = json!(columns);
            value["rows"] = json!(rows);
            assert!(
                validate_snapshot(&value, &epoch, &"a".repeat(64)).is_err(),
                "{columns}x{rows}"
            );
        }
    }
    #[test]
    fn provider_catalogue_admission_keeps_omp_offered_controls_without_faking_queue_and_requires_codex_queue()
     {
        let omp = json!({"queueAvailable":false,"offeredDeliveries":["Native","Steer","FollowUp","Aside"]});
        assert!(!provider_controls_admission(Provider::Omp, &omp).unwrap());
        assert!(provider_controls_admission(Provider::Codex, &omp).is_err());
        let codex = json!({"queueAvailable":true,"offeredDeliveries":["Native","Steer","Queue"]});
        assert!(provider_controls_admission(Provider::Codex, &codex).unwrap());
        assert!(provider_controls_admission(Provider::Omp, &codex).is_err());
        let mut invented = omp.clone();
        invented["queueAvailable"] = json!(true);
        assert!(provider_controls_admission(Provider::Omp, &invented).is_err());
        invented = omp;
        invented["offeredDeliveries"] = json!(["Native", "Queue"]);
        assert!(provider_controls_admission(Provider::Omp, &invented).is_err());
    }
    #[test]
    fn original_product_uid_default_process_and_every_capability_mask_remain_exact() {
        let (epoch, _, value) = observed_shell();
        epoch.verify(&value["process"]).unwrap();
        for (field, changed) in [
            ("uid", json!(10002)),
            ("pid", json!(124)),
            ("start", json!("457")),
            ("name", json!(TEST)),
        ] {
            let mut process = value["process"].clone();
            process[field] = changed;
            assert!(epoch.verify(&process).is_err(), "{field}");
        }
        for mask in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
            let mut process = value["process"].clone();
            process["caps"][mask] = json!("0000000800000000");
            assert!(epoch.verify(&process).is_err(), "{mask}");
        }
    }
    #[test]
    fn api30_only_notification_fallback_requires_positive_selection_and_no_acknowledged_or_unknown_entry()
     {
        let value = json!({"outcome":"blocked","guard":"physical_notification_row_ambiguous","stage":"selection","acknowledged":false});
        assert!(notification_human_fallback(30, &value).unwrap());
        assert!(!notification_human_fallback(34, &value).unwrap());
        for (field, changed) in [
            ("stage", json!("click")),
            ("stage", json!("postcondition")),
            ("acknowledged", json!(true)),
            ("guard", json!("physical_notification_row_missing")),
            ("stage", Value::Null),
        ] {
            let mut changed_value = value.clone();
            changed_value[field] = changed;
            assert!(
                !notification_human_fallback(30, &changed_value).unwrap_or(false),
                "{field}"
            );
        }
    }
    #[test]
    fn correlated_ui_outcomes_are_published_before_refusal_without_raw_observations() {
        let root = std::env::temp_dir().join(format!("goatr-ui-outcome-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let action = "OpenSession";
        let hash = "a".repeat(64);
        let run = Uuid::random();
        let process = json!({"phase":"process","runId":run.0,"action":action,"runnerPid":71,"runnerStart":72});
        let blocked = json!({"runId":run.0,"action":action,"runnerPid":71,"runnerStart":72,"outcome":"blocked","guard":"ui_tag_missing","stage":"selection","acknowledged":false,"expectedScreenObserved":false,
            "observation":"UNKNOWN_PRIVATE_OBSERVATION_SENTINEL"});
        assert!(
            publish_and_admit_ui_outcome(
                &root,
                UiExpectation::Completed,
                UiOutcome {
                    run: &run,
                    action,
                    argument_hash: &hash,
                    original: &process,
                    process: &process,
                    outcome: &blocked,
                    api: 30,
                    command_succeeded: true
                }
            )
            .is_err()
        );
        let stored = fs::read(root.join(format!("physical-ui-{}.outcome.json", run.0))).unwrap();
        assert!(
            !String::from_utf8(stored)
                .unwrap()
                .contains("UNKNOWN_PRIVATE_OBSERVATION_SENTINEL")
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let mismatched_run = Uuid::random();
        let mismatched_process = json!({"phase":"process","runId":mismatched_run.0,"action":action,"runnerPid":71,"runnerStart":72});
        let mismatched =
            json!({"runId":Uuid::random().0,"action":action,"runnerPid":71,"runnerStart":72});
        assert!(
            publish_and_admit_ui_outcome(
                &root,
                UiExpectation::Completed,
                UiOutcome {
                    run: &mismatched_run,
                    action,
                    argument_hash: &hash,
                    original: &mismatched_process,
                    process: &mismatched_process,
                    outcome: &mismatched,
                    api: 30,
                    command_succeeded: true
                }
            )
            .is_err()
        );
        assert!(
            !root
                .join(format!("physical-ui-{}.outcome.json", mismatched_run.0))
                .exists()
        );
        let nonzero_run = Uuid::random();
        let nonzero_process = json!({"phase":"process","runId":nonzero_run.0,"action":action,"runnerPid":73,"runnerStart":74});
        let nonzero = json!({"runId":nonzero_run.0,"action":action,"runnerPid":73,"runnerStart":74,"outcome":"completed","guard":"none","stage":"outcome","acknowledged":true,"expectedScreenObserved":true});
        assert!(
            publish_and_admit_ui_outcome(
                &root,
                UiExpectation::Completed,
                UiOutcome {
                    run: &nonzero_run,
                    action,
                    argument_hash: &hash,
                    original: &nonzero_process,
                    process: &nonzero_process,
                    outcome: &nonzero,
                    api: 30,
                    command_succeeded: false
                }
            )
            .is_err()
        );
        assert_eq!(
            serde_json::from_slice::<Value>(
                &fs::read(root.join(format!("physical-ui-{}.outcome.json", nonzero_run.0)))
                    .unwrap()
            )
            .unwrap()["commandExit"],
            "nonzero_or_signal"
        );
        let completed_run = Uuid::random();
        let completed_process = json!({"phase":"process","runId":completed_run.0,"action":action,"runnerPid":75,"runnerStart":76});
        let completed = json!({"runId":completed_run.0,"action":action,"runnerPid":75,"runnerStart":76,"outcome":"completed","guard":"none","stage":"outcome","acknowledged":true,"expectedScreenObserved":true});
        assert!(
            !publish_and_admit_ui_outcome(
                &root,
                UiExpectation::Completed,
                UiOutcome {
                    run: &completed_run,
                    action,
                    argument_hash: &hash,
                    original: &completed_process,
                    process: &completed_process,
                    outcome: &completed,
                    api: 30,
                    command_succeeded: true
                }
            )
            .unwrap()
            .1
        );
        let notification_run = Uuid::random();
        let notification_process = json!({"phase":"process","runId":notification_run.0,"action":action,"runnerPid":77,"runnerStart":78});
        let notification = json!({"runId":notification_run.0,"action":action,"runnerPid":77,"runnerStart":78,"outcome":"blocked","guard":"physical_notification_row_ambiguous","stage":"selection","acknowledged":false,"expectedScreenObserved":false});
        assert!(
            publish_and_admit_ui_outcome(
                &root,
                UiExpectation::NotificationTap { api: 30 },
                UiOutcome {
                    run: &notification_run,
                    action,
                    argument_hash: &hash,
                    original: &notification_process,
                    process: &notification_process,
                    outcome: &notification,
                    api: 30,
                    command_succeeded: true
                }
            )
            .unwrap()
            .1
        );
        let malformed_run = Uuid::random();
        let malformed_process = json!({"phase":"process","runId":malformed_run.0,"action":action,"runnerPid":79,"runnerStart":80});
        let malformed = json!({"runId":malformed_run.0,"action":action,"runnerPid":79,"runnerStart":80,"outcome":"blocked","guard":"physical_notification_row_ambiguous","stage":null,"acknowledged":false,"expectedScreenObserved":false});
        assert!(
            publish_and_admit_ui_outcome(
                &root,
                UiExpectation::NotificationTap { api: 30 },
                UiOutcome {
                    run: &malformed_run,
                    action,
                    argument_hash: &hash,
                    original: &malformed_process,
                    process: &malformed_process,
                    outcome: &malformed,
                    api: 30,
                    command_succeeded: true
                }
            )
            .is_err()
        );
        assert!(
            serde_json::from_slice::<Value>(
                &fs::read(root.join(format!("physical-ui-{}.outcome.json", malformed_run.0)))
                    .unwrap()
            )
            .unwrap()["preInputRefusal"]
                .is_null()
        );
        for id in [
            run,
            nonzero_run,
            completed_run,
            notification_run,
            malformed_run,
        ] {
            fs::remove_file(root.join(format!("physical-ui-{}.outcome.json", id.0))).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn human_screen_off_attestation_cannot_replace_measured_background_noninteractive_facts() {
        let (_, _, mut value) = observed_shell();
        assert!(!background_noninteractive(&value));
        value["foreground"] = json!(false);
        assert!(background_noninteractive(&value));
        for (field, changed) in [
            ("interactive", json!(true)),
            ("interactive", Value::Null),
            ("measuredAt", json!(0)),
        ] {
            let mut awake = value.clone();
            awake["ui"][field] = changed;
            assert!(!background_noninteractive(&awake));
        }
    }
    #[test]
    fn fresh_absence_and_constructor_directory_are_distinct_closed_consumer_states() {
        let (empty, constructor, _) = fresh_transition_facts();
        empty.fresh_empty().unwrap();
        empty.constructor_ready(&constructor, 10001).unwrap();
        assert!(constructor.fresh_empty().is_err());
        assert!(!empty.database_exists);
        assert!(empty.key_files["directory"].is_null());
        for family in [
            "database",
            "host",
            "orphan",
            "preference",
            "alias",
            "request",
            "request-presence",
            "label",
            "connector",
            "legacy",
            "files",
        ] {
            let mut changed = empty.clone();
            match family {
                "database" => {
                    changed.database_exists = true;
                    changed.database = retained(empty_connector(false)).database;
                }
                "host" => {
                    changed.hosts =
                        with_private(&retained(empty_connector(false)), &"a".repeat(64)).hosts
                }
                "orphan" => {
                    changed.orphans =
                        json!([{"host":"b".repeat(64),"rowsSha256":"c".repeat(64),"rowCount":1}])
                }
                "preference" => changed.preferences.push(PreferenceFact {
                    file: "unexpected".to_owned(),
                    key_sha256: "1".repeat(64),
                    kind: "boolean".to_owned(),
                    value_sha256: digest_text("false"),
                }),
                "alias" => changed.aliases.push("unexpected".to_owned()),
                "request" => changed.service_requested = true,
                "request-presence" => changed.service_requested_present = true,
                "label" => changed.run_label_collision = true,
                "connector" => changed.connector = empty_connector(true),
                "legacy" => changed.connector["legacyFilePresent"] = json!(true),
                "files" => add_key_pair(&mut changed, "goatr.installation.endpoint"),
                _ => unreachable!(),
            }
            assert!(changed.fresh_empty().is_err(), "{family}");
            assert!(
                empty.constructor_ready(&changed, 10001).is_err(),
                "{family}"
            );
        }
        let mut foreign = constructor;
        foreign.key_files["directory"]["uid"] = json!(10002);
        assert!(empty.constructor_ready(&foreign, 10001).is_err());
    }
    #[test]
    fn first_open_consumer_pins_only_exact_empty_db_locale_and_endpoint_before_private_creation() {
        let (empty, constructor, opened) = fresh_transition_facts();
        let admission = empty.first_open_admission(&constructor, &opened, 10001);
        empty
            .admitted_first_open(&constructor, &opened, 10001, &admission)
            .unwrap();
        assert!(
            admission
                .transition
                .iter()
                .chain(&admission.quiescence)
                .chain(&admission.key_material)
                .all(|(_, passed)| *passed)
        );
        for name in DB8_QUIESCENCE {
            let mut changed = opened.clone();
            changed.database["quiescence"][name] = json!(1);
            let admission = empty.first_open_admission(&constructor, &changed, 10001);
            let error = empty
                .admitted_first_open(&constructor, &changed, 10001, &admission)
                .unwrap_err();
            let diagnostic = admission_diagnostic(&error).unwrap();
            assert_eq!(
                (diagnostic.site, diagnostic.guard),
                ("first_open_quiescence", name)
            );
            assert_eq!(blocker(&error), name);
        }
        for (family, site, guard) in [
            ("missing-db", "first_open_transition", "database_exists"),
            ("host", "first_open_transition", "hosts_empty"),
            ("orphan", "first_open_transition", "orphans_empty"),
            ("preference", "first_open_transition", "preferences_empty"),
            ("request", "first_open_transition", "service_key_absent"),
            (
                "service-true",
                "first_open_transition",
                "service_not_requested",
            ),
            ("label", "first_open_transition", "run_label_absent"),
            ("connector", "first_open_transition", "connector_unchanged"),
            ("directory", "first_open_transition", "directory_unchanged"),
            ("alias", "first_open_transition", "one_alias"),
            ("alias-name", "first_open_transition", "endpoint_alias"),
            ("alias-hash", "first_open_transition", "alias_fingerprint"),
            ("file", "first_open_key_material", "two_files"),
            ("marker", "first_open_key_material", "marker_gai1"),
            ("uid", "first_open_key_material", "owned_uid"),
            ("schema", "first_open_validation", "database"),
        ] {
            let mut changed = opened.clone();
            match family {
                "missing-db" => changed.database_exists = false,
                "host" => changed.hosts = with_private(&opened, &"a".repeat(64)).hosts,
                "orphan" => {
                    changed.orphans =
                        json!([{"host":"b".repeat(64),"rowsSha256":"c".repeat(64),"rowCount":1}])
                }
                "preference" => changed.preferences.push(PreferenceFact {
                    file: "unexpected".to_owned(),
                    key_sha256: "1".repeat(64),
                    kind: "boolean".to_owned(),
                    value_sha256: digest_text("false"),
                }),
                "request" => changed.service_requested_present = true,
                "service-true" => changed.service_requested = true,
                "label" => changed.run_label_collision = true,
                "connector" => changed.connector = empty_connector(true),
                "directory" => changed.key_files["directory"]["inode"] = json!(99),
                "alias" => changed.aliases.push("foreign".to_owned()),
                "alias-name" => changed.aliases[0] = "foreign".to_owned(),
                "alias-hash" => changed.alias_sha256 = "0".repeat(64),
                "file" => changed.key_files["files"].as_array_mut().unwrap().clear(),
                "marker" => changed.key_files["files"][1]["sha256"] = json!("0".repeat(64)),
                "uid" => changed.key_files["files"][0]["uid"] = json!(10002),
                "schema" => changed.database["schemaSha256"] = Value::Null,
                _ => unreachable!(),
            }
            let admission = empty.first_open_admission(&constructor, &changed, 10001);
            let error = empty
                .admitted_first_open(&constructor, &changed, 10001, &admission)
                .unwrap_err();
            let diagnostic = admission_diagnostic(&error).unwrap();
            assert_eq!(
                (diagnostic.site, diagnostic.guard),
                (site, guard),
                "{family}"
            );
            assert_eq!(blocker(&error), guard);
        }
        let private = "a".repeat(64);
        let created = with_private(&opened, &private);
        opened
            .preserved_with_private_host(&created, &private, false, false)
            .unwrap();
        assert!(
            empty
                .preserved_with_private_host(&created, &private, false, false)
                .is_err(),
            "empty proof cannot replace first-open authority"
        );
        for family in [
            "global",
            "host",
            "orphan",
            "endpoint",
            "preference",
            "connector",
        ] {
            let mut changed = created.clone();
            match family {
                "global" => changed.database["globalSha256"] = json!("0".repeat(64)),
                "host" => changed = with_private(&changed, &"b".repeat(64)),
                "orphan" => {
                    changed.orphans =
                        json!([{"host":"b".repeat(64),"rowsSha256":"c".repeat(64),"rowCount":1}])
                }
                "endpoint" => changed.key_files["files"][0]["inode"] = json!(99),
                "preference" => changed.preferences.push(PreferenceFact {
                    file: "unexpected".to_owned(),
                    key_sha256: "1".repeat(64),
                    kind: "boolean".to_owned(),
                    value_sha256: digest_text("false"),
                }),
                "connector" => changed.connector = official_connector(),
                _ => unreachable!(),
            }
            assert!(
                opened
                    .preserved_with_private_host(&changed, &private, false, false)
                    .is_err(),
                "{family}"
            );
        }
        assert!(!empty.database_exists);
        assert!(constructor.aliases.is_empty());
    }
    #[test]
    fn retained_and_first_open_diagnostics_reject_at_the_real_boundary_without_exporting_foreign_material()
     {
        let (empty, constructor, opened) = fresh_transition_facts();
        let (epoch, _, _) = observed_shell();
        let valid = json!({"kind":"retained","facts":opened});
        validate_retained_report(&valid, &epoch, &"a".repeat(64)).unwrap();
        let foreign = "UNKNOWN_PREFERENCE_FILE_SECRET";
        let foreign_alias = "UNKNOWN_ALIAS_SECRET";
        let foreign_file = "UNKNOWN_KEY_FILE_SECRET.gcm";
        let foreign_hash = "9".repeat(64);
        let mut raw = valid["facts"].clone();
        raw["preferences"] = json!([{"file":foreign,"keySha256":foreign_hash,"type":"string","valueSha256":foreign_hash}]);
        raw["aliases"]
            .as_array_mut()
            .unwrap()
            .push(json!(foreign_alias));
        for guard in ["schema", "preferences", "key_files"] {
            let mut changed = raw.clone();
            match guard {
                "schema" => changed["UNKNOWN_FIELD_SECRET"] = json!(foreign),
                "preferences" => changed["preferences"][0]["valueSha256"] = json!(foreign),
                "key_files" => changed["keyFiles"]["files"][0]["name"] = json!(foreign_file),
                _ => unreachable!(),
            }
            let error = validate_retained_report(
                &json!({"kind":"retained","facts":changed}),
                &epoch,
                &"a".repeat(64),
            )
            .unwrap_err();
            let diagnostic = admission_diagnostic(&error).unwrap();
            assert_eq!(
                (diagnostic.site, diagnostic.guard),
                ("retained_decode", guard)
            );
            assert_eq!(blocker(&error), guard);
            let projection = json!(diagnostic);
            assert_eq!(
                projection["summary"],
                json!({"schemaIsOne":true,"databaseExists":true,"hosts":0,"preferences":1,"aliases":2,
                "orphans":0,"keyFiles":2,"serviceRequested":false,"serviceRequestedPresent":false})
            );
            let exported = projection.to_string();
            let message = error.to_string();
            for secret in [
                foreign,
                foreign_alias,
                foreign_file,
                foreign_hash.as_str(),
                "UNKNOWN_FIELD_SECRET",
            ] {
                assert!(!exported.contains(secret));
                assert!(!message.contains(secret));
            }
        }
        let unadmitted = RetainedFacts::decode(&raw).unwrap();
        let admission = empty.first_open_admission(&constructor, &unadmitted, 10001);
        let projection = json!(admission);
        assert_eq!(
            projection["transition"][3],
            json!(["preferences_empty", false])
        );
        let error = empty
            .admitted_first_open(&constructor, &unadmitted, 10001, &admission)
            .unwrap_err();
        let diagnostic = admission_diagnostic(&error).unwrap();
        assert_eq!(
            (diagnostic.site, diagnostic.guard),
            ("first_open_transition", "preferences_empty")
        );
        let exported = projection.to_string();
        let failure_export = json!(diagnostic).to_string();
        for secret in [foreign, foreign_alias, foreign_hash.as_str()] {
            assert!(!exported.contains(secret));
            assert!(!failure_export.contains(secret));
        }
    }
    #[test]
    fn first_open_rejection_survives_real_summary_publication_failure_without_admitting_the_reference()
     {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("goatr-first-open-publication-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        for (rejected, publication_failed) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let (empty, constructor, mut opened) = fresh_transition_facts();
            if rejected {
                opened.database["quiescence"]["locale"] = json!(1);
            }
            let admission = empty.first_open_admission(&constructor, &opened, 10001);
            let summary = json!({"beforeValidation":true,"admission":admission});
            let path = root.join(format!(
                "UNKNOWN_PUBLICATION_PATH_SECRET-{rejected}-{publication_failed}.json"
            ));
            if publication_failed {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .unwrap();
                file.write_all(b"UNKNOWN_PREEXISTING_CONTENT_SECRET")
                    .unwrap();
            }
            let publication = write_json(&path, &summary);
            assert_eq!(publication.is_err(), publication_failed);
            let admitted = empty.admitted_first_open(&constructor, &opened, 10001, &admission);
            let result = first_open_admission_result(admitted, publication, summary);
            if rejected || publication_failed {
                let error = result.unwrap_err();
                let diagnostic = admission_diagnostic(&error).unwrap();
                let expected = if rejected {
                    ("first_open_quiescence", "locale")
                } else {
                    ("first_open_publication", "admission_summary")
                };
                assert_eq!((diagnostic.site, diagnostic.guard), expected);
                if publication_failed {
                    let summary = diagnostic.summary.as_ref().unwrap();
                    assert_eq!(
                        summary["publicationFailure"],
                        json!({"site":"first_open_publication","guard":"admission_summary"})
                    );
                    assert_eq!(
                        summary["admission"]["quiescence"][17],
                        json!(["locale", !rejected])
                    );
                } else {
                    assert!(diagnostic.summary.is_none());
                }
                let exported = json!(diagnostic).to_string();
                let message = error.to_string();
                for secret in [
                    "UNKNOWN_PUBLICATION_PATH_SECRET",
                    "UNKNOWN_PREEXISTING_CONTENT_SECRET",
                ] {
                    assert!(!exported.contains(secret));
                    assert!(!message.contains(secret));
                }
            } else {
                result.unwrap();
            }
            if publication_failed {
                assert_eq!(
                    fs::read(&path).unwrap(),
                    b"UNKNOWN_PREEXISTING_CONTENT_SECRET"
                );
            } else {
                let receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(receipt["beforeValidation"], true);
                assert_eq!(receipt["admission"], json!(admission));
            }
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn retained_review_consumer_keeps_final_decode_diagnostic_after_success_and_never_replaces_the_primary()
     {
        let mut ordinary = None;
        let ordinary_error = failure("UNKNOWN_ORDINARY_PRIVATE_DETAIL");
        retain_review_failure(ordinary_error, &mut ordinary);
        let ordinary = ordinary.unwrap();
        assert_eq!(ordinary["io_kind"], "other");
        assert!(
            ordinary["source_file"]
                .as_str()
                .is_some_and(|file| file == "smoke/physical.rs")
        );
        assert!(ordinary["source_line"].as_u64().is_some());
        assert!(ordinary["admissionFailure"].is_null());
        assert!(
            !ordinary
                .to_string()
                .contains("UNKNOWN_ORDINARY_PRIVATE_DETAIL")
        );
        let mut late_primary = None;
        let late_error = failure("UNKNOWN_LATE_FIRST_PRIVATE_DETAIL");
        retain_review_failure(late_error, &mut late_primary);
        retain_review_failure(
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "UNKNOWN_LATE_REPLACEMENT_PRIVATE_DETAIL",
            ),
            &mut late_primary,
        );
        let late_primary = late_primary.unwrap();
        assert_eq!(late_primary["io_kind"], "other");
        assert!(
            late_primary["source_file"]
                .as_str()
                .is_some_and(|file| file == "smoke/physical.rs")
        );
        assert!(
            !late_primary
                .to_string()
                .contains("UNKNOWN_LATE_REPLACEMENT_PRIVATE_DETAIL")
        );
        let (empty, constructor, mut opened) = fresh_transition_facts();
        let (epoch, _, _) = observed_shell();
        let mut raw = json!(opened);
        raw["keyFiles"]["files"][0]["name"] = json!("UNKNOWN_FINAL_FILE_SECRET.gcm");
        let final_error = validate_retained_report(
            &json!({"kind":"retained","facts":raw}),
            &epoch,
            &"a".repeat(64),
        )
        .unwrap_err();
        let mut diagnostic = None;
        let code = retain_review_failure(final_error, &mut diagnostic);
        let owner_loss = json!({"effectFailureCode":code,"primaryFailure":diagnostic,"admissionFailure":diagnostic.as_ref().and_then(review_admission_failure)});
        assert_eq!(owner_loss["effectFailureCode"], "key_files");
        assert_eq!(owner_loss["admissionFailure"]["site"], "retained_decode");
        assert_eq!(owner_loss["admissionFailure"]["guard"], "key_files");
        assert_eq!(owner_loss["admissionFailure"]["summary"]["keyFiles"], 2);
        assert!(!owner_loss.to_string().contains("UNKNOWN_FINAL_FILE_SECRET"));
        opened.database["quiescence"]["locale"] = json!(1);
        let admission = empty.first_open_admission(&constructor, &opened, 10001);
        let primary = empty
            .admitted_first_open(&constructor, &opened, 10001, &admission)
            .unwrap_err();
        let mut earliest = None;
        assert_eq!(retain_review_failure(primary, &mut earliest), "locale");
        let later = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "UNKNOWN_LATER_PRIVATE_DETAIL",
        );
        retain_review_failure(later, &mut earliest);
        let retained = earliest.unwrap();
        assert_eq!(retained["io_kind"], "other");
        assert_eq!(
            retained["admissionFailure"]["site"],
            "first_open_quiescence"
        );
        assert_eq!(retained["admissionFailure"]["guard"], "locale");
        assert!(retained["admissionFailure"]["summary"].is_null());
        assert!(retained["source_file"].is_null());
        assert!(retained["source_line"].is_null());
        assert!(
            !retained
                .to_string()
                .contains("UNKNOWN_LATER_PRIVATE_DETAIL")
        );
    }
    #[test]
    fn pre_private_database_existence_schema_global_and_orphan_changes_are_not_startup_exceptions()
    {
        let before = retained(empty_connector(false));
        let private = "a".repeat(64);
        for (field, changed) in [
            ("schemaSha256", json!("a".repeat(64))),
            ("globalSha256", json!("b".repeat(64))),
        ] {
            let mut after = before.clone();
            after.database[field] = changed;
            assert!(before.before_private(&after, &private).is_err());
        }
        let mut after = before.clone();
        after.database_exists = false;
        assert!(before.before_private(&after, &private).is_err());
        let mut absent = before.clone();
        absent.database_exists = false;
        absent.database =
            json!({"schemaSha256":null,"globalSha256":null,"measuredAt":1,"quiescence":{}});
        assert!(absent.before_private(&before, &private).is_err());
        after = before.clone();
        after.orphans = json!([{"host":"c".repeat(64),"rowsSha256":"d".repeat(64),"rowCount":1}]);
        assert!(before.before_private(&after, &private).is_err());
        after = before.clone();
        after.connector = empty_connector(true);
        assert!(before.before_private(&after, &private).is_err());
    }
    #[test]
    fn retained_ciphertext_marker_and_inode_changes_or_unrelated_files_cannot_hide_behind_private_additions()
     {
        let private = "a".repeat(64);
        let mut before = with_private(&retained(empty_connector(false)), &"b".repeat(64));
        before.run_label_collision = false;
        add_key_pair(&mut before, "goatr.installation.endpoint");
        let valid = with_private(&before, &private);
        before
            .preserved_with_private_host(&valid, &private, false, false)
            .unwrap();
        for (field, changed) in [
            ("sha256", json!("0".repeat(64))),
            ("inode", json!(9999)),
            ("mode", json!(0o644)),
            ("size", json!(62)),
        ] {
            let mut after = valid.clone();
            after.key_files["files"][0][field] = changed;
            assert!(
                before
                    .preserved_with_private_host(&after, &private, false, false)
                    .is_err(),
                "{field}"
            );
        }
        let mut removed = valid.clone();
        removed.key_files["files"].as_array_mut().unwrap().remove(0);
        assert!(
            before
                .preserved_with_private_host(&removed, &private, false, false)
                .is_err()
        );
        let mut unrelated = valid.clone();
        add_key_pair(
            &mut unrelated,
            &format!("goatr.host.{}.push", digest_text("unrelated")),
        );
        assert!(key_files_preserved(&before, &unrelated, &private).is_err());
        let mut absent_alias = valid;
        absent_alias
            .aliases
            .retain(|alias| alias != "goatr.installation.endpoint");
        assert!(RetainedFacts::decode(&serde_json::to_value(absent_alias).unwrap()).is_err());
    }
    #[test]
    fn explicit_false_service_key_presence_type_and_value_cannot_use_absent_default_restoration() {
        let private = "a".repeat(64);
        let mut before = retained(empty_connector(false));
        before.service_requested_present = true;
        before.preferences.push(PreferenceFact {
            file: "delivery_preferences".to_owned(),
            key_sha256: digest_text("service_requested"),
            kind: "boolean".to_owned(),
            value_sha256: digest_text("false"),
        });
        let valid = with_private(&before, &private);
        before
            .preserved_with_private_host(&valid, &private, false, true)
            .unwrap();
        let mut deleted = valid.clone();
        deleted.preferences.clear();
        deleted.service_requested_present = false;
        assert!(
            before
                .preserved_with_private_host(&deleted, &private, false, true)
                .is_err()
        );
        for (kind, value) in [("string", "false"), ("boolean", "true")] {
            let mut changed = valid.clone();
            changed.preferences[0].kind = kind.to_owned();
            changed.preferences[0].value_sha256 = digest_text(value);
            assert!(
                before
                    .preserved_with_private_host(&changed, &private, false, true)
                    .is_err()
            );
        }
    }
    #[test]
    fn semantic_empty_legacy_presence_is_disclosed_but_no_keys_or_connector_locale_mutation_is_allowed()
     {
        let private = "a".repeat(64);
        for present in [false, true] {
            let mut before = retained(official_connector());
            before.connector["legacyFilePresent"] = json!(present);
            let mut after = before.clone();
            after.connector["legacyFilePresent"] = json!(!present);
            before.before_private(&after, &private).unwrap();
            after.connector["legacyKeyCount"] = json!(1);
            assert!(before.before_private(&after, &private).is_err());
        }
        let mut wrong_locale = official_connector();
        wrong_locale["localeChanges"] = json!(1);
        assert!(validate_connector(&wrong_locale).is_err());
        wrong_locale = official_connector();
        wrong_locale["platformMetadataSha256"] = Value::Null;
        assert!(validate_connector(&wrong_locale).is_err());
    }
    fn portable_seed_text(runtime: &Path, marker: &str) -> String {
        let runtime = serde_json::to_string(runtime.to_str().unwrap()).unwrap();
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let executable = serde_json::to_string(executable.to_str().unwrap()).unwrap();
        format!(
            "# {marker}\nversion=1\nruntime_dir={runtime}\nruntime_dir_agent={runtime}\nallow_scoped_shell_probe=true\nallow_terminal_control=true\nallow_omp_controls=true\nallow_attachments=false\nherdr_sockets=[]\nprojects=[]\ncodex_owners=[]\nskill_document_roots=[]\npush_allowed_origins=[\"https://ntfy.sh\"]\n[executables]\nherdr={executable}\n"
        )
    }
    fn write_owned_seed(path: &Path, reference: &[u8]) {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap()
            .write_all(reference)
            .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[test]
    fn pinned_seed_reader_accepts_two_independent_reference_pins_and_retains_the_parsed_file() {
        let root = std::env::temp_dir().join(format!("goatr-seed-reader-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let mut pins = Vec::new();
        for marker in ["first", "second"] {
            let parent = root.join(marker);
            private_directory(&parent).unwrap();
            let runtime = parent.join("r");
            private_directory(&runtime).unwrap();
            let reference = portable_seed_text(&runtime, marker);
            let expected = digest_text(&reference);
            pins.push(expected.clone());
            let path = parent.join("seed.toml");
            write_owned_seed(&path, reference.as_bytes());
            let (config, mut file, original) = read_pinned_config(&path, &expected).unwrap();
            assert_eq!(config.runtime_dir, runtime);
            assert_eq!(config.runtime_dir_agent, runtime);
            assert!(config.allow_terminal_control);
            neutral_seed_policy(&config, &parent).unwrap();
            assert_eq!(
                complete_stamp(&file.metadata().unwrap()),
                complete_stamp(&original)
            );
            let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
            assert!(flags >= 0 && flags & libc::FD_CLOEXEC != 0);
            std::io::Seek::rewind(&mut file).unwrap();
            let mut observed = String::new();
            file.read_to_string(&mut observed).unwrap();
            assert_eq!(observed, reference);
            let held = (path, file, original);
            recheck_inputs(std::slice::from_ref(&held)).unwrap();
        }
        assert_ne!(pins[0], pins[1]);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pinned_seed_reader_rejects_wrong_claim_permissions_links_aliases_bounds_and_bad_decode() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("goatr-seed-denials-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let parent = root.join("seed");
        private_directory(&parent).unwrap();
        let runtime = parent.join("r");
        private_directory(&runtime).unwrap();
        let reference = portable_seed_text(&runtime, "owned reference");
        let expected = digest_text(&reference);
        let path = parent.join("seed.toml");
        write_owned_seed(&path, reference.as_bytes());
        assert!(read_pinned_config(&path, &"0".repeat(64)).is_err());
        for pin in [
            String::new(),
            "a".repeat(63),
            "a".repeat(65),
            "g".repeat(64),
            "A".repeat(64),
        ] {
            assert!(read_pinned_config(&path, &pin).is_err());
        }
        assert!(read_pinned_config(&parent.join("missing.toml"), &expected).is_err());
        for mode in [0o400, 0o644] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(read_pinned_config(&path, &expected).is_err());
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_pinned_config(&path, &expected).is_err());
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let hard = parent.join("hard.toml");
        fs::hard_link(&path, &hard).unwrap();
        assert!(read_pinned_config(&path, &expected).is_err());
        fs::remove_file(hard).unwrap();
        let alias = parent.join("alias.toml");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(read_pinned_config(&alias, &expected).is_err());
        fs::remove_file(alias).unwrap();
        let parent_alias = root.join("parent-alias");
        std::os::unix::fs::symlink(&parent, &parent_alias).unwrap();
        assert!(read_pinned_config(&parent_alias.join("seed.toml"), &expected).is_err());
        fs::remove_file(parent_alias).unwrap();
        let oversized = vec![b' '; goatr::config::CONFIG_BYTES + 1];
        let oversized_pin = hex(&Sha256::digest(&oversized));
        let large = parent.join("large.toml");
        write_owned_seed(&large, &oversized);
        assert!(read_pinned_config(&large, &oversized_pin).is_err());
        for (name, reference) in [
            ("invalid.toml", b"invalid = [".as_slice()),
            ("nonutf8.toml", b"\xff\xfe".as_slice()),
        ] {
            let pin = hex(&Sha256::digest(reference));
            let bad = parent.join(name);
            write_owned_seed(&bad, reference);
            assert!(read_pinned_config(&bad, &pin).is_err());
        }
        assert_eq!(fs::read(&path).unwrap(), reference.as_bytes());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn neutral_seed_policy_rejects_dynamic_resources_extra_authority_and_nonpublic_origins() {
        let root = std::env::temp_dir().join(format!("goatr-seed-policy-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let runtime = root.join("r");
        private_directory(&runtime).unwrap();
        let config = Config::parse(&portable_seed_text(&runtime, "neutral policy")).unwrap();
        neutral_seed_policy(&config, &root).unwrap();
        for family in [
            "name",
            "socket",
            "project",
            "owner",
            "default-owner",
            "documents",
            "attachment",
            "probe",
            "terminal",
            "omp",
            "origin",
            "extra-origin",
            "no-origin",
        ] {
            let mut changed = config.clone();
            match family {
                "name" => changed.pairing_name = Some(Label::new("Retained name").unwrap()),
                "socket" => changed.herdr_sockets.push(runtime.join("source.sock")),
                "project" => changed.projects.push(ConfiguredProject {
                    id: Uuid::random(),
                    label: Label::new("Retained project").unwrap(),
                    path: runtime.clone(),
                    allow_read_tools: false,
                }),
                "owner" => changed.codex_owners.push(CodexOwner {
                    id: Uuid::random(),
                    socket_host: runtime.join("c.sock"),
                    socket_agent: runtime.join("c.sock"),
                    bindings_dir: runtime.clone(),
                    unit: "owned-test.service".to_owned(),
                }),
                "default-owner" => changed.default_codex_owner_id = Some(Uuid::random()),
                "documents" => changed.skill_document_roots.push(runtime.clone()),
                "attachment" => changed.allow_attachments = true,
                "probe" => changed.allow_scoped_shell_probe = false,
                "terminal" => changed.allow_terminal_control = false,
                "omp" => changed.allow_omp_controls = false,
                "origin" => {
                    changed.push_allowed_origins = vec!["https://example.invalid".to_owned()]
                }
                "extra-origin" => changed
                    .push_allowed_origins
                    .push("https://example.invalid".to_owned()),
                "no-origin" => changed.push_allowed_origins.clear(),
                _ => unreachable!(),
            }
            assert!(neutral_seed_policy(&changed, &root).is_err(), "{family}");
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn neutral_seed_runtime_must_be_same_existing_canonical_private_descendant_not_prefix_or_alias()
    {
        let root = std::env::temp_dir().join(format!("goatr-seed-runtime-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let parent = root.join("seed");
        private_directory(&parent).unwrap();
        let runtime = parent.join("r");
        private_directory(&runtime).unwrap();
        let config = Config::parse(&portable_seed_text(&runtime, "runtime policy")).unwrap();
        neutral_seed_policy(&config, &parent).unwrap();
        let outside = root.join("outside");
        private_directory(&outside).unwrap();
        let prefix = root.join("seed-suffix");
        private_directory(&prefix).unwrap();
        let alias = parent.join("alias");
        std::os::unix::fs::symlink(&runtime, &alias).unwrap();
        let missing = parent.join("missing");
        for candidate in [&outside, &prefix, &parent, &alias, &missing] {
            let mut changed = config.clone();
            changed.runtime_dir = candidate.clone();
            changed.runtime_dir_agent = candidate.clone();
            assert!(
                neutral_seed_policy(&changed, &parent).is_err(),
                "{}",
                candidate.display()
            );
        }
        assert!(!missing.exists());
        let mut different = config.clone();
        different.runtime_dir_agent = outside.clone();
        assert!(neutral_seed_policy(&different, &parent).is_err());
        assert!(neutral_seed_policy(&config, &outside).is_err());
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(neutral_seed_policy(&config, &parent).is_err());
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(neutral_seed_policy(&config, &parent).is_err());
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn held_frozen_input_rejects_replacement_across_await_without_adopting_new_named_bytes() {
        let root = std::env::temp_dir().join(format!("goatr-physical-fd-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let path = root.join("original");
        let replacement = root.join("replacement");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(b"ORIGINAL_PUBLIC_INPUT")
            .unwrap();
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&replacement)
            .unwrap()
            .write_all(b"REPLACEMENT_PUBLIC_INPUT")
            .unwrap();
        let mut pin = frozen_input(&path, &digest_text("ORIGINAL_PUBLIC_INPUT")).unwrap();
        recheck_inputs(std::slice::from_ref(&pin)).unwrap();
        let flags = unsafe { libc::fcntl(pin.1.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0 && flags & libc::FD_CLOEXEC != 0);
        let named = path.clone();
        let (release, ready) = tokio::sync::oneshot::channel::<()>();
        let swap = tokio::spawn(async move {
            ready.await.unwrap();
            fs::rename(replacement, named).unwrap();
        });
        release.send(()).unwrap();
        swap.await.unwrap();
        assert!(recheck_inputs(std::slice::from_ref(&pin)).is_err());
        std::io::Seek::rewind(&mut pin.1).unwrap();
        let mut original = Vec::new();
        pin.1.read_to_end(&mut original).unwrap();
        assert_eq!(original, b"ORIGINAL_PUBLIC_INPUT");
        drop(pin);
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[tokio::test]
    async fn initial_native_desktop_join_rejects_same_executable_other_process_before_fresh_client_or_request()
     {
        use std::os::unix::fs::PermissionsExt;
        const CHILD: &str = "GOATR_PHYSICAL_NATIVE_JOIN_CHILD";
        if std::env::var_os(CHILD).as_deref() == Some(std::ffi::OsStr::new("1")) {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).unwrap();
            assert!(bytes.is_empty());
            return;
        }
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let test = concat!(
            module_path!(),
            "::initial_native_desktop_join_rejects_same_executable_other_process_before_fresh_client_or_request"
        );
        let test = test
            .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
            .unwrap();
        let mut command = std::process::Command::new(&executable);
        command
            .args(["--exact", test, "--test-threads=1"])
            .env(CHILD, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
        let mut child = command.spawn().unwrap();
        let original = herdr::process_evidence(child.id()).unwrap();
        let expected_image = fs::metadata(&executable).unwrap();
        let actual_image = fs::metadata(format!("/proc/{}/exe", original.pid)).unwrap();
        assert_eq!(
            (expected_image.dev(), expected_image.ino()),
            (actual_image.dev(), actual_image.ino())
        );
        let root =
            std::env::temp_dir().join(format!("goatr-initial-native-join-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let socket = root.join("source.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let pending = listener.try_clone().unwrap();
        let listener = tokio::net::UnixListener::from_std(listener).unwrap();
        let native = Client::connect(&socket)
            .await
            .unwrap()
            .qualify_executable(&executable)
            .unwrap();
        native_desktop_join(
            &native,
            &herdr::process_evidence(std::process::id()).unwrap(),
        )
        .unwrap();
        assert_ne!(native.evidence.pid, original.pid);
        let (stream, _) = listener.accept().await.unwrap();
        let observed = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stream.take(65537).read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        let admitted = timeout(Duration::from_secs(5), async {
            native_desktop_join(&native, &original)?;
            let fresh = Client::connect(&socket)
                .await?
                .qualify_executable(&executable)?;
            fresh.snapshot().await.map(|_| ())
        })
        .await;
        drop(native);
        drop(child.stdin.take());
        let child_status = child.wait().unwrap();
        let bytes = timeout(Duration::from_secs(5), observed)
            .await
            .unwrap()
            .unwrap();
        let mut extra = Vec::new();
        let fresh_created = match pending.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream.read_to_end(&mut extra).unwrap();
                true
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("owned native fixture accept failed: {error}"),
        };
        drop(listener);
        drop(pending);
        fs::remove_file(socket).unwrap();
        fs::remove_dir(root).unwrap();
        assert!(child_status.success());
        let error = admitted.unwrap().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("original launched desktop process/namespace")
        );
        assert!(
            !fresh_created,
            "mismatched source must be rejected before a fresh snapshot client is created"
        );
        assert_eq!(
            bytes.len() + extra.len(),
            0,
            "same-executable wrong process must receive zero native request bytes"
        );
    }
    #[tokio::test]
    async fn original_frozen_vector_move_rejects_late_stage_and_reviewed_finish_replacement() {
        for changed in ["product-input", "test-input", "config-input"] {
            let root =
                std::env::temp_dir().join(format!("goatr-late-frozen-inputs-{}", Uuid::random().0));
            private_directory(&root).unwrap();
            let mut inputs = Vec::new();
            for name in [
                "binary-input",
                "product-input",
                "test-input",
                "config-input",
            ] {
                let path = root.join(name);
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .unwrap()
                    .write_all(name.as_bytes())
                    .unwrap();
                inputs.push(frozen_input(&path, &digest_text(name)).unwrap());
            }
            let original_fds = inputs
                .iter()
                .map(|(_, file, _)| file.as_raw_fd())
                .collect::<Vec<_>>();
            let stage_inputs = std::mem::take(&mut inputs);
            assert!(inputs.is_empty());
            assert_eq!(
                stage_inputs
                    .iter()
                    .map(|(_, file, _)| file.as_raw_fd())
                    .collect::<Vec<_>>(),
                original_fds
            );
            recheck_inputs(&stage_inputs).unwrap();
            tokio::task::yield_now().await;
            let replacement = root.join("replacement");
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&replacement)
                .unwrap()
                .write_all(b"PUBLIC_CHANGED_INPUT")
                .unwrap();
            fs::rename(replacement, root.join(changed)).unwrap();
            let late_stage = recheck_inputs(&stage_inputs);
            let reviewed_finish = recheck_inputs(&stage_inputs);
            assert!(
                late_stage.is_err(),
                "{changed} replacement must reject a later effect"
            );
            assert!(
                reviewed_finish.is_err(),
                "{changed} replacement must reject reviewed Finish"
            );
            assert_eq!(
                stage_inputs
                    .iter()
                    .map(|(_, file, _)| file.as_raw_fd())
                    .collect::<Vec<_>>(),
                original_fds,
                "no source descriptor is reacquired"
            );
            drop(stage_inputs);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    async fn preowner_blocked_status_reaches_original_consumer_as_typed_prerequisite() {
        for code in ["physical_preflight", "retained_state_snapshot_unavailable"] {
            let (mut output, status) = tokio::io::duplex(1024);
            let (sender, epoch) = watch::channel(None);
            let task = tokio::spawn(ProductInstrumentation::read_status(
                status, sender, "baseline",
            ));
            let mut instrumentation = ProductInstrumentation { epoch, task };
            output.write_all(format!("INSTRUMENTATION_RESULT: goatrPhysicalOutcome={{\"status\":\"blocked\",\"code\":\"{code}\"}}\n").as_bytes()).await.unwrap();
            drop(output);
            let error = instrumentation.original().await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotConnected);
            assert!(error.to_string().contains(code));
            assert!(instrumentation.epoch.borrow().is_none());
            assert!(instrumentation.task.is_finished());
        }
    }
    #[tokio::test]
    async fn preowner_status_loss_stays_failed_without_publishing_or_restarting_owner() {
        let (sender, epoch) = watch::channel(None);
        let task = tokio::spawn(ProductInstrumentation::read_status(
            tokio::io::empty(),
            sender,
            "baseline",
        ));
        let mut instrumentation = ProductInstrumentation { epoch, task };
        let error = instrumentation.original().await.unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::NotConnected);
        assert!(
            error
                .to_string()
                .contains("ended without a qualified finish")
        );
        assert!(instrumentation.epoch.borrow().is_none());
        assert!(instrumentation.task.is_finished());
    }
    #[tokio::test]
    async fn restored_parent_path_cannot_clear_original_input_failure_or_reauthorize_finish() {
        let root =
            std::env::temp_dir().join(format!("goatr-transient-input-parent-{}", Uuid::random().0));
        private_directory(&root).unwrap();
        let parent = root.join("original");
        private_directory(&parent).unwrap();
        let runtime = parent.join("r");
        private_directory(&runtime).unwrap();
        let path = parent.join("seed.toml");
        let reference = portable_seed_text(&runtime, "held original seed");
        let expected = digest_text(&reference);
        write_owned_seed(&path, reference.as_bytes());
        let (config, file, original) = read_pinned_config(&path, &expected).unwrap();
        neutral_seed_policy(&config, &parent).unwrap();
        let inputs = vec![(path.clone(), file, original)];
        let mut input_pin_failed = false;
        recheck_retained_inputs(&inputs, &mut input_pin_failed).unwrap();
        assert_eq!(
            finish_authorization(input_pin_failed, "FINISH ORIGINAL"),
            Some("FINISH ORIGINAL")
        );
        tokio::task::yield_now().await;
        let displaced = root.join("displaced");
        fs::rename(&parent, &displaced).unwrap();
        let observed = recheck_retained_inputs(&inputs, &mut input_pin_failed);
        assert!(observed.is_err());
        assert!(
            input_pin_failed,
            "the failure must latch before its error returns"
        );
        fs::rename(&displaced, &parent).unwrap();
        recheck_inputs(&inputs).unwrap();
        assert!(
            recheck_retained_inputs(&inputs, &mut input_pin_failed).is_err(),
            "restored named path is not recovery authority"
        );
        assert!(input_pin_failed);
        assert_eq!(
            finish_authorization(input_pin_failed, "FINISH ORIGINAL"),
            None
        );
        drop(inputs);
        fs::remove_file(path).unwrap();
        fs::remove_dir(runtime).unwrap();
        fs::remove_dir(parent).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn package_inventory_consumer_requires_exact_active_and_retained_product_absence() {
        let helpers = format!(
            "package:/data/app/test==/base.apk={TEST} uid:10101\npackage:/data/app/verify==/base.apk={VERIFY} uid:10526\n"
        );
        let packages = coherent_packages(&helpers, &helpers).unwrap();
        fresh_product_admission(&packages).unwrap();
        assert_eq!(packages.len(), 2);
        fresh_product_admission(&coherent_packages("", "").unwrap()).unwrap();
        let product = format!("{helpers}package:/data/app/product/base.apk={APP} uid:10528\n");
        assert!(fresh_product_admission(&coherent_packages(&product, &product).unwrap()).is_err());
        let retained = format!("{helpers}package:{APP} uid:10528\n");
        assert!(coherent_packages(&helpers, &retained).is_err());
        assert!(fresh_product_admission(&package_records(&retained).unwrap()).is_err());
        for malformed in [
            format!("{helpers}{helpers}"),
            format!("package:/data/app/a/base.apk={APP}.test.fake uid:10528\n"),
            format!("package:/sdcard/base.apk={APP} uid:10528\n"),
            format!("package:/data/app/a/base.apk={APP} uid:010528\n"),
            format!("package:/data/app/a/../base.apk={APP} uid:10528\n"),
            format!("package:/data/app/a/./base.apk={APP} uid:10528\n"),
            format!("package:/data/app/a//base.apk={APP} uid:10528\n"),
            format!("package:/data/app/a b/base.apk={APP} uid:10528\n"),
            format!("package:/data/app/a/base.apk={APP} uid:unknown\n"),
            format!("package:{APP}\n"),
            format!(
                "package:/data/app/a/base.apk={TEST} uid:10101\npackage:/data/app/b/base.apk={VERIFY} uid:10101\n"
            ),
            "Error: package query failed\n".to_owned(),
        ] {
            assert!(coherent_packages(&malformed, &malformed).is_err());
        }
        let product_image = Artifact {
            path: PathBuf::from("/owned/product.apk"),
            sha256: "1".repeat(64),
            signer: "2".repeat(64),
            package: APP.to_owned(),
            debug: true,
        };
        let previous = Artifact {
            package: TEST.to_owned(),
            ..product_image.clone()
        };
        let helper = previous.clone();
        test_update_admission(&previous, &helper).unwrap();
        assert!(test_update_admission(&previous, &product_image).is_err());
        let other_key = Artifact {
            signer: "3".repeat(64),
            ..helper
        };
        assert!(test_update_admission(&previous, &other_key).is_err());
    }
    #[test]
    fn package_ownership_consumer_binds_uid_subject_first_install_and_data_path() {
        let record = PackageRecord {
            package: TEST.to_owned(),
            uid: 10101,
            path: Some("/data/app/test/base.apk".to_owned()),
        };
        let dump = format!(
            "Packages:\n  Package [{TEST}] (abcd):\n    userId=10101\n    dataDir=/data/user/0/{TEST}\n    firstInstallTime=2026-10-27 04:39:23\nOther section:\n"
        );
        let actual = package_ownership_facts(&dump, &record).unwrap();
        assert_eq!(actual.uid, 10101);
        assert_eq!(actual.package, TEST);
        for changed in [
            dump.replace("userId=10101", "userId=10102"),
            dump.replace(TEST, VERIFY),
            dump.replace("    userId=10101", "    userId=10101\n    userId=10101"),
            dump.replace("/data/user/0/", "/foreign/"),
            dump.replace("/data/user/0/", "/data/user/0/./"),
            dump.replace("/data/user/0/", "/data/user//0/"),
            dump.replace("    firstInstallTime=2026-10-27 04:39:23\n", ""),
        ] {
            assert!(package_ownership_facts(&changed, &record).is_err());
        }
    }
    #[test]
    fn host_namespace_group_accepts_owned_processes_with_distinct_nspids_and_rechecks_each_original()
     {
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let test = concat!(
            module_path!(),
            "::initial_native_desktop_join_rejects_same_executable_other_process_before_fresh_client_or_request"
        );
        let test = test
            .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
            .unwrap();
        let mut command = std::process::Command::new(executable);
        command
            .args(["--exact", test, "--test-threads=1"])
            .env("GOATR_PHYSICAL_NATIVE_JOIN_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
        }
        let mut child = command.spawn().unwrap();
        let caller = herdr::process_evidence(std::process::id()).unwrap();
        let manager = herdr::process_evidence(child.id()).unwrap();
        assert_ne!(caller.namespaces.nspids, manager.namespaces.nspids);
        assert!(same_host_namespace_group(&caller, &manager));
        recheck_host_processes(&caller, &manager, None).unwrap();
        assert!(recheck_host_processes(&manager, &caller, None).is_err());
        for family in [
            "boot",
            "pid-namespace",
            "mount",
            "user",
            "empty-depth",
            "different-depth",
        ] {
            let mut changed = manager.clone();
            match family {
                "boot" => changed.linux_boot_id = Uuid::random(),
                "pid-namespace" => changed.namespaces.pid.1 += 1,
                "mount" => changed.namespaces.mount.1 += 1,
                "user" => changed.namespaces.user.1 += 1,
                "empty-depth" => changed.namespaces.nspids.clear(),
                "different-depth" => changed.namespaces.nspids.push(manager.pid),
                _ => unreachable!(),
            }
            assert!(!same_host_namespace_group(&caller, &changed), "{family}");
        }
        let mut changed_caller = caller.clone();
        changed_caller.namespaces.nspids[0] += 1;
        assert!(recheck_host_processes(&changed_caller, &manager, None).is_err());
        let mut changed_manager = manager.clone();
        changed_manager.namespaces.nspids[0] += 1;
        assert!(recheck_host_processes(&caller, &changed_manager, None).is_err());
        changed_manager = manager.clone();
        changed_manager.start_ticks += 1;
        assert!(recheck_host_processes(&caller, &changed_manager, None).is_err());
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
    }
    #[test]
    fn controller_environment_consumer_rejects_unsafe_flags_without_normalizing_live_environment() {
        let original = (
            std::env::var_os("OMP_UNSANDBOXED"),
            std::env::var_os("AGENT_SANDBOXED"),
        );
        controller_environment(false, false).unwrap();
        for flags in [(true, false), (false, true), (true, true)] {
            assert!(controller_environment(flags.0, flags.1).is_err());
        }
        assert_eq!(
            original,
            (
                std::env::var_os("OMP_UNSANDBOXED"),
                std::env::var_os("AGENT_SANDBOXED")
            )
        );
    }
}
