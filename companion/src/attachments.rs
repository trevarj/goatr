// SPDX-License-Identifier: GPL-3.0-or-later
//! Peer-owned, receipt-backed private staging. Provider paths require original process object proof.
use crate::{
    config::Config,
    herdr::{self, ProcessEvidence, Source},
    pairing::{self, SecurityResult},
    protocol::*,
    storage::{Storage, StorageError, StoreResult},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
};
const FILE_MAX: u64 = 8 * 1024 * 1024;
pub(crate) const IMAGE_ACTIONS_PER_PEER: usize = 4;
pub(crate) const IMAGE_ACTIONS_PER_HOST: usize = 16;
#[derive(Default)]
pub(crate) struct ImageBudget {
    owners: Vec<(Uuid, Uuid)>,
}
impl ImageBudget {
    pub(crate) fn counts(&self, peer: &Uuid) -> (usize, usize) {
        (
            self.owners
                .iter()
                .filter(|(owner, _)| owner == peer)
                .count(),
            self.owners.len(),
        )
    }
}
pub(crate) struct ImageReservation {
    budget: Arc<parking_lot::Mutex<ImageBudget>>,
    peer: Uuid,
    input: Uuid,
}
pub(crate) fn reserve_image_owner(
    budget: &Arc<parking_lot::Mutex<ImageBudget>>,
    facts: &ImageUseFacts,
) -> StoreResult<ImageReservation> {
    let mut held = budget.lock();
    let (peer, host) = held.counts(&facts.peer);
    if peer >= IMAGE_ACTIONS_PER_PEER || host >= IMAGE_ACTIONS_PER_HOST {
        return Err(StorageError::Policy(ErrorCode::ResourceLimit));
    }
    if held.owners.iter().any(|(_, input)| input == &facts.input) {
        return Err(StorageError::Policy(ErrorCode::NotReady));
    }
    held.owners.push((facts.peer.clone(), facts.input.clone()));
    drop(held);
    Ok(ImageReservation {
        budget: budget.clone(),
        peer: facts.peer.clone(),
        input: facts.input.clone(),
    })
}
impl Drop for ImageReservation {
    fn drop(&mut self) {
        self.budget
            .lock()
            .owners
            .retain(|(peer, input)| peer != &self.peer || input != &self.input);
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageUseFacts {
    pub peer: Uuid,
    pub peer_epoch: u64,
    pub connection: Uuid,
    pub action: Uuid,
    pub boot: Uuid,
    pub target: Target,
    pub source: Uuid,
    pub native: Id,
    pub owner: ProcessEvidence,
    pub input: Uuid,
    pub uploads: Vec<Uuid>,
}
impl ImageUseFacts {
    pub(crate) fn validate(&self) -> StoreResult<()> {
        self.peer.validate()?;
        self.connection.validate()?;
        self.action.validate()?;
        self.boot.validate()?;
        self.target.validate()?;
        self.source.validate()?;
        self.native.validate()?;
        self.input.validate()?;
        self.uploads.validate()?;
        self.owner.linux_boot_id.validate()?;
        let mut unique = std::collections::HashSet::new();
        if self.uploads.is_empty()
            || self.uploads.len() > 4
            || !self.uploads.iter().all(|upload| unique.insert(upload))
            || self.owner.pid == 0
            || self.owner.start_ticks == 0
            || self.owner.namespaces.nspids.is_empty()
            || self.owner.namespaces.nspids.len() > 32
        {
            return Err(StorageError::Corrupt);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageUseRecord {
    pub facts: ImageUseFacts,
    pub entered: bool,
}
pub(crate) struct ImageSource {
    pub original: crate::backends::OriginalContext,
    pub attachment: ProviderContext,
}
impl ImageSource {
    pub(crate) fn recheck(&self) -> SecurityResult<()> {
        if self.original.target != self.attachment.target
            || self.original.epoch != self.attachment.epoch
            || self.original.process != self.attachment.namespace.owner
            || !self
                .original
                .source
                .same_physical_identity(&self.attachment.source)
        {
            return Err(ErrorCode::StaleSession);
        }
        self.original.recheck()?;
        self.attachment.recheck()
    }
}
struct ImageFile {
    record: Record,
    directory: File,
    file: File,
    stamp: (u64, i64, i64, i64, i64),
}
fn image_stamp(metadata: &Metadata) -> (u64, i64, i64, i64, i64) {
    (
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}
impl ImageFile {
    fn capture(
        record: Record,
        verify_digest: bool,
        continue_effect: &dyn Fn() -> bool,
    ) -> StoreResult<Self> {
        let (_, directory) = upload_dir(&record)?;
        let mut file = checked_file(&record, &directory, "file", false)?;
        let before = file.metadata()?;
        if before.len() != record.size || record.size == 0 || record.size > FILE_MAX {
            return Err(StorageError::Policy(ErrorCode::NotReady));
        }
        if verify_digest {
            use sha2::Digest;
            let mut hash = sha2::Sha256::new();
            let mut bytes = [0u8; 32768];
            let mut total = 0u64;
            loop {
                if !continue_effect() {
                    return Err(StorageError::Policy(ErrorCode::NotReady));
                }
                let count = file.read(&mut bytes)?;
                if count == 0 {
                    break;
                }
                total = total
                    .checked_add(count as u64)
                    .ok_or(StorageError::Corrupt)?;
                if total > record.size {
                    bytes[..count].fill(0);
                    return Err(StorageError::Corrupt);
                }
                hash.update(&bytes[..count]);
                bytes[..count].fill(0);
            }
            if total != record.size || pairing::hex(&hash.finalize()) != record.digest {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
        }
        let stamp = image_stamp(&before);
        if image_stamp(&file.metadata()?) != stamp {
            return Err(StorageError::Policy(ErrorCode::NotReady));
        }
        Ok(Self {
            record,
            directory,
            file,
            stamp,
        })
    }
    fn recheck(&self) -> SecurityResult<()> {
        let proof = (|| -> std::io::Result<()> {
            if self.record.directory.as_ref() != Some(&object(&self.directory.metadata()?))
                || image_stamp(&self.file.metadata()?) != self.stamp
            {
                return Err(io_error());
            }
            let (_, directory) = upload_dir(&self.record)?;
            let current = checked_file(&self.record, &directory, "file", false)?;
            if image_stamp(&current.metadata()?) != self.stamp {
                return Err(io_error());
            }
            Ok(())
        })();
        proof.map_err(|_| ErrorCode::NotReady)
    }
}
pub(crate) struct ImageReadObligation {
    pub facts: ImageUseFacts,
    pub entered: std::sync::atomic::AtomicBool,
    pub reason: parking_lot::Mutex<ImagePreparationReason>,
    authority: Option<ImageSource>,
    files: Vec<ImageFile>,
    _reservation: ImageReservation,
}
impl ImageReadObligation {
    pub(crate) fn recheck(&self) -> SecurityResult<()> {
        let source = self.authority.as_ref().ok_or(ErrorCode::NotReady)?;
        source.recheck()?;
        for file in &self.files {
            if !self.entered.load(std::sync::atomic::Ordering::Acquire)
                && pairing::now_ms() >= file.record.expires
            {
                return Err(ErrorCode::NotReady);
            }
            file.recheck()?;
            source.attachment.namespace.visible(
                &file.record.provider_path(),
                file.record.file.as_ref().ok_or(ErrorCode::NotReady)?,
            )?;
        }
        source.recheck()
    }
    pub(crate) fn matches(&self, original: &crate::backends::OriginalContext) -> bool {
        self.authority.as_ref().is_some_and(|source| {
            source
                .original
                .source
                .same_physical_identity(&original.source)
        }) && self.facts.target == original.target
            && self.facts.source == original.epoch
            && self.facts.owner == original.process
            && self.facts.native == original.native
    }
    pub(crate) fn native_input(&self) -> &Uuid {
        &self.facts.input
    }
    pub(crate) fn codex_inputs(&self) -> SecurityResult<Vec<serde_json::Value>> {
        self.recheck()?;
        self.files.iter().map(|file|Ok(serde_json::json!({"type":"localImage","path":file.record.provider_path().to_str().ok_or(ErrorCode::NotReady)?}))).collect()
    }
    pub(crate) fn omp_references(&self) -> SecurityResult<Vec<serde_json::Value>> {
        self.recheck()?;
        self.files.iter().map(|file|{
            let object=file.record.file.as_ref().ok_or(ErrorCode::NotReady)?;
            Ok(serde_json::json!({"path":file.record.provider_path().to_str().ok_or(ErrorCode::NotReady)?,"size":file.record.size,"sha256":file.record.digest,"device":object.device.to_string(),"inode":object.inode.to_string()}))
        }).collect()
    }
}
fn policy<T>() -> SecurityResult<T> {
    Err(ErrorCode::NotReady)
}
fn io_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "private attachment object unavailable",
    )
}
fn object(metadata: &Metadata) -> Object {
    Object {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Object {
    device: u64,
    inode: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Namespace {
    pub owner: ProcessEvidence,
    host_root: PathBuf,
    agent_root: PathBuf,
    root: Object,
}
impl Namespace {
    pub(crate) fn capture(config: &Config, owner: &ProcessEvidence) -> SecurityResult<Self> {
        if !config.allow_attachments
            || config
                .runtime_dir_agent
                .to_str()
                .is_none_or(|path| path.len() + 128 > 4096)
        {
            return policy();
        }
        pairing::private_dir(&config.runtime_dir, false).map_err(|_| ErrorCode::NotReady)?;
        let root = object(
            &std::fs::symlink_metadata(&config.runtime_dir).map_err(|_| ErrorCode::NotReady)?,
        );
        let proof = Self {
            owner: owner.clone(),
            host_root: config.runtime_dir.clone(),
            agent_root: config.runtime_dir_agent.clone(),
            root,
        };
        proof.recheck()?;
        Ok(proof)
    }
    fn recheck(&self) -> SecurityResult<()> {
        pairing::private_dir(&self.host_root, false).map_err(|_| ErrorCode::NotReady)?;
        let current =
            object(&std::fs::symlink_metadata(&self.host_root).map_err(|_| ErrorCode::NotReady)?);
        if current != self.root
            || herdr::process_evidence(self.owner.pid).map_err(|_| ErrorCode::StaleSession)?
                != self.owner
            || herdr::process_object(&self.owner, &self.agent_root)
                .map_err(|_| ErrorCode::NotReady)?
                != (self.root.device, self.root.inode)
        {
            return policy();
        }
        Ok(())
    }
    fn visible(&self, path: &Path, expected: &Object) -> SecurityResult<()> {
        self.recheck()?;
        if herdr::process_object(&self.owner, path).map_err(|_| ErrorCode::NotReady)?
            != (expected.device, expected.inode)
        {
            return policy();
        }
        self.recheck()
    }
}
type PublicationGuard =
    Arc<dyn Fn(&mut dyn FnMut() -> SecurityResult<()>) -> SecurityResult<()> + Send + Sync>;
/// Captured from the actual installed provider Connection, never from a serialized owner label.
#[derive(Clone)]
pub(crate) struct ProviderContext {
    pub target: Target,
    pub epoch: Uuid,
    pub source: Source,
    namespace: Namespace,
    validate: Arc<dyn Fn() -> SecurityResult<()> + Send + Sync>,
    publish: PublicationGuard,
}
impl ProviderContext {
    pub(crate) fn capture(
        config: &Config,
        target: Target,
        epoch: Uuid,
        source: Source,
        owner: ProcessEvidence,
        validate: Arc<dyn Fn() -> SecurityResult<()> + Send + Sync>,
        publish: PublicationGuard,
    ) -> SecurityResult<Self> {
        let namespace = Namespace::capture(config, &owner)?;
        let context = Self {
            target,
            epoch,
            source,
            namespace,
            validate,
            publish,
        };
        context.recheck()?;
        Ok(context)
    }
    pub(crate) fn recheck(&self) -> SecurityResult<()> {
        (self.validate)()?;
        self.recheck_namespace()
    }
    fn recheck_namespace(&self) -> SecurityResult<()> {
        self.source.recheck().map_err(|_| ErrorCode::StaleSession)?;
        self.namespace.recheck()
    }
    /// Caller retains authorization; original provider membership stays locked through final proof/enqueue.
    pub(crate) fn enqueue_status(
        &self,
        state: &crate::state::State,
        connection: &Uuid,
        id: Uuid,
        status: AttachmentsStatusResult,
        record: &Record,
    ) -> SecurityResult<()> {
        let mut pending = Some((id, status));
        let result = (self.publish)(&mut || {
            let (id, mut status) = pending.take().ok_or(ErrorCode::NotReady)?;
            if status.insertion_ready && !record.insertion_ready_pinned(self) {
                status.insertion_ready = false
            }
            state.attachments_status_response(
                connection,
                id,
                status,
                Some((&self.target, &self.epoch)),
            )
        });
        if let Some((id, mut status)) = pending {
            status.insertion_ready = false;
            state.attachments_status_response(connection, id, status, None)
        } else {
            result
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Journal {
    action: Uuid,
    offset: u64,
    length: u64,
    digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub peer: Uuid,
    pub upload: Uuid,
    pub target: Target,
    pub source: Uuid,
    namespace: Namespace,
    pub size: u64,
    digest: String,
    pub committed: u64,
    pub expires: u64,
    pub directory: Option<Object>,
    pub file: Option<Object>,
    pub published: bool,
    pub discarded: bool,
    pub tombstone_until: u64,
    pub journal: Option<Journal>,
    pub publishing: bool,
    #[serde(default)]
    pub publish_at: Option<u64>,
}
impl Record {
    pub(crate) fn progress(&self) -> AttachmentProgress {
        AttachmentProgress {
            upload_id: self.upload.clone(),
            committed_bytes: Uint(self.committed),
            expires_at: Some(Uint(self.expires)),
            provider_path: self
                .published
                .then(|| self.provider_path().to_string_lossy().into_owned()),
            published: Some(self.published),
            discarded: Some(self.discarded),
        }
    }
    fn provider_path(&self) -> PathBuf {
        self.namespace
            .agent_root
            .join("attachments")
            .join(&self.peer.0)
            .join(&self.upload.0)
            .join("file")
    }
    pub(crate) fn status(&self) -> AttachmentsStatusResult {
        AttachmentsStatusResult {
            target: self.target.clone(),
            progress: self.progress(),
            insertion_ready: false,
            state: if self.discarded {
                AttachmentUploadState::Discarded
            } else if self.journal.is_some() || self.publishing {
                AttachmentUploadState::Uncertain
            } else if self.published {
                AttachmentUploadState::Complete
            } else {
                AttachmentUploadState::Receiving
            },
            size: Uint(self.size),
            native_preparation: None,
        }
    }
    pub(crate) fn validate(&self) -> StoreResult<()> {
        self.peer.validate()?;
        self.upload.validate()?;
        self.target.validate()?;
        self.source.validate()?;
        self.namespace.owner.linux_boot_id.validate()?;
        if self.namespace.owner.pid == 0
            || self.namespace.owner.start_ticks == 0
            || self.namespace.owner.namespaces.nspids.is_empty()
            || self.namespace.owner.namespaces.nspids.len() > 32
            || self.namespace.root.inode == 0
            || self.published && self.committed != self.size
            || self.discarded && self.tombstone_until == 0
        {
            return Err(StorageError::Corrupt);
        }
        if let Some(journal) = &self.journal {
            journal.action.validate()?;
            if self.published
                || self.publishing
                || journal.offset != self.committed
                || journal.length == 0
                || journal.length > 128 * 1024
                || journal.offset.saturating_add(journal.length) > self.size
                || journal.digest.len() != 64
                || !journal
                    .digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(StorageError::Corrupt);
            }
        }
        crate::config::absolute(&self.namespace.host_root)?;
        crate::config::absolute(&self.namespace.agent_root)?;
        if self.size == 0
            || self.size > FILE_MAX
            || self.committed > self.size
            || self.digest.len() != 64
            || !self
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StorageError::Corrupt);
        }
        Ok(())
    }
}
fn name(value: &str) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(value).map_err(|_| io_error())
}
fn open_at(parent: &File, leaf: &str, flags: i32, mode: u32) -> std::io::Result<File> {
    let leaf = name(leaf)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn directory(parent: &File, leaf: &str, create: bool, exclusive: bool) -> std::io::Result<File> {
    if create {
        let name = name(leaf)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let error = std::io::Error::last_os_error();
            if exclusive || error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        parent.sync_all()?;
    }
    let dir = open_at(parent, leaf, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    let metadata = dir.metadata()?;
    if !metadata.is_dir() || metadata.uid() != pairing::uid() || metadata.mode() & 0o7777 != 0o700 {
        return Err(io_error());
    }
    Ok(dir)
}
fn root(record: &Record) -> std::io::Result<File> {
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&record.namespace.host_root)?;
    let metadata = dir.metadata()?;
    if object(&metadata) != record.namespace.root
        || metadata.uid() != pairing::uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(io_error());
    }
    Ok(dir)
}
fn parents(record: &Record, create: bool) -> std::io::Result<(File, File)> {
    let base = root(record)?;
    let attachments = directory(&base, "attachments", create, false)?;
    let peer = directory(&attachments, &record.peer.0, create, false)?;
    Ok((attachments, peer))
}
fn upload_dir(record: &Record) -> std::io::Result<(File, File)> {
    let (_, peer) = parents(record, false)?;
    let dir = directory(&peer, &record.upload.0, false, false)?;
    if record.directory.as_ref() != Some(&object(&dir.metadata()?)) {
        return Err(io_error());
    }
    Ok((peer, dir))
}
fn checked_file(record: &Record, dir: &File, leaf: &str, write: bool) -> std::io::Result<File> {
    let file = open_at(
        dir,
        leaf,
        if write { libc::O_RDWR } else { libc::O_RDONLY },
        0,
    )?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != pairing::uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
        || record.file.as_ref() != Some(&object(&metadata))
        || metadata.len() > record.size
    {
        return Err(io_error());
    }
    Ok(file)
}
fn unlink(dir: &File, leaf: &str, is_dir: bool) -> std::io::Result<()> {
    let leaf = name(leaf)?;
    if unsafe {
        libc::unlinkat(
            dir.as_raw_fd(),
            leaf.as_ptr(),
            if is_dir { libc::AT_REMOVEDIR } else { 0 },
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    dir.sync_all()
}
fn rename_exclusive(dir: &File) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let part = name("data.part")?;
        let final_name = name("file")?;
        if unsafe {
            libc::renameat2(
                dir.as_raw_fd(),
                part.as_ptr(),
                dir.as_raw_fd(),
                final_name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        dir.sync_all()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        Err(io_error())
    }
}
/// The one SQLite owner also serializes file journaling. No payload enters SQLite or receipt data.
impl Storage {
    pub(crate) fn attachment_status(
        &self,
        peer: &Uuid,
        target: &Target,
        upload: &Uuid,
    ) -> StoreResult<AttachmentsStatusResult> {
        let record = self
            .attachment_load(peer, upload)?
            .ok_or(StorageError::Policy(ErrorCode::NotFound))?;
        if record.target != *target {
            return Err(StorageError::Policy(ErrorCode::StaleSession));
        }
        record.validate()?;
        Ok(record.status())
    }
    pub(crate) fn attachment_effect(
        &mut self,
        peer: &Uuid,
        action: &Uuid,
        body: &RequestBody,
        context: Option<&ProviderContext>,
        continue_effect: &dyn Fn() -> bool,
        retain: &mut dyn FnMut(AttachmentProgress) -> StoreResult<()>,
    ) -> StoreResult<AttachmentProgress> {
        if !continue_effect() {
            return Err(StorageError::Policy(ErrorCode::NotReady));
        }
        let (_, _, target, upload) = body.attachment_action().ok_or(StorageError::Corrupt)?;
        let mut record = if let RequestBody::AttachmentsBegin(params) = body {
            if self.attachment_load(peer, upload)?.is_some() {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            let context = context.ok_or(StorageError::Policy(ErrorCode::NotReady))?;
            context.recheck().map_err(StorageError::Policy)?;
            self.attachment_capacity(peer, params.size.0)?;
            let record = Record {
                peer: peer.clone(),
                upload: upload.clone(),
                target: target.clone(),
                source: context.epoch.clone(),
                namespace: context.namespace.clone(),
                size: params.size.0,
                digest: params.sha256.clone(),
                committed: 0,
                expires: pairing::now_ms() + 60 * 60 * 1000,
                directory: None,
                file: None,
                published: false,
                discarded: false,
                tombstone_until: 0,
                journal: None,
                publishing: false,
                publish_at: None,
            };
            self.attachment_save(&record, true)?;
            retain(record.progress())?;
            if !continue_effect() {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            let (_, parent) = parents(&record, true)?;
            let dir = directory(&parent, &record.upload.0, true, true)?;
            let mut record = record;
            record.directory = Some(object(&dir.metadata()?));
            self.attachment_save(&record, false)?;
            let file = open_at(
                &dir,
                "data.part",
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            file.sync_all()?;
            dir.sync_all()?;
            record.file = Some(object(&file.metadata()?));
            self.attachment_save(&record, false)?;
            retain(record.progress())?;
            record
        } else {
            self.attachment_load(peer, upload)?
                .ok_or(StorageError::Policy(ErrorCode::NotFound))?
        };
        record.validate()?;
        if record.target != *target {
            return Err(StorageError::Policy(ErrorCode::StaleSession));
        }
        // Preserve exact owned prior progress before any source/expiry/file/effect guard can fail.
        retain(record.progress())?;
        if matches!(body, RequestBody::AttachmentsDiscard(_)) {
            if self.image_upload_pinned(peer, upload)? {
                return Err(StorageError::Policy(ErrorCode::Busy));
            }
            if record.discarded {
                record.tombstone_until =
                    pairing::now_ms().saturating_add(crate::storage::RETENTION_MS);
                self.attachment_save(&record, false)?;
                return Ok(record.progress());
            }
            if !continue_effect() {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            remove_owned(&record)?;
            record.discarded = true;
            record.tombstone_until = pairing::now_ms().saturating_add(crate::storage::RETENTION_MS);
            retain(record.progress())?;
            self.attachment_save(&record, false)?;
            return Ok(record.progress());
        }
        let context = context.ok_or(StorageError::Policy(ErrorCode::NotReady))?;
        context.recheck().map_err(StorageError::Policy)?;
        if context.target != record.target
            || context.epoch != record.source
            || context.namespace.owner != record.namespace.owner
            || context.namespace.root != record.namespace.root
            || context.namespace.host_root != record.namespace.host_root
            || context.namespace.agent_root != record.namespace.agent_root
            || record.discarded
            || record.published
            || record.journal.is_some()
            || record.publishing
            || pairing::now_ms() >= record.expires
        {
            return Err(StorageError::Policy(ErrorCode::NotReady));
        }
        let (_, dir) = upload_dir(&record)?;
        if let RequestBody::AttachmentsChunk(params) = body {
            let mut bytes = STANDARD
                .decode(&params.data)
                .map_err(|_| StorageError::Corrupt)?;
            let result = (|| {
                if params.offset.0 != record.committed
                    || record.committed + bytes.len() as u64 > record.size
                {
                    return Err(StorageError::Policy(ErrorCode::InvalidRequest));
                }
                record.journal = Some(Journal {
                    action: action.clone(),
                    offset: record.committed,
                    length: bytes.len() as u64,
                    digest: params.sha256.clone(),
                });
                self.attachment_save(&record, false)?;
                let mut file = checked_file(&record, &dir, "data.part", true)?;
                if file.metadata()?.len() != record.committed {
                    return Err(StorageError::Corrupt);
                }
                if !continue_effect() {
                    return Err(StorageError::Policy(ErrorCode::NotReady));
                }
                file.seek(SeekFrom::Start(record.committed))?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                record.committed += bytes.len() as u64;
                record.journal = None;
                self.attachment_save(&record, false)?;
                retain(record.progress())?;
                Ok(())
            })();
            bytes.fill(0);
            result?;
        } else if matches!(body, RequestBody::AttachmentsComplete(_)) {
            if record.committed != record.size {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            let mut file = checked_file(&record, &dir, "data.part", false)?;
            let mut hash = sha2::Sha256::new();
            let mut buffer = [0u8; 32768];
            let mut size = 0u64;
            use sha2::Digest;
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > record.size {
                    return Err(StorageError::Corrupt);
                }
                hash.update(&buffer[..n]);
                buffer[..n].fill(0)
            }
            if size != record.size || pairing::hex(&hash.finalize()) != record.digest {
                return Err(StorageError::Corrupt);
            }
            let object = record.file.as_ref().ok_or(StorageError::Corrupt)?;
            let part = record.provider_path().with_file_name("data.part");
            context
                .namespace
                .visible(&part, object)
                .map_err(StorageError::Policy)?;
            context.recheck().map_err(StorageError::Policy)?;
            record.publishing = true;
            record.publish_at = Some(pairing::now_ms());
            self.attachment_save(&record, false)?;
            file.sync_all()?;
            if !continue_effect() {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            context.recheck().map_err(StorageError::Policy)?;
            rename_exclusive(&dir)?;
            record.published = true;
            record.expires = record
                .publish_at
                .ok_or(StorageError::Corrupt)?
                .saturating_add(7 * 24 * 60 * 60 * 1000);
            retain(record.progress())?;
            self.attachment_save(&record, false)?;
            context
                .namespace
                .visible(&record.provider_path(), object)
                .map_err(StorageError::Policy)?;
            context.recheck().map_err(StorageError::Policy)?;
            record.publishing = false;
            self.attachment_save(&record, false)?;
        }
        context.recheck().map_err(StorageError::Policy)?;
        retain(record.progress())?;
        Ok(record.progress())
    }
}

/// Only the keyed SQLite owner can obtain the retained original record for peer-owned cleanup.
pub(crate) struct CleanupContext {
    record: Record,
}
impl CleanupContext {
    pub(crate) fn from_owned(
        record: Record,
        peer: &Uuid,
        target: &Target,
        upload: &Uuid,
    ) -> StoreResult<Self> {
        record.validate()?;
        if record.peer != *peer || record.target != *target || record.upload != *upload {
            return Err(StorageError::Policy(ErrorCode::StaleSession));
        }
        Ok(Self { record })
    }
    pub(crate) fn validate(
        &self,
        peer: &Uuid,
        params: &AttachmentsDiscardParams,
    ) -> SecurityResult<()> {
        if self.record.peer != *peer
            || self.record.target != params.target
            || self.record.upload != params.upload_id
        {
            return policy();
        }
        Ok(())
    }
    pub(crate) fn progress(&self) -> AttachmentProgress {
        self.record.progress()
    }
}

fn missing(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}
fn remove_owned(record: &Record) -> std::io::Result<()> {
    let (_, parent) = match parents(record, false) {
        Ok(parents) => parents,
        Err(error) if missing(&error) => {
            root(record)?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let dir = match directory(&parent, &record.upload.0, false, false) {
        Ok(dir) => dir,
        Err(error) if missing(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    if record.directory.as_ref() != Some(&object(&dir.metadata()?)) {
        return Err(io_error());
    }
    if record.file.is_some() {
        let leaves: &[&str] = if record.publishing {
            &["data.part", "file"]
        } else if record.published {
            &["file"]
        } else {
            &["data.part"]
        };
        for leaf in leaves {
            match checked_file(record, &dir, leaf, false) {
                Ok(_file) => unlink(&dir, leaf, false)?,
                Err(error) if missing(&error) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let current = directory(&parent, &record.upload.0, false, false)?;
    if object(&current.metadata()?) != object(&dir.metadata()?) {
        return Err(io_error());
    }
    // rmdir is deliberately non-recursive and refuses any unrecorded content.
    unlink(&parent, &record.upload.0, true)
}
impl Record {
    pub(crate) fn recover(&mut self) -> std::io::Result<()> {
        if self.discarded {
            return Ok(());
        }
        if let Some(journal) = &self.journal {
            if self.published
                || self.publishing
                || journal.offset != self.committed
                || journal.length == 0
                || journal.length > 128 * 1024
            {
                return Err(io_error());
            }
            let (_, dir) = upload_dir(self)?;
            let file = checked_file(self, &dir, "data.part", true)?;
            if file.metadata()?.len() < self.committed
                || file.metadata()?.len() > self.committed.saturating_add(journal.length)
            {
                return Err(io_error());
            }
            file.set_len(self.committed)?;
            file.sync_all()?;
            self.journal = None;
        }
        if self.publishing {
            let (_, dir) = upload_dir(self)?;
            match checked_file(self, &dir, "file", false) {
                Ok(_file) => {
                    match open_at(&dir, "data.part", libc::O_RDONLY, 0) {
                        Err(error) if missing(&error) => {}
                        _ => return Err(io_error()),
                    }
                    self.published = true;
                    self.expires = self
                        .publish_at
                        .ok_or_else(io_error)?
                        .saturating_add(7 * 24 * 60 * 60 * 1000);
                    self.publishing = false;
                }
                Err(error) if missing(&error) => {
                    let _file = checked_file(self, &dir, "data.part", false)?;
                    self.publishing = false;
                    self.publish_at = None;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub(crate) fn expire(&mut self, now: u64) -> std::io::Result<()> {
        if !self.discarded && now >= self.expires {
            remove_owned(self)?;
            self.discarded = true;
            self.tombstone_until = now.saturating_add(crate::storage::RETENTION_MS);
        }
        Ok(())
    }
}

impl Record {
    pub(crate) fn insertion_ready(&self, context: &ProviderContext) -> bool {
        context.recheck().is_ok() && self.insertion_ready_pinned(context)
    }
    fn insertion_ready_pinned(&self, context: &ProviderContext) -> bool {
        if !self.published
            || self.discarded
            || self.publishing
            || self.journal.is_some()
            || pairing::now_ms() >= self.expires
            || self.target != context.target
            || self.source != context.epoch
            || self.namespace.owner != context.namespace.owner
            || self.namespace.root != context.namespace.root
        {
            return false;
        }
        let proof = (|| -> SecurityResult<()> {
            context.recheck_namespace()?;
            let (_, dir) = upload_dir(self).map_err(|_| ErrorCode::NotReady)?;
            let file = checked_file(self, &dir, "file", false).map_err(|_| ErrorCode::NotReady)?;
            if file.metadata().map_err(|_| ErrorCode::NotReady)?.len() != self.size {
                return policy();
            }
            context.namespace.visible(
                &self.provider_path(),
                self.file.as_ref().ok_or(ErrorCode::NotReady)?,
            )?;
            context.recheck_namespace()
        })();
        proof.is_ok()
    }
}

impl Record {
    pub(crate) fn matches_image_source(&self, facts: &ImageUseFacts) -> bool {
        self.peer == facts.peer
            && self.target == facts.target
            && self.source == facts.source
            && self.namespace.owner == facts.owner
            && facts.uploads.contains(&self.upload)
    }
}
impl Storage {
    pub(crate) fn prepare_image_read(
        &mut self,
        proof: &crate::storage::AuthenticatedPeer,
        facts: ImageUseFacts,
        source: ImageSource,
        continue_effect: &dyn Fn() -> bool,
    ) -> StoreResult<Arc<ImageReadObligation>> {
        facts.validate()?;
        if !self.ensure_image_reads() {
            return Err(StorageError::Policy(ErrorCode::NotReady));
        }
        let reservation = self.reserve_image_capacity(&facts)?;
        source.recheck().map_err(StorageError::Policy)?;
        if facts.peer != *proof.peer_id()
            || facts.peer_epoch != proof.epoch()
            || facts.target != source.original.target
            || facts.source != source.original.epoch
            || facts.native != source.original.native
            || facts.owner != source.original.process
        {
            return Err(StorageError::Policy(ErrorCode::StaleSession));
        }
        let mut files = Vec::with_capacity(facts.uploads.len());
        let mut total = 0u64;
        for upload in &facts.uploads {
            if !continue_effect() {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            let record = self
                .attachment_load(&facts.peer, upload)?
                .ok_or(StorageError::Policy(ErrorCode::NotFound))?;
            if !record.matches_image_source(&facts)
                || !record.published
                || record.discarded
                || record.publishing
                || record.journal.is_some()
                || record.committed != record.size
                || pairing::now_ms() >= record.expires
            {
                return Err(StorageError::Policy(ErrorCode::NotReady));
            }
            total = total
                .checked_add(record.size)
                .ok_or(StorageError::Policy(ErrorCode::ResourceLimit))?;
            if total > FILE_MAX {
                return Err(StorageError::Policy(ErrorCode::ResourceLimit));
            }
            let file = ImageFile::capture(
                record,
                source.original.provider == Provider::Codex,
                continue_effect,
            )?;
            source
                .attachment
                .namespace
                .visible(
                    &file.record.provider_path(),
                    file.record.file.as_ref().ok_or(StorageError::Corrupt)?,
                )
                .map_err(StorageError::Policy)?;
            files.push(file);
        }
        source.recheck().map_err(StorageError::Policy)?;
        if !continue_effect() {
            return Err(StorageError::Policy(ErrorCode::NotReady));
        }
        self.image_use_reserve(proof, &facts)?;
        let owner = Arc::new(ImageReadObligation {
            facts,
            entered: std::sync::atomic::AtomicBool::new(false),
            reason: parking_lot::Mutex::new(ImagePreparationReason::PendingOriginalRead),
            authority: Some(source),
            files,
            _reservation: reservation,
        });
        self.hold_image_read(owner.clone())?;
        Ok(owner)
    }
    pub(crate) fn restore_image_read(
        &self,
        facts: ImageUseFacts,
    ) -> StoreResult<Arc<ImageReadObligation>> {
        facts.validate()?;
        let mut total = 0u64;
        let mut files = Vec::with_capacity(facts.uploads.len());
        let reservation = self.reserve_image_capacity(&facts)?;
        for upload in &facts.uploads {
            let record = self
                .attachment_load(&facts.peer, upload)?
                .ok_or(StorageError::Corrupt)?;
            if !record.matches_image_source(&facts)
                || !record.published
                || record.discarded
                || record.publishing
                || record.journal.is_some()
                || record.committed != record.size
            {
                return Err(StorageError::Corrupt);
            }
            total = total
                .checked_add(record.size)
                .ok_or(StorageError::Corrupt)?;
            if total > FILE_MAX {
                return Err(StorageError::Corrupt);
            }
            // Restore only private host object/negative-hold facts; no native
            // process, channel, Source epoch or mutation authority is recreated.
            files.push(ImageFile::capture(record, true, &|| true)?);
        }
        Ok(Arc::new(ImageReadObligation {
            facts,
            entered: std::sync::atomic::AtomicBool::new(true),
            reason: parking_lot::Mutex::new(ImagePreparationReason::OriginalSourceUnavailable),
            authority: None,
            files,
            _reservation: reservation,
        }))
    }
}

#[cfg(test)]
mod image_owner_tests {
    use super::*;
    fn facts(peer: Uuid) -> ImageUseFacts {
        ImageUseFacts {
            peer,
            peer_epoch: 1,
            connection: Uuid::random(),
            action: Uuid::random(),
            boot: Uuid::random(),
            target: Target {
                session_id: Uuid::random(),
                generation: Uuid::random(),
            },
            source: Uuid::random(),
            native: Id("original-thread".into()),
            owner: herdr::process_evidence(std::process::id()).unwrap(),
            input: Uuid::random(),
            uploads: vec![Uuid::random()],
        }
    }
    #[test]
    fn image_slots_stay_charged_until_actual_last_owner_and_enforce_peer_and_host_caps() {
        let budget = Arc::new(parking_lot::Mutex::new(ImageBudget::default()));
        let peer = Uuid::random();
        let mut owners = Vec::new();
        for _ in 0..IMAGE_ACTIONS_PER_PEER {
            owners.push(Arc::new(
                reserve_image_owner(&budget, &facts(peer.clone())).unwrap(),
            ));
        }
        let actual_worker = owners[0].clone();
        drop(owners.remove(0));
        assert!(matches!(
            reserve_image_owner(&budget, &facts(peer.clone())),
            Err(StorageError::Policy(ErrorCode::ResourceLimit))
        ));
        drop(owners);
        assert_eq!(budget.lock().counts(&peer), (1, 1));
        drop(actual_worker);
        assert_eq!(budget.lock().counts(&peer), (0, 0));
        let mut host_owners = Vec::new();
        for _ in 0..IMAGE_ACTIONS_PER_HOST / IMAGE_ACTIONS_PER_PEER {
            let peer = Uuid::random();
            for _ in 0..IMAGE_ACTIONS_PER_PEER {
                host_owners.push(reserve_image_owner(&budget, &facts(peer.clone())).unwrap());
            }
        }
        assert!(matches!(
            reserve_image_owner(&budget, &facts(Uuid::random())),
            Err(StorageError::Policy(ErrorCode::ResourceLimit))
        ));
        drop(host_owners);
        assert_eq!(budget.lock().counts(&peer), (0, 0));
    }
}
