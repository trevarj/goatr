// SPDX-License-Identifier: GPL-3.0-or-later
//! Stock OMP collaboration 3. Source-derived mechanics: see NOTICE.
//! Historical 18.6.1 baseline: 2a2c6dcbbb558c0f8145f67f28b3370984f2bf60.
//! Reviewed 18.8.0 candidate: 4ef97c8826ee012829a3e756b693a2a16a414f47;
//! inspected installed sources retain the guest contract, not proof of loaded
//! provider/dist identity or live compatibility. See ../../../docs/contracts/OMP_BACKEND.md.
use crate::{
    config::Config,
    herdr::{
        self, Agent, OmpCreationReporter, OmpOfflineFailure, ProcessEvidence, Source, Topology,
    },
    native_json::{self, NATIVE, REGISTRY},
    pairing::{self, Authorized, Security, SecurityResult, hex, store_error},
    protocol::*,
    state::{HistoryOrigins, ProviderSnapshot, SafeHistory, SessionUpdate, State, merge_history},
    storage::{
        AuthenticatedPeer, CreationOutcome, ReceiptTransition, Reservation, StableSessionKey,
        StageMetadata, TransitionProof,
    },
};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::AeadInPlace};
use axum::{
    Router,
    extract::{
        State as AxumState,
        ws::{CloseFrame as RelayClose, Message as RelayMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Method as HttpMethod, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use base64::{
    Engine, alphabet,
    engine::general_purpose::URL_SAFE_NO_PAD,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File, Metadata, OpenOptions},
    future::Future,
    io::{self, BufRead, Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UnixStream},
    sync::{Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};

pub const RELAY_PORT: u16 = 7466;
pub const FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const QUEUE_FRAMES: usize = 32;
pub const QUEUE_BYTES: usize = 16 * 1024 * 1024;
pub const BRANCH_NOTICE: &str = "All branches; desktop-selected branch unavailable.";
#[path="omp_skills.rs"]
mod skills;
pub(crate) fn qualify_extension_package(path:&Path)->io::Result<()> {skills::qualify_package_root(path)}
const PRODUCER_HOOK_NAME: &str = "goatr-omp-producer-state.ts";
const PRODUCER_OVERLAY_NAME: &str = "omp-collab-producer.yml";
const PRODUCER_CAPTURE: &str = "GOATR_OWNED_OMP_PRODUCER_CAPTURE";
const PRODUCER_OVERLAY_SHA256: &str = "GOATR_OWNED_OMP_PRODUCER_OVERLAY_SHA256";
const PRODUCER_HOOK_SHA256: &str = "GOATR_OWNED_OMP_PRODUCER_HOOK_SHA256";
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "OMP native evidence is invalid or unavailable",
    )
}
fn policy(_: impl std::fmt::Display) -> ErrorCode {
    ErrorCode::NotReady
}
/// The reservation callback has already registered the original job when this runs.
/// Creation returns its durable receipt instead of waiting for native completion.
fn reservation_reply(create: bool, reservation: Reservation) -> Option<Receipt> {
    match reservation {
        Reservation::Existing(receipt) => Some(receipt),
        Reservation::Reserved(receipt) if create => Some(receipt),
        Reservation::Reserved(_) => None,
    }
}
fn io_kind(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::ConnectionRefused => "connection_refused",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        io::ErrorKind::ConnectionReset => "connection_reset",
        io::ErrorKind::ConnectionAborted => "connection_aborted",
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::NotConnected => "not_connected",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::InvalidData => "invalid_data",
        _ => "other",
    }
}
fn text(v: &Value, key: &str) -> io::Result<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn uint(v: &Value, key: &str) -> io::Result<u64> {
    v.get(key)
        .and_then(Value::as_u64)
        .filter(|n| *n <= UINT_MAX)
        .ok_or_else(invalid)
}
fn object(m: &Metadata) -> (u64, u64) {
    (m.dev(), m.ino())
}
fn stamp(m: &Metadata) -> (u64, u64, u64, i64, i64) {
    (m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec())
}
fn private_file(path: &Path, roots: &[PathBuf]) -> io::Result<File> {
    crate::config::absolute(path)?;
    let canonical = fs::canonicalize(path)?;
    if canonical != path || !roots.iter().any(|r| canonical.starts_with(r)) {
        return Err(invalid());
    }
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || before.uid() != pairing::uid() || before.mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if object(&before) != object(&file.metadata()?)
        || object(&fs::symlink_metadata(path)?) != object(&before)
    {
        return Err(invalid());
    }
    Ok(file)
}
/// An allocated native pathname is not necessarily materialized. Prove its
/// protected parent and both namespace views even while the leaf is absent.
fn history_file(
    path: &Path,
    roots: &[PathBuf],
    owner: &ProcessEvidence,
) -> io::Result<(Option<File>, (u64, u64))> {
    crate::config::absolute(path)?;
    let parent = path.parent().ok_or_else(invalid)?;
    let root = roots
        .iter()
        .find(|root| parent.starts_with(root))
        .ok_or_else(invalid)?;
    for directory in parent.ancestors() {
        crate::config::project_directory(directory)?;
        if directory == root {
            break;
        }
    }
    let before = object(&fs::symlink_metadata(parent)?);
    if herdr::process_object(owner, parent)? != before {
        return Err(invalid());
    }
    let file = match fs::symlink_metadata(path) {
        Ok(_) => {
            let file = private_file(path, roots)?;
            if herdr::process_object(owner, path)? != object(&file.metadata()?) {
                return Err(invalid());
            }
            Some(file)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match herdr::process_object(owner, path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(invalid()),
            }
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(invalid()),
            }
            None
        }
        Err(error) => return Err(error),
    };
    if object(&fs::symlink_metadata(parent)?) != before
        || herdr::process_object(owner, parent)? != before
        || herdr::process_evidence(owner.pid)? != *owner
    {
        return Err(invalid());
    }
    Ok((file, before))
}
fn room_id(s: &str) -> bool {
    (10..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

pub struct Link {
    pub ws_url: String,
    pub room: String,
    cipher: Aes256Gcm,
    write_token: Option<[u8; 16]>,
}
impl Drop for Link {
    fn drop(&mut self) {
        if let Some(token) = &mut self.write_token {
            token.fill(0);
        }
    }
}
impl Link {
    pub fn writable(&self) -> bool {
        self.write_token.is_some()
    }
    pub fn parse(input: &str) -> io::Result<Self> {
        Self::parse_at(input, 0)
    }
    fn parse_at(input: &str, depth: usize) -> io::Result<Self> {
        if depth > 16 || input.len() > 65_536 {
            return Err(invalid());
        }
        let mut s = input.trim().replace("%23", "#");
        if let Some((room, secret)) = s.split_once(['.', '#'])
            && room_id(room)
            && !secret.is_empty()
            && secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            s = format!("wss://my.omp.sh/r/{room}.{secret}");
        }
        if !s.contains("://") {
            s = format!("wss://{s}");
        }
        let mut url = url::Url::parse(&s).map_err(|_| invalid())?;
        let http = matches!(url.scheme(), "http" | "https");
        if http
            && let Some(inner) = url.fragment()
            && let Ok(link) = Self::parse_at(inner, depth + 1)
        {
            return Ok(link);
        }
        let scheme = match url.scheme() {
            "wss" | "https" => "wss",
            "ws" | "http" => "ws",
            _ => return Err(invalid()),
        };
        if scheme == "ws"
            && !matches!(
                url.host_str(),
                Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
            )
        {
            return Err(invalid());
        }
        let path = url.path().strip_prefix("/r/");
        let Some(path) = path else {
            if !http && let Some(inner) = url.fragment() {
                return Self::parse_at(inner, depth + 1);
            }
            return Err(invalid());
        };
        let (room, suffix) = path
            .split_once('.')
            .map_or((path, None), |(r, s)| (r, Some(s)));
        if !room_id(room) {
            return Err(invalid());
        }
        let room = room.to_owned();
        let secret = suffix.or(url.fragment()).ok_or_else(invalid)?;
        if secret.is_empty()
            || !secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(invalid());
        }
        // Node Buffer accepts an unmatched last sextet and nonzero trailing bits.
        let usable = if secret.len() % 4 == 1 {
            &secret[..secret.len() - 1]
        } else {
            secret
        };
        let engine = GeneralPurpose::new(
            &alphabet::URL_SAFE,
            GeneralPurposeConfig::new()
                .with_decode_padding_mode(DecodePaddingMode::Indifferent)
                .with_decode_allow_trailing_bits(true),
        );
        let mut decoded = engine.decode(usable).map_err(|_| invalid())?;
        if decoded.len() != 32 && decoded.len() != 48 {
            return Err(invalid());
        }
        let cipher = Aes256Gcm::new_from_slice(&decoded[..32]).map_err(|_| invalid())?;
        let write_token = if decoded.len() == 48 {
            Some(decoded[32..].try_into().map_err(|_| invalid())?)
        } else {
            None
        };
        decoded.fill(0);
        url.set_scheme(scheme).map_err(|_| invalid())?;
        url.set_username("").map_err(|_| invalid())?;
        url.set_password(None).map_err(|_| invalid())?;
        url.set_path(&format!("/r/{room}"));
        url.set_query(None);
        url.set_fragment(None);
        Ok(Self {
            ws_url: url.to_string(),
            room,
            cipher,
            write_token,
        })
    }
    pub fn seal(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        if plaintext.len() + 32 > FRAME_BYTES {
            return Err(invalid());
        }
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| invalid())?;
        self.seal_nonce(plaintext, nonce)
    }
    fn seal_nonce(&self, plaintext: &[u8], nonce: [u8; 12]) -> io::Result<Vec<u8>> {
        if plaintext.len() + 32 > FRAME_BYTES {
            return Err(invalid());
        }
        let mut out = Vec::with_capacity(plaintext.len() + 32);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(plaintext);
        let tag = self
            .cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), b"", &mut out[16..])
            .map_err(|_| invalid())?;
        out.extend_from_slice(&tag);
        Ok(out)
    }
    fn seal_value(&self, value: &Value) -> io::Result<Vec<u8>> {
        let mut out = vec![0; 16];
        getrandom::fill(&mut out[4..16]).map_err(|_| invalid())?;
        serde_json::to_writer(&mut out, value).map_err(|_| invalid())?;
        if out.len() + 16 > FRAME_BYTES {
            return Err(invalid());
        }
        let tag = {
            let (header, payload) = out.split_at_mut(16);
            self.cipher
                .encrypt_in_place_detached(Nonce::from_slice(&header[4..16]), b"", payload)
                .map_err(|_| invalid())?
        };
        out.extend_from_slice(&tag);
        Ok(out)
    }
    pub fn open<'a>(&self, envelope: &'a mut [u8]) -> io::Result<&'a [u8]> {
        if envelope.len() < 32 || envelope.len() > FRAME_BYTES {
            return Err(invalid());
        }
        let (header, body) = envelope.split_at_mut(16);
        let end = body.len() - 16;
        let (payload, tag) = body.split_at_mut(end);

        self.cipher
            .decrypt_in_place_detached(
                Nonce::from_slice(&header[4..16]),
                b"",
                payload,
                aes_gcm::Tag::from_slice(tag),
            )
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "native envelope authentication failed",
                )
            })?;
        Ok(payload)
    }
    pub fn hello(&self) -> io::Result<Vec<u8>> {
        let token = self.write_token.ok_or_else(invalid)?;
        self.seal_value(&json!({"t":"hello","proto":3,"name":"Goatr","writeToken":URL_SAFE_NO_PAD.encode(token)}))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    instance: String,
    pid: u32,
    endpoint: PathBuf,
    token: String,
    file: PathBuf,
    identity: (u64, u64, u64, i64, i64),
}
enum RegistryFailure {
    /// The periodic registry observer may miss one bounded observation without
    /// replacing the already authenticated guest/source.
    TimedOut,
    Other(io::Error),
}
impl RegistryFailure {
    fn other(error: io::Error) -> Self {
        Self::Other(error)
    }
    fn invalid() -> Self {
        Self::Other(invalid())
    }
    fn into_io(self) -> io::Error {
        match self {
            Self::TimedOut => invalid(),
            Self::Other(error) => error,
        }
    }
    fn is_timeout(&self) -> bool {
        matches!(self, Self::TimedOut)
    }
}
impl Drop for RegistryEntry {
    fn drop(&mut self) {
        self.token.clear();
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub instance: String,
    pub generation: u64,
    pub pid: u32,
    pub session: String,
    pub cwd: String,
    pub relay_connected: bool,
    pub control: bool,
    pub busy: Option<bool>,
    pub input_required: bool,
}
type RegistryObservation = Pin<
    Box<dyn Future<Output = Result<(RegistrySnapshot, ProcessEvidence), RegistryFailure>> + Send>,
>;
impl RegistrySnapshot {
    fn same_binding(&self, other: &Self) -> bool {
        self.instance == other.instance
            && self.generation == other.generation
            && self.pid == other.pid
            && self.session == other.session
            && self.cwd == other.cwd
    }
}
pub fn registry(dir: &Path) -> io::Result<Vec<RegistryEntry>> {
    pairing::private_dir(dir, false)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let path = item?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if entries.len() >= 512 {
            return Err(invalid());
        }
        let file = private_file(&path, &[dir.to_owned()])?;
        let m = file.metadata()?;
        if m.len() > 65_536 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(65_537).read_to_end(&mut bytes)?;
        let value = native_json::parse(&bytes, REGISTRY)?;
        bytes.fill(0);
        let instance = text(&value, "instanceId")?;
        if uint(&value, "version")? != 1
            || !(8..=64).contains(&instance.len())
            || !instance
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid());
        }
        let endpoint = PathBuf::from(text(&value, "endpoint")?);
        crate::config::socket_path(&endpoint)?;
        crate::config::socket_parent(&endpoint)?;
        let token = text(&value, "token")?;
        if token.is_empty() || token.len() > 1024 {
            return Err(invalid());
        }
        let pid = u32::try_from(uint(&value, "pid")?).map_err(|_| invalid())?;
        if pid == 0 {
            return Err(invalid());
        }
        entries.push(RegistryEntry {
            instance,
            pid,
            endpoint,
            token,
            file: path,
            identity: stamp(&m),
        });
    }
    Ok(entries)
}
impl RegistryEntry {
    async fn query_checked(
        &self,
        op: &str,
        generation: Option<u64>,
    ) -> Result<(Value, ProcessEvidence), RegistryFailure> {
        tokio::time::timeout(Duration::from_millis(1500), async {
            let file = fs::symlink_metadata(&self.file).map_err(RegistryFailure::other)?;
            let file_stamp = stamp(&file);
            if file_stamp != self.identity {
                return Err(RegistryFailure::invalid());
            }
            let m = fs::symlink_metadata(&self.endpoint).map_err(RegistryFailure::other)?;
            use std::os::unix::fs::FileTypeExt;
            if !m.file_type().is_socket() || m.uid() != pairing::uid() || m.mode() & 0o077 != 0 {
                return Err(RegistryFailure::invalid());
            }
            let mut stream = UnixStream::connect(&self.endpoint)
                .await
                .map_err(RegistryFailure::other)?;
            let cred = stream.peer_cred().map_err(RegistryFailure::other)?;
            if cred.uid() != pairing::uid() {
                return Err(RegistryFailure::invalid());
            }
            let pid = u32::try_from(cred.pid().ok_or_else(RegistryFailure::invalid)?)
                .map_err(|_| RegistryFailure::invalid())?;
            let owner = herdr::process_evidence(pid).map_err(RegistryFailure::other)?;
            if owner.namespaces.nspids.last() != Some(&self.pid) {
                return Err(RegistryFailure::invalid());
            }
            let request = if let Some(g) = generation {
                json!({"v":1,"token":self.token,"op":op,"access":"control","generation":g})
            } else {
                json!({"v":1,"token":self.token,"op":op})
            };
            let mut bytes = serde_json::to_vec(&request).map_err(|_| RegistryFailure::invalid())?;
            bytes.push(b'\n');
            if bytes.len() > 4096 {
                return Err(RegistryFailure::invalid());
            }
            stream
                .write_all(&bytes)
                .await
                .map_err(RegistryFailure::other)?;
            bytes.fill(0);
            let mut reader = BufReader::new(stream);
            let mut result = Vec::new();
            loop {
                let available = reader.fill_buf().await.map_err(RegistryFailure::other)?;
                if available.is_empty() {
                    return Err(RegistryFailure::invalid());
                }
                let n = available
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(available.len(), |n| n + 1);
                if result.len() + n > 65_536 {
                    return Err(RegistryFailure::invalid());
                }
                let done = available[n - 1] == b'\n';
                result.extend_from_slice(&available[..n]);
                reader.consume(n);
                if done {
                    break;
                }
            }
            let value = native_json::parse(&result, REGISTRY).map_err(RegistryFailure::other)?;
            result.fill(0);
            if uint(&value, "v").map_err(RegistryFailure::other)? != 1
                || value.get("ok").and_then(Value::as_bool) != Some(true)
            {
                return Err(RegistryFailure::invalid());
            }
            if herdr::process_evidence(owner.pid).map_err(RegistryFailure::other)? != owner {
                return Err(RegistryFailure::invalid());
            }
            let endpoint = fs::symlink_metadata(&self.endpoint).map_err(RegistryFailure::other)?;
            if object(&endpoint) != object(&m) {
                return Err(RegistryFailure::invalid());
            }
            let file = fs::symlink_metadata(&self.file).map_err(RegistryFailure::other)?;
            let file_stamp = stamp(&file);
            if file_stamp != self.identity {
                return Err(RegistryFailure::invalid());
            }
            Ok((value, owner))
        })
        .await
        .map_err(|_| RegistryFailure::TimedOut)?
    }
    async fn query(
        &self,
        op: &str,
        generation: Option<u64>,
    ) -> io::Result<(Value, ProcessEvidence)> {
        self.query_checked(op, generation)
            .await
            .map_err(RegistryFailure::into_io)
    }
    async fn snapshot_checked(
        &self,
    ) -> Result<(RegistrySnapshot, ProcessEvidence), RegistryFailure> {
        let (value, owner) = self.query_checked("snapshot", None).await?;
        let v = value.get("snapshot").ok_or_else(RegistryFailure::invalid)?;
        let snapshot = RegistrySnapshot {
            instance: text(v, "instanceId").map_err(RegistryFailure::other)?,
            generation: uint(v, "generation").map_err(RegistryFailure::other)?,
            pid: u32::try_from(uint(v, "pid").map_err(RegistryFailure::other)?)
                .map_err(|_| RegistryFailure::invalid())?,
            session: text(v, "sessionId").map_err(RegistryFailure::other)?,
            cwd: text(v, "cwd").map_err(RegistryFailure::other)?,
            relay_connected: v
                .get("relayConnected")
                .and_then(Value::as_bool)
                .ok_or_else(RegistryFailure::invalid)?,
            control: text(v, "access").map_err(RegistryFailure::other)? == "control",
            busy: v.get("busy").and_then(Value::as_bool),
            input_required: v
                .get("inputRequired")
                .and_then(Value::as_bool)
                .ok_or_else(RegistryFailure::invalid)?,
        };
        if snapshot.instance != self.instance {
            return Err(RegistryFailure::invalid());
        }
        if snapshot.pid != self.pid {
            return Err(RegistryFailure::invalid());
        }
        if snapshot.generation == 0 {
            return Err(RegistryFailure::invalid());
        }
        if snapshot.session.is_empty() {
            return Err(RegistryFailure::invalid());
        }
        crate::config::absolute(Path::new(&snapshot.cwd)).map_err(RegistryFailure::other)?;
        Ok((snapshot, owner))
    }
    pub async fn snapshot(&self) -> io::Result<(RegistrySnapshot, ProcessEvidence)> {
        self.snapshot_checked()
            .await
            .map_err(RegistryFailure::into_io)
    }
    pub async fn link(
        &self,
        before: &RegistrySnapshot,
        owner: &ProcessEvidence,
    ) -> io::Result<Link> {
        let (value, proof) = self.query("link", Some(before.generation)).await?;
        if &proof != owner {
            return Err(invalid());
        }
        let link = Link::parse(&text(&value, "url")?)?;
        let (after, proof) = self.snapshot().await?;
        if !after.same_binding(before)
            || &proof != owner
            || !link.writable()
            || !after.control
            || !after.relay_connected
        {
            return Err(invalid());
        }
        Ok(link)
    }
}

struct Queued {
    message: RelayMessage,
    _bytes: tokio::sync::OwnedSemaphorePermit,
}
#[derive(Clone)]
struct RelayPeer {
    id: Uuid,
    tx: mpsc::Sender<Queued>,
    budget: Arc<Semaphore>,
    close: watch::Sender<Option<u16>>,
}
impl RelayPeer {
    fn send(&self, message: RelayMessage) {
        let size = match &message {
            RelayMessage::Binary(v) | RelayMessage::Ping(v) | RelayMessage::Pong(v) => v.len(),
            RelayMessage::Text(v) => v.len(),
            RelayMessage::Close(v) => v.as_ref().map_or(2, |v| v.reason.len() + 2),
        };
        let Ok(permit) = self
            .budget
            .clone()
            .try_acquire_many_owned(size.max(1) as u32)
        else {
            let _ = self.close.send(Some(4001));
            return;
        };
        if self
            .tx
            .try_send(Queued {
                message,
                _bytes: permit,
            })
            .is_err()
        {
            let _ = self.close.send(Some(4001));
        }
    }
    fn shutdown(&self, code: u16) {
        let _ = self.close.send(Some(code));
    }
}
struct Room {
    host: RelayPeer,
    guests: HashMap<u32, RelayPeer>,
    next: u32,
}
#[derive(Clone)]
struct RelayState {
    port: u16,
    rooms: Arc<Mutex<HashMap<String, Room>>>,
}
pub struct Relay {
    listener: TcpListener,
    state: RelayState,
}
/// Non-installed smoke ownership only; no production port/config/environment override.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OwnedRelayListener {
    pub v: u32,
    pub fd: i32,
    pub device: u64,
    pub inode: u64,
    pub port: u16,
    pub parent: ProcessEvidence,
    pub runtime_dir: PathBuf,
}
impl Relay {
    pub fn inherited(fd: i32, record_path: &Path, runtime: &Path) -> io::Result<Self> {
        if fd < 3 || record_path != runtime.join("omp-relay.owner") {
            return Err(invalid());
        }
        let source = crate::setup::ExpectedFile::capture(record_path)?;
        let record: OwnedRelayListener =
            serde_json::from_slice(source.bytes().ok_or_else(invalid)?).map_err(|_| invalid())?;
        pairing::private_dir(runtime, false)?;
        if record.v != 1
            || record.fd != fd
            || record.runtime_dir != runtime
            || herdr::process_evidence(record.parent.pid)? != record.parent
            || fs::metadata(format!("/proc/{}", record.parent.pid))?.uid() != pairing::uid()
        {
            return Err(invalid());
        }
        // Dup first; never close/adopt an arbitrary caller descriptor on a rejected source.
        let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicated < 0 {
            return Err(io::Error::last_os_error());
        }
        let descriptor = unsafe { OwnedFd::from_raw_fd(duplicated) };
        let metadata = fs::metadata(format!("/proc/self/fd/{}", descriptor.as_raw_fd()))?;
        let parent = fs::metadata(format!("/proc/{}/fd/{}", record.parent.pid, record.fd))?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != pairing::uid()
            || (metadata.dev(), metadata.ino()) != (record.device, record.inode)
            || (parent.dev(), parent.ino()) != (record.device, record.inode)
        {
            return Err(invalid());
        }
        for (option, expected) in [(libc::SO_TYPE, libc::SOCK_STREAM), (libc::SO_ACCEPTCONN, 1)] {
            let mut actual = 0i32;
            let mut len = std::mem::size_of::<i32>() as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    descriptor.as_raw_fd(),
                    libc::SOL_SOCKET,
                    option,
                    (&mut actual as *mut i32).cast(),
                    &mut len,
                )
            } != 0
                || actual != expected
            {
                return Err(invalid());
            }
        }
        let listener = std::net::TcpListener::from(descriptor);
        let address = listener.local_addr()?;
        if address.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            || address.port() == 0
            || address.port() != record.port
        {
            return Err(invalid());
        }
        source.recheck(record_path)?;
        if herdr::process_evidence(record.parent.pid)? != record.parent {
            return Err(invalid());
        }
        listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(listener)?;
        Ok(Self {
            listener,
            state: RelayState {
                port: record.port,
                rooms: Arc::new(Mutex::new(HashMap::new())),
            },
        })
    }
    pub async fn bind(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            state: RelayState {
                port,
                rooms: Arc::new(Mutex::new(HashMap::new())),
            },
        })
    }
    pub fn port(&self) -> u16 {
        self.state.port
    }
    pub async fn run(self, mut stop: watch::Receiver<bool>) -> io::Result<()> {
        let state = self.state.clone();
        let router = Router::new()
            .fallback(relay_route)
            .with_state(state.clone());
        let shutdown = async move {
            while !*stop.borrow() {
                if stop.changed().await.is_err() {
                    break;
                }
            }
            let mut rooms = state.rooms.lock();
            for room in rooms.values() {
                for guest in room.guests.values() {
                    guest.send(RelayMessage::Text("{\"t\":\"room-closed\"}".into()));
                    guest.shutdown(4001);
                }
                room.host.shutdown(1001);
            }
            rooms.clear();
        };
        axum::serve(self.listener, router)
            .with_graceful_shutdown(shutdown)
            .await
    }
}
async fn relay_route(
    AxumState(state): AxumState<RelayState>,
    method: HttpMethod,
    uri: Uri,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let room = uri.path().strip_prefix("/r/").filter(|r| room_id(r));
    let role = match uri.query() {
        Some("role=host") => Some(true),
        Some("role=guest") => Some(false),
        _ => None,
    };
    let (Some(room), Some(host)) = (room, role) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if method != HttpMethod::GET {
        return StatusCode::NOT_FOUND.into_response();
    }
    if headers.contains_key("origin")
        || headers.get("host").and_then(|h| h.to_str().ok())
            != Some(format!("127.0.0.1:{}", state.port).as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(upgrade) = upgrade else {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    };
    let room = room.to_owned();
    upgrade
        .max_frame_size(FRAME_BYTES)
        .max_message_size(FRAME_BYTES)
        .on_upgrade(move |socket| relay_socket(state, room, host, socket))
        .into_response()
}
async fn relay_socket(state: RelayState, name: String, host: bool, mut socket: WebSocket) {
    let (tx, mut rx) = mpsc::channel(QUEUE_FRAMES);
    let (close, mut closed) = watch::channel(None);
    let peer = RelayPeer {
        id: Uuid::random(),
        tx,
        budget: Arc::new(Semaphore::new(QUEUE_BYTES)),
        close,
    };
    let registered = {
        let mut rooms = state.rooms.lock();
        if host {
            if rooms.contains_key(&name) {
                Err(4009)
            } else if rooms.len() >= 64 {
                Err(4029)
            } else {
                rooms.insert(
                    name.clone(),
                    Room {
                        host: peer.clone(),
                        guests: HashMap::new(),
                        next: 1,
                    },
                );
                Ok(0)
            }
        } else if let Some(room) = rooms.get_mut(&name) {
            if room.guests.len() >= 32 || room.next == u32::MAX {
                Err(4029)
            } else {
                let id = room.next;
                room.next += 1;
                room.guests.insert(id, peer.clone());
                room.host.send(RelayMessage::Text(
                    format!("{{\"t\":\"peer-joined\",\"peer\":{id}}}").into(),
                ));
                Ok(id)
            }
        } else {
            Err(4004)
        }
    };
    let id = match registered {
        Ok(id) => id,
        Err(code) => {
            let _ = socket
                .send(RelayMessage::Close(Some(RelayClose {
                    code,
                    reason: "room unavailable".into(),
                })))
                .await;
            return;
        }
    };
    loop {
        tokio::select! {
            _=closed.changed()=>{let code=(*closed.borrow()).unwrap_or(4001);let _=tokio::time::timeout(Duration::from_secs(1),async{while let Ok(next)=rx.try_recv(){socket.send(next.message).await.map_err(|_|invalid())?;}socket.send(RelayMessage::Close(Some(RelayClose{code,reason:"resnapshot required".into()}))).await.map_err(|_|invalid())}).await;break;}
            next=rx.recv()=>{let Some(next)=next else{break;};if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(next.message)).await,Ok(Ok(()))){break;}}
            next=socket.recv()=>{match next{Some(Ok(RelayMessage::Binary(bytes)))=>{if bytes.len()<4{continue;}let rooms=state.rooms.lock();let Some(room)=rooms.get(&name)else{break;};if host{if room.host.id!=peer.id{break;}let target=u32::from_be_bytes(bytes[..4].try_into().expect("prefix"));if target==0{for guest in room.guests.values(){guest.send(RelayMessage::Binary(bytes.clone()));}}else if let Some(guest)=room.guests.get(&target){guest.send(RelayMessage::Binary(bytes));}}else{if !room.guests.get(&id).is_some_and(|guest| guest.id == peer.id){break;}let mut bytes=bytes.to_vec();bytes[..4].copy_from_slice(&id.to_be_bytes());room.host.send(RelayMessage::Binary(bytes.into()));}},Some(Ok(RelayMessage::Text(_)|RelayMessage::Ping(_)|RelayMessage::Pong(_)))=>{},_=>break,}}
        }
    }
    let mut rooms = state.rooms.lock();
    if host {
        if rooms.get(&name).is_some_and(|r| r.host.id == peer.id)
            && let Some(room) = rooms.remove(&name)
        {
            for guest in room.guests.values() {
                guest.send(RelayMessage::Text("{\"t\":\"room-closed\"}".into()));
                guest.shutdown(4001);
            }
        }
    } else if let Some(room) = rooms.get_mut(&name)
        && room
            .guests
            .get(&id)
            .is_some_and(|guest| guest.id == peer.id)
    {
        room.guests.remove(&id);
        room.host.send(RelayMessage::Text(
            format!("{{\"t\":\"peer-left\",\"peer\":{id}}}").into(),
        ));
    }
}

fn overlay_path(config: &Config) -> PathBuf {
    config.runtime_dir.join("omp-collab.yml")
}

fn producer_overlay_path(config: &Config) -> PathBuf {
    config.runtime_dir.join(PRODUCER_OVERLAY_NAME)
}

fn write_overlay(config: &Config, expected: &[u8]) -> io::Result<()> {
    pairing::private_dir(&config.runtime_dir, false)?;
    write_overlay_at(config, &overlay_path(config), expected)
}

fn write_overlay_at(config: &Config, path: &Path, expected: &[u8]) -> io::Result<()> {
    pairing::private_dir(&config.runtime_dir, false)?;
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(expected)?;
            file.sync_all()?;
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let mut bytes = Vec::new();
            private_file(path, std::slice::from_ref(&config.runtime_dir))?
                .take(16 * 1024)
                .read_to_end(&mut bytes)?;
            if bytes != expected {
                return Err(invalid());
            }
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn environment(name: &str) -> io::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(invalid()),
    }
}

fn diagnostic_overlay_environment(config: &Config) -> io::Result<Option<(String, String)>> {
    let capture = environment(PRODUCER_CAPTURE)?;
    let overlay = environment(PRODUCER_OVERLAY_SHA256)?;
    let hook = environment(PRODUCER_HOOK_SHA256)?;
    if capture.is_none() && overlay.is_none() && hook.is_none() {
        return Ok(None);
    }
    if capture.as_deref() != Some("1")
        || std::env::var_os("GOATR_OWNED_OMP_CREATION_CAPTURE").as_deref()
            != Some(std::ffi::OsStr::new("1"))
    {
        return Err(invalid());
    }
    let (Some(overlay), Some(hook)) = (overlay, hook) else {
        return Err(invalid());
    };
    if !valid_sha256(&overlay) || !valid_sha256(&hook) {
        return Err(invalid());
    }
    let overlay_path = producer_overlay_path(config);
    let mut actual_overlay = Vec::new();
    private_file(&overlay_path, std::slice::from_ref(&config.runtime_dir))?
        .take(16 * 1024)
        .read_to_end(&mut actual_overlay)?;
    if hex(&sha256(&actual_overlay)) != overlay {
        return Err(invalid());
    }
    let hook_path = config.runtime_dir.join(PRODUCER_HOOK_NAME);
    let mut actual_hook = Vec::new();
    private_file(&hook_path, std::slice::from_ref(&config.runtime_dir))?
        .take(64 * 1024)
        .read_to_end(&mut actual_hook)?;
    if actual_hook.len() == 64 * 1024 || hex(&sha256(&actual_hook)) != hook {
        return Err(invalid());
    }
    Ok(Some((overlay, hook)))
}

/// Private one-run overlay. The helper supplies the already-proved effective
/// disabled array; arrays in `--config` replace, rather than merge, settings.
pub fn producer_overlay(
    config: &Config,
    port: u16,
    effective_disabled_extensions: &[String],
    hook_sha256: &str,
) -> io::Result<String> {
    if !valid_sha256(hook_sha256) {
        return Err(invalid());
    }
    let hook_path = config.runtime_dir.join(PRODUCER_HOOK_NAME);
    let mut hook = Vec::new();
    private_file(&hook_path, std::slice::from_ref(&config.runtime_dir))?
        .take(64 * 1024)
        .read_to_end(&mut hook)?;
    if hook.len() == 64 * 1024 || hex(&sha256(&hook)) != hook_sha256 {
        return Err(invalid());
    }
    let mut expected = format!(
        "collab:\n  autoStart: control\n  relayUrl: ws://127.0.0.1:{port}\ndisabledExtensions:\n"
    )
    .into_bytes();
    let stock = "extension-module:herdr-omp-agent-state".to_owned();
    if effective_disabled_extensions
        .iter()
        .any(|value| value == &stock)
    {
        // An existing exclusion is user configuration. The private explicit
        // hook would otherwise re-enable reporting for this one diagnostic.
        return Err(invalid());
    }
    let mut disabled = effective_disabled_extensions.to_vec();
    disabled.push(stock);
    for value in disabled {
        if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(invalid());
        }
        let scalar = serde_json::to_string(&value).map_err(|_| invalid())?;
        expected.extend_from_slice(b"  - ");
        expected.extend_from_slice(scalar.as_bytes());
        expected.push(b'\n');
    }
    if expected.len() > 16 * 1024 {
        return Err(invalid());
    }
    write_overlay_at(config, &producer_overlay_path(config), &expected)?;
    Ok(hex(&sha256(&expected)))
}

