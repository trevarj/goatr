// SPDX-License-Identifier: GPL-3.0-or-later
//! Single blocking SQLite owner. Run on a bounded blocking queue, never the transport reader.
//! Package3 owns authenticated hello and the live per-peer dispatch fence; the epoch is checked
//! in every transaction here. A database commit is not proof of a native write/acknowledgement.
//! SQLite INTEGER boundaries are checked; public epochs and wire Uint values stay unsigned.
use crate::protocol::*;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    time::Duration,
};

mod push;

pub const RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const PEER_CAPACITY: usize = 10_000;
pub const MIN_SQLITE_VERSION: i32 = 3_034_001;
#[derive(Debug)]
pub enum StorageError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Corrupt,
    Policy(ErrorCode),
    UnsafePath,
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Policy(code) => write!(f, "state policy: {code:?}"),
            Self::Database(_) => f.write_str("SQLite state operation failed"),
            Self::Io(_) => f.write_str("private state file operation failed"),
            Self::Corrupt => f.write_str("invalid durable state"),
            Self::UnsafePath => f.write_str("unsafe private state path"),
        }
    }
}
impl std::error::Error for StorageError {}
impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}
impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<ProtocolFailure> for StorageError {
    fn from(_: ProtocolFailure) -> Self {
        Self::Policy(ErrorCode::InvalidRequest)
    }
}
pub type StoreResult<T> = Result<T, StorageError>;
fn reject<T>(code: ErrorCode) -> StoreResult<T> {
    Err(StorageError::Policy(code))
}
fn json<T: Serialize>(v: &T) -> StoreResult<String> {
    serde_json::to_string(v).map_err(|_| StorageError::Corrupt)
}
fn decode<T: DeserializeOwned>(v: &str) -> StoreResult<T> {
    serde_json::from_str(v).map_err(|_| StorageError::Corrupt)
}
fn timestamp(now: u64) -> StoreResult<Uint> {
    Ok(Uint::new(now)?)
}
fn sql_integer(value: impl TryInto<i64>) -> StoreResult<i64> {
    value
        .try_into()
        .map_err(|_| StorageError::Policy(ErrorCode::InvalidRequest))
}
fn stored_u64(value: i64) -> StoreResult<u64> {
    value.try_into().map_err(|_| StorageError::Corrupt)
}
fn stored_uint(value: i64) -> StoreResult<u64> {
    let value = stored_u64(value)?;
    if value > UINT_MAX {
        return Err(StorageError::Corrupt);
    }
    Ok(value)
}

