//! Ephemeral, peer-scoped artifact authority. No paths or payload enter storage.
use super::*;
use base64::Engine;
use std::time::{Duration,Instant};
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum NativeArtifactKind{Agent{provider:Provider,provider_process:ProcessEvidence},Shell}
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeArtifactScope{pub pane_id:Id,pub terminal_id:Id,pub workspace_id:Id,pub tab_id:Id,pub read_identity:PaneReadIdentity,pub permission_epoch:u64,pub source:NativeArtifactKind}
impl std::fmt::Debug for NativeArtifactScope{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("NativeArtifactScope(redacted)")}}
impl NativeArtifactScope{
 pub(super) fn identity(&self)->WorktreeReadScope{WorktreeReadScope{pane_id:self.pane_id.clone(),terminal_id:self.terminal_id.clone(),workspace_id:self.workspace_id.clone(),tab_id:self.tab_id.clone(),read_identity:self.read_identity.clone()}}
 pub(super) fn validate(&self)->io::Result<()>{for id in [&self.pane_id,&self.terminal_id,&self.workspace_id,&self.tab_id]{id_valid(id)?}self.read_identity.validate()?;if self.permission_epoch==0||self.read_identity.runtime_epoch==0||self.read_identity.binding_epoch==0{return Err(invalid("artifact scope unavailable"))}Ok(())}
}
use crate::project_read::{self,Child,Directory,Location,ProjectReadContext,Root};
use crate::pairing::Authorized;