fn diagnostic_extension_args(config: &Config, authorized: bool) -> io::Result<Vec<String>> {
    if diagnostic_overlay_environment(config)?.is_none() {
        return Ok(Vec::new());
    }
    if !authorized {
        return Err(invalid());
    }
    let hook = config.runtime_dir_agent.join(PRODUCER_HOOK_NAME);
    crate::config::absolute(&hook)?;
    Ok(vec![
        "--extension".into(),
        hook.into_os_string().into_string().map_err(|_| invalid())?,
    ])
}

pub fn overlay(config: &Config, port: u16) -> io::Result<PathBuf> {
    let expected = format!("collab:\n  autoStart: control\n  relayUrl: ws://127.0.0.1:{port}\n");
    write_overlay(config, expected.as_bytes())?;
    Ok(config.runtime_dir_agent.join("omp-collab.yml"))
}

fn creation_overlay(config: &Config, port: u16, authorized: bool) -> io::Result<PathBuf> {
    if diagnostic_overlay_environment(config)?.is_some() {
        if !authorized {
            return Err(invalid());
        }
        return Ok(config.runtime_dir_agent.join(PRODUCER_OVERLAY_NAME));
    }
    overlay(config, port)
}

struct FileEntry {
    offset: u64,
    length: usize,
    #[cfg(test)]
    id: Id,
    order: u64,
    records: usize,
}
pub struct HistoryFile {
    path: PathBuf,
    roots: Vec<PathBuf>,
    owner: ProcessEvidence,
    identity: Option<(u64, u64)>,
    parent: (u64, u64),
    session: String,
    size: u64,
    completed: u64,
    pub header: Value,
    pub title: Option<String>,
    entries: Vec<FileEntry>,
    ids: HashSet<Id>,
    // Browsing may advance the file index before the live reducer publishes it.
    published: Option<(usize, Option<String>)>,
}
fn native_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let n = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |n| n + 1);
        if line.len() + n > FRAME_BYTES {
            return Err(invalid());
        }
        let complete = bytes[n - 1] == b'\n';
        line.extend_from_slice(&bytes[..n]);
        reader.consume(n);
        if complete {
            return Ok(Some(line));
        }
    }
}
impl HistoryFile {
    /// Native registry/welcome authority grants live identity, never a synthetic
    /// file/header. An existing or later file must independently join that ID.
    pub fn live(
        path: PathBuf,
        roots: Vec<PathBuf>,
        owner: ProcessEvidence,
        session: String,
    ) -> io::Result<Self> {
        let (file, parent) = history_file(&path, &roots, &owner)?;
        if let Some(file) = file {
            let history = Self::from_file(path, roots, owner, file, parent)?;
            if history.session != session {
                return Err(invalid());
            }
            return Ok(history);
        }
        Ok(Self {
            path,
            roots,
            owner,
            identity: None,
            parent,
            session,
            size: 0,
            completed: 0,
            header: Value::Null,
            title: None,
            entries: Vec::new(),
            ids: HashSet::new(),
            published: None,
        })
    }
    pub fn open(path: PathBuf, roots: Vec<PathBuf>, owner: ProcessEvidence) -> io::Result<Self> {
        let (file, parent) = history_file(&path, &roots, &owner)?;
        Self::from_file(path, roots, owner, file.ok_or_else(invalid)?, parent)
    }
    fn from_file(
        path: PathBuf,
        roots: Vec<PathBuf>,
        owner: ProcessEvidence,
        file: File,
        parent: (u64, u64),
    ) -> io::Result<Self> {
        let m = file.metadata()?;
        let mut reader = std::io::BufReader::new(file);

        let first = native_line(&mut reader)?;

        let first = first.ok_or_else(invalid)?;
        let mut completed = first.len() as u64;

        let mut header = native_json::parse(&first, NATIVE)?;
        let mut title = None;
        if header.get("type").and_then(Value::as_str) == Some("title") {
            if first.len() != 256
                || uint(&header, "v")? != 1
                || header.get("pad").and_then(Value::as_str).is_none()
            {
                return Err(invalid());
            }
            title = Some(text(&header, "title")?);

            let line = native_line(&mut reader)?;

            let line = line.ok_or_else(invalid)?;
            completed += line.len() as u64;

            header = native_json::parse(&line, NATIVE)?;
        }

        if header.get("type").and_then(Value::as_str) != Some("session")
            || text(&header, "id")?.is_empty()
        {
            return Err(invalid());
        }
        let mut history = Self {
            path,
            roots,
            owner,
            identity: Some(object(&m)),
            parent,
            session: text(&header, "id")?,
            size: m.len(),
            completed,
            header,
            title,
            entries: Vec::new(),
            ids: HashSet::new(),
            published: None,
        };
        history.scan(&mut reader)?;
        history.recheck()?;
        Ok(history)
    }
    fn scan<R: BufRead>(&mut self, reader: &mut R) -> io::Result<()> {
        while let Some(line) = native_line(reader)? {
            if self.entries.len() >= 131_072 {
                return Err(invalid());
            }

            let value = native_json::parse(&line, NATIVE)?;

            let id = Id::new(text(&value, "id")?).map_err(|_| invalid())?;

            if !self.ids.insert(id.clone()) {
                return Err(invalid());
            }
            let order = self
                .entries
                .last()
                .map_or(0, |e| e.order + e.records as u64);
            let records = projected_count(&value);
            self.entries.push(FileEntry {
                offset: self.completed,
                length: line.len(),
                #[cfg(test)]
                id,
                order,
                records,
            });
            self.completed += line.len() as u64;
        }
        Ok(())
    }
    pub fn recheck(&self) -> io::Result<()> {
        let (file, parent) = history_file(&self.path, &self.roots, &self.owner)?;

        if parent != self.parent {
            return Err(invalid());
        }

        let Some(file) = file else {
            return if self.identity.is_none() {
                Ok(())
            } else {
                Err(invalid())
            };
        };

        let m = file.metadata()?;

        if self.identity.is_some_and(|identity| identity != object(&m)) || m.len() < self.size {
            return Err(invalid());
        }
        let mut reader = std::io::BufReader::new(file);

        let first = native_line(&mut reader)?;

        let first = first.ok_or_else(invalid)?;

        let mut value = native_json::parse(&first, NATIVE)?;
        if value.get("type").and_then(Value::as_str) == Some("title") {
            if first.len() != 256
                || uint(&value, "v")? != 1
                || value.get("pad").and_then(Value::as_str).is_none()
            {
                return Err(invalid());
            }

            let line = native_line(&mut reader)?;

            let line = line.ok_or_else(invalid)?;

            value = native_json::parse(&line, NATIVE)?;
        }

        if value.get("type").and_then(Value::as_str) != Some("session")
            || text(&value, "id")? != self.session
        {
            return Err(invalid());
        }

        if self.identity.is_some() && value != self.header {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn refresh(&mut self) -> io::Result<bool> {
        if self.identity.is_none() {
            let (file, parent) = history_file(&self.path, &self.roots, &self.owner)?;

            if parent != self.parent {
                return Err(invalid());
            }
            let Some(file) = file else {
                return Ok(false);
            };
            let materialized = Self::from_file(
                self.path.clone(),
                self.roots.clone(),
                self.owner.clone(),
                file,
                parent,
            )?;

            if materialized.session != self.session {
                return Err(invalid());
            }
            *self = materialized;
            return Ok(true);
        }
        self.recheck()?;

        let file = private_file(&self.path, &self.roots)?;

        let size = file.metadata()?.len();
        let mut reader = std::io::BufReader::new(file);

        let first = native_line(&mut reader)?;

        let first = first.ok_or_else(invalid)?;

        let slot = native_json::parse(&first, NATIVE)?;

        let title = if slot.get("type").and_then(Value::as_str) == Some("title") {
            Some(text(&slot, "title")?)
        } else {
            self.title.clone()
        };
        let title_changed = title != self.title;
        self.title = title;
        if size == self.size {
            return Ok(title_changed);
        }

        reader.seek(SeekFrom::Start(self.completed))?;
        let before = self.entries.len();
        self.scan(&mut reader)?;
        self.size = size;
        self.recheck()?;
        Ok(title_changed || self.entries.len() != before)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn record_count(&self) -> usize {
        self.entries
            .last()
            .map_or(0, |e| e.order as usize + e.records)
    }
    fn entry(&self, index: usize) -> SecurityResult<Value> {
        let entry = self.entries.get(index).ok_or(ErrorCode::CursorExpired)?;

        let mut file = private_file(&self.path, &self.roots).map_err(policy)?;

        file.seek(SeekFrom::Start(entry.offset)).map_err(policy)?;
        let mut bytes = vec![0; entry.length];

        file.read_exact(&mut bytes).map_err(policy)?;

        native_json::parse(&bytes, NATIVE).map_err(policy)
    }
    #[cfg(test)]
    fn refetch_cursor(&self, anchor: Option<(Id, Option<Id>)>) -> SecurityResult<Option<Id>> {
        self.recheck().map_err(policy)?;
        let Some((id, parent)) = anchor else {
            return Ok((!self.entries.is_empty())
                .then(|| Id(format!("omp-file:{}:0", self.entries.len()))));
        };
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.id == id || parent.as_ref() == Some(&e.id))
        {
            let value = self.entry(index)?;
            let position = if id == self.entries[index].id {
                1
            } else {
                value
                    .get("message")
                    .and_then(|v| v.get("content"))
                    .and_then(Value::as_array)
                    .and_then(|blocks| {
                        blocks
                            .iter()
                            .filter(|b| b.get("type").and_then(Value::as_str) == Some("toolCall"))
                            .position(|b| {
                                b.get("id")
                                    .and_then(Value::as_str)
                                    .is_some_and(|call| id.0 == format!("omp-tool:{call}"))
                            })
                    })
                    .map_or(0, |n| n + 2)
            };
            return Ok(Some(Id(format!("omp-file:{}:{position}", index + 1))));
        }
        Ok((!self.entries.is_empty()).then(|| Id(format!("omp-file:{}:0", self.entries.len()))))
    }
    pub fn page(
        &self,
        end: usize,
        limit: usize,
        state: &State,
        target: &Target,
        source: &Uuid,
        blob: Option<&Path>,
    ) -> SecurityResult<Vec<SafeHistory>> {
        self.native_page((end, 0, limit), state, target, source, blob)
            .map(|(records, _)| records)
    }
    fn native_page(
        &self,
        bounds: (usize, usize, usize),
        state: &State,
        target: &Target,
        source: &Uuid,
        blob: Option<&Path>,
    ) -> SecurityResult<(Vec<SafeHistory>, Option<Id>)> {
        let (mut end, mut record_end, limit) = bounds;
        self.recheck().map_err(policy)?;

        state.validate_source(target, source)?;

        if end > self.entries.len() || limit > 1000 {
            return Err(ErrorCode::CursorExpired);
        }
        let mut result = Vec::with_capacity(limit);
        let deadline = std::time::Instant::now() + Duration::from_secs(14);
        let mut reads = 0usize;
        let mut bytes = 0usize;
        while end > 0 && result.len() < limit && reads < 64 {
            if std::time::Instant::now() >= deadline {
                return Err(ErrorCode::Timeout);
            }
            let index = end - 1;
            let entry = &self.entries[index];
            bytes = bytes
                .checked_add(entry.length)
                .ok_or(ErrorCode::ResourceLimit)?;
            if bytes > 16 * 1024 * 1024 {
                return Err(ErrorCode::ResourceLimit);
            }
            reads += 1;
            let upto = if record_end == 0 {
                entry.records
            } else {
                record_end
            };

            if upto > entry.records {
                return Err(ErrorCode::CursorExpired);
            }
            let start = upto.saturating_sub(limit - result.len());

            let value = self.entry(index)?;

            result.extend(project_entry_range(
                state,
                target,
                &value,
                RecordSource::Persisted,
                entry.order,
                blob,
                start..upto,
            )?);
            if start > 0 {
                record_end = start;
                break;
            }
            end -= 1;
            record_end = 0;
        }
        result.sort_by_key(|r| r.record().order);
        self.recheck().map_err(policy)?;

        state.validate_source(target, source)?;
        let next = (end > 0).then(|| Id(format!("omp-file:{end}:{record_end}")));
        Ok((result, next))
    }
}

/// Structured native credential/signature/encrypted blocks never enter display/cache/SQLite.
pub fn redacted(value: &Value) -> Value {
    if matches!(
        value.get("t").and_then(Value::as_str),
        Some("ui-request" | "ui-response")
    ) || matches!(
        value.get("type").and_then(Value::as_str),
        Some("redactedThinking" | "encrypted")
    ) {
        return Value::String("[Private native payload omitted]".into());
    }
    match value {
        Value::Object(map) => {
            let mut safe = serde_json::Map::new();
            for (k, v) in map {
                let normalized = k.to_ascii_lowercase().replace(['_', '-'], "");
                if matches!(
                    normalized.as_str(),
                    "token"
                        | "writetoken"
                        | "apikey"
                        | "authorization"
                        | "password"
                        | "secret"
                        | "signature"
                        | "thinkingsignature"
                        | "encryptedcontent"
                        | "providerpromptcachekey"
                        | "accesskey"
                        | "privatekey"
                        | "credentials"
                        | "pendingrequests"
                        | "uirequest"
                        | "rawrequest"
                        | "grants"
                ) {
                    continue;
                }
                if matches!(
                    v.get("type").and_then(Value::as_str),
                    Some("redactedThinking" | "encrypted")
                ) {
                    continue;
                }
                safe.insert(k.clone(), redacted(v));
            }
            Value::Object(safe)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .filter(|v| {
                    !matches!(
                        v.get("type").and_then(Value::as_str),
                        Some("redactedThinking" | "encrypted")
                    )
                })
                .map(redacted)
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn display(state: &State, target: &Target, mime: Mime, value: String) -> SecurityResult<Content> {
    state.content(target, mime, value, pairing::now_ms())
}
fn source_elision(text: &str) -> bool {
    text.contains("[Session persistence truncated large content]")
        || text.split("…[").skip(1).any(|tail| {
            let Some((marker, _)) = tail.split_once(']') else {
                return false;
            };
            matches!(
                marker,
                "deeper levels elided for collab session"
                    | "cyclic reference elided for collab session"
            ) || marker
                .strip_suffix(" chars elided for collab session")
                .or_else(|| marker.strip_suffix(" items elided for collab session"))
                .is_some_and(|count| {
                    count.starts_with(|c: char| matches!(c, '1'..='9'))
                        && count.bytes().all(|b| b.is_ascii_digit())
                })
        })
}
fn value_source_elision(value: &Value) -> bool {
    match value {
        Value::String(text) => source_elision(text),
        Value::Array(values) => values.iter().any(value_source_elision),
        Value::Object(values) => values.values().any(value_source_elision),
        _ => false,
    }
}
fn content_text(value: &Value, blob: Option<&Path>) -> io::Result<(String, bool)> {
    if let Some(s) = value.as_str() {
        return Ok((s.to_owned(), source_elision(s)));
    }
    let mut text = String::new();
    let mut truncated = false;
    if let Some(blocks) = value.as_array() {
        for block in blocks {
            if let Some(s) = block.as_str().filter(|s| source_elision(s)) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(s);
                truncated = true;
                continue;
            }
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(s) = block.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(s);
                        truncated |= source_elision(s);
                    }
                }
                Some("thinking") => {
                    if let Some(s) = block.get("thinking").and_then(Value::as_str) {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(s);
                        truncated |= source_elision(s);
                    }
                }
                Some("image") => {
                    let data = block.get("data").and_then(Value::as_str).unwrap_or("");
                    let verified = if let (Some(root), Some(hash)) =
                        (blob, data.strip_prefix("blob:sha256:"))
                    {
                        if hash.len() == 64
                            && hash
                                .bytes()
                                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                        {
                            let mut bytes = Vec::new();
                            let path = root.join(hash);
                            match private_file(&path, &[root.to_owned()]) {
                                Ok(file) => {
                                    file.take(CONTENT_MAX + 1).read_to_end(&mut bytes)?;
                                    bytes.len() <= CONTENT_MAX as usize
                                        && hex(&sha256(&bytes)) == hash
                                }
                                Err(_) => false,
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(if verified {
                        "[Verified native image; image display unsupported]"
                    } else {
                        "[Native image unavailable]"
                    });
                }
                Some("redactedThinking") => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str("[Private native thinking omitted]");
                }
                Some("toolCall") => {}
                _ => {
                    truncated = true;
                }
            }
        }
    }
    Ok((text, truncated))
}
fn native_timestamp(value: &Value) -> Option<Uint> {
    value
        .as_u64()
        .filter(|n| *n <= UINT_MAX)
        .map(Uint)
        .or_else(|| {
            let parsed = chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok()?;
            u64::try_from(parsed.timestamp_millis())
                .ok()
                .filter(|n| *n <= UINT_MAX)
                .map(Uint)
        })
}
fn projected_count(entry: &Value) -> usize {
    1 + entry
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .map_or(0, |blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("toolCall"))
                .count()
        })
}
pub fn project_entry(
    state: &State,
    target: &Target,
    entry: &Value,
    source: RecordSource,
    order: u64,
    blob: Option<&Path>,
) -> SecurityResult<Vec<SafeHistory>> {
    let total = projected_count(entry);
    project_entry_range(
        state,
        target,
        entry,
        source,
        order,
        blob,
        total.saturating_sub(1000)..total,
    )
}
fn project_entry_range(
    state: &State,
    target: &Target,
    entry: &Value,
    source: RecordSource,
    order: u64,
    blob: Option<&Path>,
    range: std::ops::Range<usize>,
) -> SecurityResult<Vec<SafeHistory>> {
    let start = range.start;
    let limit = range.end.saturating_sub(start);
    let id = Id::new(text(entry, "id").map_err(policy)?).map_err(policy)?;
    let parent = entry
        .get("parentId")
        .and_then(Value::as_str)
        .map(|s| Id::new(s.to_owned()))
        .transpose()
        .map_err(policy)?;
    let kind = entry.get("type").and_then(Value::as_str).unwrap_or("");
    let mut record = HistoryRecord {
        id: id.clone(),
        parent_id: parent.clone(),
        turn_id: None,
        order: Uint(order),
        timestamp: entry.get("timestamp").and_then(native_timestamp),
        role: Role::Unknown,
        kind: RecordKind::Unsupported,
        text: None,
        tool: None,
        original: None,
        source,
        complete: true,
        source_truncated: false,
    };
    let mut tools = Vec::new();
    match kind {
        "message" => {
            let message = entry.get("message").ok_or(ErrorCode::InvalidRequest)?;
            let role = message.get("role").and_then(Value::as_str).unwrap_or("");
            record.role = match role {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "developer" => Role::System,
                "toolResult" => Role::Tool,
                _ => Role::Unknown,
            };
            record.kind = if role == "toolResult" {
                RecordKind::Tool
            } else {
                RecordKind::Message
            };
            if start == 0 {
                let (body, truncated) =
                    content_text(message.get("content").unwrap_or(&Value::Null), blob)
                        .map_err(policy)?;
                record.source_truncated = truncated;
                if !body.is_empty() {
                    record.text = Some(display(state, target, Mime::TextPlain, body)?);
                }
            }
            if role == "toolResult" {
                record.tool = Some(Tool {
                    name: text(message, "toolName").map_err(policy)?,
                    call_id: message
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .map(|s| Id(s.to_owned())),
                    status: if message.get("isError").and_then(Value::as_bool) == Some(true) {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    },
                    input: None,
                    output: record.text.clone(),
                });
            }
            if let Some(blocks) = message.get("content").and_then(Value::as_array) {
                for (index, block) in blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("toolCall"))
                    .enumerate()
                {
                    let position = index + 1;
                    if position < start || position >= start.saturating_add(limit) {
                        continue;
                    }
                    let call = Id::new(text(block, "id").map_err(policy)?).map_err(policy)?;
                    let mut tool_record = record.clone();
                    tool_record.id = Id(format!("omp-tool:{}", call.0));
                    tool_record.order = Uint(order + position as u64);
                    tool_record.parent_id = Some(id.clone());
                    tool_record.role = Role::Tool;
                    tool_record.kind = RecordKind::Tool;
                    tool_record.text = None;
                    tool_record.original = None;
                    let input = block
                        .get("arguments")
                        .map(|v| {
                            let safe = redacted(v);
                            tool_record.source_truncated |= value_source_elision(&safe);
                            display(state, target, Mime::ApplicationJson, safe.to_string())
                        })
                        .transpose()?;
                    tool_record.tool = Some(Tool {
                        name: text(block, "name").map_err(policy)?,
                        call_id: Some(call),
                        status: ToolStatus::Unknown,
                        input,
                        output: None,
                    });
                    tools.push(SafeHistory::from_redacted_display(tool_record)?);
                }
            }
        }
        "custom_message" => {
            record.kind = RecordKind::Message;
            record.role =
                if entry.get("customType").and_then(Value::as_str) == Some("collab-prompt") {
                    Role::User
                } else {
                    Role::System
                };
            let (body, truncated) =
                content_text(entry.get("content").unwrap_or(&Value::Null), blob).map_err(policy)?;
            record.text = Some(display(state, target, Mime::TextPlain, body)?);
            record.source_truncated = truncated
                || entry.get("customType").and_then(Value::as_str)
                    == Some("collab-entry-too-large");
        }
        "compaction" | "branch_summary" => {
            record.role = Role::System;
            record.kind = RecordKind::Notice;
            record.text = entry
                .get("summary")
                .and_then(Value::as_str)
                .map(|s| display(state, target, Mime::TextPlain, s.to_owned()))
                .transpose()?;
        }
        "reset_boundary"
        | "title_change"
        | "model_change"
        | "thinking_level_change"
        | "service_tier_change" => {
            record.role = Role::System;
            record.kind = RecordKind::Notice;
        }
        _ => {}
    }
    let mut records = Vec::with_capacity(tools.len() + usize::from(start == 0 && limit > 0));
    if start == 0 && limit > 0 {
        let safe = redacted(entry);
        record.original = Some(display(
            state,
            target,
            Mime::ApplicationJson,
            safe.to_string(),
        )?);
        records.push(SafeHistory::from_redacted_display(record)?);
    }
    records.extend(tools);
    Ok(records)
}

