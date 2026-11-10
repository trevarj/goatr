// SPDX-License-Identifier: GPL-3.0-or-later
//! Exact enabled-catalogue documents under independently granted held roots.
use crate::{
    backends::SkillsPublication,
    config::Config,
    herdr::ProcessEvidence,
    pairing::{Authorized, SecurityResult},
    project_read::{Root, Snapshot},
    protocol::*,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct GrantedRoot {
    pathname: PathBuf,
    root: Option<Arc<Root>>,
}
#[derive(Default)]
struct Budget {
    owners: Vec<(Uuid, ProcessEvidence)>,
}
struct Reservation {
    budget: Arc<Mutex<Budget>>,
    peer: Uuid,
    process: ProcessEvidence,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget
            .lock()
            .expect("Skill document budget")
            .owners
            .retain(|(peer, process)| peer != &self.peer || process != &self.process);
    }
}
struct HeldSnapshot {
    data: Snapshot,
    _reservation: Arc<Reservation>,
}
struct ReadLease {
    publication: Arc<SkillsPublication>,
    document: Uuid,
    expires: Instant,
    snapshot: HeldSnapshot,
}
pub(crate) struct Documents {
    roots: Vec<GrantedRoot>,
    budget: Arc<Mutex<Budget>>,
    workers: Arc<tokio::sync::Semaphore>,
    reads: Mutex<HashMap<Uuid, Arc<ReadLease>>>,
}
pub(crate) struct PendingPreview {
    owner: Arc<Documents>,
    id: Uuid,
    read: Arc<ReadLease>,
    published: bool,
}
impl PendingPreview {
    pub(crate) fn published(&mut self) {
        self.published = true;
    }
}
impl Drop for PendingPreview {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let mut reads = self.owner.reads.lock().expect("Skill document reads");
        if reads
            .get(&self.id)
            .is_some_and(|read| Arc::ptr_eq(read, &self.read))
        {
            reads.remove(&self.id);
        }
    }
}
impl Documents {
    pub(crate) fn new(config: &Config) -> Arc<Self> {
        let roots = config
            .skill_document_roots
            .iter()
            .map(|pathname| GrantedRoot {
                pathname: pathname.clone(),
                root: Root::open(pathname, Instant::now() + Duration::from_secs(5))
                    .ok()
                    .map(Arc::new),
            })
            .collect();
        Arc::new(Self {
            roots,
            budget: Arc::new(Mutex::new(Budget::default())),
            workers: Arc::new(tokio::sync::Semaphore::new(4)),
            reads: Mutex::new(HashMap::new()),
        })
    }
    fn reserve(&self, peer: &Uuid, process: &ProcessEvidence) -> SecurityResult<Arc<Reservation>> {
        let mut budget = self.budget.lock().expect("Skill document budget");
        if budget.owners.len() >= 16
            || budget
                .owners
                .iter()
                .any(|(owner, native)| owner == peer || native == process)
        {
            return Err(ErrorCode::ResourceLimit);
        }
        budget.owners.push((peer.clone(), process.clone()));
        Ok(Arc::new(Reservation {
            budget: self.budget.clone(),
            peer: peer.clone(),
            process: process.clone(),
        }))
    }
    fn origin(
        publication: &SkillsPublication,
        auth: &Authorized,
        target: &Target,
        generation: &Uuid,
    ) -> SecurityResult<()> {
        if publication.origin.peer != *auth.proof.peer_id()
            || publication.origin.peer_epoch != auth.proof.epoch()
            || publication.origin.connection != auth.connection_id
        {
            return Err(ErrorCode::Unauthorized);
        }
        if publication.catalogue.target != *target
            || publication.catalogue.catalogue_generation != *generation
        {
            return Err(ErrorCode::StaleSession);
        }
        if Instant::now() >= publication.document_expires {
            return Err(ErrorCode::ContentExpired);
        }
        publication.origin.context.recheck()
    }
    /// Reserve the publication before calling. The real worker/result retains that slot on cancellation.
    pub(crate) async fn prepare(
        self: &Arc<Self>,
        mut publication: SkillsPublication,
        paths: Vec<(usize, PathBuf, Option<Uuid>)>,
    ) -> SecurityResult<SkillsPublication> {
        publication
            .catalogue
            .validate()
            .map_err(|_| ErrorCode::ResourceLimit)?;
        if paths.len() > 128 {
            return Err(ErrorCode::ResourceLimit);
        }
        if self.roots.is_empty() || paths.is_empty() {
            publication.origin.context.recheck()?;
            return Ok(publication);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let permit = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.workers.clone().acquire_owned(),
        )
        .await
        .map_err(|_| ErrorCode::Timeout)?
        .map_err(|_| ErrorCode::NotReady)?;
        let owner = self.clone();
        let worker = tokio::task::spawn_blocking(move || -> SecurityResult<SkillsPublication> {
            let _permit = permit;
            publication.origin.context.recheck()?;
            for (index, path, native_reference) in paths {
                let now = Instant::now();
                if now >= deadline || now >= publication.document_expires {
                    return Err(ErrorCode::Timeout);
                }
                let row = publication
                    .catalogue
                    .skills
                    .get_mut(index)
                    .ok_or(ErrorCode::StaleSession)?;
                if !row.enabled {
                    continue;
                }
                let root = owner
                    .roots
                    .iter()
                    .filter(|root| path.starts_with(&root.pathname))
                    .max_by_key(|root| root.pathname.components().count())
                    .and_then(|root| root.root.as_ref());
                let Some(root) = root else { continue };
                let file = match root.pin_file(&path, deadline) {
                    Ok(file) => Arc::new(file),
                    Err(_) => continue,
                };
                if publication
                    .origin
                    .context
                    .file_visible(file.pathname(), file.provider_object())
                    .is_err()
                {
                    continue;
                }
                if root.revalidate_file(&file, deadline).is_err() {
                    continue;
                }
                let reference = Uuid::random();
                row.document_ref = Some(reference.clone());
                publication.documents.insert(
                    reference,
                    crate::backends::SkillDocument {
                        root: root.clone(),
                        file,
                        native_reference,
                    },
                );
            }
            publication.origin.context.recheck()?;
            publication
                .catalogue
                .validate()
                .map_err(|_| ErrorCode::ResourceLimit)?;
            Ok(publication)
        });
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), worker)
            .await
            .map_err(|_| ErrorCode::Timeout)?
            .map_err(|_| ErrorCode::NotReady)?
    }
    fn validate_document(
        publication: &SkillsPublication,
        document: &Uuid,
        deadline: Instant,
    ) -> SecurityResult<()> {
        if Instant::now() >= publication.document_expires {
            return Err(ErrorCode::ContentExpired);
        }
        publication.origin.context.recheck()?;
        let document = publication
            .documents
            .get(document)
            .ok_or(ErrorCode::NotReady)?;
        document
            .root
            .revalidate_file(&document.file, deadline)
            .map_err(|_| ErrorCode::NotReady)?;
        publication
            .origin
            .context
            .file_visible(document.file.pathname(), document.file.provider_object())?;
        document
            .root
            .revalidate_file(&document.file, deadline)
            .map_err(|_| ErrorCode::NotReady)?;
        publication.origin.context.recheck()
    }
    fn prune(&self) {
        self.reads
            .lock()
            .expect("Skill document reads")
            .retain(|_, read| Instant::now() < read.expires);
    }
    pub(crate) async fn open(
        self: &Arc<Self>,
        auth: &Authorized,
        publication: Arc<SkillsPublication>,
        params: &SkillsPreviewOpenParams,
    ) -> SecurityResult<(SkillsPreviewOpenResult, PendingPreview)> {
        Self::origin(
            &publication,
            auth,
            &params.target,
            &params.catalogue_generation,
        )?;
        if !publication.documents.contains_key(&params.document_ref) {
            return Err(ErrorCode::NotReady);
        }
        self.prune();
        let reservation =
            self.reserve(auth.proof.peer_id(), &publication.origin.context.process)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let expires = publication
            .document_expires
            .min(Instant::now() + Duration::from_secs(120));
        let permit = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.workers.clone().acquire_owned(),
        )
        .await
        .map_err(|_| ErrorCode::Timeout)?
        .map_err(|_| ErrorCode::NotReady)?;
        let document = params.document_ref.clone();
        let worker = tokio::task::spawn_blocking(move || -> SecurityResult<Arc<ReadLease>> {
            let _permit = permit;
            Self::validate_document(&publication, &document, deadline)?;
            let selected = publication
                .documents
                .get(&document)
                .ok_or(ErrorCode::NotReady)?;
            let data = selected
                .root
                .snapshot_file(&selected.file, deadline)
                .map_err(|_| ErrorCode::NotReady)?;
            let text = std::str::from_utf8(data.bytes()).map_err(|_| ErrorCode::NotReady)?;
            if text
                .chars()
                .any(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t'))
            {
                return Err(ErrorCode::NotReady);
            }
            Self::validate_document(&publication, &document, deadline)?;
            if Instant::now() >= expires {
                return Err(ErrorCode::ContentExpired);
            }
            Ok(Arc::new(ReadLease {
                publication,
                document,
                expires,
                snapshot: HeldSnapshot {
                    data,
                    _reservation: reservation,
                },
            }))
        });
        let read = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), worker)
            .await
            .map_err(|_| ErrorCode::Timeout)?
            .map_err(|_| ErrorCode::NotReady)??;
        Self::origin(
            &read.publication,
            auth,
            &params.target,
            &params.catalogue_generation,
        )?;
        if Instant::now() >= read.expires {
            return Err(ErrorCode::ContentExpired);
        }
        let id = Uuid::random();
        let result = SkillsPreviewOpenResult {
            target: params.target.clone(),
            catalogue_generation: params.catalogue_generation.clone(),
            document_ref: params.document_ref.clone(),
            read_id: id.clone(),
            size: Uint(read.snapshot.data.bytes().len() as u64),
            sha256: read.snapshot.data.sha256.clone(),
        };
        self.reads
            .lock()
            .expect("Skill document reads")
            .insert(id.clone(), read.clone());
        Ok((
            result,
            PendingPreview {
                owner: self.clone(),
                id,
                read,
                published: false,
            },
        ))
    }
    fn read_lease(
        &self,
        auth: &Authorized,
        target: &Target,
        generation: &Uuid,
        id: &Uuid,
    ) -> SecurityResult<Arc<ReadLease>> {
        let read = self
            .reads
            .lock()
            .expect("Skill document reads")
            .get(id)
            .cloned()
            .ok_or(ErrorCode::ContentExpired)?;
        Self::origin(&read.publication, auth, target, generation)?;
        if Instant::now() >= read.expires {
            return Err(ErrorCode::ContentExpired);
        }
        Ok(read)
    }
    pub(crate) fn authority(
        &self,
        auth: &Authorized,
        params: &SkillsPreviewReadParams,
    ) -> SecurityResult<(Arc<SkillsPublication>, Uuid)> {
        let read = self.read_lease(
            auth,
            &params.target,
            &params.catalogue_generation,
            &params.read_id,
        )?;
        Ok((read.publication.clone(), read.document.clone()))
    }
    pub(crate) async fn read(
        self: &Arc<Self>,
        auth: &Authorized,
        params: &SkillsPreviewReadParams,
    ) -> SecurityResult<(SkillsPreviewReadResult, PendingPreview)> {
        params.validate().map_err(|_| ErrorCode::InvalidRequest)?;
        let read = self.read_lease(
            auth,
            &params.target,
            &params.catalogue_generation,
            &params.read_id,
        )?;
        let pending = PendingPreview {
            owner: self.clone(),
            id: params.read_id.clone(),
            read: read.clone(),
            published: false,
        };
        let offset = usize::try_from(params.offset.0).map_err(|_| ErrorCode::InvalidRequest)?;
        if offset > read.snapshot.data.bytes().len() {
            return Err(ErrorCode::InvalidRequest);
        }
        let end = read
            .snapshot
            .data
            .bytes()
            .len()
            .min(offset + params.limit.0 as usize);
        let deadline = Instant::now() + Duration::from_secs(5);
        let permit = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.workers.clone().acquire_owned(),
        )
        .await
        .map_err(|_| ErrorCode::Timeout)?
        .map_err(|_| ErrorCode::NotReady)?;
        let worker = tokio::task::spawn_blocking(
            move || -> SecurityResult<(Arc<ReadLease>, String, bool)> {
                let _permit = permit;
                Self::validate_document(&read.publication, &read.document, deadline)?;
                if Instant::now() >= read.expires {
                    return Err(ErrorCode::ContentExpired);
                }
                let data = STANDARD.encode(&read.snapshot.data.bytes()[offset..end]);
                let eof = end == read.snapshot.data.bytes().len();
                Self::validate_document(&read.publication, &read.document, deadline)?;
                Ok((read, data, eof))
            },
        );
        let (read, data, eof) =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), worker)
                .await
                .map_err(|_| ErrorCode::Timeout)?
                .map_err(|_| ErrorCode::NotReady)??;
        Self::origin(
            &read.publication,
            auth,
            &params.target,
            &params.catalogue_generation,
        )?;
        Ok((
            SkillsPreviewReadResult {
                target: params.target.clone(),
                catalogue_generation: params.catalogue_generation.clone(),
                read_id: params.read_id.clone(),
                offset: params.offset,
                data: Base64Bytes(data),
                eof,
            },
            pending,
        ))
    }
    pub(crate) fn publish(
        &self,
        pending: &PendingPreview,
        commit: impl FnOnce() -> SecurityResult<()>,
    ) -> SecurityResult<()> {
        Self::validate_document(
            &pending.read.publication,
            &pending.read.document,
            pending.read.expires,
        )?;
        let reads = self.reads.lock().expect("Skill document reads");
        if Instant::now() >= pending.read.expires
            || !reads
                .get(&pending.id)
                .is_some_and(|read| Arc::ptr_eq(read, &pending.read))
        {
            return Err(ErrorCode::ContentExpired);
        }
        commit()
    }
    pub(crate) fn close(
        &self,
        peer: &Uuid,
        params: &SkillsPreviewCloseParams,
    ) -> SkillsPreviewCloseResult {
        let mut reads = self.reads.lock().expect("Skill document reads");
        let owned = reads.get(&params.read_id).is_some_and(|read| {
            read.publication.origin.peer == *peer
                && read.publication.catalogue.target == params.target
                && read.publication.catalogue.catalogue_generation == params.catalogue_generation
        });
        if owned {
            reads.remove(&params.read_id);
        }
        SkillsPreviewCloseResult { closed: owned }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_reservation_stays_charged_to_peer_and_process_until_actual_last_owner_drops() {
        let config_budget = Arc::new(Mutex::new(Budget::default()));
        let documents = Documents {
            roots: Vec::new(),
            budget: config_budget.clone(),
            workers: Arc::new(tokio::sync::Semaphore::new(4)),
            reads: Mutex::new(HashMap::new()),
        };
        let peer = Uuid::random();
        let process = crate::herdr::process_evidence(std::process::id()).unwrap();
        let admitted = documents.reserve(&peer, &process).unwrap();
        let real_worker = admitted.clone();
        drop(admitted);
        assert!(matches!(
            documents.reserve(&peer, &process),
            Err(ErrorCode::ResourceLimit)
        ));
        assert!(matches!(
            documents.reserve(&Uuid::random(), &process),
            Err(ErrorCode::ResourceLimit)
        ));
        drop(real_worker);
        let next = documents.reserve(&peer, &process).unwrap();
        drop(next);
        assert!(config_budget.lock().unwrap().owners.is_empty());
    }
}