/// Minted only after package3 has authenticated the remote endpoint and completed hello.
/// Fields cannot be supplied directly by a network request. Every database use rechecks epoch.
#[derive(Clone, Debug)]
pub struct AuthenticatedPeer {
    peer: Uuid,
    epoch: u64,
}
impl AuthenticatedPeer {
    pub fn peer_id(&self) -> &Uuid {
        &self.peer
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub device_id: Uuid,
    pub endpoint_id: EndpointId,
    pub device_name: Label,
    pub paired_at: Uint,
    pub last_seen_at: Option<Uint>,
}
#[derive(Debug)]
pub enum Reservation {
    Existing(Receipt),
    Reserved(Receipt),
}
/// These assertions must be backed by the adapter's correlated native evidence. They are not
/// generated from timeout, dialog closure, client retry, or an absent local receipt.
pub enum TransitionProof {
    Intent,
    WriteCompleted,
    CorrelatedAcknowledgement(Id),
    /// Controls only: same-original native effective settings, never a queued-update ACK.
    NativeEffectiveState(Id),
    DefinitiveNoEffect(Id),
    LostOutcome,
    ReconciledSuccess(Id),
    ReconciledNoEffect(Id),
}
pub struct ReceiptTransition {
    pub state: ReceiptState,
    pub proof: TransitionProof,
    pub result: Option<ActionResult>,
    pub error: Option<WireError>,
    pub at: u64,
}
pub struct CreationOutcome {
    pub resources: CreationResources,
    pub metadata: StageMetadata,
    pub state: StageState,
    pub definitive: Option<Id>,
    pub at: u64,
}
pub enum CreationReconciliationProof {
    Ready(Id),
    NoEffect(Id),
}
pub struct CreationReconciliation {
    pub resources: CreationResources,
    pub proof: CreationReconciliationProof,
    pub metadata: StageMetadata,
    pub at: u64,
}
/// Mint only under3's entered live dispatch fence, after durable intent and target recheck.
/// Non-clone, not reconstructible from network credentials; settlement never returns metadata.
pub struct DispatchTicket {
    proof: AuthenticatedPeer,
    action: Uuid,
    stage: Option<CreationStage>,
    workspace_operation: WorkspaceOperation,
    probe_attempt: Option<Uuid>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceOperation {
    #[default]
    Create,
    ShellProbe,
}
/// Host-local ownership only. No raw nonce, command, result, aliases or native payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellProbeMetadata {
    pub attempt_id: Uuid,
    pub action_id: Uuid,
    pub workspace_id: Id,
    pub pane_id: Id,
    pub terminal_id: Id,
    pub shell: crate::herdr::ProcessEvidence,
    pub executable: String,
    pub result_path: String,
    pub result_device: u64,
    pub result_inode: u64,
    pub parent_device: u64,
    pub parent_inode: u64,
    pub focused_workspace_id: Id,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StageMetadata {
    #[serde(default)]
    pub execution_checkout: Option<crate::herdr::ExecutionCheckout>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub created_tab_id:Option<Id>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub placement_workspace_id:Option<Id>,
    pub socket_epoch: Option<Uuid>,
    pub linux_boot_id: Option<Uuid>,
    pub pid: Option<u32>,
    pub pid_start_ticks: Option<u64>,
    pub socket_path: Option<String>,
    pub unit_name: Option<String>,
    #[serde(default)]
    pub workspace_operation: WorkspaceOperation,
    #[serde(default)]
    pub shell_probe: Option<ShellProbeMetadata>,
    #[serde(default)]
    pub focused_workspace_id: Option<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreationStageRecord {
    pub stage: CreationStage,
    pub intent: StageMetadata,
    pub result: Option<StageMetadata>,
    pub state: StageState,
    pub proof: Option<Id>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationLocal {
    pub alias: String,
    pub stages: Vec<CreationStageRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerEvidence {
    pub linux_boot_id: Uuid,
    pub pid: u32,
    pub pid_start_ticks: u64,
    pub socket_device: u64,
    pub socket_inode: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableSessionKey {
    pub host_id: EndpointId,
    pub server_id: Uuid,
    pub server_incarnation: Uuid,
    pub terminal_id: Id,
    pub provider_owner_identity: Id,
    pub native_conversation_id: Id,
}
/// The existing finite mapping owner serves two explicitly separated resource kinds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag="kind",content="key",rename_all="snake_case",deny_unknown_fields)]
pub enum StableResourceKey {
    Agent(StableSessionKey),
    Shell {host_id:EndpointId,server_id:Uuid,server_incarnation:Uuid,terminal_id:Id},
}
impl StableResourceKey {
    fn kind(&self)->&'static str {match self {Self::Agent(_)=>"agent",Self::Shell {..}=>"shell"}}
    pub(crate) fn server(&self)->&Uuid {match self {Self::Agent(key)=>&key.server_id,Self::Shell {server_id,..}=>server_id}}
    fn validate_key(&self)->StoreResult<()> {
        match self {
            Self::Agent(key)=>{key.host_id.validate()?;key.server_id.validate()?;key.server_incarnation.validate()?;key.terminal_id.validate()?;key.provider_owner_identity.validate()?;key.native_conversation_id.validate()?;}
            Self::Shell {host_id,server_id,server_incarnation,terminal_id}=>{host_id.validate()?;server_id.validate()?;server_incarnation.validate()?;terminal_id.validate()?;}
        }Ok(())
    }
}
#[derive(Clone,Debug)]
pub struct PersistedShell {pub key:StableResourceKey,pub target:ShellTarget,pub proof:crate::herdr::ShellProof}
#[derive(Clone, Debug)]
pub struct ServerIdentity {
    pub server_id: Uuid,
    pub incarnation: Uuid,
}
#[derive(Clone, Debug)]
pub struct PersistedServer {
    pub slot: String,
    pub server_id: Uuid,
    pub incarnation: Option<Uuid>,
    pub evidence: Option<ServerEvidence>,
    pub name: Option<Label>,
}
#[derive(Clone, Debug)]
pub struct PersistedSession {
    pub key: StableSessionKey,
    pub target: Target,
    pub display: Option<SessionState>,
}
#[derive(Clone, Debug)]
pub struct AttentionTransition {
    pub target: Target,
    pub correlation: Id,
    pub previous: SessionStatus,
    pub current: SessionStatus,
    pub snapshot: bool,
    pub new_actionable: bool,
}

pub struct Storage {
    db: Connection,
    receipt_key: [u8; 32],
    pub boot_id: Uuid,
    image_reads:Vec<std::sync::Arc<crate::attachments::ImageReadObligation>>,
    image_budget:std::sync::Arc<parking_lot::Mutex<crate::attachments::ImageBudget>>,
    image_reads_restored:bool,
    image_reads_fault:bool,
}
type StorageJob = Box<dyn FnOnce(&mut Storage) + Send>;
/// Bounded blocking owner: transport tasks await completion without running SQLite on their
/// reader. Dropped responses do not undo committed reservations or authorize replay.
pub struct StorageOwner {
    sender: tokio::sync::mpsc::Sender<StorageJob>,
    worker: std::thread::JoinHandle<()>,
}
pub(crate) struct StorageAdmission<'a>(tokio::sync::mpsc::Permit<'a, StorageJob>);
impl StorageAdmission<'_> {
    pub(crate) async fn call<T: Send + 'static>(
        self,
        operation: impl FnOnce(&mut Storage) -> StoreResult<T> + Send + 'static,
    ) -> StoreResult<T> {
        let (sender, result) = tokio::sync::oneshot::channel();
        self.0.send(Box::new(move |store| {
            let _ = sender.send(operation(store));
        }));
        result.await.map_err(|_| StorageError::Corrupt)?
    }
}
impl StorageOwner {
    pub fn new(mut storage: Storage) -> StoreResult<Self> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<StorageJob>(64);
        let worker = std::thread::Builder::new()
            .name("goatr-sqlite".into())
            .spawn(move || {
                while let Some(job) = receiver.blocking_recv() {
                    job(&mut storage);
                }
            })?;
        Ok(Self { sender, worker })
    }
    pub async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Storage) -> StoreResult<T> + Send + 'static,
    ) -> StoreResult<T> {
        StorageAdmission(self.sender.try_reserve().map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => {
                StorageError::Policy(ErrorCode::Busy)
            }
            tokio::sync::mpsc::error::TrySendError::Closed(_) => StorageError::Corrupt,
        })?)
        .call(operation)
        .await
    }
    /// Settlement waits for bounded queue admission before surrendering its unique ticket.
    pub(crate) async fn admission(&self) -> StoreResult<StorageAdmission<'_>> {
        self.sender
            .reserve()
            .await
            .map(StorageAdmission)
            .map_err(|_| StorageError::Corrupt)
    }
    pub async fn close(self) -> StoreResult<()> {
        drop(self.sender);
        tokio::task::spawn_blocking(move || self.worker.join())
            .await
            .map_err(|_| StorageError::Corrupt)?
            .map_err(|_| StorageError::Corrupt)
    }
}
impl Storage {
    /// Parent must already be canonical, same-UID, private0700. Package3 supplies the UID and
    /// separately retained random receipt key; losing that key is fatal, not a re-key shortcut.
    pub fn open(
        path: &Path,
        receipt_key: [u8; 32],
        now: u64,
        expected_uid: u32,
    ) -> StoreResult<Self> {
        let parent = path.parent().ok_or(StorageError::UnsafePath)?;
        if !path.is_absolute() || fs::canonicalize(parent)? != parent {
            return Err(StorageError::UnsafePath);
        }
        let metadata = fs::symlink_metadata(parent)?;
        if !metadata.is_dir() || metadata.uid() != expected_uid || metadata.mode() & 0o777 != 0o700
        {
            return Err(StorageError::UnsafePath);
        }
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let file = std::path::PathBuf::from(format!(
                "{}{suffix}",
                path.to_str().ok_or(StorageError::UnsafePath)?
            ));
            if let Ok(meta) = fs::symlink_metadata(&file)
                && (!meta.is_file() || meta.uid() != expected_uid || meta.mode() & 0o777 != 0o600)
            {
                return Err(StorageError::UnsafePath);
            }
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        let db = Connection::open(path)?;
        Self::initialize(db, receipt_key, now)
    }
    pub fn memory(receipt_key: [u8; 32], now: u64) -> StoreResult<Self> {
        Self::initialize(Connection::open_in_memory()?, receipt_key, now)
    }
    fn initialize(mut db: Connection, receipt_key: [u8; 32], now: u64) -> StoreResult<Self> {
        timestamp(now)?;
        if rusqlite::version_number() < MIN_SQLITE_VERSION {
            return Err(StorageError::Corrupt);
        }
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS peers(peer TEXT PRIMARY KEY,epoch INTEGER NOT NULL,paired INTEGER NOT NULL,highwater INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS devices(peer TEXT PRIMARY KEY,endpoint TEXT NOT NULL UNIQUE,metadata TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS private_registration(peer TEXT PRIMARY KEY,generation TEXT NOT NULL,ciphertext BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS receipts(peer TEXT NOT NULL,action TEXT NOT NULL,epoch INTEGER NOT NULL,fingerprint BLOB NOT NULL,updated INTEGER NOT NULL,state TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(peer,action));
            CREATE TABLE IF NOT EXISTS creation(peer TEXT NOT NULL,action TEXT NOT NULL,alias TEXT NOT NULL UNIQUE,stages TEXT NOT NULL,PRIMARY KEY(peer,action),FOREIGN KEY(peer,action) REFERENCES receipts(peer,action) ON DELETE CASCADE);
            CREATE TABLE IF NOT EXISTS servers(slot TEXT PRIMARY KEY,server TEXT NOT NULL,evidence TEXT NOT NULL,incarnation TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS sessions(host TEXT NOT NULL,server TEXT NOT NULL,incarnation TEXT NOT NULL,terminal TEXT NOT NULL,owner TEXT NOT NULL,native TEXT NOT NULL,session TEXT NOT NULL UNIQUE,generation TEXT NOT NULL,PRIMARY KEY(host,server,incarnation,terminal,owner,native));
            CREATE TABLE IF NOT EXISTS checkpoint(session TEXT PRIMARY KEY,generation TEXT NOT NULL,status TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS transitions(session TEXT NOT NULL,correlation TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(session,correlation));
            CREATE TABLE IF NOT EXISTS attention(peer TEXT NOT NULL,sequence INTEGER NOT NULL,event TEXT NOT NULL,session TEXT NOT NULL,at INTEGER NOT NULL,received INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(peer,sequence));
            CREATE TABLE IF NOT EXISTS readwater(peer TEXT NOT NULL,session TEXT NOT NULL,through INTEGER NOT NULL,last_event INTEGER NOT NULL,PRIMARY KEY(peer,session));
            CREATE TABLE IF NOT EXISTS retired_push(peer TEXT NOT NULL,generation TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(peer,generation));
            CREATE TABLE IF NOT EXISTS topology_server_display(server TEXT PRIMARY KEY,name TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS topology_session_display(session TEXT PRIMARY KEY,state TEXT NOT NULL);")?;
        let version: Option<Vec<u8>> = db
            .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if version.as_deref().is_some_and(|v| v != b"1") {
            return Err(StorageError::Corrupt);
        }
        let key_check = hmac_sha256(&receipt_key, b"goatr receipt key verification v1");
        let stored: Option<Vec<u8>> = db
            .query_row(
                "SELECT value FROM meta WHERE key='receipt_key_check'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if stored.is_some_and(|check| {
            !verify_hmac_sha256(&receipt_key, b"goatr receipt key verification v1", &check)
        }) {
            return Err(StorageError::Corrupt);
        }
        db.execute_batch("CREATE TABLE IF NOT EXISTS attachments(peer TEXT NOT NULL,upload TEXT NOT NULL,size INTEGER NOT NULL,unfinished INTEGER NOT NULL,discarded INTEGER NOT NULL,record TEXT NOT NULL,PRIMARY KEY(peer,upload));")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS hc_grants(peer TEXT PRIMARY KEY,epoch INTEGER NOT NULL);")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS attachment_image_uses(peer TEXT NOT NULL,action TEXT NOT NULL,native_input TEXT NOT NULL UNIQUE,record TEXT NOT NULL,mac BLOB NOT NULL,PRIMARY KEY(peer,action));")?;
        // Receipt-owned negative shell admission obligations outlive receipt TTL/revocation.
        // No terminal text, paths or new remote authority is stored here.
        db.execute_batch("CREATE TABLE IF NOT EXISTS creation_shell_fences(peer TEXT NOT NULL,action TEXT NOT NULL,fence TEXT NOT NULL,PRIMARY KEY(peer,action));")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS push_jobs(peer TEXT NOT NULL,id TEXT NOT NULL,generation TEXT NOT NULL,probe INTEGER NOT NULL,attempts INTEGER NOT NULL,due INTEGER NOT NULL,expires INTEGER NOT NULL,PRIMARY KEY(peer,id));
            CREATE TABLE IF NOT EXISTS push_sync(peer TEXT PRIMARY KEY,required INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS attention_omp_cycles(session TEXT PRIMARY KEY,generation TEXT NOT NULL,source TEXT NOT NULL,connection TEXT NOT NULL,ordinal INTEGER NOT NULL,active TEXT);
CREATE INDEX IF NOT EXISTS push_jobs_due ON push_jobs(due,peer);
CREATE INDEX IF NOT EXISTS push_jobs_expiry ON push_jobs(expires);")?;
        db.execute(
            "INSERT OR IGNORE INTO meta VALUES('version',?1)",
            [b"1".as_slice()],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO meta VALUES('receipt_key_check',?1)",
            [key_check.as_slice()],
        )?;
        migrate_resource_mappings(&mut db)?;
        // A process restart cannot resume a prior observed live OMP cycle.
        db.execute("UPDATE attention_omp_cycles SET active=NULL",[])?;
        let mut store = Self {
            db,
            receipt_key,
            boot_id: Uuid::random(),
            image_reads:Vec::with_capacity(crate::attachments::IMAGE_ACTIONS_PER_HOST),
            image_budget:std::sync::Arc::new(parking_lot::Mutex::new(crate::attachments::ImageBudget::default())),
            image_reads_restored:false,image_reads_fault:false,
        };
        let tx = store
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows: Vec<(String, String, String)> = {
            let mut s = tx.prepare("SELECT peer,action,receipt FROM receipts WHERE state IN ('reserved','dispatching')")?;
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?
        };
        for (peer, action, data) in rows {
            let mut receipt: Receipt = decode(&data)?;
            // Durable Reserved precedes native intent. A crashed pre-entry job cannot
            // leave a negative unknown-pane fence; its historical receipt stays Uncertain.
            if receipt.state==ReceiptState::Reserved {
                tx.execute("DELETE FROM creation_shell_fences WHERE peer=?1 AND action=?2",params![peer,action])?;
            }
            let complete_input=receipt.state==ReceiptState::Dispatching&&receipt.method==ActionMethod::ShellInput
                &&matches!(&receipt.result,Some(ActionResult::ShellDispatch(result)) if result.progress.complete())
                &&receipt.validate().is_ok();
            receipt.state=if complete_input{ReceiptState::Dispatched}else{ReceiptState::Uncertain};
            receipt.updated_at = timestamp(now)?;
            if let Some(creation) = &mut receipt.creation
                && creation.stage_state == StageState::Dispatching
            {
                creation.stage_state = StageState::Uncertain;
            }
            write_receipt(&tx, &peer, &action, &receipt)?;
        }
        let sessions: Vec<String> = {
            let mut s = tx.prepare("SELECT session FROM sessions")?;
            s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        for session in sessions {
            tx.execute(
                "UPDATE sessions SET generation=?1 WHERE session=?2",
                params![Uuid::random().0, session],
            )?;
        }
        prune_all(&tx, now)?;
        tx.commit()?;
        store.attachment_maintenance(now,true)?;
        Ok(store)
    }
    /// Pair token consumption belongs to3's same fence; commit this before consuming the token
    /// or replying. A lost reply recovers this endpoint binding via a later authenticated hello.
    pub fn pair_device(&mut self, device: &PairedDevice) -> StoreResult<u64> {
        self.pair_device_authority(device, false)
    }
    /// Only a freshly redeemed HC invitation may grant full desktop authority.
    pub fn pair_device_authority(&mut self, device: &PairedDevice, hc: bool) -> StoreResult<u64> {
        device.device_id.validate()?;
        device.endpoint_id.validate()?;
        device.device_name.validate()?;
        device.paired_at.validate()?;
        device.last_seen_at.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let collision: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE endpoint=?1 AND peer<>?2)",
            params![device.endpoint_id.0, device.device_id.0],
            |r| r.get(0),
        )?;
        if collision {
            return reject(ErrorCode::InvalidRequest);
        }
        let old: Option<(i64, bool)> = tx
            .query_row(
                "SELECT epoch,paired FROM peers WHERE peer=?1",
                [&device.device_id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let old = old
            .map(|(epoch, paired)| -> StoreResult<_> { Ok((stored_u64(epoch)?, paired)) })
            .transpose()?;
        let existing_endpoint: Option<String> = tx
            .query_row(
                "SELECT endpoint FROM devices WHERE peer=?1",
                [&device.device_id.0],
                |r| r.get(0),
            )
            .optional()?;
        if existing_endpoint
            .as_ref()
            .is_some_and(|id| id != &device.endpoint_id.0)
        {
            return reject(ErrorCode::InvalidRequest);
        }
        if old.is_some_and(|(_, paired)| !paired) {
            return reject(ErrorCode::InvalidRequest);
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM peers WHERE paired=1", [], |r| {
            r.get(0)
        })?;
        if !old.is_some_and(|(_, p)| p) && count >= 32 {
            return reject(ErrorCode::ResourceLimit);
        }
        let epoch = match old {
            Some((e, true)) => e,
            Some((e, false)) => e.checked_add(1).ok_or(StorageError::Corrupt)?,
            None => 1,
        };
        tx.execute("INSERT INTO peers(peer,epoch,paired) VALUES(?1,?2,1) ON CONFLICT(peer) DO UPDATE SET epoch=excluded.epoch,paired=1", params![device.device_id.0,sql_integer(epoch)?])?;
        tx.execute("INSERT INTO devices VALUES(?1,?2,?3) ON CONFLICT(peer) DO UPDATE SET metadata=excluded.metadata", params![device.device_id.0,device.endpoint_id.0,json(device)?])?;
        if hc {
            tx.execute("INSERT INTO hc_grants(peer,epoch) VALUES(?1,?2) ON CONFLICT(peer) DO UPDATE SET epoch=excluded.epoch",
                params![device.device_id.0, sql_integer(epoch)?])?;
        }
        tx.commit()?;
        Ok(epoch)
    }
    pub fn require_hc(&self, proof: &AuthenticatedPeer) -> StoreResult<()> {
        self.recheck(proof)?;
        let granted: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hc_grants WHERE peer=?1 AND epoch=?2)",
            params![proof.peer.0, sql_integer(proof.epoch)?], |row| row.get(0))?;
        if !granted { return reject(ErrorCode::Unauthorized); }
        Ok(())
    }
    /// Local admin/security-only inventory, not a network operation.
    pub fn devices(&self) -> StoreResult<Vec<PairedDevice>> {
        let mut query = self
            .db
            .prepare("SELECT metadata FROM devices JOIN peers USING(peer) WHERE paired=1")?;
        let rows: Vec<String> = query
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut devices: Vec<PairedDevice> =
            rows.iter().map(|s| decode(s)).collect::<StoreResult<_>>()?;
        devices.sort_by_key(|d| d.paired_at);
        Ok(devices)
    }
    pub fn current_epoch(&self, remote: &EndpointId) -> StoreResult<Option<(Uuid, u64)>> {
        let row: Option<(String,i64)> = self.db.query_row("SELECT peer,epoch FROM devices JOIN peers USING(peer) WHERE endpoint=?1 AND paired=1", [&remote.0], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        row.map(|(id, e)| Ok((Uuid::new(id)?, stored_u64(e)?)))
            .transpose()
    }
    pub fn mark_seen(
        &mut self,
        proof: &AuthenticatedPeer,
        name: Label,
        now: u64,
    ) -> StoreResult<()> {
        name.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let data: String = tx.query_row(
            "SELECT metadata FROM devices WHERE peer=?1",
            [&proof.peer.0],
            |r| r.get(0),
        )?;
        let mut device: PairedDevice = decode(&data)?;
        device.device_name = name;
        device.last_seen_at = Some(timestamp(now)?);
        tx.execute(
            "UPDATE devices SET metadata=?1 WHERE peer=?2",
            params![json(&device)?, proof.peer.0],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn encrypted_registration(
        &mut self,
        proof: &AuthenticatedPeer,
        generation: &Uuid,
        ciphertext: &[u8],
        now: u64,
    ) -> StoreResult<()> {
        if ciphertext.is_empty() || ciphertext.len() > 65_536 {
            return reject(ErrorCode::InvalidRequest);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let retired: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM retired_push WHERE peer=?1 AND generation=?2)",
            params![proof.peer.0, generation.0],
            |r| r.get(0),
        )?;
        if retired {
            return reject(ErrorCode::StaleRegistration);
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT generation FROM private_registration WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous.filter(|id| id != &generation.0) {
            if now >= RETENTION_MS {
                tx.execute(
                    "DELETE FROM retired_push WHERE peer=?1 AND at<=?2",
                    params![proof.peer.0, sql_integer(now - RETENTION_MS)?],
                )?;
            }
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM retired_push WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )?;
            if count >= sql_integer(PEER_CAPACITY)? {
                return reject(ErrorCode::ResourceLimit);
            }
            tx.execute(
                "INSERT OR IGNORE INTO retired_push VALUES(?1,?2,?3)",
                params![proof.peer.0, previous, sql_integer(now)?],
            )?;
        }
        tx.execute("INSERT INTO private_registration VALUES(?1,?2,?3) ON CONFLICT(peer) DO UPDATE SET generation=excluded.generation,ciphertext=excluded.ciphertext", params![proof.peer.0,generation.0,ciphertext])?;
        tx.commit()?;
        Ok(())
    }
    pub fn registration(&self, proof: &AuthenticatedPeer) -> StoreResult<Option<(Uuid, Vec<u8>)>> {
        authorize(&self.db, proof)?;
        let row: Option<(String, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT generation,ciphertext FROM private_registration WHERE peer=?1",
                [&proof.peer.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(id, bytes)| Ok((Uuid::new(id)?, bytes)))
            .transpose()
    }
    /// The security caller must already have verified Connection.remote_id and completed hello.
    pub fn authenticated_hello(&self, peer: &Uuid, epoch: u64) -> StoreResult<AuthenticatedPeer> {
        let proof = AuthenticatedPeer {
            peer: peer.clone(),
            epoch,
        };
        authorize(&self.db, &proof)?;
        Ok(proof)
    }
    pub fn recheck(&self, proof: &AuthenticatedPeer) -> StoreResult<()> {
        authorize(&self.db, proof)
    }
    /// Package3 holds its dispatch fence while committing removal, then closes state/streams and
    /// settles already-entered bounded native boundaries before returning local revoke success.
    pub fn revoke_epoch(&mut self, peer: &Uuid, now: u64) -> StoreResult<bool> {
        timestamp(now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let epoch: Option<i64> = tx
            .query_row(
                "SELECT epoch FROM peers WHERE peer=?1 AND paired=1",
                [&peer.0],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(epoch) = epoch {
            stored_u64(epoch)?;
            let next_epoch = epoch.checked_add(1).ok_or(StorageError::Corrupt)?;
            tx.execute(
                "UPDATE peers SET epoch=?1,paired=0 WHERE peer=?2 AND paired=1",
                params![next_epoch, peer.0],
            )?;
        }
        // Reserved proves no entered effect. A settled intermediate submission has no remaining
        // entered ticket: revoke must retain its known resources as uncertain, not no-effect.
        let reserved = {
            let mut query = tx.prepare(
                "SELECT action,receipt FROM receipts WHERE peer=?1 AND state IN ('reserved','dispatching')",
            )?;
            query
                .query_map([&peer.0], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (action, data) in reserved {
            let mut receipt: Receipt = decode(&data)?;
            if receipt.state == ReceiptState::Dispatching {
                let Some(progress) = receipt.creation.as_ref() else {
                    continue;
                };
                let confirmed = progress.stage_state == StageState::Confirmed;
                let failed = progress.stage_state == StageState::Failed;
                let data: String = tx.query_row(
                    "SELECT stages FROM creation WHERE peer=?1 AND action=?2",
                    params![peer.0, action],
                    |r| r.get(0),
                )?;
                let mut records: Vec<CreationStageRecord> = decode(&data)?;
                let last = records.last_mut().ok_or(StorageError::Corrupt)?;
                let submitted = progress.stage_state == StageState::Dispatching
                    && intermediate_creation_stage(
                        progress.provider,
                        progress.stage,
                        last.intent.workspace_operation,
                    );
                if !confirmed && !submitted && !failed {
                    continue;
                }
                if last.result.is_none() || last.proof.is_none() {
                    continue;
                }
                if last.stage != progress.stage || last.state != progress.stage_state {
                    return Err(StorageError::Corrupt);
                }
                if failed {
                    receipt.state = ReceiptState::Failed;
                    receipt.updated_at = timestamp(now.max(receipt.updated_at.0))?;
                    receipt.error = Some(WireError {
                        code: ErrorCode::BackendError,
                        message: SafeMessage::new(
                            "Definitive creation stage rejection; earlier resources retained",
                        )?,
                    });
                    write_receipt(&tx, &peer.0, &action, &receipt)?;
                    continue;
                }
                if confirmed {
                    receipt.updated_at = timestamp(now.max(receipt.updated_at.0))?;
                    if progress.stage == CreationStage::Ready {
                        receipt.result = Some(ready_result(&progress.resources)?);
                        receipt.state = ReceiptState::Succeeded;
                        receipt.error = None;
                    } else {
                        receipt.state = ReceiptState::Failed;
                        receipt.error = Some(WireError {
                            code: ErrorCode::Unauthorized,
                            message: SafeMessage::new(
                                "Authorization removed before the next stage; confirmed resources retained",
                            )?,
                        });
                    }
                    write_receipt(&tx, &peer.0, &action, &receipt)?;
                    continue;
                }
                receipt.state = ReceiptState::Uncertain;
                receipt
                    .creation
                    .as_mut()
                    .ok_or(StorageError::Corrupt)?
                    .stage_state = StageState::Uncertain;
                receipt.updated_at = timestamp(now.max(receipt.updated_at.0))?;
                last.state = StageState::Uncertain;
                tx.execute(
                    "UPDATE creation SET stages=?1 WHERE peer=?2 AND action=?3",
                    params![json(&records)?, peer.0, action],
                )?;
                write_receipt(&tx, &peer.0, &action, &receipt)?;
                continue;
            }
            if receipt.state != ReceiptState::Reserved {
                return Err(StorageError::Corrupt);
            }
            receipt.state = ReceiptState::Failed;
            receipt.updated_at = timestamp(now.max(receipt.updated_at.0))?;
            receipt.error = Some(WireError {
                code: ErrorCode::Unauthorized,
                message: SafeMessage::new("Device authorization was removed before dispatch")?,
            });
            if let Some(creation) = receipt.creation.as_mut() {
                creation.stage_state = StageState::Failed;
            }
            write_receipt(&tx, &peer.0, &action, &receipt)?;
        }
        let changed = epoch.is_some();
        for table in [
            "attention",
            "readwater",
            "retired_push",
            "private_registration",
            "push_jobs",
            "push_sync",
            "devices",
        ] {
            tx.execute(&format!("DELETE FROM {table} WHERE peer=?1"), [&peer.0])?;
        }
        tx.commit()?;
        Ok(changed)
    }
    /// Authenticated canonical-request lookup only: never reserves or registers native work.
    /// Unseen boot/method checks keep their precedence before any asynchronous readiness wait.
    pub(crate) fn preflight_request(
        &mut self,
        proof: &AuthenticatedPeer,
        request: &Request,
        now: u64,
    ) -> StoreResult<Option<Receipt>> {
        let mut canonical = canonical_request(request)?;
        let result = (|| {
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            authorize(&tx, proof)?;
            expire_receipts(&tx, proof.peer_id(), now)?;
            let existing =
                lookup_matching_receipt(&tx, proof, &request.id, &self.receipt_key, &canonical)?;
            if existing.is_none() {
                validate_mutation_boot(request, &self.boot_id)?;
            }
            tx.commit()?;
            Ok(existing)
        })();
        canonical.fill(0);
        result
    }
    /// Canonical/HMAC lookup precedes stale checks, but never precedes authorization.
    /// validate_unseen runs only for a genuinely new ID (boot/target/capability/native answer).
    pub fn reserve(
        &mut self,
        proof: &AuthenticatedPeer,
        request: &Request,
        now: u64,
        validate_unseen: impl FnOnce(&Request) -> Result<(), ErrorCode>,
    ) -> StoreResult<Reservation> {
        let mut canonical = canonical_request(request)?;
        let result = (|| {
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            authorize(&tx, proof)?;
            expire_receipts(&tx, proof.peer_id(), now)?;
            if let Some(receipt) =
                lookup_matching_receipt(&tx, proof, &request.id, &self.receipt_key, &canonical)?
            {
                tx.commit()?;
                return Ok(Reservation::Existing(receipt));
            }
            let fingerprint = hmac_sha256(&self.receipt_key, &canonical);
            canonical.fill(0);
            canonical.clear();
            validate_mutation_boot(request, &self.boot_id)?;
            let (method, boot_id, target, creation) = match &request.body {
                body if body.attachment_action().is_some()=>{let (method,boot,target,_)=body.attachment_action().ok_or(StorageError::Corrupt)?;(method,boot.clone(),Some(target.clone()),None)},
                RequestBody::SessionControlsApply(p)=>(ActionMethod::SessionControlsApply,p.boot_id.clone(),Some(p.target.clone()),None),
                RequestBody::SessionQueueUpdate(p)=>(ActionMethod::SessionQueueUpdate,p.boot_id.clone(),Some(p.target.clone()),None),
                RequestBody::SessionQueueDelete(p)=>(ActionMethod::SessionQueueDelete,p.boot_id.clone(),Some(p.target.clone()),None),
                RequestBody::SessionQueueReorder(p)=>(ActionMethod::SessionQueueReorder,p.boot_id.clone(),Some(p.target.clone()),None),
                RequestBody::SessionQueueStart(p)=>(ActionMethod::SessionQueueStart,p.boot_id.clone(),Some(p.target.clone()),None),
                RequestBody::PromptSubmit(p) => (
                    ActionMethod::PromptSubmit,
                    p.boot_id.clone(),
                    Some(p.target.clone()),
                    None,
                ),
                RequestBody::TurnCancel(p) => (
                    ActionMethod::TurnCancel,
                    p.boot_id.clone(),
                    Some(p.target.clone()),
                    None,
                ),
                RequestBody::RequestRespond(p) => (
                    ActionMethod::RequestRespond,
                    p.boot_id.clone(),
                    Some(p.target.clone()),
                    None,
                ),
                RequestBody::TerminalCreate(p)=>(ActionMethod::TerminalCreate,p.boot_id.clone(),None,None),
                RequestBody::AgentRename(p) => (ActionMethod::AgentRename,p.boot_id.clone(),Some(p.target.clone()),None),
                RequestBody::WorkspaceRename(p) => (
                    ActionMethod::WorkspaceRename,
                    p.boot_id.clone(),
                    Some(p.target.clone()),
                    None,
                ),
                RequestBody::ShellInput(p) => (ActionMethod::ShellInput,p.boot_id.clone(),None,None),
                RequestBody::ShellResize(p)=>(ActionMethod::ShellResize,p.boot_id.clone(),None,None),
                RequestBody::ShellScroll(p)=>(ActionMethod::ShellScroll,p.boot_id.clone(),None,None),
                RequestBody::SessionCreate(p) => (
                    ActionMethod::SessionCreate,
                    p.boot_id.clone(),
                    None,
                    Some(Box::new(CreationProgress {
                        source_target: p.placement.as_ref().map(|placement|placement.source_target.clone()),
                        server_id: p.server_id.clone(),
                        project_id: p.project_id.clone(),
                        provider: p.provider,
                        stage: CreationStage::Workspace,
                        stage_state: StageState::NotStarted,
                        resources: CreationResources {
                            checkout: None,
                            workspace_id: None,
                            pane_id: None,
                            terminal_id: None,
                            native_id: None,
                            binding_id: None,
                            session_id: None,
                            generation: None,
                            owner_started: None,
                            relay_started: None,
                        },
                    })),
                ),
                _ => return reject(ErrorCode::InvalidRequest),
            };
            validate_unseen(request).map_err(StorageError::Policy)?;
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM receipts WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )?;
            if count >= sql_integer(PEER_CAPACITY)? {
                return reject(ErrorCode::ActionCapacity);
            }
            let receipt = Receipt {
                action_id: request.id.clone(),
                shell_target:match &request.body {RequestBody::ShellInput(params)=>Some(params.target.clone()),RequestBody::ShellResize(params)=>Some(params.target.clone()),RequestBody::ShellScroll(params)=>Some(params.target.clone()),_=>None},
                method,
                boot_id,
                target,
                created_at: timestamp(now)?,
                updated_at: timestamp(now)?,
                state: ReceiptState::Reserved,
                error: None,
                result: None,
                creation,
            };
            receipt.validate()?;
            tx.execute(
                "INSERT INTO receipts VALUES(?1,?2,?3,?4,?5,'reserved',?6)",
                params![
                    proof.peer.0,
                    request.id.0,
                    sql_integer(proof.epoch)?,
                    fingerprint.as_slice(),
                    sql_integer(now)?,
                    json(&receipt)?
                ],
            )?;
            if let Some(progress) = &receipt.creation {
                let provider = match progress.provider {
                    Provider::Omp => "omp",
                    Provider::Codex => "codex",
                };
                let alias = format!(
                    "goatr-{provider}-{}",
                    request
                        .id
                        .0
                        .replace('-', "")
                        .get(..20)
                        .ok_or(StorageError::Corrupt)?
                );
                let collision: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM creation WHERE alias=?1)",
                    [&alias],
                    |r| r.get(0),
                )?;
                if collision {
                    return reject(ErrorCode::ActionIdConflict);
                }
                tx.execute(
                    "INSERT INTO creation VALUES(?1,?2,?3,'[]')",
                    params![proof.peer.0, request.id.0, alias],
                )?;
            }
            tx.commit()?;
            Ok(Reservation::Reserved(receipt))
        })();
        canonical.fill(0);
        result
    }
    pub fn status(
        &mut self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
        now: u64,
    ) -> StoreResult<Option<Receipt>> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        expire_receipts(&tx, proof.peer_id(), now)?;
        let receipt = get_receipt(&tx, proof, action)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn transition(
        &mut self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
        change: ReceiptTransition,
    ) -> StoreResult<Receipt> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let mut receipt =
            get_receipt(&tx, proof, action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        apply_transition(&mut receipt, change)?;
        write_receipt(&tx, &proof.peer.0, &action.0, &receipt)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn dispatch_ticket(
        &self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
    ) -> StoreResult<DispatchTicket> {
        authorize(&self.db, proof)?;
        let receipt = get_receipt(&self.db, proof, action)?
            .ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if receipt.state != ReceiptState::Dispatching
            || receipt
                .creation
                .as_ref()
                .is_some_and(|p| p.stage_state != StageState::Dispatching)
        {
            return reject(ErrorCode::NotReady);
        }
        let metadata = if receipt.creation.is_some() {
            stages(&self.db, proof, action)?
                .last()
                .ok_or(StorageError::Corrupt)?
                .intent
                .clone()
        } else {
            StageMetadata::default()
        };
        Ok(DispatchTicket {
            proof: proof.clone(),
            action: action.clone(),
            stage: receipt.creation.as_ref().map(|p| p.stage),
            workspace_operation: metadata.workspace_operation,
            probe_attempt: metadata.shell_probe.map(|p| p.attempt_id),
        })
    }
    /// The original entered Thread ticket retains its acknowledged ID without settling it.
    pub(crate) fn checkpoint_created_thread(
        &mut self,
        ticket: &DispatchTicket,
        thread: Id,
        now: u64,
    ) -> StoreResult<()> {
        thread.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, &ticket.proof)?;
        let mut receipt = ticket_receipt(&tx, ticket)?;
        let progress = receipt.creation.as_mut().ok_or(StorageError::Corrupt)?;
        if ticket.stage != Some(CreationStage::Thread)
            || progress.provider != Provider::Codex
            || progress.stage_state != StageState::Dispatching
            || progress.resources.native_id.is_some()
            || now < receipt.updated_at.0
        {
            return reject(ErrorCode::InvalidRequest);
        }
        progress.resources.native_id = Some(thread);
        receipt.updated_at = timestamp(now)?;
        write_receipt(&tx, &ticket.proof.peer.0, &ticket.action.0, &receipt)?;
        tx.commit()?;
        Ok(())
    }
    /// Entered facts survive cancellation even if their queued checkpoint did not commit yet.
    pub(crate) fn settle_creation_lost(&mut self,ticket:DispatchTicket,resources:CreationResources,metadata:StageMetadata,at:u64)->StoreResult<()> {
        let receipt=ticket_receipt(&self.db,&ticket)?;
        let progress=receipt.creation.as_ref().ok_or(StorageError::Corrupt)?;
        let records=stages(&self.db,&ticket.proof,&ticket.action)?;
        let last=records.last().ok_or(StorageError::Corrupt)?;
        retain_creation_facts(&progress.resources,last.result.as_ref().unwrap_or(&last.intent),&resources,&metadata)?;
        self.settle_creation_stage(ticket,CreationOutcome {resources,metadata,state:StageState::Uncertain,definitive:None,at:at.max(receipt.updated_at.0)})
    }
    /// An already-entered bounded write may settle after revoke. This consumes its local
    /// ticket, cannot start a write/reconcile/replay, and deliberately returns no receipt.
    pub fn settle_dispatch(
        &mut self,
        ticket: DispatchTicket,
        outcome: ReceiptTransition,
    ) -> StoreResult<()> {
        if ticket.stage.is_some()
            || !matches!(
                &outcome.proof,
                TransitionProof::WriteCompleted
                    | TransitionProof::CorrelatedAcknowledgement(_)
                    | TransitionProof::NativeEffectiveState(_)
                    | TransitionProof::DefinitiveNoEffect(_)
                    | TransitionProof::LostOutcome
            )
        {
            return reject(ErrorCode::InvalidRequest);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut receipt = ticket_receipt(&tx, &ticket)?;
        apply_transition(&mut receipt, outcome)?;
        write_receipt(&tx, &ticket.proof.peer.0, &ticket.action.0, &receipt)?;
        tx.commit()?;
        Ok(())
    }
    /// Retain the entered creation stage's confirmed resources after removal, but never
    /// advance another stage. Known partial creation becomes failed; lost effect uncertain.
    pub fn settle_creation_stage(
        &mut self,
        ticket: DispatchTicket,
        outcome: CreationOutcome,
    ) -> StoreResult<()> {
        self.settle_creation(ticket, outcome, None)
    }
    pub(crate) fn settle_creation_failure(
        &mut self,
        ticket: DispatchTicket,
        outcome: CreationOutcome,
        failure: ReceiptTransition,
    ) -> StoreResult<()> {
        if outcome.state != StageState::Failed
            || failure.state != ReceiptState::Failed
            || !matches!(&failure.proof, TransitionProof::DefinitiveNoEffect(_))
            || outcome.at != failure.at
        {
            return reject(ErrorCode::InvalidRequest);
        }
        self.settle_creation(ticket, outcome, Some(failure))
    }
    fn settle_creation(
        &mut self,
        ticket: DispatchTicket,
        outcome: CreationOutcome,
        failure: Option<ReceiptTransition>,
    ) -> StoreResult<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut receipt = ticket_receipt(&tx, &ticket)?;
        if ticket.stage.is_none() {
            return reject(ErrorCode::InvalidRequest);
        }
        apply_creation_outcome(&tx, &ticket.proof, &ticket.action, &mut receipt, outcome)?;
        if let Some(failure) = failure {
            apply_transition(&mut receipt, failure)?;
        }
        let revoked = match authorize(&tx, &ticket.proof) {
            Ok(()) => false,
            Err(StorageError::Policy(ErrorCode::Unauthorized)) => true,
            Err(error) => return Err(error),
        };
        if revoked
            && !matches!(
                receipt.state,
                ReceiptState::Uncertain | ReceiptState::Failed
            )
        {
            if receipt.creation.as_ref().is_some_and(|p| {
                p.stage == CreationStage::Ready && p.stage_state == StageState::Confirmed
            }) {
                receipt.result = Some(ready_result(
                    &receipt
                        .creation
                        .as_ref()
                        .ok_or(StorageError::Corrupt)?
                        .resources,
                )?);
                receipt.state = ReceiptState::Succeeded;
            } else if receipt.creation.as_ref().is_some_and(|p| {
                p.stage_state == StageState::Dispatching
                    && intermediate_creation_stage(p.provider, p.stage, ticket.workspace_operation)
            }) {
                receipt.state = ReceiptState::Uncertain;
                receipt
                    .creation
                    .as_mut()
                    .ok_or(StorageError::Corrupt)?
                    .stage_state = StageState::Uncertain;
                let mut records = stages(&tx, &ticket.proof, &ticket.action)?;
                records.last_mut().ok_or(StorageError::Corrupt)?.state = StageState::Uncertain;
                tx.execute(
                    "UPDATE creation SET stages=?1 WHERE peer=?2 AND action=?3",
                    params![json(&records)?, ticket.proof.peer.0, ticket.action.0],
                )?;
            } else {
                receipt.state = ReceiptState::Failed;
                receipt.error = Some(WireError {
                    code: ErrorCode::Unauthorized,
                    message: SafeMessage::new("Device authorization was removed")?,
                });
            }
        }
        write_receipt(&tx, &ticket.proof.peer.0, &ticket.action.0, &receipt)?;
        tx.commit()?;
        Ok(())
    }
    /// Local maintenance, no credential lookup or metadata returned. Security3 schedules this
    /// while serving; reopening also prunes. Only expired receipt/push protection is evicted.
    pub fn prune(&mut self, now: u64) -> StoreResult<()> {
        timestamp(now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_all(&tx, now)?;
        tx.commit()?;
        Ok(())
    }
    /// Commit stage intent before the native effect. Confirmed resources are never removed.
    pub fn creation_intent(
        &mut self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
        stage: CreationStage,
        metadata: StageMetadata,
        now: u64,
    ) -> StoreResult<Receipt> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let mut receipt =
            get_receipt(&tx, proof, action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if receipt.state != ReceiptState::Dispatching {
            return reject(ErrorCode::InvalidRequest);
        }
        let p = receipt
            .creation
            .as_mut()
            .ok_or(StorageError::Policy(ErrorCode::InvalidRequest))?;
        let mut stages = stages(&tx, proof, action)?;
        let is_probe = metadata.workspace_operation == WorkspaceOperation::ShellProbe;
        if is_probe {
            let probe = metadata
                .shell_probe
                .as_ref()
                .ok_or(StorageError::Policy(ErrorCode::InvalidRequest))?;
            if stage != CreationStage::Workspace
                || p.stage != CreationStage::Workspace
                || p.stage_state != StageState::Confirmed
                || stages
                    .iter()
                    .any(|s| s.intent.workspace_operation == WorkspaceOperation::ShellProbe)
                || !probe_matches(probe, action, &p.resources)
            {
                return reject(ErrorCode::InvalidRequest);
            }
        } else if metadata.shell_probe.is_some() {
            return reject(ErrorCode::InvalidRequest);
        }
        let expected = if is_probe {
            CreationStage::Workspace
        } else {
            match (p.provider, p.stage, p.stage_state) {
                (_, CreationStage::Workspace, StageState::NotStarted) => CreationStage::Workspace,
                (Provider::Omp, CreationStage::Workspace, StageState::Confirmed) => {
                    if stage == CreationStage::Relay {
                        CreationStage::Relay
                    } else {
                        CreationStage::Launch
                    }
                }
                (Provider::Omp, CreationStage::Relay, StageState::Confirmed) => {
                    CreationStage::Launch
                }
                (Provider::Codex, CreationStage::Workspace, StageState::Confirmed) => {
                    CreationStage::Owner
                }
                (Provider::Codex, CreationStage::Owner, StageState::Confirmed) => {
                    CreationStage::Thread
                }
                (Provider::Codex, CreationStage::Thread, StageState::Confirmed) => {
                    CreationStage::Relay
                }
                (Provider::Codex, CreationStage::Relay, StageState::Confirmed) => {
                    CreationStage::Launch
                }
                (_, CreationStage::Launch, StageState::Confirmed) => CreationStage::Ready,
                _ => return reject(ErrorCode::InvalidRequest),
            }
        };
        if stage != expected {
            return reject(ErrorCode::InvalidRequest);
        }
        if stage != CreationStage::Workspace && !confirmed_probe(&stages, action, &p.resources) {
            return reject(ErrorCode::NotReady);
        }
        stages.push(CreationStageRecord {
            stage,
            intent: metadata,
            result: None,
            state: StageState::Dispatching,
            proof: None,
        });
        p.stage = stage;
        p.stage_state = StageState::Dispatching;
        receipt.updated_at = timestamp(now)?;
        write_receipt(&tx, &proof.peer.0, &action.0, &receipt)?;
        tx.execute(
            "UPDATE creation SET stages=?1 WHERE peer=?2 AND action=?3",
            params![json(&stages)?, proof.peer.0, action.0],
        )?;
        tx.commit()?;
        Ok(receipt)
    }
    /// Read-only reconciliation, never another stage dispatch. Exact native proof must establish
    /// the attempted stage or already-ready desktop binding; partial resources remain retained.
    pub fn creation_reconcile(
        &mut self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
        change: CreationReconciliation,
    ) -> StoreResult<Receipt> {
        let CreationReconciliation {
            resources,
            proof: reconciliation,
            metadata,
            at: now,
        } = change;
        let (ready, evidence) = match reconciliation {
            CreationReconciliationProof::Ready(id) => (true, id),
            CreationReconciliationProof::NoEffect(id) => (false, id),
        };
        evidence.validate()?;
        resources.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let mut receipt =
            get_receipt(&tx, proof, action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if receipt.state != ReceiptState::Uncertain {
            return reject(ErrorCode::InvalidRequest);
        }
        let progress = receipt
            .creation
            .as_mut()
            .ok_or(StorageError::Policy(ErrorCode::InvalidRequest))?;
        retain_resources(&progress.resources, &resources)?;
        if ready
            && (!stage_resources(CreationStage::Ready, progress.provider, &resources)
                || !stage_resources(CreationStage::Workspace, progress.provider, &resources)
                || (progress.provider == Provider::Codex
                    && (resources.owner_started.is_none()
                        || resources.relay_started != Some(true)
                        || resources.binding_id.is_none())))
        {
            return reject(ErrorCode::NotReady);
        }
        progress.resources = resources;
        progress.stage_state = if ready {
            StageState::Confirmed
        } else {
            StageState::Failed
        };
        if ready {
            progress.stage = CreationStage::Ready;
        }
        let mut records = stages(&tx, proof, action)?;
        records.push(CreationStageRecord {
            stage: progress.stage,
            intent: metadata.clone(),
            result: Some(metadata),
            state: progress.stage_state,
            proof: Some(evidence),
        });
        receipt.updated_at = timestamp(now)?;
        write_receipt(&tx, &proof.peer.0, &action.0, &receipt)?;
        tx.execute(
            "UPDATE creation SET stages=?1 WHERE peer=?2 AND action=?3",
            params![json(&records)?, proof.peer.0, action.0],
        )?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn creation_result(
        &mut self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
        outcome: CreationOutcome,
    ) -> StoreResult<Receipt> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let mut receipt =
            get_receipt(&tx, proof, action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        apply_creation_outcome(&tx, proof, action, &mut receipt, outcome)?;
        write_receipt(&tx, &proof.peer.0, &action.0, &receipt)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn creation_probe(
        &self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
    ) -> StoreResult<ShellProbeMetadata> {
        authorize(&self.db, proof)?;
        let receipt = get_receipt(&self.db, proof, action)?
            .ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        let progress = receipt
            .creation
            .as_ref()
            .ok_or(StorageError::Policy(ErrorCode::InvalidRequest))?;
        let records = stages(&self.db, proof, action)?;
        if receipt.state != ReceiptState::Dispatching
            || !confirmed_probe(&records, action, &progress.resources)
        {
            return reject(ErrorCode::NotReady);
        }
        records
            .into_iter()
            .find_map(|r| {
                (r.intent.workspace_operation == WorkspaceOperation::ShellProbe
                    && r.state == StageState::Confirmed)
                    .then_some(r.intent.shell_probe)
                    .flatten()
            })
            .ok_or(StorageError::Corrupt)
    }

    pub fn creation_local(
        &self,
        proof: &AuthenticatedPeer,
        action: &Uuid,
    ) -> StoreResult<Option<CreationLocal>> {
        authorize(&self.db, proof)?;
        let data: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT alias,stages FROM creation WHERE peer=?1 AND action=?2",
                params![proof.peer.0, action.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        data.map(|(alias, data)| {
            Ok(CreationLocal {
                alias,
                stages: decode(&data)?,
            })
        })
        .transpose()
    }
    /// A UI grouping only: absent evidence confers no incarnation/native/creation authority.
    pub fn enroll_server(&mut self, slot: &str, name: &Label) -> StoreResult<Uuid> {
        name.validate()?;
        if slot.is_empty() || slot.len() > 4096 || slot.chars().any(char::is_control) {
            return reject(ErrorCode::InvalidRequest);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row("SELECT server FROM servers WHERE slot=?1", [slot], |r| {
                r.get(0)
            })
            .optional()?;
        let server = if let Some(old) = old {
            Uuid::new(old)?
        } else {
            if tx.query_row("SELECT count(*) FROM servers", [], |r| r.get::<_, i64>(0))? >= 64 {
                return reject(ErrorCode::ResourceLimit);
            }
            let id = Uuid::random();
            tx.execute(
                "INSERT INTO servers VALUES(?1,?2,'null','')",
                params![slot, id.0],
            )?;
            id
        };
        tx.execute("INSERT INTO topology_server_display VALUES(?1,?2) ON CONFLICT(server) DO UPDATE SET name=excluded.name", params![server.0, json(name)?])?;
        tx.commit()?;
        Ok(server)
    }
    pub fn persisted_servers(&self) -> StoreResult<Vec<PersistedServer>> {
        let mut statement = self.db.prepare("SELECT s.slot,s.server,s.evidence,s.incarnation,d.name FROM servers s LEFT JOIN topology_server_display d ON d.server=s.server ORDER BY s.slot")?;
        let mut rows = statement.query([])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            if result.len() == 64 {
                return reject(ErrorCode::ResourceLimit);
            }
            let incarnation: String = row.get(3)?;
            let evidence: Option<ServerEvidence> = decode(&row.get::<_, String>(2)?)?;
            if evidence.is_some() != !incarnation.is_empty() {
                return Err(StorageError::Corrupt);
            }
            result.push(PersistedServer {
                slot: row.get(0)?,
                server_id: Uuid::new(row.get::<_, String>(1)?)?,
                incarnation: if incarnation.is_empty() {
                    None
                } else {
                    Some(Uuid::new(incarnation)?)
                },
                evidence,
                name: row
                    .get::<_, Option<String>>(4)?
                    .map(|s| decode(&s))
                    .transpose()?,
            });
        }
        Ok(result)
    }
    pub fn persisted_sessions(&self) -> StoreResult<Vec<PersistedSession>> {
        let mut statement=self.db.prepare("SELECT s.resource_key,s.session,s.generation,d.state FROM sessions s LEFT JOIN topology_session_display d ON d.session=s.session WHERE s.kind='agent' ORDER BY s.session")?;
        let mut rows=statement.query([])?;let mut result=Vec::new();
        while let Some(row)=rows.next()? {
            if result.len()==512 {return reject(ErrorCode::ResourceLimit)}
            let resource:StableResourceKey=decode(&row.get::<_,String>(0)?)?;resource.validate_key()?;
            let StableResourceKey::Agent(key)=resource else {return Err(StorageError::Corrupt)};
            let target=Target {session_id:Uuid::new(row.get::<_,String>(1)?)?,generation:Uuid::new(row.get::<_,String>(2)?)?};
            let mut display:Option<SessionState>=row.get::<_,Option<String>>(3)?.map(|s|decode(&s)).transpose()?;
            if let Some(state) = &mut display {
                if state.summary.session_id != target.session_id
                    || state.summary.server_id != key.server_id
                    || state.summary.terminal_id != key.terminal_id
                    || state.summary.native_id != key.native_conversation_id
                {
                    return Err(StorageError::Corrupt);
                }
                state.summary.generation = target.generation.clone();
                state.summary.connectivity = SessionConnectivity::Offline;
                state.summary.reason =
                    Some("Native binding has not been revalidated after companion restart".into());
                state.summary.capabilities.history = false;
                state.summary.history_coverage.source = HistorySource::Unavailable;
                state.summary.history_coverage.older_available = false;
                state.native_turn_id = None;
                state.activity = Activity {
                    state: ActivityState::Unknown,
                    text: None,
                    tool_record_id: None,
                };
            }
            result.push(PersistedSession {
                key,
                target,
                display,
            });
        }
        Ok(result)
    }
    /// Display metadata only; never persist native history, pending forms or control payloads.
    pub fn remember_session_display(
        &mut self,
        key: &StableSessionKey,
        state: &SessionState,
    ) -> StoreResult<()> {
        state.validate()?;
        let resource=StableResourceKey::Agent(key.clone());resource.validate_key()?;
        let matches:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE kind='agent' AND resource_key=?1 AND session=?2 AND generation=?3)",params![json(&resource)?,state.summary.session_id.0,state.summary.generation.0],|row|row.get(0))?;
        if !matches {
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE session=?1)",
                [&state.summary.session_id.0],
                |r| r.get(0),
            )?;
            return reject(if exists {
                ErrorCode::StaleSession
            } else {
                ErrorCode::NotFound
            });
        }
        if state.summary.server_id != key.server_id
            || state.summary.terminal_id != key.terminal_id
            || state.summary.native_id != key.native_conversation_id
        {
            return reject(ErrorCode::StaleSession);
        }
        let mut display = state.clone();
        display.native_turn_id = None;
        display.activity = Activity {
            state: ActivityState::Unknown,
            text: None,
            tool_record_id: None,
        };
        self.db.execute("INSERT INTO topology_session_display VALUES(?1,?2) ON CONFLICT(session) DO UPDATE SET state=excluded.state", params![state.summary.session_id.0, json(&display)?])?;
        Ok(())
    }
    /// The topology owner proves history/source no longer needs this row. Durable obligations
    /// are checked independently; no invented age or eviction of a needed identity.
    pub fn retire_session_mapping(
        &mut self,
        session: &Uuid,
        history_required: bool,
        now: u64,
    ) -> StoreResult<bool> {
        session.validate()?;
        if history_required {
            return Ok(false);
        }
        // Provider retirement must never consume a shell mapping or its obligations.
        if !self.db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE session=?1 AND kind='agent')", [&session.0], |row| row.get::<_,bool>(0))? { return Ok(false); }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_all(&tx, now)?;
        if mapping_receipt_required(&tx, Some(session), None)? || tx.query_row("SELECT EXISTS(SELECT 1 FROM attention WHERE session=?1 UNION ALL SELECT 1 FROM readwater WHERE session=?1)", [&session.0], |r| r.get::<_, bool>(0))? { return Ok(false); }
        tx.execute(
            "DELETE FROM topology_session_display WHERE session=?1",
            [&session.0],
        )?;
        tx.execute("DELETE FROM checkpoint WHERE session=?1", [&session.0])?;
        tx.execute("DELETE FROM transitions WHERE session=?1", [&session.0])?;
        let removed = tx.execute("DELETE FROM sessions WHERE session=?1 AND kind='agent'", [&session.0])? != 0;
        tx.commit()?;
        Ok(removed)
    }
    pub fn retire_server_mapping(
        &mut self,
        slot: &str,
        source_required: bool,
        now: u64,
    ) -> StoreResult<bool> {
        if source_required {
            return Ok(false);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_all(&tx, now)?;
        let server: Option<String> = tx
            .query_row("SELECT server FROM servers WHERE slot=?1", [slot], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(server) = server else {
            return Ok(false);
        };
        let mapped_keys:Vec<String>={let mut statement=tx.prepare("SELECT resource_key FROM sessions")?;let rows=statement.query_map([],|row|row.get(0))?;rows.collect::<Result<_,_>>()?};
        let has_mapping=mapped_keys.iter().map(|data|decode::<StableResourceKey>(data)).collect::<StoreResult<Vec<_>>>()?.iter().any(|key|key.server().0==server);
        if has_mapping || mapping_receipt_required(&tx,None,Some(&Uuid::new(&server)?))?
        {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM topology_server_display WHERE server=?1",
            [&server],
        )?;
        tx.execute("DELETE FROM servers WHERE slot=?1", [slot])?;
        tx.commit()?;
        Ok(true)
    }
    /// Topology owner supplies canonical logical slot and verified process/socket evidence.
    /// A label/PID alone is deliberately insufficient. Concurrent live slot collisions must be
    /// rejected by discovery before calling this operation; this API does not guess ownership.
    pub fn server_identity(
        &mut self,
        slot: &str,
        evidence: &ServerEvidence,
    ) -> StoreResult<ServerIdentity> {
        if slot.is_empty()
            || evidence.pid == 0
            || evidence.pid_start_ticks == 0
            || evidence.socket_inode == 0
        {
            return reject(ErrorCode::InvalidRequest);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM servers WHERE slot=?1)",
            [slot],
            |r| r.get(0),
        )?;
        if !exists
            && tx.query_row("SELECT count(*) FROM servers", [], |r| r.get::<_, i64>(0))? >= 64
        {
            return reject(ErrorCode::ResourceLimit);
        }
        let old: Option<(String, String, String)> = tx
            .query_row(
                "SELECT server,evidence,incarnation FROM servers WHERE slot=?1",
                [slot],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let identity = if let Some((server, data, incarnation)) = old {
            ServerIdentity {
                server_id: Uuid::new(server)?,
                incarnation: if data != "null" && decode::<ServerEvidence>(&data)? == *evidence {
                    Uuid::new(incarnation)?
                } else {
                    Uuid::random()
                },
            }
        } else {
            ServerIdentity {
                server_id: Uuid::random(),
                incarnation: Uuid::random(),
            }
        };
        tx.execute("INSERT INTO servers VALUES(?1,?2,?3,?4) ON CONFLICT(slot) DO UPDATE SET evidence=excluded.evidence,incarnation=excluded.incarnation", params![slot, identity.server_id.0, json(evidence)?, identity.incarnation.0])?;
        tx.commit()?;
        Ok(identity)
    }
    pub fn session_identity(
        &mut self,
        proven: &StableSessionKey,
        reconnect: bool,
    ) -> StoreResult<Target> {
        let (id,generation)=self.resource_identity(&StableResourceKey::Agent(proven.clone()),None,reconnect)?;
        Ok(Target {session_id:id,generation})
    }
    fn resource_identity(&mut self,key:&StableResourceKey,proof:Option<&crate::herdr::ShellProof>,reconnect:bool)->StoreResult<(Uuid,Uuid)> {
        key.validate_key()?;
        if (key.kind()=="shell")!=proof.is_some() {return Err(StorageError::Corrupt)}
        if let Some(proof)=proof {proof.validate_for(key).map_err(|_|StorageError::Corrupt)?;}
        let key_data=json(key)?;let proof_data=proof.map(json).transpose()?;
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old:Option<(String,String,Option<String>)>=tx.query_row("SELECT session,generation,proof FROM sessions WHERE kind=?1 AND resource_key=?2",params![key.kind(),key_data],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let (id,generation)=if let Some((id,generation,old_proof))=old {
            let changed=match (old_proof.as_deref(),proof) {(None,None)=>false,(Some(old),Some(current))=>{let old:crate::herdr::ShellProof=decode(old)?;old.validate_for(key).map_err(|_|StorageError::Corrupt)?;!old.same_authority(current)},_=>return Err(StorageError::Corrupt)};
            (Uuid::new(id)?,if reconnect || changed {Uuid::random()}else{Uuid::new(generation)?})
        }else{
            if tx.query_row("SELECT count(*) FROM sessions",[],|row|row.get::<_,i64>(0))?>=512 {return reject(ErrorCode::ResourceLimit)}
            (Uuid::random(),Uuid::random())
        };
        tx.execute("INSERT INTO sessions(session,generation,kind,resource_key,proof) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(kind,resource_key) DO UPDATE SET generation=excluded.generation,proof=excluded.proof",params![id.0,generation.0,key.kind(),key_data,proof_data])?;
        tx.commit()?;Ok((id,generation))
    }
    pub fn shell_identity(&mut self,key:&StableResourceKey,proof:&crate::herdr::ShellProof,reconnect:bool)->StoreResult<ShellTarget> {
        if !matches!(key,StableResourceKey::Shell {..}) {return Err(StorageError::Corrupt)}
        let (shell_id,generation)=self.resource_identity(key,Some(proof),reconnect)?;Ok(ShellTarget {shell_id,generation})
    }
    pub fn persisted_shells(&self)->StoreResult<Vec<PersistedShell>> {
        let mut statement=self.db.prepare("SELECT resource_key,session,generation,proof FROM sessions WHERE kind='shell' ORDER BY session")?;
        let mut rows=statement.query([])?;let mut shells=Vec::new();
        while let Some(row)=rows.next()? {
            if shells.len()>=512 {return reject(ErrorCode::ResourceLimit)}
            let key:StableResourceKey=decode(&row.get::<_,String>(0)?)?;key.validate_key()?;
            let proof:crate::herdr::ShellProof=decode(&row.get::<_,String>(3)?)?;proof.validate_for(&key).map_err(|_|StorageError::Corrupt)?;
            shells.push(PersistedShell {key,target:ShellTarget {shell_id:Uuid::new(row.get::<_,String>(1)?)?,generation:Uuid::new(row.get::<_,String>(2)?)?},proof});
        }Ok(shells)
    }

    /// Snapshot installation updates checkpoint only. Native correlation and all per-peer event
    /// allocations commit together, preventing duplicate completion/attention after restart.
    pub fn attention_transition(
        &mut self,
        change: &AttentionTransition,
        now: u64,
    ) -> StoreResult<Vec<(Uuid, AttentionEvent)>> {
        change.target.validate()?;
        change.correlation.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let events=attention_transition_tx(&tx,change,now)?;
        tx.commit()?;
        Ok(events)
    }
    pub fn notifications_sync(
        &mut self,
        proof: &AuthenticatedPeer,
        after: u64,
        limit: usize,
        now: u64,
    ) -> StoreResult<NotificationsSyncResult> {
        if after > UINT_MAX || !(1..=100).contains(&limit) {
            return reject(ErrorCode::InvalidRequest);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        prune_attention(&tx, &proof.peer.0, now)?;
        let latest: i64 = tx.query_row(
            "SELECT highwater FROM peers WHERE peer=?1",
            [&proof.peer.0],
            |r| r.get(0),
        )?;
        let latest = stored_uint(latest)?;
        if after > latest {
            return reject(ErrorCode::InvalidRequest);
        }
        let retained: Option<i64> = tx.query_row(
            "SELECT min(sequence) FROM attention WHERE peer=?1",
            [&proof.peer.0],
            |r| r.get(0),
        )?;
        let retained_from = retained
            .map(stored_uint)
            .transpose()?
            .unwrap_or(latest.checked_add(1).ok_or(StorageError::Corrupt)?);
        let data: Vec<String> = {
            let mut s = tx.prepare("SELECT event FROM attention WHERE peer=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
            s.query_map(
                params![proof.peer.0, sql_integer(after)?, sql_integer(limit + 1)?],
                |r| r.get(0),
            )?
            .collect::<Result<_, _>>()?
        };
        let mut events: Vec<AttentionEvent> =
            data.iter().map(|s| decode(s)).collect::<StoreResult<_>>()?;
        let more = events.len() > limit;
        events.truncate(limit);
        let read: Vec<ReadWatermark> = {
            let mut s =
                tx.prepare("SELECT session,through FROM readwater WHERE peer=?1 ORDER BY session")?;
            let data: Vec<(String, i64)> = s
                .query_map([&proof.peer.0], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            data.into_iter()
                .map(|(id, n)| {
                    Ok(ReadWatermark {
                        session_id: Uuid::new(id)?,
                        through_sequence: Uint(stored_uint(n)?),
                    })
                })
                .collect::<StoreResult<_>>()?
        };
        let next_sequence = if more {
            events.last().map(|e| e.sequence)
        } else {
            None
        };
        let result = NotificationsSyncResult {
            events,
            read,
            latest_sequence: Uint(latest),
            retained_from: Uint(retained_from),
            next_sequence,
            reset_required: after.saturating_add(1) < retained_from,
        };
        result.validate()?;
        tx.commit()?;
        Ok(result)
    }
    /// Called only after the authenticated live/sync frame has actually been written, not
    /// merely returned/queued. A lost sync response must not establish a read watermark guard.
    pub fn notifications_received(
        &mut self,
        proof: &AuthenticatedPeer,
        events: &[AttentionEvent],
    ) -> StoreResult<()> {
        if events.len() > 100 {
            return reject(ErrorCode::ResourceLimit);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        for event in events {
            let stored: Option<String> = tx
                .query_row(
                    "SELECT event FROM attention WHERE peer=?1 AND sequence=?2",
                    params![proof.peer.0, sql_integer(event.sequence.0)?],
                    |r| r.get(0),
                )
                .optional()?;
            if stored
                .as_deref()
                .map(decode::<AttentionEvent>)
                .transpose()?
                .as_ref()
                != Some(event)
            {
                return reject(ErrorCode::InvalidRequest);
            }
            tx.execute(
                "UPDATE attention SET received=1 WHERE peer=?1 AND sequence=?2",
                params![proof.peer.0, sql_integer(event.sequence.0)?],
            )?;
        }
        tx.execute("UPDATE push_sync SET required=0 WHERE peer=?1 AND NOT EXISTS(SELECT 1 FROM attention WHERE peer=?1 AND received=0)",[&proof.peer.0])?;
        tx.commit()?;
        Ok(())
    }
    /// The caller's actually-viewing proof must come from authenticated session UI state.
    pub fn notifications_read(
        &mut self,
        proof: &AuthenticatedPeer,
        session: &Uuid,
        through: u64,
        actually_viewing: bool,
    ) -> StoreResult<ReadWatermark> {
        if !actually_viewing || through > UINT_MAX {
            return reject(ErrorCode::InvalidRequest);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        let prior: Option<i64> = tx
            .query_row(
                "SELECT through FROM readwater WHERE peer=?1 AND session=?2",
                params![proof.peer.0, session.0],
                |r| r.get(0),
            )
            .optional()?;
        let prior = prior.map(stored_uint).transpose()?;
        if prior != Some(through) {
            let received: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM attention WHERE peer=?1 AND session=?2 AND sequence=?3 AND received=1)", params![proof.peer.0,session.0,sql_integer(through)?], |r| r.get(0))?;
            if !received {
                return reject(ErrorCode::InvalidRequest);
            }
        }
        if prior.is_none() {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM readwater WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )?;
            if count >= 512 {
                return reject(ErrorCode::ResourceLimit);
            }
            let at: i64 = tx.query_row(
                "SELECT coalesce(max(at),0) FROM attention WHERE peer=?1 AND session=?2",
                params![proof.peer.0, session.0],
                |r| r.get(0),
            )?;
            stored_uint(at)?;
            tx.execute(
                "INSERT INTO readwater VALUES(?1,?2,?3,?4)",
                params![proof.peer.0, session.0, sql_integer(through)?, at],
            )?;
        } else {
            tx.execute(
                "UPDATE readwater SET through=max(through,?1) WHERE peer=?2 AND session=?3",
                params![sql_integer(through)?, proof.peer.0, session.0],
            )?;
        }
        tx.commit()?;
        Ok(ReadWatermark {
            session_id: session.clone(),
            through_sequence: Uint(prior.unwrap_or(0).max(through)),
        })
    }
    pub fn retire_push_generation(
        &mut self,
        proof: &AuthenticatedPeer,
        generation: &Uuid,
        now: u64,
    ) -> StoreResult<()> {
        generation.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorize(&tx, proof)?;
        if now >= RETENTION_MS {
            tx.execute(
                "DELETE FROM retired_push WHERE peer=?1 AND at<=?2",
                params![proof.peer.0, sql_integer(now - RETENTION_MS)?],
            )?;
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM retired_push WHERE peer=?1 AND generation=?2)",
            params![proof.peer.0, generation.0],
            |r| r.get(0),
        )?;
        if !exists {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM retired_push WHERE peer=?1",
                [&proof.peer.0],
                |r| r.get(0),
            )?;
            if count >= sql_integer(PEER_CAPACITY)? {
                return reject(ErrorCode::ResourceLimit);
            }
            tx.execute(
                "INSERT INTO retired_push VALUES(?1,?2,?3)",
                params![proof.peer.0, generation.0, sql_integer(now)?],
            )?;
        }
        tx.execute(
            "DELETE FROM private_registration WHERE peer=?1 AND generation=?2",
            params![proof.peer.0, generation.0],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn push_generation_retired(
        &self,
        proof: &AuthenticatedPeer,
        generation: &Uuid,
    ) -> StoreResult<bool> {
        authorize(&self.db, proof)?;
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM retired_push WHERE peer=?1 AND generation=?2)",
            params![proof.peer.0, generation.0],
            |r| r.get(0),
        )?)
    }
}
fn authorize(db: &Connection, proof: &AuthenticatedPeer) -> StoreResult<()> {
    let epoch: Option<i64> = db
        .query_row(
            "SELECT epoch FROM peers JOIN devices USING(peer) WHERE peer=?1 AND paired=1",
            [&proof.peer.0],
            |r| r.get(0),
        )
        .optional()?;
    if epoch.map(stored_u64).transpose()? != Some(proof.epoch) {
        return reject(ErrorCode::Unauthorized);
    }
    Ok(())
}
fn expire_receipts(tx: &Transaction<'_>, peer: &Uuid, now: u64) -> StoreResult<()> {
    if now >= RETENTION_MS {
        tx.execute("DELETE FROM receipts WHERE peer=?1 AND state IN ('failed','succeeded','uncertain','dispatched') AND updated<=?2", params![peer.0,sql_integer(now - RETENTION_MS)?])?;
    }
    Ok(())
}
fn lookup_matching_receipt(
    db: &Connection,
    proof: &AuthenticatedPeer,
    action: &Uuid,
    key: &[u8; 32],
    canonical: &[u8],
) -> StoreResult<Option<Receipt>> {
    let stored: Option<(Vec<u8>, String)> = db
        .query_row(
            "SELECT fingerprint,receipt FROM receipts WHERE peer=?1 AND action=?2",
            params![proof.peer.0, action.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match stored {
        Some((fingerprint, data)) => {
            if !verify_hmac_sha256(key, canonical, &fingerprint) {
                return reject(ErrorCode::ActionIdConflict);
            }
            decode(&data).map(Some)
        }
        None => Ok(None),
    }
}
fn validate_mutation_boot(request: &Request, current: &Uuid) -> StoreResult<()> {
    let boot = match &request.body {
        RequestBody::SessionControlsApply(p)=>&p.boot_id,
        RequestBody::SessionQueueUpdate(p)=>&p.boot_id,
        RequestBody::SessionQueueDelete(p)=>&p.boot_id,
        RequestBody::SessionQueueReorder(p)=>&p.boot_id,
        RequestBody::SessionQueueStart(p)=>&p.boot_id,
        RequestBody::PromptSubmit(p) => &p.boot_id,
        RequestBody::TurnCancel(p) => &p.boot_id,
        RequestBody::RequestRespond(p) => &p.boot_id,
        RequestBody::SessionCreate(p) => &p.boot_id,
        RequestBody::ShellInput(p) => &p.boot_id,
        RequestBody::ShellResize(p)=>&p.boot_id,
        RequestBody::ShellScroll(p)=>&p.boot_id,
        body if body.attachment_action().is_some()=>body.attachment_action().ok_or(StorageError::Corrupt)?.1,
        RequestBody::TerminalCreate(p)=>&p.boot_id,
        RequestBody::AgentRename(p) => &p.boot_id,
        RequestBody::WorkspaceRename(p) => &p.boot_id,
        _ => return reject(ErrorCode::InvalidRequest),
    };
    if boot != current {
        return reject(ErrorCode::StaleHost);
    }
    Ok(())
}
fn get_receipt(
    db: &Connection,
    proof: &AuthenticatedPeer,
    action: &Uuid,
) -> StoreResult<Option<Receipt>> {
    let data: Option<String> = db
        .query_row(
            "SELECT receipt FROM receipts WHERE peer=?1 AND action=?2",
            params![proof.peer.0, action.0],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|s| decode(&s)).transpose()
}
fn write_receipt(
    tx: &Transaction<'_>,
    peer: &str,
    action: &str,
    receipt: &Receipt,
) -> StoreResult<()> {
    receipt.validate()?;
    tx.execute(
        "UPDATE receipts SET receipt=?1,state=?2,updated=?3 WHERE peer=?4 AND action=?5",
        params![
            json(receipt)?,
            match receipt.state {
                ReceiptState::Reserved => "reserved",
                ReceiptState::Dispatching => "dispatching",
                ReceiptState::Dispatched => "dispatched",
                ReceiptState::Succeeded => "succeeded",
                ReceiptState::Failed => "failed",
                ReceiptState::Uncertain => "uncertain",
            },
            sql_integer(receipt.updated_at.0)?,
            peer,
            action
        ],
    )?;
    Ok(())
}
fn probe_matches(probe: &ShellProbeMetadata, action: &Uuid, resources: &CreationResources) -> bool {
    probe.action_id == *action
        && resources.workspace_id.as_ref() == Some(&probe.workspace_id)
        && resources.pane_id.as_ref() == Some(&probe.pane_id)
        && resources.terminal_id.as_ref() == Some(&probe.terminal_id)
        && probe.shell.pid != 0
        && !probe.shell.namespaces.nspids.is_empty()
        && probe.shell.namespaces.nspids.len() <= 32
        && probe.attempt_id.validate().is_ok()
        && probe.shell.linux_boot_id.validate().is_ok()
        && crate::config::absolute(Path::new(&probe.executable)).is_ok()
        && crate::config::absolute(Path::new(&probe.result_path)).is_ok()
}
fn confirmed_probe(
    records: &[CreationStageRecord],
    action: &Uuid,
    resources: &CreationResources,
) -> bool {
    records.iter().any(|r| {
        r.stage == CreationStage::Workspace
            && r.intent.workspace_operation == WorkspaceOperation::ShellProbe
            && r.state == StageState::Confirmed
            && r.proof.is_some()
            && r.intent
                .shell_probe
                .as_ref()
                .is_some_and(|p| probe_matches(p, action, resources))
            && r.result.as_ref().is_some_and(|m| {
                m.workspace_operation == WorkspaceOperation::ShellProbe
                    && m.shell_probe == r.intent.shell_probe
            })
    })
}

fn stages(
    tx: &Connection,
    proof: &AuthenticatedPeer,
    action: &Uuid,
) -> StoreResult<Vec<CreationStageRecord>> {
    let data: String = tx.query_row(
        "SELECT stages FROM creation WHERE peer=?1 AND action=?2",
        params![proof.peer.0, action.0],
        |r| r.get(0),
    )?;
    decode(&data)
}
pub(crate) fn retain_creation_facts(old:&CreationResources,old_metadata:&StageMetadata,new:&CreationResources,new_metadata:&StageMetadata)->StoreResult<()> {
    retain_resources(old,new)?;
    if old_metadata.workspace_operation!=new_metadata.workspace_operation || old_metadata.shell_probe!=new_metadata.shell_probe
        || old_metadata.placement_workspace_id!=new_metadata.placement_workspace_id
        || old_metadata.created_tab_id.as_ref().is_some_and(|tab|new_metadata.created_tab_id.as_ref()!=Some(tab))
        || match (&old_metadata.execution_checkout,&new_metadata.execution_checkout) {
            (Some(old),Some(new))=>!new.retains(old), (None,None)=>false, _=>true,
        } {return reject(ErrorCode::InvalidRequest)}
    Ok(())
}
fn retain_resources(old: &CreationResources, new: &CreationResources) -> StoreResult<()> {
    macro_rules! keep { ($($field:ident),*) => { $(if old.$field.is_some() && old.$field != new.$field { return reject(ErrorCode::InvalidRequest); })* }; }
    if let Some(old_checkout) = &old.checkout {
        let next = new.checkout.as_ref().ok_or(StorageError::Policy(ErrorCode::InvalidRequest))?;
        if old_checkout.kind != next.kind || old_checkout.branch != next.branch || old_checkout.requested_base != next.requested_base
            || old_checkout.resolved_commit.as_ref().is_some_and(|commit| next.resolved_commit.as_ref() != Some(commit)) {
            return reject(ErrorCode::InvalidRequest);
        }
    }
    keep!(
        workspace_id,
        pane_id,
        terminal_id,
        native_id,
        binding_id,
        session_id,
        generation,
        owner_started,
        relay_started
    );
    Ok(())
}
fn ready_result(r: &CreationResources) -> StoreResult<ActionResult> {
    r.validate()?;
    Ok(ActionResult::Created(Box::new(ActionResultCreated {
        target: Target {
            session_id: r.session_id.clone().ok_or(StorageError::Corrupt)?,
            generation: r.generation.clone().ok_or(StorageError::Corrupt)?,
        },
        workspace_id: r.workspace_id.clone().ok_or(StorageError::Corrupt)?,
        terminal_id: r.terminal_id.clone().ok_or(StorageError::Corrupt)?,
        pane_id: r.pane_id.clone().ok_or(StorageError::Corrupt)?,
        native_id: r.native_id.clone().ok_or(StorageError::Corrupt)?,
        binding_id: r.binding_id.clone(),
    })))
}
fn intermediate_creation_stage(
    provider: Provider,
    stage: CreationStage,
    operation: WorkspaceOperation,
) -> bool {
    stage == CreationStage::Launch
        || (stage == CreationStage::Workspace && operation == WorkspaceOperation::ShellProbe)
        || (provider == Provider::Codex
            && matches!(stage, CreationStage::Owner | CreationStage::Relay))
}
fn stage_resources(stage: CreationStage, provider: Provider, r: &CreationResources) -> bool {
    match stage {
        CreationStage::Workspace => {
            r.workspace_id.is_some() && r.pane_id.is_some() && r.terminal_id.is_some()
        }
        CreationStage::Owner => r.owner_started.is_some(),
        CreationStage::Thread => r.native_id.is_some(),
        CreationStage::Relay => {
            r.relay_started == Some(true) && (provider == Provider::Omp || r.binding_id.is_some())
        }
        CreationStage::Launch => {
            r.native_id.is_some() && (provider == Provider::Omp || r.binding_id.is_some())
        }
        CreationStage::Ready => {
            r.session_id.is_some() && r.generation.is_some() && r.native_id.is_some()
        }
    }
}
fn prune_attention(tx: &Transaction<'_>, peer: &str, now: u64) -> StoreResult<()> {
    if now >= RETENTION_MS {
        tx.execute(
            "DELETE FROM attention WHERE peer=?1 AND at<=?2",
            params![peer, sql_integer(now - RETENTION_MS)?],
        )?;
    }
    tx.execute("DELETE FROM attention WHERE peer=?1 AND sequence IN (SELECT sequence FROM attention WHERE peer=?1 ORDER BY sequence DESC LIMIT -1 OFFSET ?2)", params![peer,sql_integer(PEER_CAPACITY)?])?;
    if now >= RETENTION_MS {
        tx.execute("DELETE FROM readwater WHERE peer=?1 AND last_event<=?2 AND NOT EXISTS(SELECT 1 FROM attention WHERE attention.peer=readwater.peer AND attention.session=readwater.session)", params![peer,sql_integer(now - RETENTION_MS)?])?;
    }
    Ok(())
}
fn apply_transition(receipt: &mut Receipt, change: ReceiptTransition) -> StoreResult<()> {
    let ReceiptTransition {
        state: next,
        proof: evidence,
        result,
        error,
        at: now,
    } = change;
    match &evidence {
        TransitionProof::CorrelatedAcknowledgement(id)
        | TransitionProof::NativeEffectiveState(id)
        | TransitionProof::DefinitiveNoEffect(id)
        | TransitionProof::ReconciledSuccess(id)
        | TransitionProof::ReconciledNoEffect(id) => id.validate()?,
        _ => {}
    }
    let allowed = matches!(
        (receipt.state, next, &evidence),
        (
            ReceiptState::Reserved,
            ReceiptState::Dispatching,
            TransitionProof::Intent
        ) | (
            ReceiptState::Reserved,
            ReceiptState::Failed,
            TransitionProof::DefinitiveNoEffect(_)
        ) | (
            ReceiptState::Reserved | ReceiptState::Dispatching,
            ReceiptState::Uncertain,
            TransitionProof::LostOutcome
        ) | (
            ReceiptState::Dispatching,
            ReceiptState::Dispatched,
            TransitionProof::WriteCompleted
        ) | (
            ReceiptState::Dispatching | ReceiptState::Dispatched,
            ReceiptState::Succeeded,
            TransitionProof::CorrelatedAcknowledgement(_)
        ) | (
            ReceiptState::Dispatching,
            ReceiptState::Failed,
            TransitionProof::DefinitiveNoEffect(_)
        ) | (
            ReceiptState::Uncertain,
            ReceiptState::Succeeded,
            TransitionProof::ReconciledSuccess(_)
        ) | (
            ReceiptState::Uncertain,
            ReceiptState::Failed,
            TransitionProof::ReconciledNoEffect(_)
        )
    );
    let allowed=allowed||receipt.method==ActionMethod::SessionControlsApply
        &&matches!((receipt.state,next,&evidence),(ReceiptState::Dispatching|ReceiptState::Dispatched,ReceiptState::Succeeded,TransitionProof::NativeEffectiveState(_)));
    if !allowed {
        return reject(ErrorCode::InvalidRequest);
    }
    if next == ReceiptState::Succeeded && receipt.method == ActionMethod::SessionCreate {
        let p = receipt
            .creation
            .as_ref()
            .ok_or(StorageError::Policy(ErrorCode::NotReady))?;
        let Some(ActionResult::Created(created)) = &result else {
            return reject(ErrorCode::NotReady);
        };
        if p.stage != CreationStage::Ready || p.stage_state != StageState::Confirmed {
            return reject(ErrorCode::NotReady);
        }
        let r = &p.resources;
        if r.session_id.as_ref() != Some(&created.target.session_id)
            || r.generation.as_ref() != Some(&created.target.generation)
            || r.workspace_id.as_ref() != Some(&created.workspace_id)
            || r.terminal_id.as_ref() != Some(&created.terminal_id)
            || r.pane_id.as_ref() != Some(&created.pane_id)
            || r.native_id.as_ref() != Some(&created.native_id)
            || r.binding_id != created.binding_id
        {
            return reject(ErrorCode::InvalidRequest);
        }
    }
    if next == ReceiptState::Failed
        && receipt.creation.as_ref().is_some_and(|p| {
            matches!(
                p.stage_state,
                StageState::Dispatching | StageState::Uncertain
            )
        })
    {
        return reject(ErrorCode::InvalidRequest);
    }
    if now < receipt.updated_at.0 {
        return reject(ErrorCode::InvalidRequest);
    }
    receipt.state = next;
    receipt.updated_at = timestamp(now)?;
    receipt.result = result;
    receipt.error = error;
    if next == ReceiptState::Uncertain
        && let Some(p) = &mut receipt.creation
        && p.stage_state == StageState::Dispatching
    {
        p.stage_state = StageState::Uncertain;
    }
    receipt.validate()?;
    Ok(())
}
fn apply_creation_outcome(
    tx: &Transaction<'_>,
    proof: &AuthenticatedPeer,
    action: &Uuid,
    receipt: &mut Receipt,
    outcome: CreationOutcome,
) -> StoreResult<()> {
    let CreationOutcome {
        resources,
        metadata,
        state: stage_state,
        definitive,
        at: now,
    } = outcome;
    resources.validate()?;
    definitive.validate()?;
    let p = receipt
        .creation
        .as_mut()
        .ok_or(StorageError::Policy(ErrorCode::InvalidRequest))?;
    if receipt.state != ReceiptState::Dispatching
        || p.stage_state != StageState::Dispatching
        || !matches!(
            stage_state,
            StageState::Confirmed
                | StageState::Failed
                | StageState::Uncertain
                | StageState::Dispatching
        )
        || (stage_state != StageState::Uncertain && definitive.is_none())
        || now < receipt.updated_at.0
    {
        return reject(ErrorCode::InvalidRequest);
    }
    retain_resources(&p.resources, &resources)?;
    let mut records = stages(tx, proof, action)?;
    let last = records.last_mut().ok_or(StorageError::Corrupt)?;
    if last.stage != p.stage
        || last.state != StageState::Dispatching
        || metadata.workspace_operation != last.intent.workspace_operation
        || metadata.shell_probe != last.intent.shell_probe
        || metadata.placement_workspace_id!=last.intent.placement_workspace_id
        || last.intent.created_tab_id.as_ref().is_some_and(|tab|metadata.created_tab_id.as_ref()!=Some(tab))
        || match (&last.intent.execution_checkout,&metadata.execution_checkout) {
            (Some(old),Some(current)) => !current.retains(old),
            (None,None) => false,
            _ => true,
        }
    {
        return reject(ErrorCode::InvalidRequest);
    }
    if let Some(previous)=last.result.as_ref() {retain_creation_facts(&p.resources,previous,&resources,&metadata)?;}
    if stage_state == StageState::Dispatching
        && (!intermediate_creation_stage(p.provider, p.stage, last.intent.workspace_operation)
            || !stage_resources(CreationStage::Workspace, p.provider, &resources)
            || resources.native_id != p.resources.native_id
            || resources.session_id.is_some()
            || resources.generation.is_some())
    {
        return reject(ErrorCode::InvalidRequest);
    }
    if stage_state == StageState::Confirmed && !stage_resources(p.stage, p.provider, &resources) {
        return reject(ErrorCode::NotReady);
    }
    last.result = Some(metadata);
    last.state = stage_state;
    last.proof = definitive;
    p.resources = resources;
    p.stage_state = stage_state;
    receipt.updated_at = timestamp(now)?;
    if stage_state == StageState::Uncertain {
        receipt.state = ReceiptState::Uncertain;
    }
    tx.execute(
        "UPDATE creation SET stages=?1 WHERE peer=?2 AND action=?3",
        params![json(&records)?, proof.peer.0, action.0],
    )?;
    Ok(())
}
fn ticket_receipt(tx: &Connection, ticket: &DispatchTicket) -> StoreResult<Receipt> {
    let epoch: Option<i64> = tx
        .query_row(
            "SELECT epoch FROM receipts WHERE peer=?1 AND action=?2",
            params![ticket.proof.peer.0, ticket.action.0],
            |r| r.get(0),
        )
        .optional()?;
    let epoch = epoch.map(stored_u64).transpose()?;
    if epoch != Some(ticket.proof.epoch) {
        return reject(ErrorCode::Unauthorized);
    }
    let receipt = get_receipt(tx, &ticket.proof, &ticket.action)?
        .ok_or(StorageError::Policy(ErrorCode::NotFound))?;
    if receipt.state != ReceiptState::Dispatching
        || receipt.creation.as_ref().map(|p| p.stage) != ticket.stage
    {
        return reject(ErrorCode::InvalidRequest);
    }
    if receipt.creation.is_some() {
        let records = stages(tx, &ticket.proof, &ticket.action)?;
        let last = records.last().ok_or(StorageError::Corrupt)?;
        if last.intent.workspace_operation != ticket.workspace_operation
            || last.intent.shell_probe.as_ref().map(|p| &p.attempt_id)
                != ticket.probe_attempt.as_ref()
        {
            return reject(ErrorCode::InvalidRequest);
        }
    }
    Ok(receipt)
}
fn mapping_receipt_required(
    db: &Connection,
    session: Option<&Uuid>,
    server: Option<&Uuid>,
) -> StoreResult<bool> {
    let mut statement = db.prepare("SELECT receipt FROM receipts")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let receipt: Receipt = decode(&row.get::<_, String>(0)?)?;
        if session.is_some_and(|s| receipt.target.as_ref().is_some_and(|t| &t.session_id == s)
            || receipt.shell_target.as_ref().is_some_and(|t| &t.shell_id==s)
            || matches!(&receipt.result,Some(ActionResult::ShellCreation(c)) if c.resources.created_target.as_ref().is_some_and(|target|&target.shell_id==s))
            || receipt.creation.as_ref().is_some_and(|p| p.resources.session_id.as_ref() == Some(s))
            || matches!(&receipt.result, Some(ActionResult::Created(c)) if &c.target.session_id == s))
            || server.is_some_and(|s| receipt.creation.as_ref().is_some_and(|p| &p.server_id == s) || matches!(&receipt.result,Some(ActionResult::ShellCreation(c)) if c.resources.server_id.as_ref()==Some(s))) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn prune_all(tx: &Transaction<'_>, now: u64) -> StoreResult<()> {
    if now >= RETENTION_MS {
        let cutoff = sql_integer(now - RETENTION_MS)?;
        tx.execute("DELETE FROM receipts WHERE state IN ('failed','succeeded','uncertain','dispatched') AND updated<=?1",[cutoff])?;
        tx.execute("DELETE FROM retired_push WHERE at<=?1", [cutoff])?;
        tx.execute("DELETE FROM attention WHERE at<=?1", [cutoff])?;
        tx.execute("DELETE FROM readwater WHERE last_event<=?1 AND NOT EXISTS(SELECT 1 FROM attention WHERE attention.peer=readwater.peer AND attention.session=readwater.session)",[cutoff])?;
        tx.execute("DELETE FROM transitions WHERE at<=?1", [cutoff])?;
        tx.execute("DELETE FROM peers WHERE paired=0 AND NOT EXISTS(SELECT 1 FROM receipts WHERE receipts.peer=peers.peer) AND NOT EXISTS(SELECT 1 FROM attachment_image_uses WHERE attachment_image_uses.peer=peers.peer)",[])?;
    }
    Ok(())
}

type LegacySessionRow = (String,String,String,String,String,String,String,String);
/// Startup-only schema migration. Do not rename the old table: SQLite can rewrite
/// unrelated foreign keys to that temporary name. No FK cascade may erase data.
fn migrate_resource_mappings(db:&mut Connection)->StoreResult<()> {
    let columns:Vec<(String,i64)>={let mut statement=db.prepare("PRAGMA table_info(sessions)")?;let rows=statement.query_map([],|row|Ok((row.get(1)?,row.get(5)?)))?;rows.collect::<Result<_,_>>()?};
    let marker:Option<Vec<u8>>=db.query_row("SELECT value FROM meta WHERE key='resource_mapping_schema'",[],|row|row.get(0)).optional()?;
    let legacy=["host","server","incarnation","terminal","owner","native","session","generation"];
    let current=["session","generation","kind","resource_key","proof"];
    let names:Vec<_>=columns.iter().map(|(name,_)|name.as_str()).collect();
    // Unknown DDL, triggers or explicit indexes are not silently discarded.
    let ddl:String=db.query_row("SELECT sql FROM sqlite_master WHERE type='table' AND name='sessions'",[],|row|row.get(0))?;
    let normalize=|sql:&str|sql.chars().filter(|ch|!ch.is_whitespace() && *ch!='\"' && *ch!=';').flat_map(char::to_lowercase).collect::<String>();
    let expected=if marker.as_deref()==Some(b"2") {"CREATE TABLE sessions(session TEXT PRIMARY KEY,generation TEXT NOT NULL,kind TEXT NOT NULL,resource_key TEXT NOT NULL,proof TEXT,UNIQUE(kind,resource_key))"}else{"CREATE TABLE sessions(host TEXT NOT NULL,server TEXT NOT NULL,incarnation TEXT NOT NULL,terminal TEXT NOT NULL,owner TEXT NOT NULL,native TEXT NOT NULL,session TEXT NOT NULL UNIQUE,generation TEXT NOT NULL,PRIMARY KEY(host,server,incarnation,terminal,owner,native))"};
    if normalize(&ddl)!=normalize(expected) || db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE tbl_name='sessions' AND type IN ('trigger','index') AND sql IS NOT NULL)",[],|row|row.get::<_,bool>(0))? {return Err(StorageError::Corrupt)}
    if db.query_row("SELECT count(*) FROM sessions",[],|row|row.get::<_,i64>(0))?>512 {return Err(StorageError::Corrupt)}
    if marker.as_deref()==Some(b"2") {
        if names!=current || columns.first().map(|column|column.1)!=Some(1) {return Err(StorageError::Corrupt)}
        let mut statement=db.prepare("SELECT session,generation,kind,resource_key,proof FROM sessions")?;
        let mut rows=statement.query([])?;let mut identities=std::collections::HashSet::new();
        while let Some(row)=rows.next()? {
            Uuid::new(row.get::<_,String>(0)?)?;Uuid::new(row.get::<_,String>(1)?)?;
            let kind:String=row.get(2)?;let data:String=row.get(3)?;let key:StableResourceKey=decode(&data)?;key.validate_key()?;
            if kind!=key.kind() || json(&key)?!=data || !identities.insert((kind.clone(),data)) {return Err(StorageError::Corrupt)}
            let proof:Option<String>=row.get(4)?;
            match (key.kind(),proof) {
                ("agent",None)=>{},
                ("shell",Some(data))=>decode::<crate::herdr::ShellProof>(&data)?.validate_for(&key).map_err(|_|StorageError::Corrupt)?,
                _=>return Err(StorageError::Corrupt),
            }
        }return Ok(())
    }
    if marker.is_some() || names!=legacy {return Err(StorageError::Corrupt)}
    // Connection-local, before BEGIN; restored on either result. No other storage
    // job can run while this owner is still being initialized.
    db.execute_batch("PRAGMA foreign_keys=OFF")?;
    if db.query_row("PRAGMA foreign_keys",[],|row|row.get::<_,i64>(0))?!=0 {return Err(StorageError::Corrupt)}
    let migration=(||->StoreResult<()> {
        let tx=db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE sessions_resource_v2(session TEXT PRIMARY KEY,generation TEXT NOT NULL,kind TEXT NOT NULL,resource_key TEXT NOT NULL,proof TEXT,UNIQUE(kind,resource_key));")?;
        let legacy_rows:Vec<LegacySessionRow>={let mut statement=tx.prepare("SELECT host,server,incarnation,terminal,owner,native,session,generation FROM sessions")?;let rows=statement.query_map([],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)))?;rows.collect::<Result<_,_>>()?};
        if legacy_rows.len()>512 {return Err(StorageError::Corrupt)}
        for (host,server,incarnation,terminal,owner,native,id,generation) in &legacy_rows {
            let key=StableResourceKey::Agent(StableSessionKey {host_id:EndpointId::new(host.clone())?,server_id:Uuid::new(server.clone())?,server_incarnation:Uuid::new(incarnation.clone())?,terminal_id:Id::new(terminal.clone())?,provider_owner_identity:Id::new(owner.clone())?,native_conversation_id:Id::new(native.clone())?});
            key.validate_key()?;Uuid::new(id.clone())?;Uuid::new(generation.clone())?;
            tx.execute("INSERT INTO sessions_resource_v2(session,generation,kind,resource_key,proof) VALUES(?1,?2,'agent',?3,NULL)",params![id,generation,json(&key)?])?;
        }
        if tx.query_row("SELECT count(*) FROM sessions_resource_v2",[],|row|row.get::<_,i64>(0))?!=legacy_rows.len() as i64 {return Err(StorageError::Corrupt)}
        tx.execute_batch("DROP TABLE sessions; ALTER TABLE sessions_resource_v2 RENAME TO sessions;")?;
        // Existing displays/checkpoints/receipts/attention/readwater are untouched.
        // Incompatible historical FKs fail and roll back the entire replacement.
        let violation:bool={let mut statement=tx.prepare("PRAGMA foreign_key_check")?;let mut rows=statement.query([])?;rows.next()?.is_some()};
        if violation {return Err(StorageError::Corrupt)}
        tx.execute("INSERT INTO meta(key,value) VALUES('resource_mapping_schema',?1)",[b"2".as_slice()])?;
        tx.commit()?;Ok(())
    })();
    db.execute_batch("PRAGMA foreign_keys=ON")?;
    if db.query_row("PRAGMA foreign_keys",[],|row|row.get::<_,i64>(0))?!=1 {return Err(StorageError::Corrupt)}
    migration
}

pub(crate) fn retain_shell_facts(old:&ShellCreationResources,new:&ShellCreationResources)->StoreResult<()> {
    old.validate()?;new.validate()?;
    macro_rules! keep {($field:ident)=>{if old.$field.as_ref().is_some_and(|known|new.$field.as_ref()!=Some(known)){return Err(StorageError::Corrupt)}};}
    keep!(server_id);keep!(workspace_id);keep!(tab_id);keep!(pane_id);keep!(terminal_id);keep!(created_target);Ok(())
}
fn shell_facts(receipt:&Receipt)->StoreResult<ShellCreationResources> {
    if receipt.creation.is_some() || match receipt.method {ActionMethod::ShellCreate=>receipt.shell_target.is_none()||receipt.target.is_some(),ActionMethod::TerminalCreate=>receipt.shell_target.is_some(),_=>true}{return Err(StorageError::Corrupt)}
    match &receipt.result {None=>Ok(ShellCreationResources::default()),Some(ActionResult::ShellCreation(result))=>Ok(result.resources.clone()),_=>Err(StorageError::Corrupt)}
}
impl Storage {
    pub(crate) fn checkpoint_shell_facts(&mut self,ticket:&DispatchTicket,resources:ShellCreationResources,at:u64)->StoreResult<()> {
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;authorize(&tx,&ticket.proof)?;
        let mut receipt=ticket_receipt(&tx,ticket)?;
        if ticket.stage.is_some() || receipt.state!=ReceiptState::Dispatching {return reject(ErrorCode::NotReady)}
        retain_shell_facts(&shell_facts(&receipt)?,&resources)?;
        receipt.result=Some(ActionResult::ShellCreation(ActionResultShellCreation {resources}));receipt.updated_at=timestamp(at.max(receipt.updated_at.0))?;
        write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
    /// Final same-original source/root/kernel-cwd proof guards the actual receipt commit.
    /// Acquire SQLite before this short proof, never await while holding either owner.
    pub(crate) fn settle_terminal_creation_dispatch(
        &mut self,ticket:DispatchTicket,mut outcome:ReceiptTransition,resources:ShellCreationResources,
        acknowledge:impl FnOnce(&ShellCreationResources,&mut dyn FnMut()->Result<(),ErrorCode>)->Result<(),ErrorCode>,
    )->StoreResult<()> {
        if ticket.stage.is_some(){return reject(ErrorCode::InvalidRequest)}
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt=ticket_receipt(&tx,&ticket)?;
        if receipt.method!=ActionMethod::TerminalCreate{return reject(ErrorCode::InvalidRequest)}
        retain_shell_facts(&shell_facts(&receipt)?,&resources)?;
        match &outcome.result {
            Some(ActionResult::ShellCreation(result))=>retain_shell_facts(&resources,&result.resources)?,
            None=>outcome.result=Some(ActionResult::ShellCreation(ActionResultShellCreation {resources:resources.clone()})),
            Some(_)=>return reject(ErrorCode::InvalidRequest),
        }
        let known_effect=resources.workspace_id.is_some()||resources.tab_id.is_some()||resources.pane_id.is_some()||resources.terminal_id.is_some()||resources.created_target.is_some();
        if outcome.state==ReceiptState::Failed&&known_effect{outcome.state=ReceiptState::Uncertain;outcome.proof=TransitionProof::LostOutcome;outcome.error=None;}
        let peer_current=match authorize(&tx,&ticket.proof){Ok(())=>true,Err(StorageError::Policy(_))=>false,Err(error)=>return Err(error)};
        let mut pending=Some((tx,receipt,outcome));let mut committed=None;
        if peer_current&&pending.as_ref().is_some_and(|(_,_,outcome)|outcome.state==ReceiptState::Succeeded){
            let _guard_result=acknowledge(&resources,&mut ||{
                let Some((tx,mut receipt,outcome))=pending.take()else{return Err(ErrorCode::InvalidRequest)};
                committed=Some((||{
                    apply_transition(&mut receipt,outcome)?;
                    write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;
                    tx.execute("DELETE FROM creation_shell_fences WHERE peer=?1 AND action=?2",params![ticket.proof.peer.0,ticket.action.0])?;
                    tx.commit()?;Ok(())
                })());
                match committed.as_ref() {
                    Some(Ok(()))=>Ok(()),
                    Some(Err(StorageError::Policy(code)))=>Err(*code),
                    _=>Err(ErrorCode::NotReady),
                }
            });
        }
        if let Some(result)=committed{return result}
        let Some((tx,mut receipt,mut outcome))=pending else{return Err(StorageError::Corrupt)};
        // Missing original acknowledgement authority cannot be substituted by
        // the earlier successful reply. Keep every known resource fact.
        if outcome.state==ReceiptState::Succeeded{outcome.state=ReceiptState::Uncertain;outcome.proof=TransitionProof::LostOutcome;outcome.error=None;}
        apply_transition(&mut receipt,outcome)?;write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
}

pub(crate) fn retain_shell_input_facts(old:&ShellDispatchProgress,new:&ShellDispatchProgress)->StoreResult<()> {
    old.validate()?;new.validate()?;if !new.retains(old){return Err(StorageError::Corrupt)}Ok(())
}
fn shell_input_facts(receipt:&Receipt)->StoreResult<ShellDispatchProgress> {
    if receipt.method!=ActionMethod::ShellInput||receipt.shell_target.is_none()||receipt.target.is_some()||receipt.creation.is_some(){return Err(StorageError::Corrupt)}
    match &receipt.result {None=>Ok(ShellDispatchProgress::default()),Some(ActionResult::ShellDispatch(result))=>Ok(result.progress.clone()),_=>Err(StorageError::Corrupt)}
}
impl Storage {
    pub(crate) fn checkpoint_shell_input_facts(&mut self,ticket:&DispatchTicket,progress:ShellDispatchProgress,at:u64)->StoreResult<()> {
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;authorize(&tx,&ticket.proof)?;
        let mut receipt=ticket_receipt(&tx,ticket)?;
        if ticket.stage.is_some() || receipt.state!=ReceiptState::Dispatching {return reject(ErrorCode::NotReady)}
        retain_shell_input_facts(&shell_input_facts(&receipt)?,&progress)?;
        receipt.result=Some(ActionResult::ShellDispatch(ActionResultShellDispatch {progress}));receipt.updated_at=timestamp(at.max(receipt.updated_at.0))?;
        write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
    pub(crate) fn settle_shell_input_dispatch(&mut self,ticket:DispatchTicket,mut outcome:ReceiptTransition,progress:ShellDispatchProgress)->StoreResult<()> {
        if ticket.stage.is_some() {return reject(ErrorCode::InvalidRequest)}
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut receipt=ticket_receipt(&tx,&ticket)?;
        retain_shell_input_facts(&shell_input_facts(&receipt)?,&progress)?;
        match &outcome.result {
            Some(ActionResult::ShellDispatch(result))=>retain_shell_input_facts(&progress,&result.progress)?,
            None=>outcome.result=Some(ActionResult::ShellDispatch(ActionResultShellDispatch {progress:progress.clone()})),
            Some(_)=>return reject(ErrorCode::InvalidRequest),
        }
        let known_effect=progress.socket_bytes_written.is_some_and(|n|n>0)||progress.frame_complete==Some(true);
        let complete=matches!(&outcome.result,Some(ActionResult::ShellDispatch(result)) if result.progress.complete());
        if complete {
            // A lost waiter/source cannot undo a positively observed complete socket frame.
            outcome.state=ReceiptState::Dispatched;outcome.proof=TransitionProof::WriteCompleted;outcome.error=None;
        }else if outcome.state==ReceiptState::Failed&&known_effect {
            outcome.state=ReceiptState::Uncertain;outcome.proof=TransitionProof::LostOutcome;outcome.error=None;
        }
        apply_transition(&mut receipt,outcome)?;
        write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
}

fn retained_creation_shell_fence(receipt:&Receipt,stages:Option<&[CreationStageRecord]>)->Option<crate::herdr::shells::CreationShellFence> {
    let (server,pane,terminal)=if receipt.method==ActionMethod::SessionCreate&&receipt.state!=ReceiptState::Succeeded {
        let creation=receipt.creation.as_ref()?;
        (creation.server_id.clone(),creation.resources.pane_id.clone()?,creation.resources.terminal_id.clone()?)
    }else if matches!(receipt.method,ActionMethod::ShellCreate|ActionMethod::TerminalCreate)&&receipt.state!=ReceiptState::Succeeded {
        let ActionResult::ShellCreation(result)=receipt.result.as_ref()? else{return None};
        (result.resources.server_id.clone()?,result.resources.pane_id.clone()?,result.resources.terminal_id.clone()?)
    }else{return None};
    let incarnation=stages.and_then(|records|records.iter().find(|record|record.stage==CreationStage::Workspace&&record.intent.workspace_operation==WorkspaceOperation::Create))
        .and_then(|record|record.result.as_ref().and_then(|metadata|metadata.socket_epoch.clone()).or_else(||record.intent.socket_epoch.clone()));
    Some(crate::herdr::shells::CreationShellFence{server,incarnation,baseline:Default::default(),created:Some((pane,terminal))})
}

impl Storage {
    pub(crate) fn creation_shell_fences(&self)->StoreResult<(Vec<(Uuid,Uuid,crate::herdr::shells::CreationShellFence)>,std::collections::HashSet<Uuid>)> {
        let mut statement=self.db.prepare("SELECT peer,action,fence FROM creation_shell_fences LIMIT 513")?;
        let rows=statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?;
        let mut result=Vec::new();let mut unproved=std::collections::HashSet::new();
        for row in rows {
            let (peer,action,json)=row?;
            let fence:crate::herdr::shells::CreationShellFence=serde_json::from_str(&json).map_err(|_|StorageError::Corrupt)?;
            fence.validate()?;result.push((Uuid::new(peer)?,Uuid::new(action)?,fence));
            if result.len()>512{return Err(StorageError::Corrupt);}
        }
        // Retained bootstrap roots are negative facts, never ordinary-terminal authority.
        // Missing historical incarnation data can only deny this exact pane/terminal pair.
        let mut statement=self.db.prepare("SELECT r.peer,r.action,r.receipt,c.stages FROM receipts r LEFT JOIN creation c ON c.peer=r.peer AND c.action=r.action WHERE NOT EXISTS (SELECT 1 FROM creation_shell_fences f WHERE f.peer=r.peer AND f.action=r.action)")?;
        let rows=statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?)))?;
        for row in rows {
            let (peer,action,json,stages)=row?;
            let receipt:Receipt=decode(&json)?;
            let stages:Option<Vec<CreationStageRecord>>=stages.as_deref().map(decode).transpose()?;
            if let Some(fence)=retained_creation_shell_fence(&receipt,stages.as_deref()) {
                fence.validate()?;result.push((Uuid::new(peer)?,Uuid::new(action)?,fence));
                if result.len()>512{return reject(ErrorCode::ResourceLimit);}
            }else if !matches!(receipt.state,ReceiptState::Succeeded|ReceiptState::Failed) {
                if let Some(creation)=&receipt.creation {
                    if creation.resources.workspace_id.is_some()||creation.stage_state==StageState::Uncertain
                        &&matches!(creation.stage,CreationStage::Workspace|CreationStage::Launch|CreationStage::Ready)
                    {unproved.insert(creation.server_id.clone());}
                }else if matches!(receipt.method,ActionMethod::ShellCreate|ActionMethod::TerminalCreate) {
                    let resources=match &receipt.result {Some(ActionResult::ShellCreation(result))=>Some(&result.resources),_=>None};
                    let source=receipt.target.as_ref().map(|target|&target.session_id).or_else(||receipt.shell_target.as_ref().map(|target|&target.shell_id));
                    if let Some(server)=resources.and_then(|resources|resources.server_id.as_ref()){unproved.insert(server.clone());}
                    else if let Some(source)=source {
                        let key:Option<String>=self.db.query_row("SELECT resource_key FROM sessions WHERE session=?1",[&source.0],|row|row.get(0)).optional()?;
                        let key:StableResourceKey=decode(&key.ok_or(StorageError::Policy(ErrorCode::NotReady))?)?;
                        unproved.insert(key.server().clone());
                    }else if resources.is_some(){return reject(ErrorCode::NotReady);}
                }
            }
        }Ok((result,unproved))
    }
    pub(crate) fn retire_confirmed_creation_shell_fences(&mut self,server:&Uuid,incarnation:&Uuid,snapshot:&crate::herdr::Snapshot)->StoreResult<Vec<(Uuid,Uuid)>> {
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed={
            let mut statement=tx.prepare("SELECT f.peer,f.action,f.fence,r.receipt FROM creation_shell_fences f JOIN receipts r ON r.peer=f.peer AND r.action=f.action WHERE r.state='succeeded'")?;
            let rows=statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?)))?;
            let mut removed=Vec::new();
            for row in rows {
                let (peer,action,json,receipt)=row?;
                let fence:crate::herdr::shells::CreationShellFence=decode(&json)?;fence.validate()?;
                let receipt:Receipt=decode(&receipt)?;receipt.validate()?;
                if &fence.server!=server||receipt.method!=ActionMethod::SessionCreate{continue;}
                let creation=receipt.creation.as_ref().ok_or(StorageError::Corrupt)?;
                let pane=creation.resources.pane_id.as_ref().ok_or(StorageError::Corrupt)?;
                let terminal=creation.resources.terminal_id.as_ref().ok_or(StorageError::Corrupt)?;
                if creation.server_id!=fence.server||fence.created.as_ref().is_some_and(|known|known.0!=*pane||known.1!=*terminal){return Err(StorageError::Corrupt);}
                let observed=fence.incarnation.as_ref().is_some_and(|original|original!=incarnation)
                    ||snapshot.pane(pane).map_or(true,|current|current.terminal_id!=*terminal||current.agent.is_some()||current.agent_session.is_some()
                        ||snapshot.agents.iter().any(|agent|agent.pane_id==*pane||agent.terminal_id==*terminal));
                if observed{removed.push((Uuid::new(peer)?,Uuid::new(action)?));}
            }removed
        };
        for (peer,action) in &removed {tx.execute("DELETE FROM creation_shell_fences WHERE peer=?1 AND action=?2",params![peer.0,action.0])?;}
        tx.commit()?;Ok(removed)
    }
    pub(crate) fn retain_creation_shell_fence(&mut self,proof:&AuthenticatedPeer,action:&Uuid,fence:crate::herdr::shells::CreationShellFence)->StoreResult<()> {
        fence.validate()?;
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;authorize(&tx,proof)?;
        let receipt=get_receipt(&tx,proof,action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if !matches!(receipt.method,ActionMethod::SessionCreate|ActionMethod::TerminalCreate){return reject(ErrorCode::InvalidRequest);}
        let old:Option<String>=tx.query_row("SELECT fence FROM creation_shell_fences WHERE peer=?1 AND action=?2",params![proof.peer.0,action.0],|row|row.get(0)).optional()?;
        if let Some(old)=old {
            let old:crate::herdr::shells::CreationShellFence=serde_json::from_str(&old).map_err(|_|StorageError::Corrupt)?;
            if old.server!=fence.server||old.incarnation!=fence.incarnation||(old.baseline!=fence.baseline&&!(fence.created.is_some()&&fence.baseline.is_empty()))
                ||old.created.as_ref().is_some_and(|known|fence.created.as_ref()!=Some(known)){return Err(StorageError::Corrupt);}
        }else{
            let count:i64=tx.query_row("SELECT count(*) FROM creation_shell_fences",[],|row|row.get(0))?;
            if count>=512{return reject(ErrorCode::ResourceLimit);}
            if receipt.state!=ReceiptState::Reserved{return reject(ErrorCode::NotReady);}
        }
        tx.execute("INSERT INTO creation_shell_fences(peer,action,fence) VALUES(?1,?2,?3) ON CONFLICT(peer,action) DO UPDATE SET fence=excluded.fence",
            params![proof.peer.0,action.0,serde_json::to_string(&fence).map_err(|_|StorageError::Corrupt)?])?;
        tx.commit()?;Ok(())
    }
    pub(crate) fn narrow_creation_shell_fence(&mut self,proof:&AuthenticatedPeer,action:&Uuid,pane:&Id,terminal:&Id)->StoreResult<crate::herdr::shells::CreationShellFence> {
        authorize(&self.db,proof)?;
        let receipt=get_receipt(&self.db,proof,action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        let matches=receipt.creation.as_ref().is_some_and(|creation|creation.resources.pane_id.as_ref()==Some(pane)&&creation.resources.terminal_id.as_ref()==Some(terminal))
            ||matches!(&receipt.result,Some(ActionResult::ShellCreation(result)) if result.resources.complete()&&result.resources.pane_id.as_ref()==Some(pane)&&result.resources.terminal_id.as_ref()==Some(terminal));
        if !matches{return reject(ErrorCode::NotReady);}
        let value:String=self.db.query_row("SELECT fence FROM creation_shell_fences WHERE peer=?1 AND action=?2",params![proof.peer.0,action.0],|row|row.get(0))?;
        let mut fence:crate::herdr::shells::CreationShellFence=decode(&value)?;
        fence.created=Some((pane.clone(),terminal.clone()));
        fence.baseline.clear();
        self.retain_creation_shell_fence(proof,action,fence.clone())?;Ok(fence)
    }
    pub(crate) fn release_creation_shell_fence(&mut self,proof:&AuthenticatedPeer,action:&Uuid)->StoreResult<()> {
        // Original-job no-effect cleanup only; revocation must not strand an unentered
        // denial. This grants no read/write authority and never removes native resources.
        let epoch:i64=self.db.query_row("SELECT epoch FROM receipts WHERE peer=?1 AND action=?2",params![proof.peer.0,action.0],|row|row.get(0))?;
        if stored_u64(epoch)?!=proof.epoch(){return reject(ErrorCode::Unauthorized);}
        let receipt=get_receipt(&self.db,proof,action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if !matches!(receipt.method,ActionMethod::SessionCreate|ActionMethod::TerminalCreate)
            ||!matches!(receipt.state,ReceiptState::Reserved|ReceiptState::Failed)
            ||receipt.creation.as_ref().is_some_and(|creation|creation.resources.workspace_id.is_some()||creation.resources.pane_id.is_some()||creation.resources.terminal_id.is_some())
            ||receipt.result.as_ref().is_some_and(|result|!matches!(result,ActionResult::ShellCreation(result) if result.resources.workspace_id.is_none()&&result.resources.tab_id.is_none()&&result.resources.pane_id.is_none()&&result.resources.terminal_id.is_none()))
        {return reject(ErrorCode::NotReady);}
        self.db.execute("DELETE FROM creation_shell_fences WHERE peer=?1 AND action=?2",params![proof.peer.0,action.0])?;Ok(())
    }
}

fn validate_alias_facts(receipt:&Receipt,result:&Option<ActionResultAgentRenamed>)->StoreResult<()> {
    if receipt.method!=ActionMethod::AgentRename || receipt.target.is_none() || receipt.shell_target.is_some() || receipt.creation.is_some() {return Err(StorageError::Corrupt)}
    if let Some(result)=result {result.validate()?;if Some(&result.target)!=receipt.target.as_ref() {return Err(StorageError::Corrupt)}}
    match &receipt.result {None=>Ok(()),Some(ActionResult::AgentRenamed(old)) if result.as_ref()==Some(old)=>Ok(()),_=>Err(StorageError::Corrupt)}
}
impl Storage {
    pub(crate) fn checkpoint_agent_alias_facts(&mut self,ticket:&DispatchTicket,result:ActionResultAgentRenamed,at:u64)->StoreResult<()> {
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;authorize(&tx,&ticket.proof)?;
        let mut receipt=ticket_receipt(&tx,ticket)?;
        if ticket.stage.is_some() || receipt.state!=ReceiptState::Dispatching {return reject(ErrorCode::NotReady)}
        validate_alias_facts(&receipt,&Some(result.clone()))?;
        receipt.result=Some(ActionResult::AgentRenamed(result));receipt.updated_at=timestamp(at.max(receipt.updated_at.0))?;
        write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
    pub(crate) fn settle_agent_alias_dispatch(&mut self,ticket:DispatchTicket,mut outcome:ReceiptTransition,result:Option<ActionResultAgentRenamed>)->StoreResult<()> {
        if ticket.stage.is_some() {return reject(ErrorCode::InvalidRequest)}
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;let mut receipt=ticket_receipt(&tx,&ticket)?;
        validate_alias_facts(&receipt,&result)?;
        match &outcome.result {Some(ActionResult::AgentRenamed(value)) if result.as_ref()==Some(value)=>{},None=>outcome.result=result.clone().map(ActionResult::AgentRenamed),_=>return Err(StorageError::Corrupt)}
        if outcome.state==ReceiptState::Failed && result.is_some() {outcome.state=ReceiptState::Uncertain;outcome.proof=TransitionProof::LostOutcome;outcome.error=None;}
        apply_transition(&mut receipt,outcome)?;write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
}


impl Storage {
    pub(crate) fn attachment_load(&self,peer:&Uuid,upload:&Uuid)->StoreResult<Option<crate::attachments::Record>> {
        let value:Option<(i64,bool,bool,String)>=self.db.query_row("SELECT size,unfinished,discarded,record FROM attachments WHERE peer=?1 AND upload=?2",params![peer.0,upload.0],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
        value.map(|(size,unfinished,discarded,value)|{let size=u64::try_from(size).map_err(|_|StorageError::Corrupt)?;if value.len()>16384{return Err(StorageError::Corrupt)}let record:crate::attachments::Record=decode(&value)?;record.validate()?;if record.peer!=*peer||record.upload!=*upload||record.size!=size||unfinished!=(!record.published&&!record.discarded)||discarded!=record.discarded{return Err(StorageError::Corrupt)}Ok(record)}).transpose()
    }
    pub(crate) fn attachment_save(&mut self,record:&crate::attachments::Record,new:bool)->StoreResult<()> {
        record.validate()?;
        let size=i64::try_from(record.size).map_err(|_|StorageError::Corrupt)?;
        let changed=if new {self.db.execute("INSERT INTO attachments(peer,upload,size,unfinished,discarded,record) VALUES(?1,?2,?3,?4,?5,?6)",params![record.peer.0,record.upload.0,size,!record.published&&!record.discarded,record.discarded,json(record)?])?}
        else {self.db.execute("UPDATE attachments SET unfinished=?3,discarded=?4,record=?5 WHERE peer=?1 AND upload=?2 AND size=?6",params![record.peer.0,record.upload.0,!record.published&&!record.discarded,record.discarded,json(record)?,size])?};
        if changed!=1{return Err(StorageError::Corrupt)}Ok(())
    }
    pub(crate) fn attachment_capacity(&self,peer:&Uuid,size:u64)->StoreResult<()> {
        let (count,unfinished,bytes):(i64,i64,i64)=self.db.query_row("SELECT COUNT(*),COALESCE(SUM(unfinished),0),COALESCE(SUM(size),0) FROM attachments WHERE peer=?1 AND discarded=0",params![peer.0],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let (count,unfinished,bytes)=(u64::try_from(count).map_err(|_|StorageError::Corrupt)?,u64::try_from(unfinished).map_err(|_|StorageError::Corrupt)?,u64::try_from(bytes).map_err(|_|StorageError::Corrupt)?);
        let all:i64=self.db.query_row("SELECT COALESCE(SUM(size),0) FROM attachments WHERE discarded=0",[],|r|r.get(0))?;
        let all=u64::try_from(all).map_err(|_|StorageError::Corrupt)?;
        let consumed:i64=self.db.query_row("SELECT COUNT(*) FROM attachments WHERE peer=?1",params![peer.0],|r|r.get(0))?;
        let consumed=u64::try_from(consumed).map_err(|_|StorageError::Corrupt)?;
        let peer_bytes=bytes.checked_add(size).ok_or(StorageError::Policy(ErrorCode::ResourceLimit))?;
        let host_bytes=all.checked_add(size).ok_or(StorageError::Policy(ErrorCode::ResourceLimit))?;
        if consumed>=4096||count>=32||unfinished>=4||peer_bytes>64*1024*1024||host_bytes>256*1024*1024{return reject(ErrorCode::ResourceLimit)}Ok(())
    }
    pub(crate) fn checkpoint_attachment_facts(&mut self,ticket:&DispatchTicket,result:ActionResultAttachment,at:u64)->StoreResult<()> {
        result.validate()?;let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;authorize(&tx,&ticket.proof)?;
        let mut receipt=ticket_receipt(&tx,ticket)?;
        if ticket.stage.is_some()||receipt.state!=ReceiptState::Dispatching||receipt.target.as_ref()!=Some(&result.target){return reject(ErrorCode::NotReady)}
        if let Some(known)=&receipt.result {let ActionResult::Attachment(known)=known else{return Err(StorageError::Corrupt)};if !result.progress.retains(&known.progress){return Err(StorageError::Corrupt)}}
        receipt.result=Some(ActionResult::Attachment(result));receipt.updated_at=timestamp(at.max(receipt.updated_at.0))?;write_receipt(&tx,&ticket.proof.peer.0,&ticket.action.0,&receipt)?;tx.commit()?;Ok(())
    }
    pub(crate) fn settle_attachment_dispatch(&mut self,ticket:DispatchTicket,mut outcome:ReceiptTransition,result:ActionResultAttachment)->StoreResult<()> {
        result.validate()?;
        if let Some(ActionResult::Attachment(known))=&outcome.result {if known.target!=result.target||!known.progress.retains(&result.progress){return Err(StorageError::Corrupt)}}else if outcome.result.is_none(){outcome.result=Some(ActionResult::Attachment(result.clone()))}else{return Err(StorageError::Corrupt)}
        // An entered local file effect is never projected as a definitive absence.
        if outcome.state==ReceiptState::Failed {outcome.state=ReceiptState::Uncertain;outcome.proof=TransitionProof::LostOutcome;outcome.error=None;}
        self.settle_dispatch(ticket,outcome)
    }
}
impl Storage {
    pub(crate) fn attachment_cleanup_context(&self,proof:&AuthenticatedPeer,params:&AttachmentsDiscardParams)->StoreResult<crate::attachments::CleanupContext> {
        authorize(&self.db,proof)?;
        let record=self.attachment_load(proof.peer_id(),&params.upload_id)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        let context=crate::attachments::CleanupContext::from_owned(record,proof.peer_id(),&params.target,&params.upload_id)?;
        match self.image_upload_pinned(proof.peer_id(),&params.upload_id) {
            Ok(false)=>Ok(context),Ok(true)=>reject(ErrorCode::Busy),Err(_)=>reject(ErrorCode::NotReady),
        }
    }
    pub(crate) fn authorized_attachment_status(&mut self,proof:&AuthenticatedPeer,params:&AttachmentsStatusParams,context:Option<&crate::attachments::ProviderContext>)->StoreResult<(AttachmentsStatusResult,crate::attachments::Record)> {
        authorize(&self.db,proof)?;
        let mut result=self.attachment_status(proof.peer_id(),&params.target,&params.upload_id)?;
        let record=self.attachment_load(proof.peer_id(),&params.upload_id)?.ok_or(StorageError::Corrupt)?;
        result.insertion_ready=context.is_some_and(|context|record.insertion_ready(context));
        if context.is_some_and(|context|context.epoch!=record.source) {self.retire_image_source(&record.target,&record.source);}
        result.native_preparation=self.image_preparation_status(proof.peer_id(),&params.upload_id);
        Ok((result,record))
    }
}
impl Storage {
    pub(crate) fn attachment_maintenance(&mut self,now:u64,recover:bool)->StoreResult<()> {
        for peer in self.attachment_peers()? {self.attachment_maintenance_peer_mode(&peer,&[],now,recover)?;}Ok(())
    }}
impl Storage {
    pub(crate) fn attachment_peers(&self)->StoreResult<Vec<Uuid>> {let mut statement=self.db.prepare("SELECT DISTINCT peer FROM attachments ORDER BY peer LIMIT 10001")?;let rows:Vec<String>=statement.query_map([],|row|row.get(0))?.collect::<Result<_,_>>()?;if rows.len()>PEER_CAPACITY{return Err(StorageError::Corrupt)}rows.into_iter().map(|id|Uuid::new(id).map_err(Into::into)).collect()}
    pub(crate) fn attachment_maintenance_peer(&mut self,peer:&Uuid,busy:&[Uuid],now:u64)->StoreResult<()> {self.attachment_maintenance_peer_mode(peer,busy,now,false)}
    fn attachment_maintenance_peer_mode(&mut self,peer:&Uuid,busy:&[Uuid],now:u64,recover:bool)->StoreResult<()> {
        self.ensure_image_reads();
        let records:Vec<String>={let mut statement=self.db.prepare("SELECT record FROM attachments WHERE peer=?1 LIMIT 4097")?;statement.query_map(params![peer.0],|row|row.get(0))?.collect::<Result<_,_>>()?};
        if records.len()>4096{return Err(StorageError::Corrupt)}
        for data in records {if data.len()>16384{return Err(StorageError::Corrupt)}let decoded:crate::attachments::Record=decode(&data)?;let mut record=self.attachment_load(peer,&decoded.upload)?.ok_or(StorageError::Corrupt)?;record.validate()?;if record.peer!=*peer{return Err(StorageError::Corrupt)}
        if busy.contains(&record.upload){continue}
            // Only entered native readers retain a bounded negative obligation
            // beyond ordinary upload/receipt TTL. Unknown ownership never unlinks.
            match self.image_upload_pinned(peer,&record.upload) {Ok(false)=>{},Ok(true)|Err(_)=>continue}
            if recover&&record.recover().is_err(){continue}
            if record.discarded {if record.tombstone_until<=now {self.db.execute("DELETE FROM attachments WHERE peer=?1 AND upload=?2 AND discarded=1",params![record.peer.0,record.upload.0])?;}continue}
            if record.expire(now).is_err(){continue}self.attachment_save(&record,false)?;
        }Ok(())
    }
}

struct ImageUseSqlRow {peer:String,action:String,input:String,record:String,mac:Vec<u8>}
fn image_use_row(row:&rusqlite::Row<'_>)->rusqlite::Result<ImageUseSqlRow> {
    let text=|index,limit|->rusqlite::Result<String> {
        let rusqlite::types::ValueRef::Text(bytes)=row.get_ref(index)? else {return Err(rusqlite::Error::InvalidQuery)};
        if bytes.len()>limit {return Err(rusqlite::Error::InvalidQuery)}
        std::str::from_utf8(bytes).map(str::to_owned).map_err(|_|rusqlite::Error::InvalidQuery)
    };
    let rusqlite::types::ValueRef::Blob(mac)=row.get_ref(4)? else {return Err(rusqlite::Error::InvalidQuery)};
    if mac.len()!=32 {return Err(rusqlite::Error::InvalidQuery)}
    Ok(ImageUseSqlRow {peer:text(0,36)?,action:text(1,36)?,input:text(2,36)?,record:text(3,8192)?,mac:mac.to_vec()})
}
fn decode_image_use(key:&[u8;32],row:ImageUseSqlRow)->StoreResult<crate::attachments::ImageUseRecord> {
    if row.record.len()>8192||row.mac.len()!=32||!verify_hmac_sha256(key,row.record.as_bytes(),&row.mac) {return Err(StorageError::Corrupt)}
    let record:crate::attachments::ImageUseRecord=decode(&row.record)?;record.facts.validate()?;
    if record.facts.peer.0!=row.peer||record.facts.action.0!=row.action||record.facts.input.0!=row.input {return Err(StorageError::Corrupt)}
    Ok(record)
}
impl Storage {
    pub(crate) fn reserve_image_capacity(&self,facts:&crate::attachments::ImageUseFacts)->StoreResult<crate::attachments::ImageReservation> {
        crate::attachments::reserve_image_owner(&self.image_budget,facts)
    }
    pub(crate) fn hold_image_read(&mut self,owner:std::sync::Arc<crate::attachments::ImageReadObligation>)->StoreResult<()> {
        if self.image_reads.len()>=crate::attachments::IMAGE_ACTIONS_PER_HOST||self.image_reads.iter().any(|known|known.facts.input==owner.facts.input) {return Err(StorageError::Corrupt)}
        self.image_reads.push(owner);Ok(())
    }
    pub(crate) fn ensure_image_reads(&mut self)->bool {
        if self.image_reads_restored {return !self.image_reads_fault}
        self.image_reads_restored=true;
        let records=match self.image_uses() {Ok(records)=>records,Err(_)=>{self.image_reads_fault=true;return false}};
        for record in records {
            if !record.entered {
                // The authenticated durable mark must precede every native byte.
                // A pre-entry crash restores no positive authority and wrote no input.
                if self.image_use_release(&record.facts).is_err() {self.image_reads_fault=true;return false}
                continue;
            }
            match self.restore_image_read(record.facts) {
                Ok(owner)=>if self.hold_image_read(owner).is_err() {self.image_reads_fault=true;return false},
                Err(_)=>{self.image_reads_fault=true;return false},
            }
        }
        true
    }
    pub(crate) fn image_preparation_status(&mut self,peer:&Uuid,upload:&Uuid)->Option<NativeImagePreparation> {
        let ready=self.ensure_image_reads();let (peer_count,host_count)=self.image_budget.lock().counts(peer);
        let owners:Vec<_>=self.image_reads.iter().filter(|owner|&owner.facts.peer==peer&&owner.facts.uploads.contains(upload)).collect();
        let mut actions:Vec<_>=owners.iter().map(|owner|owner.facts.action.clone()).collect();
        if !ready {return Some(NativeImagePreparation {reason:ImagePreparationReason::OwnershipProofUnavailable,actions,peer_owners:None,host_owners:None})}
        let reason=if owners.is_empty() {
            if peer_count<crate::attachments::IMAGE_ACTIONS_PER_PEER&&host_count<crate::attachments::IMAGE_ACTIONS_PER_HOST {return None}
            actions=self.image_reads.iter().filter(|owner|&owner.facts.peer==peer).map(|owner|owner.facts.action.clone()).collect();
            ImagePreparationReason::CapacityInUse
        } else if owners.iter().any(|owner|*owner.reason.lock()==ImagePreparationReason::OwnershipProofUnavailable) {ImagePreparationReason::OwnershipProofUnavailable}
        else if owners.iter().any(|owner|*owner.reason.lock()==ImagePreparationReason::OriginalSourceUnavailable) {ImagePreparationReason::OriginalSourceUnavailable}
        else {ImagePreparationReason::PendingOriginalRead};
        Some(NativeImagePreparation {reason,actions,peer_owners:Some(Uint(peer_count as u64)),host_owners:Some(Uint(host_count as u64))})
    }
    pub(crate) fn retire_image_source(&mut self,target:&Target,source:&Uuid) {
        for owner in &self.image_reads {
            if &owner.facts.target==target&&&owner.facts.source==source {*owner.reason.lock()=ImagePreparationReason::OriginalSourceUnavailable;}
        }
    }
    pub(crate) fn image_read_prepared(&mut self,original:&crate::backends::OriginalContext,input:&Uuid)->StoreResult<bool> {
        if !self.ensure_image_reads() {return Ok(false)}
        let Some(owner)=self.image_reads.iter().find(|owner|&owner.facts.input==input).cloned() else {return Ok(false)};
        if !owner.entered.load(std::sync::atomic::Ordering::Acquire)||!owner.matches(original) {return Ok(false)}
        if original.recheck().is_err()||owner.recheck().is_err() {
            *owner.reason.lock()=ImagePreparationReason::OwnershipProofUnavailable;return Ok(false)
        }
        self.release_image_owner(&owner)?;Ok(true)
    }
    pub(crate) fn release_image_owner(&mut self,owner:&std::sync::Arc<crate::attachments::ImageReadObligation>)->StoreResult<()> {
        if let Err(error)=self.image_use_release(&owner.facts) {
            self.image_reads_fault=true;*owner.reason.lock()=ImagePreparationReason::OwnershipProofUnavailable;return Err(error)
        }
        self.image_reads.retain(|known|!std::sync::Arc::ptr_eq(known,owner));
        Ok(())
    }
    fn image_use_key(&self)->[u8;32] {hmac_sha256(&self.receipt_key,b"goatr native image read obligation key v1")}
    pub(crate) fn image_uses(&self)->StoreResult<Vec<crate::attachments::ImageUseRecord>> {
        let mut statement=self.db.prepare("SELECT peer,action,native_input,record,mac FROM attachment_image_uses LIMIT ?1")?;
        let rows=statement.query_map([crate::attachments::IMAGE_ACTIONS_PER_HOST as i64+1],image_use_row)?.collect::<Result<Vec<_>,_>>()?;
        if rows.len()>crate::attachments::IMAGE_ACTIONS_PER_HOST {return Err(StorageError::Corrupt)}
        let key=self.image_use_key();let mut counts=HashMap::new();
        rows.into_iter().map(|row|{
            let record=decode_image_use(&key,row)?;
            let epoch:Option<i64>=self.db.query_row("SELECT epoch FROM peers WHERE peer=?1",[&record.facts.peer.0],|row|row.get(0)).optional()?;
            if epoch.and_then(|epoch|u64::try_from(epoch).ok()).is_none_or(|epoch|epoch<record.facts.peer_epoch) {return Err(StorageError::Corrupt)}
            let count=counts.entry(record.facts.peer.clone()).or_insert(0usize);*count+=1;
            if *count>crate::attachments::IMAGE_ACTIONS_PER_PEER {return Err(StorageError::Corrupt)}Ok(record)
        }).collect()
    }
    pub(crate) fn image_use_for_input(&self,input:&Uuid)->StoreResult<Option<crate::attachments::ImageUseRecord>> {
        let row=self.db.query_row("SELECT peer,action,native_input,record,mac FROM attachment_image_uses WHERE native_input=?1",[&input.0],image_use_row).optional()?;
        row.map(|row|decode_image_use(&self.image_use_key(),row)).transpose()
    }
    fn image_use_for_action(&self,peer:&Uuid,action:&Uuid)->StoreResult<Option<crate::attachments::ImageUseRecord>> {
        let row=self.db.query_row("SELECT peer,action,native_input,record,mac FROM attachment_image_uses WHERE peer=?1 AND action=?2",params![peer.0,action.0],image_use_row).optional()?;
        row.map(|row|decode_image_use(&self.image_use_key(),row)).transpose()
    }
    pub(crate) fn image_use_reserve(&mut self,proof:&AuthenticatedPeer,facts:&crate::attachments::ImageUseFacts)->StoreResult<()> {
        authorize(&self.db,proof)?;facts.validate()?;
        if facts.peer!=*proof.peer_id()||facts.peer_epoch!=proof.epoch() {return reject(ErrorCode::Unauthorized)}
        let receipt=get_receipt(&self.db,proof,&facts.action)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if receipt.state!=ReceiptState::Reserved||receipt.method!=ActionMethod::PromptSubmit||receipt.target.as_ref()!=Some(&facts.target)||receipt.boot_id!=facts.boot {return reject(ErrorCode::NotReady)}
        let known=self.image_uses()?;
        if known.len()>=crate::attachments::IMAGE_ACTIONS_PER_HOST||known.iter().filter(|known|known.facts.peer==facts.peer).count()>=crate::attachments::IMAGE_ACTIONS_PER_PEER {return reject(ErrorCode::ResourceLimit)}
        for upload in &facts.uploads {
            let record=self.attachment_load(&facts.peer,upload)?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
            if !record.matches_image_source(facts)||!record.published||record.discarded {return reject(ErrorCode::StaleSession)}
        }
        let record=json(&crate::attachments::ImageUseRecord {facts:facts.clone(),entered:false})?;
        if record.len()>8192 {return Err(StorageError::Corrupt)}
        let mac=hmac_sha256(&self.image_use_key(),record.as_bytes());
        self.db.execute("INSERT INTO attachment_image_uses(peer,action,native_input,record,mac) VALUES(?1,?2,?3,?4,?5)",params![facts.peer.0,facts.action.0,facts.input.0,record,mac.as_slice()])?;
        Ok(())
    }
    pub(crate) fn image_use_enter(&mut self,ticket:&DispatchTicket,facts:&crate::attachments::ImageUseFacts)->StoreResult<()> {
        let receipt=ticket_receipt(&self.db,ticket)?;
        if ticket.stage.is_some()||ticket.action!=facts.action||ticket.proof.peer_id()!=&facts.peer||ticket.proof.epoch()!=facts.peer_epoch
            ||receipt.method!=ActionMethod::PromptSubmit||receipt.state!=ReceiptState::Dispatching||receipt.target.as_ref()!=Some(&facts.target)||receipt.boot_id!=facts.boot {return reject(ErrorCode::NotReady)}
        let mut known=self.image_use_for_action(&facts.peer,&facts.action)?.ok_or(StorageError::Corrupt)?;
        if known.entered||known.facts!=*facts {return reject(ErrorCode::NotReady)}
        known.entered=true;let record=json(&known)?;let mac=hmac_sha256(&self.image_use_key(),record.as_bytes());
        if self.db.execute("UPDATE attachment_image_uses SET record=?3,mac=?4 WHERE peer=?1 AND action=?2",params![facts.peer.0,facts.action.0,record,mac.as_slice()])?!=1 {return Err(StorageError::Corrupt)}Ok(())
    }
    pub(crate) fn image_use_release(&mut self,facts:&crate::attachments::ImageUseFacts)->StoreResult<()> {
        let Some(known)=self.image_use_for_action(&facts.peer,&facts.action)? else {return Ok(())};
        if known.facts!=*facts {return Err(StorageError::Corrupt)}
        self.db.execute("DELETE FROM attachment_image_uses WHERE peer=?1 AND action=?2 AND native_input=?3",params![facts.peer.0,facts.action.0,facts.input.0])?;Ok(())
    }
    pub(crate) fn image_upload_pinned(&self,peer:&Uuid,upload:&Uuid)->StoreResult<bool> {
        Ok(self.image_uses()?.iter().any(|record|record.entered&&&record.facts.peer==peer&&record.facts.uploads.contains(upload)))
    }
}

fn attention_transition_tx(tx:&Transaction<'_>,change:&AttentionTransition,now:u64)->StoreResult<Vec<(Uuid,AttentionEvent)>> {
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT generation,status FROM checkpoint WHERE session=?1",
                [&change.target.session_id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let category = if change.snapshot {
            None
        } else if change.new_actionable
            || (change.current == SessionStatus::Attention
                && change.previous != SessionStatus::Attention)
        {
            Some(AttentionCategory::Attention)
        } else if change.previous == SessionStatus::Working
            && change.current == SessionStatus::Idle
            && old.as_ref().is_some_and(|(generation, status)| {
                generation == &change.target.generation.0
                    && decode::<SessionStatus>(status).ok() == Some(SessionStatus::Working)
            })
        {
            Some(AttentionCategory::Completed)
        } else {
            None
        };
        tx.execute("INSERT INTO checkpoint VALUES(?1,?2,?3) ON CONFLICT(session) DO UPDATE SET generation=excluded.generation,status=excluded.status", params![change.target.session_id.0, change.target.generation.0, json(&change.current)?])?;
        let duplicate: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM transitions WHERE session=?1 AND correlation=?2)",
            params![change.target.session_id.0, change.correlation.0],
            |r| r.get(0),
        )?;
        let mut events = vec![];
        if let Some(category) = category.filter(|_| !duplicate) {
            tx.execute(
                "INSERT INTO transitions VALUES(?1,?2,?3)",
                params![
                    change.target.session_id.0,
                    change.correlation.0,
                    sql_integer(now)?
                ],
            )?;
            let peers: Vec<(String, i64)> = {
                let mut s = tx.prepare("SELECT peer,highwater FROM peers WHERE paired=1")?;
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?
            };
            for (peer, highwater) in peers {
                let highwater = stored_uint(highwater)?;
                if highwater >= UINT_MAX - 1 {
                    return reject(ErrorCode::ResourceLimit);
                }
                prune_attention(tx, &peer, now)?;
                let event = AttentionEvent {
                    event_id: Uuid::random(),
                    sequence: Uint(highwater + 1),
                    session_id: change.target.session_id.clone(),
                    generation: change.target.generation.clone(),
                    category,
                    created_at: timestamp(now)?,
                };
                tx.execute(
                    "INSERT INTO attention(peer,sequence,event,session,at) VALUES(?1,?2,?3,?4,?5)",
                    params![
                        peer,
                        sql_integer(event.sequence.0)?,
                        json(&event)?,
                        event.session_id.0,
                        sql_integer(now)?
                    ],
                )?;
                tx.execute(
                    "UPDATE peers SET highwater=?1 WHERE peer=?2",
                    params![sql_integer(event.sequence.0)?, peer],
                )?;
                tx.execute(
                    "UPDATE readwater SET last_event=?1 WHERE peer=?2 AND session=?3",
                    params![sql_integer(now)?, peer, event.session_id.0],
                )?;
                prune_attention(tx, &peer, now)?;
                push::enqueue_event(tx,&peer,&event,now)?;
                events.push((Uuid::new(peer)?, event));
            }
        }
        // Native replay must be installed as a snapshot, not declared a new transition by5.
        // The correlation ledger shares the attention dedup age/count policy.
        if now >= RETENTION_MS {
            tx.execute(
                "DELETE FROM transitions WHERE at<=?1",
                [sql_integer(now - RETENTION_MS)?],
            )?;
        }
        tx.execute("DELETE FROM transitions WHERE rowid IN (SELECT rowid FROM transitions ORDER BY at DESC,rowid DESC LIMIT -1 OFFSET ?1)",[sql_integer(PEER_CAPACITY)?])?;

 Ok(events)
}

#[cfg(test)]
mod terminal_fact_tests {
    use super::*;
    fn owner()->(Storage,AuthenticatedPeer) {
        let mut store=Storage::memory([1;32],0).unwrap();
        let device=PairedDevice{device_id:Uuid::random(),endpoint_id:EndpointId::new(iroh::SecretKey::from_bytes(&[52;32]).public().to_string()).unwrap(),
            device_name:Label::new("terminal fact owner").unwrap(),paired_at:Uint(0),last_seen_at:None};
        let epoch=store.pair_device(&device).unwrap();
        let proof=store.authenticated_hello(&device.device_id,epoch).unwrap();
        (store,proof)
    }
    #[test]
    fn corrupt_image_obligation_fails_image_restore_closed_without_blocking_other_receipt_reservation() {
        let (mut store,proof)=owner();let action=Uuid::random();let input=Uuid::random();
        store.db.execute("INSERT INTO attachment_image_uses(peer,action,native_input,record,mac) VALUES(?1,?2,?3,?4,?5)",
            params![proof.peer_id().0,action.0,input.0,"invalid private metadata",&[0u8;32][..]]).unwrap();
        assert!(!store.ensure_image_reads());
        let status=store.image_preparation_status(proof.peer_id(),&Uuid::random()).unwrap();
        assert_eq!(status.reason,ImagePreparationReason::OwnershipProofUnavailable);assert!(status.actions.is_empty());
        assert!(status.peer_owners.is_none()&&status.host_owners.is_none());
        let request=Request{id:Uuid::random(),body:RequestBody::PromptSubmit(PromptSubmitParams {
            boot_id:store.boot_id.clone(),target:Target{session_id:Uuid::random(),generation:Uuid::random()},
            text:PromptText::new("independent text receipt").unwrap(),skill:None,delivery:None,native_turn_id:None,images:None,
        })};
        assert!(matches!(store.reserve(&proof,&request,1,|_|Ok(())).unwrap(),Reservation::Reserved(_)));
    }
    #[test]
    fn restart_preserves_only_valid_complete_new_input_checkpoints_as_dispatched() {
        for kind in 0..4 {
            let (mut store,proof)=owner();
            let request=Request{id:Uuid::random(),body:RequestBody::ShellInput(ShellInputParams{boot_id:store.boot_id.clone(),
                target:ShellTarget{shell_id:Uuid::random(),generation:Uuid::random()},subscription:Uuid::random(),input:ShellInput::Text{text:"x".into()}})};
            store.reserve(&proof,&request,1,|_|Ok(())).unwrap();
            if kind!=3 {store.transition(&proof,&request.id,ReceiptTransition{state:ReceiptState::Dispatching,proof:TransitionProof::Intent,result:None,error:None,at:2}).unwrap();}
            let result=if kind==2 {
                serde_json::from_value::<ActionResult>(serde_json::json!({"kind":"shell_input","progress":{"encodedBytes":19,"writtenBytes":19,"flushCompleted":true}})).unwrap()
            }else{ActionResult::ShellDispatch(ActionResultShellDispatch{progress:ShellDispatchProgress{frame_bytes:Some(19),socket_bytes_written:Some(if kind==1{7}else{19}),frame_complete:Some(kind!=1)}})};
            if kind<2 {
                let ticket=store.dispatch_ticket(&proof,&request.id).unwrap();
                let ActionResult::ShellDispatch(dispatch)=&result else{panic!()};
                store.checkpoint_shell_input_facts(&ticket,dispatch.progress.clone(),3).unwrap();
            }else{
                let mut receipt=store.status(&proof,&request.id,3).unwrap().unwrap();receipt.result=Some(result.clone());
                let tx=store.db.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();write_receipt(&tx,&proof.peer.0,&request.id.0,&receipt).unwrap();tx.commit().unwrap();
            }
            let db=std::mem::replace(&mut store.db,Connection::open_in_memory().unwrap());drop(store);
            let mut reopened=Storage::initialize(db,[1;32],4).unwrap();
            let recovered=reopened.status(&proof,&request.id,5).unwrap().unwrap();
            assert_eq!(recovered.state,if kind==0{ReceiptState::Dispatched}else{ReceiptState::Uncertain});
            assert_eq!(recovered.result,Some(result));
        }
    }

    #[test]
    fn strongest_frame_facts_survive_lost_waiters_and_matching_receipts_need_no_new_lease() {
        for written in [0,7,19] {
            let (mut store,proof)=owner();
            let request=Request{id:Uuid::random(),body:RequestBody::ShellInput(ShellInputParams{boot_id:store.boot_id.clone(),
                target:ShellTarget{shell_id:Uuid::random(),generation:Uuid::random()},subscription:Uuid::random(),
                input:ShellInput::Text{text:"memory-only terminal commit".into()}})};
            assert!(matches!(store.reserve(&proof,&request,1,|_|Ok(())).unwrap(),Reservation::Reserved(_)));
            store.transition(&proof,&request.id,ReceiptTransition{state:ReceiptState::Dispatching,proof:TransitionProof::Intent,result:None,error:None,at:2}).unwrap();
            let ticket=store.dispatch_ticket(&proof,&request.id).unwrap();
            let progress=ShellDispatchProgress{frame_bytes:Some(19),socket_bytes_written:Some(written),frame_complete:Some(written==19)};
            store.checkpoint_shell_input_facts(&ticket,progress.clone(),3).unwrap();
            store.settle_shell_input_dispatch(ticket,ReceiptTransition{state:ReceiptState::Uncertain,proof:TransitionProof::LostOutcome,result:None,error:None,at:4},progress.clone()).unwrap();
            let receipt=store.status(&proof,&request.id,5).unwrap().unwrap();
            assert_eq!(receipt.state,if written==19{ReceiptState::Dispatched}else{ReceiptState::Uncertain});
            assert_eq!(receipt.result,Some(ActionResult::ShellDispatch(ActionResultShellDispatch{progress})));
            assert!(!serde_json::to_string(&receipt).unwrap().contains("memory-only terminal commit"));
            let existing=store.reserve(&proof,&request,6,|_|panic!("a matching receipt must precede a retired lease/capacity check")).unwrap();
            assert!(matches!(existing,Reservation::Existing(known) if known==receipt));
            let mut changed=request.clone();
            if let RequestBody::ShellInput(params)=&mut changed.body {params.subscription=Uuid::random();}
            assert!(matches!(store.reserve(&proof,&changed,7,|_|panic!("conflicts cannot enter a new effect")),Err(StorageError::Policy(ErrorCode::ActionIdConflict))));
        }
    }
    #[test]
    fn queued_creation_denies_only_new_roots_then_narrows_without_inventing_a_native_epoch() {
        use crate::herdr::shells::CreationShellFence;
        let server=Uuid::random();let incarnation=Uuid::random();
        let old=(Id::new("w1:p1").unwrap(),Id::new("old-terminal").unwrap());
        let new=(Id::new("w1:p2").unwrap(),Id::new("new-terminal").unwrap());
        let mut fence=CreationShellFence{server:server.clone(),incarnation:Some(incarnation.clone()),
            baseline:std::collections::HashMap::from([old.clone()]),created:None};
        fence.validate().unwrap();
        assert!(!fence.blocks(&server,&incarnation,&old.0,&old.1));
        assert!(fence.blocks(&server,&incarnation,&new.0,&new.1));
        assert!(fence.blocks(&server,&incarnation,&old.0,&new.1));
        assert!(!fence.blocks(&server,&Uuid::random(),&new.0,&new.1));
        fence.created=Some(new.clone());fence.baseline.clear();
        assert!(fence.blocks(&server,&incarnation,&new.0,&new.1));
        assert!(!fence.blocks(&server,&incarnation,&old.0,&old.1));
        fence.incarnation=None;fence.validate().unwrap();
        assert!(fence.blocks(&server,&Uuid::random(),&new.0,&new.1));
        assert!(!fence.blocks(&Uuid::random(),&incarnation,&new.0,&new.1));
    }
    #[test]
    fn legacy_entered_missing_roots_are_source_scoped_but_owner_only_and_unentered_new_factories_are_not() {
        let (mut store,proof)=owner();let missing=Uuid::random();let manager_only=Uuid::random();
        for (server,stage) in [(&missing,CreationStage::Workspace),(&manager_only,CreationStage::Owner)] {
            let request=Request{id:Uuid::random(),body:RequestBody::SessionCreate(SessionCreateParams{boot_id:store.boot_id.clone(),server_id:server.clone(),
                project_id:Uuid::random(),provider:Provider::Codex,name:Label::new("retained uncertainty").unwrap(),directory_ref:None,placement:None})};
            let Reservation::Reserved(mut receipt)=store.reserve(&proof,&request,1,|_|Ok(())).unwrap() else{panic!()};
            receipt.state=ReceiptState::Uncertain;let creation=receipt.creation.as_mut().unwrap();creation.stage=stage;creation.stage_state=StageState::Uncertain;
            let tx=store.db.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();write_receipt(&tx,&proof.peer.0,&request.id.0,&receipt).unwrap();tx.commit().unwrap();
        }
        let request=Request{id:Uuid::random(),body:RequestBody::TerminalCreate(TerminalCreateParams{boot_id:store.boot_id.clone(),server_id:Uuid::random(),
            workspace_id:Id("w".into()),project_id:Uuid::random(),directory_ref:None,source:None})};
        store.reserve(&proof,&request,1,|_|Ok(())).unwrap();
        store.transition(&proof,&request.id,ReceiptTransition{state:ReceiptState::Uncertain,proof:TransitionProof::LostOutcome,result:None,error:None,at:2}).unwrap();
        let (fences,unproved)=store.creation_shell_fences().unwrap();
        assert!(fences.is_empty());assert_eq!(unproved,std::collections::HashSet::from([missing]));
    }
    #[test]
    fn completed_bootstrap_negative_retires_only_after_observed_agent_or_loss_and_never_erases_uncertainty() {
        use crate::herdr::shells::CreationShellFence;
        let (mut store,proof)=owner();let server=Uuid::random();let incarnation=Uuid::random();let mut actions=Vec::new();
        for (index,(pane,terminal)) in [("p1","n1"),("p2","n2")].into_iter().enumerate() {
            let request=Request{id:Uuid::random(),body:RequestBody::SessionCreate(SessionCreateParams{boot_id:store.boot_id.clone(),server_id:server.clone(),
                project_id:Uuid::random(),provider:Provider::Omp,name:Label::new("stored original facts").unwrap(),directory_ref:None,placement:None})};
            let Reservation::Reserved(mut receipt)=store.reserve(&proof,&request,1,|_|Ok(())).unwrap() else{panic!()};
            store.retain_creation_shell_fence(&proof,&request.id,CreationShellFence{server:server.clone(),incarnation:Some(incarnation.clone()),baseline:Default::default(),created:None}).unwrap();
            let creation=receipt.creation.as_mut().unwrap();
            creation.resources.workspace_id=Some(Id("w".into()));creation.resources.pane_id=Some(Id(pane.into()));creation.resources.terminal_id=Some(Id(terminal.into()));
            if index==0 {
                let target=Target{session_id:Uuid::random(),generation:Uuid::random()};
                creation.resources.native_id=Some(Id("native-original".into()));creation.resources.session_id=Some(target.session_id.clone());creation.resources.generation=Some(target.generation.clone());
                creation.stage=CreationStage::Ready;creation.stage_state=StageState::Confirmed;receipt.state=ReceiptState::Succeeded;
                receipt.result=Some(ActionResult::Created(Box::new(ActionResultCreated{target,workspace_id:Id("w".into()),pane_id:Id(pane.into()),terminal_id:Id(terminal.into()),native_id:Id("native-original".into()),binding_id:None})));
            }else{creation.stage_state=StageState::Uncertain;receipt.state=ReceiptState::Uncertain;}
            // Fixture-only authoritative journal facts; no native provider effect is claimed.
            let tx=store.db.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();write_receipt(&tx,&proof.peer.0,&request.id.0,&receipt).unwrap();tx.commit().unwrap();
            store.narrow_creation_shell_fence(&proof,&request.id,&Id(pane.into()),&Id(terminal.into())).unwrap();
            actions.push((request.id,receipt));
        }
        let mut snapshot:crate::herdr::Snapshot=serde_json::from_value(serde_json::json!({"version":"0.9.1","protocol":22,"focused_workspace_id":"w",
            "workspaces":[{"workspace_id":"w","number":1,"label":"fixture","focused":true,"active_tab_id":"t","agent_status":"idle"}],
            "tabs":[{"tab_id":"t","workspace_id":"w","number":1,"label":"fixture","focused":true,"agent_status":"idle"}],
            "panes":[{"pane_id":"p1","terminal_id":"n1","workspace_id":"w","tab_id":"t","focused":true,"agent_status":"idle"},
                {"pane_id":"p2","terminal_id":"n2","workspace_id":"w","tab_id":"t","focused":false,"agent_status":"idle"}],"agents":[]})).unwrap();
        assert!(store.retire_confirmed_creation_shell_fences(&server,&incarnation,&snapshot).unwrap().is_empty());
        snapshot.panes[0].agent=Some("omp".into());snapshot.panes[1].agent=Some("omp".into());
        assert_eq!(store.retire_confirmed_creation_shell_fences(&server,&incarnation,&snapshot).unwrap(),vec![(proof.peer.clone(),actions[0].0.clone())]);
        let (fences,unproved)=store.creation_shell_fences().unwrap();
        assert!(unproved.is_empty());assert_eq!(fences.len(),1);assert_eq!(fences[0].1,actions[1].0);
        for(action,receipt)in actions {assert_eq!(store.status(&proof,&action,2).unwrap(),Some(receipt));}
    }

    #[test]
    fn retained_agent_bootstraps_never_turn_into_an_ordinary_terminal_after_restart() {
        let (mut store,proof)=owner();let server=Uuid::random();let incarnation=Uuid::random();
        let request=Request{id:Uuid::random(),body:RequestBody::SessionCreate(SessionCreateParams{boot_id:store.boot_id.clone(),server_id:server.clone(),
            project_id:Uuid::random(),provider:Provider::Omp,name:Label::new("owned bootstrap").unwrap(),directory_ref:None,placement:None})};
        let Reservation::Reserved(mut receipt)=store.reserve(&proof,&request,1,|_|Ok(())).unwrap() else{panic!()};
        let creation=receipt.creation.as_mut().unwrap();
        creation.resources.pane_id=Some(Id::new("w2:p1").unwrap());creation.resources.terminal_id=Some(Id::new("bootstrap-terminal").unwrap());
        receipt.state=ReceiptState::Uncertain;
        let stage=CreationStageRecord{stage:CreationStage::Workspace,intent:StageMetadata{socket_epoch:Some(incarnation.clone()),..StageMetadata::default()},
            result:None,state:StageState::Uncertain,proof:None};
        let fence=retained_creation_shell_fence(&receipt,Some(&[stage])).unwrap();
        assert_eq!(fence.incarnation,Some(incarnation.clone()));
        assert!(fence.blocks(&server,&incarnation,&Id::new("w2:p1").unwrap(),&Id::new("bootstrap-terminal").unwrap()));
        assert!(!fence.blocks(&server,&incarnation,&Id::new("w1:p1").unwrap(),&Id::new("ordinary-terminal").unwrap()));
        let legacy=retained_creation_shell_fence(&receipt,None).unwrap();
        assert_eq!(legacy.incarnation,None);
        assert!(legacy.blocks(&server,&Uuid::random(),&Id::new("w2:p1").unwrap(),&Id::new("bootstrap-terminal").unwrap()));
    }
}