impl Topology {
    pub(crate) fn project_location(&self,process:&ProcessEvidence,cwd:&Path,kernel:bool,deadline:Instant)->SecurityResult<(Uuid,Arc<Location>)> {
        let project=self.config.projects.iter().filter(|project|project.allow_read_tools&&cwd.strip_prefix(&project.path).is_ok())
            .max_by_key(|project|project.path.components().count()).ok_or(ErrorCode::NotReady)?;
        let root=self.creation_roots.get(&project.id).and_then(Option::as_ref).ok_or(ErrorCode::NotReady)?.clone();
        let location=Location::capture(root,process.clone(),cwd,kernel,deadline).map_err(project_read::filesystem)?;
        Ok((project.id.clone(),location))
    }
    pub(crate) fn project_reads_available(&self,process:&ProcessEvidence,cwd:&Path)->bool {
        self.project_location(process,cwd,false,Instant::now()+Duration::from_secs(5)).is_ok()
    }
    pub(crate) fn approved_read_roots(&self)->Vec<Arc<Root>> {
        self.config.projects.iter().filter(|project|project.allow_read_tools)
            .filter_map(|project|self.creation_roots.get(&project.id).and_then(Option::as_ref).cloned()).collect()
    }
    pub(crate) fn capture_project_context(
        self:&Arc<Self>,origin:project_read::ProjectReadOrigin,process:ProcessEvidence,cwd:&Path,kernel:bool,
    )->SecurityResult<ProjectReadContext> {
        (origin.validate)()?;
        let (project,location)=self.project_location(&process,cwd,kernel,origin.captured+Duration::from_secs(5))?;
        ProjectReadContext::capture(self,origin,project,location)
    }
    pub(crate) fn with_project_context(&self,context:&ProjectReadContext,operation:&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()> {
        let project=self.config.projects.iter().find(|project|&project.id==context.project()&&project.allow_read_tools).ok_or(ErrorCode::NotReady)?;
        if !self.creation_roots.get(&project.id).and_then(Option::as_ref).is_some_and(|root|Arc::ptr_eq(root,&context.location().root)) {
            return Err(ErrorCode::StaleSession);
        }
        match context.source() {
            ArtifactSource::Shell{target}=>self.with_shell_project_source(target,context.epoch(),context.physical(),operation),
            ArtifactSource::Session{target}=>{
                context.physical().recheck_identity().map_err(policy)?;
                let inner=self.inner.lock().expect("topology mutex");
                let current=inner.servers.get(&context.physical().server_id).and_then(|server|server.source.as_ref()).ok_or(ErrorCode::StaleSession)?;
                if !current.same_physical_identity(context.physical()) {return Err(ErrorCode::StaleSession);}
                if !current.recheck_readiness().map_err(policy)? {return Err(ErrorCode::NotReady);}
                let binding=inner.bindings.get(&target.session_id).filter(|binding|binding.target==*target&&&binding.source_epoch==context.epoch()
                    &&binding.key.server_id==current.server_id&&binding.key.server_incarnation==current.incarnation).ok_or(ErrorCode::StaleSession)?;
                let state=self.state.session_state(target,context.epoch())?;
                if state.summary.connectivity!=SessionConnectivity::Ready||state.summary.terminal_id!=binding.key.terminal_id {return Err(ErrorCode::NotReady);}
                operation()
            }
        }
    }
    fn with_shell_project_source(&self,target:&ShellTarget,epoch:&Uuid,physical:&Source,operation:&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()> {
        physical.recheck_identity().map_err(policy)?;
        let inner=self.inner.lock().expect("topology mutex");
        let current=inner.servers.get(&physical.server_id).and_then(|server|server.source.as_ref()).ok_or(ErrorCode::StaleSession)?;
        if !current.same_physical_identity(physical)||!current.recheck_readiness().map_err(policy)? {return Err(ErrorCode::NotReady);}
        let binding=inner.shell_bindings.get(&target.shell_id).filter(|binding|binding.target==*target&&binding.proof.logical&&binding.artifacts
            &&&binding.proof.source_epoch==epoch).ok_or(ErrorCode::StaleSession)?;
        if !binding.source.as_ref().is_some_and(|source|source.same_physical_identity(current)) {return Err(ErrorCode::StaleSession);}
        let route=&binding.proof.identity;let pane=current.snapshot.pane(&route.pane_id).map_err(policy)?;
        let original=physical.snapshot.pane(&route.pane_id).map_err(policy)?;
        if original.terminal_id!=route.terminal_id||original.workspace_id!=route.workspace_id||original.tab_id!=route.tab_id {return Err(ErrorCode::StaleSession);}
        if pane.terminal_id!=route.terminal_id||pane.workspace_id!=route.workspace_id||pane.tab_id!=route.tab_id
            ||!shells::ordinary_shell(&current.snapshot,pane)||self.state.creation_shell_blocked(&current.server_id,&current.incarnation,&route.pane_id,&route.terminal_id) {
            return Err(ErrorCode::StaleSession);
        }
        self.state.validate_artifact_source(&ArtifactSource::Shell{target:target.clone()},epoch)?;
        operation()
    }
    pub(crate) async fn shell_project_context(self:&Arc<Self>,target:&ShellTarget,job:Arc<ReadJob>)->SecurityResult<ProjectReadContext> {
        let captured=Instant::now();let deadline=job.deadline.min(captured+Duration::from_secs(5));job.current()?;
        let (physical,epoch,pane)={
            let inner=self.inner.lock().expect("topology mutex");
            let binding=inner.shell_bindings.get(&target.shell_id).filter(|binding|binding.target==*target&&binding.proof.logical&&binding.artifacts).ok_or(ErrorCode::NotReady)?;
            (binding.source.clone().ok_or(ErrorCode::NotReady)?,binding.proof.source_epoch.clone(),binding.proof.identity.pane_id.clone())
        };
        self.with_shell_project_source(target,&epoch,&physical,&mut ||Ok(()))?;
        let info=tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),async {
            physical.recheck().map_err(policy)?;
            let client = Client::connect_source(&physical).await.map_err(policy)?;
            if client.evidence!=physical.evidence {return Err(ErrorCode::StaleSession);}
            let info=client.process_info(&pane).await.map_err(policy)?;
            physical.recheck().map_err(policy)?;Ok(info)
        }).await.map_err(|_|ErrorCode::Timeout)??;
        let reported=info.shell_pid.ok_or(ErrorCode::NotReady)?;
        drop(info);
        let topology=self.clone();let target=target.clone();
        let worker=tokio::task::spawn_blocking(move||{
            job.current()?;topology.with_shell_project_source(&target,&epoch,&physical,&mut ||Ok(()))?;
            let reporter=process_evidence(physical.evidence.pid).map_err(policy)?;
            let process=reported_process(&reporter,reported).map_err(policy)?;
            if process_evidence(reporter.pid).map_err(policy)?!=reporter {return Err(ErrorCode::StaleSession);}
            let (_,cwd)=project_read::kernel_cwd(&process,deadline).map_err(project_read::filesystem)?;
            let weak=Arc::downgrade(&topology);let selected=target.clone();let original=physical.clone();let source_epoch=epoch.clone();
            let validate:Arc<dyn Fn()->SecurityResult<()>+Send+Sync>=Arc::new(move||{
                weak.upgrade().ok_or(ErrorCode::StaleSession)?.with_shell_project_source(&selected,&source_epoch,&original,&mut ||Ok(()))
            });
            let checked=validate.clone();
            let publish=Arc::new(move|commit:&mut dyn FnMut()->SecurityResult<()>|{checked()?;commit()});
            job.current()?;
            topology.capture_project_context(project_read::ProjectReadOrigin {source:ArtifactSource::Shell{target},epoch,physical,validate,publish,captured},process,&cwd,true)
        });
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),worker).await.map_err(|_|ErrorCode::Timeout)?.map_err(|_|ErrorCode::NotReady)?
    }
}