pub fn project_form(
    state: &State,
    target: &Target,
    request: &Value,
) -> SecurityResult<PendingRequest> {
    let title = text(request, "title").map_err(policy)?;
    let projected = (|| -> SecurityResult<Presentation> {
        if request.as_object().is_none_or(|o| {
            o.keys().any(|k| {
                !matches!(
                    k.as_str(),
                    "reqId"
                        | "kind"
                        | "title"
                        | "options"
                        | "initialIndex"
                        | "selectionMarker"
                        | "checkedIndices"
                        | "markableCount"
                        | "helpText"
                        | "prefill"
                )
            })
        }) {
            return Err(ErrorCode::InvalidAnswer);
        }
        match request.get("kind").and_then(Value::as_str) {
            Some("select") => {
                let options = request
                    .get("options")
                    .and_then(Value::as_array)
                    .ok_or(ErrorCode::InvalidAnswer)?;
                if options.len() > 256 {
                    return Err(ErrorCode::ResourceLimit);
                }
                let mut offered = Vec::with_capacity(options.len());
                for option in options {
                    let label = option
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| {
                            option
                                .get("label")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .ok_or(ErrorCode::InvalidAnswer)?;
                    offered.push(SelectOption {
                        value: label.clone(),
                        label: Label::new(label).map_err(policy)?,
                        description: option
                            .get("description")
                            .map(|v| {
                                v.as_str().ok_or(ErrorCode::InvalidAnswer).and_then(|s| {
                                    display(state, target, Mime::TextPlain, s.to_owned())
                                })
                            })
                            .transpose()?,
                    });
                }
                let marker = match request.get("selectionMarker").and_then(Value::as_str) {
                    None | Some("radio") => SelectionMarker::Radio,
                    Some("checkbox") => SelectionMarker::Checkbox,
                    _ => return Err(ErrorCode::InvalidAnswer),
                };
                let selected = request
                    .get("initialIndex")
                    .map(|_| uint(request, "initialIndex").map(Uint).map_err(policy))
                    .transpose()?;
                let checked = match request.get("checkedIndices") {
                    None => Vec::new(),
                    Some(value) => value
                        .as_array()
                        .ok_or(ErrorCode::InvalidAnswer)?
                        .iter()
                        .map(|v| {
                            v.as_u64()
                                .filter(|v| *v <= UINT_MAX)
                                .map(Uint)
                                .ok_or(ErrorCode::InvalidAnswer)
                        })
                        .collect::<SecurityResult<Vec<_>>>()?,
                };
                let form = Presentation::OmpSelect(PresentationOmpSelect {
                    title: display(state, target, Mime::TextPlain, title.clone())?,
                    options: offered,
                    selection_marker: marker,
                    help_text: request
                        .get("helpText")
                        .map(|v| {
                            v.as_str()
                                .ok_or(ErrorCode::InvalidAnswer)
                                .and_then(|s| display(state, target, Mime::TextPlain, s.to_owned()))
                        })
                        .transpose()?,
                    markable_count: request
                        .get("markableCount")
                        .map(|_| uint(request, "markableCount").map(Uint).map_err(policy))
                        .transpose()?,
                    selected,
                    checked,
                });
                form.validate().map_err(policy)?;
                Ok(form)
            }
            Some("editor") => Ok(Presentation::OmpEditor(PresentationOmpEditor {
                title: display(state, target, Mime::TextPlain, title.clone())?,
                prefill: display(
                    state,
                    target,
                    Mime::TextPlain,
                    request
                        .get("prefill")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                )?,
            })),
            _ => Err(ErrorCode::InvalidAnswer),
        }
    })();
    let presentation = match projected {
        Ok(presentation) => presentation,
        Err(_) => Presentation::DesktopOnly(PresentationDesktopOnly {
            title: display(state, target, Mime::TextPlain, title)?,
            reason: "Stock native dialog cannot be represented safely; answer on desktop".into(),
        }),
    };
    let kind = match &presentation {
        Presentation::OmpSelect(_) => PresentationKind::OmpSelect,
        Presentation::OmpEditor(_) => PresentationKind::OmpEditor,
        _ => PresentationKind::DesktopOnly,
    };
    let pending = PendingRequest {
        request_key: Uuid::random(),
        provider: Provider::Omp,
        kind,
        presentation,
    };
    pending.validate().map_err(policy)?;
    Ok(pending)
}

/// Counting entries never substitutes for the authoritative FINAL boundary.
pub struct SnapshotBoundary {
    expected: usize,
    seen: usize,
    finalized: bool,
}
impl SnapshotBoundary {
    pub fn new(welcome: &Value, session: &str) -> io::Result<Self> {
        if uint(welcome, "proto")? != 3
            || !matches!(welcome.get("readOnly"), None | Some(Value::Bool(false)))
            || welcome
                .get("header")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                != Some(session)
        {
            return Err(invalid());
        }
        let expected = usize::try_from(uint(welcome, "entryCount")?).map_err(|_| invalid())?;
        if expected > 131_072 {
            return Err(invalid());
        }
        Ok(Self {
            expected,
            seen: 0,
            finalized: false,
        })
    }
    pub fn chunk(&mut self, chunk: &Value) -> io::Result<bool> {
        if self.finalized {
            return Err(invalid());
        }
        let entries = chunk
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        self.seen = self.seen.checked_add(entries.len()).ok_or_else(invalid)?;
        if self.seen > self.expected {
            return Err(invalid());
        }
        let final_chunk = chunk
            .get("final")
            .and_then(Value::as_bool)
            .ok_or_else(invalid)?;
        if final_chunk {
            if self.seen != self.expected {
                return Err(invalid());
            }
            self.finalized = true;
        }
        Ok(self.finalized)
    }
}

type NativeSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type NativeWriter = futures_util::stream::SplitSink<NativeSocket, Message>;
struct Dialog {
    native: NativeRpcId,
    pending: PendingRequest,
}
impl Dialog {
    fn response(&self, answer: &Answer) -> SecurityResult<Value> {
        validate_answer(&self.pending.presentation, answer)
            .map_err(|_| ErrorCode::InvalidAnswer)?;
        let native = native_json::parse(
            serde_json::to_string(&self.native)
                .map_err(policy)?
                .as_bytes(),
            REGISTRY,
        )
        .map_err(policy)?;
        let value = match answer {
            Answer::OmpSelect(v) => v.value.clone(),
            Answer::OmpEditor(v) => v.text.as_ref().map(|s| s.0.clone()),
            _ => return Err(ErrorCode::InvalidAnswer),
        };
        let mut payload = serde_json::Map::new();
        payload.insert("t".into(), Value::String("ui-response".into()));
        payload.insert("reqId".into(), native);
        if let Some(value) = value {
            payload.insert("value".into(), Value::String(value));
        }
        Ok(Value::Object(payload))
    }
}
struct Connection {
    key: StableSessionKey,
    owner: ProcessEvidence,
    source: Source,
    target: Target,
    epoch: Uuid,
    link: Link,
    writer: tokio::sync::Mutex<NativeWriter>,
    fd: OwnedFd,
    dialogs: Mutex<HashMap<Uuid, Dialog>>,
    history: Mutex<HistoryFile>,
    order: AtomicUsize,
    room_identity: String,
    registry: RegistryEntry,
    registry_snapshot: RegistrySnapshot,
    desktop_attention: Uuid,
    attention_open: AtomicBool,
    controls:Mutex<crate::backends::ControlsObservation>,
}
impl Connection {
    fn validate_native(&self, state: &State) -> SecurityResult<()> {
        state.validate_source(&self.target, &self.epoch)?;
        self.source
            .recheck_identity()
            .map_err(|_| ErrorCode::StaleSession)?;
        let current =
            herdr::process_evidence(self.owner.pid).map_err(|_| ErrorCode::StaleSession)?;
        if current != self.owner {
            return Err(ErrorCode::StaleSession);
        }
        self.history
            .lock()
            .recheck()
            .map_err(|_| ErrorCode::StaleSession)
    }
    fn validate(&self, state: &State, topology: &Topology) -> SecurityResult<()> {
        self.validate_checked(state, topology)
            .map_err(|(_, code)| code)
    }
    fn validate_checked(
        &self,
        state: &State,
        topology: &Topology,
    ) -> Result<(), (&'static str, ErrorCode)> {
        self.validate_native(state)
            .map_err(|code| ("native_identity", code))?;
        let current = topology
            .source_readiness(&self.key.server_id)
            .map_err(|code| ("source_lookup", code))?
            .ok_or(("source_pending", ErrorCode::NotReady))?;
        let report = current
            .snapshot
            .agents
            .iter()
            .find(|agent| agent.terminal_id == self.key.terminal_id)
            .map(|agent| (&agent.agent, &agent.agent_session));
        let captured = self
            .source
            .snapshot
            .agents
            .iter()
            .find(|agent| agent.terminal_id == self.key.terminal_id)
            .map(|agent| (&agent.agent, &agent.agent_session));
        if current.incarnation != self.key.server_incarnation
            || current.evidence != self.source.evidence
            || current.socket_path != self.source.socket_path
            || report != captured
            || !current
                .snapshot
                .panes
                .iter()
                .any(|pane| pane.terminal_id == self.key.terminal_id)
        {
            return Err(("source_identity", ErrorCode::StaleSession));
        }
        if current
            .recheck_readiness()
            .map_err(|error| ("source_recheck_io", policy(error)))?
        {
            state
                .observation_ready(&self.target, &self.epoch)
                .map_err(|code| ("observation_pending", code))
        } else {
            Err(("source_pending", ErrorCode::NotReady))
        }
    }
    /// Continue already-authenticated reads during a dirty topology revision; mutations still
    /// require validate. The native epoch/owner/file/root/socket checks above never relax.
    fn validate_read(&self, state: &State, topology: &Topology) -> SecurityResult<()> {
        match self.validate(state, topology) {
            Err(ErrorCode::NotReady) => Ok(()),
            result => result,
        }
    }
    fn fd(&self) -> SecurityResult<OwnedFd> {
        self.fd.try_clone().map_err(policy)
    }
    fn shutdown(&self) {
        // SAFETY: shutdown acts on the live connected stream and invalidates all FD clones.
        unsafe {
            libc::shutdown(self.fd.as_raw_fd(), libc::SHUT_RDWR);
        }
    }
}
struct OriginalMutation {
    request:Request,connection:Option<Arc<Connection>>,creation:Option<Arc<herdr::CreationSelection>>,
    controls:Option<Arc<crate::backends::ControlsPublication>>,
}
pub struct Runtime {
    skills:Arc<skills::Manager>,
    topology: Arc<Topology>,
    security: Arc<Security>,
    connections: Mutex<HashMap<Uuid, Arc<Connection>>>,
    jobs: Mutex<JoinSet<()>>,
    relay_ready: watch::Sender<Option<u16>>,
    owned_relay: Mutex<Option<Relay>>,
    producer_capture_authorized: bool,
    failed_rooms: Mutex<HashMap<(Uuid, Id), String>>,
    stopping: AtomicBool,
    first_guard: Option<OmpCreationReporter>,
    #[cfg(test)]
    core_wait_entered: Mutex<Option<mpsc::Sender<()>>>,
}
impl Runtime {
    pub fn backend(&self) -> Backend {
        let ready = self
            .connections
            .lock()
            .values()
            .any(|c| c.validate(&self.security.state, &self.topology).is_ok());
        let reason = self
            .topology
            .config
            .backend_reason(Provider::Omp)
            .ok()
            .flatten();
        Backend {
            provider: Provider::Omp,
            version: "collaboration 3".into(),
            availability: if ready {
                Availability::Ready
            } else if reason.is_some() {
                Availability::Unavailable
            } else {
                Availability::SetupRequired
            },
            reason: if ready {
                Some("Same-process native control; busy prompts steer, Stop current work; stock select/editor only".into())
            } else {
                Some(reason.unwrap_or("Requires explicit desktop control collaboration and a proved registry/process/terminal/header/final-snapshot join").into())
            },
        }
    }
    pub fn new(
        topology: Arc<Topology>,
        security: Arc<Security>,
        owned_relay: Option<Relay>,
    ) -> Arc<Self> {
        Self::new_with_first_guard(topology, security, owned_relay, None)
    }
    pub(crate) fn new_with_first_guard(
        topology: Arc<Topology>,
        security: Arc<Security>,
        owned_relay: Option<Relay>,
        first_guard: Option<OmpCreationReporter>,
    ) -> Arc<Self> {
        if let Some(reporter) = &first_guard {
            topology.set_omp_offline_reporter(reporter.clone());
        }
        let producer_capture_authorized = owned_relay.is_some() && first_guard.is_some();
        let (relay_ready, _) = watch::channel(None);
        let skills=skills::Manager::new(&topology.config);
        Arc::new(Self {
            skills,
            topology,
            security,
            connections: Mutex::new(HashMap::new()),
            jobs: Mutex::new(JoinSet::new()),
            relay_ready,
            owned_relay: Mutex::new(owned_relay),
            producer_capture_authorized,
            stopping: AtomicBool::new(false),
            failed_rooms: Mutex::new(HashMap::new()),
            first_guard,
            #[cfg(test)]
            core_wait_entered: Mutex::new(None),
        })
    }
    fn follower_guard(reason: &'static str) -> &'static str {
        match reason {
            "Herdr native session report is absent" => "follower_native_report_absent",
            "Herdr native session report agent is not OMP"
            | "Herdr OMP native session report is not a path" => "follower_native_report_wrong",
            "OMP collaboration registry is not configured"
            | "No readable private stock collaboration publication; enable desktop control collaboration explicitly"
            | "Stock collaboration registry has no candidates" => "follower_registry_absent",
            "Stock collaboration registry snapshots are unavailable"
            | "Stock collaboration registry snapshots and terminal/owner joins are unavailable" => {
                "follower_registry_snapshot"
            }
            "Herdr process evidence is unavailable"
            | "Herdr terminal process evidence is unavailable"
            | "Herdr reporting namespace cannot be proved" => "follower_registry_read",
            "Registry native ID and exact kernel terminal/owner namespace ancestry join could not be proved" => {
                "follower_owner_join"
            }
            "Multiple stock OMP owners match this terminal; no control target selected" => {
                "follower_ambiguous"
            }
            "Exact optional native history path/header or namespace object views cannot be proved" => {
                "follower_history"
            }
            "Stock room authentication/protocol failed; this room will not reconnect until its native generation changes"
            | "Stock guest authentication unavailable"
            | "Stock guest authentication write failed"
            | "Stock writable welcome unavailable"
            | "Stock welcome protocol mismatch"
            | "Writable stock welcome/header identity mismatch" => "follower_auth",
            "Exact native pane/terminal/workspace report changed"
            | "Native failed-room evidence capacity reached"
            | "Desktop collaboration is view-only or relay disconnected; control is unavailable"
            | "Authenticated control link or registry generation changed"
            | "Verified stock relay connection timed out"
            | "Verified stock relay connection unavailable"
            | "Authenticated registry/welcome cwd changed"
            | "Registry changed before guest installation"
            | "Registry/process/topology identity changed before native installation"
            | "Actual connected stock WebSocket FD unavailable"
            | "Native identity/reducer installation unavailable"
            | "Stock guest snapshot, history, crypto or source identity became unavailable" => {
                "follower_source"
            }
            _ => "follower_other",
        }
    }
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) -> io::Result<()> {
        let skills_job=tokio::spawn(self.skills.clone().run(Arc::downgrade(&self),stop.clone()));
        let mut relay_job = None;
        if diagnostic_overlay_environment(&self.topology.config)?.is_some()
            && !self.producer_capture_authorized
        {
            return Err(invalid());
        }
        if self
            .topology
            .config
            .backend_reason(Provider::Omp)?
            .is_none()
        {
            let inherited = self.owned_relay.lock().take();
            let relay = match inherited {
                Some(relay) => Ok(relay),
                None => Relay::bind(RELAY_PORT).await,
            };
            match relay {
                Ok(relay) => {
                    let port = relay.port();
                    if overlay(&self.topology.config, port).is_ok() {
                        let rx = stop.clone();
                        relay_job = Some(tokio::spawn(relay.run(rx)));
                        self.relay_ready.send_replace(Some(port));
                        self.topology
                            .omp_availability(None)
                            .map_err(|_| invalid())?;
                    } else {
                        self.topology.omp_availability(Some("OMP creation unavailable: private local relay overlay cannot be proved; no settings were replaced")).map_err(|_|invalid())?;
                    }
                }
                Err(error) => {
                    let reason = if error.kind() == io::ErrorKind::AddrInUse {
                        "OMP creation unavailable: required loopback relay port 7466 is occupied; no foreign relay or fallback port is used"
                    } else {
                        "OMP creation unavailable: required companion-owned loopback relay bind failed"
                    };
                    self.topology
                        .omp_availability(Some(reason))
                        .map_err(|_| invalid())?;
                }
            }
        }
        let mut followers: HashMap<(Uuid, Id), tokio::task::JoinHandle<()>> = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        while !*stop.borrow() {
            tokio::select! {_=stop.changed()=>{},_=tick.tick()=>{
                if relay_job.as_ref().is_some_and(|job|job.is_finished()) { self.relay_ready.send_replace(None); let _=self.topology.omp_availability(Some("OMP creation unavailable: companion-owned loopback relay stopped")); }
                followers.retain(|_,job|!job.is_finished());
                self.refresh_skills_capabilities();
                {let mut jobs=self.jobs.lock();while jobs.try_join_next().is_some(){}}
                for source in self.topology.sources(){for agent in &source.snapshot.agents{
                    if agent.agent.as_deref()!=Some("omp"){continue;}
                    let key=(source.server_id.clone(),agent.terminal_id.clone());
                    if followers.contains_key(&key){continue;}
                    if followers.len()>=512{break;}
                    let owner=self.clone();let agent=agent.clone();let source=source.clone();let rx=stop.clone();
                    followers.insert(key,tokio::spawn(async move{
                        if let Err(reason)=owner.clone().follow(source.clone(),agent.clone(),rx).await{
                            let (kind,reason)=reason;
                            if let Some(reporter)=&owner.first_guard {
                                reporter.report(
                                    "follower",
                                    Self::follower_guard(reason),
                                    Some(ErrorCode::NotReady),
                                );
                            }
                            let _=owner.topology.unbound_reason(&source,&agent.terminal_id,Provider::Omp,kind,reason);
                        }
                    }));
                }}
            }}
        }
        self.stopping.store(true, Ordering::Release);
        let _=skills_job.await;
        self.relay_ready.send_replace(None);
        let _ = self.topology.omp_availability(Some(
            "OMP creation unavailable: foreground companion is stopping",
        ));
        for connection in self.connections.lock().values() {
            connection.shutdown();
        }
        for (_, job) in followers {
            let _ = job.await;
        }
        // Admission waits even when SQLite's queue is full; earlier commits register
        // their originals before the final JoinSet is taken. Later validation fails stopping.
        let barrier = match self.security.storage.admission().await {
            Ok(admission) => admission.call(|_| Ok(())).await,
            Err(error) => Err(error),
        };
        let mut jobs = {
            let mut owned = self.jobs.lock();
            std::mem::replace(&mut *owned, JoinSet::new())
        };
        while jobs.join_next().await.is_some() {}
        if let Some(job) = relay_job {
            job.await.map_err(|_| invalid())??;
        }
        barrier.map_err(|_| invalid())?;
        Ok(())
    }
    fn connection(&self, target: &Target) -> SecurityResult<Arc<Connection>> {
        self.connection_checked(target).map_err(|(_, code)| code)
    }
    fn connection_checked(
        &self,
        target: &Target,
    ) -> Result<Arc<Connection>, (&'static str, ErrorCode)> {
        let connection = self.connection_instance(target)?;
        connection.validate_checked(&self.security.state, &self.topology)?;
        Ok(connection)
    }
    fn connection_instance(
        &self,
        target: &Target,
    ) -> Result<Arc<Connection>, (&'static str, ErrorCode)> {
        let connection = self
            .connections
            .lock()
            .get(&target.session_id)
            .cloned()
            .ok_or(("connection_missing", ErrorCode::NotReady))?;
        if connection.target != *target {
            return Err(("connection_target", ErrorCode::StaleSession));
        }
        Ok(connection)
    }
    pub(crate) fn project_context(self:&Arc<Self>,target:&Target)->SecurityResult<crate::project_read::ProjectReadContext> {
        let connection=self.connection(target)?;
        let original=self.original_context(target)?;
        if original.epoch!=connection.epoch||!Arc::ptr_eq(&connection,&self.connection(target)?) {return Err(ErrorCode::StaleSession);}
        let retained=Arc::new((original,connection));
        let checked=retained.clone();
        let validate=Arc::new(move||checked.0.recheck());
        let checked=retained.clone();
        let publish=Arc::new(move|commit:&mut dyn FnMut()->SecurityResult<()>|checked.0.with_current(||commit()));
        self.topology.capture_project_context(crate::project_read::ProjectReadOrigin {source:ArtifactSource::Session{target:target.clone()},epoch:retained.0.epoch.clone(),
            physical:retained.0.source.clone(),validate,publish,captured:std::time::Instant::now()},retained.0.process.clone(),Path::new(&retained.1.registry_snapshot.cwd),false)
    }
    pub(crate) fn original_context(self:&Arc<Self>,target:&Target)->SecurityResult<crate::backends::OriginalContext> {
        let original=self.connection(target)?;let runtime=Arc::downgrade(self);let retained=original.clone();
        let validate=Arc::new(move||{
            let runtime=runtime.upgrade().ok_or(ErrorCode::NotReady)?;
            let current=runtime.connections.lock().get(&retained.target.session_id).cloned().ok_or(ErrorCode::StaleSession)?;
            if !Arc::ptr_eq(&current,&retained){return Err(ErrorCode::StaleSession)}
            retained.validate(&runtime.security.state,&runtime.topology)
        });
        let runtime=Arc::downgrade(self);let retained=original.clone();
        let publish=Arc::new(move|enqueue:&mut dyn FnMut()->SecurityResult<()>|{
            let runtime=runtime.upgrade().ok_or(ErrorCode::NotReady)?;
            let connections=runtime.connections.lock();
            if !connections.get(&retained.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&retained)){return Err(ErrorCode::StaleSession)}
            retained.validate(&runtime.security.state,&runtime.topology)?;
            enqueue()
        });
        crate::backends::OriginalContext::capture(Provider::Omp,original.target.clone(),original.epoch.clone(),
            original.source.clone(),original.owner.clone(),original.key.native_conversation_id.clone(),validate,publish)
    }
    pub(crate) fn attachment_context(self:&Arc<Self>,target:&Target)->SecurityResult<crate::attachments::ProviderContext> {
        let original=self.connection(target)?;let runtime=Arc::downgrade(self);let retained=original.clone();
        let validate=Arc::new(move||{let runtime=runtime.upgrade().ok_or(ErrorCode::NotReady)?;let current=runtime.connections.lock().get(&retained.target.session_id).cloned().ok_or(ErrorCode::NotReady)?;if !Arc::ptr_eq(&current,&retained){return Err(ErrorCode::StaleSession)}retained.validate(&runtime.security.state,&runtime.topology)});
        let runtime=Arc::downgrade(self);let retained=original.clone();
        let publish=Arc::new(move|enqueue:&mut dyn FnMut()->SecurityResult<()>|{
            let runtime=runtime.upgrade().ok_or(ErrorCode::NotReady)?;
            let connections=runtime.connections.lock();
            if !connections.get(&retained.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&retained)){return Err(ErrorCode::StaleSession)}
            retained.validate(&runtime.security.state,&runtime.topology)?;
            enqueue()
        });
        crate::attachments::ProviderContext::capture(&self.topology.config,original.target.clone(),original.epoch.clone(),original.source.clone(),original.owner.clone(),validate,publish)
    }
    pub fn validate(&self, request: &Request) -> SecurityResult<()> {
        self.validate_checked(request, None, false)
            .map_err(|(_, code)| code)
    }
    fn validate_checked(
        &self,
        request: &Request,
        original: Option<&Arc<Connection>>,
        allow_pending: bool,
    ) -> Result<(), (&'static str, ErrorCode)> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(("stopping", ErrorCode::NotReady));
        }
        if matches!(&request.body,RequestBody::PromptSubmit(params) if params.skill.is_some()) {return Err(("skill_provider",ErrorCode::NotReady))}
        self.security.state.validate_session_mutation_checked(
            request,
            if allow_pending {
                original.map(|c| &c.epoch)
            } else {
                None
            },
        )?;
        let target = match &request.body {
            RequestBody::SessionControlsApply(p)=>&p.target,
            RequestBody::PromptSubmit(p) => &p.target,
            RequestBody::TurnCancel(p) => &p.target,
            RequestBody::RequestRespond(p) => &p.target,
            _ => return Err(("method", ErrorCode::InvalidRequest)),
        };
        let current;
        let connection = if let Some(connection) = original {
            if connection.target != *target {
                return Err(("connection_target", ErrorCode::StaleSession));
            }
            {
                let connections = self.connections.lock();
                let mapped = connections
                    .get(&target.session_id)
                    .ok_or(("connection_missing", ErrorCode::NotReady))?;
                if !Arc::ptr_eq(mapped, connection) {
                    return Err(("connection_target", ErrorCode::StaleSession));
                }
            }
            match connection.validate_checked(&self.security.state, &self.topology) {
                Ok(()) => {}
                Err(("source_pending", ErrorCode::NotReady)) if allow_pending => {}
                Err(error) => return Err(error),
            }
            if !self
                .connections
                .lock()
                .get(&target.session_id)
                .is_some_and(|mapped| Arc::ptr_eq(mapped, connection))
            {
                return Err(("connection_target", ErrorCode::StaleSession));
            }
            connection.as_ref()
        } else {
            current = self.connection_checked(target)?;
            current.as_ref()
        };
        if let RequestBody::RequestRespond(p) = &request.body {
            let dialogs = connection.dialogs.lock();
            let dialog = dialogs
                .get(&p.request_key)
                .ok_or(("dialog_lookup", ErrorCode::AlreadyResolved))?;
            validate_answer(&dialog.pending.presentation, &p.answer)
                .map_err(|_| ("answer", ErrorCode::InvalidAnswer))?;
        }
        Ok(())
    }
    fn pending_core(
        &self,
        connection: &Connection,
        request: &Request,
    ) -> Result<bool, (&'static str, ErrorCode)> {
        // Reuse the canonical Core lock: the returned connectivity has already
        // fenced this epoch, install state, dirty revision, capability and request.
        let connectivity = self
            .security
            .state
            .validate_session_mutation_checked(request, Some(&connection.epoch))?;
        if let RequestBody::RequestRespond(p) = &request.body {
            let dialogs = connection.dialogs.lock();
            let dialog = dialogs
                .get(&p.request_key)
                .ok_or(("dialog_lookup", ErrorCode::AlreadyResolved))?;
            validate_answer(&dialog.pending.presentation, &p.answer)
                .map_err(|_| ("answer", ErrorCode::InvalidAnswer))?;
        }
        match connectivity {
            SessionConnectivity::Ready => Ok(false),
            SessionConnectivity::Synchronizing => Ok(true),
            SessionConnectivity::Offline => Err(("core_offline", ErrorCode::NotReady)),
            SessionConnectivity::SetupRequired => Err(("core_setup_required", ErrorCode::NotReady)),
            SessionConnectivity::Unavailable => Err(("core_unavailable", ErrorCode::NotReady)),
        }
    }
    async fn wait_connection(
        &self,
        auth: &Authorized,
        request: &Request,
        target: &Target,
        deadline: tokio::time::Instant,
        connection: Arc<Connection>,
    ) -> Result<Arc<Connection>, (&'static str, ErrorCode)> {
        // Admission and the committed writer share this one captured connection.
        if connection.target != *target {
            return Err(("connection_target", ErrorCode::StaleSession));
        }
        #[cfg(test)]
        let mut wait_observed = false;
        loop {
            if self.stopping.load(Ordering::Acquire) {
                return Err(("stopping", ErrorCode::NotReady));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(("source_pending", ErrorCode::Timeout));
            }
            let guard = tokio::time::timeout_at(deadline, self.security.native_guard(auth))
                .await
                .map_err(|_| ("source_pending", ErrorCode::Timeout))?
                .map_err(|code| ("none", code))?;
            if self.stopping.load(Ordering::Acquire) {
                return Err(("stopping", ErrorCode::NotReady));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(("source_pending", ErrorCode::Timeout));
            }
            {
                let current = self.connections.lock();
                let current = current
                    .get(&target.session_id)
                    .ok_or(("connection_missing", ErrorCode::NotReady))?;
                if !Arc::ptr_eq(current, &connection) {
                    return Err(("connection_target", ErrorCode::StaleSession));
                }
            }
            let source_pending =
                match connection.validate_checked(&self.security.state, &self.topology) {
                    Ok(()) => false,
                    Err(("source_pending", ErrorCode::NotReady)) => true,
                    Err(error) => return Err(error),
                };
            let core_pending = self.pending_core(&connection, request)?;
            if !source_pending && !core_pending {
                // The existing strict validator is the final authority, including dialog identity.
                match self.validate_checked(request, None, false) {
                    Ok(()) => {
                        if !self
                            .connections
                            .lock()
                            .get(&target.session_id)
                            .is_some_and(|current| Arc::ptr_eq(current, &connection))
                        {
                            return Err(("connection_target", ErrorCode::StaleSession));
                        }
                        if self.stopping.load(Ordering::Acquire) {
                            return Err(("stopping", ErrorCode::NotReady));
                        }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(("source_pending", ErrorCode::Timeout));
                        }
                        return Ok(connection);
                    }
                    Err(("source_pending", ErrorCode::NotReady)) => {}
                    Err(("core_sync", ErrorCode::NotReady)) => {
                        // The original Core read saw Sync. Its existing reread may already
                        // be Ready; either outcome needs fresh strict validation, not a
                        // stale rejection or permission to dispatch.
                        self.pending_core(&connection, request)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            // Revocation/shutdown must not wait behind an idle readiness poll.
            drop(guard);
            #[cfg(test)]
            if core_pending && !wait_observed {
                if let Some(entered) = self.core_wait_entered.lock().as_ref() {
                    entered.try_send(()).expect("owned Core wait entry");
                }
                wait_observed = true;
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(100)),
            )
            .await;
        }
    }
    async fn follow(
        self: Arc<Self>,
        source: Source,
        agent: Agent,
        mut stop: watch::Receiver<bool>,
    ) -> Result<(), (UnboundState, &'static str)> {
        let unbound = |reason| (UnboundState::SetupRequired, reason);
        let native = agent
            .agent_session
            .as_ref()
            .ok_or_else(|| unbound("Herdr native session report is absent"))?;
        if native.agent != "omp" {
            return Err(unbound("Herdr native session report agent is not OMP"));
        }
        if native.kind != "path" {
            return Err(unbound("Herdr OMP native session report is not a path"));
        }
        let path = PathBuf::from(&native.value);
        let roots = self.topology.config.omp_session_roots.clone();
        let dir = self
            .topology
            .config
            .omp_registry_dir
            .as_ref()
            .ok_or_else(|| unbound("OMP collaboration registry is not configured"))?;
        let entries=registry(dir).map_err(|_|unbound("No readable private stock collaboration publication; enable desktop control collaboration explicitly"))?;
        let client = herdr::Client::connect(&source.socket_path)
            .await
            .map_err(|_| unbound("Herdr process evidence is unavailable"))?;
        let info = client
            .process_info(&agent.pane_id)
            .await
            .map_err(|_| unbound("Herdr terminal process evidence is unavailable"))?;
        if info.pane_id != agent.pane_id
            || !source.snapshot.panes.iter().any(|pane| {
                pane.pane_id == agent.pane_id
                    && pane.terminal_id == agent.terminal_id
                    && pane.workspace_id == agent.workspace_id
                    && pane.tab_id == agent.tab_id
            })
        {
            return Err(unbound(
                "Exact native pane/terminal/workspace report changed",
            ));
        }
        let reporter = herdr::process_evidence(source.evidence.pid)
            .map_err(|_| unbound("Herdr reporting namespace cannot be proved"))?;
        let queries = futures_util::stream::iter(entries.into_iter().map(|entry| async move {
            let result = entry.snapshot().await;
            (entry, result)
        }))
        .buffer_unordered(8);
        tokio::pin!(queries);
        let mut matches = Vec::new();
        let mut snapshot_failed = false;
        let mut owner_join_failed = false;
        while let Some((entry, result)) = queries.next().await {
            match result {
                Ok((snapshot, owner)) => {
                    if terminal_owner(&reporter, &owner, &info).is_ok() {
                        matches.push((entry, snapshot, owner));
                    } else {
                        owner_join_failed = true;
                    }
                }
                Err(_) => snapshot_failed = true,
            }
        }
        if matches.len() > 1 {
            return Err((
                UnboundState::Ambiguous,
                "Multiple stock OMP owners match this terminal; no control target selected",
            ));
        }
        let (entry, snapshot, owner) = matches.pop().ok_or_else(|| {
            // Aggregate every rejected candidate, independent of completion order.
            unbound(match (snapshot_failed, owner_join_failed) {
                (false, false) => "Stock collaboration registry has no candidates",
                (true, false) => "Stock collaboration registry snapshots are unavailable",
                (false, true) => "Registry native ID and exact kernel terminal/owner namespace ancestry join could not be proved",
                (true, true) => "Stock collaboration registry snapshots and terminal/owner joins are unavailable",
            })
        })?;
        let session = snapshot.session.clone();
        let room_identity = format!(
            "{}:{}:{}:{}",
            snapshot.instance, snapshot.generation, owner.pid, owner.start_ticks
        );
        {
            let failed = self.failed_rooms.lock();
            let slot = (source.server_id.clone(), agent.terminal_id.clone());
            if failed.get(&slot) == Some(&room_identity) {
                return Err(unbound(
                    "Stock room authentication/protocol failed; this room will not reconnect until its native generation changes",
                ));
            }
            if failed.len() >= 512 && !failed.contains_key(&slot) {
                return Err(unbound("Native failed-room evidence capacity reached"));
            }
        }
        if !snapshot.control || !snapshot.relay_connected {
            return Err(unbound(
                "Desktop collaboration is view-only or relay disconnected; control is unavailable",
            ));
        }
        let history = HistoryFile::live(path, roots, owner.clone(), session.clone()).map_err(|_| {
            unbound("Exact optional native history path/header or namespace object views cannot be proved")
        })?;
        let link = entry
            .link(&snapshot, &owner)
            .await
            .map_err(|_| unbound("Authenticated control link or registry generation changed"))?;
        let cfg = WebSocketConfig::default()
            .max_message_size(Some(FRAME_BYTES))
            .max_frame_size(Some(FRAME_BYTES))
            .write_buffer_size(0)
            .max_write_buffer_size(QUEUE_BYTES);
        let (mut socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async_with_config(
                format!("{}?role=guest", link.ws_url),
                Some(cfg),
                true,
            ),
        )
        .await
        .map_err(|_| unbound("Verified stock relay connection timed out"))?
        .map_err(|_| unbound("Verified stock relay connection unavailable"))?;
        socket
            .send(Message::Binary(
                link.hello()
                    .map_err(|_| unbound("Stock guest authentication unavailable"))?
                    .into(),
            ))
            .await
            .map_err(|_| unbound("Stock guest authentication write failed"))?;
        let welcome = next_native(&mut socket, &link, Duration::from_secs(30))
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::PermissionDenied {
                    self.failed_rooms.lock().insert(
                        (source.server_id.clone(), agent.terminal_id.clone()),
                        room_identity.clone(),
                    );
                }
                unbound("Stock writable welcome unavailable")
            })?;
        if welcome.get("t").and_then(Value::as_str) != Some("welcome") {
            return Err(unbound("Stock welcome protocol mismatch"));
        }
        let mut boundary = SnapshotBoundary::new(&welcome, &session)
            .map_err(|_| unbound("Writable stock welcome/header identity mismatch"))?;
        if welcome
            .get("header")
            .and_then(|header| header.get("type"))
            .and_then(Value::as_str)
            != Some("session")
            || welcome
                .get("header")
                .and_then(|header| header.get("cwd"))
                .and_then(Value::as_str)
                != Some(snapshot.cwd.as_str())
        {
            return Err(unbound("Authenticated registry/welcome cwd changed"));
        }
        let (after, again) = entry
            .snapshot()
            .await
            .map_err(|_| unbound("Registry changed before guest installation"))?;
        if !after.same_binding(&snapshot)
            || again != owner
            || !after.control
            || !after.relay_connected
            || source.recheck().is_err()
        {
            return Err(unbound(
                "Registry/process/topology identity changed before native installation",
            ));
        }
        let mut state = initial_state(&source, &agent, &snapshot, &welcome, &history);
        state.summary.capabilities.artifacts=self.topology.project_reads_available(&owner,Path::new(&snapshot.cwd));
        state.summary.capabilities.worktrees_read=state.summary.capabilities.artifacts&&crate::project_read::git::available();
        state.summary.capabilities.changes_read=state.summary.capabilities.worktrees_read;
        state.summary.capabilities.terminal_create=self.topology.terminal_creation_available(&source,&state.summary.workspace_id);
        state.summary.capabilities.attachments=crate::attachments::Namespace::capture(&self.topology.config,&owner).is_ok();
        if let Some(title) = &history.title {
            state.summary.title = title.clone();
        }
        let fd = socket_fd(&socket)
            .map_err(|_| unbound("Actual connected stock WebSocket FD unavailable"))?;
        let key = StableSessionKey {
            host_id: self.security.host.clone(),
            server_id: source.server_id.clone(),
            server_incarnation: source.incarnation.clone(),
            terminal_id: agent.terminal_id.clone(),
            provider_owner_identity: Id(format!(
                "omp:{}:{}:{}:{}",
                snapshot.instance, owner.linux_boot_id.0, owner.pid, owner.start_ticks
            )),
            native_conversation_id: Id(session),
        };
        let (target, epoch) = self
            .topology
            .begin_native(key.clone(), owner.clone(), state, true)
            .await
            .map_err(|_| unbound("Native identity/reducer installation unavailable"))?;
        let (writer, mut incoming) = socket.split();
        let connection = Arc::new(Connection {
            key: key.clone(),
            owner: owner.clone(),
            source: source.clone(),
            target: target.clone(),
            epoch: epoch.clone(),
            link,
            writer: tokio::sync::Mutex::new(writer),
            fd,
            dialogs: Mutex::new(HashMap::new()),
            history: Mutex::new(history),
            order: AtomicUsize::new(0),
            room_identity,
            registry: entry,
            registry_snapshot: snapshot,
            desktop_attention: Uuid::random(),
            attention_open: AtomicBool::new(false),
            controls:Mutex::new(crate::backends::ControlsObservation::new(omp_effective_controls(welcome.get("state").unwrap_or(&Value::Null)))),
        });
        let install = self
            .clone()
            .install_and_read(
                connection.clone(),
                &mut incoming,
                &mut boundary,
                welcome,
                &mut stop,
            )
            .await;
        if let Err(code) = install.as_ref() {
            // This is the final original follower error before shutdown and
            // binding_offline replace the retained source UUID.
            if let Some(reporter) = &self.first_guard {
                reporter.offline_source(
                    &target,
                    &epoch,
                    &source,
                    &agent.terminal_id,
                    OmpOfflineFailure::new("follower", "follower_source", *code, "none"),
                );
            }
        }
        connection.shutdown();
        {
            let mut connections = self.connections.lock();
            if connections
                .get(&target.session_id)
                .is_some_and(|c| c.epoch == epoch)
            {
                connections.remove(&target.session_id);
            }
        }
        if install.is_err() {
            let _ = self.security.state.abort_install(&target, &epoch);
        }
        let retired_target=target.clone();let retired_epoch=epoch.clone();let retired_connection=connection.desktop_attention.clone();
        let retired=self.security.storage.call(move|store|{
            store.retire_image_source(&retired_target,&retired_epoch);
            store.omp_cycle_retire(&retired_target,&retired_epoch,&retired_connection)
        }).await;
        let _=self.security.state.binding_offline(&target,&epoch,"Stock collaboration/file/topology source disconnected or replaced; reopen after authoritative final snapshot");
        if retired.is_err(){return Err(unbound("Durable attention source retirement is unavailable"))}
        install.map_err(|_| {
            unbound("Stock guest snapshot, history, crypto or source identity became unavailable")
        })
    }
    async fn install_and_read(
        self: Arc<Self>,
        connection: Arc<Connection>,
        incoming: &mut futures_util::stream::SplitStream<NativeSocket>,
        boundary: &mut SnapshotBoundary,
        welcome: Value,
        stop: &mut watch::Receiver<bool>,
    ) -> SecurityResult<()> {
        let state = &self.security.state;
        let target = &connection.target;
        let epoch = &connection.epoch;
        let mut records: HashMap<Id, (HistoryRecord, HistoryOrigins)> = HashMap::new();
        let mut pending_updates = VecDeque::new();
        let mut pending_bytes = 0usize;
        loop {
            let frame = tokio::select! {_=stop.changed()=>return Err(ErrorCode::NotReady),v=next_split(incoming,&connection.link,Some(Duration::from_secs(30)))=>self.native_read_result(&connection,v)?};
            self.validate_reported(&connection)?;
            if frame.get("t").and_then(Value::as_str) == Some("snapshot-chunk") {
                let finalized = boundary.chunk(&frame).map_err(policy)?;
                for entry in frame
                    .get("entries")
                    .and_then(Value::as_array)
                    .ok_or(ErrorCode::InvalidRequest)?
                {
                    let order = connection
                        .order
                        .fetch_add(projected_count(entry), Ordering::Relaxed)
                        as u64;
                    for record in project_entry(
                        state,
                        target,
                        entry,
                        RecordSource::Live,
                        order,
                        self.topology.config.omp_blob_root.as_deref(),
                    )? {
                        merge_projected(&mut records, record);
                    }
                    if records.len() > 2000 {
                        let cutoff = connection
                            .order
                            .load(Ordering::Relaxed)
                            .saturating_sub(2000) as u64;
                        records.retain(|_, (r, _)| r.order.0 >= cutoff);
                    }
                }
                if finalized {
                    break;
                }
            } else {
                pending_bytes += frame.to_string().len();
                if pending_updates.len() >= QUEUE_FRAMES || pending_bytes > QUEUE_BYTES {
                    return Err(ErrorCode::ResourceLimit);
                }
                pending_updates.push_back(frame);
            }
        }
        let (native, owner) = connection.registry.snapshot().await.map_err(policy)?;
        if !native.same_binding(&connection.registry_snapshot)
            || owner != connection.owner
            || !native.control
            || !native.relay_connected
        {
            return Err(ErrorCode::StaleSession);
        }
        connection.validate(state, &self.topology)?;
        let snapshot = {
            let mut history = connection.history.lock();
            history.refresh().map_err(policy)?;
            let (file_records, native_older) = history.native_page(
                (history.len(), 0, 1000),
                state,
                target,
                epoch,
                self.topology.config.omp_blob_root.as_deref(),
            )?;
            for record in file_records {
                merge_projected(&mut records, record);
            }
            connection.order.store(
                history
                    .record_count()
                    .max(connection.order.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
            let mut display = state.session_state(target, epoch)?;
            apply_native_state(&mut display, welcome.get("state").unwrap_or(&Value::Null));
            display.summary.connectivity = SessionConnectivity::Ready;
            display.summary.history_coverage.source = if history.identity.is_some() {
                HistorySource::PersistedAndLive
            } else {
                HistorySource::LiveOnly
            };
            display.summary.history_coverage.older_available =
                native_older.is_some() || (history.identity.is_some() && boundary.expected > 1000);
            display.summary.history_coverage.source_truncated =
                records.values().any(|(r, _)| r.source_truncated);
            history.published = history
                .identity
                .map(|_| (history.record_count(), history.title.clone()));
            ProviderSnapshot {
                state: display,
                pending: Vec::new(),
                history: records
                    .into_values()
                    .map(|(r, _)| SafeHistory::from_redacted_display(r))
                    .collect::<SecurityResult<Vec<_>>>()?,
                older_available: native_older.is_some()
                    || (history.identity.is_some() && boundary.expected > 1000),
                native_older,
            }
        };
        for frame in pending_updates {
            self.apply_frame(&connection, frame)?;
        }
        self.topology
            .finish_native(&connection.key, &connection.owner, epoch, snapshot)
            .await?;
        self.connections
            .lock()
            .insert(target.session_id.clone(), connection.clone());
        self.refresh_skills_capabilities();
        let status=state.session_state(target,epoch)?.summary.status;
        let installed_target=target.clone();let installed_epoch=epoch.clone();let installed_connection=connection.desktop_attention.clone();
        self.attention_commit(&connection,move|store|store.omp_cycle_install(&installed_target,&installed_epoch,&installed_connection,status,crate::pairing::now_ms())).await?;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut observation: Option<RegistryObservation> = None;
        loop {
            tokio::select! {
                _=stop.changed()=>return Ok(()),
                _=tick.tick(),if observation.is_none()=>{
                    self.validate_read_reported(&connection)?;
                    let registry=connection.registry.clone();
                    observation=Some(Box::pin(async move{registry.snapshot_checked().await}));
                }
                observed=async {
                    observation
                        .as_mut()
                        .expect("observation branch is guarded")
                        .as_mut()
                        .await
                },if observation.is_some()=>{
                    observation=None;
                    let(native,owner)=match observed {
                        Ok(value)=>value,
                        Err(failure) if failure.is_timeout()=>{
                            state.observation_pending(target,epoch)?;
                            continue;
                        }
                        Err(failure)=>{
                            let error=failure.into_io();
                            let code=policy(&error);
                            self.report_offline(&connection,"follower_registry",code,io_kind(&error));
                            return Err(code);
                        },
                    };
                    if !native.same_binding(&connection.registry_snapshot) {
                        self.report_offline(&connection, "follower_registry_snapshot", ErrorCode::StaleSession, "none");
                        return Err(ErrorCode::StaleSession);
                    }
                    if owner != connection.owner {
                        self.report_offline(&connection, "follower_registry_owner", ErrorCode::StaleSession, "none");
                        return Err(ErrorCode::StaleSession);
                    }
                    if !native.control {
                        self.report_offline(&connection, "follower_registry_control", ErrorCode::StaleSession, "none");
                        return Err(ErrorCode::StaleSession);
                    }
                    if !native.relay_connected {
                        self.report_offline(&connection, "follower_registry_relay", ErrorCode::StaleSession, "none");
                        return Err(ErrorCode::StaleSession);
                    }
                    // The asynchronous registry result cannot clear the mutation gate until
                    // the original connection's physical/source checks are fresh again.
                    self.validate_read_reported(&connection)?;
                    state.observation_revalidated(target,epoch)?;
                    let mirrored=!connection.dialogs.lock().is_empty();
                    let mut display=state.session_state(target,epoch)?;
                    display.summary.status=if native.input_required{SessionStatus::Attention}else{match native.busy{Some(true)=>SessionStatus::Working,Some(false)=>SessionStatus::Idle,None=>SessionStatus::Unknown}};
                    let previous=state.session_state(target,epoch)?.summary.status;
                    let change=crate::storage::AttentionTransition{target:target.clone(),correlation:Id("registry-checkpoint".into()),previous,current:display.summary.status,snapshot:true,new_actionable:false};
                    self.attention_commit(&connection,move|store|store.attention_transition(&change,crate::pairing::now_ms())).await?;
                    state.update_from_source(target,epoch,SessionUpdate::State(display))?;
                    if native.input_required&&!mirrored {
                        if !connection.attention_open.swap(true,Ordering::Relaxed){state.update_from_source(target,epoch,SessionUpdate::Open(PendingRequest{request_key:connection.desktop_attention.clone(),provider:Provider::Omp,kind:PresentationKind::DesktopOnly,presentation:Presentation::DesktopOnly(PresentationDesktopOnly{title:Content::Text(ContentText{text:InlineText("Desktop attention".into())}),reason:"No mirrored stock select/editor is currently offered; answer this native dialog on desktop".into()})}))?;}
                    }else if connection.attention_open.swap(false,Ordering::Relaxed){state.update_from_source(target,epoch,SessionUpdate::Close(connection.desktop_attention.clone(),RequestClosedReason::DesktopOnly))?;}
                    self.refresh_history(&connection, true)?;
                }
                v=next_split(incoming,&connection.link,None)=>{
                    let frame = self.native_read_result(&connection,v)?;
                    self.attention_before_frame(&connection,&frame).await?;
                    self.apply_frame(&connection,frame)?;
                }
            }
        }
    }
    fn refresh_history(&self, connection: &Connection, follower: bool) -> SecurityResult<()> {
        let state = &self.security.state;
        if follower {
            self.validate_read_reported(connection)?;
        } else {
            connection.validate_read(state, &self.topology)?;
        }
        let mut history = connection.history.lock();
        history.refresh().map_err(|error| {
            let code = policy(&error);
            if follower {
                self.report_offline(connection, "follower_history", code, io_kind(&error));
            }
            code
        })?;
        let checkpoint = (history.record_count(), history.title.clone());
        if history.identity.is_none() || history.published.as_ref() == Some(&checkpoint) {
            return Ok(());
        }
        let before = history.published.as_ref().map_or(0, |p| p.0);
        let (records, next) = history.native_page(
            (
                history.len(),
                0,
                history.record_count().saturating_sub(before).min(1000),
            ),
            state,
            &connection.target,
            &connection.epoch,
            self.topology.config.omp_blob_root.as_deref(),
        )?;
        connection
            .order
            .fetch_max(history.record_count(), Ordering::Relaxed);

        state.history_from_source(&connection.target, &connection.epoch, records)?;

        let mut display = state.session_state(&connection.target, &connection.epoch)?;
        display.summary.history_coverage.source = HistorySource::PersistedAndLive;
        display.summary.history_coverage.older_available |=
            next.is_some() || history.record_count() > 1000;
        if let Some(title) = &history.title {
            display.summary.title = title.clone();
        }
        state.update_from_source(
            &connection.target,
            &connection.epoch,
            SessionUpdate::State(display),
        )?;
        history.published = Some(checkpoint);
        Ok(())
    }
    fn refresh_history_if_ready(&self, target: &Target) -> SecurityResult<()> {
        let connection = self.connections.lock().get(&target.session_id).cloned();
        if let Some(connection) = connection
            && connection.target == *target
        {
            let state = &self.security.state;

            let current_epoch = state.source_epoch(target)?;
            if current_epoch == connection.epoch
                && state
                    .session_state(target, &current_epoch)?
                    .summary
                    .connectivity
                    == SessionConnectivity::Ready
            {
                self.refresh_history(&connection, false)?;
            }
        }
        Ok(())
    }
    fn report_offline(
        &self,
        connection: &Connection,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) {
        if let Some(reporter) = &self.first_guard {
            reporter.offline_source(
                &connection.target,
                &connection.epoch,
                &connection.source,
                &connection.key.terminal_id,
                OmpOfflineFailure::new("follower", guard, code, io),
            );
        }
    }
    fn validate_reported(&self, connection: &Connection) -> SecurityResult<()> {
        connection
            .validate_checked(&self.security.state, &self.topology)
            .inspect_err(|(guard, code)| self.report_offline(connection, guard, *code, "none"))
            .map_err(|(_, code)| code)
    }
    fn validate_read_reported(&self, connection: &Connection) -> SecurityResult<()> {
        match connection.validate_checked(&self.security.state, &self.topology) {
            Ok(()) | Err((_, ErrorCode::NotReady)) => Ok(()),
            Err((guard, code)) => {
                self.report_offline(connection, guard, code, "none");
                Err(code)
            }
        }
    }
    fn native_read_result(
        &self,
        connection: &Connection,
        result: io::Result<Value>,
    ) -> SecurityResult<Value> {
        result.map_err(|error| {
            let code = policy(&error);
            self.report_offline(connection, "follower_native_read", code, io_kind(&error));
            if error.kind() == io::ErrorKind::PermissionDenied {
                self.failed_rooms.lock().insert(
                    (
                        connection.key.server_id.clone(),
                        connection.key.terminal_id.clone(),
                    ),
                    connection.room_identity.clone(),
                );
            }
            code
        })
    }

    async fn attention_commit<T:Send+'static>(self:&Arc<Self>,c:&Arc<Connection>,commit:impl FnOnce(&mut crate::storage::Storage)->crate::storage::StoreResult<T>+Send+'static)->SecurityResult<T>{
        let runtime=self.clone();let original=c.clone();
        self.security.storage.call(move|store|{
            original.validate_read(&runtime.security.state,&runtime.topology).map_err(crate::storage::StorageError::Policy)?;
            let connections=runtime.connections.lock();
            if !connections.get(&original.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&original)){return Err(crate::storage::StorageError::Policy(ErrorCode::StaleSession))}
            runtime.security.state.with_attention_source(&original.target,&original.epoch,Provider::Omp,||commit(store)).map_err(crate::storage::StorageError::Policy)?
        }).await.map_err(store_error)
    }
    async fn attention_before_frame(self:&Arc<Self>,c:&Arc<Connection>,frame:&Value)->SecurityResult<()>{
        // Called only for source-ordered post-install frames, never snapshot replay.
        c.validate_read(&self.security.state,&self.topology)?;
        if frame.get("t").and_then(Value::as_str)==Some("state") {
            self.observe_controls(c,frame.get("state").unwrap_or(&Value::Null))?;
        }
        if frame["t"]=="event"&&matches!(frame["event"]["type"].as_str(),Some("agent_start"|"agent_end")){
            let target=c.target.clone();let epoch=c.epoch.clone();let connection=c.desktop_attention.clone();let start=frame["event"]["type"]=="agent_start";
            let events=self.attention_commit(c,move|store|store.omp_cycle(&target,&epoch,&connection,start,crate::pairing::now_ms())).await?;
            self.security.publish_attention(events).await?;return Ok(())
        }
        let before=self.security.state.session_state(&c.target,&c.epoch)?;let current;let mut native=None;let mut actionable=false;
        match frame.get("t").and_then(Value::as_str){
            Some("ui-request")=>{
                let request=frame.get("request").ok_or(ErrorCode::InvalidRequest)?;let id=native_json::rpc_id(request.get("reqId").ok_or(ErrorCode::InvalidRequest)?).map_err(policy)?;
                if !matches!(&id,NativeRpcId::Number(_)){return Err(ErrorCode::InvalidRequest)}
                {let dialogs=c.dialogs.lock();if dialogs.len()>=64||dialogs.values().any(|d|d.native==id){return Ok(())}}
                project_form(&self.security.state,&c.target,request)?;
                native=Some(serde_json::to_string(&id).map_err(policy)?);current=SessionStatus::Attention;actionable=true;
            },
            Some("state")=>{let mut display=before.clone();apply_native_state(&mut display,frame.get("state").unwrap_or(&Value::Null));current=display.summary.status;},
            Some("event")=>{let event=&frame["event"];match event.get("type").and_then(Value::as_str){
                Some("agent_start"|"turn_start")=>current=SessionStatus::Working,
                Some("agent_end"|"turn_end")=>current=SessionStatus::Idle,
                _=>return Ok(())
            }},_=>return Ok(())
        }
        let snapshot=native.is_none();let correlation=native.as_deref().map(|native|crate::push::correlation(&c.target,"omp",native,if actionable{"request"}else{"completed"})).unwrap_or_else(||Id("native-checkpoint".into()));
        let change=crate::storage::AttentionTransition{target:c.target.clone(),correlation,previous:before.summary.status,current,snapshot,new_actionable:actionable};
        let events=self.attention_commit(c,move|store|store.attention_transition(&change,crate::pairing::now_ms())).await?;
        self.security.publish_attention(events).await
    }
    fn observe_controls(&self,connection:&Connection,native:&Value)->SecurityResult<()> {
        let changed=connection.controls.lock().update(omp_effective_controls(native));
        if changed {self.security.state.controls_changed(&connection.target,&connection.epoch)?;}
        Ok(())
    }
    fn apply_frame(&self, connection: &Connection, frame: Value) -> SecurityResult<()> {
        self.validate_read_reported(connection)?;

        let state = &self.security.state;
        let target = &connection.target;
        let epoch = &connection.epoch;
        match frame.get("t").and_then(Value::as_str) {
            Some("entry") => {
                let entry = frame.get("entry").ok_or(ErrorCode::InvalidRequest)?;
                let count = projected_count(entry);
                let order = connection.order.fetch_add(count, Ordering::Relaxed) as u64;
                state.history_from_source(
                    target,
                    epoch,
                    project_entry(
                        state,
                        target,
                        entry,
                        RecordSource::Live,
                        order,
                        self.topology.config.omp_blob_root.as_deref(),
                    )?,
                )?;
                if count > 1000 {
                    let mut display = state.session_state(target, epoch)?;
                    display.summary.history_coverage.older_available = true;
                    state.update_from_source(target, epoch, SessionUpdate::State(display))?;
                }
            }
            Some("state") => {
                self.observe_controls(connection,frame.get("state").unwrap_or(&Value::Null))?;
                let mut display = state.session_state(target, epoch)?;
                apply_native_state(&mut display, frame.get("state").unwrap_or(&Value::Null));
                state.update_from_source(target, epoch, SessionUpdate::State(display))?;
            }
            Some("ui-request") => {
                if connection.attention_open.swap(false, Ordering::Relaxed) {
                    state.update_from_source(
                        target,
                        epoch,
                        SessionUpdate::Close(
                            connection.desktop_attention.clone(),
                            RequestClosedReason::DesktopOnly,
                        ),
                    )?;
                }
                let request = frame.get("request").ok_or(ErrorCode::InvalidRequest)?;
                let native =
                    native_json::rpc_id(request.get("reqId").ok_or(ErrorCode::InvalidRequest)?)
                        .map_err(policy)?;
                if !matches!(&native, NativeRpcId::Number(_)) {
                    return Err(ErrorCode::InvalidRequest);
                }
                let pending = project_form(state, target, request)?;
                let mut dialogs = connection.dialogs.lock();
                if dialogs.len() >= 64 {
                    return Err(ErrorCode::ResourceLimit);
                }
                if dialogs.values().any(|d| d.native == native) {
                    return Err(ErrorCode::InvalidRequest);
                }
                dialogs.insert(
                    pending.request_key.clone(),
                    Dialog {
                        native,
                        pending: pending.clone(),
                    },
                );
                drop(dialogs);
                state.update_from_source(target, epoch, SessionUpdate::Open(pending))?;
            }
            Some("ui-request-end") => {
                let native =
                    native_json::rpc_id(frame.get("reqId").ok_or(ErrorCode::InvalidRequest)?)
                        .map_err(policy)?;
                let mut dialogs = connection.dialogs.lock();
                let key = dialogs
                    .iter()
                    .find(|(_, d)| d.native == native)
                    .map(|(key, _)| key.clone());
                if let Some(key) = key {
                    dialogs.remove(&key);
                    drop(dialogs);
                    state.update_from_source(
                        target,
                        epoch,
                        SessionUpdate::Close(key, RequestClosedReason::Resolved),
                    )?;
                }
            }
            Some("event") => self.apply_event(
                connection,
                frame.get("event").ok_or(ErrorCode::InvalidRequest)?,
            )?,
            Some("bus") => {
                if let Some(data) = frame.get("data") {
                    state.update_from_source(
                        target,
                        epoch,
                        SessionUpdate::Activity(Activity {
                            state: ActivityState::Unknown,
                            text: Some(display(
                                state,
                                target,
                                Mime::ApplicationJson,
                                redacted(data).to_string(),
                            )?),
                            tool_record_id: None,
                        }),
                    )?;
                }
            }
            Some("bye" | "error") => return Err(ErrorCode::NotReady),
            Some("welcome" | "snapshot-chunk") => return Err(ErrorCode::StaleSession),
            _ => {}
        }
        Ok(())
    }
    fn apply_event(&self, connection: &Connection, event: &Value) -> SecurityResult<()> {
        let state = &self.security.state;
        let target = &connection.target;
        let epoch = &connection.epoch;
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(
            kind,
            "tool_execution_start" | "tool_execution_update" | "tool_execution_end"
        ) {
            let call = Id::new(text(event, "toolCallId").map_err(policy)?).map_err(policy)?;
            let id = Id(format!("omp-tool:{}", call.0));
            let tool = Tool {
                name: text(event, "toolName").map_err(policy)?,
                call_id: Some(call),
                status: if kind == "tool_execution_end" {
                    if event.get("isError").and_then(Value::as_bool) == Some(true) {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    }
                } else {
                    ToolStatus::Running
                },
                input: event
                    .get("args")
                    .map(|v| {
                        display(
                            state,
                            target,
                            Mime::ApplicationJson,
                            redacted(v).to_string(),
                        )
                    })
                    .transpose()?,
                output: event
                    .get("result")
                    .or_else(|| event.get("partialResult"))
                    .map(|v| {
                        display(
                            state,
                            target,
                            Mime::ApplicationJson,
                            redacted(v).to_string(),
                        )
                    })
                    .transpose()?,
            };
            let record = HistoryRecord {
                id: id.clone(),
                parent_id: None,
                turn_id: None,
                order: Uint(connection.order.fetch_add(1, Ordering::Relaxed) as u64),
                timestamp: None,
                role: Role::Tool,
                kind: RecordKind::Tool,
                text: None,
                tool: Some(tool),
                original: None,
                source: RecordSource::Live,
                complete: true,
                source_truncated: false,
            };
            state.update_from_source(
                target,
                epoch,
                SessionUpdate::History(vec![SafeHistory::from_redacted_display(record)?]),
            )?;
            state.update_from_source(
                target,
                epoch,
                SessionUpdate::Activity(Activity {
                    state: if kind == "tool_execution_end" {
                        ActivityState::Unknown
                    } else {
                        ActivityState::Working
                    },
                    text: None,
                    tool_record_id: Some(id),
                }),
            )?;
        } else if matches!(kind, "message_start" | "message_update") {
            let (body, _) = content_text(
                event
                    .get("message")
                    .and_then(|v| v.get("content"))
                    .unwrap_or(&Value::Null),
                self.topology.config.omp_blob_root.as_deref(),
            )
            .map_err(policy)?;
            state.update_from_source(
                target,
                epoch,
                SessionUpdate::Activity(Activity {
                    state: ActivityState::Working,
                    text: Some(display(state, target, Mime::TextPlain, body)?),
                    tool_record_id: None,
                }),
            )?;
        } else if matches!(
            kind,
            "agent_start" | "agent_end" | "turn_start" | "turn_end"
        ) {
            let working = matches!(kind, "agent_start" | "turn_start");
            let mut display = state.session_state(target, epoch)?;
            display.summary.status = if working {
                SessionStatus::Working
            } else {
                SessionStatus::Idle
            };
            display.activity = Activity {
                state: if working {
                    ActivityState::Working
                } else {
                    ActivityState::Idle
                },
                text: None,
                tool_record_id: None,
            };
            state.update_from_source(target, epoch, SessionUpdate::State(display))?;
        } else if kind == "notice" {
            state.update_from_source(
                target,
                epoch,
                SessionUpdate::Activity(Activity {
                    state: ActivityState::Unknown,
                    text: event
                        .get("message")
                        .and_then(Value::as_str)
                        .map(|s| display(state, target, Mime::TextPlain, s.to_owned()))
                        .transpose()?,
                    tool_record_id: None,
                }),
            )?;
        }
        Ok(())
    }
    /// Commit installs the exact original job on the SQLite owner before the cancellable reply.
    /// Validation, capacity, alias and SQL failures cannot register native work.
    pub async fn mutate(
        self: Arc<Self>,
        auth: Arc<Authorized>,
        request: Request,
    ) -> SecurityResult<Receipt> {
        let create = matches!(&request.body, RequestBody::SessionCreate(_));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        let (request,existing)=self.security.preflight_request(&auth,request).await?;
        if let Some(receipt)=existing{return Ok(receipt)}
        let selection=if let RequestBody::SessionCreate(params)=&request.body {
            Some(self.topology.capture_creation(&auth,params).await)
        } else {None};
        let admitted_selection=selection.clone();
        let controls=if let RequestBody::SessionControlsApply(params)=&request.body {
            Some(self.security.state.controls_publication(&auth,&params.target,&params.catalogue_generation))
        } else {None};
        let admitted_controls=controls.clone();let admission_authorization=auth.clone();
        let (request, readiness) = if create {
            (request, None)
        } else {
            let ready=if matches!(&request.body,RequestBody::PromptSubmit(params) if params.skill.is_some()) {
                Err(("skill_provider",ErrorCode::NotReady))
            } else {
            let target = match &request.body {
                RequestBody::SessionControlsApply(p)=>&p.target,
                RequestBody::PromptSubmit(p) => &p.target,
                RequestBody::TurnCancel(p) => &p.target,
                RequestBody::RequestRespond(p) => &p.target,
                _ => return Err(ErrorCode::InvalidRequest),
            };
            let captured = self
                .security
                .state
                .source_epoch(target)
                .map_err(|code| ("native_identity", code))
                .and_then(|_| self.connection_instance(target));
            match captured {
                Ok(connection) => {
                    self.wait_connection(&auth, &request, target, deadline, connection)
                        .await
                }
                Err(error) => Err(error),
            }
            };
            // Still perform the one final atomic matching lookup if a concurrent original won.
            // An unseen request retains its original readiness failure without a reserve retry.
            (request, Some(ready))
        };
        let pinned = readiness
            .as_ref()
            .and_then(|ready| ready.as_ref().ok())
            .cloned();
        let lookup_action = request.id.clone();
        let (tx, rx) = oneshot::channel();
        let owner = self.clone();
        let validator = self.clone();
        let authorization = auth.clone();
        let original = request.clone();
        let handle = tokio::runtime::Handle::current();
        let reservation = self
            .security
            .reserve(
                &auth,
                request,
                move |r| {
                    if create {
                        validator.validate_create(r)?;
                        let RequestBody::SessionCreate(params)=&r.body else{return Err(ErrorCode::InvalidRequest)};
                        admitted_selection.as_ref().ok_or(ErrorCode::NotReady)?.as_ref().map_err(|code|*code)?
                            .validate(&validator.topology,params)?;
                    } else {
                        let checked = match &readiness {
                            Some(Ok(connection)) => {
                                if tokio::time::Instant::now() >= deadline {
                                    Err(("source_pending", ErrorCode::Timeout))
                                } else {
                                    validator.validate_checked(r, Some(connection), true)
                                }
                            }
                            Some(Err(error)) => Err(*error),
                            None => Err(("method", ErrorCode::InvalidRequest)),
                        };
                        checked.map_err(|(_, code)| code)?;
                        if let RequestBody::SessionControlsApply(params)=&r.body {
                            let publication=admitted_controls.as_ref().ok_or(ErrorCode::NotReady)?.as_ref().map_err(|code|*code)?;
                            validator.security.state.controls_publication_current(&admission_authorization,publication)?;
                            let connection=readiness.as_ref().ok_or(ErrorCode::NotReady)?.as_ref().map_err(|(_,code)|*code)?;
                            validator.validate_controls_selection(connection,publication,params)?;
                        }
                    }
                    if validator.jobs.lock().len() >= 64 {
                        return Err(ErrorCode::Busy);
                    }
                    Ok(())
                },
                move |committed| {
                    owner.register_committed_original(
                        authorization,
                        OriginalMutation {request:original,connection:pinned,creation:selection.and_then(Result::ok),controls:controls.and_then(Result::ok)},
                        committed,
                        deadline,
                        tx,
                        handle,
                    );
                },
            )
            .await?;
        if let Some(receipt) = reservation_reply(create, reservation) {
            return Ok(receipt);
        }
        match tokio::time::timeout(Duration::from_secs(if create { 119 } else { 14 }), rx).await {
            Ok(Ok(result)) => result,
            _ => receipt(&self.security, &auth, &lookup_action).await,
        }
    }
    fn register_committed_original(
        self: &Arc<Self>,
        authorization: Arc<Authorized>,
        original:OriginalMutation,
        committed: Receipt,
        deadline: tokio::time::Instant,
        tx: oneshot::Sender<SecurityResult<Receipt>>,
        handle: tokio::runtime::Handle,
    ) {
        let OriginalMutation {request:original,connection,creation:selection,controls}=original;
        let worker = self.clone();
        self.jobs.lock().spawn_on(
            async move {
                // This callback owns the committed request fingerprint and paired
                // epoch; no action-ID-only lookup can adopt a different reservation.
                let action = committed.action_id;
                let outcome = if let RequestBody::SessionCreate(params) = &original.body {
                    match selection.as_deref() {
                        Some(selection)=>worker.clone()
                            .creation_job(&authorization,&action,params,selection,deadline).await,
                        None=>Err(ErrorCode::NotReady),
                    }
                } else if let Some(connection) = connection {
                    worker
                        .write_once(&authorization, original, deadline, connection,controls)
                        .await
                        .map(|_| ())
                } else {
                    Err(ErrorCode::NotReady)
                };
                if let Err(code) = outcome {
                    if selection.as_ref().is_some_and(|selection|selection.unproved()) {
                        let _=worker.topology.uncertain_creation(&authorization,&action,code).await;
                    }
                    let _ = worker.fail_unentered(&authorization, &action, code).await;
                }
                let result = receipt(&worker.security, &authorization, &action).await;
                let _ = tx.send(result);
            },
            &handle,
        );
    }

    fn boundary_readiness(
        &self,
        request: &Request,
        connection: &Arc<Connection>,
    ) -> Result<pairing::BoundaryReadiness, (&'static str, ErrorCode)> {
        match self.validate_checked(request, Some(connection), false) {
            Ok(()) => Ok(pairing::BoundaryReadiness::Ready),
            Err(("source_pending" | "core_sync", ErrorCode::NotReady)) => {
                // Deferral is not write permission. Reuse the physically fenced,
                // exact-epoch no-effect rule; every other failure remains terminal.
                self.validate_checked(request, Some(connection), true)?;
                Ok(pairing::BoundaryReadiness::Pending)
            }
            Err(error) => Err(error),
        }
    }
    async fn write_once(
        self:&Arc<Self>,
        auth: &Arc<Authorized>,
        request: Request,
        deadline: tokio::time::Instant,
        original_connection: Arc<Connection>,
        controls:Option<Arc<crate::backends::ControlsPublication>>,
    ) -> SecurityResult<Receipt> {
        if matches!(&request.body,RequestBody::SessionControlsApply(_)) {
            return self.write_controls_once(auth,request,deadline,original_connection,controls.ok_or(ErrorCode::NotReady)?).await;
        }
        if matches!(&request.body,RequestBody::PromptSubmit(params) if params.skill.is_some()) {return Err(ErrorCode::NotReady)}
        if matches!(&request.body,RequestBody::PromptSubmit(params) if params.images.is_some()||params.delivery.is_some_and(|delivery|delivery!=PromptDelivery::Native)) {
            return self.write_prompt_extension(auth,request,deadline,original_connection).await;
        }
        let mut boundary = None;
        let outcome = async {
            let target = match &request.body {
                RequestBody::PromptSubmit(p) => p.target.clone(),
                RequestBody::TurnCancel(p) => p.target.clone(),
                RequestBody::RequestRespond(p) => p.target.clone(),
                _ => return Err(ErrorCode::InvalidRequest),
            };
            let mut connection = self
                .wait_connection(auth, &request, &target, deadline, original_connection)
                .await
                .map_err(|(_, code)| code)?;
            let mut encrypted = None;
            let mut result = None;
            loop {
                let writer_deadline =
                    deadline.min(tokio::time::Instant::now() + Duration::from_secs(5));
                let mut writer = tokio::time::timeout_at(writer_deadline, connection.writer.lock())
                    .await
                    .map_err(|_| {
                        if tokio::time::Instant::now() >= deadline {
                            ErrorCode::Timeout
                        } else {
                            ErrorCode::Busy
                        }
                    })?;
                if boundary.is_none() {
                    let (payload, action_result) = match &request.body {
                        RequestBody::PromptSubmit(p) => (
                            json!({"t":"prompt","text":p.text.0}),
                            ActionResult::Prompt(ActionResultPrompt {
                                native_turn_id: None,
                            }),
                        ),
                        RequestBody::TurnCancel(_) => (
                            json!({"t":"abort"}),
                            ActionResult::Cancel(ActionResultCancel {
                                native_turn_id: None,
                            }),
                        ),
                        RequestBody::RequestRespond(p) => {
                            let dialogs = connection.dialogs.lock();
                            let dialog = dialogs
                                .get(&p.request_key)
                                .ok_or(ErrorCode::AlreadyResolved)?;
                            (
                                dialog.response(&p.answer)?,
                                ActionResult::Response(ActionResultResponse {
                                    resolution: ResponseResolution::Written,
                                }),
                            )
                        }
                        _ => return Err(ErrorCode::InvalidRequest),
                    };
                    encrypted = Some(connection.link.seal_value(&payload).map_err(policy)?);
                    result = Some(action_result);
                    let fd = connection.fd()?;
                    boundary = Some(
                        self.security
                            .unstarted_boundary(auth, request.id.clone(), fd, deadline)
                            .await?,
                    );
                }
                boundary = boundary
                    .take()
                    .expect("original unstarted boundary")
                    .try_start(
                        || {
                            self.boundary_readiness(&request, &connection)
                                .map_err(|(_, code)| code)
                        },
                        async {
                            // Only a started effect consumes these original bytes/result.
                            let bytes = encrypted.take().expect("original sealed native message");
                            let written = writer.send(Message::Binary(bytes.into())).await.is_ok();
                            ReceiptTransition {
                                state: if written {
                                    ReceiptState::Dispatched
                                } else {
                                    ReceiptState::Uncertain
                                },
                                proof: if written {
                                    TransitionProof::WriteCompleted
                                } else {
                                    TransitionProof::LostOutcome
                                },
                                result: if written { result.take() } else { None },
                                error: None,
                                at: pairing::now_ms(),
                            }
                        },
                    )
                    .await?;
                // No writer/auth/SQL/start permit survives the source readiness wait.
                drop(writer);
                if boundary.is_none() {
                    break;
                }
                connection = self
                    .wait_connection(auth, &request, &target, deadline, connection)
                    .await
                    .map_err(|(_, code)| code)?;
            }
            receipt(&self.security, auth, &request.id).await
        }
        .await;
        match (outcome, boundary) {
            (Err(code), Some(boundary)) => boundary.fail(code).await.and(Err(code)),
            (outcome, _) => outcome,
        }
    }
    pub fn session_open(
        &self,
        auth: &Authorized,
        request: Uuid,
        target: &Target,
    ) -> SecurityResult<Uuid> {
        let state = &self.security.state;
        self.refresh_history_if_ready(target)?;
        state.session_open(&auth.connection_id, request, target, pairing::now_ms())
    }
    pub async fn history(
        &self,
        peer: &AuthenticatedPeer,
        params: &SessionHistoryParams,
    ) -> SecurityResult<SessionHistoryResult> {
        use crate::state::{BrowseLookup, BrowsePosition};
        let state = &self.security.state;
        let connection = self.connection(&params.target)?;
        let limit = params.limit.unwrap_or(Uint(100)).0 as usize;
        let ticket = if let Some(cursor) = &params.cursor {
            match state.browse_page(
                peer,
                &params.target,
                params.view_id.as_ref().ok_or(ErrorCode::InvalidRequest)?,
                cursor,
                limit,
                pairing::now_ms(),
            )? {
                BrowseLookup::Cached(page) => return Ok(page),
                BrowseLookup::Fetch(ticket) => ticket,
            }
        } else {
            if params.view_id.is_some() {
                return Err(ErrorCode::InvalidRequest);
            }
            let end = {
                let mut history = connection.history.lock();
                history.refresh().map_err(policy)?;
                history.entries.len()
            };
            state.begin_browse(
                peer,
                &params.target,
                &connection.epoch,
                BrowsePosition::new((end, 0usize, UINT_MAX), 64)?,
                limit,
                pairing::now_ms(),
            )?
        };
        let gate = ticket.gate();
        let _fetch = gate.lock().await;
        let ticket = match state.browse_page(
            peer,
            &params.target,
            ticket.view_id(),
            &ticket.cursor(),
            limit,
            pairing::now_ms(),
        )? {
            BrowseLookup::Cached(page) => return Ok(page),
            BrowseLookup::Fetch(ticket) => ticket,
        };
        if ticket.source() != &connection.epoch {
            return Err(ErrorCode::CursorExpired);
        }
        let &(end, record_end, before) = ticket.position::<(usize, usize, u64)>()?;
        let (projected, next) = {
            let history = connection.history.lock();
            history.native_page(
                (end, record_end, 100),
                state,
                &params.target,
                &connection.epoch,
                self.topology.config.omp_blob_root.as_deref(),
            )?
        };
        let base = before
            .checked_sub(projected.len() as u64)
            .ok_or(ErrorCode::ResourceLimit)?;
        let records = projected
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let mut record = row.record().clone();
                record.order = Uint(base + index as u64);
                SafeHistory::from_redacted_display(record)
            })
            .collect::<SecurityResult<Vec<_>>>()?;
        let next = next
            .map(|token| {
                let (a, b) = token
                    .0
                    .strip_prefix("omp-file:")
                    .and_then(|s| s.split_once(':'))
                    .ok_or(ErrorCode::CursorExpired)?;
                let end = a.parse::<usize>().map_err(|_| ErrorCode::CursorExpired)?;
                let offset = b.parse::<usize>().map_err(|_| ErrorCode::CursorExpired)?;
                BrowsePosition::new((end, offset, base), 64)
            })
            .transpose()?;
        self.connection(&params.target)?;
        state.finish_browse(ticket, records, next, pairing::now_ms())
    }
}
pub fn terminal_owner(
    reporter: &ProcessEvidence,
    owner: &ProcessEvidence,
    info: &herdr::PaneProcessInfo,
) -> io::Result<()> {
    let mut candidate = owner.clone();
    for _ in 0..128 {
        if info
            .foreground_processes
            .iter()
            .any(|p| herdr::reported_pid_matches(reporter, &candidate, p.pid))
        {
            if herdr::process_evidence(owner.pid)? == *owner {
                return Ok(());
            }
            return Err(invalid());
        }
        let stat = fs::read_to_string(format!("/proc/{}/stat", candidate.pid))?;
        herdr::process_start_ticks(&stat, candidate.pid)?;
        let tail = stat.rsplit_once(") ").ok_or_else(invalid)?.1;
        let parent = tail
            .split_whitespace()
            .nth(1)
            .ok_or_else(invalid)?
            .parse::<u32>()
            .map_err(|_| invalid())?;
        if parent == 0 || parent == candidate.pid {
            return Err(invalid());
        }
        let next = herdr::process_evidence(parent)?;
        if herdr::process_evidence(candidate.pid)? != candidate {
            return Err(invalid());
        }
        candidate = next;
    }
    Err(invalid())
}
fn socket_fd(socket: &NativeSocket) -> io::Result<OwnedFd> {
    let stream = match socket.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => s.get_ref().0,
        _ => return Err(invalid()),
    };
    stream.as_fd().try_clone_to_owned()
}
fn decrypt_frame(link: &Link, bytes: axum::body::Bytes) -> io::Result<Value> {
    let decode = |bytes: &mut [u8]| native_json::parse(link.open(bytes)?, NATIVE);
    match bytes.try_into_mut() {
        Ok(mut bytes) => decode(&mut bytes),
        Err(bytes) => decode(&mut bytes.to_vec()),
    }
}
async fn next_native(
    socket: &mut NativeSocket,
    link: &Link,
    timeout: Duration,
) -> io::Result<Value> {
    tokio::time::timeout(timeout, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Binary(bytes))) => return decrypt_frame(link, bytes),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return Err(invalid()),
            }
        }
    })
    .await
    .map_err(|_| invalid())?
}
async fn next_split(
    stream: &mut futures_util::stream::SplitStream<NativeSocket>,
    link: &Link,
    timeout: Option<Duration>,
) -> io::Result<Value> {
    let next = async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Binary(bytes))) => return decrypt_frame(link, bytes),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(_)) => return Err(invalid()),
                Some(Err(_)) | None => return Err(invalid()),
            }
        }
    };
    match timeout {
        Some(timeout) => tokio::time::timeout(timeout, next)
            .await
            .map_err(|_| invalid())?,
        None => next.await,
    }
}
fn initial_state(
    source: &Source,
    agent: &Agent,
    snapshot: &RegistrySnapshot,
    welcome: &Value,
    history: &HistoryFile,
) -> SessionState {
    let mut state = SessionState {
        summary: SessionSummary {
            session_id: Uuid::random(),
            generation: Uuid::random(),
            server_id: source.server_id.clone(),
            workspace_id: agent.workspace_id.clone(),
            terminal_id: agent.terminal_id.clone(),
            pane_id: agent.pane_id.clone(),
            provider: Provider::Omp,
            native_id: Id(snapshot.session.clone()),
            title: agent.title.clone().unwrap_or_else(|| "OMP".into()),
            status: SessionStatus::Unknown,
            connectivity: SessionConnectivity::Synchronizing,
            capabilities: Capabilities { controls: false, terminal_create:false,artifacts:false, attachments:false, history: true,
            prompt: true,
            cancel: true,
            respond: true,
            prompt_mode: PromptMode::SubmitOrSteer,
            cancel_mode: CancelMode::CurrentWork,
            terminal_read: false,
            worktrees_read: false,
            changes_read: false,
            workspace_rename: false, agent_rename: false, skills_read: false, },
            skills_generation: None,
            history_coverage: HistoryCoverage {
                scope: HistoryScope::AllBranches,
                source: if history.identity.is_some() {
                    HistorySource::PersistedAndLive
                } else {
                    HistorySource::LiveOnly
                },
                older_available: history.record_count() > 1000,
                source_truncated: false,
                desktop_branch_known: false,
            },
            reason: Some(format!(
                "{BRANCH_NOTICE} Busy prompts steer; Stop cancels current work. Stock select/editor only; custom dialogs remain desktop-only."
            )),
        },
        native_turn_id: None,
        desktop_selection: DesktopSelection::Unknown,
        activity: Activity {
            state: ActivityState::Unknown,
            text: None,
            tool_record_id: None,
        },
    };
    apply_native_state(&mut state, welcome.get("state").unwrap_or(&Value::Null));
    state
}
fn omp_effective_controls(native:&Value)->crate::backends::EffectiveControls {
    let model=native.get("model").and_then(Value::as_object).and_then(|model|{
        let provider=model.get("provider")?.as_str()?;let id=model.get("id")?.as_str()?;
        if provider.is_empty()||id.is_empty()||provider.len()>512||id.len()>512 {return None}
        Some(crate::backends::NativeModelIdentity {provider:provider.to_owned(),model:id.to_owned()})
    });
    let effort=native.get("thinkingLevel").and_then(Value::as_str).filter(|effort|effort.len()<=FRAME_MAX).map(str::to_owned);
    crate::backends::EffectiveControls {model,effort,mode:None}
}
fn apply_native_state(state: &mut SessionState, native: &Value) {
    let busy = native.get("isStreaming").and_then(Value::as_bool);
    state.summary.status = match busy {
        Some(true) => SessionStatus::Working,
        Some(false) => SessionStatus::Idle,
        None => SessionStatus::Unknown,
    };
    state.activity.state = match busy {
        Some(true) => ActivityState::Working,
        Some(false) => ActivityState::Idle,
        None => ActivityState::Unknown,
    };
    if let Some(title) = native.get("sessionName").and_then(Value::as_str) {
        state.summary.title = title.to_owned();
    }
}
fn merge_projected(
    records: &mut HashMap<Id, (HistoryRecord, HistoryOrigins)>,
    record: SafeHistory,
) {
    let record = record.record().clone();
    if let Some((old, origins)) = records.get_mut(&record.id) {
        if record.source == RecordSource::Persisted {
            old.order = record.order;
        }
        *old = merge_history(Provider::Omp, old, record, origins, false);
    } else {
        let origins = HistoryOrigins::from_record(&record);
        records.insert(record.id.clone(), (record, origins));
    }
}
async fn receipt(security: &Security, auth: &Authorized, action: &Uuid) -> SecurityResult<Receipt> {
    let proof = auth.proof.clone();
    let action = action.clone();
    let receipt = security
        .storage
        .call(move |s| s.status(&proof, &action, pairing::now_ms()))
        .await
        .map_err(pairing::store_error)?
        .ok_or(ErrorCode::NotFound)?;
    security.state.peer_event(
        &auth.proof,
        EventBody::ActionStatus(ActionStatusEvent {
            receipt: receipt.clone(),
        }),
    )?;
    Ok(receipt)
}

