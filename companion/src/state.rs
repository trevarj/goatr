// SPDX-License-Identifier: GPL-3.0-or-later
//! Serialized snapshot/event handoff. One owner survives phone connections; adapters supply
//! already-redacted, completed display records, never pending originals or secret answers.
use crate::protocol::*;
use crate::storage::AuthenticatedPeer;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, atomic::AtomicUsize},
};

pub const HISTORY_RECORDS: usize = 1_000;
pub const SESSION_BYTES: usize = 64 * 1024 * 1024;
pub const CONTENT_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub const CONTENT_TTL_MS: u64 = 15 * 60 * 1000;
pub const OUTPUT_EVENTS: usize = 256;
pub const OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub type StateResult<T> = Result<T, ErrorCode>;
fn valid<T: WireValidate>(x: &T) -> StateResult<()> {
    x.validate().map_err(|_| ErrorCode::InvalidRequest)
}
fn frame(x: &Envelope) -> StateResult<Vec<u8>> {
    encode_payload(x).map_err(|_| ErrorCode::ResourceLimit)
}

/// This is a trust-boundary acknowledgement, not a universal secret scanner. The adapter
/// must remove sensitive native fields/capability links before constructing this value.
#[derive(Clone)]
pub struct SafeHistory(HistoryRecord);
impl SafeHistory {
    pub fn from_redacted_display(record: HistoryRecord) -> StateResult<Self> {
        valid(&record)?;
        if !record.complete {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(Self(record))
    }
    pub fn record(&self) -> &HistoryRecord {
        &self.0
    }
}

#[derive(Clone)]
pub struct ProviderSnapshot {
    pub state: SessionState,
    pub pending: Vec<PendingRequest>,
    pub history: Vec<SafeHistory>,
    /// True if a native refetch can recover material not retained here.
    pub older_available: bool,
    /// Actual provider continuation from the installed snapshot, never a guessed cursor.
    pub native_older: Option<Id>,
}
#[derive(Clone)]
pub enum SessionUpdate {
    State(SessionState),
    History(Vec<SafeHistory>),
    Activity(Activity),
    Open(PendingRequest),
    Close(Uuid, RequestClosedReason),
    SourceGap,
}
/// The byte count belongs to this exact immutable view, including cursor-pinned copies.
#[derive(Clone, Default)]
struct HistorySnapshot {
    records: Vec<HistoryRecord>,
    bytes: usize,
}
impl HistorySnapshot {
    fn new(records: Vec<HistoryRecord>) -> Self {
        let bytes = records.iter().map(record_bytes).sum();
        Self { records, bytes }
    }
    fn push(&mut self, record: HistoryRecord) {
        self.bytes += record_bytes(&record);
        self.records.push(record);
    }
    fn replace(&mut self, index: usize, record: HistoryRecord) {
        self.bytes = self.bytes - record_bytes(&self.records[index]) + record_bytes(&record);
        self.records[index] = record;
    }
    fn sort(&mut self) {
        self.records.sort_by_key(|r| r.order);
    }
    fn drain_prefix(&mut self, count: usize) -> std::vec::Drain<'_, HistoryRecord> {
        self.bytes -= self.records[..count]
            .iter()
            .map(record_bytes)
            .sum::<usize>();
        self.records.drain(..count)
    }
}
impl std::ops::Deref for HistorySnapshot {
    type Target = [HistoryRecord];
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}
struct Session {
    state: SessionState,
    source: Uuid,
    history: Arc<HistorySnapshot>,
    origins: HashMap<Id, HistoryOrigins>,
    pending: Vec<PendingRequest>,
    older: bool,
    native_older: Option<Id>,
    installing: bool,
    install_failed: bool,
    topology_dirty: bool,
    observation_pending: bool,
    native_queue: VecDeque<SessionUpdate>,
    native_bytes: usize,
}
impl Session {
    fn target(&self) -> Target {
        Target {
            session_id: self.state.summary.session_id.clone(),
            generation: self.state.summary.generation.clone(),
        }
    }
    fn trim(&mut self) {
        let mut bytes = self.history.bytes;
        let mut evicted = 0;
        for record in self.history.iter() {
            if self.history.len() - evicted <= HISTORY_RECORDS && bytes <= SESSION_BYTES {
                break;
            }
            bytes -= record_bytes(record);
            evicted += 1;
        }
        if evicted > 0 {
            for removed in Arc::make_mut(&mut self.history).drain_prefix(evicted) {
                self.origins.remove(&removed.id);
            }
            self.older = true;
            self.native_older = None;
        }
        self.state.summary.history_coverage.older_available |= self.older;
    }
}
fn record_bytes(r: &HistoryRecord) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, r).expect("validated display record");
    count.0
}
#[derive(Default, Clone)]
pub struct HistoryOrigins {
    persisted_text: bool,
    persisted_original: bool,
    live_tool: bool,
}
impl HistoryOrigins {
    pub fn observe(&mut self, record: &HistoryRecord) {
        self.persisted_text |= record.source == RecordSource::Persisted && record.text.is_some();
        self.persisted_original |=
            record.source == RecordSource::Persisted && record.original.is_some();
        self.live_tool |= record.source == RecordSource::Live && record.tool.is_some();
    }
    pub fn from_record(record: &HistoryRecord) -> Self {
        let mut origins = Self::default();
        origins.observe(record);
        origins
    }
}
/// Provider merge policy is shared by the host and phone reducers.
pub fn merge_history(
    provider: Provider,
    old: &HistoryRecord,
    mut incoming: HistoryRecord,
    origins: &mut HistoryOrigins,
    historical: bool,
) -> HistoryRecord {
    incoming.order = old.order;
    if provider == Provider::Codex
        && old.source == RecordSource::Live
        && (historical || incoming.source != RecordSource::Live)
    {
        let mut merged = old.clone();
        if merged.text.is_none() {
            merged.text = incoming.text;
        }
        if merged.original.is_none() {
            merged.original = incoming.original;
        }
        if merged.parent_id.is_none() {
            merged.parent_id = incoming.parent_id;
        }
        if merged.turn_id.is_none() {
            merged.turn_id = incoming.turn_id;
        }
        if merged.timestamp.is_none() {
            merged.timestamp = incoming.timestamp;
        }
        if merged.tool.is_none() {
            merged.tool = incoming.tool;
        } else if let (Some(live), Some(page)) = (&mut merged.tool, incoming.tool) {
            if live.call_id.is_none() {
                live.call_id = page.call_id;
            }
            if live.input.is_none() {
                live.input = page.input;
            }
            if live.output.is_none() {
                live.output = page.output;
            }
        }
        return merged;
    }
    if incoming.text.is_none() {
        incoming.text = old.text.clone();
    }
    if incoming.original.is_none() {
        incoming.original = old.original.clone();
    }
    if incoming.parent_id.is_none() {
        incoming.parent_id = old.parent_id.clone();
    }
    if incoming.turn_id.is_none() {
        incoming.turn_id = old.turn_id.clone();
    }
    if incoming.timestamp.is_none() {
        incoming.timestamp = old.timestamp;
    }
    if incoming.tool.is_none() {
        incoming.tool = old.tool.clone();
    } else if let (Some(tool), Some(older)) = (&mut incoming.tool, &old.tool) {
        if tool.call_id.is_none() {
            tool.call_id = older.call_id.clone();
        }
        if tool.input.is_none() {
            tool.input = older.input.clone();
        }
        if tool.output.is_none() {
            tool.output = older.output.clone();
        }
    }
    if provider == Provider::Omp {
        let source = incoming.source;
        if (origins.persisted_text && (historical || source == RecordSource::Live))
            || (historical && old.source == RecordSource::Live && source == RecordSource::Live)
        {
            incoming.text = old.text.clone();
        }
        if origins.persisted_original && (historical || source == RecordSource::Live) {
            incoming.original = old.original.clone();
        }
        if (historical || (origins.live_tool && source == RecordSource::Persisted))
            && let Some(live) = &old.tool
        {
            let mut tool = incoming.tool.unwrap_or_else(|| live.clone());
            tool.status = live.status;
            if live.output.is_some() {
                tool.output = live.output.clone();
            }
            incoming.tool = Some(tool);
        }
    }
    origins.observe(&incoming);
    incoming
}
enum SubscriptionKind {
    Host,
    Session(Uuid),
    Grid(Arc<crate::herdr::grid::GridLease>),
}
struct Subscription {
    kind: SubscriptionKind,
    seq: u64,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let SubscriptionKind::Grid(lease) = &self.kind {
            lease.cancel();
        }
    }
}
pub(crate) struct Queued {
    pub bytes: Vec<u8>,
    event: bool,
    pub grid: Option<(
        Arc<crate::herdr::grid::GridLease>,
        tokio::sync::oneshot::Sender<()>,
        tokio::time::Instant,
    )>,
}
struct Connection {
    peer: Uuid,
    epoch: u64,
    skills: Option<Arc<crate::backends::SkillsPublication>>,
    controls_catalogue:Option<Arc<crate::backends::ControlsPublication>>,
    queue_view:Option<Arc<crate::backends::QueuePublication>>,
    subscriptions: HashMap<Uuid, Subscription>,
    queue: VecDeque<Queued>,
    bytes: usize,
    events: usize,
    closed: bool,
    transient: Option<(Uuid, Activity)>,
}
impl Connection {
    fn enqueue(&mut self, bytes: Vec<u8>, event: bool) -> bool {
        if self.closed
            || self.bytes + bytes.len() + 4 > OUTPUT_BYTES
            || (event && self.events == OUTPUT_EVENTS)
        {
            return false;
        }
        self.bytes += bytes.len() + 4;
        self.events += usize::from(event);
        self.queue.push_back(Queued {
            bytes,
            event,
            grid: None,
        });
        true
    }
    fn emit(&mut self, subscription: &Uuid, body: EventBody) {
        if self.closed {
            return;
        }
        let Some(sub) = self.subscriptions.get_mut(subscription) else {
            return;
        };
        let Some(seq) = sub.seq.checked_add(1).filter(|n| *n <= UINT_MAX) else {
            self.closed = true;
            return;
        };
        let bytes = frame(&Envelope::Event(Event {
            subscription: subscription.clone(),
            seq: Uint(seq),
            body,
        }));
        if bytes.is_ok_and(|bytes| self.enqueue(bytes, true)) {
            self.subscriptions
                .get_mut(subscription)
                .expect("subscription exists")
                .seq = seq;
        } else {
            // Never remove prior frames to make room: the recovery frame also has to fit in order.
            if let Ok(bytes) = frame(&Envelope::Event(Event {
                subscription: subscription.clone(),
                seq: Uint(seq),
                body: EventBody::SyncRequired(SyncRequiredEvent {
                    reason: SyncReason::Overflow,
                }),
            })) {
                self.enqueue(bytes, true);
            }
            self.closed = true;
        }
    }
    fn flush(&mut self) {
        if let Some((id, activity)) = self.transient.take() {
            self.emit(
                &id,
                EventBody::ActivityUpdate(ActivityUpdateEvent { activity }),
            );
        }
    }
}
fn grid_queue(
    c: &mut Connection,
    payload: Vec<u8>,
    event: bool,
    lease: Arc<crate::herdr::grid::GridLease>,
    deadline: tokio::time::Instant,
    bytes: &mut usize,
) -> StateResult<tokio::sync::oneshot::Receiver<()>> {
    crate::herdr::grid::before(deadline)?;
    if !lease.active()
        || c.queue.iter().any(|q| {
            q.grid
                .as_ref()
                .is_some_and(|(old, _, _)| old.id == lease.id)
        })
    {
        return Err(ErrorCode::StaleSession);
    }
    *bytes = bytes
        .checked_add(payload.len())
        .ok_or(ErrorCode::ResourceLimit)?;
    if *bytes > 8 * 1024 * 1024 {
        return Err(ErrorCode::ResourceLimit);
    }
    crate::herdr::grid::before(deadline)?;
    if !c.enqueue(payload, event) {
        c.closed = true;
        return Err(ErrorCode::ResourceLimit);
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    c.queue.back_mut().expect("enqueued grid").grid = Some((lease, send, deadline));
    Ok(receive)
}
fn close_grid(c: &mut Connection, id: &Uuid) {
    if c.subscriptions.remove(id).is_some() {
        // Failure removes/cancels the exact lease AND explicitly closes the shared connection.
        c.queue.clear();
        c.transient = None;
        c.bytes = 0;
        c.events = 0;
        c.closed = true;
    }
}
fn cancel_grids(i: &mut Inner, affected: impl Fn(&crate::herdr::grid::GridLease) -> bool) {
    for c in i.connections.values_mut() {
        let id = c.subscriptions.iter().find_map(|(id, s)| match &s.kind {
            SubscriptionKind::Grid(lease) if affected(lease) => Some(id.clone()),
            _ => None,
        });
        if let Some(id) = id {
            close_grid(c, &id);
        }
    }
}
struct Payload {
    target: Target,
    source: Uuid,
    data: Vec<u8>,
    expires: u64,
    used: u64,
}
struct Cursor {
    peer: Uuid,
    target: Target,
    source: Uuid,
    records: Arc<HistorySnapshot>,
    end: usize,
    expires: u64,
    native_older: Option<Id>,
}
struct NativeCursor {
    peer: Uuid,
    target: Target,
    source: Uuid,
    native: Id,
    expires: u64,
    pages: HashMap<usize, SessionHistoryResult>,
    page_bytes: usize,
}
pub struct NativePageTicket {
    peer: Uuid,
    target: Target,
    source: Uuid,
    token: Uuid,
    limit: usize,
    native: Id,
}
impl NativePageTicket {
    pub fn target(&self) -> &Target {
        &self.target
    }
    pub fn native_cursor(&self) -> &Id {
        &self.native
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
}
pub enum NativePageLookup {
    Cached(SessionHistoryResult),
    Fetch(NativePageTicket),
}
#[derive(Default)]
struct Inner {
    browse: HashMap<Uuid, browse::BrowseNode>,
    adapter_history_bytes: HashMap<Uuid, (Uuid, usize)>,
    sessions: HashMap<Uuid, Session>,
    shells: HashMap<Uuid,(ShellSummary,Uuid)>,
    creation_shell_fences:HashMap<(Uuid,Uuid),crate::herdr::shells::CreationShellFence>,
    connections: HashMap<Uuid, Connection>,
    skills_publications: Option<Arc<AtomicUsize>>,
    controls_publications:Option<Arc<AtomicUsize>>,
    queue_publications:Option<Arc<AtomicUsize>>,
    servers: Vec<Server>,
    projects: Vec<Project>,
    unbound: Vec<UnboundRow>,
    hierarchy: Vec<NativeHierarchy>,
    inventory_sessions: Option<Vec<SessionSummary>>,
    content: HashMap<Uuid, Payload>,
    cursors: HashMap<Uuid, Cursor>,
    native_cursors: HashMap<Uuid, NativeCursor>,
    clock: u64,
}
/// ponytail: a single short mutex makes cross-reducer/output handoff atomic; split by
/// session only if measured throughput requires it. Never perform provider I/O under it.
#[derive(Default)]
pub struct State {
    inner: Mutex<Inner>,
    pub(crate) creation_directories:Arc<crate::herdr::creation_directories::Leases>,
}
#[path = "state_browse.rs"]
mod browse;
pub use browse::{BrowseLookup, BrowsePosition, BrowseTicket};

fn inventory_sessions(i: &Inner) -> Vec<SessionSummary> {
    let mut sessions: Vec<_> = i
        .sessions
        .values()
        .map(|session| session.state.summary.clone())
        .collect();
    sessions.sort_by(|left, right| left.session_id.0.cmp(&right.session_id.0));
    sessions
}

fn creation_context_valid(i:&Inner,context:&crate::herdr::creation_directories::DirectoryContext,publication:bool)->StateResult<()> {
    if std::time::Instant::now()>=context.expires{return Err(ErrorCode::CursorExpired)}
    let connection=i.connections.get(&context.connection).ok_or(ErrorCode::Unauthorized)?;
    if connection.closed||connection.peer!=context.peer||connection.epoch!=context.peer_epoch{return Err(ErrorCode::Unauthorized)}
    let hierarchy=i.hierarchy.iter().find(|hierarchy|hierarchy.server_id==context.physical.server_id
        &&hierarchy.server_incarnation==context.physical.incarnation
        &&if publication {hierarchy.freshness==NativeFreshness::Current}else{hierarchy.freshness!=NativeFreshness::Unavailable}).ok_or(ErrorCode::NotReady)?;
    let published_routes=publication||hierarchy.freshness==NativeFreshness::Current;
    if let Some(origin)=&context.origin {
        let pane=hierarchy.panes.iter().find(|pane|pane.pane_id==origin.pane
            &&pane.terminal_id==origin.terminal&&pane.workspace_id==origin.workspace&&pane.tab_id==origin.tab).ok_or(ErrorCode::StaleSession)?;
        match &origin.source {
            ArtifactSource::Session{target}=>{
                let session=current(i,target)?;
                if session.source!=origin.epoch||session.installing||session.install_failed
                    ||!(session.state.summary.connectivity==SessionConnectivity::Ready
                        ||(!publication&&session.state.summary.connectivity==SessionConnectivity::Synchronizing))
                    ||session.state.summary.server_id!=context.physical.server_id
                    ||session.state.summary.pane_id!=origin.pane||session.state.summary.terminal_id!=origin.terminal
                    ||session.state.summary.workspace_id!=origin.workspace||(published_routes&&pane.target.as_ref()!=Some(target)) {
                    return Err(ErrorCode::StaleSession);
                }
            }
            ArtifactSource::Shell{target}=>{
                let (shell,epoch)=i.shells.get(&target.shell_id).ok_or(ErrorCode::StaleSession)?;
                if &shell.target!=target||epoch!=&origin.epoch
                    ||!(shell.connectivity==SessionConnectivity::Ready||(!publication&&shell.connectivity==SessionConnectivity::Synchronizing))
                    ||shell.server_id!=context.physical.server_id||shell.pane_id!=origin.pane
                    ||shell.terminal_id!=origin.terminal||shell.workspace_id!=origin.workspace
                    ||shell.tab_id!=origin.tab||(published_routes&&pane.shell_target.as_ref()!=Some(target)) {
                    return Err(ErrorCode::StaleSession);
                }
            }
        }
    }
    Ok(())
}

impl State {
    pub fn connect(&self, peer: &AuthenticatedPeer) -> Uuid {
        self.creation_directories.release_peer(peer.peer_id());
        let mut inner = self.inner.lock().expect("state mutex");
        inner.connections.retain(|_, c| c.peer != *peer.peer_id());
        let id = Uuid::random();
        inner.connections.insert(
            id.clone(),
            Connection {
                peer: peer.peer_id().clone(),
                epoch: peer.epoch(),
                skills: None,
                controls_catalogue:None,
                queue_view:None,
                subscriptions: HashMap::new(),
                queue: VecDeque::new(),
                bytes: 0,
                events: 0,
                closed: false,
                transient: None,
            },
        );
        id
    }
    pub fn disconnect(&self, connection: &Uuid) {
        self.creation_directories.release_connection(connection);
        let mut i=self.inner.lock().expect("state mutex");
        if let Some(connection)=i.connections.remove(connection)
            &&let Some(publication)=connection.queue_view {
            for payload in &publication.payloads {i.content.remove(payload);}
        }
    }
    pub fn revoke(&self, peer: &Uuid) {
        self.creation_directories.release_peer(peer);
        let mut i = self.inner.lock().expect("state mutex");
        {
            let Inner {connections,content,..}=&mut *i;
            for connection in connections.values_mut().filter(|connection|&connection.peer==peer) {
                if let Some(publication)=connection.queue_view.take() {
                    for payload in &publication.payloads {content.remove(payload);}
                }
            }
        }
        i.connections.retain(|_, c| &c.peer != peer);
        i.cursors.retain(|_, c| &c.peer != peer);
        i.native_cursors.retain(|_, c| &c.peer != peer);
        i.browse.retain(|_, c| &c.peer != peer);
    }
    pub(crate) fn reserve_publication(
        &self,auth:&crate::pairing::Authorized,kind:crate::backends::PublicationKind,
    )->StateResult<Arc<crate::backends::PublicationSlot>> {
        let mut i=self.inner.lock().expect("state mutex");
        prune_publications(&mut i,std::time::Instant::now());
        let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||&c.peer!=auth.proof.peer_id()||c.epoch!=auth.proof.epoch() {return Err(ErrorCode::Unauthorized)}
        let counter=match kind {
            crate::backends::PublicationKind::Skills=>&mut i.skills_publications,
            crate::backends::PublicationKind::Controls=>&mut i.controls_publications,
            crate::backends::PublicationKind::Queue=>&mut i.queue_publications,
        }.get_or_insert_with(||Arc::new(AtomicUsize::new(0))).clone();
        crate::backends::PublicationSlot::reserve(counter,32)
    }
    pub(crate) fn clear_skills_publication(&self, connection:&Uuid) {
        if let Some(c)=self.inner.lock().expect("state mutex").connections.get_mut(connection) {c.skills=None;}
    }
    pub(crate) fn clear_peer_skills(&self, peer:&Uuid) {
        for c in self.inner.lock().expect("state mutex").connections.values_mut().filter(|c|&c.peer==peer) {c.skills=None;}
    }
    pub(crate) fn skills_publication(
        &self,auth:&crate::pairing::Authorized,target:&Target,generation:&Uuid,
    )->StateResult<Arc<crate::backends::SkillsPublication>> {
        let mut i=self.inner.lock().expect("state mutex");prune_publications(&mut i,std::time::Instant::now());
        let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||&c.peer!=auth.proof.peer_id()||c.epoch!=auth.proof.epoch() {return Err(ErrorCode::Unauthorized)}
        let publication=c.skills.as_ref().ok_or(ErrorCode::NotReady)?;
        if &publication.catalogue.target!=target||&publication.catalogue.catalogue_generation!=generation {return Err(ErrorCode::StaleSession)}
        Ok(publication.clone())
    }
    pub(crate) fn skills_publication_current(&self,auth:&crate::pairing::Authorized,publication:&Arc<crate::backends::SkillsPublication>)->StateResult<()> {
        let i=self.inner.lock().expect("state mutex");
        let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed || &c.peer!=auth.proof.peer_id() || c.epoch!=auth.proof.epoch() || publication.origin.peer!=c.peer || publication.origin.peer_epoch!=c.epoch || publication.origin.connection!=auth.connection_id {return Err(ErrorCode::Unauthorized)}
        if !c.skills.as_ref().is_some_and(|p|Arc::ptr_eq(p,publication)){return Err(ErrorCode::StaleSession)}
        Ok(())
    }
    /// Captured provider epochs fence pane associations independently of the Herdr revision.
    pub fn replace_inventory(
        &self,
        servers: Vec<Server>,
        projects: Vec<Project>,
        shells: Vec<(ShellSummary,Uuid)>,
        mut unbound: Vec<UnboundRow>,
        mut hierarchy: Vec<NativeHierarchy>,
        association_epochs: &[(&Target, &Uuid)],
        validate: impl FnOnce(&mut [NativeHierarchy]) -> StateResult<()>,
    ) -> StateResult<()> {
        valid(&servers)?;
        valid(&projects)?;
        let mut shell_map=HashMap::new();
        for (summary,source) in shells {valid(&summary)?;valid(&source)?;if shell_map.insert(summary.target.shell_id.clone(),(summary,source)).is_some(){return Err(ErrorCode::InvalidRequest)}}
        valid(&unbound)?;
        valid(&hierarchy)?;
        if association_epochs.len() > 512 {
            return Err(ErrorCode::ResourceLimit);
        }
        let mut epochs = HashMap::with_capacity(association_epochs.len());
        for &(target, source) in association_epochs {
            valid(target)?;
            valid(source)?;
            if epochs
                .insert(&target.session_id, (target, source))
                .is_some()
            {
                return Err(ErrorCode::InvalidRequest);
            }
        }
        let mut rows: HashMap<_, _> = unbound.iter().map(|r| (&r.row_id, r)).collect();
        if rows.len() != unbound.len() {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut i = self.inner.lock().expect("state mutex");
        if servers.len() > 64
            || projects.len() > 128
            || unbound.len() + i.sessions.len() + shell_map.len() > 512
            || hierarchy.len() > 64
            || hierarchy
                .iter()
                .map(|h| h.workspaces.len() + h.tabs.len() + h.panes.len())
                .sum::<usize>()
                > 512
        {
            return Err(ErrorCode::ResourceLimit);
        }
        let bound: HashSet<_> = i
            .sessions
            .values()
            .filter(|s| {
                !s.install_failed
                    && matches!(
                        s.state.summary.connectivity,
                        SessionConnectivity::Ready | SessionConnectivity::Synchronizing
                    )
            })
            .map(|s| {
                let summary = &s.state.summary;
                (&summary.server_id, &summary.terminal_id, summary.provider)
            })
            .collect();
        for h in &mut hierarchy {
            for p in &mut h.panes {
                let provider = p.agent.as_ref().and_then(|a| match a.0.as_str() {
                    "omp" => Some(Provider::Omp),
                    "codex" => Some(Provider::Codex),
                    _ => None,
                });
                if let Some(target) = &p.target {
                    let (captured, epoch) = epochs
                        .remove(&target.session_id)
                        .filter(|(captured, _)| *captured == target)
                        .ok_or(ErrorCode::InvalidRequest)?;
                    if provider.is_none() {
                        return Err(ErrorCode::InvalidRequest);
                    }
                    if let Some(s) = i.sessions.get(&captured.session_id) {
                        let summary = &s.state.summary;
                        if summary.server_id != h.server_id
                            || summary.terminal_id != p.terminal_id
                            || provider != Some(summary.provider)
                        {
                            return Err(ErrorCode::InvalidRequest);
                        }
                        if summary.generation == captured.generation
                            && &s.source == epoch
                            && !s.install_failed
                            && !s.topology_dirty
                            && summary.workspace_id == p.workspace_id
                            && summary.pane_id == p.pane_id
                            && matches!(
                                summary.connectivity,
                                SessionConnectivity::Ready | SessionConnectivity::Synchronizing
                            )
                        {
                            continue;
                        }
                    }
                    p.target = None;
                }
                if let Some(id) = &p.unbound_row_id {
                    let row = rows.remove(id).ok_or(ErrorCode::InvalidRequest)?;
                    if row.server_id != h.server_id
                        || row.workspace_id != p.workspace_id
                        || row.terminal_id != p.terminal_id
                        || row.pane_id != p.pane_id
                        || provider != Some(row.provider)
                    {
                        return Err(ErrorCode::InvalidRequest);
                    }
                    if bound.contains(&(&h.server_id, &p.terminal_id, row.provider)) {
                        p.unbound_row_id = None;
                    }
                }
            }
        }
        drop(rows);
        unbound.retain(|r| !bound.contains(&(&r.server_id, &r.terminal_id, r.provider)));
        // Atomic-current checks only: no source locks, filesystem/provider I/O or waits.
        validate(&mut hierarchy)?;
        for (summary,_) in shell_map.values_mut() {
            let associated=hierarchy.iter().any(|graph|graph.server_id==summary.server_id && graph.freshness==NativeFreshness::Current && graph.panes.iter().any(|pane|pane.shell_target.as_ref()==Some(&summary.target) && pane.terminal_id==summary.terminal_id && pane.workspace_id==summary.workspace_id && pane.tab_id==summary.tab_id && pane.pane_id==summary.pane_id));
            if !associated {summary.connectivity=SessionConnectivity::Offline;summary.capabilities=ShellCapabilities {read:false,create:false,input:false,close:false,artifacts:false,grid:false};}
        }
        let sessions = inventory_sessions(&i);
        cancel_grids(&mut i, |lease| {
            let Some((target,epoch))=lease.shell() else{return !lease.retains_agent_inventory(&sessions,&hierarchy);};
            let Some((summary,source))=shell_map.get(&target.shell_id) else{return true;};
            if summary.target!=*target||source!=epoch{return true;}
            let Some(graph)=hierarchy.iter().find(|graph|&graph.server_id==lease.server()) else{return true;};
            if !lease.retains_inventory(summary,graph){return true;}
            match graph.freshness {
                NativeFreshness::Stale=>{lease.inventory_updated();false}
                NativeFreshness::Current=>{
                    let current=summary.connectivity==SessionConnectivity::Ready&&summary.capabilities.read&&summary.capabilities.grid&&summary.capabilities.input;
                    if current{lease.inventory_updated();}
                    !current
                }
                _=>true,
            }
        });
        if i.inventory_sessions.as_ref() == Some(&sessions)
            && i.servers == servers
            && i.projects == projects
            && i.shells == shell_map
            && i.unbound == unbound
            && i.hierarchy == hierarchy
        {
            return Ok(());
        }
        i.servers = servers;
        i.projects = projects;
        i.shells = shell_map;
        i.unbound = unbound;
        i.hierarchy = hierarchy;
        self.creation_directories.retain(|context|creation_context_valid(&i, context, false).is_ok());
        i.inventory_sessions = Some(sessions);
        publish(
            &mut i,
            None,
            None,
            EventBody::HostChanged(HostChangedEvent {}),
        );
        Ok(())
    }
    /// Return the source epoch captured for this installation, including its completion/abort.
    pub fn begin_install(&self, mut state: SessionState) -> StateResult<Uuid> {
        state.summary.capabilities.terminal_read=false;
        valid(&state)?;
        let mut i = self.inner.lock().expect("state mutex");
        let id = state.summary.session_id.clone();
        let replacing_unbound = |row: &UnboundRow| {
            row.server_id == state.summary.server_id
                && row.terminal_id == state.summary.terminal_id
                && row.provider == state.summary.provider
        };
        let remaining_unbound = i
            .unbound
            .iter()
            .filter(|row| !replacing_unbound(row))
            .count();
        if !i.sessions.contains_key(&id) && i.sessions.len() + remaining_unbound + i.shells.len() >= 512 {
            return Err(ErrorCode::ResourceLimit);
        }
        if let Some(old) = i.sessions.get(&id) {
            if old.installing {
                return Err(ErrorCode::Busy);
            }
            if !same_binding(&old.state.summary, &state.summary) {
                return Err(ErrorCode::StaleSession);
            }
            if old.state.summary.generation == state.summary.generation {
                return Err(ErrorCode::StaleSession);
            }
            publish(
                &mut i,
                Some(&id),
                None,
                EventBody::SyncRequired(SyncRequiredEvent {
                    reason: SyncReason::GenerationChanged,
                }),
            );
        }
        cancel_grids(&mut i,|lease|lease.agent_target().is_some_and(|target|target.session_id==id));
        invalidate_publications(&mut i,&id);
        i.unbound.retain(|row| !replacing_unbound(row));
        for h in &mut i.hierarchy {
            for p in &mut h.panes {
                if h.server_id == state.summary.server_id
                    && p.terminal_id == state.summary.terminal_id
                {
                    p.target = None;
                    p.unbound_row_id = None;
                }
            }
        }
        i.content.retain(|_, p| p.target.session_id != id);
        i.cursors.retain(|_, c| c.target.session_id != id);
        i.native_cursors.retain(|_, c| c.target.session_id != id);
        i.browse.retain(|_, c| c.target.session_id != id);
        let source = Uuid::random();
        i.sessions.insert(
            id,
            Session {
                state,
                source: source.clone(),
                history: Arc::new(HistorySnapshot::default()),
                origins: HashMap::new(),
                pending: vec![],
                older: false,
                native_older: None,
                installing: true,
                install_failed: false,
                topology_dirty: false,
                observation_pending: false,
                native_queue: VecDeque::new(),
                native_bytes: 0,
            },
        );
        self.creation_directories.retain(|context|creation_context_valid(&i, context, false).is_ok());
        Ok(source)
    }
    /// Topology4 retires only a no-longer-required row. This never stops the native provider
    /// or deletes its durable identity/resources; all old handles and subscribers invalidate.
    pub fn retire_session(&self, session: &Uuid) {
        let mut i = self.inner.lock().expect("state mutex");
        cancel_grids(&mut i,|lease|lease.agent_target().is_some_and(|target|&target.session_id==session));
        publish(
            &mut i,
            Some(session),
            None,
            EventBody::SyncRequired(SyncRequiredEvent {
                reason: SyncReason::GenerationChanged,
            }),
        );
        invalidate_publications(&mut i,session);
        i.sessions.remove(session);
        self.creation_directories.retain(|context|creation_context_valid(&i, context, false).is_ok());
        i.adapter_history_bytes.remove(session);
        i.browse
            .retain(|_, node| &node.target.session_id != session);
        clear_native_target(&mut i, session);
        i.content.retain(|_, p| &p.target.session_id != session);
        i.cursors.retain(|_, c| &c.target.session_id != session);
        i.native_cursors
            .retain(|_, c| &c.target.session_id != session);
        for c in i.connections.values_mut() {
            c.subscriptions
                .retain(|_, s| !matches!(&s.kind, SubscriptionKind::Session(id) if id == session));
        }
        publish(
            &mut i,
            None,
            None,
            EventBody::HostChanged(HostChangedEvent {}),
        );
    }
    pub fn begin_source_install(&self, target: &Target) -> StateResult<Uuid> {
        let mut i = self.inner.lock().expect("state mutex");
        let s = i
            .sessions
            .get_mut(&target.session_id)
            .ok_or(ErrorCode::NotFound)?;
        if s.target() != *target {
            return Err(ErrorCode::StaleSession);
        }
        if s.installing {
            return Err(ErrorCode::Busy);
        }
        s.installing = true;
        s.install_failed = false;
        s.observation_pending = false;
        s.source = Uuid::random();
        s.state.summary.skills_generation=None;
        s.state.summary.capabilities.skills_read=false;
        s.state.summary.capabilities.controls=false;
        s.native_queue.clear();
        s.native_bytes = 0;
        let source = s.source.clone();
        cancel_grids(&mut i,|lease|lease.agent_target()==Some(target));
        invalidate_publications(&mut i,&target.session_id);
        i.content.retain(|_, p| p.target != *target);
        i.cursors.retain(|_, c| c.target != *target);
        i.native_cursors.retain(|_, c| c.target != *target);
        i.browse.retain(|_, c| c.target != *target);
        self.creation_directories.retain(|context|creation_context_valid(&i,context,false).is_ok());
        publish(
            &mut i,
            Some(&target.session_id),
            None,
            EventBody::SyncRequired(SyncRequiredEvent {
                reason: SyncReason::SourceGap,
            }),
        );
        Ok(source)
    }
    pub fn abort_install(&self, target: &Target, source: &Uuid) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        let s = i
            .sessions
            .get_mut(&target.session_id)
            .ok_or(ErrorCode::NotFound)?;
        if s.target() != *target || &s.source != source || !s.installing {
            return Err(ErrorCode::StaleSession);
        }
        s.installing = false;
        s.install_failed = true;
        s.observation_pending = false;
        s.native_queue.clear();
        s.native_bytes = 0;
        s.state.summary.connectivity = SessionConnectivity::Synchronizing;
        publish(
            &mut i,
            Some(&target.session_id),
            None,
            EventBody::SyncRequired(SyncRequiredEvent {
                reason: SyncReason::SourceGap,
            }),
        );
        Ok(())
    }
    pub fn finish_install(&self, source: &Uuid, mut snapshot: ProviderSnapshot) -> StateResult<()> {
        snapshot.state.summary.capabilities.terminal_read=false;
        valid(&snapshot.state)?;
        valid(&snapshot.pending)?;
        valid(&snapshot.native_older)?;
        if snapshot.pending.len() > 64 {
            return Err(ErrorCode::ResourceLimit);
        }
        if snapshot
            .pending
            .iter()
            .any(|p| p.provider != snapshot.state.summary.provider)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut i = self.inner.lock().expect("state mutex");
        let id = snapshot.state.summary.session_id.clone();
        let s = i.sessions.get_mut(&id).ok_or(ErrorCode::NotFound)?;
        if &s.source != source
            || !s.installing
            || s.target()
                != (Target {
                    session_id: id.clone(),
                    generation: snapshot.state.summary.generation.clone(),
                })
        {
            return Err(ErrorCode::StaleSession);
        }
        if s.install_failed {
            return Err(ErrorCode::ResourceLimit);
        }
        if !same_binding(&s.state.summary, &snapshot.state.summary) {
            return Err(ErrorCode::StaleSession);
        }
        s.state = snapshot.state;
        s.pending = snapshot.pending;
        s.history = Arc::new(HistorySnapshot::new(
            snapshot.history.into_iter().map(|r| r.0).collect(),
        ));
        s.native_older = snapshot.native_older;
        Arc::make_mut(&mut s.history).sort();
        s.older = snapshot.older_available;
        s.trim();
        s.installing = false;
        s.observation_pending = false;
        s.origins = s
            .history
            .iter()
            .map(|r| (r.id.clone(), HistoryOrigins::from_record(r)))
            .collect();
        let queued = std::mem::take(&mut s.native_queue);
        s.native_bytes = 0;
        for update in queued {
            apply(&mut i, &id, update)?;
        }
        trim_history(&mut i);
        publish(
            &mut i,
            None,
            None,
            EventBody::HostChanged(HostChangedEvent {}),
        );
        Ok(())
    }
    pub fn update(&self, target: &Target, update: SessionUpdate) -> StateResult<()> {
        update_locked(&mut self.inner.lock().expect("state mutex"), target, update)
    }
    /// Capture before an asynchronous file/page read; stale source material cannot be appended
    /// to its replacement even when the stable Target has not changed.
    /// Catalogue authority changes are ordered with ordinary session output at native ingress.
    pub(crate) fn skills_generation(&self, target: &Target, source: &Uuid, generation: Option<Uuid>) -> StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");
        let session=current(&i,target)?;
        if &session.source!=source { return Err(ErrorCode::StaleSession); }
        if session.state.summary.skills_generation==generation && session.state.summary.capabilities.skills_read==generation.is_some() {return Ok(());}
        let mut display=session.state.clone();
        display.summary.capabilities.skills_read=generation.is_some();
        display.summary.skills_generation=generation;
        invalidate_skills(&mut i,&target.session_id);
        update_locked(&mut i,target,SessionUpdate::State(display))
    }
    pub(crate) fn skills_list_response(
        &self,connection:&Uuid,response_id:Uuid,mut publication:crate::backends::SkillsPublication,
    )->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");
        let session=current(&i,&publication.catalogue.target)?;
        if publication.origin.context.target!=publication.catalogue.target
            ||session.source!=publication.origin.context.epoch||session.state.summary.provider!=publication.catalogue.provider
            ||session.state.summary.skills_generation.as_ref()!=Some(&publication.catalogue.catalogue_generation) {
            return Err(ErrorCode::StaleSession);
        }
        if session.state.summary.connectivity!=SessionConnectivity::Ready||!session.state.summary.capabilities.skills_read {
            return Err(ErrorCode::NotReady);
        }
        let invoke=publication.catalogue.provider==Provider::Codex&&session.state.summary.capabilities.prompt;
        let c=i.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||publication.origin.peer!=c.peer||publication.origin.peer_epoch!=c.epoch||&publication.origin.connection!=connection {
            return Err(ErrorCode::Unauthorized);
        }
        c.skills=None;
        for row in &mut publication.catalogue.skills {row.invocation_ref=if invoke&&row.enabled {Some(Uuid::random())}else{None};}
        valid(&publication.catalogue)?;
        let crate::backends::SkillsPublication {origin,catalogue,documents,document_expires,native_catalogue,native_cwd,slot}=publication;
        let envelope=Envelope::Response(Response{id:response_id,outcome:Ok(ResultBody::SkillsList(catalogue))});
        let bytes=frame(&envelope)?;
        c.flush();if !c.enqueue(bytes,false) {c.closed=true;return Err(ErrorCode::ResourceLimit)}
        let Envelope::Response(Response{outcome:Ok(ResultBody::SkillsList(catalogue)),..})=envelope else {return Err(ErrorCode::InvalidRequest)};
        c.skills=Some(Arc::new(crate::backends::SkillsPublication {origin,catalogue,documents,document_expires,native_catalogue,native_cwd,slot}));
        Ok(())
    }
    pub(crate) fn skills_preview_response(
        &self,connection:&Uuid,response_id:Uuid,publication:&Arc<crate::backends::SkillsPublication>,result:ResultBody,
    )->StateResult<()> {
        let target=&publication.catalogue.target;let generation=&publication.catalogue.catalogue_generation;
        let matches=match &result {
            ResultBody::SkillsPreviewOpen(value)=>&value.target==target&&&value.catalogue_generation==generation,
            ResultBody::SkillsPreviewRead(value)=>&value.target==target&&&value.catalogue_generation==generation,
            _=>false,
        };
        if !matches {return Err(ErrorCode::InvalidRequest)}
        let bytes=frame(&Envelope::Response(Response{id:response_id,outcome:Ok(result)}))?;
        let mut i=self.inner.lock().expect("state mutex");let session=current(&i,target)?;
        if session.source!=publication.origin.context.epoch||session.state.summary.skills_generation.as_ref()!=Some(generation) {
            return Err(ErrorCode::StaleSession);
        }
        if session.state.summary.connectivity!=SessionConnectivity::Ready||!session.state.summary.capabilities.skills_read {
            return Err(ErrorCode::NotReady);
        }
        let c=i.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||!c.skills.as_ref().is_some_and(|current|Arc::ptr_eq(current,publication))
            ||publication.origin.peer!=c.peer||publication.origin.peer_epoch!=c.epoch||&publication.origin.connection!=connection {
            return Err(ErrorCode::StaleSession);
        }
        c.flush();if !c.enqueue(bytes,false) {c.closed=true;return Err(ErrorCode::ResourceLimit)}Ok(())
    }
    pub(crate) fn controls_available(&self,target:&Target,source:&Uuid,available:bool)->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");let session=current(&i,target)?;
        if &session.source!=source {return Err(ErrorCode::StaleSession)}
        if session.state.summary.capabilities.controls==available {return Ok(())}
        let mut display=session.state.clone();display.summary.capabilities.controls=available;
        if !available {invalidate_controls(&mut i,&target.session_id);}
        update_locked(&mut i,target,SessionUpdate::State(display))
    }
    pub(crate) fn project_reads_available(&self,target:&Target,source:&Uuid,artifacts:bool,git:bool)->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");let session=current(&i,target)?;
        if &session.source!=source {return Err(ErrorCode::StaleSession);}
        let git=artifacts&&git;let capabilities=&session.state.summary.capabilities;
        if capabilities.artifacts==artifacts&&capabilities.worktrees_read==git&&capabilities.changes_read==git {return Ok(());}
        let mut display=session.state.clone();display.summary.capabilities.artifacts=artifacts;
        display.summary.capabilities.worktrees_read=git;display.summary.capabilities.changes_read=git;
        update_locked(&mut i,target,SessionUpdate::State(display))
    }
    pub(crate) fn controls_changed(&self,target:&Target,source:&Uuid)->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");
        if &current(&i,target)?.source!=source {return Err(ErrorCode::StaleSession)}
        invalidate_controls(&mut i,&target.session_id);
        publish(&mut i,Some(&target.session_id),None,EventBody::ControlsChanged(ControlsChangedEvent {target:target.clone()}));Ok(())
    }
    pub(crate) fn queue_changed(&self,target:&Target,source:&Uuid)->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");
        if &current(&i,target)?.source!=source {return Err(ErrorCode::StaleSession)}
        invalidate_queue(&mut i,&target.session_id);
        publish(&mut i,Some(&target.session_id),None,EventBody::QueueChanged(QueueChangedEvent {target:target.clone()}));Ok(())
    }
    pub(crate) fn clear_controls_publication(&self,connection:&Uuid) {
        if let Some(c)=self.inner.lock().expect("state mutex").connections.get_mut(connection) {c.controls_catalogue=None;}
    }
    pub(crate) fn controls_publication(
        &self,auth:&crate::pairing::Authorized,target:&Target,generation:&Uuid,
    )->StateResult<Arc<crate::backends::ControlsPublication>> {
        let mut i=self.inner.lock().expect("state mutex");prune_publications(&mut i,std::time::Instant::now());
        let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||&c.peer!=auth.proof.peer_id()||c.epoch!=auth.proof.epoch() {return Err(ErrorCode::Unauthorized)}
        let publication=c.controls_catalogue.as_ref().ok_or(ErrorCode::NotReady)?;
        if &publication.catalogue.target!=target||&publication.catalogue.catalogue_generation!=generation {return Err(ErrorCode::StaleSession)}
        Ok(publication.clone())
    }
    pub(crate) fn controls_publication_current(
        &self,auth:&crate::pairing::Authorized,publication:&Arc<crate::backends::ControlsPublication>,
    )->StateResult<()> {
        let i=self.inner.lock().expect("state mutex");let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||&c.peer!=auth.proof.peer_id()||c.epoch!=auth.proof.epoch()
            ||publication.origin.peer!=c.peer||publication.origin.peer_epoch!=c.epoch||publication.origin.connection!=auth.connection_id {
            return Err(ErrorCode::Unauthorized);
        }
        if std::time::Instant::now()>=publication.expires||!c.controls_catalogue.as_ref().is_some_and(|p|Arc::ptr_eq(p,publication)) {
            return Err(ErrorCode::StaleSession);
        }
        Ok(())
    }
    pub(crate) fn controls_response(
        &self,connection:&Uuid,response_id:Uuid,publication:crate::backends::ControlsPublication,
    )->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");let session=current(&i,&publication.catalogue.target)?;
        if session.source!=publication.origin.context.epoch||session.state.summary.provider!=publication.origin.context.provider {
            return Err(ErrorCode::StaleSession);
        }
        if session.state.summary.connectivity!=SessionConnectivity::Ready||!session.state.summary.capabilities.controls {
            return Err(ErrorCode::NotReady);
        }
        if publication.models.len()!=publication.catalogue.models.len()||std::time::Instant::now()>=publication.expires {
            return Err(ErrorCode::StaleSession);
        }
        valid(&publication.catalogue)?;
        let c=i.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||publication.origin.peer!=c.peer||publication.origin.peer_epoch!=c.epoch||&publication.origin.connection!=connection {
            return Err(ErrorCode::Unauthorized);
        }
        c.controls_catalogue=None;
        let crate::backends::ControlsPublication {origin,catalogue,models,current_native,native_catalogue,observation,expires,slot}=publication;
        let envelope=Envelope::Response(Response{id:response_id,outcome:Ok(ResultBody::SessionControlsRead(catalogue))});
        let bytes=frame(&envelope)?;c.flush();if !c.enqueue(bytes,false) {c.closed=true;return Err(ErrorCode::ResourceLimit)}
        let Envelope::Response(Response{outcome:Ok(ResultBody::SessionControlsRead(catalogue)),..})=envelope else {return Err(ErrorCode::InvalidRequest)};
        c.controls_catalogue=Some(Arc::new(crate::backends::ControlsPublication {origin,catalogue,models,current_native,native_catalogue,observation,expires,slot}));
        Ok(())
    }
    pub(crate) fn clear_queue_publication(&self,connection:&Uuid) {
        let mut i=self.inner.lock().expect("state mutex");
        if let Some(publication)=i.connections.get_mut(connection).and_then(|c|c.queue_view.take()) {
            for payload in &publication.payloads {i.content.remove(payload);}
        }
    }
    pub(crate) fn queue_publication(
        &self,auth:&crate::pairing::Authorized,target:&Target,cursor:Option<&Id>,
    )->StateResult<Arc<crate::backends::QueuePublication>> {
        let mut i=self.inner.lock().expect("state mutex");prune_publications(&mut i,std::time::Instant::now());
        let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||&c.peer!=auth.proof.peer_id()||c.epoch!=auth.proof.epoch() {return Err(ErrorCode::Unauthorized)}
        let publication=c.queue_view.as_ref().ok_or(ErrorCode::NotReady)?;
        if &publication.origin.context.target!=target||cursor.is_some_and(|cursor|publication.next_cursor.as_ref()!=Some(cursor))
            ||current(&i,target)?.source!=publication.origin.context.epoch {return Err(ErrorCode::StaleSession)}
        Ok(publication.clone())
    }
    pub(crate) fn queue_publication_current(
        &self,auth:&crate::pairing::Authorized,publication:&Arc<crate::backends::QueuePublication>,
    )->StateResult<()> {
        let i=self.inner.lock().expect("state mutex");let c=i.connections.get(&auth.connection_id).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||&c.peer!=auth.proof.peer_id()||c.epoch!=auth.proof.epoch()
            ||publication.origin.peer!=c.peer||publication.origin.peer_epoch!=c.epoch||publication.origin.connection!=auth.connection_id {return Err(ErrorCode::Unauthorized)}
        if std::time::Instant::now()>=publication.expires||!c.queue_view.as_ref().is_some_and(|current|Arc::ptr_eq(current,publication))
            ||current(&i,&publication.origin.context.target)?.source!=publication.origin.context.epoch {return Err(ErrorCode::StaleSession)}
        Ok(())
    }
    pub(crate) fn queue_response(
        &self,connection:&Uuid,response_id:Uuid,mut publication:crate::backends::QueuePublication,
        rows:Vec<crate::backends::QueueRow>,previous:Option<&Arc<crate::backends::QueuePublication>>,
    )->StateResult<()> {
        let mut i=self.inner.lock().expect("state mutex");
        let now=std::time::Instant::now();let target=&publication.origin.context.target;
        let session=current(&i,target)?;
        if session.source!=publication.origin.context.epoch||session.state.summary.provider!=Provider::Codex {return Err(ErrorCode::StaleSession)}
        if session.state.summary.connectivity!=SessionConnectivity::Ready {return Err(ErrorCode::NotReady)}
        if publication.entries.len()>128||rows.len()>100||rows.len()>publication.entries.len()||now>=publication.expires
            ||publication.next_cursor.is_some()!=publication.native_cursor.is_some() {return Err(ErrorCode::ResourceLimit)}
        let c=i.connections.get(connection).ok_or(ErrorCode::Unauthorized)?;
        if c.closed||publication.origin.peer!=c.peer||publication.origin.peer_epoch!=c.epoch||&publication.origin.connection!=connection {return Err(ErrorCode::Unauthorized)}
        match (previous,c.queue_view.as_ref()) {
            (None,None)=>{},
            (Some(previous),Some(current)) if Arc::ptr_eq(previous,current)=>{},
            _=>return Err(ErrorCode::StaleSession),
        }
        let mut unique=HashSet::new();
        if publication.entries.iter().any(|entry|!unique.insert(&entry.id))
            ||!publication.entries[publication.entries.len()-rows.len()..].iter().zip(&rows).all(|(binding,row)|binding.id==row.id&&binding.editable==row.editable) {return Err(ErrorCode::StaleSession)}
        drop(unique);
        let now_ms=crate::pairing::now_ms();
        let expires=now_ms.saturating_add(publication.expires.saturating_duration_since(now).as_millis().try_into().map_err(|_|ErrorCode::ResourceLimit)?);
        let mut fresh=Vec::new();
        let result=(||->StateResult<()> {
            let target=&publication.origin.context.target;
            let mut result=SessionQueueListResult {target:target.clone(),entries:Vec::with_capacity(rows.len()),next_cursor:publication.next_cursor.clone()};
            let envelope_bytes=crate::protocol::encoded_size(&result,256*1024).map_err(|_|ErrorCode::ResourceLimit)?;
            for row in rows {
                // Charge the complete escaped text, not merely its eventual content reference.
                let mut entry=SessionQueueEntry {id:row.id,text:Content::Text(ContentText {text:InlineText(row.text)}),editable:row.editable};
                publication.bytes=publication.bytes.checked_add(crate::protocol::encoded_size(&entry,256*1024).map_err(|_|ErrorCode::ResourceLimit)?).ok_or(ErrorCode::ResourceLimit)?;
                if publication.bytes.checked_add(envelope_bytes+publication.entries.len().saturating_sub(1)).is_none_or(|bytes|bytes>256*1024) {return Err(ErrorCode::ResourceLimit)}
                let Content::Text(text)=entry.text else {return Err(ErrorCode::InvalidRequest)};
                entry.text=project_content(&mut i,target,Mime::TextPlain,text.text.0,now_ms,expires)?;
                if let Content::Ref(reference)=&entry.text {fresh.push(reference.handle.clone());publication.payloads.push(reference.handle.clone());}
                result.entries.push(entry);
            }
            if publication.bytes.checked_add(envelope_bytes+publication.entries.len().saturating_sub(1)).is_none_or(|bytes|bytes>256*1024) {return Err(ErrorCode::ResourceLimit)}
            valid(&result)?;
            let bytes=frame(&Envelope::Response(Response {id:response_id,outcome:Ok(ResultBody::SessionQueueList(result))}))?;
            if std::time::Instant::now()>=publication.expires {return Err(ErrorCode::StaleSession)}
            let c=i.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;c.flush();
            if !c.enqueue(bytes,false) {c.closed=true;return Err(ErrorCode::ResourceLimit)}
            c.queue_view=Some(Arc::new(publication));Ok(())
        })();
        if result.is_err() {for payload in fresh {i.content.remove(&payload);}}
        result
    }
    pub fn source_epoch(&self, target: &Target) -> StateResult<Uuid> {
        Ok(current(&self.inner.lock().expect("state mutex"), target)?
            .source
            .clone())
    }
    /// Host-local display snapshot for topology routing; never an alternative native source.
    pub fn session_state(&self, target: &Target, source: &Uuid) -> StateResult<SessionState> {
        let i = self.inner.lock().expect("state mutex");
        let session = current(&i, target)?;
        if &session.source != source {
            return Err(ErrorCode::StaleSession);
        }
        Ok(session.state.clone())
    }
    pub fn validate_source(&self, target: &Target, source: &Uuid) -> StateResult<()> {
        let i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::StaleSession);
        }
        Ok(())
    }
    /// Source-ordered attention is committed synchronously while retirement is
    /// excluded. The caller proves physical/backend identity before entering;
    /// `commit` must perform only its SQLite transaction, never State/provider I/O.
    pub(crate) fn with_attention_source<T>(&self,target:&Target,source:&Uuid,provider:Provider,commit:impl FnOnce()->T)->StateResult<T>{
        let i=self.inner.lock().expect("state mutex");let session=current(&i,target)?;
        if &session.source!=source||session.state.summary.provider!=provider{return Err(ErrorCode::StaleSession)}
        let continuous=session.state.summary.connectivity==SessionConnectivity::Ready
            ||session.state.summary.connectivity==SessionConnectivity::Synchronizing&&(session.topology_dirty||session.observation_pending);
        if session.installing||session.install_failed||!continuous{return Err(ErrorCode::NotReady)}
        Ok(commit())
    }
    /// Native reads may continue while a bounded registry observation is pending, but
    /// no adapter may treat that source as mutation-ready until its own complete
    /// same-binding observation clears the gate.
    pub(crate) fn observation_ready(&self, target: &Target, source: &Uuid) -> StateResult<()> {
        let i = self.inner.lock().expect("state mutex");
        let session = current(&i, target)?;
        if &session.source != source {
            return Err(ErrorCode::StaleSession);
        }
        if session.observation_pending {
            return Err(ErrorCode::NotReady);
        }
        Ok(())
    }
    pub(crate) fn validate_agent_observer(&self,target:&Target,epoch:&Uuid,provider:Provider,server:&Uuid,route:&crate::herdr::shells::ShellIdentity)->StateResult<()> {
        let i=self.inner.lock().expect("state mutex");let session=current(&i,target)?;let summary=&session.state.summary;
        if &session.source!=epoch||summary.provider!=provider||&summary.server_id!=server
            ||summary.workspace_id!=route.workspace_id||summary.pane_id!=route.pane_id||summary.terminal_id!=route.terminal_id
        {return Err(ErrorCode::StaleSession);}
        if session.installing||session.install_failed||session.topology_dirty||session.observation_pending||summary.connectivity!=SessionConnectivity::Ready {
            return Err(ErrorCode::NotReady);
        }
        Ok(())
    }
    /// Topology revisions gate mutations without replacing a still-live native source.
    pub(crate) fn topology_synchronizing(&self, server: &Uuid) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        stale_hierarchy(&mut i, server);
        let targets: Vec<_> = i
            .sessions
            .values()
            .filter(|s| {
                &s.state.summary.server_id == server
                    && !s.installing
                    && !s.install_failed
                    && (s.state.summary.connectivity == SessionConnectivity::Ready
                        || s.observation_pending)
            })
            .map(|s| s.target())
            .collect();
        for target in targets {
            i.sessions
                .get_mut(&target.session_id)
                .expect("current session")
                .topology_dirty = true;
            let mut state = current(&i, &target)?.state.clone();
            state.summary.connectivity = SessionConnectivity::Synchronizing;
            apply(&mut i, &target.session_id, SessionUpdate::State(state))?;
        }
        Ok(())
    }
    pub(crate) fn topology_revalidated(
        &self,
        target: &Target,
        source: &Uuid,
        workspace: &Id,
        pane: &Id,
        validate: impl FnOnce() -> StateResult<()>,
    ) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::StaleSession);
        }
        validate()?;
        let session = i
            .sessions
            .get_mut(&target.session_id)
            .expect("current session");
        session.topology_dirty = false;
        let mut state = session.state.clone();
        state.summary.workspace_id = workspace.clone();
        state.summary.pane_id = pane.clone();
        if !session.installing
            && !session.install_failed
            && !session.observation_pending
            && state.summary.connectivity == SessionConnectivity::Synchronizing
        {
            state.summary.connectivity = SessionConnectivity::Ready;
        }
        apply(&mut i, &target.session_id, SessionUpdate::State(state))
    }
    /// A bounded native observation missed its deadline. It keeps this source and its
    /// read path alive, but atomically removes mutation readiness until the same binding
    /// later supplies a complete fresh observation.
    pub(crate) fn observation_pending(&self, target: &Target, source: &Uuid) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::StaleSession);
        }
        let session = i
            .sessions
            .get_mut(&target.session_id)
            .expect("current session");
        if session.observation_pending {
            return Ok(());
        }
        session.observation_pending = true;
        let mut state = session.state.clone();
        state.summary.connectivity = SessionConnectivity::Synchronizing;
        apply(&mut i, &target.session_id, SessionUpdate::State(state))
    }
    /// Only the adapter's complete same-source native identity/control observation may
    /// clear the pending gate; topology refreshes and ordinary provider updates cannot.
    pub(crate) fn observation_revalidated(
        &self,
        target: &Target,
        source: &Uuid,
    ) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::StaleSession);
        }
        let session = i
            .sessions
            .get_mut(&target.session_id)
            .expect("current session");
        if !session.observation_pending {
            return Ok(());
        }
        session.observation_pending = false;
        let mut state = session.state.clone();
        if !session.installing && !session.install_failed && !session.topology_dirty {
            state.summary.connectivity = SessionConnectivity::Ready;
        }
        apply(&mut i, &target.session_id, SessionUpdate::State(state))
    }
    pub(crate) fn adapter_history_size(
        &self,
        target: &Target,
        source: &Uuid,
        bytes: usize,
    ) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::StaleSession);
        }
        if bytes > SESSION_BYTES / 2 {
            return Err(ErrorCode::ResourceLimit);
        }
        let other = i
            .adapter_history_bytes
            .iter()
            .filter(|(id, _)| *id != &target.session_id)
            .map(|(_, (_, bytes))| *bytes)
            .sum::<usize>();
        if other.saturating_add(bytes) > SESSION_BYTES {
            return Err(ErrorCode::ResourceLimit);
        }
        i.adapter_history_bytes
            .insert(target.session_id.clone(), (source.clone(), bytes));
        trim_history(&mut i);
        Ok(())
    }
    pub fn session_required(&self, session: &Uuid) -> bool {
        let i = self.inner.lock().expect("state mutex");
        i.sessions.get(session).is_some_and(|s| {
            s.installing || !s.install_failed || !s.history.is_empty() || !s.pending.is_empty()
        }) || i.content.values().any(|p| &p.target.session_id == session)
            || i.cursors.values().any(|c| &c.target.session_id == session)
            || i.native_cursors
                .values()
                .any(|c| &c.target.session_id == session)
            || i.browse
                .values()
                .any(|node| &node.target.session_id == session)
            || i.connections.values().any(|c| {
                c.subscriptions
                    .values()
                    .any(|s| matches!(&s.kind, SubscriptionKind::Session(id) if id == session))
            })
    }
    pub fn binding_offline(&self, target: &Target, source: &Uuid, reason: &str) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::StaleSession);
        }
        offline_binding(&mut i, &target.session_id, reason)?;
        self.creation_directories.retain(|context|creation_context_valid(&i, context, false).is_ok());
        Ok(())
    }
    /// Loss of authoritative topology invalidates every old native callback/handle, while
    /// retaining the proven identity/history truthfully offline. This never stops providers.
    pub fn topology_offline(&self, server: &Uuid, reason: &str) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        cancel_grids(&mut i,|lease|lease.server()==server);
        stale_hierarchy(&mut i, server);
        self.creation_directories.retain(|context|&context.physical.server_id!=server);
        let ids: Vec<_> = i
            .sessions
            .iter()
            .filter(|(_, s)| &s.state.summary.server_id == server)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            offline_binding(&mut i, &id, reason)?;
        }
        Ok(())
    }
    pub fn update_from_source(
        &self,
        target: &Target,
        source: &Uuid,
        mut update: SessionUpdate,
    ) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        if &current(&i, target)?.source != source {
            return Err(ErrorCode::CursorExpired);
        }
        if let SessionUpdate::State(state) = &mut update {
            let routing = &current(&i, target)?.state.summary;
            state.summary.workspace_id = routing.workspace_id.clone();
            state.summary.pane_id = routing.pane_id.clone();
            state.summary.skills_generation = routing.skills_generation.clone();
            state.summary.capabilities.skills_read = routing.capabilities.skills_read;
        }
        update_locked(&mut i, target, update)
    }
    /// Native record batches may exceed a phone frame, never its reducer/event boundary.
    pub fn history_from_source(
        &self,
        target: &Target,
        source: &Uuid,
        records: Vec<SafeHistory>,
    ) -> StateResult<()> {
        let mut batch = Vec::new();
        let mut bytes = 0;
        for record in records {
            let size = record_bytes(record.record());
            if size > FRAME_MAX - 1024 {
                return Err(ErrorCode::ResourceLimit);
            }
            if batch.len() == 100 || bytes + size > FRAME_MAX - 1024 {
                self.update_from_source(
                    target,
                    source,
                    SessionUpdate::History(std::mem::take(&mut batch)),
                )?;
                bytes = 0;
            }
            bytes += size;
            batch.push(record);
        }
        if !batch.is_empty() {
            self.update_from_source(target, source, SessionUpdate::History(batch))?;
        }
        Ok(())
    }

    pub fn host_snapshot(&self, connection: &Uuid, request: Uuid) -> StateResult<Uuid> {
        let mut i = self.inner.lock().expect("state mutex");
        let subscription = Uuid::random();
        let result = HostSnapshotResult {
            subscription: subscription.clone(),
            seq: ZeroValue(0),
            servers: i.servers.clone(),
            projects: i.projects.clone(),
            sessions: inventory_sessions(&i),
            shells: {let mut shells:Vec<_>=i.shells.values().map(|(summary,_)|summary.clone()).collect();shells.sort_by(|left,right|left.target.shell_id.0.cmp(&right.target.shell_id.0));shells},
            unbound: i.unbound.clone(),
            hierarchy: i.hierarchy.clone(),
        };
        let bytes =
            host_inventory_payload(request, result).map_err(|_| ErrorCode::ResourceLimit)?;
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        if c.subscriptions.len() >= 32
            && !c
                .subscriptions
                .values()
                .any(|s| matches!(s.kind, SubscriptionKind::Host))
        {
            return Err(ErrorCode::ResourceLimit);
        }
        if !c.enqueue(bytes, false) {
            c.closed = true;
            return Err(ErrorCode::ResourceLimit);
        }
        c.subscriptions
            .retain(|_, s| !matches!(s.kind, SubscriptionKind::Host));
        c.subscriptions.insert(
            subscription.clone(),
            Subscription {
                kind: SubscriptionKind::Host,
                seq: 0,
            },
        );
        Ok(subscription)
    }
    pub fn session_open(
        &self,
        connection: &Uuid,
        request: Uuid,
        target: &Target,
        _now: u64,
    ) -> StateResult<Uuid> {
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        if c.subscriptions.len() >= 32 {
            return Err(ErrorCode::ResourceLimit);
        }
        let s = current(&i, target)?;
        if s.installing {
            return Err(ErrorCode::NotReady);
        }
        let subscription = Uuid::random();
        let (history, _) = latest_page(&s.history, s.history.len(), 100)?;
        let result = SessionOpenResult {
            subscription: subscription.clone(),
            seq: ZeroValue(0),
            state: Box::new(s.state.clone()),
            pending_requests: s.pending.clone(),
            history,
        };
        let mut response = Envelope::Response(Response {
            id: request,
            outcome: Ok(ResultBody::SessionOpen(result)),
        });
        let bytes = loop {
            if let Ok(bytes) = frame(&response) {
                break bytes;
            }
            let Envelope::Response(Response {
                outcome: Ok(ResultBody::SessionOpen(result)),
                ..
            }) = &mut response
            else {
                unreachable!()
            };
            if result.history.is_empty() {
                return Err(ErrorCode::ResourceLimit);
            }
            result.history.remove(0);
        };
        let c = i
            .connections
            .get_mut(connection)
            .expect("connection exists");
        c.flush();
        if !c.enqueue(bytes, false) {
            c.closed = true;
            return Err(ErrorCode::ResourceLimit);
        }
        c.subscriptions.insert(
            subscription.clone(),
            Subscription {
                kind: SubscriptionKind::Session(target.session_id.clone()),
                seq: 0,
            },
        );
        Ok(subscription)
    }
    /// Revoke/self-unpair: discard all queued session/private frames, enqueue only the caller's
    /// final unpair response, then drain/close. Package3 calls this under its revocation fence.
    pub fn self_unpair_final(&self, connection: &Uuid, response_id: Uuid) -> StateResult<()> {
        let bytes = frame(&Envelope::Response(Response {
            id: response_id,
            outcome: Ok(ResultBody::DeviceUnpair(DeviceUnpairResult {
                unpaired: TrueValue(true),
            })),
        }))?;
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        c.queue.clear();
        c.transient = None;
        c.skills = None;
        c.subscriptions.clear();
        c.bytes = 0;
        c.events = 0;
        c.closed = false;
        c.enqueue(bytes, false);
        c.closed = true;
        Ok(())
    }
    pub fn response(&self, connection: &Uuid, response: Response) -> StateResult<()> {
        let bytes = frame(&Envelope::Response(response))?;
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        if !c.enqueue(bytes, false) {
            c.closed = true;
            return Err(ErrorCode::ResourceLimit);
        }
        Ok(())
    }
    pub(crate) fn prune_creation_directories(&self) {
        let i=self.inner.lock().expect("state mutex");
        self.creation_directories.retain(|context|creation_context_valid(&i, context, false).is_ok());
    }
    pub(crate) fn creation_directories_response(
        &self,
        context:&crate::herdr::creation_directories::DirectoryContext,
        response_id:Uuid,
        result:CreationDirectoriesResult,
        publish_references:impl FnOnce()->StateResult<()>,
    )->StateResult<()> {
        let bytes=frame(&Envelope::Response(Response{id:response_id,outcome:Ok(ResultBody::CreationDirectories(result))}))?;
        let mut i=self.inner.lock().expect("state mutex");
        creation_context_valid(&i, context, true)?;
        publish_references()?;
        let c=i.connections.get_mut(&context.connection).ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        if !c.enqueue(bytes,false){c.closed=true;return Err(ErrorCode::ResourceLimit)}
        Ok(())
    }
    /// Worktree browsing is memory-only, but its current connection, original Target, source
    /// epoch and capability must still be admitted together with the queued response.
    pub fn worktrees_list_response(
        &self,
        connection: &Uuid,
        target: &Target,
        source: &Uuid,
        response_id: Uuid,
        result: WorktreesListResult,
    ) -> StateResult<()> {
        let bytes = frame(&Envelope::Response(Response {
            id: response_id,
            outcome: Ok(ResultBody::WorktreesList(result)),
        }))?;
        let mut i = self.inner.lock().expect("state mutex");
        {
            let session = current(&i, target)?;
            if &session.source != source {
                return Err(ErrorCode::StaleSession);
            }
            if session.state.summary.connectivity != SessionConnectivity::Ready
                || !session.state.summary.capabilities.worktrees_read
            {
                return Err(ErrorCode::NotReady);
            }
        }
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        if !c.enqueue(bytes, false) {
            c.closed = true;
            return Err(ErrorCode::ResourceLimit);
        }
        Ok(())
    }
    /// Changes remain memory-only and are queued only while the exact original source and
    /// capability are still live under this same state lock.
    pub fn changes_read_response(
        &self,
        connection: &Uuid,
        target: &Target,
        source: &Uuid,
        response_id: Uuid,
        result: ChangesReadResult,
    ) -> StateResult<()> {
        let bytes = frame(&Envelope::Response(Response {
            id: response_id,
            outcome: Ok(ResultBody::ChangesRead(result)),
        }))?;
        let mut i = self.inner.lock().expect("state mutex");
        {
            let session = current(&i, target)?;
            if &session.source != source { return Err(ErrorCode::StaleSession); }
            if session.state.summary.connectivity != SessionConnectivity::Ready
                || !session.state.summary.capabilities.changes_read { return Err(ErrorCode::NotReady); }
        }
        let c = i.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        if !c.enqueue(bytes, false) { c.closed = true; return Err(ErrorCode::ResourceLimit); }
        Ok(())
    }
    /// Use after existing-receipt lookup and again under3's live dispatch fence. Creation is
    /// deliberately not accepted here:4 validates actual server/project/wrapper availability.
    pub fn validate_session_mutation(&self, request: &Request) -> StateResult<()> {
        self.validate_session_mutation_checked(request, None)
            .map(|_| ())
            .map_err(|(_, code)| code)
    }
    /// A reservation source permits only OMP's dirty-topology, no-effect intent.
    /// The same-lock connectivity also classifies no-effect readiness waits.
    /// None retains strict dispatch validation. Closed guard context is temporary.
    pub(crate) fn validate_session_mutation_checked(
        &self,
        request: &Request,
        reservation_source: Option<&Uuid>,
    ) -> Result<SessionConnectivity, (&'static str, ErrorCode)> {
        let mut guard = "core_mutation";
        let result = (|| {
            valid(&request.body)?;
            let target = match &request.body {
                RequestBody::SessionControlsApply(p)=>&p.target,
                RequestBody::SessionQueueUpdate(p)=>&p.target,
                RequestBody::SessionQueueDelete(p)=>&p.target,
                RequestBody::SessionQueueReorder(p)=>&p.target,
                RequestBody::SessionQueueStart(p)=>&p.target,
                RequestBody::AttachmentsBegin(p)=>&p.target,
                RequestBody::AttachmentsChunk(p)=>&p.target,
                RequestBody::AttachmentsComplete(p)=>&p.target,
                RequestBody::AgentRename(p) => &p.target,
                RequestBody::WorkspaceRename(p) => &p.target,
                RequestBody::PromptSubmit(p) => &p.target,
                RequestBody::TurnCancel(p) => &p.target,
                RequestBody::RequestRespond(p) => &p.target,
                _ => return Err(ErrorCode::InvalidRequest),
            };
            let i = self.inner.lock().expect("state mutex");
            let session = current(&i, target)?;
            if reservation_source.is_some_and(|source| &session.source != source) {
                guard = "core_epoch";
                return Err(ErrorCode::StaleSession);
            }
            if session.installing {
                guard = "core_installing";
                return Err(ErrorCode::NotReady);
            }
            if reservation_source.is_some() && session.install_failed {
                guard = "core_install_failed";
                return Err(ErrorCode::NotReady);
            }
            if session.observation_pending {
                guard = "core_observation_pending";
                return Err(ErrorCode::NotReady);
            }
            let connectivity = session.state.summary.connectivity;
            let pending_intent = reservation_source.is_some()
                && session.state.summary.provider == Provider::Omp
                && session.topology_dirty
                && connectivity == SessionConnectivity::Synchronizing;
            if connectivity != SessionConnectivity::Ready && !pending_intent {
                guard = match connectivity {
                    SessionConnectivity::Ready => "core_mutation",
                    SessionConnectivity::Synchronizing if reservation_source.is_none() => {
                        "core_sync"
                    }
                    SessionConnectivity::Synchronizing
                        if session.state.summary.provider != Provider::Omp =>
                    {
                        "core_sync_provider"
                    }
                    SessionConnectivity::Synchronizing => "core_sync_clean",
                    SessionConnectivity::Offline => "core_offline",
                    SessionConnectivity::SetupRequired => "core_setup_required",
                    SessionConnectivity::Unavailable => "core_unavailable",
                };
                return Err(ErrorCode::NotReady);
            }
            match &request.body {
                RequestBody::SessionControlsApply(_) if !session.state.summary.capabilities.controls=>{guard="cap_controls";Err(ErrorCode::NotReady)}
                RequestBody::SessionQueueUpdate(_)|RequestBody::SessionQueueDelete(_)|RequestBody::SessionQueueReorder(_)|RequestBody::SessionQueueStart(_)
                    if session.state.summary.provider!=Provider::Codex=>{guard="queue_provider";Err(ErrorCode::NotReady)}
                body if body.attachment_action().is_some() && !session.state.summary.capabilities.attachments=>{guard="cap_attachments";Err(ErrorCode::NotReady)}
                RequestBody::AgentRename(_) if !session.state.summary.capabilities.agent_rename => { guard = "cap_agent_rename"; Err(ErrorCode::NotReady) }
                RequestBody::WorkspaceRename(_) if !session.state.summary.capabilities.workspace_rename => {
                    guard = "cap_workspace_rename";
                    Err(ErrorCode::NotReady)
                }
                RequestBody::PromptSubmit(_) if !session.state.summary.capabilities.prompt => {
                    guard = "cap_prompt";
                    Err(ErrorCode::NotReady)
                }
                RequestBody::PromptSubmit(p)=>{
                    if p.images.is_some()&&!session.state.summary.capabilities.attachments {guard="cap_attachments";return Err(ErrorCode::NotReady)}
                    match (session.state.summary.provider,p.delivery) {
                        (Provider::Omp,Some(PromptDelivery::Queue))|(Provider::Codex,Some(PromptDelivery::FollowUp|PromptDelivery::Aside))=>return Err(ErrorCode::InvalidRequest),
                        (Provider::Codex,Some(PromptDelivery::Steer))=>{
                            if p.native_turn_id.is_none()||p.native_turn_id!=session.state.native_turn_id {return Err(ErrorCode::StaleSession)}
                        }
                        _=>if p.native_turn_id.is_some() {return Err(ErrorCode::InvalidRequest)},
                    }
                    Ok(())
                }
                RequestBody::TurnCancel(p) => {
                    if !session.state.summary.capabilities.cancel {
                        guard = "cap_cancel";
                        return Err(ErrorCode::NotReady);
                    }
                    validate_turn_cancel(session.state.summary.provider, p)
                        .map_err(|_| ErrorCode::InvalidRequest)?;
                    if session.state.summary.provider == Provider::Codex
                        && p.native_turn_id != session.state.native_turn_id
                    {
                        return Err(ErrorCode::StaleSession);
                    }
                    Ok(())
                }
                RequestBody::RequestRespond(p) => {
                    if !session.state.summary.capabilities.respond {
                        guard = "cap_respond";
                        return Err(ErrorCode::NotReady);
                    }
                    let pending = session
                        .pending
                        .iter()
                        .find(|r| r.request_key == p.request_key)
                        .ok_or(ErrorCode::AlreadyResolved)?;
                    validate_answer(&pending.presentation, &p.answer)
                        .map_err(|_| ErrorCode::InvalidAnswer)
                }
                _ => Ok(()),
            }?;
            Ok(connectivity)
        })();
        result.map_err(|code| (guard, code))
    }
    pub fn unsubscribe(&self, connection: &Uuid, subscription: &Uuid) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        if let Some(sub) = c.subscriptions.remove(subscription)
            && matches!(sub.kind, SubscriptionKind::Grid(_))
        {
            if let SubscriptionKind::Grid(lease)=&sub.kind {lease.release();}
            c.queue.retain(|q| {
                q.grid
                    .as_ref()
                    .is_none_or(|(lease, _, _)| &lease.id != subscription)
            });
            c.bytes = c.queue.iter().map(|q| q.bytes.len() + 4).sum();
            c.events = c.queue.iter().filter(|q| q.event).count();
        }
        Ok(())
    }
    /// Returns payload bytes in exact enqueue order. A closed queue may be drained before closing
    /// the stream; no new frames are accepted. Only this connection's writer calls this method.
    pub fn pop_output(&self, connection: &Uuid) -> StateResult<(Option<Vec<u8>>, bool)> {
        let (output, closed) = self.pop_tagged_output(connection)?;
        Ok((output.map(|q| q.bytes), closed))
    }
    pub(crate) fn pop_tagged_output(
        &self,
        connection: &Uuid,
    ) -> StateResult<(Option<Queued>, bool)> {
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        c.flush();
        let output = c.queue.pop_front();
        if let Some((lease, _, deadline)) = output.as_ref().and_then(|q| q.grid.as_ref())
            && crate::herdr::grid::before(*deadline).is_err()
        {
            close_grid(c, &lease.id);
            return Err(ErrorCode::Timeout);
        }
        if let Some(q) = &output {
            c.bytes -= q.bytes.len() + 4;
            c.events -= usize::from(q.event);
        }
        Ok((output, c.closed))
    }
    pub(crate) fn grid_available(&self, connection: &Uuid) -> StateResult<()> {
        let i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        if c.closed {
            return Err(ErrorCode::Unauthorized);
        }
        if c.subscriptions
            .values()
            .any(|s| matches!(s.kind, SubscriptionKind::Grid(_)))
        {
            return Err(ErrorCode::Busy);
        }
        if c.subscriptions.len() >= 32 {
            return Err(ErrorCode::ResourceLimit);
        }
        Ok(())
    }
    pub(crate) fn grid_lease(&self,connection:&Uuid,subscription:&Uuid)->StateResult<Arc<crate::herdr::grid::GridLease>> {
        let i=self.inner.lock().expect("state mutex");
        let c=i.connections.get(connection).filter(|connection|!connection.closed).ok_or(ErrorCode::Unauthorized)?;
        match c.subscriptions.get(subscription).map(|subscription|&subscription.kind) {
            Some(SubscriptionKind::Grid(lease))if lease.active()=>Ok(lease.clone()),
            _=>Err(ErrorCode::StaleSession),
        }
    }
    pub(crate) fn reserve_grid(
        &self,
        connection: &Uuid,
        lease: Arc<crate::herdr::grid::GridLease>,
        deadline: tokio::time::Instant,
    ) -> StateResult<()> {
        let mut i = self.inner.lock().expect("state mutex");
        crate::herdr::grid::before(deadline)?;
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        if c.closed {
            return Err(ErrorCode::Unauthorized);
        }
        if c.subscriptions
            .values()
            .any(|s| matches!(s.kind, SubscriptionKind::Grid(_)))
        {
            return Err(ErrorCode::Busy);
        }
        if c.subscriptions.len() >= 32 {
            return Err(ErrorCode::ResourceLimit);
        }
        crate::herdr::grid::before(deadline)?;
        c.subscriptions.insert(
            lease.id.clone(),
            Subscription {
                kind: SubscriptionKind::Grid(lease),
                seq: 0,
            },
        );
        Ok(())
    }
    pub(crate) fn grid_open_response(
        &self,
        connection: &Uuid,
        request: Uuid,
        result: ResultBody,
        lease: Arc<crate::herdr::grid::GridLease>,
        deadline: tokio::time::Instant,
        bytes: &mut usize,
    ) -> StateResult<tokio::sync::oneshot::Receiver<()>> {
        let payload = frame(&Envelope::Response(Response {
            id: request,
            outcome: Ok(result),
        }))?;
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        if !c.subscriptions.get(&lease.id).is_some_and(
            |s| matches!(&s.kind,SubscriptionKind::Grid(original) if Arc::ptr_eq(original,&lease)),
        ) {
            return Err(ErrorCode::StaleSession);
        }
        grid_queue(c, payload, false, lease, deadline, bytes)
    }
    pub(crate) fn grid_event(
        &self,
        connection: &Uuid,
        lease: &Arc<crate::herdr::grid::GridLease>,
        body: EventBody,
        deadline: tokio::time::Instant,
        bytes: &mut usize,
    ) -> StateResult<tokio::sync::oneshot::Receiver<()>> {
        let mut i = self.inner.lock().expect("state mutex");
        let c = i
            .connections
            .get_mut(connection)
            .ok_or(ErrorCode::Unauthorized)?;
        let sub = c
            .subscriptions
            .get(&lease.id)
            .filter(|s| matches!(&s.kind,SubscriptionKind::Grid(old) if Arc::ptr_eq(old,lease)))
            .ok_or(ErrorCode::StaleSession)?;
        let seq = sub
            .seq
            .checked_add(1)
            .filter(|n| *n <= UINT_MAX)
            .ok_or(ErrorCode::ResourceLimit)?;
        let payload = frame(&Envelope::Event(Event {
            subscription: lease.id.clone(),
            seq: Uint(seq),
            body,
        }))?;
        let receiver = grid_queue(c, payload, true, lease.clone(), deadline, bytes)?;
        c.subscriptions.get_mut(&lease.id).expect("grid exists").seq = seq;
        Ok(receiver)
    }
    pub(crate) fn grid_failed(&self, connection: &Uuid, subscription: &Uuid) {
        let mut i = self.inner.lock().expect("state mutex");
        if let Some(c) = i.connections.get_mut(connection) {
            close_grid(c, subscription);
        }
    }
    pub(crate) fn reconcile_grid_source(&self, source: &crate::herdr::Source) {
        let mut i = self.inner.lock().expect("state mutex");
        cancel_grids(&mut i, |lease| {
            if lease.server()!=&source.server_id{return false;}
            if lease.accepts_source(source){lease.source_updated(source);false}else{true}
        });
    }
    pub fn peer_event(&self, peer: &AuthenticatedPeer, event: EventBody) -> StateResult<()> {
        if !matches!(
            event,
            EventBody::ActionStatus(_)
                | EventBody::NotificationUpsert(_)
                | EventBody::NotificationRead(_)
                | EventBody::SyncRequired(_)
        ) {
            return Err(ErrorCode::InvalidRequest);
        }
        valid(&event)?;
        publish(
            &mut self.inner.lock().expect("state mutex"),
            None,
            Some(peer.peer_id()),
            event,
        );
        Ok(())
    }
    pub fn history(
        &self,
        peer: &AuthenticatedPeer,
        target: &Target,
        token: Option<&Id>,
        limit: usize,
        now: u64,
    ) -> StateResult<HistoryPage> {
        if !(1..=100).contains(&limit) {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut i = self.inner.lock().expect("state mutex");
        let s = current(&i, target).map_err(|error| {
            if token.is_some() {
                ErrorCode::CursorExpired
            } else {
                error
            }
        })?;
        if s.installing {
            return Err(ErrorCode::NotReady);
        }
        let source = s.source.clone();
        let older = s.older;
        let (records, end) = if let Some(token) = token {
            let c = i
                .cursors
                .get(&Uuid(token.0.clone()))
                .ok_or(ErrorCode::CursorExpired)?;
            if c.peer != *peer.peer_id()
                || c.target != *target
                || c.source != source
                || now >= c.expires
            {
                return Err(ErrorCode::CursorExpired);
            }
            (Arc::clone(&c.records), c.end)
        } else {
            (Arc::clone(&s.history), s.history.len())
        };
        let (page, end) = latest_page(&records, end, limit)?;
        let refetch_older = (end == 0 && older).then(|| HistoryRefetch {
            target: target.clone(),
            source: source.clone(),
            oldest: records
                .first()
                .map(|record| (record.id.clone(), record.parent_id.clone(), record.order.0)),
            originating_cursor: token.cloned(),
            records: Arc::clone(&records),
        });
        let next_cursor = cursor(
            &mut i,
            peer.peer_id().clone(),
            target.clone(),
            source,
            records,
            end,
            now,
        )?;
        let result = refresh_history_page(
            &i,
            target,
            &SessionHistoryResult {
                view_id: token
                    .map(|t| Uuid(t.0.clone()))
                    .unwrap_or_else(Uuid::random),
                records: page,
                next_cursor,
            },
        )?;
        Ok(HistoryPage {
            result,
            refetch_older,
        })
    }
    /// Opaque phone token -> provider read-only continuation, scoped to peer/Target/source.
    pub fn native_history_cursor(
        &self,
        peer: &AuthenticatedPeer,
        target: &Target,
        expected_source: &Uuid,
        native: Id,
        refetch: Option<&HistoryRefetch>,
        now: u64,
    ) -> StateResult<Id> {
        valid(&native)?;
        let mut i = self.inner.lock().expect("state mutex");
        let source = current(&i, target)?.source.clone();
        if &source != expected_source {
            return Err(ErrorCode::CursorExpired);
        }
        let native = if let Some(refetch) = refetch {
            if refetch.target != *target || refetch.source != source {
                return Err(ErrorCode::CursorExpired);
            }
            if let Some(original) = &refetch.originating_cursor {
                let original = i
                    .cursors
                    .get(&Uuid(original.0.clone()))
                    .ok_or(ErrorCode::CursorExpired)?;
                if original.peer != *peer.peer_id()
                    || original.target != *target
                    || original.source != source
                    || now >= original.expires
                    || !Arc::ptr_eq(&original.records, &refetch.records)
                {
                    return Err(ErrorCode::CursorExpired);
                }
                let existing = i
                    .cursors
                    .values()
                    .filter(|cursor| {
                        cursor.peer == *peer.peer_id()
                            && cursor.target == *target
                            && cursor.source == source
                            && now < cursor.expires
                            && Arc::ptr_eq(&cursor.records, &refetch.records)
                    })
                    .find_map(|cursor| cursor.native_older.clone());
                if let Some(existing) = existing {
                    existing
                } else {
                    for cursor in i.cursors.values_mut().filter(|cursor| {
                        cursor.peer == *peer.peer_id()
                            && cursor.target == *target
                            && cursor.source == source
                            && now < cursor.expires
                            && Arc::ptr_eq(&cursor.records, &refetch.records)
                    }) {
                        cursor.native_older = Some(native.clone());
                    }
                    native
                }
            } else {
                native
            }
        } else {
            native
        };
        native_cursor(
            &mut i,
            peer.peer_id().clone(),
            target.clone(),
            source,
            native,
            now,
        )
    }
    pub fn native_history_page(
        &self,
        peer: &AuthenticatedPeer,
        target: &Target,
        token: &Id,
        limit: usize,
        now: u64,
    ) -> StateResult<NativePageLookup> {
        if !(1..=100).contains(&limit) {
            return Err(ErrorCode::InvalidRequest);
        }
        let i = self.inner.lock().expect("state mutex");
        let source = current(&i, target)
            .map_err(|_| ErrorCode::CursorExpired)?
            .source
            .clone();
        let token = Uuid(token.0.clone());
        let cursor = i
            .native_cursors
            .get(&token)
            .ok_or(ErrorCode::CursorExpired)?;
        if cursor.peer != *peer.peer_id()
            || cursor.target != *target
            || cursor.source != source
            || now >= cursor.expires
        {
            return Err(ErrorCode::CursorExpired);
        }
        if let Some(page) = cursor.pages.get(&limit) {
            return Ok(NativePageLookup::Cached(refresh_history_page(
                &i, target, page,
            )?));
        }
        Ok(NativePageLookup::Fetch(NativePageTicket {
            peer: cursor.peer.clone(),
            target: target.clone(),
            source,
            token,
            limit,
            native: cursor.native.clone(),
        }))
    }
    /// Install the actual redacted native response. Racing/repeated fetches return the first
    /// installed page for the same cursor/limit; reads never consume or replay a provider write.
    pub fn finish_native_history_page(
        &self,
        ticket: NativePageTicket,
        records: Vec<SafeHistory>,
        next_native: Option<Id>,
        now: u64,
    ) -> StateResult<SessionHistoryResult> {
        self.finish_native_history_page_with_budget(
            ticket,
            records,
            next_native,
            now,
            SESSION_BYTES,
        )
    }
    fn finish_native_history_page_with_budget(
        &self,
        ticket: NativePageTicket,
        records: Vec<SafeHistory>,
        next_native: Option<Id>,
        now: u64,
        budget: usize,
    ) -> StateResult<SessionHistoryResult> {
        if records.len() > HISTORY_RECORDS {
            return Err(ErrorCode::ResourceLimit);
        }
        let mut i = self.inner.lock().expect("state mutex");
        let source = current(&i, &ticket.target)
            .map_err(|_| ErrorCode::CursorExpired)?
            .source
            .clone();
        let cursor = i
            .native_cursors
            .get(&ticket.token)
            .ok_or(ErrorCode::CursorExpired)?;
        if cursor.peer != ticket.peer
            || cursor.target != ticket.target
            || cursor.source != ticket.source
            || source != ticket.source
            || now >= cursor.expires
        {
            return Err(ErrorCode::CursorExpired);
        }
        if let Some(page) = cursor.pages.get(&ticket.limit) {
            return refresh_history_page(&i, &ticket.target, page);
        }
        let mut records: Vec<_> = records.into_iter().map(|r| r.0).collect();
        records.sort_by_key(|r| r.order);
        let records = Arc::new(HistorySnapshot::new(records));
        let (page, end) = latest_page(&records, records.len(), ticket.limit)?;
        let page_bytes = page.iter().map(record_bytes).sum::<usize>();
        let retained = (if end > 0 { records.bytes } else { 0 }) + page_bytes;
        // The page and a new native successor are retained together. Reusing the exact
        // unexpired successor allocates nothing; rejection must precede either insertion.
        let successor_bytes = if end == 0 {
            next_native.as_ref().map_or(0, |native| {
                if i.native_cursors.values().any(|cursor| {
                    cursor.peer == ticket.peer
                        && cursor.target == ticket.target
                        && cursor.source == ticket.source
                        && cursor.native == *native
                        && now < cursor.expires
                }) {
                    0
                } else {
                    native.0.len() + 256
                }
            })
        } else {
            0
        };
        if history_storage_bytes(&i) + retained + successor_bytes > budget {
            return Err(ErrorCode::ResourceLimit);
        }
        let next_cursor = if end > 0 {
            let id = Uuid::random();
            i.cursors.insert(
                id.clone(),
                Cursor {
                    peer: ticket.peer.clone(),
                    target: ticket.target.clone(),
                    source: ticket.source.clone(),
                    records,
                    end,
                    expires: now.saturating_add(CONTENT_TTL_MS),
                    native_older: next_native,
                },
            );
            Some(Id(id.0))
        } else {
            next_native
                .map(|native| {
                    native_cursor(
                        &mut i,
                        ticket.peer.clone(),
                        ticket.target.clone(),
                        ticket.source.clone(),
                        native,
                        now,
                    )
                })
                .transpose()?
        };
        let result = SessionHistoryResult {
            view_id: ticket.token.clone(),
            records: page,
            next_cursor,
        };
        let refreshed = refresh_history_page(&i, &ticket.target, &result)?;
        let cursor = i
            .native_cursors
            .get_mut(&ticket.token)
            .ok_or(ErrorCode::CursorExpired)?;
        cursor.pages.insert(ticket.limit, result);
        cursor.page_bytes += page_bytes;
        Ok(refreshed)
    }

    pub fn content(
        &self,
        target: &Target,
        mime: Mime,
        text: String,
        now: u64,
    ) -> StateResult<Content> {
        project_content(&mut self.inner.lock().expect("state mutex"),target,mime,text,now,now.saturating_add(CONTENT_TTL_MS))
    }
    /// The runtime must recheck this current authenticated peer under its package3 dispatch
    /// fence. Display payloads are host-shared; this state API is not a revocation fence.
    pub fn read_content(
        &self,
        _peer: &AuthenticatedPeer,
        request: &ContentReadParams,
        now: u64,
    ) -> StateResult<ContentReadResult> {
        valid(request)?;
        let mut i = self.inner.lock().expect("state mutex");
        prune_publications(&mut i,std::time::Instant::now());
        let source = current(&i, &request.target)
            .map_err(|_| ErrorCode::ContentExpired)?
            .source
            .clone();
        i.clock += 1;
        let used = i.clock;
        let p = i
            .content
            .get_mut(&request.handle)
            .ok_or(ErrorCode::ContentExpired)?;
        if p.target != request.target || p.source != source || now >= p.expires {
            return Err(ErrorCode::ContentExpired);
        }
        let start =
            usize::try_from(request.offset_bytes.0).map_err(|_| ErrorCode::InvalidRequest)?;
        if start > p.data.len() {
            return Err(ErrorCode::InvalidRequest);
        }
        let end = p.data.len().min(start + request.limit_bytes.0 as usize);
        p.used = used;
        // Mutex pins the payload through the bounded base64 copy, including UTF-8 split chunks.
        Ok(ContentReadResult {
            bytes: Base64Bytes(STANDARD.encode(&p.data[start..end])),
            next_offset_bytes: Uint(end as u64),
            eof: end == p.data.len(),
        })
    }
}
pub struct HistoryPage {
    pub result: SessionHistoryResult,
    /// Immutable retained-view boundary captured under the cursor lock. Native adapters
    /// must not consult the live session after their asynchronous provider read begins.
    pub refetch_older: Option<HistoryRefetch>,
}
#[derive(Clone)]
pub struct HistoryRefetch {
    pub target: Target,
    pub source: Uuid,
    pub oldest: Option<(Id, Option<Id>, u64)>,
    originating_cursor: Option<Id>,
    records: Arc<HistorySnapshot>,
}
fn content_size(i: &Inner, session: Option<&Uuid>) -> usize {
    i.content
        .values()
        .filter(|p| session.is_none_or(|id| &p.target.session_id == id))
        .map(|p| p.data.len())
        .sum()
}
fn clear_native_target(i: &mut Inner, id: &Uuid) {
    for h in &mut i.hierarchy {
        for p in &mut h.panes {
            if p.target
                .as_ref()
                .is_some_and(|target| &target.session_id == id)
            {
                p.target = None;
            }
        }
    }
}
fn stale_hierarchy(i: &mut Inner, server: &Uuid) {
    let mut changed = false;
    for h in i.hierarchy.iter_mut().filter(|h| &h.server_id == server) {
        if h.freshness == NativeFreshness::Current
            || h.reason == Some(NativeUnavailableReason::TooLarge)
        {
            if h.freshness != NativeFreshness::Unavailable {
                h.freshness = NativeFreshness::Stale;
            }
            h.reason = Some(NativeUnavailableReason::SourceUnavailable);
            for p in &mut h.panes {
                p.target = None;
                p.unbound_row_id = None;
                p.shell_target = None;
            }
            changed = true;
        }
    }
    for (shell, _) in i.shells.values_mut().filter(|(shell, _)| &shell.server_id == server) {
        let capabilities = ShellCapabilities {
            read: false,
            create: false,
            input: false,
            close: false,
            artifacts: false,
            grid: false,
        };
        if shell.connectivity != SessionConnectivity::Offline || shell.capabilities != capabilities {
            shell.connectivity = SessionConnectivity::Offline;
            shell.capabilities = capabilities;
            changed = true;
        }
    }
    for connection in i.connections.values() {
        for subscription in connection.subscriptions.values() {
            if let SubscriptionKind::Grid(lease)=&subscription.kind
                &&lease.server()==server {lease.inventory_updated();}
        }
    }
    if changed {
        publish(i, None, None, EventBody::HostChanged(HostChangedEvent {}));
    }
}
fn invalidate_publications(i:&mut Inner,id:&Uuid) {
    invalidate_skills(i,id);invalidate_controls(i,id);invalidate_queue(i,id);
}
fn invalidate_skills(i:&mut Inner,id:&Uuid) {
    for c in i.connections.values_mut() {
        if c.skills.as_ref().is_some_and(|p|&p.catalogue.target.session_id==id) {c.skills=None;}
    }
}
fn invalidate_controls(i:&mut Inner,id:&Uuid) {
    for c in i.connections.values_mut() {
        if c.controls_catalogue.as_ref().is_some_and(|p|&p.catalogue.target.session_id==id) {c.controls_catalogue=None;}
    }
}
fn project_content(i:&mut Inner,target:&Target,mime:Mime,text:String,now:u64,expires:u64)->StateResult<Content> {
    let source=current(i,target)?.source.clone();
    if text.len()<=CONTENT_INLINE_MAX {return Ok(Content::Text(ContentText {text:InlineText(text)}))}
    if text.len()>CONTENT_MAX as usize {return Ok(Content::Unavailable(ContentUnavailable {reason:ContentUnavailableReason::TooLarge,byte_length:Some(Uint(text.len() as u64))}))}
    i.content.retain(|_,payload|now<payload.expires);
    i.clock+=1;
    while content_size(i,Some(&target.session_id))+text.len()>SESSION_BYTES||content_size(i,None)+text.len()>CONTENT_TOTAL_BYTES {
        let session_full=content_size(i,Some(&target.session_id))+text.len()>SESSION_BYTES;
        let victim=i.content.iter().filter(|(_,payload)|!session_full||payload.target.session_id==target.session_id)
            .min_by_key(|(_,payload)|payload.used).map(|(id,_)|id.clone()).ok_or(ErrorCode::ResourceLimit)?;
        i.content.remove(&victim);
    }
    let handle=Uuid::random();let byte_length=Uint(text.len() as u64);let used=i.clock;
    i.content.insert(handle.clone(),Payload {target:target.clone(),source,data:text.into_bytes(),expires,used});
    Ok(Content::Ref(ContentRef {handle,byte_length,mime}))
}
fn invalidate_queue(i:&mut Inner,id:&Uuid) {
    for c in i.connections.values_mut() {
        if c.queue_view.as_ref().is_some_and(|p|&p.origin.context.target.session_id==id)
            &&let Some(publication)=c.queue_view.take() {
            for payload in &publication.payloads {i.content.remove(payload);}
        }
    }
}
fn prune_publications(i:&mut Inner,now:std::time::Instant) {
    for c in i.connections.values_mut() {
        if c.skills.as_ref().is_some_and(|p|now>=p.document_expires) {c.skills=None;}
        if c.controls_catalogue.as_ref().is_some_and(|p|now>=p.expires) {c.controls_catalogue=None;}
        if c.queue_view.as_ref().is_some_and(|p|now>=p.expires)
            &&let Some(publication)=c.queue_view.take() {
            for payload in &publication.payloads {i.content.remove(payload);}
        }
    }
}
fn offline_binding(i: &mut Inner, id: &Uuid, reason: &str) -> StateResult<()> {
    invalidate_publications(i,id);
    i.adapter_history_bytes.remove(id);
    let session = i.sessions.get_mut(id).ok_or(ErrorCode::NotFound)?;
    session.source = Uuid::random();
    session.installing = false;
    session.install_failed = true;
    session.native_queue.clear();
    session.native_bytes = 0;
    session.pending.clear();
    let mut display = session.state.clone();
    display.summary.connectivity = SessionConnectivity::Offline;
    display.summary.skills_generation=None;
    display.summary.capabilities.skills_read=false;
    display.summary.capabilities.controls=false;
    display.summary.reason = Some(reason.into());
    display.native_turn_id = None;
    display.activity = Activity {
        state: ActivityState::Unknown,
        text: None,
        tool_record_id: None,
    };
    clear_native_target(i, id);
    i.content.retain(|_, p| &p.target.session_id != id);
    i.cursors.retain(|_, c| &c.target.session_id != id);
    i.native_cursors.retain(|_, c| &c.target.session_id != id);
    i.browse.retain(|_, c| &c.target.session_id != id);
    apply(i, id, SessionUpdate::State(display))?;
    publish(
        i,
        Some(id),
        None,
        EventBody::SyncRequired(SyncRequiredEvent {
            reason: SyncReason::SourceGap,
        }),
    );
    Ok(())
}
fn current<'a>(i: &'a Inner, target: &Target) -> StateResult<&'a Session> {
    let s = i
        .sessions
        .get(&target.session_id)
        .ok_or(ErrorCode::NotFound)?;
    if s.target() != *target {
        return Err(ErrorCode::StaleSession);
    }
    Ok(s)
}
fn cursor(
    i: &mut Inner,
    peer: Uuid,
    target: Target,
    source: Uuid,
    records: Arc<HistorySnapshot>,
    end: usize,
    now: u64,
) -> StateResult<Option<Id>> {
    i.cursors.retain(|_, c| now < c.expires);
    let native_older = i
        .cursors
        .values()
        .find(|c| {
            c.peer == peer
                && Arc::ptr_eq(&c.records, &records)
                && c.target == target
                && c.source == source
        })
        .map(|c| c.native_older.clone())
        .unwrap_or_else(|| {
            i.sessions
                .get(&target.session_id)
                .and_then(|s| s.native_older.clone())
        });
    if end == 0 {
        return native_older
            .map(|native| native_cursor(i, peer, target, source, native, now))
            .transpose();
    }
    // Retrying a page returns the same successor cursor; no moving native cursor is consumed.
    if let Some((id, _)) = i.cursors.iter().find(|(_, c)| {
        c.peer == peer
            && c.target == target
            && c.source == source
            && c.end == end
            && Arc::ptr_eq(&c.records, &records)
    }) {
        return Ok(Some(Id(id.0.clone())));
    }
    let id = Uuid::random();
    i.cursors.insert(
        id.clone(),
        Cursor {
            peer,
            target,
            source,
            records,
            end,
            expires: now.saturating_add(CONTENT_TTL_MS),
            native_older,
        },
    );
    Ok(Some(Id(id.0)))
}
fn native_cursor_bytes(i: &Inner) -> usize {
    i.native_cursors
        .values()
        .map(|c| c.native.0.len() + 256 + c.page_bytes)
        .sum()
}
fn native_cursor(
    i: &mut Inner,
    peer: Uuid,
    target: Target,
    source: Uuid,
    native: Id,
    now: u64,
) -> StateResult<Id> {
    valid(&native)?;
    i.native_cursors.retain(|_, c| now < c.expires);
    i.browse.retain(|_, c| now < c.expires);
    if let Some((id, _)) = i.native_cursors.iter().find(|(_, c)| {
        c.peer == peer && c.target == target && c.source == source && c.native == native
    }) {
        return Ok(Id(id.0.clone()));
    }
    if history_storage_bytes(i) + native.0.len() + 256 > SESSION_BYTES {
        return Err(ErrorCode::ResourceLimit);
    }
    let id = Uuid::random();
    i.native_cursors.insert(
        id.clone(),
        NativeCursor {
            peer,
            target,
            source,
            native,
            expires: now.saturating_add(CONTENT_TTL_MS),
            pages: HashMap::new(),
            page_bytes: 0,
        },
    );
    Ok(Id(id.0))
}
fn latest_page(
    records: &[HistoryRecord],
    end: usize,
    limit: usize,
) -> StateResult<(Vec<HistoryRecord>, usize)> {
    let mut start = end.saturating_sub(limit);
    loop {
        let page = records[start..end].to_vec();
        let result = SessionHistoryResult {
            view_id: Uuid::random(),
            records: page.clone(),
            next_cursor: (start > 0).then(|| Id(Uuid::random().0)),
        };
        if start == end && end > 0 {
            return Err(ErrorCode::ResourceLimit);
        }
        if frame(&Envelope::Response(Response {
            id: Uuid::random(),
            outcome: Ok(ResultBody::SessionHistory(result)),
        }))
        .is_ok()
        {
            return Ok((page, start));
        }
        if start == end {
            return Err(ErrorCode::ResourceLimit);
        }
        start += 1;
    }
}
fn publish(i: &mut Inner, session: Option<&Uuid>, peer: Option<&Uuid>, body: EventBody) {
    for c in i
        .connections
        .values_mut()
        .filter(|c| peer.is_none_or(|p| &c.peer == p))
    {
        let subscriptions: Vec<_> = c
            .subscriptions
            .iter()
            .filter(|(_, s)| match &s.kind {
                SubscriptionKind::Host => session.is_none(),
                SubscriptionKind::Session(id) => session == Some(id),
                SubscriptionKind::Grid(_) => false,
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in subscriptions {
            if let EventBody::ActivityUpdate(update) = &body {
                if c.transient.as_ref().is_some_and(|(old, _)| old != &id) {
                    c.flush();
                }
                c.transient = Some((id, update.activity.clone()));
            } else {
                c.flush();
                c.emit(&id, body.clone());
            }
        }
    }
}
fn update_size(update: &SessionUpdate) -> StateResult<usize> {
    let body = match update {
        SessionUpdate::State(state) => EventBody::SessionState(SessionStateEvent {
            state: state.clone(),
        }),
        SessionUpdate::History(records) => EventBody::HistoryUpsert(HistoryUpsertEvent {
            records: records.iter().map(|r| r.0.clone()).collect(),
        }),
        SessionUpdate::Activity(activity) => EventBody::ActivityUpdate(ActivityUpdateEvent {
            activity: activity.clone(),
        }),
        SessionUpdate::Open(request) => EventBody::RequestOpened(RequestOpenedEvent {
            request: request.clone(),
        }),
        SessionUpdate::Close(request_key, reason) => EventBody::RequestClosed(RequestClosedEvent {
            request_key: request_key.clone(),
            reason: *reason,
        }),
        SessionUpdate::SourceGap => EventBody::SyncRequired(SyncRequiredEvent {
            reason: SyncReason::SourceGap,
        }),
    };
    valid(&body)?;
    Ok(serde_json::to_vec(&body)
        .map_err(|_| ErrorCode::ResourceLimit)?
        .len())
}
fn apply(i: &mut Inner, id: &Uuid, mut update: SessionUpdate) -> StateResult<()> {
    if let SessionUpdate::State(state)=&mut update {state.summary.capabilities.terminal_read=false;}
    update_size(&update)?;
    let old=i.sessions.get(id).ok_or(ErrorCode::NotFound)?;
    let invalidate=match &update {
        SessionUpdate::State(value)=>value.summary.connectivity!=SessionConnectivity::Ready || !value.summary.capabilities.prompt || !value.summary.capabilities.skills_read || value.summary.skills_generation!=old.state.summary.skills_generation,
        SessionUpdate::SourceGap=>true,
        _=>false,
    };
    let controls_invalidate=match &update {
        SessionUpdate::State(value)=>value.summary.connectivity!=SessionConnectivity::Ready||!value.summary.capabilities.controls,
        SessionUpdate::SourceGap=>true,_=>false,
    };
    let queue_invalidate=match &update {
        SessionUpdate::State(value)=>value.summary.connectivity!=SessionConnectivity::Ready,
        SessionUpdate::SourceGap=>true,_=>false,
    };
    let retire_agent=match &update {
        SessionUpdate::State(value)=>value.summary.connectivity!=SessionConnectivity::Ready
            ||value.summary.provider!=old.state.summary.provider||value.summary.server_id!=old.state.summary.server_id
            ||value.summary.workspace_id!=old.state.summary.workspace_id||value.summary.pane_id!=old.state.summary.pane_id
            ||value.summary.terminal_id!=old.state.summary.terminal_id,
        SessionUpdate::SourceGap=>true,_=>false,
    };
    if retire_agent {cancel_grids(i,|lease|lease.agent_target().is_some_and(|target|&target.session_id==id));}
    if controls_invalidate {invalidate_controls(i,id);}
    if queue_invalidate {invalidate_queue(i,id);}
    if invalidate {invalidate_skills(i,id);}
    let s = i.sessions.get_mut(id).ok_or(ErrorCode::NotFound)?;
    let changes_history = matches!(&update, SessionUpdate::History(_));
    let mut changes_summary = false;
    let mut identical_state = false;
    let body = match update {
        SessionUpdate::State(mut state) => {
            if s.target()
                != (Target {
                    session_id: state.summary.session_id.clone(),
                    generation: state.summary.generation.clone(),
                })
            {
                return Err(ErrorCode::StaleSession);
            }
            if (s.topology_dirty || s.observation_pending)
                && state.summary.connectivity == SessionConnectivity::Ready
            {
                state.summary.connectivity = SessionConnectivity::Synchronizing;
            }
            s.older |= state.summary.history_coverage.older_available;
            identical_state = state == s.state;
            changes_summary = state.summary != s.state.summary;
            if !identical_state {
                s.state = state.clone();
            }
            EventBody::SessionState(SessionStateEvent { state })
        }
        SessionUpdate::History(records) => {
            let mut changed = vec![];
            for record in records {
                let incoming = record.0;
                let history = Arc::make_mut(&mut s.history);
                let origins = s.origins.entry(incoming.id.clone()).or_default();
                let merged = if let Some(index) = history.iter().position(|r| r.id == incoming.id) {
                    let merged = merge_history(
                        s.state.summary.provider,
                        &history[index],
                        incoming,
                        origins,
                        false,
                    );
                    history.replace(index, merged.clone());
                    merged
                } else {
                    origins.observe(&incoming);
                    history.push(incoming.clone());
                    incoming
                };
                changed.push(merged);
            }
            Arc::make_mut(&mut s.history).sort();
            s.trim();
            EventBody::HistoryUpsert(HistoryUpsertEvent { records: changed })
        }
        SessionUpdate::Activity(activity) => {
            s.state.activity = activity.clone();
            EventBody::ActivityUpdate(ActivityUpdateEvent { activity })
        }
        SessionUpdate::Open(request) => {
            if let Some(old) = s
                .pending
                .iter_mut()
                .find(|r| r.request_key == request.request_key)
            {
                *old = request.clone();
            } else {
                if s.pending.len() == 64 {
                    return Err(ErrorCode::ResourceLimit);
                }
                s.pending.push(request.clone());
            }
            EventBody::RequestOpened(RequestOpenedEvent { request })
        }
        SessionUpdate::Close(request_key, reason) => {
            s.pending.retain(|r| r.request_key != request_key);
            EventBody::RequestClosed(RequestClosedEvent {
                request_key,
                reason,
            })
        }
        SessionUpdate::SourceGap => {
            s.source = Uuid::random();
            s.observation_pending = false;
            s.state.summary.connectivity = SessionConnectivity::Synchronizing;
            s.state.summary.history_coverage.source_truncated = true;
            i.content.retain(|_, p| &p.target.session_id != id);
            i.cursors.retain(|_, c| &c.target.session_id != id);
            i.native_cursors.retain(|_, c| &c.target.session_id != id);
            i.browse.retain(|_, c| &c.target.session_id != id);
            EventBody::SyncRequired(SyncRequiredEvent {
                reason: SyncReason::SourceGap,
            })
        }
    };
    if changes_history {
        trim_history(i);
    }
    if !identical_state {
        publish(i, Some(id), None, body);
    }
    if changes_summary {
        publish(i, None, None, EventBody::HostChanged(HostChangedEvent {}));
    }
    Ok(())
}
fn history_storage_bytes(i: &Inner) -> usize {
    let mut seen = std::collections::HashSet::new();
    i.sessions
        .values()
        .map(|s| &s.history)
        .chain(i.cursors.values().map(|c| &c.records))
        .filter(|records| seen.insert(Arc::as_ptr(records) as usize))
        .map(|records| records.bytes)
        .sum::<usize>()
        + native_cursor_bytes(i)
        + browse::storage_bytes(i)
        + i.adapter_history_bytes
            .values()
            .map(|(_, bytes)| *bytes)
            .sum::<usize>()
}
fn trim_history(i: &mut Inner) {
    trim_history_to_budget(i, SESSION_BYTES);
}
fn trim_history_to_budget(i: &mut Inner, budget: usize) {
    if history_storage_bytes(i) <= budget {
        return;
    }
    // Pinned cursor views share immutable records. If old views cause the host history budget
    // to overflow, expire them explicitly instead of keeping unbounded snapshot copies.
    while history_storage_bytes(i) > budget {
        let Some(view) = i
            .browse
            .values()
            .min_by_key(|n| n.expires)
            .map(|n| n.view.clone())
        else {
            break;
        };
        i.browse.retain(|_, n| n.view != view);
    }
    if history_storage_bytes(i) <= budget {
        return;
    }
    i.cursors.clear();
    i.native_cursors.clear();
    let mut bytes: usize = i.sessions.values().map(|s| s.history.bytes).sum();
    while bytes > budget {
        let Some(session) = i
            .sessions
            .values_mut()
            .filter(|s| !s.history.is_empty())
            .max_by_key(|s| s.history.len())
        else {
            break;
        };
        let mut evicted = 0;
        for record in session.history.iter() {
            if bytes <= budget {
                break;
            }
            bytes -= record_bytes(record);
            evicted += 1;
        }
        for removed in Arc::make_mut(&mut session.history).drain_prefix(evicted) {
            session.origins.remove(&removed.id);
        }
        if evicted > 0 {
            session.older = true;
            session.native_older = None;
            session.state.summary.history_coverage.older_available = true;
        }
    }
}
fn refresh_history_page(
    i: &Inner,
    target: &Target,
    page: &SessionHistoryResult,
) -> StateResult<SessionHistoryResult> {
    let s = current(i, target)?;
    let records = page
        .records
        .iter()
        .map(|record| {
            if let Some(current) = s.history.iter().find(|r| r.id == record.id) {
                let mut origins = s.origins.get(&record.id).cloned().unwrap_or_default();
                merge_history(
                    s.state.summary.provider,
                    current,
                    record.clone(),
                    &mut origins,
                    true,
                )
            } else {
                record.clone()
            }
        })
        .collect();
    let response = Envelope::Response(Response {
        id: Uuid::random(),
        outcome: Ok(ResultBody::SessionHistory(SessionHistoryResult {
            view_id: page.view_id.clone(),
            records,
            next_cursor: page.next_cursor.clone(),
        })),
    });
    frame(&response)?;
    let Envelope::Response(Response {
        outcome: Ok(ResultBody::SessionHistory(page)),
        ..
    }) = response
    else {
        unreachable!()
    };
    Ok(page)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SequenceDecision {
    Apply,
    Duplicate,
    Resnapshot,
}
#[derive(Default)]
pub struct ClientSequence {
    subscription: Option<Uuid>,
    applied: u64,
    ready: bool,
}
impl ClientSequence {
    pub fn snapshot(&mut self, subscription: Uuid) {
        self.subscription = Some(subscription);
        self.applied = 0;
        self.ready = true;
    }
    pub fn disconnected(&mut self) {
        self.ready = false;
    }
    pub fn mutations_enabled(&self) -> bool {
        self.ready
    }
    pub fn event(&mut self, event: &Event) -> SequenceDecision {
        if self.subscription.as_ref() != Some(&event.subscription)
            || !self.ready
            || event.seq.0 == 0
        {
            self.ready = false;
            return SequenceDecision::Resnapshot;
        }
        if event.seq.0 <= self.applied {
            return SequenceDecision::Duplicate;
        }
        if event.seq.0 != self.applied + 1 || matches!(event.body, EventBody::SyncRequired(_)) {
            self.ready = false;
            return SequenceDecision::Resnapshot;
        }
        self.applied = event.seq.0;
        SequenceDecision::Apply
    }
}
fn same_binding(a: &SessionSummary, b: &SessionSummary) -> bool {
    a.session_id == b.session_id
        && a.server_id == b.server_id
        && a.terminal_id == b.terminal_id
        && a.provider == b.provider
        && a.native_id == b.native_id
}
fn update_locked(i: &mut Inner, target: &Target, update: SessionUpdate) -> StateResult<()> {
    let s = i
        .sessions
        .get_mut(&target.session_id)
        .ok_or(ErrorCode::NotFound)?;
    if s.target() != *target {
        return Err(ErrorCode::StaleSession);
    }
    if let SessionUpdate::State(value) = &update
        && (!same_binding(&s.state.summary, &value.summary)
            || value.summary.generation != target.generation)
    {
        return Err(ErrorCode::StaleSession);
    }
    if let SessionUpdate::Open(request) = &update
        && request.provider != s.state.summary.provider
    {
        return Err(ErrorCode::InvalidRequest);
    }
    if s.installing {
        if s.install_failed {
            return Err(ErrorCode::ResourceLimit);
        }
        let size = update_size(&update)?;
        if s.native_queue.len() == OUTPUT_EVENTS || s.native_bytes + size > OUTPUT_BYTES {
            s.native_queue.clear();
            s.native_bytes = 0;
            s.install_failed = true;
            s.state.summary.connectivity = SessionConnectivity::Synchronizing;
            return Err(ErrorCode::ResourceLimit);
        }
        s.native_bytes += size;
        s.native_queue.push_back(update);
        Ok(())
    } else {
        apply(i, &target.session_id, update)
    }
}

#[cfg(test)]
impl State {
    pub(crate) fn history_storage_bytes_for_test(&self) -> usize {
        history_storage_bytes(&self.inner.lock().expect("state mutex"))
    }
}


impl State {
    pub(crate) fn validate_shell(&self,target:&ShellTarget,source:&Uuid,access:crate::herdr::shells::ShellAccess)->StateResult<()> {
        let inner=self.inner.lock().expect("state mutex");
        let (shell,epoch)=inner.shells.get(&target.shell_id).filter(|(shell,_)|shell.target==*target).ok_or(ErrorCode::StaleSession)?;
        if epoch!=source {return Err(ErrorCode::StaleSession)}
        if shell.connectivity!=SessionConnectivity::Ready||!shell.capabilities.read||access==crate::herdr::shells::ShellAccess::Input&&!shell.capabilities.input {return Err(ErrorCode::NotReady)}Ok(())
    }
}


impl State {
    pub(crate) fn creation_shell_fence(&self,peer:Uuid,action:Uuid,fence:Option<crate::herdr::shells::CreationShellFence>) {
        let mut inner=self.inner.lock().expect("state mutex");
        if let Some(fence)=fence{inner.creation_shell_fences.insert((peer,action),fence);}else{inner.creation_shell_fences.remove(&(peer,action));}
    }
    pub(crate) fn has_creation_shell_fences(&self,server:&Uuid)->bool {
        self.inner.lock().expect("state mutex").creation_shell_fences.values().any(|fence|&fence.server==server)
    }
    pub(crate) fn creation_shell_blocked(&self,server:&Uuid,incarnation:&Uuid,pane:&Id,terminal:&Id)->bool {
        self.inner.lock().expect("state mutex").creation_shell_fences.values().any(|fence|fence.blocks(server,incarnation,pane,terminal))
    }
}

impl State {
    pub(crate) fn validate_attachment_mutation(&self,request:&Request)->StateResult<()> {
        request.body.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        if matches!(request.body,RequestBody::AttachmentsDiscard(_)){return Ok(())}
        if request.body.attachment_action().is_none(){return Err(ErrorCode::InvalidRequest)}
        self.validate_session_mutation(request)
    }
}
impl State {
    pub(crate) fn attachments_status_response(&self,connection:&Uuid,id:Uuid,mut status:AttachmentsStatusResult,original:Option<(&Target,&Uuid)>)->StateResult<()> {
        let mut inner=self.inner.lock().expect("state mutex");
        if status.insertion_ready {let ready=original.is_some_and(|(target,source)|current(&inner,target).is_ok_and(|session|&session.source==source&&session.state.summary.connectivity==SessionConnectivity::Ready&&session.state.summary.capabilities.attachments&&!session.installing&&!session.install_failed&&!session.observation_pending));if !ready {status.insertion_ready=false}}
        let bytes=frame(&Envelope::Response(Response {id,outcome:Ok(ResultBody::AttachmentsStatus(status))}))?;
        let output=inner.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;output.flush();if !output.enqueue(bytes,false){output.closed=true;return Err(ErrorCode::ResourceLimit)}Ok(())
    }
}

impl State{
 pub(crate) fn validate_artifact_source(&self,source:&ArtifactSource,epoch:&Uuid)->StateResult<()>{
  let inner=self.inner.lock().expect("state mutex");match source{
   ArtifactSource::Session{target}=>{let session=current(&inner,target)?;if session.source!=*epoch{return Err(ErrorCode::StaleSession)}
   if session.state.summary.connectivity!=SessionConnectivity::Ready||!session.state.summary.capabilities.artifacts{return Err(ErrorCode::NotReady)}},
   ArtifactSource::Shell{target}=>{let (shell,current)=inner.shells.get(&target.shell_id).filter(|(s,_)|s.target==*target).ok_or(ErrorCode::StaleSession)?;if current!=epoch{return Err(ErrorCode::StaleSession)}
   if shell.connectivity!=SessionConnectivity::Ready||!shell.capabilities.artifacts{return Err(ErrorCode::NotReady)}}
  }Ok(())
 }
 pub(crate) fn artifact_response(&self,connection:&Uuid,id:Uuid,source:&ArtifactSource,epoch:&Uuid,result:ResultBody)->StateResult<()>{
  let bytes=frame(&Envelope::Response(Response{id,outcome:Ok(result)}))?;let mut inner=self.inner.lock().expect("state mutex");
  match source{ArtifactSource::Session{target}=>{let session=current(&inner,target)?;if session.source!=*epoch{return Err(ErrorCode::StaleSession)}
  if session.state.summary.connectivity!=SessionConnectivity::Ready||!session.state.summary.capabilities.artifacts{return Err(ErrorCode::NotReady)}},ArtifactSource::Shell{target}=>{let (shell,current)=inner.shells.get(&target.shell_id).filter(|(shell,_)|shell.target==*target).ok_or(ErrorCode::StaleSession)?;if current!=epoch{return Err(ErrorCode::StaleSession)}
  if shell.connectivity!=SessionConnectivity::Ready||!shell.capabilities.artifacts{return Err(ErrorCode::NotReady)}}}
  let c=inner.connections.get_mut(connection).ok_or(ErrorCode::Unauthorized)?;c.flush();if !c.enqueue(bytes,false){c.closed=true;return Err(ErrorCode::ResourceLimit)}Ok(())
 }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PairedDevice, Storage};
    use serde_json::json;

    fn serialized_history_bytes(records: &[HistoryRecord]) -> usize {
        records
            .iter()
            .map(|record| serde_json::to_vec(record).unwrap().len())
            .sum()
    }

    fn assert_history_accounting(inner: &Inner) {
        let mut seen = HashSet::new();
        let mut total = 0;
        for snapshot in inner
            .sessions
            .values()
            .map(|s| &s.history)
            .chain(inner.cursors.values().map(|c| &c.records))
        {
            assert_eq!(snapshot.bytes, serialized_history_bytes(snapshot));
            if seen.insert(Arc::as_ptr(snapshot) as usize) {
                total += snapshot.bytes;
            }
        }
        for cursor in inner.native_cursors.values() {
            let pages: usize = cursor
                .pages
                .values()
                .map(|page| serialized_history_bytes(&page.records))
                .sum();
            assert_eq!(cursor.page_bytes, pages);
            total += cursor.native.0.len() + 256 + pages;
        }
        assert_eq!(history_storage_bytes(inner), total);
    }

    fn paging_state() -> (State, Target, Uuid, AuthenticatedPeer, AuthenticatedPeer) {
        let state = State::default();
        let value: SessionState = serde_json::from_value(json!({
            "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":Uuid::random(),
                "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp",
                "nativeId":"native","title":"test","status":"idle","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                    "promptMode":"submit_or_steer","cancelMode":"current_work"},
                "historyCoverage":{"scope":"all_branches","source":"persisted_and_live",
                    "olderAvailable":true,"sourceTruncated":false,"desktopBranchKnown":false}},
            "desktopSelection":"unknown","activity":{"state":"idle"}
        })).unwrap();
        let target = Target {
            session_id: value.summary.session_id.clone(),
            generation: value.summary.generation.clone(),
        };
        let epoch = state.begin_install(value.clone()).unwrap();
        let records = (0..4).map(|n| SafeHistory::from_redacted_display(serde_json::from_value(json!({
            "id":format!("record-{n}"),"order":n,"role":"assistant","kind":"message",
            "text":{"kind":"text","text":"retained"},"source":"live","complete":true,"sourceTruncated":false
        })).unwrap()).unwrap()).collect();
        state
            .finish_install(
                &epoch,
                ProviderSnapshot {
                    state: value,
                    pending: vec![],
                    history: records,
                    older_available: true,
                    native_older: None,
                },
            )
            .unwrap();
        let mut storage = Storage::memory([1; 32], 0).unwrap();
        let mut peer = |key| {
            let device = PairedDevice {
                device_id: Uuid::random(),
                endpoint_id: EndpointId(
                    iroh::SecretKey::from_bytes(&[key; 32]).public().to_string(),
                ),
                device_name: Label::new("test phone").unwrap(),
                paired_at: Uint(0),
                last_seen_at: None,
            };
            let authorization = storage.pair_device(&device).unwrap();
            storage
                .authenticated_hello(&device.device_id, authorization)
                .unwrap()
        };
        (state, target, epoch, peer(61), peer(62))
    }
    #[test]
    fn identical_state_is_compared_after_native_and_topology_gates() {
        for native in [true,false] {
            let (state,target,epoch,peer,_)=paging_state();
            let connection=state.connect(&peer);
            state.session_open(&connection,Uuid::random(),&target,0).unwrap();
            state.pop_output(&connection).unwrap();
            let ready=state.session_state(&target,&epoch).unwrap();
            if native {state.observation_pending(&target,&epoch).unwrap();}
            else {state.topology_synchronizing(&ready.summary.server_id).unwrap();}
            let Envelope::Event(event)=decode_payload(&state.pop_output(&connection).unwrap().0.unwrap(),None).unwrap() else {panic!("gate loss must publish immediately")};
            let EventBody::SessionState(value)=event.body else {panic!("gate loss must carry effective state")};
            assert_eq!(value.state.summary.connectivity,SessionConnectivity::Synchronizing);
            state.update_from_source(&target,&epoch,SessionUpdate::State(ready.clone())).unwrap();
            assert!(state.pop_output(&connection).unwrap().0.is_none(),"raw Ready cannot cause a duplicate effective Synchronizing event");
            if native {state.observation_revalidated(&target,&epoch).unwrap();}
            else {state.topology_revalidated(&target,&epoch,&ready.summary.workspace_id,&ready.summary.pane_id,||Ok(())).unwrap();}
            let Envelope::Event(event)=decode_payload(&state.pop_output(&connection).unwrap().0.unwrap(),None).unwrap() else {panic!("recovery must publish immediately")};
            let EventBody::SessionState(value)=event.body else {panic!("recovery must carry Ready state")};
            assert_eq!(value.state.summary.connectivity,SessionConnectivity::Ready);
        }
    }

    fn browse_rows(start: u64, count: u64) -> Vec<SafeHistory> {
        (start..start+count).map(|n|SafeHistory::from_redacted_display(serde_json::from_value(json!({"id":format!("native-{n}"),"order":n,"role":"assistant","kind":"message","text":{"kind":"text","text":"captured native"},"source":"native_page","complete":true,"sourceTruncated":false})).unwrap()).unwrap()).collect()
    }
    #[test]
    fn browse_native_membership_is_immutable_and_separate_from_live_order() {
        let (state, target, source, peer, foreign) = paging_state();
        let live = state.inner.lock().unwrap().sessions[&target.session_id]
            .history
            .records
            .clone();
        let ticket = state
            .begin_browse(
                &peer,
                &target,
                &source,
                BrowsePosition::new("head", 64).unwrap(),
                2,
                0,
            )
            .unwrap();
        let root = ticket.cursor();
        let view = ticket.view_id().clone();
        let next = BrowsePosition::new("native opaque successor", 64).unwrap();
        let first = state
            .finish_browse(ticket, browse_rows(10, 4), Some(next), 1)
            .unwrap();
        assert_eq!(
            first
                .records
                .iter()
                .map(|r| r.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["native-12", "native-13"]
        );
        assert_eq!(
            state.inner.lock().unwrap().sessions[&target.session_id]
                .history
                .records,
            live
        );
        let newer=SafeHistory::from_redacted_display(serde_json::from_value(json!({"id":"native-12","order":1000,"role":"assistant","kind":"message","text":{"kind":"text","text":"later live overlay"},"source":"live","complete":true,"sourceTruncated":false})).unwrap()).unwrap();
        state
            .history_from_source(&target, &source, vec![newer])
            .unwrap();
        let BrowseLookup::Cached(repeated) = state
            .browse_page(&peer, &target, &view, &root, 2, 2)
            .unwrap()
        else {
            panic!("memoized page")
        };
        assert_eq!(first, repeated);
        assert!(matches!(
            state.browse_page(&foreign, &target, &view, &root, 2, 2),
            Err(ErrorCode::CursorExpired)
        ));
        assert!(matches!(
            state.browse_page(&peer, &target, &Uuid::random(), &root, 2, 2),
            Err(ErrorCode::CursorExpired)
        ));
        let BrowseLookup::Cached(wide) = state
            .browse_page(&peer, &target, &view, &root, 100, 3)
            .unwrap()
        else {
            panic!("captured native page")
        };
        assert_eq!(wide.records.len(), 4);
        let BrowseLookup::Cached(tail) = state
            .browse_page(
                &peer,
                &target,
                &view,
                first.next_cursor.as_ref().unwrap(),
                2,
                3,
            )
            .unwrap()
        else {
            panic!("local slice")
        };
        assert_eq!(tail.records.len(), 2);
        assert_eq!(tail.next_cursor, wide.next_cursor);
        assert!(matches!(
            state.browse_page(&peer, &target, &view, &root, 2, CONTENT_TTL_MS),
            Err(ErrorCode::CursorExpired)
        ));
    }
    #[test]
    fn browse_continues_past_thousand_with_split_pages_stable_repeats_and_terminal_native_cursor() {
        let (state, target, source, peer, _) = paging_state();
        let ticket = state
            .begin_browse(
                &peer,
                &target,
                &source,
                BrowsePosition::new(0usize, 64).unwrap(),
                37,
                0,
            )
            .unwrap();
        let view = ticket.view_id().clone();
        let mut page = state
            .finish_browse(
                ticket,
                browse_rows(900, 300),
                Some(BrowsePosition::new(1usize, 64).unwrap()),
                1,
            )
            .unwrap();
        let mut ids = HashSet::new();
        let mut advances = 1;
        loop {
            for row in &page.records {
                assert!(ids.insert(row.id.clone()));
            }
            let Some(cursor) = page.next_cursor.clone() else {
                break;
            };
            let lookup = state
                .browse_page(&peer, &target, &view, &cursor, 37, 2)
                .unwrap();
            page = match lookup {
                BrowseLookup::Cached(page) => page,
                BrowseLookup::Fetch(ticket) => {
                    let batch = *ticket.position::<usize>().unwrap();
                    advances += 1;
                    let start = 900 - batch as u64 * 300;
                    state
                        .finish_browse(
                            ticket,
                            browse_rows(start, 300),
                            (batch < 3).then(|| BrowsePosition::new(batch + 1, 64).unwrap()),
                            2,
                        )
                        .unwrap()
                }
            };
            let BrowseLookup::Cached(repeated) = state
                .browse_page(&peer, &target, &view, &cursor, 37, 3)
                .unwrap()
            else {
                panic!("repeat fetched native source")
            };
            assert_eq!(page, repeated);
        }
        assert_eq!(advances, 4, "all split native pages must remain reachable");
        assert_eq!(ids.len(), 1_200);
        assert!(ids.contains(&Id("native-0".into())));
        assert!(ids.contains(&Id("native-1199".into())));
        assert!(page.next_cursor.is_none());
    }
    #[test]
    fn browse_empty_progress_and_transactional_capacity_preserve_original_ticket() {
        let (state, target, source, peer, _) = paging_state();
        let ticket = state
            .begin_browse(
                &peer,
                &target,
                &source,
                BrowsePosition::new("head", 64).unwrap(),
                100,
                0,
            )
            .unwrap();
        let retry = ticket.clone();
        let view = ticket.view_id().clone();
        let huge = BrowsePosition::new("too large next position", SESSION_BYTES).unwrap();
        assert!(matches!(
            state.finish_browse(ticket, vec![], Some(huge), 1),
            Err(ErrorCode::ResourceLimit)
        ));
        assert_eq!(state.inner.lock().unwrap().browse.len(), 1);
        assert!(matches!(
            state
                .browse_page(&peer, &target, &view, &retry.cursor(), 100, 1)
                .unwrap(),
            BrowseLookup::Fetch(_)
        ));
        let empty = state
            .finish_browse(
                retry,
                vec![],
                Some(BrowsePosition::new("next raw page", 64).unwrap()),
                2,
            )
            .unwrap();
        assert!(empty.records.is_empty());
        assert!(empty.next_cursor.is_some());
        let BrowseLookup::Fetch(next) = state
            .browse_page(
                &peer,
                &target,
                &view,
                empty.next_cursor.as_ref().unwrap(),
                100,
                3,
            )
            .unwrap()
        else {
            panic!("empty native successor")
        };
        assert_eq!(*next.position::<&str>().unwrap(), "next raw page");
        assert!(matches!(
            state.finish_browse(next, vec![], None, CONTENT_TTL_MS),
            Err(ErrorCode::CursorExpired)
        ));
    }
    #[test]
    fn browse_capacity_evicts_one_old_view_without_clearing_healthy_views() {
        let (state, target, source, peer, _) = paging_state();
        let mut roots = Vec::new();
        for time in 0..65 {
            roots.push(
                state
                    .begin_browse(
                        &peer,
                        &target,
                        &source,
                        BrowsePosition::new(time, 64).unwrap(),
                        100,
                        time,
                    )
                    .unwrap(),
            );
        }
        assert_eq!(state.inner.lock().unwrap().browse.len(), 64);
        assert!(matches!(
            state.browse_page(
                &peer,
                &target,
                roots[0].view_id(),
                &roots[0].cursor(),
                100,
                65
            ),
            Err(ErrorCode::CursorExpired)
        ));
        for ticket in &roots[1..] {
            assert!(matches!(
                state
                    .browse_page(&peer, &target, ticket.view_id(), &ticket.cursor(), 100, 65)
                    .unwrap(),
                BrowseLookup::Fetch(_)
            ));
        }
    }
    #[tokio::test]
    async fn browse_concurrent_repeats_fetch_once_and_reject_source_replacement() {
        let (state, target, source, peer, _) = paging_state();
        let state = Arc::new(state);
        let ticket = state
            .begin_browse(
                &peer,
                &target,
                &source,
                BrowsePosition::new("original position", 64).unwrap(),
                100,
                0,
            )
            .unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let read = |ticket: BrowseTicket| {
            let state = state.clone();
            let calls = calls.clone();
            let peer = peer.clone();
            async move {
                let gate = ticket.gate();
                let _guard = gate.lock().await;
                match state
                    .browse_page(
                        &peer,
                        ticket.target(),
                        ticket.view_id(),
                        &ticket.cursor(),
                        100,
                        1,
                    )
                    .unwrap()
                {
                    BrowseLookup::Cached(page) => page,
                    BrowseLookup::Fetch(ticket) => {
                        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tokio::task::yield_now().await;
                        state
                            .finish_browse(ticket, browse_rows(10, 3), None, 2)
                            .unwrap()
                    }
                }
            }
        };
        let (a, b) = tokio::join!(read(ticket.clone()), read(ticket));
        assert_eq!(a, b);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let ticket = state
            .begin_browse(
                &peer,
                &target,
                &source,
                BrowsePosition::new("old source", 64).unwrap(),
                100,
                3,
            )
            .unwrap();
        {
            state
                .inner
                .lock()
                .unwrap()
                .sessions
                .get_mut(&target.session_id)
                .unwrap()
                .source = Uuid::random();
        }
        assert!(matches!(
            state.finish_browse(ticket, browse_rows(20, 1), None, 4),
            Err(ErrorCode::CursorExpired)
        ));
    }
    #[test]
    fn native_handoff_pins_first_boundary_per_peer_view_and_expires_old_source() {
        let (state, target, epoch, a, b) = paging_state();
        let cursor_a = state
            .history(&a, &target, None, 1, 0)
            .unwrap()
            .result
            .next_cursor
            .unwrap();
        let cursor_b = state
            .history(&b, &target, None, 1, 0)
            .unwrap()
            .result
            .next_cursor
            .unwrap();
        let old_a = state
            .history(&a, &target, Some(&cursor_a), 100, 0)
            .unwrap()
            .refetch_older
            .unwrap();
        let old_b = state
            .history(&b, &target, Some(&cursor_b), 100, 0)
            .unwrap()
            .refetch_older
            .unwrap();
        let first = state
            .native_history_cursor(
                &a,
                &target,
                &epoch,
                Id("native-first".into()),
                Some(&old_a),
                0,
            )
            .unwrap();
        assert_eq!(
            state.native_history_cursor(&b, &target, &epoch, Id("foreign".into()), Some(&old_a), 0),
            Err(ErrorCode::CursorExpired)
        );
        let other = state
            .native_history_cursor(
                &b,
                &target,
                &epoch,
                Id("native-other-peer".into()),
                Some(&old_b),
                0,
            )
            .unwrap();
        assert_ne!(first, other);
        let record = SafeHistory::from_redacted_display(serde_json::from_value(json!({
            "id":"new-live","order":4,"role":"assistant","kind":"message","text":{"kind":"text","text":"new"},
            "source":"live","complete":true,"sourceTruncated":false
        })).unwrap()).unwrap();
        state
            .history_from_source(&target, &epoch, vec![record])
            .unwrap();
        let repeated = state.history(&a, &target, Some(&cursor_a), 100, 0).unwrap();
        assert_eq!(repeated.result.records.len(), 3);
        assert_eq!(
            repeated.refetch_older.as_ref().unwrap().oldest,
            old_a.oldest
        );
        assert_eq!(
            state
                .native_history_cursor(
                    &a,
                    &target,
                    &epoch,
                    Id("new-random-provider-token".into()),
                    repeated.refetch_older.as_ref(),
                    0
                )
                .unwrap(),
            first
        );
        let NativePageLookup::Fetch(ticket) = state
            .native_history_page(&a, &target, &first, 100, 0)
            .unwrap()
        else {
            panic!("first provider read expected")
        };
        assert_eq!(ticket.native, Id("native-first".into()));
        assert_eq!(
            state.native_history_cursor(
                &a,
                &target,
                &epoch,
                Id("late".into()),
                Some(&old_a),
                CONTENT_TTL_MS
            ),
            Err(ErrorCode::CursorExpired)
        );
        let replacement = state.begin_source_install(&target).unwrap();
        assert_ne!(replacement, epoch);
        assert!(
            state
                .native_history_cursor(&a, &target, &epoch, Id("stale".into()), Some(&old_a), 0)
                .is_err()
        );
        assert_eq!(
            state.finish_native_history_page(ticket, vec![], None, 0),
            Err(ErrorCode::CursorExpired)
        );
    }
    #[test]
    fn global_history_prefix_eviction_clears_native_boundary_and_rebases_without_gaps() {
        let (state, target, epoch, peer, _) = paging_state();
        {
            let mut inner = state.inner.lock().unwrap();
            inner
                .sessions
                .get_mut(&target.session_id)
                .unwrap()
                .native_older = Some(Id("old-native-boundary".into()));
        }
        let cursor = state
            .history(&peer, &target, None, 1, 0)
            .unwrap()
            .result
            .next_cursor
            .unwrap();
        let pinned = {
            let mut inner = state.inner.lock().unwrap();
            let pinned = inner.cursors[&Uuid(cursor.0.clone())].records.clone();
            let budget = pinned.iter().skip(1).map(record_bytes).sum();
            trim_history_to_budget(&mut inner, budget);
            let current = &inner.sessions[&target.session_id];
            assert!(current.native_older.is_none());
            assert!(current.older && current.state.summary.history_coverage.older_available);
            assert_eq!(current.history.len(), 3);
            assert_eq!(current.history[0].id, Id("record-1".into()));
            assert!(!Arc::ptr_eq(&pinned, &current.history));
            assert_history_accounting(&inner);
            assert_eq!(pinned.bytes, serialized_history_bytes(&pinned));
            pinned
        };
        assert_eq!(pinned.len(), 4);
        assert!(
            state
                .history(&peer, &target, Some(&cursor), 100, 0)
                .is_err()
        );
        let fresh = state.history(&peer, &target, None, 100, 0).unwrap();
        let boundary = fresh.refetch_older.unwrap();
        assert_eq!(boundary.oldest, Some((Id("record-1".into()), None, 1)));
        assert_eq!(
            fresh
                .result
                .records
                .iter()
                .map(|r| r.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["record-1", "record-2", "record-3"]
        );
        let next = state
            .native_history_cursor(
                &peer,
                &target,
                &epoch,
                Id("before-record-1".into()),
                Some(&boundary),
                0,
            )
            .unwrap();
        let NativePageLookup::Fetch(ticket) = state
            .native_history_page(&peer, &target, &next, 100, 0)
            .unwrap()
        else {
            panic!("recomputed native read expected")
        };
        assert_eq!(ticket.native, Id("before-record-1".into()));
        let missing = SafeHistory::from_redacted_display(pinned[0].clone()).unwrap();
        let older = state
            .finish_native_history_page(ticket, vec![missing], None, 0)
            .unwrap();
        assert_eq!(older.records[0].id, Id("record-0".into()));
        assert_history_accounting(&state.inner.lock().unwrap());
        assert!(matches!(
            state
                .native_history_page(&peer, &target, &next, 100, 0)
                .unwrap(),
            NativePageLookup::Cached(_)
        ));
        assert_history_accounting(&state.inner.lock().unwrap());
    }

    #[test]
    fn native_page_admission_counts_page_and_only_new_successor_before_insertion() {
        for reuse in [false, true] {
            let (state, target, epoch, peer, _) = paging_state();
            let native = Id("next-native-page".into());
            let (first, existing, record, original_bytes, original_cursors) = {
                let mut inner = state.inner.lock().unwrap();
                let first = native_cursor(
                    &mut inner,
                    peer.peer_id().clone(),
                    target.clone(),
                    epoch.clone(),
                    Id("first-native-page".into()),
                    0,
                )
                .unwrap();
                let existing = reuse.then(|| {
                    native_cursor(
                        &mut inner,
                        peer.peer_id().clone(),
                        target.clone(),
                        epoch.clone(),
                        native.clone(),
                        0,
                    )
                    .unwrap()
                });
                let record = SafeHistory::from_redacted_display(
                    inner.sessions[&target.session_id].history[0].clone(),
                )
                .unwrap();
                (
                    first,
                    existing,
                    record,
                    history_storage_bytes(&inner),
                    inner.native_cursors.len(),
                )
            };
            let page_bytes = serde_json::to_vec(record.record()).unwrap().len();
            let overhead = if reuse { 0 } else { native.0.len() + 256 };
            let exact_budget = original_bytes + page_bytes + overhead;
            if !reuse {
                assert!(original_bytes + page_bytes < exact_budget);
                assert!(original_bytes + overhead < exact_budget);
            }
            let fetch = || {
                let NativePageLookup::Fetch(ticket) = state
                    .native_history_page(&peer, &target, &first, 100, 0)
                    .unwrap()
                else {
                    panic!("rejected completion must leave the original page uncached")
                };
                ticket
            };
            assert_eq!(
                state.finish_native_history_page_with_budget(
                    fetch(),
                    vec![record.clone()],
                    Some(native.clone()),
                    0,
                    exact_budget - 1
                ),
                Err(ErrorCode::ResourceLimit)
            );
            {
                let inner = state.inner.lock().unwrap();
                assert_eq!(history_storage_bytes(&inner), original_bytes);
                assert_eq!(inner.native_cursors.len(), original_cursors);
                assert!(inner.cursors.is_empty());
                assert!(
                    inner.native_cursors[&Uuid(first.0.clone())]
                        .pages
                        .is_empty()
                );
                assert_history_accounting(&inner);
            }
            let result = state
                .finish_native_history_page_with_budget(
                    fetch(),
                    vec![record],
                    Some(native),
                    0,
                    exact_budget,
                )
                .unwrap();
            assert_eq!(result.records.len(), 1);
            let successor = result.next_cursor.unwrap();
            if let Some(existing) = existing {
                assert_eq!(successor, existing);
            }
            let inner = state.inner.lock().unwrap();
            assert_eq!(history_storage_bytes(&inner), exact_budget);
            assert_eq!(
                inner.native_cursors.len(),
                original_cursors + usize::from(!reuse)
            );
            assert!(inner.native_cursors.contains_key(&Uuid(successor.0)));
            assert_history_accounting(&inner);
        }
    }

    #[test]
    fn history_snapshot_bytes_follow_upserts_without_changing_pinned_views() {
        let (state, target, epoch, peer, other_peer) = paging_state();
        let first = state
            .history(&peer, &target, None, 1, 0)
            .unwrap()
            .result
            .next_cursor
            .unwrap();
        state.history(&other_peer, &target, None, 1, 0).unwrap();
        let pinned = {
            let inner = state.inner.lock().unwrap();
            assert_history_accounting(&inner);
            let pinned = inner.cursors[&Uuid(first.0.clone())].records.clone();
            assert_eq!(history_storage_bytes(&inner), pinned.bytes);
            pinned
        };
        let original = serde_json::to_vec(&pinned.records).unwrap();
        for (id, order, text) in [
            ("record-4", 4, "inserted"),
            ("record-1", 1, r#"longer "escaped" text"#),
            ("record-1", 1, "x"),
        ] {
            let record = SafeHistory::from_redacted_display(serde_json::from_value(json!({
                "id":id,"order":order,"role":"assistant","kind":"message",
                "text":{"kind":"text","text":text},"source":"live","complete":true,"sourceTruncated":false
            })).unwrap()).unwrap();
            state
                .history_from_source(&target, &epoch, vec![record])
                .unwrap();
            let inner = state.inner.lock().unwrap();
            assert_history_accounting(&inner);
            assert!(!Arc::ptr_eq(
                &pinned,
                &inner.sessions[&target.session_id].history
            ));
            assert_eq!(serde_json::to_vec(&pinned.records).unwrap(), original);
        }
        let repeated = state.history(&peer, &target, Some(&first), 100, 0).unwrap();
        assert_eq!(
            repeated
                .result
                .records
                .iter()
                .map(|r| &r.id)
                .collect::<Vec<_>>(),
            pinned[..3].iter().map(|r| &r.id).collect::<Vec<_>>()
        );
        let mut inner = state.inner.lock().unwrap();
        let budget = inner.sessions[&target.session_id].history.bytes;
        trim_history_to_budget(&mut inner, budget);
        assert!(inner.cursors.is_empty());
        assert_eq!(inner.sessions[&target.session_id].history.len(), 5);
        assert_history_accounting(&inner);
    }

    #[test]
    fn history_snapshot_bytes_follow_session_record_limit() {
        let (state, target, _, _, _) = paging_state();
        let mut inner = state.inner.lock().unwrap();
        let session = inner.sessions.get_mut(&target.session_id).unwrap();
        let template = session.history[0].clone();
        let records = (0..HISTORY_RECORDS + 2)
            .map(|n| {
                let mut record = template.clone();
                record.id = Id(format!("row-{n}"));
                record.order = Uint(n as u64);
                record
            })
            .collect();
        session.history = Arc::new(HistorySnapshot::new(records));
        let pinned = session.history.clone();
        session.trim();
        assert_eq!(session.history.len(), HISTORY_RECORDS);
        assert_eq!(session.history[0].id, Id("row-2".into()));
        assert_eq!(pinned.bytes, serialized_history_bytes(&pinned));
        assert_eq!(pinned.len(), HISTORY_RECORDS + 2);
        assert_history_accounting(&inner);
    }

    #[test]
    fn omp_reservation_intent_is_dirty_source_scoped_and_does_not_enable_dispatch() {
        for provider in [Provider::Omp, Provider::Codex] {
            let state = State::default();
            let value: SessionState = serde_json::from_value(json!({
                "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":Uuid::random(),
                    "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":provider,
                    "nativeId":"native","title":"test","status":"idle","connectivity":"ready",
                    "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                        "promptMode":"submit_or_steer","cancelMode":if provider == Provider::Omp { "current_work" } else { "native_turn" }},
                    "historyCoverage":{"scope":"all_branches","source":"persisted_and_live",
                        "olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
                "desktopSelection":"unknown","activity":{"state":"idle"}
            })).unwrap();
            let target = Target {
                session_id: value.summary.session_id.clone(),
                generation: value.summary.generation.clone(),
            };
            let request = Request {
                id: Uuid::random(),
                body: RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: Uuid::random(),
                target: target.clone(),
                text: PromptText::new("original intent").unwrap(), }),
            };
            let epoch = state.begin_install(value.clone()).unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_installing", ErrorCode::NotReady))
            );
            state.abort_install(&target, &epoch).unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_install_failed", ErrorCode::NotReady))
            );
            let epoch = state.begin_source_install(&target).unwrap();
            state
                .finish_install(
                    &epoch,
                    ProviderSnapshot {
                        state: value.clone(),
                        pending: vec![],
                        history: vec![],
                        older_available: false,
                        native_older: None,
                    },
                )
                .unwrap();
            state.observation_pending(&target, &epoch).unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_observation_pending", ErrorCode::NotReady))
            );
            state
                .topology_synchronizing(&value.summary.server_id)
                .unwrap();
            state
                .topology_revalidated(
                    &target,
                    &epoch,
                    &value.summary.workspace_id,
                    &value.summary.pane_id,
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_observation_pending", ErrorCode::NotReady))
            );
            state.observation_revalidated(&target, &epoch).unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Ok(SessionConnectivity::Ready)
            );
            state
                .topology_synchronizing(&value.summary.server_id)
                .unwrap();
            assert_eq!(state.source_epoch(&target).unwrap(), epoch);
            state.observation_pending(&target, &epoch).unwrap();
            state
                .topology_revalidated(
                    &target,
                    &epoch,
                    &value.summary.workspace_id,
                    &value.summary.pane_id,
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_observation_pending", ErrorCode::NotReady))
            );
            state.observation_revalidated(&target, &epoch).unwrap();
            assert_eq!(state.validate_session_mutation(&request), Ok(()));
            // Either observation may arrive first; neither independently opens
            // the mutation gate or replaces the original source/Target.
            state
                .topology_synchronizing(&value.summary.server_id)
                .unwrap();
            state.observation_pending(&target, &epoch).unwrap();
            state.observation_revalidated(&target, &epoch).unwrap();
            assert_eq!(state.source_epoch(&target).unwrap(), epoch);
            assert_eq!(
                state
                    .session_state(&target, &epoch)
                    .unwrap()
                    .summary
                    .generation,
                target.generation
            );
            assert_eq!(
                state.validate_session_mutation_checked(&request, None),
                Err(("core_sync", ErrorCode::NotReady))
            );
            state
                .topology_revalidated(
                    &target,
                    &epoch,
                    &value.summary.workspace_id,
                    &value.summary.pane_id,
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(state.validate_session_mutation(&request), Ok(()));
            state
                .topology_synchronizing(&value.summary.server_id)
                .unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, None),
                Err(("core_sync", ErrorCode::NotReady))
            );
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                if provider == Provider::Omp {
                    Ok(SessionConnectivity::Synchronizing)
                } else {
                    Err(("core_sync_provider", ErrorCode::NotReady))
                }
            );
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&Uuid::random())),
                Err(("core_epoch", ErrorCode::StaleSession))
            );
            if provider == Provider::Omp {
                let mut disabled = value.clone();
                disabled.summary.capabilities.prompt = false;
                state
                    .update_from_source(&target, &epoch, SessionUpdate::State(disabled))
                    .unwrap();
                assert_eq!(
                    state.validate_session_mutation_checked(&request, Some(&epoch)),
                    Err(("cap_prompt", ErrorCode::NotReady))
                );
                state
                    .update_from_source(&target, &epoch, SessionUpdate::State(value.clone()))
                    .unwrap();
            }
            state
                .topology_revalidated(
                    &target,
                    &epoch,
                    &value.summary.workspace_id,
                    &value.summary.pane_id,
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(state.validate_session_mutation(&request), Ok(()));
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Ok(SessionConnectivity::Ready)
            );
            let mut clean_sync = value.clone();
            clean_sync.summary.connectivity = SessionConnectivity::Synchronizing;
            state
                .update_from_source(&target, &epoch, SessionUpdate::State(clean_sync))
                .unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err((
                    if provider == Provider::Omp {
                        "core_sync_clean"
                    } else {
                        "core_sync_provider"
                    },
                    ErrorCode::NotReady
                ))
            );
            state
                .binding_offline(&target, &epoch, "source unavailable")
                .unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_epoch", ErrorCode::StaleSession))
            );
            let current = state.source_epoch(&target).unwrap();
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&current)),
                Err(("core_install_failed", ErrorCode::NotReady))
            );
            assert_eq!(
                state.validate_session_mutation(&request),
                Err(ErrorCode::NotReady)
            );
            assert_eq!(
                state.validate_session_mutation_checked(&request, None),
                Err(("core_offline", ErrorCode::NotReady))
            );
        }
    }

    #[test]
    fn topology_routing_keeps_target_source_loss_blocks_callbacks_and_proven_row_replaces_unbound_at_capacity()
     {
        let state = State::default();
        let value: SessionState = serde_json::from_value(json!({
            "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":Uuid::random(),
                "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp",
                "nativeId":"native","title":"test","status":"idle","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                    "promptMode":"submit_or_steer","cancelMode":"current_work"},
                "historyCoverage":{"scope":"all_branches","source":"persisted_and_live",
                    "olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
            "desktopSelection":"unknown","activity":{"state":"idle"}
        })).unwrap();
        let expected = Target {
            session_id: value.summary.session_id.clone(),
            generation: value.summary.generation.clone(),
        };
        let rows = (0..512)
            .map(|n| UnboundRow {
                row_id: Uuid::random(),
                server_id: value.summary.server_id.clone(),
                workspace_id: value.summary.workspace_id.clone(),
                terminal_id: if n == 0 {
                    value.summary.terminal_id.clone()
                } else {
                    Id(format!("terminal-{n}"))
                },
                pane_id: if n == 0 {
                    value.summary.pane_id.clone()
                } else {
                    Id(format!("pane-{n}"))
                },
                provider: Provider::Omp,
                title: "outside configured projects".into(),
                state: UnboundState::SetupRequired,
                reason: "owner proof missing".into(),
            })
            .collect();
        state
             .replace_inventory(vec![], vec![], vec![], rows, vec![], &[], |_| Ok(()))
            .unwrap();
        let epoch = state.begin_install(value.clone()).unwrap();
        state
            .finish_install(
                &epoch,
                ProviderSnapshot {
                    state: value.clone(),
                    pending: vec![],
                    history: vec![
                        SafeHistory::from_redacted_display(
                            serde_json::from_value(json!({
                                "id":"retained","order":1,"role":"assistant","kind":"message",
                                "text":{"kind":"text","text":"retained history"},"source":"live",
                                "complete":true,"sourceTruncated":false
                            }))
                            .unwrap(),
                        )
                        .unwrap(),
                    ],
                    older_available: false,
                    native_older: None,
                },
            )
            .unwrap();
        let workspace = Id("moved-workspace".into());
        let pane = Id("moved-pane".into());
        // Only the topology owner may update routing. Failed fresh proof must not publish it.
        assert_eq!(
            state.topology_revalidated(&expected, &epoch, &workspace, &pane, || Err(
                ErrorCode::NotReady
            )),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(state.session_state(&expected, &epoch).unwrap(), value);
        state
            .topology_revalidated(&expected, &epoch, &workspace, &pane, || Ok(()))
            .unwrap();
        let mut stale_provider = value.clone();
        stale_provider.summary.status = SessionStatus::Working;
        stale_provider.activity.state = ActivityState::Working;
        state
            .update_from_source(&expected, &epoch, SessionUpdate::State(stale_provider))
            .unwrap();
        let moved = state.session_state(&expected, &epoch).unwrap();
        assert_eq!(moved.summary.session_id, expected.session_id);
        assert_eq!(moved.summary.generation, expected.generation);
        assert_eq!(state.source_epoch(&expected).unwrap(), epoch);
        assert_eq!(moved.summary.workspace_id, workspace);
        assert_eq!(moved.summary.pane_id, pane);
        assert_eq!(moved.summary.status, SessionStatus::Working);
        assert_eq!(moved.activity.state, ActivityState::Working);
        let mut other = value.clone();
        other.summary.session_id = Uuid::random();
        other.summary.generation = Uuid::random();
        other.summary.terminal_id = Id("other-terminal".into());
        assert_eq!(
            state.begin_install(other).unwrap_err(),
            ErrorCode::ResourceLimit
        );
        assert!(state.session_required(&expected.session_id));
        state
            .binding_offline(&expected, &epoch, "terminal/source unavailable")
            .unwrap();
        assert_eq!(
            state.update_from_source(&expected, &epoch, SessionUpdate::State(value)),
            Err(ErrorCode::CursorExpired)
        );
        let fresh = state.source_epoch(&expected).unwrap();
        let offline = state.session_state(&expected, &fresh).unwrap();
        assert_eq!(offline.summary.session_id, expected.session_id);
        assert_eq!(offline.summary.generation, expected.generation);
        assert_eq!(offline.summary.connectivity, SessionConnectivity::Offline);
        assert_eq!(offline.summary.workspace_id, workspace);
        assert_eq!(offline.summary.pane_id, pane);
        let mut storage = Storage::memory([1; 32], 0).unwrap();
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[61; 32]).public().to_string()),
            device_name: Label::new("test phone").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let authorization_epoch = storage.pair_device(&device).unwrap();
        let peer = storage
            .authenticated_hello(&device.device_id, authorization_epoch)
            .unwrap();
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: storage.boot_id.clone(),
            target: expected,
            text: PromptText::new("no effect").unwrap(), }),
        };
        assert_eq!(
            state.validate_session_mutation(&request),
            Err(ErrorCode::NotReady)
        );
        let connection = state.connect(&peer);
        state.host_snapshot(&connection, Uuid::random()).unwrap();
        let (Some(bytes), _) = state.pop_output(&connection).unwrap() else {
            panic!("snapshot missing")
        };
        let Envelope::Response(response) =
            decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
        else {
            panic!("response missing")
        };
        let Ok(ResultBody::HostSnapshot(snapshot)) = response.outcome else {
            panic!("inventory missing")
        };
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.unbound.len(), 511);
    }
}