const METADATA_PAGE:usize=256*1024;
type RefId=uuid::Uuid;
fn reference(value:&Uuid)->SecurityResult<RefId> {RefId::parse_str(&value.0).map_err(|_|ErrorCode::InvalidRequest)}
fn public(value:RefId)->Uuid {Uuid(value.hyphenated().to_string())}

#[derive(Clone,Copy,Default,PartialEq,Eq)]
struct Usage {metadata:usize,directories:usize,authorities:usize,snapshots:usize,snapshot_bytes:u64,jobs:usize,fds:usize}
impl Usage {
    fn add(self,r:Self)->Option<Self> {Some(Self {
        metadata:self.metadata.checked_add(r.metadata)?,directories:self.directories.checked_add(r.directories)?,
        authorities:self.authorities.checked_add(r.authorities)?,snapshots:self.snapshots.checked_add(r.snapshots)?,
        snapshot_bytes:self.snapshot_bytes.checked_add(r.snapshot_bytes)?,jobs:self.jobs.checked_add(r.jobs)?,fds:self.fds.checked_add(r.fds)?,
    })}
    fn subtract(&mut self,r:Self) {
        self.metadata-=r.metadata;self.directories-=r.directories;self.authorities-=r.authorities;
        self.snapshots-=r.snapshots;self.snapshot_bytes-=r.snapshot_bytes;self.jobs-=r.jobs;self.fds-=r.fds;
    }
    fn allowed(self,peer:bool)->bool {
        self.metadata<=(if peer {4*1024*1024}else{16*1024*1024})
            &&self.directories<=(if peer {64}else{256})&&self.authorities<=(if peer {32}else{128})
            &&self.snapshots<=(if peer {1}else{32})&&self.snapshot_bytes<=(if peer {64*1024*1024}else{256*1024*1024})
            &&self.jobs<=(if peer {1}else{4})&&self.fds<=(if peer {4096}else{8192})
    }
}
#[derive(Default)]
struct Book {total:Usage,peers:HashMap<Uuid,Usage>}
struct Reservation {book:Arc<Mutex<Book>>,peer:Uuid,usage:Mutex<Usage>}
impl Reservation {
    fn reserve(book:&Arc<Mutex<Book>>,peer:&Uuid,usage:Usage)->SecurityResult<Arc<Self>> {
        let mut current=book.lock().map_err(|_|ErrorCode::NotReady)?;
        let total=current.total.add(usage).filter(|value|value.allowed(false)).ok_or(ErrorCode::ResourceLimit)?;
        let selected=current.peers.get(peer).copied().unwrap_or_default().add(usage).filter(|value|value.allowed(true)).ok_or(ErrorCode::ResourceLimit)?;
        current.total=total;current.peers.insert(peer.clone(),selected);
        Ok(Arc::new(Self {book:book.clone(),peer:peer.clone(),usage:Mutex::new(usage)}))
    }
    fn adjust(&self,metadata:Option<usize>,fds:Option<usize>)->SecurityResult<()> {
        let mut own=self.usage.lock().map_err(|_|ErrorCode::NotReady)?;
        let mut proposed=*own;
        if let Some(bytes)=metadata {proposed.metadata=bytes;}
        if let Some(count)=fds {proposed.fds=proposed.fds.max(count);}
        if proposed==*own {return Ok(());}
        let mut book=self.book.lock().map_err(|_|ErrorCode::NotReady)?;
        let mut total=book.total;total.subtract(*own);
        let total=total.add(proposed).filter(|value|value.allowed(false)).ok_or(ErrorCode::ResourceLimit)?;
        let mut peer=book.peers.get(&self.peer).copied().ok_or(ErrorCode::NotReady)?;peer.subtract(*own);
        let peer=peer.add(proposed).filter(|value|value.allowed(true)).ok_or(ErrorCode::ResourceLimit)?;
        book.total=total;book.peers.insert(self.peer.clone(),peer);*own=proposed;Ok(())
    }
    fn metadata(&self,bytes:usize)->SecurityResult<()> {self.adjust(Some(bytes),None)}
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let (Ok(usage),Ok(mut book))=(self.usage.get_mut(),self.book.lock()) {
            book.total.subtract(*usage);
            if let Some(peer)=book.peers.get_mut(&self.peer) {peer.subtract(*usage);if *peer==Usage::default(){book.peers.remove(&self.peer);}}
        }
    }
}
pub(crate) struct ReadJob {_reservation:Arc<Reservation>,auth:Arc<Authorized>,pub(crate) deadline:Instant}
impl ReadJob {
    pub(crate) fn current(&self)->SecurityResult<()> {self.auth.live_connection()?;if Instant::now()>=self.deadline {return Err(ErrorCode::Timeout);}Ok(())}
    pub(crate) fn reserve_fds(&self,count:usize)->SecurityResult<()> {self.current()?;self._reservation.adjust(None,Some(count))}
}
struct Listing {id:RefId,directory:Directory,children:Vec<Child>,references:Vec<RefId>,bytes:usize,_reservation:Arc<Reservation>}
struct EntryRef {listing:Arc<Listing>,index:usize}
struct Cursor {listing:Arc<Listing>,offset:usize,parent:Option<RefId>}
struct Authority {
    peer:Uuid,epoch:u64,connection:Uuid,context:ProjectReadContext,expires:Instant,
    refs:HashMap<RefId,EntryRef>,listings:HashMap<Option<RefId>,Arc<Listing>>,cursors:HashMap<RefId,Cursor>,
    positions:HashMap<(RefId,usize),RefId>,bytes:usize,index_bytes:usize,base_bytes:usize,_reservation:Arc<Reservation>,
}
impl Authority {
    fn admitted(&self,auth:&Authorized,source:&ArtifactSource)->bool {
        self.peer==*auth.proof.peer_id()&&self.epoch==auth.proof.epoch()&&self.connection==auth.connection_id
            &&self.context.source()==source&&Instant::now()<self.expires
    }
    fn charge(&mut self,extra:usize)->SecurityResult<()> {
        let bytes=self.bytes.checked_add(extra).filter(|bytes|*bytes<=METADATA_PAGE).ok_or(ErrorCode::ResourceLimit)?;
        let index_bytes=self.index_bytes.checked_add(extra).ok_or(ErrorCode::ResourceLimit)?;
        self._reservation.metadata(self.base_bytes+index_bytes)?;
        self.bytes=bytes;self.index_bytes=index_bytes;Ok(())
    }
    fn clear(&mut self)->SecurityResult<()> {
        self.refs=HashMap::new();self.listings=HashMap::new();self.cursors=HashMap::new();self.positions=HashMap::new();self.bytes=0;self.index_bytes=0;
        self._reservation.metadata(self.base_bytes)
    }
}
struct SnapshotOwner {peer:Uuid,epoch:u64,connection:Uuid,context:ProjectReadContext,snapshot:project_read::Snapshot,expires:Instant,_reservation:Arc<Reservation>,_authority:Arc<Mutex<Authority>>}
struct ReadLease {value:Arc<SnapshotOwner>}
#[derive(Default)]
pub(super) struct ArtifactLeases {
    book:Arc<Mutex<Book>>,authorities:Vec<Arc<Mutex<Authority>>>,reads:HashMap<Uuid,ReadLease>,
}
impl ArtifactLeases {
    pub(super) fn maintain(&mut self) {
        let now=Instant::now();self.reads.retain(|_,read|now<read.value.expires);
        for authority in &self.authorities {if let Ok(mut authority)=authority.lock()&&now>=authority.expires {let _=authority.clear();}}
        self.authorities.retain(|authority|authority.lock().is_ok_and(|authority|now<authority.expires));
    }
}
pub(crate) struct PendingArtifact {
    topology:Arc<Topology>,opened:Option<(Uuid,Arc<SnapshotOwner>)>,armed:bool,
    _job:Arc<ReadJob>,_listing:Option<Arc<Listing>>,_read:Option<Arc<SnapshotOwner>>,pub(crate) expires:Instant,
}
impl PendingArtifact {
    pub(crate) fn published(&mut self) {self.armed=false;}
}
impl Drop for PendingArtifact {
    fn drop(&mut self) {
        if self.armed&&let Some((id,value))=&self.opened {
            let mut leases=self.topology.artifact_leases.lock().expect("artifact leases");
            if leases.reads.get(id).is_some_and(|read|Arc::ptr_eq(&read.value,value)){leases.reads.remove(id);}
        }
    }
}
impl Topology {
    pub(crate) fn reserve_project_read(&self,auth:&Arc<Authorized>,git:bool,deadline:Instant)->SecurityResult<Arc<ReadJob>> {
        auth.live_connection()?;
        if Instant::now()>=deadline{return Err(ErrorCode::Timeout);}
        let book=self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?.book.clone();
        let reservation=Reservation::reserve(&book,auth.proof.peer_id(),Usage {jobs:1,metadata:if git {6*METADATA_PAGE}else{METADATA_PAGE},fds:if git {256}else{16},..Usage::default()})?;
        Ok(Arc::new(ReadJob {_reservation:reservation,auth:auth.clone(),deadline}))
    }
    pub(crate) async fn artifact_request(
        self:&Arc<Self>,auth:&Arc<Authorized>,body:&RequestBody,initial:Option<ProjectReadContext>,job:Arc<ReadJob>,
    )->SecurityResult<(ProjectReadContext,ResultBody,PendingArtifact)> {
        job.current()?;
        let source=match body {RequestBody::ArtifactsList(p)=>&p.source,RequestBody::ArtifactsOpen(p)=>&p.source,RequestBody::ArtifactsRead(p)=>&p.source,RequestBody::ArtifactsClose(p)=>&p.source,_=>return Err(ErrorCode::InvalidRequest)};
        let mut pending=PendingArtifact {topology:self.clone(),opened:None,armed:true,_job:job.clone(),_listing:None,_read:None,expires:job.deadline};
        if let RequestBody::ArtifactsClose(params)=body {
            let mut leases=self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?;leases.maintain();
            let read=leases.reads.get(&params.read_id).filter(|read|read.value.peer==*auth.proof.peer_id()&&read.value.epoch==auth.proof.epoch()&&read.value.context.source()==source).ok_or(ErrorCode::NotReady)?;
            let context=read.value.context.clone();pending._read=Some(read.value.clone());leases.reads.remove(&params.read_id);
            return Ok((context,ResultBody::ArtifactsClose(ArtifactsCloseResult {source:source.clone(),read_id:params.read_id.clone(),closed:true}),pending));
        }
        if let RequestBody::ArtifactsRead(params)=body {
            let value={
                let mut leases=self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?;leases.maintain();
                leases.reads.get(&params.read_id).filter(|read|read.value.peer==*auth.proof.peer_id()&&read.value.epoch==auth.proof.epoch()
                    &&read.value.connection==auth.connection_id&&read.value.context.source()==source).ok_or(ErrorCode::NotReady)?.value.clone()
            };
            let deadline=job.deadline.min(value.expires);value.context.recheck(deadline)?;job.current()?;
            let start=usize::try_from(params.offset.0).map_err(|_|ErrorCode::InvalidRequest)?;
            if start>value.snapshot.bytes().len(){return Err(ErrorCode::InvalidRequest);}
            let end=start.saturating_add(params.limit.0 as usize).min(value.snapshot.bytes().len());
            let data=Base64Bytes(base64::engine::general_purpose::STANDARD.encode(&value.snapshot.bytes()[start..end]));
            let result=ArtifactsReadResult {source:source.clone(),read_id:params.read_id.clone(),offset:params.offset,data,eof:end==value.snapshot.bytes().len()};
            result.validate().map_err(|_|ErrorCode::InvalidRequest)?;
            pending.expires=deadline;pending._read=Some(value.clone());
            return Ok((value.context.clone(),ResultBody::ArtifactsRead(result),pending));
        }
        let authority={
            let mut leases=self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?;leases.maintain();
            if matches!(body,RequestBody::ArtifactsList(params)if params.parent.is_none()&&params.cursor.is_none()) {
                let context=initial.ok_or(ErrorCode::NotReady)?;
                if context.source()!=source {return Err(ErrorCode::StaleSession);}
                let known=leases.authorities.iter().find(|authority|authority.lock().is_ok_and(|authority|authority.admitted(auth,source)&&authority.context.same_authority(&context))).cloned();
                if let Some(known)=known {
                    known.lock().map_err(|_|ErrorCode::NotReady)?.clear()?;known
                }else{
                    let base_bytes=context.retained_bytes();
                    let reservation=Reservation::reserve(&leases.book,auth.proof.peer_id(),Usage {authorities:1,metadata:base_bytes,fds:3,..Usage::default()})?;
                    let authority=Arc::new(Mutex::new(Authority {peer:auth.proof.peer_id().clone(),epoch:auth.proof.epoch(),connection:auth.connection_id.clone(),
                        context,expires:Instant::now()+Duration::from_secs(300),refs:HashMap::new(),listings:HashMap::new(),cursors:HashMap::new(),positions:HashMap::new(),bytes:0,index_bytes:0,base_bytes,_reservation:reservation}));
                    leases.authorities.push(authority.clone());authority
                }
            }else{
                let wanted=match body {RequestBody::ArtifactsList(params)=>params.cursor.as_ref().or(params.parent.as_ref()),RequestBody::ArtifactsOpen(params)=>Some(&params.file),_=>None}.ok_or(ErrorCode::InvalidRequest)?;
                let wanted=reference(wanted)?;
                leases.authorities.iter().find(|authority|authority.lock().is_ok_and(|authority|authority.admitted(auth,source)&&(authority.refs.contains_key(&wanted)||authority.cursors.contains_key(&wanted))))
                    .cloned().ok_or(ErrorCode::NotReady)?
            }
        };
        let (context,expires)={let authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;(authority.context.clone(),authority.expires)};
        let deadline=job.deadline.min(expires);context.recheck(deadline)?;job.current()?;pending.expires=deadline;
        match body {
            RequestBody::ArtifactsList(params)=>{
                let parent=params.parent.as_ref().map(reference).transpose()?;
                let (listing,offset)=if let Some(cursor)=&params.cursor {
                    let authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;
                    let cursor=authority.cursors.get(&reference(cursor)?).ok_or(ErrorCode::NotReady)?;
                    if parent.is_some(){return Err(ErrorCode::InvalidRequest);}
                    if !authority.listings.get(&cursor.parent).is_some_and(|listing|Arc::ptr_eq(listing,&cursor.listing)){return Err(ErrorCode::StaleSession);}
                    (cursor.listing.clone(),cursor.offset)
                }else{
                    let selected={
                        let authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;
                        if !authority.admitted(auth,source){return Err(ErrorCode::StaleSession);}
                        parent.map(|parent|authority.refs.get(&parent).map(|entry|(entry.listing.clone(),entry.index)).ok_or(ErrorCode::NotReady)).transpose()?
                    };
                    {
                        let authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;
                        if authority.listings.len()>=16&&!authority.listings.contains_key(&parent){return Err(ErrorCode::ResourceLimit);}
                    }
                    {
                        let book=self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?.book.clone();
                        let reservation=Reservation::reserve(&book,auth.proof.peer_id(),Usage {directories:1,metadata:METADATA_PAGE,fds:1,..Usage::default()})?;
                        let worker=context.clone();let owner=job.clone();
                        let task=tokio::task::spawn_blocking(move||->SecurityResult<Arc<Listing>> {
                            owner.current()?;worker.recheck(deadline)?;
                            let location=worker.location();
                            let directory=match selected {
                                Some((listing,index))=>location.root.child_directory(&listing.directory,listing.children.get(index).ok_or(ErrorCode::NotReady)?,deadline),
                                None=>location.root.select(&[],deadline),
                            }.map_err(project_read::filesystem)?;
                            location.visible_directory(&directory,deadline).map_err(project_read::filesystem)?;
                            let children=location.root.list_entries(&directory,deadline).map_err(project_read::filesystem)?;
                            location.visible_children(&directory,&children,deadline).map_err(project_read::filesystem)?;
                            let bytes=children.iter().map(Child::retained_bytes).sum::<usize>().checked_add((children.capacity()-children.len())*std::mem::size_of::<Child>())
                                .and_then(|bytes|bytes.checked_add(children.len()*std::mem::size_of::<RefId>()+directory.retained_bytes())).filter(|bytes|*bytes<=METADATA_PAGE).ok_or(ErrorCode::ResourceLimit)?;
                            let references=(0..children.len()).map(|_|RefId::new_v4()).collect();
                            reservation.metadata(bytes)?;owner.current()?;worker.recheck(deadline)?;
                            Ok(Arc::new(Listing {id:RefId::new_v4(),directory,children,references,bytes,_reservation:reservation}))
                        });
                        let listing=tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),task).await.map_err(|_|ErrorCode::Timeout)?.map_err(|_|ErrorCode::NotReady)??;
                        job.current()?;context.recheck(deadline)?;
                        let mut authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;
                        if !authority.admitted(auth,source){return Err(ErrorCode::StaleSession);}
                        let bytes=authority.bytes.checked_add(listing.bytes).and_then(|bytes|bytes.checked_add(128)).filter(|bytes|*bytes<=METADATA_PAGE).ok_or(ErrorCode::ResourceLimit)?;
                        authority.charge(128)?;authority.bytes=bytes;authority.listings.insert(parent,listing.clone());
                        (listing,0)
                    }
                };
                context.location().visible_directory(&listing.directory,deadline).map_err(project_read::filesystem)?;
                let end=offset.checked_add(params.limit.0 as usize).ok_or(ErrorCode::ResourceLimit)?.min(listing.children.len());
                if offset>listing.children.len(){return Err(ErrorCode::InvalidRequest);}
                let mut authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;
                if !authority.admitted(auth,source){return Err(ErrorCode::StaleSession);}
                let mut entries=Vec::with_capacity(end-offset);
                for index in offset..end {
                    let child=&listing.children[index];let id=listing.references[index];
                    if !authority.refs.contains_key(&id) {
                        if authority.refs.len()==4096 {return Err(ErrorCode::ResourceLimit);}
                        authority.charge(128)?;authority.refs.insert(id,EntryRef {listing:listing.clone(),index});
                    }
                    entries.push(ArtifactEntry {reference:public(id),name:child.name.clone(),kind:if child.is_directory(){ArtifactKind::Directory}else{ArtifactKind::File},
                        size:child.size().map(Uint::new).transpose().map_err(|_|ErrorCode::ResourceLimit)?});
                }
                let next_cursor=if end<listing.children.len() {
                    let position=(listing.id,end);
                    let cursor=if let Some(cursor)=authority.positions.get(&position) {*cursor}else{
                        authority.charge(256)?;let cursor=RefId::new_v4();
                        authority.cursors.insert(cursor,Cursor {listing:listing.clone(),offset:end,parent});authority.positions.insert(position,cursor);cursor
                    };
                    Some(public(cursor))
                }else{None};
                let result=ArtifactsListResult {source:source.clone(),entries,next_cursor};
                result.validate().map_err(|_|ErrorCode::NotReady)?;
                pending._listing=Some(listing);
                Ok((context,ResultBody::ArtifactsList(result),pending))
            }
            RequestBody::ArtifactsOpen(params)=>{
                let (listing,index)={
                    let authority=authority.lock().map_err(|_|ErrorCode::NotReady)?;
                    let entry=authority.refs.get(&reference(&params.file)?).ok_or(ErrorCode::NotReady)?;(entry.listing.clone(),entry.index)
                };
                let child=listing.children.get(index).ok_or(ErrorCode::NotReady)?;
                let size=child.size().filter(|size|*size<=32*1024*1024).ok_or(ErrorCode::ResourceLimit)?;
                let name=child.name.clone();
                let book=self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?.book.clone();
                let reservation=Reservation::reserve(&book,auth.proof.peer_id(),Usage {snapshots:1,snapshot_bytes:size,..Usage::default()})?;
                let worker=context.clone();let owner=job.clone();let peer=auth.proof.peer_id().clone();let epoch=auth.proof.epoch();let connection=auth.connection_id.clone();
                let read_expires=Instant::now()+Duration::from_secs(615);
                let retained_authority=authority.clone();
                let task=tokio::task::spawn_blocking(move||->SecurityResult<Arc<SnapshotOwner>> {
                    owner.current()?;worker.recheck(deadline)?;
                    let location=worker.location();let child=listing.children.get(index).ok_or(ErrorCode::NotReady)?;
                    let file=location.root.child_file(&listing.directory,child,deadline).map_err(project_read::filesystem)?;
                    location.visible_file(&file,deadline).map_err(project_read::filesystem)?;
                    let snapshot=location.root.snapshot_file(&file,deadline).map_err(project_read::filesystem)?;
                    location.visible_file(&file,deadline).map_err(project_read::filesystem)?;
                    owner.current()?;worker.recheck(deadline)?;
                    Ok(Arc::new(SnapshotOwner {peer,epoch,connection,context:worker,snapshot,expires:read_expires,_reservation:reservation,_authority:retained_authority}))
                });
                let value=tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),task).await.map_err(|_|ErrorCode::Timeout)?.map_err(|_|ErrorCode::NotReady)??;
                job.current()?;context.recheck(deadline)?;
                let preview_kind=if value.snapshot.bytes().len()>1024*1024 {ArtifactPreviewKind::Unavailable}
                    else if std::str::from_utf8(value.snapshot.bytes()).is_ok_and(|text|!text.chars().any(|character|character.is_control()&&!matches!(character,'\t'|'\r'|'\n'))) {ArtifactPreviewKind::Text}else{ArtifactPreviewKind::Binary};
                let read_id=Uuid::random();
                let result=ArtifactsOpenResult {source:source.clone(),read_id:read_id.clone(),name,size:Uint(size),sha256:value.snapshot.sha256.clone(),preview_kind};
                result.validate().map_err(|_|ErrorCode::NotReady)?;
                self.artifact_leases.lock().map_err(|_|ErrorCode::NotReady)?.reads.insert(read_id.clone(),ReadLease {value:value.clone()});
                pending.opened=Some((read_id,value.clone()));pending._read=Some(value);
                Ok((context,ResultBody::ArtifactsOpen(result),pending))
            }
            _=>Err(ErrorCode::InvalidRequest),
        }
    }
    pub(crate) fn enqueue_artifact(&self,connection:&Uuid,id:Uuid,context:&ProjectReadContext,result:ResultBody,pending:&PendingArtifact)->SecurityResult<()> {
        pending._job.current()?;
        if matches!(&result,ResultBody::ArtifactsClose(_)){return self.state.response(connection,Response {id,outcome:Ok(result)});}
        if Instant::now()>=pending.expires {return Err(ErrorCode::NotReady);}
        let mut result=Some(result);
        let mut response_id=Some(id);
        context.with_current(pending.expires,||{
            pending._job.current()?;if Instant::now()>=pending.expires{return Err(ErrorCode::NotReady);}
            self.state.artifact_response(connection,response_id.take().ok_or(ErrorCode::NotReady)?,context.source(),context.epoch(),result.take().ok_or(ErrorCode::NotReady)?)
        })
    }
}

