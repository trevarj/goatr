// SPDX-License-Identifier: GPL-3.0-or-later
//! One bounded, same-UID local NDJSON operation per private Unix connection.
use crate::{
    pairing::{Security, private_dir, uid},
    protocol::{Uint, Uuid, parse_raw_json},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    task::JoinSet,
};
pub const ADMIN_MAX: usize = 65_536;
pub const ADMIN_TIMEOUT: Duration = Duration::from_secs(5);
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid local admin request")
}
pub fn default_socket() -> io::Result<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(p).join("goatr/admin.sock"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR is unavailable; use --admin-socket PATH",
            )
        })
}
pub fn validate_socket_path(
    path: &Path,
    create_parent: bool,
    require_socket: bool,
) -> io::Result<()> {
    let text = path.to_str().ok_or_else(invalid)?;
    if !path.is_absolute() || text.as_bytes().contains(&0) || path.file_name().is_none() {
        return Err(invalid());
    }
    if text.len() > 107 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Unix socket path exceeds 107 UTF-8 bytes: {path:?}"),
        ));
    }
    private_dir(path.parent().ok_or_else(invalid)?, create_parent)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() && m.uid() == uid() && m.mode() & 0o7777 == 0o600 => {
            Ok(())
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe local admin socket",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound && !require_socket => Ok(()),
        Err(e) => Err(e),
    }
}
pub fn same_uid(stream: &UnixStream) -> io::Result<()> {
    if stream.peer_cred()?.uid() != uid() {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local admin peer is unauthorized",
        ))
    } else {
        Ok(())
    }
}
pub async fn read_line(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    tokio::time::timeout(ADMIN_TIMEOUT, async {
        let mut bytes = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Err(invalid());
            }
            if bytes.len() + n > ADMIN_MAX {
                return Err(invalid());
            }
            if let Some(at) = chunk[..n].iter().position(|b| *b == b'\n') {
                if at + 1 != n {
                    return Err(invalid());
                }
                bytes.extend_from_slice(&chunk[..at]);
                return Ok(bytes);
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "local admin read timed out"))?
}
pub async fn write_line(
    stream: &mut (impl tokio::io::AsyncWrite + Unpin),
    value: &Value,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() + 1 > ADMIN_MAX {
        return Err(invalid());
    }
    bytes.push(b'\n');
    tokio::time::timeout(ADMIN_TIMEOUT, stream.write_all(&bytes))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "local admin write timed out"))?
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminRequest {
    pub v: u64,
    pub id: Uuid,
    pub method: String,
    pub params: Value,
}
pub fn decode_request(bytes: &[u8]) -> io::Result<AdminRequest> {
    if bytes.len() + 1 > ADMIN_MAX {
        return Err(invalid());
    }
    let raw = parse_raw_json(bytes).map_err(|_| invalid())?;
    let r: AdminRequest = serde_json::from_value(raw).map_err(|_| invalid())?;
    if !r.params.is_object() {
        return Err(invalid());
    }
    Ok(r)
}
fn error(id: &Uuid, code: &str) -> Value {
    json!({"v":1,"id":id,"error":{"code":code,"message":"Local admin operation failed"}})
}
async fn respond(security: &Security, r: AdminRequest) -> Value {
    if r.v != 1 {
        return error(&r.id, "unsupported_version");
    }
    let result=match r.method.as_str() {
        "pair.open" if r.params==json!({}) => security.open_pair().await.and_then(|p|serde_json::to_value(p).map_err(|_|crate::protocol::ErrorCode::NotReady)),
        "devices.list" if r.params==json!({}) => security.devices().await.map(|devices|json!({"devices":devices.into_iter().map(|d| {let mut v=json!({"deviceId":d.device_id,"endpointId":d.endpoint_id,"deviceName":d.device_name,"pairedAt":d.paired_at}); if let Some(at)=d.last_seen_at {v["lastSeenAt"]=json!(at);} v}).collect::<Vec<_>>()})),
        "device.revoke" => {
            #[derive(Deserialize)] #[serde(deny_unknown_fields,rename_all="camelCase")] struct Revoke {device_id:Uuid}
            match serde_json::from_value::<Revoke>(r.params) {Ok(p)=>security.revoke(p.device_id,None).await.map(|revoked|json!({"revoked":revoked})),Err(_)=>return error(&r.id,"invalid_request")}
        }
        "pair.open"|"devices.list"=>return error(&r.id,"invalid_request"),
        _=>return error(&r.id,"unsupported_method"),
    };
    match result {
        Ok(result) => json!({"v":1,"id":r.id,"result":result}),
        Err(crate::protocol::ErrorCode::Unauthorized) => error(&r.id, "unauthorized"),
        Err(crate::protocol::ErrorCode::Busy) => error(&r.id, "busy"),
        Err(crate::protocol::ErrorCode::Timeout) => error(&r.id, "timeout"),
        Err(_) => error(&r.id, "internal_error"),
    }
}
/// Binding never adopts/unlinks an existing socket, even a stale-looking one.
pub struct AdminSocket {
    listener: UnixListener,
    path: PathBuf,
    dev: u64,
    ino: u64,
}
impl AdminSocket {
    pub fn bind(path: PathBuf) -> io::Result<Self> {
        validate_socket_path(&path, true, false)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "admin socket already exists; do not unlink an unproven owner",
            ));
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        validate_socket_path(&path, false, true)?;
        let m = fs::symlink_metadata(&path)?;
        Ok(Self {
            listener,
            path,
            dev: m.dev(),
            ino: m.ino(),
        })
    }
    pub async fn serve(
        &self,
        security: Arc<Security>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> io::Result<()> {
        let admission = Arc::new(Semaphore::new(8));
        let mut jobs = JoinSet::new();
        loop {
            tokio::select! {
                changed=stop.changed()=>{if changed.is_err() || *stop.borrow() {break;}},
                accepted=self.listener.accept()=>{let (mut stream,_)=accepted?; let Ok(permit)=admission.clone().try_acquire_owned() else {continue;}; if same_uid(&stream).is_err() {continue;} let security=security.clone(); jobs.spawn(async move {let _permit=permit; if let Ok(bytes)=read_line(&mut stream).await && let Ok(request)=decode_request(&bytes) {let reply=respond(&security,request).await; let _=write_line(&mut stream,&reply).await;} let _=stream.shutdown().await;});},
                Some(_)=jobs.join_next(),if !jobs.is_empty()=>{},
            }
        }
        jobs.abort_all();
        while jobs.join_next().await.is_some() {}
        Ok(())
    }
}
impl Drop for AdminSocket {
    fn drop(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.path)
            && m.file_type().is_socket()
            && m.uid() == uid()
            && m.dev() == self.dev
            && m.ino() == self.ino
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// Returns only a checked correlated local result; this never opens durable state.
pub async fn call(path: &Path, method: &str, params: Value) -> io::Result<Value> {
    validate_socket_path(path, false, true)?;
    let mut stream = tokio::time::timeout(ADMIN_TIMEOUT, UnixStream::connect(path))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "local admin connect timed out"))??;
    same_uid(&stream)?;
    validate_socket_path(path, false, true)?;
    let id = Uuid::random();
    write_line(
        &mut stream,
        &json!({"v":1,"id":id,"method":method,"params":params}),
    )
    .await?;
    let bytes = read_line(&mut stream).await?;
    let value = parse_raw_json(&bytes).map_err(|_| invalid())?;
    let o = value.as_object().ok_or_else(invalid)?;
    if !o
        .keys()
        .all(|k| matches!(k.as_str(), "v" | "id" | "result" | "error"))
        || o.get("v") != Some(&json!(1))
        || o.get("id") != Some(&json!(id))
        || o.contains_key("result") == o.contains_key("error")
    {
        return Err(invalid());
    }
    if let Some(result) = o.get("result") {
        return Ok(result.clone());
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LocalError {
        code: String,
        message: String,
    }
    let e: LocalError = serde_json::from_value(o["error"].clone()).map_err(|_| invalid())?;
    if !matches!(
        e.code.as_str(),
        "invalid_request"
            | "unsupported_version"
            | "unsupported_method"
            | "unauthorized"
            | "busy"
            | "timeout"
            | "internal_error"
    ) || e.message.len() > 512
    {
        return Err(invalid());
    }
    Err(io::Error::other(format!("local admin {}", e.code)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Device {
    pub device_id: Uuid,
    pub endpoint_id: crate::protocol::EndpointId,
    pub device_name: crate::protocol::Label,
    pub paired_at: Uint,
    pub last_seen_at: Option<Uint>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Devices {
    pub devices: Vec<Device>,
}