impl Runtime {
    pub fn validate_create(&self, request: &Request) -> SecurityResult<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(ErrorCode::NotReady);
        }
        let RequestBody::SessionCreate(params) = &request.body else {
            return Err(ErrorCode::InvalidRequest);
        };
        if params.provider != Provider::Omp {
            return Err(ErrorCode::NotReady);
        }
        self.topology.validate_create(request)?;
        if self.relay_ready.borrow().is_none() {
            return Err(ErrorCode::NotReady);
        }
        Ok(())
    }
    async fn creation_job(
        self: Arc<Self>,
        auth: &Authorized,
        action: &Uuid,
        params: &SessionCreateParams,
        selection:&herdr::CreationSelection,
        deadline: tokio::time::Instant,
    ) -> SecurityResult<()> {
        if self.stopping.load(Ordering::Acquire) || tokio::time::Instant::now() >= deadline {
            return Err(ErrorCode::NotReady);
        }
        let workspace = self
            .topology.workspace_once(&self.security, auth, action, params, selection)
            .await?;
        if !matches!(workspace.state, ReceiptState::Dispatching) {
            return Ok(());
        }
        if workspace.creation.as_ref().is_none_or(|p| {
            p.stage != CreationStage::Workspace || p.stage_state != StageState::Confirmed
        }) {
            return Ok(());
        }
        let prepared=match self.topology.prepare_created_shell(
            &self.security,auth,(action,params,selection),deadline,self.first_guard.as_ref(),
        ).await {
            Ok(Some(prepared))=>prepared,
            Ok(None)=>return Ok(()),
            Err(code)=>{
                self.topology.uncertain_creation(auth,action,code).await?;
                return Ok(());
            }
        };
        let probed=self.topology.shell_probe_once_reported(&self.security,auth,prepared,self.first_guard.as_ref()).await?;
        if probed.state != ReceiptState::Dispatching {
            return Ok(());
        }
        let progress = probed.creation.ok_or(ErrorCode::InvalidRequest)?;
        if progress.stage != CreationStage::Workspace
            || progress.stage_state != StageState::Confirmed
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
            return Err(ErrorCode::Timeout);
        }
        let port = (*self.relay_ready.borrow()).ok_or(ErrorCode::NotReady)?;
        let resources = progress.resources;
        let mut relay_resources = resources.clone();
        relay_resources.relay_started = Some(true);
        self.security.start_boundary(auth, action.clone(), (CreationStage::Relay, StageMetadata::default()), || { selection.validate(&self.topology, params).map_err(|code| crate::pairing::CreationAdmissionFailure::guarded(code, crate::pairing::CreationAdmissionGuard::Directory))?; let checked: crate::pairing::SecurityResult<()> = (|| {
            if tokio::time::Instant::now() >= deadline {
                return Err(ErrorCode::Timeout);
            }
            self.topology
                .validate_provider_stage(action, params, &resources)?;
            if *self.relay_ready.borrow() != Some(port) {
                return Err(ErrorCode::NotReady);
            }
            Ok(())
        })(); checked.map_err(Into::into) }, || {
            let provider_path = overlay(&self.topology.config, port).map_err(policy)?;
            Ok((
                provider_path,
                CreationOutcome {
                    resources: relay_resources,
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(Id(
                        "Private overlay and companion-owned loopback relay ready".into(),
                    )),
                    at: pairing::now_ms(),
                },
            ))
        })
            .await?;
        // New root-pane shell measurement is mandatory. Never borrow another pane's proof.
        let project = match self.topology.execution_directory(selection, params) {
            Ok(project) => match project.to_str() {
                Some(path) => path.to_owned(),
                None => {
                    if let Some(reporter) = &self.first_guard { reporter.report_launch_preflight("project_path", ErrorCode::NotReady, "none"); }
                    return Err(ErrorCode::NotReady);
                }
            },
            Err(code) => {
                if let Some(reporter) = &self.first_guard { reporter.report_launch_preflight("project_lookup", code, "none"); }
                return Err(code);
            }
        };
        let overlay = match creation_overlay(&self.topology.config, port, self.producer_capture_authorized) {
            Ok(overlay) => match overlay.to_str() {
                Some(path) => path.to_owned(),
                None => {
                    if let Some(reporter) = &self.first_guard { reporter.report_launch_preflight("creation_overlay_path", ErrorCode::NotReady, "none"); }
                    return Err(ErrorCode::NotReady);
                }
            },
            Err(error) => {
                let code = policy(&error);
                if let Some(reporter) = &self.first_guard { reporter.report_launch_preflight("creation_overlay", code, io_kind(&error)); }
                return Err(code);
            }
        };
        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
            return Err(ErrorCode::Timeout);
        }
        let mut launch_args = vec!["--cwd".into(), project, "--config".into(), overlay];
        match diagnostic_extension_args(&self.topology.config, self.producer_capture_authorized) {
            Ok(args) => launch_args.extend(args),
            Err(error) => {
                let code = policy(&error);
                if let Some(reporter) = &self.first_guard { reporter.report_launch_preflight("diagnostic_extension_args", code, io_kind(&error)); }
                return Err(code);
            }
        }
        // Optional Skills support never changes the approved outer wrapper or
        // the normal provider creation path when source/grant proof is absent.
        if let Some(pane)=&resources.pane_id{
            // The already-admitted creation request owns the original server ID;
            // result resources carry only the action's produced native IDs.
            if let Ok(source) = self.topology.source(&params.server_id)
                && let Ok(args) = self.skills.prepare(source, pane)
            {
                launch_args.extend(args)
            }
        }
        let launched = self
            .topology
            .launch_once_reported(
                &self.security,
                auth,
                action,
                (params,selection),
                launch_args,
                self.first_guard.clone(),
            )
            .await?;
        let progress = launched.creation.ok_or(ErrorCode::InvalidRequest)?;
        if launched.state != ReceiptState::Dispatching
            || progress.stage != CreationStage::Launch
            || progress.stage_state != StageState::Dispatching
        {
            return Ok(());
        }
        let proof = auth.proof.clone();
        let id = action.clone();
        let alias = self
            .security
            .storage
            .call(move |s| s.creation_local(&proof, &id))
            .await
            .map_err(pairing::store_error)?
            .ok_or(ErrorCode::NotFound)?
            .alias;
        let proof = auth.proof.clone();
        let id = action.clone();
        let probe = self
            .security
            .storage
            .call(move |s| s.creation_probe(&proof, &id))
            .await
            .map_err(pairing::store_error)?;
        let mut known = progress.resources;
        let pane = known.pane_id.clone().ok_or(ErrorCode::NotReady)?;
        loop {
            if self.stopping.load(Ordering::Acquire) {
                return Err(ErrorCode::NotReady);
            }
            if tokio::time::Instant::now() >= deadline {
                if let Some(reporter) = &self.first_guard {
                    reporter.creation_readiness_failed();
                }
                let proof = auth.proof.clone();
                let id = action.clone();
                self.security
                    .storage
                    .call(move |s| {
                        s.creation_result(
                            &proof,
                            &id,
                            CreationOutcome {
                                resources: known,
                                metadata: StageMetadata::default(),
                                state: StageState::Uncertain,
                                definitive: None,
                                at: pairing::now_ms(),
                            },
                        )
                    })
                    .await
                    .map_err(pairing::store_error)?;
                return Ok(());
            }
            {
                let _guard = self.security.native_guard(auth).await?;
            }
            let source = match self.topology.source(&params.server_id) {
                Ok(source) => source,
                Err(ErrorCode::NotReady) => {
                    // Launch events invalidate topology; wait for its authoritative refresh.
                    if let Some(reporter) = &self.first_guard {
                        reporter.defer_creation_topology_not_ready();
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(code) => {
                    if let Some(reporter) = &self.first_guard {
                        reporter.report("omp_ready", "topology_failure", Some(code));
                    }
                    return Err(code);
                }
            };
            let root = source.snapshot.pane(&pane).map_err(policy)?;
            if source.snapshot.focused_workspace_id.as_ref() != Some(&probe.focused_workspace_id)
                || root.focused
            {
                if let Some(reporter) = &self.first_guard {
                    reporter.report("omp_ready", "topology_not_ready", Some(ErrorCode::NotReady));
                }
                return Err(ErrorCode::NotReady);
            }
            let agent = source.snapshot.agents.iter().find(|a| {
                a.name.as_deref() == Some(alias.as_str())
                    && a.pane_id == pane
                    && known.workspace_id.as_ref() == Some(&a.workspace_id)
                    && known.terminal_id.as_ref() == Some(&a.terminal_id)
                    && a.agent.as_deref() == Some("omp")
            });
            if let Some(reporter) = &self.first_guard {
                reporter.creation_agent_observation(agent);
            }
            let connection = agent.and_then(|agent| {
                self.connections
                    .lock()
                    .values()
                    .find(|c| {
                        c.key.server_id == params.server_id
                            && c.key.server_incarnation == source.incarnation
                            && c.key.terminal_id == agent.terminal_id
                            && c.source.evidence == source.evidence
                            && match c.validate_checked(&self.security.state, &self.topology) {
                                Ok(()) => true,
                                Err((guard, code)) => {
                                    if let Some(reporter) = &self.first_guard {
                                        reporter.report("omp_connection", guard, Some(code));
                                    }
                                    false
                                }
                            }
                    })
                    .cloned()
            });
            if let Some(connection) = connection {
                let _guard = self.security.native_guard(auth).await?;
                connection.validate(&self.security.state, &self.topology)?;
                let display = self
                    .security
                    .state
                    .session_state(&connection.target, &connection.epoch)?;
                if display.summary.connectivity != SessionConnectivity::Ready {
                    if let Some(reporter) = &self.first_guard {
                        reporter.report(
                            "omp_ready",
                            "topology_not_ready",
                            Some(ErrorCode::NotReady),
                        );
                    }
                    return Err(ErrorCode::NotReady);
                }
                known.native_id = Some(connection.key.native_conversation_id.clone());
                let metadata = StageMetadata {
                    socket_epoch: Some(connection.epoch.clone()),
                    linux_boot_id: Some(connection.owner.linux_boot_id.clone()),
                    pid: Some(connection.owner.pid),
                    pid_start_ticks: Some(connection.owner.start_ticks),
                    ..StageMetadata::default()
                };
                let proof = auth.proof.clone();
                let id = action.clone();
                let resources = known.clone();
                let evidence = metadata.clone();
                self.security.storage.call(move|s|s.creation_result(&proof,&id,CreationOutcome{resources,metadata:evidence,state:StageState::Confirmed,definitive:Some(Id("Correlated launch alias, native header and exact kernel terminal owner".into())),at:pairing::now_ms()})).await.map_err(pairing::store_error)?;
                known.session_id = Some(connection.target.session_id.clone());
                known.generation = Some(connection.target.generation.clone());
                let proof = auth.proof.clone();
                let id = action.clone();
                let intent = metadata.clone();
                self.security
                    .storage
                    .call(move |s| {
                        s.creation_intent(
                            &proof,
                            &id,
                            CreationStage::Ready,
                            intent,
                            pairing::now_ms(),
                        )
                    })
                    .await
                    .map_err(pairing::store_error)?;
                connection.validate(&self.security.state, &self.topology)?;
                let proof = auth.proof.clone();
                let id = action.clone();
                let final_resources = known.clone();
                self.security
                    .storage
                    .call(move |s| {
                        s.creation_result(
                            &proof,
                            &id,
                            CreationOutcome {
                                resources: final_resources,
                                metadata,
                                state: StageState::Confirmed,
                                definitive: Some(Id(
                                    "Exact native FINAL writable snapshot installed".into(),
                                )),
                                at: pairing::now_ms(),
                            },
                        )?;
                        s.transition(
                            &proof,
                            &id,
                            ReceiptTransition {
                                state: ReceiptState::Succeeded,
                                proof: TransitionProof::CorrelatedAcknowledgement(Id(
                                    "Ready confirmed native binding".into(),
                                )),
                                result: Some(ActionResult::Created(Box::new(
                                    ActionResultCreated {
                                        target: connection.target.clone(),
                                        workspace_id: known.workspace_id.ok_or(
                                            crate::storage::StorageError::Policy(
                                                ErrorCode::NotReady,
                                            ),
                                        )?,
                                        terminal_id: known.terminal_id.ok_or(
                                            crate::storage::StorageError::Policy(
                                                ErrorCode::NotReady,
                                            ),
                                        )?,
                                        pane_id: known.pane_id.ok_or(
                                            crate::storage::StorageError::Policy(
                                                ErrorCode::NotReady,
                                            ),
                                        )?,
                                        native_id: known.native_id.ok_or(
                                            crate::storage::StorageError::Policy(
                                                ErrorCode::NotReady,
                                            ),
                                        )?,
                                        binding_id: None,
                                    },
                                ))),
                                error: None,
                                at: pairing::now_ms(),
                            },
                        )
                    })
                    .await
                    .map_err(pairing::store_error)?;
                return Ok(());
            }
            if let Some(reporter) = &self.first_guard {
                let guard = match agent {
                    None => "topology_not_ready",
                    Some(agent) => match agent.agent_session.as_ref() {
                        None => "native_report_absent",
                        Some(report) if report.agent != "omp" || report.kind != "path" => {
                            "native_report_wrong"
                        }
                        Some(_) => "no_connection",
                    },
                };
                reporter.report("omp_ready", guard, Some(ErrorCode::NotReady));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    pub async fn fail_unentered(
        &self,
        auth: &Authorized,
        action: &Uuid,
        code: ErrorCode,
    ) -> SecurityResult<Receipt> {
        let proof = auth.proof.clone();
        let id = action.clone();
        self.security
            .storage
            .call(move |s| {
                let current = s
                    .status(&proof, &id, pairing::now_ms())?
                    .ok_or(crate::storage::StorageError::Policy(ErrorCode::NotFound))?;
                if matches!(
                    current.state,
                    ReceiptState::Failed
                        | ReceiptState::Succeeded
                        | ReceiptState::Dispatched
                        | ReceiptState::Uncertain
                ) {
                    return Ok(current);
                }
                let pending = current.state == ReceiptState::Dispatching
                    && current
                        .creation
                        .as_ref()
                        .is_none_or(|p| p.stage_state == StageState::Dispatching);
                s.transition(
                    &proof,
                    &id,
                    ReceiptTransition {
                        state: if pending {
                            ReceiptState::Uncertain
                        } else {
                            ReceiptState::Failed
                        },
                        proof: if pending {
                            TransitionProof::LostOutcome
                        } else {
                            TransitionProof::DefinitiveNoEffect(Id(
                                "No new native effect was admitted".into(),
                            ))
                        },
                        result: None,
                        error: Some(crate::transport::wire_error(code)),
                        at: pairing::now_ms(),
                    },
                )
            })
            .await
            .map_err(pairing::store_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::NativePageLookup;
    use crate::storage::{PairedDevice, Storage};
    use base64::engine::general_purpose::STANDARD;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            // The discovery executable requires protected ancestors, not shared /tmp.
            let path = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(format!(".go-{}", &Uuid::random().0[..8]));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn herdr_executable(&self, socket: &Path) -> PathBuf {
            let executable = self.0.join("herdr");
            let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .filter_map(|path| fs::canonicalize(path.join("sh")).ok())
                .find(|path| path.is_file())
                .expect("fixture discovery interpreter is required");
            let listing = json!({"sessions":[{"name":"owned","default":false,"running":true,"socket_path":socket,"session_dir":self.0}]});
            fs::write(
                &executable,
                format!("#!{}\nprintf '%s\\n' '{}'\n", shell.display(), listing),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            executable
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn display_state() -> SessionState {
        serde_json::from_value(json!({"summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":Uuid::random(),"workspaceId":"w","terminalId":"t","paneId":"p","provider":"omp","nativeId":"native","title":"OMP","status":"idle","connectivity":"ready","capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},"historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":true,"sourceTruncated":false,"desktopBranchKnown":false}},"desktopSelection":"unknown","activity":{"state":"idle"}})).unwrap()
    }
    fn peer() -> AuthenticatedPeer {
        let mut store = Storage::memory([4; 32], 0).unwrap();
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[5; 32]).public().to_string()),
            device_name: Label::new("native history").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let epoch = store.pair_device(&device).unwrap();
        store.authenticated_hello(&device.device_id, epoch).unwrap()
    }
    #[test]
    fn retained_boundary_inside_one_native_entry_refetches_all_tools_with_overlap() {
        let root = Root::new();
        let path = root.0.join("s.jsonl");
        let entry = json!({"type":"message","id":"one-entry","parentId":null,"message":{"role":"assistant","content":(0..1005).map(|n|json!({"type":"toolCall","id":format!("call-{n}"),"name":"tool","arguments":{"n":n}})).collect::<Vec<_>>()}});
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        writeln!(file,"{}",json!({"type":"session","id":"native","timestamp":"2026-10-07T00:00:00.000Z","cwd":"/not-authority"})).unwrap();
        writeln!(file, "{entry}").unwrap();
        drop(file);
        let history = HistoryFile::open(
            path,
            vec![root.0.clone()],
            herdr::process_evidence(std::process::id()).unwrap(),
        )
        .unwrap();
        assert_eq!(history.record_count(), 1006);
        let state = State::default();
        let display = display_state();
        let target = Target {
            session_id: display.summary.session_id.clone(),
            generation: display.summary.generation.clone(),
        };
        let epoch = state.begin_install(display.clone()).unwrap();
        let (records, native_older) = history
            .native_page((history.len(), 0, 1000), &state, &target, &epoch, None)
            .unwrap();
        assert_eq!(records.len(), 1000);
        assert_eq!(records[0].record().id.0, "omp-tool:call-5");
        state
            .finish_install(
                &epoch,
                ProviderSnapshot {
                    state: display,
                    pending: vec![],
                    history: records,
                    older_available: true,
                    native_older,
                },
            )
            .unwrap();
        let transient = SafeHistory::from_redacted_display(HistoryRecord {
            id: Id("transient".into()),
            parent_id: None,
            turn_id: None,
            order: Uint(1007),
            timestamp: None,
            role: Role::Tool,
            kind: RecordKind::Tool,
            text: None,
            tool: Some(Tool {
                name: "live".into(),
                call_id: None,
                status: ToolStatus::Running,
                input: None,
                output: None,
            }),
            original: None,
            source: RecordSource::Live,
            complete: true,
            source_truncated: false,
        })
        .unwrap();
        state
            .history_from_source(&target, &epoch, vec![transient])
            .unwrap();
        let p = peer();
        let mut cursor = None;
        let mut visible = HashSet::new();
        let refetch;
        loop {
            let page = state
                .history(&p, &target, cursor.as_ref(), 100, pairing::now_ms())
                .unwrap();
            for r in page.result.records {
                visible.insert(r.id.0);
            }
            cursor = page.result.next_cursor;
            if cursor.is_none() {
                refetch = page
                    .refetch_older
                    .expect("captured native refetch boundary");
                break;
            }
        }
        assert_eq!(refetch.source, epoch);
        assert_eq!(refetch.oldest.as_ref().unwrap().0.0, "omp-tool:call-6");
        let native = history
            .refetch_cursor(refetch.oldest.clone().map(|(id, parent, _)| (id, parent)))
            .unwrap()
            .unwrap();
        assert_eq!(native.0, "omp-file:1:8");
        let opaque = state
            .native_history_cursor(
                &p,
                &target,
                &refetch.source,
                native,
                Some(&refetch),
                pairing::now_ms(),
            )
            .unwrap();
        let NativePageLookup::Fetch(ticket) = state
            .native_history_page(&p, &target, &opaque, 100, pairing::now_ms())
            .unwrap()
        else {
            panic!("actual native read required")
        };
        let (records, next) = history
            .native_page((1, 8, 100), &state, &target, &epoch, None)
            .unwrap();
        assert!(next.is_none());
        assert_eq!(records.len(), 8);
        let page = state
            .finish_native_history_page(ticket, records, next, pairing::now_ms())
            .unwrap();
        assert_eq!(
            page.records
                .iter()
                .filter(|r| visible.contains(&r.id.0))
                .count(),
            1
        );
        for record in page.records {
            visible.insert(record.id.0);
        }
        for n in 0..1005 {
            assert!(visible.contains(&format!("omp-tool:call-{n}")));
        }
        assert!(visible.contains("one-entry"));
        assert!(visible.contains("transient"));
        assert_eq!(visible.len(), 1007);
        let NativePageLookup::Cached(retry) = state
            .native_history_page(&p, &target, &opaque, 100, pairing::now_ms())
            .unwrap()
        else {
            panic!("read-only repeated page must be stable")
        };
        assert_eq!(retry.records.len(), 8);
        let another = state
            .native_history_cursor(
                &p,
                &target,
                &epoch,
                Id("omp-file:1:7".into()),
                None,
                pairing::now_ms(),
            )
            .unwrap();
        let NativePageLookup::Fetch(late) = state
            .native_history_page(&p, &target, &another, 100, pairing::now_ms())
            .unwrap()
        else {
            panic!("fresh continuation required")
        };
        let replacement = state.begin_source_install(&target).unwrap();
        assert_eq!(
            state.finish_native_history_page(late, vec![], None, pairing::now_ms()),
            Err(ErrorCode::CursorExpired)
        );
        assert!(
            history
                .page(history.len(), 100, &state, &target, &epoch, None)
                .is_err()
        );
        assert!(state.abort_install(&target, &epoch).is_err());
        state.abort_install(&target, &replacement).unwrap();
    }
    #[test]
    fn relay_queue_frame_and_byte_overflow_close_without_dropping_and_continuing() {
        for (frames, size, accepted) in [(33, 1, 32), (3, FRAME_BYTES, 2)] {
            let (tx, mut rx) = mpsc::channel(QUEUE_FRAMES);
            let (close, closed) = watch::channel(None);
            let peer = RelayPeer {
                id: Uuid::random(),
                tx,
                budget: Arc::new(Semaphore::new(QUEUE_BYTES)),
                close,
            };
            let payload: axum::body::Bytes = vec![5; size].into();
            for _ in 0..frames {
                peer.send(RelayMessage::Binary(payload.clone()));
            }
            assert_eq!(*closed.borrow(), Some(4001));
            let mut count = 0;
            while let Ok(frame) = rx.try_recv() {
                assert_eq!(frame.message.into_data(), payload);
                count += 1;
            }
            assert_eq!(count, accepted);
            assert_eq!(peer.budget.available_permits(), QUEUE_BYTES);
        }
    }
    #[tokio::test]
    async fn actual_websocket_fd_shutdown_invalidates_all_write_clones() {
        let relay = Relay::bind(0).await.unwrap();
        let port = relay.port();
        let (stop, rx) = watch::channel(false);
        let job = tokio::spawn(relay.run(rx));
        let (mut socket, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{port}/r/abcdefghij?role=host"
        ))
        .await
        .unwrap();
        let fd = socket_fd(&socket).unwrap();
        let duplicate = fd.try_clone().unwrap();
        // SAFETY: both descriptors refer to the actual established native stream.
        assert_eq!(
            unsafe { libc::shutdown(duplicate.as_raw_fd(), libc::SHUT_RDWR) },
            0
        );
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            socket.send(Message::Binary(vec![0; 32].into())).await?;
            socket.flush().await
        })
        .await
        .unwrap();
        assert!(result.is_err());
        stop.send(true).unwrap();
        job.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn periodic_registry_deadline_is_typed_and_distinct_from_malformed_source() {
        let root = Root::new();
        let endpoint = root.0.join("r.sock");
        let listener = tokio::net::UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.0.join("entry.json"))
            .unwrap();
        writeln!(file,"{}",json!({"version":1,"instanceId":"instance-a","pid":std::process::id(),"endpoint":endpoint,"createdAt":0,"token":"private-token"})).unwrap();
        drop(file);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = Vec::new();
            reader.read_until(b'\n', &mut request).await.unwrap();
            // The original 1500-ms query expires while this same socket stays open.
            tokio::time::sleep(Duration::from_millis(1700)).await;
            drop(reader);
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.write_all(b"{}\n").await.unwrap();
        });
        let entries = registry(&root.0).unwrap();
        assert!(matches!(
            entries[0].snapshot_checked().await,
            Err(RegistryFailure::TimedOut)
        ));
        assert!(matches!(
            entries[0].snapshot_checked().await,
            Err(RegistryFailure::Other(_))
        ));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn registry_control_link_accepts_busy_change_but_rejects_identity_race() {
        for (generation_after, cwd_after, accepted) in [
            (1, "/", true),
            (2, "/", false),
            (1, "/different-project", false),
        ] {
            let root = Root::new();
            let endpoint = root.0.join("r.sock");
            let listener = tokio::net::UnixListener::bind(&endpoint).unwrap();
            fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
            let metadata = root.0.join("entry.json");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&metadata)
                .unwrap();
            writeln!(file,"{}",json!({"version":1,"instanceId":"instance-a","pid":std::process::id(),"endpoint":endpoint,"createdAt":0,"token":"private-token"})).unwrap();
            drop(file);
            let token = URL_SAFE_NO_PAD.encode([8; 48]);
            let link = format!("abcdefghij.{token}");
            let server = tokio::spawn(async move {
                for step in 0..3 {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = Vec::new();
                    reader.read_until(b'\n', &mut request).await.unwrap();
                    let request = native_json::parse(&request, REGISTRY).unwrap();
                    assert_eq!(request["token"], "private-token");
                    let response = if step == 1 {
                        assert_eq!(request["generation"], 1);
                        json!({"v":1,"ok":true,"url":link})
                    } else {
                        json!({"v":1,"ok":true,"snapshot":{"instanceId":"instance-a","generation":if step==0{1}else{generation_after},"pid":std::process::id(),"sessionId":"native","cwd":if step==0{"/"}else{cwd_after},"relayConnected":true,"inputRequired":false,"busy":step==0,"access":"control"}})
                    };
                    reader
                        .get_mut()
                        .write_all(format!("{response}\n").as_bytes())
                        .await
                        .unwrap();
                }
            });
            let entries = registry(&root.0).unwrap();
            let (snapshot, owner) = entries[0].snapshot().await.unwrap();
            let resolved = entries[0].link(&snapshot, &owner).await;
            assert_eq!(resolved.is_ok(), accepted);
            server.await.unwrap();
        }
    }

    // Real private Herdr/registry sockets, kernel owner evidence and authenticated
    // welcome/FINAL exercise the consumer. Synthetic native data is not stock runtime proof.
    struct NativeFixture {
        root: Root,
        runtime: Arc<Runtime>,
        auth: Arc<Authorized>,
        host_endpoint: iroh::Endpoint,
        phone_endpoint: iroh::Endpoint,
        phone_connection: iroh::endpoint::Connection,
        host_socket: NativeSocket,
        connection: Arc<Connection>,
        snapshot: watch::Sender<Value>,
        snapshot_gate: Arc<Semaphore>,
        events: watch::Sender<Option<Value>>,
        stop: watch::Sender<bool>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
        database: PathBuf,
    }
    async fn fixture_until<T>(mut observe: impl FnMut() -> Option<T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(value) = observe() {
                    return value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("owned fixture state transition did not complete")
    }
    async fn fixture_frame(socket: &mut NativeSocket) -> Message {
        tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }
    fn subscribed_topology_server(
        listener: tokio::net::UnixListener,
        view: watch::Sender<Value>,
        snapshots: Arc<Semaphore>,
        events_rx: watch::Receiver<Option<Value>>,
    ) -> tokio::task::JoinHandle<()> {
        // SAFETY: getpgrp has no pointer or lifetime preconditions.
        let process_group = unsafe { libc::getpgrp() };
        tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let view = view.clone();
                let snapshots = snapshots.clone();
                let mut events = events_rx.clone();
                clients.spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let request: Value = serde_json::from_slice(&herdr::read_line(&mut reader).await.unwrap()).unwrap();
                    let result = match request["method"].as_str().unwrap() {
                        "events.subscribe" => json!({"type":"subscription_started"}),
                        "session.snapshot" => {
                            let _gate = snapshots.acquire().await.unwrap();
                            json!({"type":"session_snapshot","snapshot":view.borrow().clone()})
                        },
                        "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                            "pane_id":request["params"]["pane_id"],"shell_pid":std::process::id(),
                            "foreground_process_group_id":process_group,
                            "foreground_processes":[{"pid":std::process::id(),"name":"owned-native-fixture"}]
                        }}),
                        _ => panic!("fixture received an unauthorized public effect"),
                    };
                    reader.get_mut().write_all(format!("{}\n", json!({"id":request["id"],"result":result})).as_bytes()).await.unwrap();
                    if request["method"] == "events.subscribe" {
                        while events.changed().await.is_ok() {
                            let event = events.borrow_and_update().clone();
                            if let Some(event) = event
                                && reader.get_mut().write_all(format!("{event}\n").as_bytes()).await.is_err()
                            {
                                break;
                            }
                        }
                    }
                });
            }
        })
    }
    fn fixture_inventory(state: &State, connection: &Uuid) -> HostSnapshotResult {
        let request = Uuid::random();
        state.host_snapshot(connection, request.clone()).unwrap();
        loop {
            let (Some(bytes), closed) = state.pop_output(connection).unwrap() else {
                panic!("host.snapshot response required")
            };
            assert!(!closed);
            if let Envelope::Response(response) =
                decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
                && response.id == request
            {
                let Ok(ResultBody::HostSnapshot(snapshot)) = response.outcome else {
                    panic!("inventory required")
                };
                return snapshot;
            }
        }
    }
    impl NativeFixture {
        async fn new(entries: Option<Vec<Value>>) -> Self {
            let root = Root::new();
            let herdr_socket = root.0.join("h.sock");
            let registry_dir = root.0.join("registry");
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&registry_dir)
                .unwrap();
            let registry_socket = registry_dir.join("r.sock");
            let history_path = root.0.join("native.jsonl");
            if let Some(entries) = entries {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&history_path)
                    .unwrap();
                writeln!(file, "{}", json!({"type":"session","id":"native","timestamp":"2026-10-07T00:00:00.000Z","cwd":"/not-identity"})).unwrap();
                for entry in &entries {
                    writeln!(file, "{entry}").unwrap();
                }
            }
            let executable = root.herdr_executable(&herdr_socket);
            let config = Config::parse(&format!(
                "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\nomp_registry_dir={registry_dir:?}\nomp_session_roots=[{:?}]\n[executables]\nherdr={executable:?}\n",
                root.0, root.0, root.0
            )).unwrap();
            let (snapshot, _) = watch::channel(json!({
                "version":"0.9.1","protocol":22,"focused_workspace_id":"desktop",
                "workspaces":[{"workspace_id":"desktop","number":1,"label":"Desktop","focused":true,"active_tab_id":"","agent_status":"unknown"},{"workspace_id":"native-workspace","number":2,"label":"Native workspace","focused":false,"active_tab_id":"tab","agent_status":"idle"}],
                "tabs":[{"tab_id":"tab","workspace_id":"native-workspace","number":1,"label":"Native tab","focused":false,"agent_status":"idle"}],
                "panes":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"native-workspace","tab_id":"tab","focused":false,"agent_status":"idle","agent":"omp"}],
                "agents":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"native-workspace","tab_id":"tab","agent":"omp","agent_session":{"source":"fixture","agent":"omp","kind":"path","value":history_path}}]
            }));
            let (events, events_rx) = watch::channel(None::<Value>);
            let listener = tokio::net::UnixListener::bind(&herdr_socket).unwrap();
            fs::set_permissions(&herdr_socket, fs::Permissions::from_mode(0o600)).unwrap();
            let snapshot_gate = Arc::new(Semaphore::new(1));
            let herdr_task = subscribed_topology_server(
                listener,
                snapshot.clone(),
                snapshot_gate.clone(),
                events_rx,
            );
            let relay = Relay::bind(0).await.unwrap();
            let port = relay.port();
            let (stop, rx) = watch::channel(false);
            let relay_task = tokio::spawn(async move { relay.run(rx).await.unwrap() });
            let secret = URL_SAFE_NO_PAD.encode([19; 48]);
            let link_text = format!("ws://127.0.0.1:{port}/r/abcdefghij#{secret}");
            let link = Link::parse(&link_text).unwrap();
            let (mut host_socket, _) = tokio_tungstenite::connect_async(format!(
                "ws://127.0.0.1:{port}/r/abcdefghij?role=host"
            ))
            .await
            .unwrap();
            let listener = tokio::net::UnixListener::bind(&registry_socket).unwrap();
            fs::set_permissions(&registry_socket, fs::Permissions::from_mode(0o600)).unwrap();
            let mut metadata = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(registry_dir.join("owner.json"))
                .unwrap();
            writeln!(metadata, "{}", json!({"version":1,"instanceId":"owned-instance","pid":std::process::id(),"endpoint":registry_socket,"createdAt":0,"token":"synthetic-registry-token"})).unwrap();
            drop(metadata);
            let registry_task = tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let request =
                        native_json::parse(&herdr::read_line(&mut reader).await.unwrap(), REGISTRY)
                            .unwrap();
                    assert_eq!(request["token"], "synthetic-registry-token");
                    let result = match request["op"].as_str().unwrap() {
                        "snapshot" => json!({"v":1,"ok":true,"snapshot":{
                            "instanceId":"owned-instance","generation":1,"pid":std::process::id(),
                            "sessionId":"native","cwd":"/not-identity","relayConnected":true,"inputRequired":false,
                            "busy":false,"access":"control"
                        }}),
                        "link" => {
                            assert_eq!(request["generation"], 1);
                            json!({"v":1,"ok":true,"url":link_text})
                        }
                        _ => panic!("fixture registry operation is not read-only"),
                    };
                    reader
                        .get_mut()
                        .write_all(format!("{result}\n").as_bytes())
                        .await
                        .unwrap();
                }
            });
            let host_endpoint = crate::endpoint_builder(iroh::SecretKey::from_bytes(&[91; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .bind()
                .await
                .unwrap();
            let phone_endpoint = crate::endpoint_builder(iroh::SecretKey::from_bytes(&[92; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .bind()
                .await
                .unwrap();
            let database = root.0.join("state.sqlite3");
            let mut storage =
                Storage::open(&database, [23; 32], pairing::now_ms(), pairing::uid()).unwrap();
            storage
                .pair_device(&PairedDevice {
                    device_id: Uuid::random(),
                    endpoint_id: EndpointId(phone_endpoint.id().to_string()),
                    device_name: Label::new("owned regression phone").unwrap(),
                    paired_at: Uint(pairing::now_ms()),
                    last_seen_at: None,
                })
                .unwrap();
            let boot = storage.boot_id.clone();
            let security = Security::new(
                crate::storage::StorageOwner::new(storage).unwrap(),
                Arc::new(State::default()),
                EndpointId(host_endpoint.id().to_string()),
                boot,
                vec![],
            );
            let endpoint = host_endpoint.clone();
            let incoming =
                tokio::spawn(async move { endpoint.accept().await.unwrap().await.unwrap() });
            let phone_connection = phone_endpoint
                .connect(host_endpoint.addr(), crate::ALPN)
                .await
                .unwrap();
            let server = incoming.await.unwrap();
            let auth = Arc::new(
                security
                    .hello(
                        EndpointId(server.remote_id().to_string()),
                        HelloParams {
                            app_version: "regression".into(),
                            device_name: Label::new("owned phone").unwrap(),
                        },
                        server,
                    )
                    .await
                    .unwrap(),
            );
            let topology = Topology::new(
                config,
                security.storage.clone(),
                security.state.clone(),
                security.host.clone(),
            )
            .await
            .unwrap();
            let topology_owner = topology.clone();
            let rx = stop.subscribe();
            let topology_task = tokio::spawn(async move { topology_owner.run(rx).await.unwrap() });
            let source = fixture_until(|| topology.sources().into_iter().next()).await;
            let runtime = Runtime::new(topology, security, None);
            let native_owner = runtime.clone();
            let native_source = source.clone();
            let agent = source.snapshot.agents[0].clone();
            let rx = stop.subscribe();
            let follower =
                tokio::spawn(async move { native_owner.follow(native_source, agent, rx).await });
            assert_eq!(
                fixture_frame(&mut host_socket).await.to_text().unwrap(),
                "{\"t\":\"peer-joined\",\"peer\":1}"
            );
            let Message::Binary(hello) = fixture_frame(&mut host_socket).await else {
                panic!("authenticated native hello required")
            };
            assert_eq!(
                native_json::parse(link.open(&mut hello.to_vec()).unwrap(), NATIVE).unwrap()["t"],
                "hello"
            );
            for value in [
                json!({"t":"welcome","proto":3,"header":{"type":"session","id":"native","cwd":"/not-identity"},"entryCount":0,"state":{"busy":false}}),
                json!({"t":"snapshot-chunk","entries":[],"final":true}),
            ] {
                let mut bytes = link.seal_value(&value).unwrap();
                bytes[..4].copy_from_slice(&1u32.to_be_bytes());
                host_socket
                    .send(Message::Binary(bytes.into()))
                    .await
                    .unwrap();
            }
            let connection =
                fixture_until(|| runtime.connections.lock().values().next().cloned()).await;
            assert_eq!(
                runtime
                    .security
                    .state
                    .session_state(&connection.target, &connection.epoch)
                    .unwrap()
                    .summary
                    .connectivity,
                SessionConnectivity::Ready
            );
            // Freeze periodic reads only after genuine native installation, retaining the actual
            // connected writer and captured Source for deterministic consumer calls below.
            follower.abort();
            let _ = follower.await;
            Self {
                root,
                runtime,
                auth,
                host_endpoint,
                phone_endpoint,
                phone_connection,
                host_socket,
                connection,
                snapshot,
                snapshot_gate,
                events,
                stop,
                tasks: vec![herdr_task, registry_task, topology_task, relay_task],
                database,
            }
        }
        fn open(&self) -> SessionOpenResult {
            let request = Uuid::random();
            let state = &self.runtime.security.state;
            self.runtime
                .session_open(&self.auth, request.clone(), &self.connection.target)
                .unwrap();
            loop {
                let (Some(bytes), closed) = state.pop_output(&self.auth.connection_id).unwrap()
                else {
                    panic!("session.open response required")
                };
                assert!(!closed, "fixture consumer unexpectedly disconnected");
                if let Envelope::Response(response) =
                    decode_payload(&bytes, Some(Method::SessionOpen)).unwrap()
                    && response.id == request
                {
                    let Ok(ResultBody::SessionOpen(open)) = response.outcome else {
                        panic!("session.open result required")
                    };
                    return open;
                }
            }
        }
        fn inventory(&self) -> HostSnapshotResult {
            fixture_inventory(&self.runtime.security.state, &self.auth.connection_id)
        }
        async fn history(&self) -> SessionHistoryResult {
            self.runtime
                .history(
                    &self.auth.proof,
                    &SessionHistoryParams {
                        target: self.connection.target.clone(),
                        cursor: None,
                        view_id: None,
                        limit: Some(Uint(100)),
                    },
                )
                .await
                .unwrap()
        }
        fn prompt(&self, text: &str) -> Request {
            Request {
                id: Uuid::random(),
                body: RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: self.runtime.security.boot.clone(),
                target: self.connection.target.clone(),
                text: PromptText::new(text).unwrap(), }),
            }
        }
        async fn refresh(&self, event: &str, snapshot: Value) -> Source {
            let previous = self
                .runtime
                .topology
                .source(&self.connection.source.server_id)
                .unwrap();
            self.snapshot.send_replace(snapshot.clone());
            self.events
                .send_replace(Some(json!({"event":event,"data":{}})));
            fixture_until(|| {
                self.runtime
                    .topology
                    .source(&self.connection.source.server_id)
                    .ok()
                    .filter(|source| {
                        !Arc::ptr_eq(&source.snapshot, &previous.snapshot)
                            && source
                                .snapshot
                                .focused_workspace_id
                                .as_ref()
                                .map(|id| id.0.as_str())
                                == snapshot["focused_workspace_id"].as_str()
                            && source.snapshot.agents[0]
                                .agent_session
                                .as_ref()
                                .map(|report| report.value.as_str())
                                == snapshot["agents"][0]["agent_session"]["value"].as_str()
                    })
            })
            .await
        }
        async fn close(self) {
            self.stop.send_replace(true);
            for task in self.tasks {
                task.abort();
                let _ = task.await;
            }
            self.connection.shutdown();
            drop(self.host_socket);
            drop(self.phone_connection);
            self.runtime.security.shutdown().await.unwrap();
            crate::close_endpoint(&self.phone_endpoint).await.unwrap();
            crate::close_endpoint(&self.host_endpoint).await.unwrap();
        }
    }

    #[tokio::test]
    async fn codex_skill_selector_never_becomes_an_omp_text_prompt_or_native_readiness_wait() {
        let mut fixture=NativeFixture::new(None).await;
        let mut request=fixture.prompt("");
        let RequestBody::PromptSubmit(params)=&mut request.body else {unreachable!()};
        params.skill=Some(SkillSelector{catalogue_generation:Uuid::random(),reference:Uuid::random()});
        let action=request.id.clone();
        assert_eq!(fixture.runtime.clone().mutate(fixture.auth.clone(),request).await,Err(ErrorCode::NotReady));
        assert!(fixture.runtime.jobs.lock().is_empty());
        assert!(receipt(&fixture.runtime.security,&fixture.auth,&action).await.is_err());
        assert!(tokio::time::timeout(Duration::from_millis(100),fixture.host_socket.next()).await.is_err());
        fixture.close().await;
    }

    #[tokio::test]
    async fn readiness_rejections_preserve_report_precedence_and_registry_aggregation() {
        let fixture = NativeFixture::new(None).await;
        let source = fixture.connection.source.clone();
        let agent = source.snapshot.agents[0].clone();
        for (report, expected) in [
            (None, "Herdr native session report is absent"),
            (
                Some(("codex", "id")),
                "Herdr native session report agent is not OMP",
            ),
            (
                Some(("codex", "path")),
                "Herdr native session report agent is not OMP",
            ),
            (
                Some(("omp", "id")),
                "Herdr OMP native session report is not a path",
            ),
        ] {
            let mut candidate = agent.clone();
            candidate.agent_session = report.map(|(provider, kind)| {
                let mut native = agent.agent_session.clone().unwrap();
                native.agent = provider.into();
                native.kind = kind.into();
                native
            });
            assert_eq!(
                fixture
                    .runtime
                    .clone()
                    .follow(source.clone(), candidate, fixture.stop.subscribe())
                    .await,
                Err((UnboundState::SetupRequired, expected))
            );
        }

        // Keep genuine registry snapshots, but report no matching foreground owner.
        let socket = fixture.root.0.join("unjoined.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let process_server = tokio::spawn(async move {
            for _ in 0..4 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let request: Value =
                    serde_json::from_slice(&herdr::read_line(&mut reader).await.unwrap()).unwrap();
                assert_eq!(request["method"], "pane.process_info");
                let response = json!({"id":request["id"],"result":{
                    "type":"pane_process_info","process_info":{
                        "pane_id":"pane","foreground_processes":[]
                    }
                }});
                reader
                    .get_mut()
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let mut unjoined = source;
        unjoined.socket_path = socket;
        let registry_dir = fixture.root.0.join("registry");
        let owner_file = registry_dir.join("owner.json");
        let saved_owner = registry_dir.join("owner.saved");
        let failed_file = registry_dir.join("failed.json");
        for (step, expected) in [
            "Registry native ID and exact kernel terminal/owner namespace ancestry join could not be proved",
            "Stock collaboration registry snapshots and terminal/owner joins are unavailable",
            "Stock collaboration registry snapshots are unavailable",
            "Stock collaboration registry has no candidates",
        ]
        .into_iter()
        .enumerate()
        {
            match step {
                1 => {
                    // A valid private publication with an absent endpoint fails snapshot only.
                    let mut publication: Value =
                        serde_json::from_slice(&fs::read(&owner_file).unwrap()).unwrap();
                    publication["endpoint"] = json!(registry_dir.join("absent.sock"));
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&failed_file)
                        .unwrap();
                    writeln!(file, "{publication}").unwrap();
                }
                2 => fs::rename(&owner_file, &saved_owner).unwrap(),
                3 => fs::remove_file(&failed_file).unwrap(),
                _ => {}
            }
            assert_eq!(
                fixture
                    .runtime
                    .clone()
                    .follow(unjoined.clone(), agent.clone(), fixture.stop.subscribe())
                    .await,
                Err((UnboundState::SetupRequired, expected))
            );
        }
        process_server.await.unwrap();
        fixture.close().await;
    }

    #[tokio::test]
    async fn zero_turn_native_authority_materializes_history_without_rebinding() {
        // Exercise Runtime.follow, private registry IPC, authenticated welcome/FINAL
        // and the real history consumer; this is not stock-provider runtime proof.
        let fixture = NativeFixture::new(None).await;
        let target = fixture.connection.target.clone();
        let epoch = fixture.connection.epoch.clone();
        let path = fixture.root.0.join("native.jsonl");
        assert!(!path.exists());
        let open = fixture.open();
        assert!(open.history.is_empty());
        assert_eq!(open.state.summary.native_id.0, "native");
        assert_eq!(
            open.state.summary.history_coverage.source,
            HistorySource::LiveOnly
        );
        assert!(!open.state.summary.history_coverage.older_available);
        let page = fixture.history().await;
        assert!(page.records.is_empty());
        assert!(page.next_cursor.is_none());
        assert!(fixture.connection.history.lock().header.is_null());

        fixture
            .runtime
            .apply_frame(
                &fixture.connection,
                json!({
                    "t":"entry","entry":{"type":"message","id":"entry-0","parentId":null,
                        "message":{"role":"assistant","content":"live first message"}}
                }),
            )
            .unwrap();
        assert!(
            fixture.history().await.records.is_empty(),
            "live-only rows cannot supply native browse proof"
        );
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        assert_eq!(
            fixture
                .connection
                .history
                .lock()
                .recheck()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        writeln!(
            file,
            "{}",
            json!({"type":"session","id":"native","cwd":"/not-identity"})
        )
        .unwrap();
        for n in 0..1005 {
            writeln!(
                file,
                "{}",
                json!({"type":"message","id":format!("entry-{n}"),
                "parentId":if n == 0 { None } else { Some(format!("entry-{}", n - 1)) },
                "message":{"role":"assistant","content":format!("persisted message {n}")}})
            )
            .unwrap();
        }
        drop(file);
        let mut ids = HashSet::new();
        let mut page = fixture.history().await;
        loop {
            let page_ids: HashSet<_> = page
                .records
                .iter()
                .map(|record| record.id.clone())
                .collect();
            assert_eq!(
                page_ids.len(),
                page.records.len(),
                "native records duplicated within a page"
            );
            ids.extend(page_ids);
            let Some(cursor) = page.next_cursor else {
                break;
            };
            page = fixture
                .runtime
                .history(
                    &fixture.auth.proof,
                    &SessionHistoryParams {
                        target: target.clone(),
                        cursor: Some(cursor),
                        view_id: Some(page.view_id.clone()),
                        limit: Some(Uint(100)),
                    },
                )
                .await
                .unwrap();
        }
        assert_eq!(ids.len(), 1005);
        assert!(ids.contains(&Id("entry-0".into())));
        assert!(ids.contains(&Id("entry-5".into())));
        assert!(ids.contains(&Id("entry-1004".into())));
        let open = fixture.open();
        assert_eq!(
            open.state.summary.history_coverage.source,
            HistorySource::PersistedAndLive
        );
        assert_eq!(open.state.summary.native_id.0, "native");
        assert_eq!(fixture.connection.target, target);
        assert_eq!(
            fixture
                .runtime
                .security
                .state
                .source_epoch(&target)
                .unwrap(),
            epoch
        );
        assert_eq!(fixture.runtime.connections.lock().len(), 1);
        assert_eq!(fixture.connection.history.lock().header["id"], "native");

        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"message","id":"entry-1005","parentId":"entry-1004",
            "message":{"role":"assistant","content":"later native append"}})
        )
        .unwrap();
        drop(file);
        let page = fixture.history().await;
        assert_eq!(page.records.last().unwrap().id.0, "entry-1005");
        assert_eq!(
            fixture
                .runtime
                .security
                .state
                .source_epoch(&target)
                .unwrap(),
            epoch
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            fixture
                .runtime
                .history(
                    &fixture.auth.proof,
                    &SessionHistoryParams {
                        target: target.clone(),
                        cursor: None,
                        view_id: None,
                        limit: None,
                    }
                )
                .await,
            Err(ErrorCode::StaleSession)
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement = fixture.root.0.join("replaced.jsonl");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&replacement)
            .unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"session","id":"different-native","cwd":"/not-identity"})
        )
        .unwrap();
        drop(file);
        fs::rename(replacement, &path).unwrap();
        assert_eq!(
            fixture
                .runtime
                .history(
                    &fixture.auth.proof,
                    &SessionHistoryParams {
                        target,
                        cursor: None,
                        view_id: None,
                        limit: None,
                    }
                )
                .await,
            Err(ErrorCode::StaleSession)
        );
        {
            let history = fixture.connection.history.lock();
            assert_eq!(history.entry(0), Err(ErrorCode::NotReady));
            fs::remove_file(&path).unwrap();
            assert_eq!(history.entry(0), Err(ErrorCode::NotReady));
            assert_eq!(history.entry(history.len()), Err(ErrorCode::CursorExpired));
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn subscribed_shell_node_overflow_invalidates_display_and_fences_late_capture() {
        let root = Root::new();
        let socket = root.0.join("h.sock");
        let executable = root.herdr_executable(&socket);
        let config = Config::parse(&format!(
            "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\n[executables]\nherdr={executable:?}\n",
            root.0, root.0
        ))
        .unwrap();
        let (snapshot, _) = watch::channel(json!({
            "version":"0.9.1","protocol":22,"focused_workspace_id":"shell-workspace",
            "workspaces":[{"workspace_id":"shell-workspace","number":1,"label":"Shell only",
                "focused":true,"active_tab_id":"shell-tab","agent_status":"unknown"}],
            "tabs":[{"tab_id":"shell-tab","workspace_id":"shell-workspace","number":1,
                "label":"Shell tab","focused":true,"agent_status":"unknown"}],
            "panes":(0..512).map(|n| json!({
                "pane_id":format!("shell-pane-{n}"),"terminal_id":format!("shell-terminal-{n}"),
                "workspace_id":"shell-workspace","tab_id":"shell-tab",
                "focused":n==0,"agent_status":"unknown"
            })).collect::<Vec<_>>(),
            "agents":[]
        }));
        let (events, events_rx) = watch::channel(None::<Value>);
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let snapshot_gate = Arc::new(Semaphore::new(1));
        let server = subscribed_topology_server(
            listener,
            snapshot.clone(),
            snapshot_gate.clone(),
            events_rx,
        );
        let storage = Arc::new(
            crate::storage::StorageOwner::new(Storage::memory([13; 32], 0).unwrap()).unwrap(),
        );
        let state = Arc::new(State::default());
        let connection = state.connect(&peer());
        let host = EndpointId(iroh::SecretKey::from_bytes(&[13; 32]).public().to_string());
        let topology = Topology::new(config, storage.clone(), state.clone(), host)
            .await
            .unwrap();
        let (stop, rx) = watch::channel(false);
        let owner = topology.clone();
        let job = tokio::spawn(async move { owner.run(rx).await.unwrap() });
        let source = fixture_until(|| topology.sources().into_iter().next()).await;
        let gate = snapshot_gate.acquire().await.unwrap();
        // Installing Source precedes awaited shell enrollment and State publication.
        // Hold the original snapshot gate, but wait for this exact published overflow.
        let captured = fixture_until(|| {
            let inventory=fixture_inventory(&state,&connection);
            inventory.hierarchy.iter().any(|graph|graph.server_id==source.server_id
                &&graph.server_incarnation==source.incarnation
                &&graph.reason==Some(NativeUnavailableReason::TooLarge)).then_some(inventory)
        }).await;

        assert!(captured.sessions.is_empty() && captured.unbound.is_empty());
        assert!(captured.projects.is_empty());
        assert!(source.snapshot.agents.is_empty());
        assert_eq!(source.snapshot.panes.len(), 512);
        assert_eq!(
            source.snapshot.workspaces.len()
                + source.snapshot.tabs.len()
                + source.snapshot.panes.len(),
            514
        );
        assert_eq!(captured.servers[0].connectivity, ServerConnectivity::Online);
        assert_eq!(
            captured.hierarchy[0].freshness,
            NativeFreshness::Unavailable
        );
        assert_eq!(
            captured.hierarchy[0].reason,
            Some(NativeUnavailableReason::TooLarge)
        );
        assert!(
            captured.hierarchy[0].workspaces.is_empty()
                && captured.hierarchy[0].tabs.is_empty()
                && captured.hierarchy[0].panes.is_empty()
        );
        source.recheck().unwrap();
        snapshot.send_modify(|view| {
            view["workspaces"][0]["label"] = Value::from("After native event");
        });
        events.send_replace(Some(json!({"event":"workspace.focused","data":{}})));
        fixture_until(|| source.recheck().is_err().then_some(())).await;
        let changed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (bytes, closed) = state.pop_output(&connection).unwrap();
                assert!(
                    !closed,
                    "dirty shell inventory must not disconnect the host"
                );
                if let Some(bytes) = bytes {
                    let Envelope::Event(event) = decode_payload(&bytes, None).unwrap() else {
                        panic!("subscribed host event required")
                    };
                    break event;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("native dirty event must publish host.changed even without sessions");
        assert_eq!(changed.subscription, captured.subscription);
        assert_eq!(changed.seq, Uint(1));
        assert!(matches!(changed.body, EventBody::HostChanged(_)));
        let dirty = fixture_inventory(&state, &connection);
        assert!(dirty.sessions.is_empty() && dirty.unbound.is_empty());
        assert_eq!(dirty.hierarchy[0].freshness, NativeFreshness::Unavailable);
        assert_eq!(
            dirty.hierarchy[0].reason,
            Some(NativeUnavailableReason::SourceUnavailable)
        );
        assert!(
            dirty.hierarchy[0].workspaces.is_empty()
                && dirty.hierarchy[0].tabs.is_empty()
                && dirty.hierarchy[0].panes.is_empty()
        );

        state
            .replace_inventory(
                captured.servers,
                captured.projects,
                vec![],
                captured.unbound,
                captured.hierarchy,
                &[],
                |graphs| source.fence_inventory(&mut graphs[0]),
            )
            .unwrap();
        let late = fixture_inventory(&state, &connection);
        assert_eq!(late.hierarchy[0].freshness, NativeFreshness::Unavailable);
        assert_eq!(
            late.hierarchy[0].reason,
            Some(NativeUnavailableReason::SourceUnavailable),
            "a pre-event display overflow cannot restore current-source eligibility"
        );
        assert!(
            late.hierarchy[0].workspaces.is_empty()
                && late.hierarchy[0].tabs.is_empty()
                && late.hierarchy[0].panes.is_empty()
        );
        assert!(source.recheck().is_err());

        drop(gate);
        let recovered = fixture_until(|| {
            topology.source(&source.server_id).ok().filter(|current| {
                current.snapshot.workspaces[0].label == "After native event"
                    && !Arc::ptr_eq(&current.snapshot, &source.snapshot)
            })
        })
        .await;
        recovered.recheck().unwrap();
        // A replacement Source is also installed before its recovered State display.
        let current = fixture_until(|| {
            let inventory=fixture_inventory(&state,&connection);
            inventory.hierarchy.iter().any(|graph|graph.server_id==recovered.server_id
                &&graph.server_incarnation==recovered.incarnation
                &&graph.reason==Some(NativeUnavailableReason::TooLarge)).then_some(inventory)
        }).await;

        assert!(current.sessions.is_empty() && current.unbound.is_empty());
        assert_eq!(current.servers[0].connectivity, ServerConnectivity::Online);
        assert_eq!(current.hierarchy[0].freshness, NativeFreshness::Unavailable);
        assert_eq!(
            current.hierarchy[0].reason,
            Some(NativeUnavailableReason::TooLarge)
        );
        assert_eq!(current.hierarchy[0].server_incarnation, source.incarnation);
        assert!(source.recheck().is_err());
        stop.send_replace(true);
        job.await.unwrap();
        server.abort();
        let _ = server.await;
        drop(topology);
        Arc::try_unwrap(storage)
            .ok()
            .unwrap()
            .close()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn late_inventory_capture_cannot_reattach_disconnected_provider() {
        let fixture = NativeFixture::new(Some(vec![])).await;
        let state = &fixture.runtime.security.state;
        let target = &fixture.connection.target;
        let epoch = &fixture.connection.epoch;
        let source = fixture
            .runtime
            .topology
            .source(&fixture.connection.key.server_id)
            .unwrap();
        let _gate = fixture.snapshot_gate.acquire().await.unwrap();
        let captured = fixture.inventory();
        assert_eq!(captured.hierarchy[0].freshness, NativeFreshness::Current);
        assert_eq!(captured.hierarchy[0].panes[0].target.as_ref(), Some(target));
        state.validate_source(target, epoch).unwrap();
        source.recheck().unwrap();

        state
            .binding_offline(target, epoch, "native provider source disconnected")
            .unwrap();
        assert_ne!(state.source_epoch(target).unwrap(), *epoch);
        source.recheck().unwrap();
        let disconnected = fixture.inventory();
        assert_eq!(
            disconnected.sessions[0].connectivity,
            SessionConnectivity::Offline
        );
        assert!(disconnected.hierarchy[0].panes[0].target.is_none());

        state
            .replace_inventory(
                captured.servers,
                captured.projects,
                vec![],
                captured.unbound,
                captured.hierarchy,
                &[(target, epoch)],
                |graphs| source.fence_inventory(&mut graphs[0]),
            )
            .unwrap();
        let committed = fixture.inventory();
        assert_eq!(
            committed.sessions[0].connectivity,
            SessionConnectivity::Offline
        );
        assert_eq!(committed.hierarchy[0].freshness, NativeFreshness::Current);
        assert!(committed.hierarchy[0].panes[0].target.is_none());
        assert!(committed.hierarchy[0].panes[0].unbound_row_id.is_none());
        drop(_gate);
        fixture.close().await;
    }

    #[tokio::test]
    async fn dirty_topology_gates_mutation_without_rotating_or_disconnect() {
        let fixture = NativeFixture::new(Some(vec![])).await;
        let state = &fixture.runtime.security.state;
        let target = &fixture.connection.target;
        let epoch = &fixture.connection.epoch;
        fixture.runtime.apply_frame(&fixture.connection, json!({
            "t":"ui-request","request":{"reqId":73,"kind":"editor","title":"retained","prefill":"draft"}
        })).unwrap();
        let dialog = fixture.open().pending_requests[0].request_key.clone();
        let content = state
            .content(
                target,
                Mime::TextPlain,
                "unchanged".repeat(3000),
                pairing::now_ms(),
            )
            .unwrap();
        let Content::Ref(content) = content else {
            panic!("large shared content required")
        };
        let previous = fixture
            .runtime
            .topology
            .source(&fixture.connection.key.server_id)
            .unwrap();
        let captured = fixture.inventory();
        assert_eq!(captured.hierarchy[0].panes[0].target.as_ref(), Some(target));
        let gate = fixture.snapshot_gate.acquire().await.unwrap();
        fixture
            .events
            .send_replace(Some(json!({"event":"workspace.focused","data":{}})));
        fixture_until(|| {
            (previous.recheck().is_err()
                && state
                    .session_state(target, epoch)
                    .unwrap()
                    .summary
                    .connectivity
                    == SessionConnectivity::Synchronizing)
                .then_some(())
        })
        .await;
        let stale = fixture.inventory();
        assert_eq!(stale.hierarchy[0].freshness, NativeFreshness::Stale);
        assert!(
            stale.hierarchy[0]
                .panes
                .iter()
                .all(|p| p.target.is_none() && p.unbound_row_id.is_none())
        );
        // This display was captured before the actual subscribed event, but commits after
        // the drain's stale transition. It must not reinstall current associations.
        state
            .replace_inventory(
                captured.servers,
                captured.projects,
                vec![],
                captured.unbound,
                captured.hierarchy,
                &[(target, epoch)],
                |graphs| previous.fence_inventory(&mut graphs[0]),
            )
            .unwrap();
        assert_eq!(
            fixture.inventory().hierarchy[0].freshness,
            NativeFreshness::Stale
        );
        assert!(fixture.inventory().hierarchy[0].panes[0].target.is_none());
        assert_eq!(
            fixture
                .runtime
                .validate(&fixture.prompt("blocked while dirty")),
            Err(ErrorCode::NotReady)
        );
        fixture
            .runtime
            .apply_frame(
                &fixture.connection,
                json!({"t":"state","state":{"busy":true}}),
            )
            .unwrap();
        fixture
            .runtime
            .apply_frame(
                &fixture.connection,
                json!({"t":"event","event":{"type":"agent_start"}}),
            )
            .unwrap();
        let syncing = fixture.open();
        assert_eq!(
            syncing.state.summary.connectivity,
            SessionConnectivity::Synchronizing
        );
        assert_eq!(syncing.pending_requests[0].request_key, dialog);
        assert_eq!(state.source_epoch(target).unwrap(), *epoch);
        state
            .read_content(
                &fixture.auth.proof,
                &ContentReadParams {
                    target: target.clone(),
                    handle: content.handle,
                    offset_bytes: Uint(0),
                    limit_bytes: Uint(32),
                },
                pairing::now_ms(),
            )
            .unwrap();
        drop(gate);
        fixture_until(|| {
            fixture
                .runtime
                .topology
                .source(&fixture.connection.key.server_id)
                .ok()
                .filter(|source| {
                    !Arc::ptr_eq(&source.snapshot, &previous.snapshot)
                        && state
                            .session_state(target, epoch)
                            .unwrap()
                            .summary
                            .connectivity
                            == SessionConnectivity::Ready
                })
        })
        .await;
        let fresh = fixture.open();
        assert_eq!(fresh.state.summary.connectivity, SessionConnectivity::Ready);
        assert_eq!(fresh.state.summary.status, SessionStatus::Working);
        assert_eq!(fresh.state.activity.state, ActivityState::Working);
        assert_eq!(fresh.pending_requests[0].request_key, dialog);
        assert_eq!(state.source_epoch(target).unwrap(), *epoch);
        let current = fixture.inventory();
        assert_eq!(current.hierarchy[0].freshness, NativeFreshness::Current);
        assert_eq!(current.hierarchy[0].panes[0].target.as_ref(), Some(target));
        assert!(previous.recheck().is_err());
        fixture
            .runtime
            .validate(&fixture.prompt("same owner after proof"))
            .unwrap();
        fixture.close().await;
    }

    #[tokio::test]
    async fn authoritative_routing_refresh_retains_native_target_epoch_dialog_and_content() {
        let mut fixture = NativeFixture::new(Some(vec![json!({"type":"message","id":"retained","message":{"role":"assistant","content":"retained history"}})])).await;
        let state = &fixture.runtime.security.state;
        let target = fixture.connection.target.clone();
        let epoch = fixture.connection.epoch.clone();
        fixture.runtime.apply_frame(&fixture.connection, json!({
            "t":"ui-request","request":{"reqId":71,"kind":"editor","title":"same native dialog","prefill":"draft"}
        })).unwrap();
        let before = fixture.open();
        let dialog = before.pending_requests[0].request_key.clone();
        let content = state
            .content(
                &target,
                Mime::TextPlain,
                "retained content".repeat(2000),
                pairing::now_ms(),
            )
            .unwrap();
        let Content::Ref(content) = content else {
            panic!("large native content handle required")
        };
        let read = ContentReadParams {
            target: target.clone(),
            handle: content.handle,
            offset_bytes: Uint(0),
            limit_bytes: Uint(32),
        };
        for (index, event) in [
            "pane.agent_status_changed",
            "workspace.focused",
            "pane.moved",
        ]
        .into_iter()
        .enumerate()
        {
            let old_source = fixture
                .runtime
                .topology
                .source(&fixture.connection.source.server_id)
                .unwrap();
            let mut snapshot = fixture.snapshot.borrow().clone();
            if index == 0 {
                snapshot["panes"][0]["agent_status"] = json!("working");
                snapshot["workspaces"][1]["label"] = json!("Renamed workspace");
                snapshot["tabs"][0]["label"] = json!("Renamed tab");
                snapshot["panes"][0]["label"] = json!("Manual desktop pane");
                snapshot["panes"][0]["title"] = json!("Native pane title");
                snapshot["agents"][0]["name"] = json!("desktop-alias");
            } else if index == 1 {
                snapshot["focused_workspace_id"] = json!("native-workspace");
                snapshot["workspaces"][0]["focused"] = json!(false);
                snapshot["workspaces"][1]["focused"] = json!(true);
                snapshot["tabs"][0]["focused"] = json!(true);
                snapshot["panes"][0]["focused"] = json!(true);
            } else {
                snapshot["workspaces"][1]["workspace_id"] = json!("moved-workspace");
                snapshot["tabs"][0]["workspace_id"] = json!("moved-workspace");
                snapshot["panes"][0]["workspace_id"] = json!("moved-workspace");
                snapshot["agents"][0]["workspace_id"] = json!("moved-workspace");
                snapshot["focused_workspace_id"] = json!("moved-workspace");
            }
            let current = fixture.refresh(event, snapshot).await;
            let inventory = fixture_until(|| {
                let inventory = fixture.inventory();
                inventory
                    .hierarchy
                    .first()
                    .filter(|g| {
                        g.freshness == NativeFreshness::Current
                            && g.panes[0].workspace_id == current.snapshot.panes[0].workspace_id
                    })
                    .map(|_| inventory.clone())
            })
            .await;
            let graph = &inventory.hierarchy[0];
            assert_eq!(graph.panes[0].target.as_ref(), Some(&target));
            assert_eq!(
                graph.workspaces[1].label.as_ref().unwrap().0,
                "Renamed workspace"
            );
            assert_eq!(graph.tabs[0].label.as_ref().unwrap().0, "Renamed tab");
            assert_eq!(
                graph.panes[0].label.as_ref().unwrap().0,
                "Manual desktop pane"
            );
            assert_eq!(
                graph.panes[0].title.as_ref().unwrap().0,
                "Native pane title"
            );
            assert_eq!(
                graph.panes[0].agent_name.as_ref().unwrap().0,
                "desktop-alias"
            );
            assert_eq!(graph.panes[0].agent_status, NativeAgentStatus::Working);
            assert_eq!(graph.panes[0].focused, index != 0);
            assert_eq!(current.incarnation, old_source.incarnation);
            assert_eq!(current.evidence, old_source.evidence);
            assert!(
                old_source.recheck().is_err(),
                "captured stale revision must remain untrusted"
            );
            current.recheck().unwrap();
            assert_eq!(state.source_epoch(&target).unwrap(), epoch);
            let open = fixture.open();
            assert_eq!(open.state.summary.generation, target.generation);
            assert_eq!(open.state.summary.connectivity, SessionConnectivity::Ready);
            assert_eq!(
                open.state.summary.workspace_id.0,
                if index == 2 {
                    "moved-workspace"
                } else {
                    "native-workspace"
                }
            );
            assert_eq!(open.pending_requests[0].request_key, dialog);
            state
                .read_content(&fixture.auth.proof, &read, pairing::now_ms())
                .unwrap();
            fixture
                .runtime
                .validate(&fixture.prompt("same native owner after routing refresh"))
                .expect("authoritative routing-only refresh must not disable native mutation");
            fixture
                .runtime
                .apply_frame(
                    &fixture.connection,
                    json!({"t":"event","event":{"type":"notice","message":"fresh native event"}}),
                )
                .expect("fresh native events must survive routing-only evidence refresh");
            assert_eq!(
                fixture.open().state.activity.text,
                Some(Content::Text(ContentText {
                    text: InlineText("fresh native event".into())
                }))
            );
        }
        let response = Request {
            id: Uuid::random(),
            body: RequestBody::RequestRespond(RequestRespondParams {
                boot_id: fixture.runtime.security.boot.clone(),
                target: target.clone(),
                request_key: dialog,
                answer: Answer::OmpEditor(AnswerOmpEditor {
                    action: SubmitAction::Submit,
                    text: Some(PromptText::new("same native dialog reply").unwrap()),
                }),
            }),
        };
        let receipt = fixture
            .runtime
            .clone()
            .mutate(fixture.auth.clone(), response)
            .await
            .unwrap();
        assert_eq!(receipt.state, ReceiptState::Dispatched);
        let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
            panic!("retained native dialog must remain answerable")
        };
        assert_eq!(
            native_json::parse(
                fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                NATIVE
            )
            .unwrap(),
            json!({"t":"ui-response","reqId":71,"value":"same native dialog reply"})
        );
        fixture
            .runtime
            .apply_frame(
                &fixture.connection,
                json!({"t":"ui-request-end","reqId":71}),
            )
            .unwrap();
        assert!(fixture.open().pending_requests.is_empty());
        assert_eq!(state.source_epoch(&target).unwrap(), epoch);
        fixture.close().await;
    }

    #[tokio::test]
    async fn native_report_identity_loss_invalidates_dialog_content_and_late_file_refetch() {
        let fixture = NativeFixture::new(Some(vec![json!({"type":"message","id":"exact-native","message":{"role":"assistant","content":"x".repeat(CONTENT_INLINE_MAX + 33)}})])).await;
        fixture.runtime.apply_frame(&fixture.connection, json!({"t":"ui-request","request":{"reqId":72,"kind":"editor","title":"old dialog","prefill":"draft"}})).unwrap();
        let open = fixture.open();
        let Content::Ref(content) = open.history[0].text.as_ref().unwrap() else {
            panic!("native ref required")
        };
        let read = ContentReadParams {
            target: fixture.connection.target.clone(),
            handle: content.handle.clone(),
            offset_bytes: Uint(0),
            limit_bytes: Uint(1),
        };
        let mut snapshot = fixture.snapshot.borrow().clone();
        snapshot["agents"][0]["agent_session"]["value"] =
            json!(fixture.root.0.join("different-native.jsonl"));
        fixture.refresh("pane_updated", snapshot).await;
        let state = &fixture.runtime.security.state;
        assert_ne!(
            state.source_epoch(&fixture.connection.target).unwrap(),
            fixture.connection.epoch
        );
        let (guard, code) = fixture
            .connection
            .validate_checked(state, &fixture.runtime.topology)
            .unwrap_err();
        assert_eq!((guard, code), ("native_identity", ErrorCode::StaleSession));
        let retained = format!("guard={guard} code={code:?}");
        assert!(!retained.contains(fixture.root.0.to_str().unwrap()));
        let offline = fixture.open();
        assert_eq!(
            offline.state.summary.connectivity,
            SessionConnectivity::Offline
        );
        assert!(offline.pending_requests.is_empty());
        assert_eq!(
            state.read_content(&fixture.auth.proof, &read, pairing::now_ms()),
            Err(ErrorCode::ContentExpired)
        );
        assert!(
            fixture
                .runtime
                .validate(&fixture.prompt("must not retarget"))
                .is_err()
        );
        assert!(
            fixture
                .connection
                .history
                .lock()
                .page(
                    1,
                    100,
                    state,
                    &fixture.connection.target,
                    &fixture.connection.epoch,
                    None
                )
                .is_err(),
            "old exact native file cannot rehydrate after identity loss"
        );
        assert!(
            state
                .read_content(&fixture.auth.proof, &read, pairing::now_ms())
                .is_err()
        );
        fixture.close().await;
    }

    async fn expired_content_live_open_does_not_scan_and_new_browse_refetches(open: bool) {
        let text = "exact-native-large-".repeat(1100);
        let mut fixture = NativeFixture::new(Some(vec![
            json!({"type":"message","id":"native-large","parentId":"real-parent","message":{"role":"assistant","content":[
                {"type":"text","text":text},{"type":"toolCall","id":"refresh-call","name":"read","arguments":{"path":"synthetic"}}
            ]}}),
            json!({"type":"message","id":"decoy-large","message":{"role":"assistant","content":"different-native-record-".repeat(1100)}}),
        ])).await;
        fixture.runtime.apply_event(&fixture.connection, &json!({
            "type":"tool_execution_end","toolCallId":"refresh-call","toolName":"read","isError":false,
            "result":"live-newer-output"
        })).unwrap();
        let initial = if open {
            fixture.open().history
        } else {
            fixture.history().await.records
        };
        let original = initial
            .iter()
            .find(|record| record.id.0 == "native-large")
            .unwrap();
        let Content::Ref(old) = original.text.as_ref().unwrap() else {
            panic!("native >16-KiB ref required")
        };
        let old = old.clone();
        let state = &fixture.runtime.security.state;
        let target = fixture.connection.target.clone();
        let epoch = fixture.connection.epoch.clone();
        let read = ContentReadParams {
            target: target.clone(),
            handle: old.handle.clone(),
            offset_bytes: Uint(0),
            limit_bytes: Uint(65536),
        };
        let bytes = state
            .read_content(&fixture.auth.proof, &read, pairing::now_ms())
            .unwrap();
        assert_eq!(STANDARD.decode(bytes.bytes.0).unwrap(), text.as_bytes());
        // Advance the existing cache's explicit time input, evicting expired payloads without
        // sleeping, changing the live binding, or introducing a clock/testing hook.
        state
            .content(
                &target,
                Mime::TextPlain,
                "expiry-trigger".repeat(1500),
                pairing::now_ms() + crate::state::CONTENT_TTL_MS,
            )
            .unwrap();
        assert_eq!(
            state.read_content(&fixture.auth.proof, &read, pairing::now_ms()),
            Err(ErrorCode::ContentExpired)
        );
        fixture.connection.history.lock().recheck().unwrap();
        let records = if open {
            fixture.open().history
        } else {
            fixture.history().await.records
        };
        let refreshed = records
            .iter()
            .find(|record| record.id.0 == "native-large")
            .unwrap();
        let Content::Ref(fresh) = refreshed.text.as_ref().unwrap() else {
            panic!("reopen/refetch must return usable native content")
        };
        if open {
            assert_eq!(fresh.handle, old.handle);
            assert_eq!(
                state.read_content(&fixture.auth.proof, &read, pairing::now_ms()),
                Err(ErrorCode::ContentExpired)
            );
        } else {
            assert_ne!(fresh.handle, old.handle);
            let bytes = state
                .read_content(
                    &fixture.auth.proof,
                    &ContentReadParams {
                        handle: fresh.handle.clone(),
                        ..read
                    },
                    pairing::now_ms(),
                )
                .unwrap();
            assert_eq!(STANDARD.decode(bytes.bytes.0).unwrap(), text.as_bytes());
        }
        assert_eq!(refreshed.parent_id.as_ref().unwrap().0, "real-parent");
        let live = fixture.open();
        let tool = live
            .history
            .iter()
            .find(|r| r.id.0 == "omp-tool:refresh-call")
            .unwrap()
            .tool
            .as_ref()
            .unwrap();
        assert_eq!(tool.status, ToolStatus::Succeeded);
        let Content::Text(output) = tool.output.as_ref().unwrap() else {
            panic!("live newer output required")
        };
        assert_eq!(output.text.0, "\"live-newer-output\"");
        assert_eq!(state.source_epoch(&target).unwrap(), epoch);
        assert_eq!(
            state
                .session_state(&target, &epoch)
                .unwrap()
                .summary
                .connectivity,
            SessionConnectivity::Ready
        );
        let prompt = fixture.prompt("same still-connected native source");
        let receipt = fixture
            .runtime
            .clone()
            .mutate(fixture.auth.clone(), prompt)
            .await
            .unwrap();
        assert_eq!(receipt.state, ReceiptState::Dispatched);
        let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
            panic!("native connection must stay usable after content refetch")
        };
        assert_eq!(
            native_json::parse(
                fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                NATIVE
            )
            .unwrap(),
            json!({"t":"prompt","text":"same still-connected native source"})
        );
        fixture.close().await;
    }
    #[tokio::test]
    async fn new_native_browse_projects_exact_source_content_without_rotation() {
        expired_content_live_open_does_not_scan_and_new_browse_refetches(false).await;
    }
    #[tokio::test]
    async fn session_open_keeps_expired_content_explicit_without_native_scan() {
        expired_content_live_open_does_not_scan_and_new_browse_refetches(true).await;
    }

    #[tokio::test]
    async fn runtime_stop_drains_late_committed_original_after_cancelled_reservation_waiter() {
        let mut fixture = NativeFixture::new(Some(vec![])).await;
        let request = fixture.prompt("late committed original must not write or replay");
        let reservation = request.clone();
        let original = request.clone();
        let security = fixture.runtime.security.clone();
        let storage = security.storage.clone();
        let dormant = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let handle = dormant.handle().clone();
        let (admitted_tx, admitted_rx) = oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = oneshot::channel();
        let owner = fixture.runtime.clone();
        let validator = owner.clone();
        let auth = fixture.auth.clone();
        let authorization = auth.clone();
        let reserve_security = security.clone();
        let original_connection = fixture.connection.clone();
        let waiter = tokio::spawn(async move {
            reserve_security
                .reserve(
                    &auth,
                    reservation,
                    move |request| {
                        validator.validate(request)?;
                        admitted_tx.send(()).unwrap();
                        // The public owner executes this after real unseen-request validation and
                        // before its SQL commit/post-commit registration; only its reply is cancelled.
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(())
                    },
                    move |committed| {
                        owner.register_committed_original(authorization,OriginalMutation {request:original,connection:Some(original_connection),creation:None,controls:None}, committed, tokio::time::Instant::now() + Duration::from_secs(120), result_tx, handle);
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), admitted_rx)
            .await
            .unwrap()
            .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        let (stop, rx) = watch::channel(false);
        let mut shutdown = Box::pin(fixture.runtime.clone().run(rx));
        stop.send(true).unwrap();
        // Poll the actual shutdown after stop, before releasing the admitted SQLite job.
        // An empty final JoinSet incorrectly returns Ready here; keep cleanup deterministic
        // even on that before-fix path instead of stranding the serialized owner.
        let waited_for_admission = match futures_util::poll!(shutdown.as_mut()) {
            std::task::Poll::Pending => true,
            std::task::Poll::Ready(result) => {
                result.unwrap();
                false
            }
        };
        release_tx.send(()).unwrap();
        security.storage.call(|_| Ok(())).await.unwrap();
        let waited_for_original = if waited_for_admission {
            match futures_util::poll!(shutdown.as_mut()) {
                std::task::Poll::Pending => true,
                std::task::Poll::Ready(result) => {
                    result.unwrap();
                    false
                }
            }
        } else {
            false
        };
        // Drive the exact production-registered original, not a forwarding mock or a
        // callback counter. Its native executor was deliberately dormant until this point.
        let terminal = tokio::task::spawn_blocking(move || {
            dormant.block_on(async {
                tokio::time::timeout(Duration::from_secs(5), result_rx)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
            })
        })
        .await
        .unwrap();
        if waited_for_original {
            tokio::time::timeout(Duration::from_secs(5), shutdown.as_mut())
                .await
                .unwrap()
                .unwrap();
        }
        drop(shutdown);
        assert_eq!(terminal.action_id, request.id);
        assert_eq!(terminal.method, ActionMethod::PromptSubmit);
        assert_eq!(terminal.target.as_ref(), Some(&fixture.connection.target));
        assert_eq!(terminal.state, ReceiptState::Failed);
        assert_eq!(terminal.error.as_ref().unwrap().code, ErrorCode::NotReady);
        assert_eq!(
            fixture
                .runtime
                .clone()
                .mutate(fixture.auth.clone(), request)
                .await
                .unwrap(),
            terminal,
            "matching terminal receipt must not replay its original"
        );
        assert_eq!(
            fixture_frame(&mut fixture.host_socket)
                .await
                .to_text()
                .unwrap(),
            "{\"t\":\"peer-left\",\"peer\":1}",
            "unentered original wrote native bytes before shutdown"
        );
        fixture.close().await;
        drop(Arc::into_inner(security).expect("runtime security owner still retained"));
        Arc::into_inner(storage)
            .expect("runtime storage owner still retained")
            .close()
            .await
            .unwrap();
        assert!(
            waited_for_admission,
            "Runtime::run finished before admitted original could commit/register"
        );
        assert!(
            waited_for_original,
            "Runtime::run did not join the late registered original"
        );
    }

    #[tokio::test]
    async fn native_browse_does_not_copy_or_rehydrate_live_history_under_capacity_pressure() {
        let text = "exact-budget-native-".repeat(1100);
        let entries = (0..110).map(|n| json!({"type":"message","id":format!("budget-{n}"),"message":{"role":"assistant","content":text}})).collect();
        let fixture = NativeFixture::new(Some(entries)).await;
        let state = &fixture.runtime.security.state;
        let target = fixture.connection.target.clone();
        let before = fixture.open();
        let native = fixture.history().await;
        assert!(native.next_cursor.is_some());
        assert!(
            native
                .records
                .iter()
                .all(|r| r.source == RecordSource::Persisted)
        );
        assert_eq!(
            fixture.open().history,
            before.history,
            "native browsing mutated the live window"
        );
        let bytes = state.history_storage_bytes_for_test();
        assert!(bytes <= crate::state::SESSION_BYTES);
        let source = state.source_epoch(&target).unwrap();
        let oversized = crate::state::BrowsePosition::new((), crate::state::SESSION_BYTES).unwrap();
        assert!(matches!(
            state.begin_browse(
                &fixture.auth.proof,
                &target,
                &source,
                oversized,
                100,
                pairing::now_ms()
            ),
            Err(ErrorCode::ResourceLimit)
        ));
        assert!(state.history_storage_bytes_for_test() <= crate::state::SESSION_BYTES);
        assert_eq!(fixture.open().history, before.history);
        fixture.close().await;
    }

    struct FixtureWake(tokio::sync::Notify);
    impl std::task::Wake for FixtureWake {
        fn wake(self: Arc<Self>) {
            self.0.notify_one();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.notify_one();
        }
    }
    async fn mutate_with_native_jobs_parked(
        phase: &'static str,
        dormant: &tokio::runtime::Runtime,
        runtime: Arc<Runtime>,
        auth: Arc<Authorized>,
        request: Request,
    ) -> SecurityResult<Receipt> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let signal = Arc::new(FixtureWake(tokio::sync::Notify::new()));
        let waker = std::task::Waker::from(signal.clone());
        let mut context = std::task::Context::from_waker(&waker);
        // This harness manually polls only reservation/admission. Without the scheduler's
        // budget reset, a cooperative pending future can self-wake without completing.
        let mut future = Box::pin(tokio::task::coop::unconstrained(
            runtime.mutate(auth, request),
        ));
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{phase} manual reservation poll exceeded five seconds"
            );
            let notified = signal.0.notified();
            let result = {
                // Poll only the reservation caller; spawned original native jobs cannot run
                // until this separate current-thread executor is explicitly driven below.
                let _entered = dormant.enter();
                future.as_mut().poll(&mut context)
            };
            if let std::task::Poll::Ready(result) = result {
                return result;
            }
            tokio::time::timeout_at(deadline, notified)
                .await
                .unwrap_or_else(|_| {
                    panic!("{phase} manual reservation poll exceeded five seconds")
                });
        }
    }
    #[tokio::test]
    async fn create_receipt_component_returns_committed_receipt_before_registered_job_progresses() {
        let fixture = NativeFixture::new(Some(vec![])).await;
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::SessionCreate(SessionCreateParams { directory_ref: None, placement: None, boot_id: fixture.runtime.security.boot.clone(),
            server_id: fixture.connection.source.server_id.clone(),
            project_id: Uuid::random(),
            provider: Provider::Omp,
            name: Label::new("parked component creation").unwrap(), }),
        };
        let security = fixture.runtime.security.clone();
        let auth = fixture.auth.clone();
        let action = request.id.clone();
        let proof = auth.proof.clone();
        let releases = Arc::new(tokio::sync::Notify::new());
        let registered = Arc::new(AtomicUsize::new(0));
        let jobs = Arc::new(Mutex::new(JoinSet::new()));
        let handle = tokio::runtime::Handle::current();
        let reservation = security
            .reserve(&auth, request.clone(), |_| Ok(()), {
                let releases = releases.clone();
                let registered = registered.clone();
                let jobs = jobs.clone();
                let security = security.clone();
                move |_| {
                    registered.fetch_add(1, Ordering::SeqCst);
                    let action = action.clone();
                    let proof = proof.clone();
                    jobs.lock().spawn_on(
                        async move {
                            releases.notified().await;
                            security
                                .storage
                                .call(move |store| {
                                    store
                                        .transition(
                                            &proof,
                                            &action,
                                            ReceiptTransition {
                                                state: ReceiptState::Dispatching,
                                                proof: TransitionProof::Intent,
                                                result: None,
                                                error: None,
                                                at: pairing::now_ms(),
                                            },
                                        )
                                        .map(|_| ())
                                })
                                .await
                                .unwrap();
                        },
                        &handle,
                    );
                }
            })
            .await
            .unwrap();
        let Some(committed) = reservation_reply(true, reservation) else {
            panic!("SessionCreate must return its committed receipt")
        };
        assert_eq!(committed.state, ReceiptState::Reserved);
        assert_eq!(registered.load(Ordering::SeqCst), 1);
        assert_eq!(jobs.lock().len(), 1);
        let duplicate = security
            .reserve(
                &auth,
                request,
                |_| panic!("matching SessionCreate must not revalidate"),
                |_| panic!("matching SessionCreate must not register another job"),
            )
            .await
            .unwrap();
        let Some(duplicate) = reservation_reply(true, duplicate) else {
            panic!("matching SessionCreate must return its canonical receipt")
        };
        assert_eq!(duplicate, committed);
        assert_eq!(registered.load(Ordering::SeqCst), 1);
        releases.notify_one();
        loop {
            if let Some(result) = jobs.lock().try_join_next() {
                result.unwrap();
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            receipt(&security, &auth, &committed.action_id)
                .await
                .unwrap()
                .state,
            ReceiptState::Dispatching,
            "the registered original progresses through the canonical receipt after return"
        );
        fixture.close().await;
    }
    #[tokio::test]
    async fn unstarted_native_boundary_defers_after_intent_without_replay_or_blocking_revocation() {
        async fn write_original(
            writer: &mut NativeWriter,
            encrypted: &mut Option<Vec<u8>>,
            result: &mut Option<ActionResult>,
            polls: &AtomicUsize,
        ) -> ReceiptTransition {
            polls.fetch_add(1, Ordering::Relaxed);
            let bytes = encrypted.take().expect("one original message");
            let written = writer.send(Message::Binary(bytes.into())).await.is_ok();
            ReceiptTransition {
                state: if written {
                    ReceiptState::Dispatched
                } else {
                    ReceiptState::Uncertain
                },
                proof: if written {
                    TransitionProof::WriteCompleted
                } else {
                    TransitionProof::LostOutcome
                },
                result: if written { result.take() } else { None },
                error: None,
                at: pairing::now_ms(),
            }
        }
        for case in [
            "refresh",
            "revoke",
            "shutdown",
            "deadline",
            "source_io",
            "mapping_loss",
            "capability",
            "dialog",
            "cancel",
        ] {
            let mut fixture = NativeFixture::new(Some(vec![])).await;
            // The deadline case expires only after the actual source-pending barrier.
            // Keep Tokio from auto-advancing across blocking storage/socket work, but
            // bound that runnable clock owner by the existing fixture's five-second budget.
            let clock_driver = if case == "deadline" {
                tokio::time::pause();
                let until = std::time::Instant::now() + Duration::from_secs(5);
                Some(tokio::spawn(async move {
                    while std::time::Instant::now() < until {
                        tokio::task::yield_now().await;
                    }
                }))
            } else {
                None
            };
            let target = fixture.connection.target.clone();
            let epoch = fixture.connection.epoch.clone();
            let state = fixture.runtime.security.state.clone();
            let body = if case == "dialog" {
                fixture.runtime.apply_frame(&fixture.connection, json!({
                    "t":"ui-request","request":{"reqId":79,"kind":"editor","title":"original","prefill":"draft"}
                })).unwrap();
                RequestBody::RequestRespond(RequestRespondParams {
                    boot_id: fixture.runtime.security.boot.clone(),
                    target: target.clone(),
                    request_key: fixture.open().pending_requests[0].request_key.clone(),
                    answer: Answer::OmpEditor(AnswerOmpEditor {
                        action: SubmitAction::Submit,
                        text: Some(PromptText::new("original answer").unwrap()),
                    }),
                })
            } else {
                RequestBody::TurnCancel(TurnCancelParams {
                    boot_id: fixture.runtime.security.boot.clone(),
                    target: target.clone(),
                    native_turn_id: None,
                })
            };
            let request = Request {
                id: Uuid::random(),
                body,
            };
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(if case == "deadline" { 1 } else { 120 });
            let connection = fixture
                .runtime
                .wait_connection(
                    &fixture.auth,
                    &request,
                    &target,
                    deadline,
                    fixture.connection.clone(),
                )
                .await
                .unwrap();
            let validator = fixture.runtime.clone();
            let (committed_tx, committed_rx) = oneshot::channel();
            let reserved = fixture
                .runtime
                .security
                .reserve(
                    &fixture.auth,
                    request.clone(),
                    move |request| validator.validate(request),
                    move |committed| {
                        let _ = committed_tx.send(committed);
                    },
                )
                .await
                .unwrap();
            assert!(matches!(reserved, Reservation::Reserved(_)));
            let committed = committed_rx.await.unwrap();
            assert_eq!(committed.state, ReceiptState::Reserved);
            let checkpoint = fixture
                .runtime
                .security
                .unstarted_boundary(
                    &fixture.auth,
                    committed.action_id,
                    connection.fd().unwrap(),
                    deadline,
                )
                .await
                .unwrap();
            assert_eq!(
                receipt(&fixture.runtime.security, &fixture.auth, &request.id)
                    .await
                    .unwrap()
                    .state,
                ReceiptState::Dispatching
            );
            // Only now invalidate the actual subscribed source: the intent is already durable.
            let previous = fixture
                .runtime
                .topology
                .source(&connection.key.server_id)
                .unwrap();
            let mut gate = Some(fixture.snapshot_gate.clone().acquire_owned().await.unwrap());
            fixture
                .events
                .send_replace(Some(json!({"event":"pane.agent_status_changed","data":{}})));
            fixture_until(|| {
                (previous.recheck().is_err()
                    && state
                        .session_state(&target, &epoch)
                        .unwrap()
                        .summary
                        .connectivity
                        == SessionConnectivity::Synchronizing)
                    .then_some(())
            })
            .await;
            assert_eq!(
                connection.validate_checked(&state, &fixture.runtime.topology),
                Err(("source_pending", ErrorCode::NotReady))
            );
            let (payload, action_result) = if let RequestBody::RequestRespond(p) = &request.body {
                let dialogs = connection.dialogs.lock();
                (
                    dialogs
                        .get(&p.request_key)
                        .unwrap()
                        .response(&p.answer)
                        .unwrap(),
                    ActionResult::Response(ActionResultResponse {
                        resolution: ResponseResolution::Written,
                    }),
                )
            } else {
                (
                    json!({"t":"abort"}),
                    ActionResult::Cancel(ActionResultCancel {
                        native_turn_id: None,
                    }),
                )
            };
            let mut encrypted = Some(connection.link.seal_value(&payload).unwrap());
            let mut result = Some(action_result);
            let polls = AtomicUsize::new(0);
            let mut writer = connection.writer.lock().await;
            let checkpoint = checkpoint
                .try_start(
                    || {
                        fixture
                            .runtime
                            .boundary_readiness(&request, &connection)
                            .map_err(|(_, code)| code)
                    },
                    write_original(&mut writer, &mut encrypted, &mut result, &polls),
                )
                .await
                .unwrap()
                .expect("same original must remain unstarted");
            drop(writer);
            assert_eq!(polls.load(Ordering::Relaxed), 0);
            assert!(encrypted.is_some() && result.is_some());
            assert_eq!(
                receipt(&fixture.runtime.security, &fixture.auth, &request.id)
                    .await
                    .unwrap()
                    .state,
                ReceiptState::Dispatching
            );
            let marker = json!({"t":"fixture-pending"});
            connection
                .writer
                .lock()
                .await
                .send(Message::Binary(
                    connection.link.seal_value(&marker).unwrap().into(),
                ))
                .await
                .unwrap();
            let Message::Binary(bytes) = fixture_frame(&mut fixture.host_socket).await else {
                panic!("unstarted original must leave the native stream writable")
            };
            assert_eq!(
                native_json::parse(connection.link.open(&mut bytes.to_vec()).unwrap(), NATIVE)
                    .unwrap(),
                marker
            );
            let mut checkpoint = Some(checkpoint);
            match case {
                "refresh" => {
                    drop(gate.take());
                    let fresh = fixture
                        .runtime
                        .wait_connection(
                            &fixture.auth,
                            &request,
                            &target,
                            deadline,
                            connection.clone(),
                        )
                        .await
                        .unwrap();
                    assert!(Arc::ptr_eq(&fresh, &connection));
                    assert_eq!(state.source_epoch(&target).unwrap(), epoch);
                }
                "revoke" => {
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        fixture
                            .runtime
                            .security
                            .revoke(fixture.auth.proof.peer_id().clone(), None),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                }
                "shutdown" => {
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        fixture.runtime.security.shutdown(),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                }
                "deadline" => {
                    tokio::time::advance(
                        deadline.saturating_duration_since(tokio::time::Instant::now()),
                    )
                    .await;
                }
                "source_io" => fs::remove_file(&connection.source.socket_path).unwrap(),
                "mapping_loss" => {
                    fixture
                        .runtime
                        .connections
                        .lock()
                        .remove(&target.session_id)
                        .unwrap();
                }
                "capability" => {
                    let mut disabled = state.session_state(&target, &epoch).unwrap();
                    disabled.summary.capabilities.cancel = false;
                    state
                        .update_from_source(&target, &epoch, SessionUpdate::State(disabled))
                        .unwrap();
                }
                "dialog" => fixture
                    .runtime
                    .apply_frame(&connection, json!({"t":"ui-request-end","reqId":79}))
                    .unwrap(),
                "cancel" => drop(checkpoint.take()),
                _ => unreachable!(),
            }
            if let Some(checkpoint) = checkpoint {
                let mut writer = connection.writer.lock().await;
                let attempt = checkpoint
                    .try_start(
                        || {
                            fixture
                                .runtime
                                .boundary_readiness(&request, &connection)
                                .map_err(|(_, code)| code)
                        },
                        write_original(&mut writer, &mut encrypted, &mut result, &polls),
                    )
                    .await;
                drop(writer);
                if case == "refresh" {
                    assert!(attempt.unwrap().is_none());
                    assert_eq!(polls.load(Ordering::Relaxed), 1);
                    let Message::Binary(bytes) = fixture_frame(&mut fixture.host_socket).await
                    else {
                        panic!("one authenticated original abort required")
                    };
                    assert_eq!(
                        native_json::parse(
                            connection.link.open(&mut bytes.to_vec()).unwrap(),
                            NATIVE
                        )
                        .unwrap(),
                        json!({"t":"abort"})
                    );
                    assert_eq!(
                        receipt(&fixture.runtime.security, &fixture.auth, &request.id)
                            .await
                            .unwrap()
                            .state,
                        ReceiptState::Dispatched
                    );
                } else {
                    let code = attempt
                        .err()
                        .expect("terminal guard must deny the original");
                    match case {
                        "revoke" | "shutdown" => assert_eq!(code, ErrorCode::Unauthorized),
                        "deadline" => assert_eq!(code, ErrorCode::Timeout),
                        "dialog" => assert_eq!(code, ErrorCode::AlreadyResolved),
                        "source_io" => assert_eq!(code, ErrorCode::StaleSession),
                        _ => assert_eq!(code, ErrorCode::NotReady),
                    }
                }
            }
            if case != "refresh" {
                assert_eq!(polls.load(Ordering::Relaxed), 0);
                assert!(encrypted.is_some() && result.is_some());
                if !matches!(case, "revoke" | "shutdown") {
                    let terminal = tokio::time::timeout(Duration::from_secs(1), async {
                        loop {
                            let current =
                                receipt(&fixture.runtime.security, &fixture.auth, &request.id)
                                    .await
                                    .unwrap();
                            if current.state == ReceiptState::Failed {
                                break current;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert_eq!(terminal.target.as_ref(), Some(&target));
                }
            }
            let marker = json!({"t":"fixture-end"});
            connection
                .writer
                .lock()
                .await
                .send(Message::Binary(
                    connection.link.seal_value(&marker).unwrap().into(),
                ))
                .await
                .unwrap();
            let Message::Binary(bytes) = fixture_frame(&mut fixture.host_socket).await else {
                panic!("ordered final no-extra-effect marker required")
            };
            assert_eq!(
                native_json::parse(connection.link.open(&mut bytes.to_vec()).unwrap(), NATIVE)
                    .unwrap(),
                marker,
                "{case}: forbidden or repeated native effect"
            );
            drop(gate);
            if let Some(driver) = clock_driver {
                driver.abort();
                tokio::time::resume();
            }
            drop(connection);
            fixture.close().await;
        }
    }

    #[tokio::test]
    async fn committed_original_waits_for_same_source_then_dispatches_once_or_fails_without_effect()
    {
        for case in [
            "refresh",
            "source_io",
            "mapping_loss",
            "revoke",
            "deadline",
            "shutdown",
            "capability",
            "dialog",
        ] {
            let mut fixture = NativeFixture::new(Some(vec![])).await;
            let target = fixture.connection.target.clone();
            let epoch = fixture.connection.epoch.clone();
            let state = fixture.runtime.security.state.clone();
            let body = if case == "dialog" {
                fixture.runtime.apply_frame(&fixture.connection, json!({
                    "t":"ui-request","request":{"reqId":79,"kind":"editor","title":"original","prefill":"draft"}
                })).unwrap();
                let key = fixture.open().pending_requests[0].request_key.clone();
                RequestBody::RequestRespond(RequestRespondParams {
                    boot_id: fixture.runtime.security.boot.clone(),
                    target: target.clone(),
                    request_key: key,
                    answer: Answer::OmpEditor(AnswerOmpEditor {
                        action: SubmitAction::Submit,
                        text: Some(PromptText::new("original answer").unwrap()),
                    }),
                })
            } else {
                RequestBody::TurnCancel(TurnCancelParams {
                    boot_id: fixture.runtime.security.boot.clone(),
                    target: target.clone(),
                    native_turn_id: None,
                })
            };
            let request = Request {
                id: Uuid::random(),
                body,
            };
            let dormant = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let handle = dormant.handle().clone();
            let owner = fixture.runtime.clone();
            let validator = owner.clone();
            let authorization = fixture.auth.clone();
            let original = request.clone();
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(if case == "deadline" { 1 } else { 120 });
            let (result_tx, result_rx) = oneshot::channel();
            let original_connection = fixture
                .runtime
                .wait_connection(
                    &fixture.auth,
                    &request,
                    &target,
                    deadline,
                    fixture.connection.clone(),
                )
                .await
                .unwrap();
            let validation_connection = original_connection.clone();
            let previous = fixture
                .runtime
                .topology
                .source(&fixture.connection.key.server_id)
                .unwrap();
            let mut gate = Some(fixture.snapshot_gate.clone().acquire_owned().await.unwrap());
            fixture
                .events
                .send_replace(Some(json!({"event":"pane.agent_status_changed","data":{}})));
            fixture_until(|| {
                (previous.recheck().is_err()
                    && state
                        .session_state(&target, &epoch)
                        .unwrap()
                        .summary
                        .connectivity
                        == SessionConnectivity::Synchronizing)
                    .then_some(())
            })
            .await;
            assert_eq!(
                fixture
                    .connection
                    .validate_checked(&state, &fixture.runtime.topology),
                Err(("source_pending", ErrorCode::NotReady))
            );
            assert_eq!(
                fixture.runtime.validate_checked(&request, None, false),
                Err(("core_sync", ErrorCode::NotReady)),
                "the old final strict reservation rejects this actual post-prewait window"
            );
            let reserved = fixture
                .runtime
                .security
                .reserve(
                    &fixture.auth,
                    request.clone(),
                    move |request| {
                        validator
                            .validate_checked(request, Some(&validation_connection), true)
                            .map_err(|(_, code)| code)
                    },
                    move |committed| {
                        owner.register_committed_original(authorization,OriginalMutation {request:original,connection:Some(original_connection),creation:None,controls:None}, committed, deadline, result_tx, handle)
                    },
                )
                .await
                .unwrap();
            assert!(matches!(reserved, Reservation::Reserved(_)));
            let refs = Arc::strong_count(&fixture.connection);
            let driver = tokio::task::spawn_blocking(move || {
                dormant.block_on(async {
                    tokio::time::timeout(Duration::from_secs(5), result_rx)
                        .await
                        .unwrap()
                        .unwrap()
                })
            });
            if case != "deadline" {
                // The committed job already owns its captured Arc; it must keep that
                // reference while the real source gate remains closed.
                tokio::time::sleep(Duration::from_millis(150)).await;
                let pinned =
                    !driver.is_finished() && Arc::strong_count(&fixture.connection) >= refs;
                assert!(
                    pinned,
                    "committed original must remain pending on its first Arc: {case}"
                );
                let pending = receipt(&fixture.runtime.security, &fixture.auth, &request.id)
                    .await
                    .unwrap();
                assert_eq!(pending.state, ReceiptState::Reserved);
                assert_eq!(pending.target.as_ref(), Some(&target));
                // Ordered bytes prove the original abort has not crossed the dirty gate.
                let marker = json!({"t":"fixture-pending"});
                fixture
                    .connection
                    .writer
                    .lock()
                    .await
                    .send(Message::Binary(
                        fixture.connection.link.seal_value(&marker).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
                    panic!("pending no-effect marker required: {case}")
                };
                assert_eq!(
                    native_json::parse(
                        fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                        NATIVE,
                    )
                    .unwrap(),
                    marker,
                    "{case}: native effect preceded authoritative refresh"
                );
            }
            let mut shutdown = None;
            match case {
                "refresh" => drop(gate.take()),
                "source_io" => fs::remove_file(&fixture.connection.source.socket_path).unwrap(),
                "mapping_loss" => {
                    fixture
                        .runtime
                        .connections
                        .lock()
                        .remove(&target.session_id)
                        .unwrap();
                }
                "revoke" => {
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        fixture
                            .runtime
                            .security
                            .revoke(fixture.auth.proof.peer_id().clone(), None),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                }
                "shutdown" => {
                    let (_, rx) = watch::channel(true);
                    shutdown = Some(tokio::spawn(fixture.runtime.clone().run(rx)));
                }
                "capability" => {
                    let mut current = state.session_state(&target, &epoch).unwrap();
                    current.summary.capabilities.cancel = false;
                    state
                        .update_from_source(&target, &epoch, SessionUpdate::State(current))
                        .unwrap();
                }
                "dialog" => {
                    fixture
                        .runtime
                        .apply_frame(
                            &fixture.connection,
                            json!({"t":"ui-request-end","reqId":79}),
                        )
                        .unwrap();
                }
                "deadline" => {}
                _ => unreachable!(),
            }
            let result = driver.await.unwrap();
            if let Some(shutdown) = shutdown {
                tokio::time::timeout(Duration::from_secs(5), shutdown)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
            }
            if case == "refresh" {
                let terminal = result.unwrap();
                assert_eq!(terminal.state, ReceiptState::Dispatched);
                assert_eq!(terminal.target.as_ref(), Some(&target));
                assert_eq!(state.source_epoch(&target).unwrap(), epoch);
                assert!(Arc::ptr_eq(
                    fixture
                        .runtime
                        .connections
                        .lock()
                        .get(&target.session_id)
                        .unwrap(),
                    &fixture.connection,
                ));
                let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
                    panic!("the refreshed original must write one authenticated abort")
                };
                assert_eq!(
                    native_json::parse(
                        fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                        NATIVE,
                    )
                    .unwrap(),
                    json!({"t":"abort"})
                );
                let duplicate = fixture
                    .runtime
                    .clone()
                    .mutate(fixture.auth.clone(), request.clone())
                    .await
                    .unwrap();
                assert_eq!(
                    duplicate, terminal,
                    "matching receipt cannot reserve or dispatch again"
                );
            } else if case == "revoke" {
                assert!(matches!(result, Err(ErrorCode::Unauthorized)));
            } else {
                let terminal = result.unwrap();
                assert_eq!(terminal.state, ReceiptState::Failed, "{case}");
                assert!(terminal.result.is_none());
                if case == "deadline" {
                    assert_eq!(terminal.error.unwrap().code, ErrorCode::Timeout);
                } else if case == "dialog" {
                    assert_eq!(terminal.error.unwrap().code, ErrorCode::AlreadyResolved);
                }
            }
            if case == "shutdown" {
                assert_eq!(
                    fixture_frame(&mut fixture.host_socket)
                        .await
                        .to_text()
                        .unwrap(),
                    "{\"t\":\"peer-left\",\"peer\":1}"
                );
            } else {
                // Same ordered authenticated stream: a second/forbidden abort would precede this.
                let marker = json!({"t":"fixture-end"});
                fixture
                    .connection
                    .writer
                    .lock()
                    .await
                    .send(Message::Binary(
                        fixture.connection.link.seal_value(&marker).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
                    panic!("ordered no-extra-effect marker required: {case}")
                };
                assert_eq!(
                    native_json::parse(
                        fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                        NATIVE,
                    )
                    .unwrap(),
                    marker,
                    "{case}: unexpected native effect before the boundary marker"
                );
            }
            drop(gate);
            fixture.close().await;
        }
    }
    #[tokio::test]
    async fn unseen_core_sync_wait_preserves_matching_receipts_and_one_final_reservation() {
        for case in [
            "refresh",
            "offline",
            "revoke",
            "race_match",
            "race_conflict",
        ] {
            let mut fixture = NativeFixture::new(Some(vec![])).await;
            let target = fixture.connection.target.clone();
            let epoch = fixture.connection.epoch.clone();
            let state = fixture.runtime.security.state.clone();
            let request = if case == "race_conflict" {
                fixture.prompt("first original body")
            } else {
                Request {
                    id: Uuid::random(),
                    body: RequestBody::TurnCancel(TurnCancelParams {
                        boot_id: fixture.runtime.security.boot.clone(),
                        target: target.clone(),
                        native_turn_id: None,
                    }),
                }
            };
            let previous = fixture
                .runtime
                .topology
                .source(&fixture.connection.key.server_id)
                .unwrap();
            let mut gate = Some(fixture.snapshot_gate.clone().acquire_owned().await.unwrap());
            fixture
                .events
                .send_replace(Some(json!({"event":"pane.agent_status_changed","data":{}})));
            fixture_until(|| {
                (previous.recheck().is_err()
                    && state
                        .session_state(&target, &epoch)
                        .unwrap()
                        .summary
                        .connectivity
                        == SessionConnectivity::Synchronizing)
                    .then_some(())
            })
            .await;
            assert_eq!(
                fixture.runtime.validate_checked(&request, None, false),
                Err(("core_sync", ErrorCode::NotReady))
            );
            let (entered, mut waits) = mpsc::channel(2);
            *fixture.runtime.core_wait_entered.lock() = Some(entered);
            let owner = fixture.runtime.clone();
            let auth = fixture.auth.clone();
            let first = request.clone();
            let first = tokio::spawn(async move { owner.mutate(auth, first).await });
            let second = if matches!(case, "race_match" | "race_conflict") {
                let mut second = request.clone();
                if case == "race_conflict" {
                    second.body = RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: fixture.runtime.security.boot.clone(),
                    target: target.clone(),
                    text: PromptText::new("conflicting original body").unwrap(), });
                }
                let owner = fixture.runtime.clone();
                let auth = fixture.auth.clone();
                Some(tokio::spawn(
                    async move { owner.mutate(auth, second).await },
                ))
            } else {
                None
            };
            tokio::time::timeout(Duration::from_secs(5), async {
                for _ in 0..1 + usize::from(second.is_some()) {
                    waits.recv().await.expect("original must enter real Core wait");
                }
            })
            .await
            .expect("original Core waits must be entered before admission");
            assert!(
                !first.is_finished() && second.as_ref().is_none_or(|job| !job.is_finished()),
                "unseen original must wait before reserving: {case}"
            );
            assert!(fixture.runtime.jobs.lock().is_empty());
            let proof = fixture.auth.proof.clone();
            let id = request.id.clone();
            assert!(
                fixture
                    .runtime
                    .security
                    .storage
                    .call(move |s| s.status(&proof, &id, pairing::now_ms()))
                    .await
                    .unwrap()
                    .is_none()
            );
            match case {
                "offline" => {
                    state
                        .binding_offline(&target, &epoch, "real source identity lost")
                        .unwrap();
                }
                "revoke" => {
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        fixture
                            .runtime
                            .security
                            .revoke(fixture.auth.proof.peer_id().clone(), None),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                }
                _ => drop(gate.take()),
            }
            let first = tokio::time::timeout(Duration::from_secs(5), first)
                .await
                .unwrap()
                .unwrap();
            let second = match second {
                Some(job) => Some(
                    tokio::time::timeout(Duration::from_secs(5), job)
                        .await
                        .unwrap()
                        .unwrap(),
                ),
                None => None,
            };
            if matches!(case, "offline" | "revoke") {
                let expected = if case == "revoke" {
                    ErrorCode::Unauthorized
                } else {
                    ErrorCode::StaleSession
                };
                assert_eq!(first, Err(expected));
                let stored = rusqlite::Connection::open_with_flags(
                    &fixture.database,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                let count: i64 = stored
                    .query_row(
                        "SELECT count(*) FROM receipts WHERE action=?1",
                        [&request.id.0],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(count, 0, "{case}: no reservation was authorized");
            } else {
                let first_won = first.is_ok();
                if case == "race_conflict" {
                    assert_ne!(first.is_ok(), second.as_ref().unwrap().is_ok());
                    let rejected = if first_won {
                        second.as_ref().unwrap()
                    } else {
                        &first
                    };
                    assert_eq!(rejected, &Err(ErrorCode::ActionIdConflict));
                } else {
                    assert!(first.is_ok());
                    if let Some(second) = &second {
                        assert!(second.is_ok())
                    }
                }
                let terminal = receipt(&fixture.runtime.security, &fixture.auth, &request.id)
                    .await
                    .unwrap();
                assert_eq!(terminal.state, ReceiptState::Dispatched);
                assert_eq!(terminal.target.as_ref(), Some(&target));
                assert_eq!(state.source_epoch(&target).unwrap(), epoch);
                let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
                    panic!("one original authenticated native write required")
                };
                let native = native_json::parse(
                    fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                    NATIVE,
                )
                .unwrap();
                if case == "race_conflict" {
                    assert_eq!(
                        native,
                        json!({"t":"prompt","text":if first_won {
                        "first original body"
                    } else { "conflicting original body" }})
                    );
                } else {
                    assert_eq!(native, json!({"t":"abort"}));
                }
                if case == "refresh" {
                    // An existing canonical receipt still wins when both host/Core are stale.
                    state.begin_source_install(&target).unwrap();
                    fixture
                        .runtime
                        .security
                        .storage
                        .call(|s| {
                            s.boot_id = Uuid::random();
                            Ok(())
                        })
                        .await
                        .unwrap();
                    assert_eq!(
                        tokio::time::timeout(
                            Duration::from_secs(1),
                            fixture
                                .runtime
                                .clone()
                                .mutate(fixture.auth.clone(), request.clone()),
                        )
                        .await
                        .unwrap()
                        .unwrap(),
                        terminal
                    );
                    let mut conflict = request.clone();
                    conflict.body = RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: fixture.runtime.security.boot.clone(),
                    target: target.clone(),
                    text: PromptText::new("different fingerprint").unwrap(), });
                    assert_eq!(
                        fixture
                            .runtime
                            .clone()
                            .mutate(fixture.auth.clone(), conflict)
                            .await,
                        Err(ErrorCode::ActionIdConflict)
                    );
                    let mut unseen = request.clone();
                    unseen.id = Uuid::random();
                    assert_eq!(
                        fixture
                            .runtime
                            .clone()
                            .mutate(fixture.auth.clone(), unseen)
                            .await,
                        Err(ErrorCode::StaleHost)
                    );
                }
            }
            // Ordered native-stream evidence detects an extra/raced/forbidden effect.
            let marker = json!({"t":"fixture-end"});
            fixture
                .connection
                .writer
                .lock()
                .await
                .send(Message::Binary(
                    fixture.connection.link.seal_value(&marker).unwrap().into(),
                ))
                .await
                .unwrap();
            let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
                panic!("ordered final boundary marker required")
            };
            assert_eq!(
                native_json::parse(
                    fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                    NATIVE,
                )
                .unwrap(),
                marker,
                "{case}: unexpected additional native write"
            );
            drop(gate);
            fixture.close().await;
        }
    }
    #[tokio::test]
    async fn capacity_rejection_cannot_execute_original_against_reused_uuid_reservation() {
        let mut fixture = NativeFixture::new(Some(vec![])).await;
        let original = fixture.prompt("rejected original must never execute");
        let template_request = fixture.prompt("capacity fixture");
        let template_proof = fixture.auth.proof.clone();
        let template = fixture
            .runtime
            .security
            .storage
            .call(move |storage| {
                storage.reserve(
                    &template_proof,
                    &template_request,
                    pairing::now_ms(),
                    |_| Ok(()),
                )?;
                storage.transition(
                    &template_proof,
                    &template_request.id,
                    ReceiptTransition {
                        state: ReceiptState::Failed,
                        proof: TransitionProof::DefinitiveNoEffect(Id(
                            "owned fixture never entered".into(),
                        )),
                        result: None,
                        error: None,
                        at: pairing::now_ms(),
                    },
                )
            })
            .await
            .unwrap();
        // Retain one storage-produced failed receipt, then batch the same valid metadata. The
        // production admission below still performs the actual capacity count and lookup.
        let peer = fixture.auth.proof.peer_id().clone();
        let epoch = i64::try_from(fixture.auth.proof.epoch()).unwrap();
        let boot = fixture.runtime.security.boot.clone();
        let target = fixture.connection.target.clone();
        let at = pairing::now_ms();
        let mut seeded = rusqlite::Connection::open(&fixture.database).unwrap();
        let transaction = seeded.transaction().unwrap();
        {
            let mut insert = transaction
                .prepare("INSERT INTO receipts VALUES(?1,?2,?3,?4,?5,'failed',?6)")
                .unwrap();
            for _ in 1..crate::storage::PEER_CAPACITY {
                let request = Request {
                    id: Uuid::random(),
                    body: RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: boot.clone(),
                    target: target.clone(),
                    text: PromptText::new("capacity fixture").unwrap(), }),
                };
                let fingerprint = hmac_sha256(&[23; 32], &canonical_request(&request).unwrap());
                let mut receipt = template.clone();
                receipt.action_id = request.id.clone();
                receipt.created_at = Uint(at);
                receipt.updated_at = Uint(at);
                insert
                    .execute(rusqlite::params![
                        &peer.0,
                        &request.id.0,
                        epoch,
                        fingerprint,
                        i64::try_from(at).unwrap(),
                        serde_json::to_string(&receipt).unwrap(),
                    ])
                    .unwrap();
            }
        }
        transaction.commit().unwrap();
        let receipt_count: i64 = seeded
            .query_row(
                "SELECT count(*) FROM receipts WHERE peer=?1",
                [&peer.0],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            usize::try_from(receipt_count).unwrap(),
            crate::storage::PEER_CAPACITY,
            "capacity fixture must retain the exact peer receipt limit"
        );
        drop(seeded);
        let dormant = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let rejected = mutate_with_native_jobs_parked(
            "capacity rejection",
            &dormant,
            fixture.runtime.clone(),
            fixture.auth.clone(),
            original.clone(),
        )
        .await;
        assert_eq!(rejected, Err(ErrorCode::ActionCapacity));
        let proof = fixture.auth.proof.clone();
        let action = original.id.clone();
        assert!(
            fixture
                .runtime
                .security
                .storage
                .call(move |storage| { storage.status(&proof, &action, pairing::now_ms()) })
                .await
                .unwrap()
                .is_none(),
            "failed reserve must leave this UUID unreserved"
        );
        let proof = fixture.auth.proof.clone();
        let action = original.id.clone();
        fixture
            .runtime
            .security
            .storage
            .call(move |storage| {
                storage.status(
                    &proof,
                    &action,
                    pairing::now_ms() + crate::storage::RETENTION_MS,
                )
            })
            .await
            .unwrap();
        let replacement = Request {
            id: original.id.clone(),
            body: RequestBody::TurnCancel(TurnCancelParams {
                boot_id: fixture.runtime.security.boot.clone(),
                target: fixture.connection.target.clone(),
                native_turn_id: None,
            }),
        };
        let original_fingerprint = hmac_sha256(&[23; 32], &canonical_request(&original).unwrap());
        let replacement_fingerprint =
            hmac_sha256(&[23; 32], &canonical_request(&replacement).unwrap());
        assert_ne!(original_fingerprint, replacement_fingerprint);
        let Reservation::Reserved(reserved) = fixture
            .runtime
            .security
            .reserve(
                &fixture.auth,
                replacement.clone(),
                {
                    let runtime = fixture.runtime.clone();
                    move |request| runtime.validate(request)
                },
                |_| {},
            )
            .await
            .unwrap()
        else {
            panic!("different canonical request must reuse the unreserved UUID")
        };
        assert_eq!(reserved.method, ActionMethod::TurnCancel);
        let stored = rusqlite::Connection::open(&fixture.database).unwrap();
        let (fingerprint, epoch): (Vec<u8>, i64) = stored
            .query_row(
                "SELECT fingerprint,epoch FROM receipts WHERE peer=?1 AND action=?2",
                rusqlite::params![fixture.auth.proof.peer_id().0, replacement.id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(fingerprint, replacement_fingerprint);
        assert_eq!(epoch, i64::try_from(fixture.auth.proof.epoch()).unwrap());
        assert_eq!(
            mutate_with_native_jobs_parked(
                "capacity matching receipt",
                &dormant,
                fixture.runtime.clone(),
                fixture.auth.clone(),
                replacement.clone()
            )
            .await
            .unwrap(),
            reserved,
            "matching existing reservation remains zero effects"
        );
        let mut jobs = {
            let mut jobs = fixture.runtime.jobs.lock();
            std::mem::replace(&mut *jobs, JoinSet::new())
        };
        let parked_jobs = tokio::task::spawn_blocking(move || {
            dormant.block_on(async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while let Some(result) = jobs.join_next().await {
                        result.unwrap();
                    }
                })
                .await
            })
        })
        .await
        .unwrap();
        assert!(
            parked_jobs.is_ok(),
            "capacity rejection parked native-job drain exceeded five seconds"
        );
        let proof = fixture.auth.proof.clone();
        let action = replacement.id.clone();
        let after_rejected_jobs = fixture
            .runtime
            .security
            .storage
            .call(move |storage| storage.status(&proof, &action, pairing::now_ms()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after_rejected_jobs, reserved,
            "rejected native original executed or terminalized the different committed fingerprint"
        );
        let dispatched = fixture
            .runtime
            .write_once(
                &fixture.auth,
                replacement.clone(),
                tokio::time::Instant::now() + Duration::from_secs(5),
                fixture.connection.clone(),
                None,
            )
            .await
            .expect("capacity rejection fixture write readiness exceeded five seconds");
        assert_eq!(dispatched.state, ReceiptState::Dispatched);
        assert_eq!(dispatched.method, ActionMethod::TurnCancel);
        let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
            panic!("new native cancellation required")
        };
        assert_eq!(
            native_json::parse(
                fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                NATIVE
            )
            .unwrap(),
            json!({"t":"abort"})
        );
        let duplicate = fixture
            .runtime
            .clone()
            .mutate(fixture.auth.clone(), replacement.clone())
            .await
            .unwrap();
        assert_eq!(duplicate, dispatched);
        let mut jobs = {
            let mut jobs = fixture.runtime.jobs.lock();
            std::mem::replace(&mut *jobs, JoinSet::new())
        };
        let completed_jobs = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(result) = jobs.join_next().await {
                result.unwrap();
            }
        })
        .await;
        assert!(
            completed_jobs.is_ok(),
            "capacity rejection post-write native-job drain exceeded five seconds"
        );
        // Same ordered native writer/relay stream makes an unexpected retry observable,
        // rather than relying on a timeout asserting that nothing happened.
        let marker = json!({"t":"fixture-end"});
        fixture
            .connection
            .writer
            .lock()
            .await
            .send(Message::Binary(
                fixture.connection.link.seal_value(&marker).unwrap().into(),
            ))
            .await
            .unwrap();
        let Message::Binary(binary) = fixture_frame(&mut fixture.host_socket).await else {
            panic!("ordered native boundary marker required")
        };
        assert_eq!(
            native_json::parse(
                fixture.connection.link.open(&mut binary.to_vec()).unwrap(),
                NATIVE
            )
            .unwrap(),
            marker,
            "matching receipt created a second native effect"
        );
        let fingerprint_after: Vec<u8> = stored
            .query_row(
                "SELECT fingerprint FROM receipts WHERE peer=?1 AND action=?2",
                rusqlite::params![fixture.auth.proof.peer_id().0, replacement.id.0],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fingerprint_after, replacement_fingerprint);
        drop(stored);
        fixture.close().await;
    }

    async fn recreated_room_rejects_delayed_guest_owner(old_binary: bool) {
        let relay = Relay::bind(0).await.unwrap();
        let state = relay.state.clone();
        let url = format!("ws://127.0.0.1:{}/r/abcdefghij", relay.port());
        let (stop, rx) = watch::channel(false);
        let task = tokio::spawn(relay.run(rx));
        let (mut old_host, _) = tokio_tungstenite::connect_async(format!("{url}?role=host"))
            .await
            .unwrap();
        let (mut old_guest, _) = tokio_tungstenite::connect_async(format!("{url}?role=guest"))
            .await
            .unwrap();
        assert_eq!(
            fixture_frame(&mut old_host).await.to_text().unwrap(),
            "{\"t\":\"peer-joined\",\"peer\":1}"
        );
        // Perform the host-loss room transition while deliberately deferring notification
        // to the old guest. Its real socket stays alive at the exact ownership race boundary.
        let old_room = state.rooms.lock().remove("abcdefghij").unwrap();
        let old_peer = old_room.guests.get(&1).unwrap().clone();
        let (mut new_host, _) = tokio_tungstenite::connect_async(format!("{url}?role=host"))
            .await
            .unwrap();
        let (mut new_guest, _) = tokio_tungstenite::connect_async(format!("{url}?role=guest"))
            .await
            .unwrap();
        assert_eq!(
            fixture_frame(&mut new_host).await.to_text().unwrap(),
            "{\"t\":\"peer-joined\",\"peer\":1}"
        );
        let new_peer = state
            .rooms
            .lock()
            .get("abcdefghij")
            .unwrap()
            .guests
            .get(&1)
            .unwrap()
            .id
            .clone();
        assert_ne!(old_peer.id, new_peer);
        let packet = |label: &[u8]| [0u32.to_be_bytes().as_slice(), label].concat();
        new_guest
            .send(Message::Binary(packet(b"new-before").into()))
            .await
            .unwrap();
        let Message::Binary(bytes) = fixture_frame(&mut new_host).await else {
            panic!("new guest must route before delayed old work")
        };
        assert_eq!(&bytes[..4], &1u32.to_be_bytes());
        assert_eq!(&bytes[4..], b"new-before");
        let mut response = packet(b"new-targeted-before");
        response[..4].copy_from_slice(&1u32.to_be_bytes());
        new_host
            .send(Message::Binary(response.clone().into()))
            .await
            .unwrap();
        assert_eq!(
            fixture_frame(&mut new_guest).await.into_data().as_ref(),
            response.as_slice()
        );
        if old_binary {
            old_guest
                .send(Message::Binary(packet(b"stale-old-owner").into()))
                .await
                .unwrap();
            old_guest.send(Message::Ping(vec![7].into())).await.unwrap();
            // A Pong fences the processed old input; an ownership-rejection exit is also
            // valid. Receiver closure occurs only after the actual socket handler cleanup.
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    _ = old_peer.tx.closed() => {}
                    frame = old_guest.next() => {
                        match frame {
                            Some(Ok(Message::Pong(_))) => {}
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => old_peer.tx.closed().await,
                            _ => panic!("old guest input barrier received unexpected data"),
                        }
                    }
                }
            }).await.unwrap();
        } else {
            old_guest.close(None).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), old_peer.tx.closed())
                .await
                .unwrap();
            let retained = state
                .rooms
                .lock()
                .get("abcdefghij")
                .unwrap()
                .guests
                .get(&1)
                .map(|peer| peer.id.clone());
            assert_eq!(
                retained,
                Some(new_peer.clone()),
                "delayed old guest closure removed recreated guest ownership"
            );
        }
        new_guest
            .send(Message::Binary(packet(b"new-after").into()))
            .await
            .unwrap();
        let Message::Binary(bytes) = fixture_frame(&mut new_host).await else {
            panic!("old guest must not publish peer-left for the recreated guest")
        };
        assert_eq!(&bytes[..4], &1u32.to_be_bytes());
        assert_eq!(
            &bytes[4..],
            b"new-after",
            "delayed old guest binary routed using the recreated guest numeric ID"
        );
        let mut response = packet(b"new-targeted-after");
        response[..4].copy_from_slice(&1u32.to_be_bytes());
        new_host
            .send(Message::Binary(response.clone().into()))
            .await
            .unwrap();
        assert_eq!(
            fixture_frame(&mut new_guest).await.into_data().as_ref(),
            response.as_slice()
        );
        assert_eq!(
            state
                .rooms
                .lock()
                .get("abcdefghij")
                .unwrap()
                .guests
                .get(&1)
                .unwrap()
                .id,
            new_peer
        );
        drop(old_room);
        drop(old_guest);
        drop(old_host);
        stop.send_replace(true);
        task.abort();
        let _ = task.await;
    }
    #[tokio::test]
    async fn delayed_old_guest_close_cannot_remove_recreated_numeric_guest() {
        recreated_room_rejects_delayed_guest_owner(false).await;
    }
    #[tokio::test]
    async fn delayed_old_guest_binary_cannot_route_as_recreated_numeric_guest() {
        recreated_room_rejects_delayed_guest_owner(true).await;
    }
}