#[cfg(test)] mod project_read_quota_tests {
    use super::*;
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn abandoned_caller_does_not_release_the_actual_read_worker_fd_or_quota() {
        use std::{os::unix::fs::DirBuilderExt,sync::mpsc};
        let directory=std::fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("goatr-read-owner-{}",Uuid::random().0));
        std::fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();let path=directory.join("read-only");std::fs::write(&path,b"owned").unwrap();
        let file=std::fs::File::open(&path).unwrap();let descriptor=file.as_raw_fd();
        let book=Arc::new(Mutex::new(Book::default()));let peer=Uuid::random();
        let reservation=Reservation::reserve(&book,&peer,Usage {jobs:1,fds:1,metadata:METADATA_PAGE,..Usage::default()}).unwrap();
        let (entered,waiting)=mpsc::channel();let (release,finish)=mpsc::channel();let (done,completed)=mpsc::channel();
        let worker=tokio::task::spawn_blocking(move||{
            let owner=(file,reservation);entered.send(()).unwrap();finish.recv_timeout(Duration::from_secs(5)).unwrap();drop(owner);done.send(()).unwrap();
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(tokio::time::timeout(Duration::ZERO,worker).await.is_err());
        assert!(unsafe{libc::fcntl(descriptor,libc::F_GETFD)}>=0);
        assert_eq!(book.lock().unwrap().total.jobs,1);assert_eq!(book.lock().unwrap().total.fds,1);
        assert_eq!(Reservation::reserve(&book,&peer,Usage {jobs:1,..Usage::default()}).err(),Some(ErrorCode::ResourceLimit));
        release.send(()).unwrap();completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(book.lock().unwrap().total==Usage::default());assert!(!book.lock().unwrap().peers.contains_key(&peer));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn failed_growth_keeps_actual_owner_accounted_until_the_last_arc_drops() {
        let book=Arc::new(Mutex::new(Book::default()));let peer=Uuid::random();
        let owner=Reservation::reserve(&book,&peer,Usage {metadata:METADATA_PAGE,fds:1,..Usage::default()}).unwrap();let retained=owner.clone();
        assert_eq!(owner.adjust(Some(5*1024*1024),None),Err(ErrorCode::ResourceLimit));
        assert_eq!(book.lock().unwrap().total.metadata,METADATA_PAGE);
        owner.adjust(None,Some(4096)).unwrap();assert_eq!(owner.adjust(None,Some(4097)),Err(ErrorCode::ResourceLimit));
        drop(owner);assert_eq!(book.lock().unwrap().total.fds,4096);drop(retained);
        assert!(book.lock().unwrap().total==Usage::default());assert!(book.lock().unwrap().peers.is_empty());
    }
}

