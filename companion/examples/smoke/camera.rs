// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed SDK36.6.11 controller consumer and sealed, never-issued synthetic QR feed.
//! No gRPC framework, endpoint creation, inline poster bytes, live capability or image files.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use std::{
    collections::HashSet,
    ffi::CString,
    io::{Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
};
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
struct Advertisement {
    path: PathBuf,
    file: File,
    stamp: (u64, u64, u64, i64, i64, i64, i64),
    port: u16,
    auth: reqwest::header::HeaderValue,
}
fn native_advertisement(l: &Lease) -> io::Result<Advertisement> {
    if !emulator_matches(l) {
        return Err(failure("native controller owner changed"));
    }
    let parent = emulator_runtime_dir(l)?.join("avd/running");
    private_dir(&parent, false)?;
    let path = parent.join(format!("pid_{}.ini", l.process_identity.pid));
    let m = fs::symlink_metadata(&path)?;
    if !m.is_file()
        || m.uid() != uid()
        || m.nlink() != 1
        || ![0o600, 0o1600].contains(&(m.mode() & 0o7777))
        || fs::canonicalize(&path)? != path
    {
        return Err(failure(
            "exact owned native-controller advertisement is not private/canonical",
        ));
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    let before = stamp(&file.metadata()?);
    let mut raw = Vec::new();
    (&mut file).take(65537).read_to_end(&mut raw)?;
    if raw.len() > 65536 || before != stamp(&fs::symlink_metadata(&path)?) {
        raw.fill(0);
        return Err(failure("native controller advertisement changed/oversized"));
    }
    let text =
        std::str::from_utf8(&raw).map_err(|_| failure("invalid native advertisement encoding"))?;
    let mut seen = HashSet::new();
    let mut port = None;
    let mut token = None;
    let mut avd = None;
    let mut serial = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| failure("invalid native advertisement shape"))?;
        let key = key.trim();
        let value = value.trim();
        if !seen.insert(key) {
            return Err(failure("duplicate native advertisement key"));
        }
        match key {
            "grpc.port" => port = value.parse::<u16>().ok(),
            "grpc.token" => token = Some(value),
            "avd.dir" => avd = Some(value),
            "port.serial" => serial = Some(value),
            _ => {}
        }
    }
    let port=port.filter(|p|*p!=0).ok_or_else(||prerequisite("already-running owned authenticated controller unavailable; never start a new endpoint"))?;
    let console_port = lease_console_port(l)?.to_string();
    if avd != l.avd_path.to_str() || serial != Some(console_port.as_str()) {
        return Err(failure(
            "native advertisement does not join exact owned AVD/console",
        ));
    }
    let token = token
        .filter(|t| {
            !t.is_empty()
                && t.len() <= 256
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=_-".contains(&b))
        })
        .ok_or_else(|| prerequisite("owned private controller authentication unavailable"))?;
    let mut value = format!("Bearer {token}");
    let mut auth = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
        .map_err(|_| failure("invalid controller authorization encoding"))?;
    auth.set_sensitive(true);
    unsafe { value.as_bytes_mut().fill(0) };
    raw.fill(0);
    let ad = Advertisement {
        path,
        file,
        stamp: before,
        port,
        auth,
    };
    controller_socket(l, port)?;
    Ok(ad)
}
fn controller_socket(l: &Lease, port: u16) -> io::Result<u64> {
    if !emulator_matches(l) {
        return Err(failure("controller process epoch changed"));
    }
    let mut owned = HashSet::new();
    for entry in fs::read_dir(format!("/proc/{}/fd", l.process_identity.pid))? {
        if let Ok(link) = fs::read_link(entry?.path())
            && let Some(n) = link
                .to_str()
                .and_then(|s| s.strip_prefix("socket:["))
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<u64>().ok())
        {
            owned.insert(n);
        }
    }
    let mut found = None;
    for protocol in ["tcp", "tcp6"] {
        let text = fs::read_to_string(format!("/proc/{}/net/{protocol}", l.process_identity.pid))?;
        for line in text.lines().skip(1) {
            let f = line.split_whitespace().collect::<Vec<_>>();
            if f.len() < 10 || f[3] != "0A" {
                continue;
            }
            let Some((address, p)) = f[1].split_once(':') else {
                continue;
            };
            if u16::from_str_radix(p, 16).ok() != Some(port) {
                continue;
            }
            if !["0100007F", "0000000000000000FFFF00000100007F"].contains(&address) {
                return Err(failure(
                    "native controller is not exact IPv4/mapped loopback; no wildcard endpoint accepted",
                ));
            }
            let inode = f[9]
                .parse::<u64>()
                .map_err(|_| failure("invalid native controller kernel socket"))?;
            if !owned.contains(&inode) {
                continue;
            }
            if found.replace(inode).is_some() {
                return Err(failure("ambiguous owned native controller listener"));
            }
        }
    }
    found.ok_or_else(|| failure("advertised controller port is not an owned live kernel listener"))
}
fn varint(bytes: &[u8], at: &mut usize) -> io::Result<u64> {
    let mut result = 0u64;
    for n in 0..10 {
        let b = *bytes
            .get(*at)
            .ok_or_else(|| failure("truncated native protobuf varint"))?;
        *at += 1;
        if n == 9 && b > 1 {
            return Err(failure("native protobuf varint overflow"));
        }
        result |= u64::from(b & 127) << (n * 7);
        if b & 128 == 0 {
            return Ok(result);
        }
    }
    Err(failure("oversized native protobuf varint"))
}
fn field<'a>(bytes: &'a [u8], at: &mut usize, wire: u64) -> io::Result<&'a [u8]> {
    let start = *at;
    let size = match wire {
        0 => {
            varint(bytes, at)?;
            return Ok(&bytes[start..*at]);
        }
        1 => 8,
        2 => usize::try_from(varint(bytes, at)?).map_err(io::Error::other)?,
        5 => 4,
        _ => return Err(failure("unsupported native protobuf wire type")),
    };
    let start = *at;
    let end = start
        .checked_add(size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| failure("native protobuf extent overflow"))?;
    *at = end;
    Ok(&bytes[start..end])
}
fn model(bytes: &[u8], wanted: u8) -> io::Result<[f32; 3]> {
    if bytes.len() > 4096 {
        return Err(failure("native PhysicalModelValue exceeds bound"));
    }
    let mut at = 0;
    let mut target = 0;
    let mut status = 0;
    let mut data = None;
    let mut seen = HashSet::new();
    while at < bytes.len() {
        let tag = varint(bytes, &mut at)?;
        let key = tag >> 3;
        let wire = tag & 7;
        if key == 0 {
            return Err(failure("zero native protobuf field"));
        }
        let b = field(bytes, &mut at, wire)?;
        if key <= 4 && !seen.insert(key) {
            return Err(failure("duplicate fixed native protobuf field"));
        }
        match key {
            1 | 2 if wire == 0 => {
                let mut i = 0;
                let value = varint(b, &mut i)?;
                if key == 1 {
                    target = value;
                } else {
                    status = value;
                }
            }
            3 if wire == 2 => {
                let mut i = 0;
                let tag = varint(b, &mut i)?;
                if tag != 10 {
                    return Err(failure("native ParameterValue has unexpected fixed field"));
                }
                let floats = field(b, &mut i, 2)?;
                if i != b.len() || floats.len() != 12 {
                    return Err(failure(
                        "native physical pose does not have exactly three packed float32 values",
                    ));
                }
                let mut result = [0.0; 3];
                for (n, value) in result.iter_mut().enumerate() {
                    *value = f32::from_le_bytes(
                        floats[n * 4..n * 4 + 4]
                            .try_into()
                            .map_err(io::Error::other)?,
                    );
                    if !value.is_finite() {
                        return Err(failure("native physical pose is nonfinite"));
                    }
                }
                data = Some(result);
            }
            1..=4 => return Err(failure("native physical model field type changed")),
            _ => {}
        }
    }
    if target != u64::from(wanted) || status != 0 {
        return Err(failure(
            "native physical model target/status is unavailable",
        ));
    }
    data.ok_or_else(|| failure("native physical model value absent"))
}
fn model_request(target: u8, values: Option<[f32; 3]>) -> Vec<u8> {
    let mut b = vec![8, target];
    if let Some(values) = values {
        b.extend_from_slice(&[26, 14, 10, 12]);
        for value in values {
            b.extend_from_slice(&value.to_le_bytes());
        }
        b.extend_from_slice(&[32, 1]);
    }
    b
}
fn grpc_message(bytes: &[u8]) -> io::Result<&[u8]> {
    if bytes.len() < 5 || bytes[0] != 0 {
        return Err(failure("native gRPC frame missing/compressed"));
    }
    let n = u32::from_be_bytes(bytes[1..5].try_into().map_err(io::Error::other)?) as usize;
    if n > 4096 || bytes.len() != n + 5 {
        return Err(failure(
            "native unary gRPC has extra/incomplete/oversized message",
        ));
    }
    Ok(&bytes[5..])
}
fn header_size(headers: &reqwest::header::HeaderMap) -> usize {
    headers
        .iter()
        .map(|(key, value)| key.as_str().len() + value.as_bytes().len())
        .sum()
}
fn crc(bytes: &[u8]) -> u32 {
    let mut value = 0xffff_ffffu32;
    for &byte in bytes {
        value ^= u32::from(byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320u32.wrapping_mul(value & 1));
        }
    }
    !value
}
fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = output.len();
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let crc = crc(&output[start..]);
    output.extend_from_slice(&crc.to_be_bytes());
}
/// One grayscale image, uncompressed zlib stored blocks; not an image/archive framework.
fn png(width: usize, height: usize, pixel: impl Fn(usize, usize) -> u8) -> io::Result<Vec<u8>> {
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err(failure("camera PNG dimension bound"));
    }
    let mut raw = Vec::with_capacity((width + 1) * height);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.push(pixel(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut a = 1u32;
    let mut b = 0u32;
    for &v in &raw {
        a = (a + u32::from(v)) % 65521;
        b = (b + a) % 65521;
    }
    for (index, chunk) in raw.chunks(65535).enumerate() {
        z.push(u8::from((index + 1) * 65535 >= raw.len()));
        let n = chunk.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    raw.fill(0);
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    png_chunk(&mut output, b"IHDR", &ihdr);
    png_chunk(&mut output, b"IDAT", &z);
    z.fill(0);
    png_chunk(&mut output, b"IEND", &[]);
    Ok(output)
}
fn sealed(mut bytes: Vec<u8>) -> io::Result<File> {
    let name = CString::new("goatr-camera-synthetic").map_err(io::Error::other)?;
    let fd =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(&bytes)?;
    bytes.fill(0);
    file.seek(SeekFrom::Start(0))?;
    if unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            libc::F_ADD_SEALS,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}
fn fixture_png(fresh: bool) -> io::Result<File> {
    let host = "523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea";
    let token = URL_SAFE_NO_PAD.encode([11u8; 32]);
    let name = if fresh {
        "Never-issued fresh camera fixture"
    } else {
        "Never-issued camera fixture"
    };
    let expiry = if fresh { 2 } else { 1 };
    let raw = format!(
        "{{\"v\":1,\"host\":\"{host}\",\"name\":\"{name}\",\"token\":\"{token}\",\"expiresAt\":{expiry}}}"
    );
    let mut uri = format!(
        "goatr://pair/v1?data={}",
        URL_SAFE_NO_PAD.encode(raw.as_bytes())
    );
    let qr = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map_err(|_| failure("synthetic camera QR encoding failed"))?;
    unsafe { uri.as_bytes_mut().fill(0) };
    let modules = qr.width();
    let scale = 12;
    let side = (modules + 8) * scale;
    // Inversion deliberately exercises actual CameraX/ZXing ALSO_INVERTED, not only decoder fixtures.
    sealed(png(side, side, |x, y| {
        let mx = x / scale;
        let my = y / scale;
        let dark = mx >= 4
            && my >= 4
            && mx < modules + 4
            && my < modules + 4
            && qr[(mx - 4, my - 4)] == qrcode::Color::Dark;
        if dark { 255 } else { 0 }
    })?)
}
pub(super) async fn app_process(l: &Lease) -> io::Result<Option<(u32, u64)>> {
    let table = adb(l, &["shell", "ps", "-A", "-o", "PID,NAME"]).await?;
    let mut found = None;
    for row in table.lines() {
        let fields = row.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 2 || !(fields[1] == APP || fields[1].starts_with(&format!("{APP}:"))) {
            continue;
        }
        let pid = fields[0]
            .parse::<u32>()
            .map_err(|_| failure("invalid actual camera target PID"))?;
        let stat = adb(l, &["shell", "cat", &format!("/proc/{pid}/stat")]).await?;
        let ticks = herdr::process_start_ticks(stat.trim(), pid)?;
        if found.replace((pid, ticks)).is_some() {
            return Err(failure(
                "multiple debug APK processes; no camera process adoption",
            ));
        }
    }
    Ok(found)
}
fn admitted_app_matches(
    current: Option<(u32, u64)>,
    owned: Option<(u32, u64)>,
) -> io::Result<bool> {
    match (current, owned) {
        (None, _) => Ok(false),
        (Some(current), Some(owned)) if current == owned => Ok(true),
        _ => Err(failure(
            "cleanup refuses replacement/unproved APK process; target untouched",
        )),
    }
}
pub(super) async fn stop_app(l: &Lease, owned: Option<(u32, u64)>) -> io::Result<bool> {
    if admitted_app_matches(app_process(l).await?, owned)? {
        // Never authorize a package stop with a late sample: require the original stream epoch again.
        if !admitted_app_matches(app_process(l).await?, owned)? {
            return Ok(false);
        }
        adb(l, &["shell", "am", "force-stop", APP]).await?;
        if app_process(l).await?.is_some() {
            return Err(failure(
                "original owned APK process did not exit; replacement never adopted",
            ));
        }
        return Ok(true);
    }
    Ok(false)
}
async fn camera_clients_closed(l: &Lease) -> io::Result<bool> {
    let dump = adb(l, &["shell", "dumpsys", "media.camera"]).await?;
    if let Some((_, rest)) = dump.split_once("Active Camera Clients:") {
        let rest = rest.trim_start();
        let body = rest
            .strip_prefix('[')
            .ok_or_else(|| failure("native active-camera client section format unavailable"))?;
        let end = body
            .find(']')
            .ok_or_else(|| failure("native active-camera client section incomplete"))?;
        return Ok(body[..end].trim().is_empty());
    }
    if dump.contains("No active camera clients yet.") {
        return Ok(true);
    }
    Err(prerequisite(
        "guest camera-service active-client schema unavailable; native release cannot be proved",
    ))
}
const POSITION_TOLERANCE: f32 = 0.005;
const ROTATION_TOLERANCE_DEGREES: f64 = 0.005;

fn position_matches(actual: &[f32; 3], expected: &[f32; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, b)| (*a - *b).abs() < POSITION_TOLERANCE)
}
fn rotation_quaternion(degrees: &[f32; 3]) -> [f64; 4] {
    let (sx, cx) = (f64::from(degrees[0]).to_radians() * 0.5).sin_cos();
    let (sy, cy) = (f64::from(degrees[1]).to_radians() * 0.5).sin_cos();
    let (sz, cz) = (f64::from(degrees[2]).to_radians() * 0.5).sin_cos();
    // PhysicalModel's XYZ construction is X * Y * Z; components below are w, x, y, z.
    [
        cx * cy * cz - sx * sy * sz,
        sx * cy * cz + cx * sy * sz,
        cx * sy * cz - sx * cy * sz,
        cx * cy * sz + sx * sy * cz,
    ]
}
fn rotation_limit_squared() -> f64 {
    // Unit-quaternion chord length is 2*sin(angle/4); avoid acos precision loss near zero.
    let chord = 2.0 * (ROTATION_TOLERANCE_DEGREES.to_radians() * 0.25).sin();
    chord * chord
}
fn rotation_matches(actual: &[f64; 4], expected: &[f64; 4], limit_squared: f64) -> bool {
    let mut same = 0.0;
    let mut opposite = 0.0;
    for (a, b) in actual.iter().zip(expected) {
        let difference = a - b;
        let sum = a + b;
        same += difference * difference;
        opposite += sum * sum;
    }
    // q and -q describe the same orientation.
    same.min(opposite) < limit_squared
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CameraFailure {
    stage: &'static str,
    exception_class: String,
}

pub(super) struct Feed<'a> {
    lease: &'a Lease,
    ad: Advertisement,
    client: reqwest::Client,
    socket: u64,
    position: [f32; 3],
    rotation: [f32; 3],
    neutral: File,
    qr: File,
    fresh: File,
    changed: bool,
    phases: Vec<&'static str>,
    failures: Vec<CameraFailure>,
    owner: Option<(u32, u64)>,
    closed: bool,
    policy: Option<(bool, u32)>,
    policy_changed: bool,
    _lock: goatr::setup::SetupLock,
}
impl<'a> Feed<'a> {
    pub(super) async fn new(l: &'a Lease) -> io::Result<Self> {
        let lock = goatr::setup::SetupLock::acquire(&l.root)?;
        if app_process(l).await?.is_some() {
            return Err(prerequisite(
                "camera producer refuses prior/unowned live debug APK process",
            ));
        }
        let ad = native_advertisement(l)?;
        let socket = controller_socket(l, ad.port)?;
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            rustls::crypto::ring::default_provider()
                .install_default()
                .map_err(|_| failure("selected rustls ring provider initialization failed"))?;
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .http2_prior_knowledge()
            .http2_max_header_list_size(8192)
            .timeout(Duration::from_secs(2))
            .connect_timeout(Duration::from_secs(1))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| failure("fixed native HTTP2 client unavailable"))?;
        let neutral = sealed(png(64, 64, |_, _| 128)?)?;
        let qr = fixture_png(false)?;
        let fresh = fixture_png(true)?;
        let mut feed = Self {
            lease: l,
            ad,
            client,
            socket,
            position: [0.; 3],
            rotation: [0.; 3],
            neutral,
            qr,
            fresh,
            changed: false,
            phases: Vec::new(),
            failures: Vec::new(),
            owner: None,
            closed: false,
            policy: None,
            policy_changed: false,
            _lock: lock,
        };
        feed.position = feed.get(0).await?;
        feed.rotation = feed.get(1).await?;
        Ok(feed)
    }
    pub(super) fn failure_status(
        &mut self,
        stage: &str,
        exception_class: String,
    ) -> io::Result<()> {
        let stage = [
            "before-scenario",
            "first-native-open",
            "first-qr-preview",
            "first-qr-synthetic-identity",
            "first-qr-authority-displayed",
            "first-qr-confirm-present",
            "first-qr-no-pair",
            "back",
            "reentry",
            "paste",
            "paste-empty-edit",
            "paste-source-privacy",
            "paste-manual-preview",
            "paste-authority",
            "paste-no-pair",
            "paste-closed",
            "rapid-back",
            "policy",
            "error",
            "retry",
            "fresh-qr",
            "final-restoration",
        ]
        .into_iter()
        .find(|known| *known == stage)
        .ok_or_else(|| failure("unknown fixed native camera failure stage"))?;
        if exception_class.is_empty()
            || exception_class.len() > 256
            || !exception_class
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'$'))
            || self.failures.len() >= 2
        {
            return Err(failure(
                "invalid/bounded native camera exception-class status",
            ));
        }
        self.failures.push(CameraFailure {
            stage,
            exception_class,
        });
        Ok(())
    }
    pub(super) fn failure(&self) -> Option<(&'static str, &str)> {
        self.failures
            .first()
            .map(|failure| (failure.stage, failure.exception_class.as_str()))
    }
    fn failure_evidence(&self, status: &'static str, cleanup_succeeded: bool) -> Value {
        json!({"gate":"native-camera","status":status,"failures":self.failures,"reachedPhases":self.phases,"originalSource":self.recheck().is_ok(),"restored":!self.changed,"closed":self.closed,"cleanupSucceeded":cleanup_succeeded})
    }
    fn recheck(&self) -> io::Result<()> {
        if !emulator_matches(self.lease)
            || controller_socket(self.lease, self.ad.port)? != self.socket
            || stamp(&self.ad.file.metadata()?) != self.ad.stamp
            || stamp(&fs::symlink_metadata(&self.ad.path)?) != self.ad.stamp
        {
            return Err(failure(
                "owned controller/socket/advertisement source epoch changed",
            ));
        }
        Ok(())
    }
    pub(super) async fn owner_status(
        &mut self,
        state: &str,
        pid: u32,
        ticks: u64,
    ) -> io::Result<()> {
        self.recheck()?;
        match state {
            "fresh-start" => {
                if self.owner.is_some() || app_process(self.lease).await? != Some((pid, ticks)) {
                    return Err(failure(
                        "camera fresh-start does not join newly launched exact target PID/start",
                    ));
                }
                self.owner = Some((pid, ticks));
            }
            "fresh-closed" => {
                if self.owner != Some((pid, ticks)) {
                    return Err(failure(
                        "native camera close belongs to another target process",
                    ));
                }
                if app_process(self.lease)
                    .await?
                    .is_some_and(|p| p != (pid, ticks))
                {
                    return Err(failure(
                        "camera target process replaced during native closure",
                    ));
                }
                self.closed = true;
            }
            _ => return Err(failure("unknown native camera ownership status")),
        }
        Ok(())
    }
    pub(super) async fn policy_capture(&mut self, enabled: bool, user: u32) -> io::Result<()> {
        if self.policy.replace((enabled, user)).is_some() {
            return Err(failure("duplicate original native camera policy capture"));
        }
        let current = adb(self.lease, &["shell", "am", "get-current-user"])
            .await?
            .trim()
            .parse::<u32>()
            .map_err(io::Error::other)?;
        if current != user {
            return Err(failure(
                "camera policy capture belongs to another guest user",
            ));
        }
        // This SDK parses USER_ID before dispatching even the help command.
        let command = super::ui::adb_command(
            self.lease,
            &["shell", "cmd", "sensor_privacy", "help", &user.to_string()],
        )
        .await?;
        let out = capture(command, 15, 1024 * 1024).await?;
        // Read-only help has a native exit contract distinct from state-changing commands.
        if !matches!(out.status.code(), Some(0 | 255)) || !out.stderr.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!(
                    "SDK sensor-policy help inspection rejected: exit {:?}, stdout {} bytes, stderr {} bytes; raw output omitted",
                    out.status.code(),
                    out.stdout.len(),
                    out.stderr.len()
                ),
            ));
        }
        let help = std::str::from_utf8(&out.stdout)
            .map_err(|_| prerequisite("native sensor-policy help is not valid UTF-8"))?;
        let mut supported = [false; 3];
        for line in help.lines().map(str::trim) {
            match line {
                "help" => supported[0] = true,
                "enable USER_ID SENSOR" => supported[1] = true,
                "disable USER_ID SENSOR" => supported[2] = true,
                _ => {}
            }
        }
        if supported != [true; 3] {
            return Err(prerequisite(
                "installed native sensor-privacy shell API unavailable; policy unchanged",
            ));
        }
        if enabled {
            self.policy_set(false).await?;
        }
        Ok(())
    }
    async fn policy_set(&mut self, enabled: bool) -> io::Result<()> {
        self.recheck()?;
        let (_, user) = self
            .policy
            .ok_or_else(|| failure("actual original native camera policy was not captured"))?;
        self.policy_changed = true;
        adb(
            self.lease,
            &[
                "shell",
                "cmd",
                "sensor_privacy",
                if enabled { "enable" } else { "disable" },
                &user.to_string(),
                "camera",
            ],
        )
        .await?;
        Ok(())
    }
    async fn rpc(&self, method: &str, protobuf: Vec<u8>) -> io::Result<Vec<u8>> {
        self.recheck()?;
        if !["getPhysicalModel", "setPhysicalModel"].contains(&method) || protobuf.len() > 64 {
            return Err(failure("fixed native controller operation bound"));
        }
        let mut body = vec![0];
        body.extend_from_slice(&(protobuf.len() as u32).to_be_bytes());
        body.extend_from_slice(&protobuf);
        let url = format!(
            "http://127.0.0.1:{}/android.emulation.control.EmulatorController/{method}",
            self.ad.port
        );
        let response = self
            .client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, self.ad.auth.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/grpc+proto")
            .header("te", "trailers")
            .header("grpc-timeout", "2000m")
            .header("grpc-accept-encoding", "identity")
            .body(body)
            .send()
            .await
            .map_err(|_| prerequisite("owned authenticated native controller RPC unavailable"))?;
        if response.status() != reqwest::StatusCode::OK
            || response.version() != reqwest::Version::HTTP_2
            || header_size(response.headers()) > 8192
            || !response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("application/grpc"))
        {
            return Err(failure("native controller HTTP2/gRPC response mismatch"));
        }
        let initial = response.headers().get("grpc-status").cloned();
        let mut status = initial;
        let mut data = Vec::new();
        let mut body = reqwest::Body::from(response);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while let Some(frame) = tokio::time::timeout_at(deadline, body.frame())
            .await
            .map_err(|_| failure("native controller bounded body deadline"))?
        {
            let frame = frame.map_err(|_| failure("native controller body failed"))?;
            match frame.into_data() {
                Ok(chunk) => {
                    if data.len() + chunk.len() > 4101 {
                        return Err(failure("native controller unary body exceeds fixed bound"));
                    }
                    data.extend_from_slice(&chunk);
                    if data.len() >= 5
                        && (data[0] != 0
                            || u32::from_be_bytes(data[1..5].try_into().map_err(io::Error::other)?)
                                > 4096)
                    {
                        return Err(failure("native gRPC compression/advertised message bound"));
                    }
                }
                Err(frame) => {
                    if let Ok(trailers) = frame.into_trailers() {
                        if header_size(&trailers) > 8192 || status.is_some() {
                            return Err(failure("duplicate/oversized native gRPC final status"));
                        }
                        status = trailers.get("grpc-status").cloned();
                    }
                }
            }
        }
        if status.as_ref().and_then(|s| s.to_str().ok()) != Some("0") {
            data.fill(0);
            return Err(failure(
                "native controller final grpc-status missing/nonzero; HTTP200 is not success",
            ));
        }
        let result = grpc_message(&data)?.to_vec();
        data.fill(0);
        self.recheck()?;
        Ok(result)
    }
    async fn get(&self, target: u8) -> io::Result<[f32; 3]> {
        model(
            &self
                .rpc("getPhysicalModel", model_request(target, None))
                .await?,
            target,
        )
    }
    async fn set(&self, target: u8, values: [f32; 3]) -> io::Result<()> {
        if self
            .rpc("setPhysicalModel", model_request(target, Some(values)))
            .await?
            .is_empty()
        {
            Ok(())
        } else {
            Err(failure(
                "native physical setter returned a nonempty fixed Empty message",
            ))
        }
    }
    async fn observed(&self, target: u8, values: [f32; 3], restoring: bool) -> io::Result<()> {
        let expected_rotation =
            (target == 1).then(|| (rotation_quaternion(&values), rotation_limit_squared()));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let now = self.get(target).await?;
            let reached = match expected_rotation.as_ref() {
                Some((expected, limit)) => {
                    rotation_matches(&rotation_quaternion(&now), expected, *limit)
                }
                None => position_matches(&now, &values),
            };
            if reached {
                return Ok(());
            }
            if Instant::now() > deadline {
                let phase = if restoring {
                    "original-state restoration"
                } else {
                    "preparation"
                };
                return Err(io::Error::other(if target == 1 {
                    format!(
                        "native rotation did not reach {ROTATION_TOLERANCE_DEGREES}-degree angular tolerance within 5 seconds during {phase}"
                    )
                } else {
                    format!(
                        "native position did not reach {POSITION_TOLERANCE} per-component distance tolerance within 5 seconds during {phase}"
                    )
                }));
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    async fn poster(&self, file: &File) -> io::Result<()> {
        self.recheck()?;
        let path = format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd());
        let reply = adb(self.lease, &["emu", "virtualscene-image", "wall", &path]).await?;
        if !reply.lines().any(|line| line.trim() == "OK") {
            return Err(failure("native poster filename enqueue rejected"));
        }
        self.recheck()
    }
    pub(super) async fn prepare(&mut self) -> io::Result<()> {
        self.changed = true;
        self.poster(&self.neutral).await?;
        // Source Y-up: actual nonsecret view proves this height clears the foreground table.
        let position = [-2.006, 1.0, 3.119];
        self.set(0, position).await?;
        self.set(1, [0.0, -150.0, 0.0]).await?;
        self.observed(0, position, false).await?;
        self.observed(1, [0.0, -150.0, 0.0], false).await
    }
    pub(super) async fn phase(&mut self, phase: &str) -> io::Result<()> {
        if self.owner.is_none() {
            return Err(failure(
                "native camera phase lacks fresh exact APK process ownership",
            ));
        }
        let expected = [
            "first-qr",
            "neutral-after-back",
            "policy-enable",
            "policy-restore",
            "second-qr",
            "policy-original",
        ];
        if (phase != "policy-original" && expected.get(self.phases.len()) != Some(&phase))
            || self.phases.last() == Some(&"policy-original")
        {
            return Err(failure("native camera barrier order/once mismatch"));
        }
        match phase {
            "first-qr" => self.poster(&self.qr).await?,
            "neutral-after-back" => self.poster(&self.neutral).await?,
            "policy-enable" => self.policy_set(true).await?,
            "policy-restore" => self.policy_set(false).await?,
            "second-qr" => self.poster(&self.fresh).await?,
            "policy-original" => {
                let (original, _) = self
                    .policy
                    .ok_or_else(|| failure("original native camera policy missing"))?;
                self.policy_set(original).await?;
                self.policy_changed = false;
            }
            _ => return Err(failure("unknown native camera phase")),
        }
        self.phases.push(if phase == "policy-original" {
            "policy-original"
        } else {
            expected[self.phases.len()]
        });
        adb(
            self.lease,
            &[
                "shell",
                "am",
                "broadcast",
                "-a",
                "io.github.trevarj.goatr.debug.CAMERA_ACCEPTANCE_ACK",
                "--es",
                "phase",
                phase,
                "-p",
                APP,
            ],
        )
        .await?;
        Ok(())
    }
    pub(super) async fn cleanup(&mut self) -> io::Result<()> {
        if !self.changed {
            return Ok(());
        }
        self.recheck()?;
        admitted_app_matches(app_process(self.lease).await?, self.owner)?;
        if !self.closed {
            stop_app(self.lease, self.owner).await?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if camera_clients_closed(self.lease).await? {
                    self.closed = true;
                    break;
                }
                if Instant::now() > deadline {
                    return Err(failure(
                        "actual native camera clients still active; poster/pose cleanup refused",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        if self.policy_changed {
            let (original, _) = self
                .policy
                .ok_or_else(|| failure("original native policy missing during cleanup"))?;
            self.policy_set(original).await?;
            self.policy_changed = false;
        }
        adb(self.lease, &["emu", "virtualscene-image", "wall"]).await?;
        self.set(0, self.position).await?;
        self.set(1, self.rotation).await?;
        self.observed(0, self.position, true).await?;
        self.observed(1, self.rotation, true).await?;
        self.changed = false;
        Ok(())
    }
    pub(super) fn evidence(&self) -> Value {
        json!({"controller":"existing authenticated owned process endpoint; no new endpoint/config","controllerPort":self.ad.port,"kernelSocketInode":self.socket.to_string(),"source":"installed SDK36.6.11 emulator_controller.proto PhysicalModelValue fields1/2/3/4 and ParameterValue packed float32","originalPosition":self.position,"originalRotation":self.rotation,"restored":!self.changed,"phases":self.phases,"input":"sealed retained PNG memfds only; inverted never-issued synthetic QR and neutral grayscale","successAuthority":"actual installed CameraX fresh-frame/ZXing preview/Back drain/reentry test, not filename ACK or queued pose setter","limits":"not live camera authorization, physical hardware, emulator source reproduction or real16KiB proof"})
    }
}
pub(super) async fn exercise(l: &Lease) -> io::Result<Value> {
    let mut feed = Feed::new(l).await?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let prepared = tokio::select! {
        result = feed.prepare() => result,
        _ = tokio::signal::ctrl_c() => Err(failure("owned camera acceptance cancelled; admitted setters may have committed")),
        _ = terminate.recv() => Err(failure("owned camera acceptance cancelled; admitted setters may have committed")),
    };
    let result = match prepared {
        Ok(()) => super::ui::instrument(l,"io.github.trevarj.goatr.PairingUiTest#realCameraFramesDecodeSyntheticInvitationAndBackReentryPasteNeverPair",None,true,Some(&mut feed)).await,
        Err(error) => Err(error),
    };
    let cleanup = match tokio::time::timeout(Duration::from_secs(25), feed.cleanup()).await {
        Ok(result) => result,
        Err(_) => Err(failure(
            "owned camera cleanup deadline; original resources retained",
        )),
    };
    if let Err(error) = cleanup {
        println!("{}", feed.failure_evidence("blocked", false));
        return Err(error);
    }
    if let Err(error) = result {
        let status = if matches!(
            error.kind(),
            io::ErrorKind::NotConnected | io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
        ) {
            "blocked"
        } else {
            "fail"
        };
        println!("{}", feed.failure_evidence(status, true));
        return Err(error);
    }
    if feed.phases
        != [
            "first-qr",
            "neutral-after-back",
            "policy-enable",
            "policy-restore",
            "second-qr",
            "policy-original",
        ]
        || !feed.closed
    {
        println!("{}", feed.failure_evidence("fail", true));
        return Err(failure(
            "actual native camera/error/retry/fresh-frame/closed barriers not all exercised",
        ));
    }
    Ok(feed.evidence())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_pose_accepts_equivalent_xyz_rotations_without_relaxing_bounds() {
        let limit = rotation_limit_squared();
        // Noncommuting X/Y rotations also pin the inspected XYZ construction order.
        assert!(rotation_matches(
            &rotation_quaternion(&[90.0, 90.0, 0.0]),
            &[0.5; 4],
            limit
        ));
        let requested = rotation_quaternion(&[0.0, -150.0, 0.0]);
        for equivalent in [
            [180.0, -30.000004, 180.0], // Actual native normalized readback.
            [0.0, 210.0, 0.0],          // Same rotation, opposite quaternion sign.
            [360.0, -150.0, 0.0],       // Full-turn axis wrap.
        ] {
            assert!(rotation_matches(
                &rotation_quaternion(&equivalent),
                &requested,
                limit
            ));
        }
        assert!(rotation_matches(
            &requested.map(|value| -value),
            &requested,
            limit
        ));
        let zero = rotation_quaternion(&[0.0, 0.0, 0.0]);
        assert!(rotation_matches(
            &rotation_quaternion(&[0.0, 0.0049, 0.0]),
            &zero,
            limit
        ));
        assert!(!rotation_matches(
            &rotation_quaternion(&[0.0, 0.0051, 0.0]),
            &zero,
            limit
        ));
        assert!(!rotation_matches(
            &rotation_quaternion(&[0.0, -149.99, 0.0]),
            &requested,
            limit
        ));
        assert!(position_matches(&[0.0049, -0.0049, 0.0049], &[0.0; 3]));
        assert!(!position_matches(
            &[POSITION_TOLERANCE, 0.0, 0.0],
            &[0.0; 3]
        ));
        assert!(!position_matches(&[0.0, 0.0051, 0.0], &[0.0; 3]));
    }
    #[test]
    fn teardown_and_death_never_adopt_a_replacement_or_unproved_app_epoch() {
        let original = (731, 1900);
        assert!(admitted_app_matches(Some(original), Some(original)).unwrap());
        assert!(!admitted_app_matches(None, Some(original)).unwrap());
        assert!(admitted_app_matches(Some((732, 1901)), Some(original)).is_err());
        assert!(admitted_app_matches(Some((731, 1901)), Some(original)).is_err());
        assert!(admitted_app_matches(Some(original), None).is_err());
    }
    #[test]
    fn fixed_native_wire_and_png_reject_overflow_wrong_target_and_extra_messages() {
        let mut response = model_request(1, Some([0., -150., 0.]));
        response.truncate(response.len() - 2);
        assert_eq!(model(&response, 1).unwrap(), [0., -150., 0.]);
        assert!(model(&response, 0).is_err());
        assert!(model(&[26, 255], 0).is_err());
        assert!(grpc_message(&[0, 0, 0, 0, 1]).is_err());
        assert!(grpc_message(&[1, 0, 0, 0, 0]).is_err());
        assert!(grpc_message(&[0, 0, 0, 0, 0, 0]).is_err());
        assert_eq!(crc(b"123456789"), 0xcbf43926);
        let image = png(2, 2, |x, y| ((x + y) % 2 * 255) as u8).unwrap();
        assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n");
        assert!(png(2049, 1, |_, _| 0).is_err());
    }
}