#[cfg(test)]
mod native_answer_tests {
    use super::*;
    #[test]
    fn native_select_and_editor_responses_keep_numeric_ids_and_omit_cancel_value() {
        // Small inline projections need no artificial provider binding/content handle.
        let pending = PendingRequest {
            request_key: Uuid::random(),
            provider: Provider::Omp,
            kind: PresentationKind::OmpSelect,
            presentation: Presentation::OmpSelect(PresentationOmpSelect {
                title: Content::Text(ContentText {
                    text: InlineText("choose".into()),
                }),
                options: vec![SelectOption {
                    value: "offered".into(),
                    label: Label::new("offered").unwrap(),
                    description: None,
                }],
                selection_marker: SelectionMarker::Checkbox,
                help_text: None,
                markable_count: None,
                selected: None,
                checked: vec![],
            }),
        };
        let dialog = Dialog {
            native: NativeRpcId::from_raw("-9007199254740993").unwrap(),
            pending,
        };
        let submit = dialog
            .response(&Answer::OmpSelect(AnswerOmpSelect {
                action: SubmitAction::Submit,
                value: Some("offered".into()),
            }))
            .unwrap();
        assert_eq!(submit["reqId"].to_string(), "-9007199254740993");
        assert_eq!(submit["value"], "offered");
        let cancel = dialog
            .response(&Answer::OmpSelect(AnswerOmpSelect {
                action: SubmitAction::Cancel,
                value: None,
            }))
            .unwrap();
        assert_eq!(cancel["reqId"].to_string(), "-9007199254740993");
        assert!(cancel.get("value").is_none());
        assert!(
            dialog
                .response(&Answer::OmpSelect(AnswerOmpSelect {
                    action: SubmitAction::Submit,
                    value: Some("not offered".into())
                }))
                .is_err()
        );
        let pending = PendingRequest {
            request_key: Uuid::random(),
            provider: Provider::Omp,
            kind: PresentationKind::OmpEditor,
            presentation: Presentation::OmpEditor(PresentationOmpEditor {
                title: Content::Text(ContentText {
                    text: InlineText("edit".into()),
                }),
                prefill: Content::Text(ContentText {
                    text: InlineText("existing".into()),
                }),
            }),
        };
        let editor = Dialog {
            native: NativeRpcId::from_raw("17").unwrap(),
            pending,
        };
        let edited = editor
            .response(&Answer::OmpEditor(AnswerOmpEditor {
                action: SubmitAction::Submit,
                text: Some(PromptText::new("edited\ntext").unwrap()),
            }))
            .unwrap();
        assert_eq!(edited["value"], "edited\ntext");
        let cancelled = editor
            .response(&Answer::OmpEditor(AnswerOmpEditor {
                action: SubmitAction::Cancel,
                text: None,
            }))
            .unwrap();
        assert!(cancelled.get("value").is_none());
    }
}
