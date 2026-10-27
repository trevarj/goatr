// SPDX-License-Identifier: GPL-3.0-or-later
//! Ephemeral creation-directory references; configured creation consent is not document authority.
use super::*;
use crate::project_read::{Directory, Root};
use std::{sync::atomic::{AtomicBool, AtomicUsize}, time::Instant};

const LEASE:Duration=Duration::from_secs(300);
const WORK:Duration=Duration::from_secs(5);
const GLOBAL_FDS:usize=8192;
const PEER_FDS:usize=4096;
const WORKERS:usize=4;

fn filesystem(error:io::Error)->ErrorCode {
    match error.kind() {
        io::ErrorKind::OutOfMemory=>ErrorCode::ResourceLimit,
        io::ErrorKind::TimedOut=>ErrorCode::Timeout,
        _ if matches!(error.raw_os_error(),Some(libc::EMFILE|libc::ENFILE|libc::ENOMEM))=>ErrorCode::ResourceLimit,
        _=>ErrorCode::NotReady,
    }
}
fn before(deadline:Instant)->SecurityResult<()> {
    if Instant::now()>=deadline {Err(ErrorCode::Timeout)} else {Ok(())}
}
#[derive(Default)]struct Quota {fds:usize,workers:usize,peers:HashMap<Uuid,(usize,usize)>}
struct Reservation {quota:Arc<Mutex<Quota>>,peer:Uuid,fds:AtomicUsize,worker:AtomicBool}
impl Reservation {
    fn finish(&self,count:usize) {
        let mut quota=self.quota.lock().expect("creation directory quota");
        let previous=self.fds.swap(count,Ordering::AcqRel);
        assert!(count<=previous);
        quota.fds-=previous-count;
        let worker=usize::from(self.worker.swap(false,Ordering::AcqRel));
        quota.workers-=worker;
        let peer=quota.peers.get_mut(&self.peer).expect("reserved directory peer");
        peer.0-=previous-count;peer.1-=worker;
        if *peer==(0,0){quota.peers.remove(&self.peer);}
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut quota=self.quota.lock().expect("creation directory quota");
        let count=self.fds.load(Ordering::Acquire);
        let worker=usize::from(self.worker.load(Ordering::Acquire));
        quota.fds-=count;quota.workers-=worker;
        if let Some(peer)=quota.peers.get_mut(&self.peer) {
            peer.0-=count;peer.1-=worker;
            if *peer==(0,0){quota.peers.remove(&self.peer);}
        }
    }
}
#[derive(Clone)]
pub(crate) struct Origin {
    pub(crate) source:ArtifactSource,
    pub(crate) epoch:Uuid,
    pub(crate) pane:Id,
    pub(crate) terminal:Id,
    pub(crate) workspace:Id,
    pub(crate) tab:Id,
}
/// Retain physical authority, not an entire superseded inventory for every chooser.
pub(crate) struct OriginalSource {
    pub(crate) server_id:Uuid,
    pub(crate) incarnation:Uuid,
    socket_path:PathBuf,
    evidence:ServerEvidence,
    executable: Arc<InstalledExecutablePin>,
    invalidation:Arc<AtomicU64>,
}
impl OriginalSource {
    fn capture(source:Source)->Self {
        Self {
            server_id: source.server_id,
            incarnation: source.incarnation,
            socket_path: source.socket_path,
            evidence: source.evidence,
            executable: source.executable,
            invalidation: source.invalidation,
        }
    }
    fn matches(&self,current:&Source)->bool {
        self.server_id==current.server_id&&self.incarnation==current.incarnation
            &&self.socket_path==current.socket_path&&self.evidence==current.evidence
            && self.executable.same_identity(&current.executable)
            &&Arc::ptr_eq(&self.invalidation,&current.invalidation)
    }
}
pub(crate) struct DirectoryContext {
    pub(crate) peer:Uuid,
    pub(crate) peer_epoch:u64,
    pub(crate) connection:Uuid,
    pub(crate) physical:OriginalSource,
    pub(crate) server_process:ProcessEvidence,
    pub(crate) project:Uuid,
    pub(crate) origin:Option<Origin>,
    root:Arc<Root>,
    pub(crate) expires:Instant,
}
impl DirectoryContext {
    fn accepts(&self,proof:&AuthenticatedPeer,connection:&Uuid,server:&Uuid,project:&Uuid)->SecurityResult<()> {
        if Instant::now()>=self.expires{return Err(ErrorCode::CursorExpired)}
        if self.peer!=*proof.peer_id()||self.peer_epoch!=proof.epoch()||self.connection!=*connection {
            return Err(ErrorCode::CursorExpired);
        }
        if self.physical.server_id!=*server||self.project!=*project{return Err(ErrorCode::CursorExpired)}
        Ok(())
    }
    fn accepts_read(&self,proof:&AuthenticatedPeer,connection:&Uuid,params:&CreationDirectoriesParams)->SecurityResult<()> {
        self.accepts(proof,connection,&params.server_id,&params.project_id)?;
        if self.origin.as_ref().map(|origin|&origin.source)!=params.source.as_ref(){return Err(ErrorCode::CursorExpired)}
        Ok(())
    }
}
struct OwnedDirectory {value:Directory,_reservation:Arc<Reservation>}
struct Reference {context:Arc<DirectoryContext>,directory:Arc<OwnedDirectory>}
struct Catalogue {
    context:Arc<DirectoryContext>,parent:Option<Uuid>,directory:Arc<OwnedDirectory>,
    entries:Vec<(Uuid,Arc<OwnedDirectory>)>,limit:usize,cursors:Vec<Uuid>,
}
struct Cursor {catalogue:Arc<Catalogue>,offset:usize}
#[derive(Default)]struct Registry {references:HashMap<Uuid,Reference>,cursors:HashMap<Uuid,Cursor>}
#[derive(Default)]pub(crate) struct Leases {inner:Mutex<Registry>,quota:Arc<Mutex<Quota>>}
impl Leases {
    fn reserve(&self,peer:&Uuid,fds:usize)->SecurityResult<Arc<Reservation>> {
        let mut quota=self.quota.lock().expect("creation directory quota");
        let (used,workers)=quota.peers.get(peer).copied().unwrap_or_default();
        if quota.workers>=WORKERS||workers!=0{return Err(ErrorCode::Busy)}
        if quota.fds.saturating_add(fds)>GLOBAL_FDS||used.saturating_add(fds)>PEER_FDS{return Err(ErrorCode::ResourceLimit)}
        if !quota.peers.contains_key(peer){quota.peers.try_reserve(1).map_err(|_|ErrorCode::ResourceLimit)?;}
        quota.fds+=fds;quota.workers+=1;quota.peers.insert(peer.clone(),(used+fds,1));
        Ok(Arc::new(Reservation{quota:self.quota.clone(),peer:peer.clone(),fds:AtomicUsize::new(fds),worker:AtomicBool::new(true)}))
    }
    pub(crate) fn retain(&self,valid:impl Fn(&DirectoryContext)->bool) {
        let now=Instant::now();
        let mut registry=self.inner.lock().expect("creation directory leases");
        registry.references.retain(|_,entry|entry.context.expires>now&&valid(&entry.context));
        registry.cursors.retain(|_,entry|entry.catalogue.context.expires>now&&valid(&entry.catalogue.context));
    }
    fn maintain(&self){self.retain(|_|true)}
    pub(crate) fn release_connection(&self,connection:&Uuid){self.retain(|context|&context.connection!=connection)}
    pub(crate) fn release_peer(&self,peer:&Uuid){self.retain(|context|&context.peer!=peer)}
    fn reference(&self,proof:&AuthenticatedPeer,connection:&Uuid,id:&Uuid,server:&Uuid,project:&Uuid)->SecurityResult<(Arc<DirectoryContext>,Arc<OwnedDirectory>)> {
        self.maintain();
        let registry=self.inner.lock().expect("creation directory leases");
        let reference=registry.references.get(id).ok_or(ErrorCode::CursorExpired)?;
        reference.context.accepts(proof,connection,server,project)?;
        Ok((reference.context.clone(),reference.directory.clone()))
    }
    fn cursor(&self,proof:&AuthenticatedPeer,connection:&Uuid,params:&CreationDirectoriesParams)->SecurityResult<(Arc<Catalogue>,usize)> {
        self.maintain();
        let registry=self.inner.lock().expect("creation directory leases");
        let cursor=registry.cursors.get(params.cursor.as_ref().ok_or(ErrorCode::InvalidRequest)?).ok_or(ErrorCode::CursorExpired)?;
        let catalogue=&cursor.catalogue;
        catalogue.context.accepts_read(proof,connection,params)?;
        if catalogue.parent!=params.parent||catalogue.limit!=params.limit.map_or(100,|limit|limit.0 as usize) {
            return Err(ErrorCode::CursorExpired);
        }
        Ok((catalogue.clone(),cursor.offset))
    }
    fn publish(&self,catalogue:Option<&Arc<Catalogue>>)->SecurityResult<()> {
        let Some(catalogue)=catalogue else{return Ok(())};
        let mut registry=self.inner.lock().expect("creation directory leases");
        if catalogue.entries.iter().any(|(id,_)|registry.references.contains_key(id))
            ||catalogue.cursors.iter().any(|id|registry.cursors.contains_key(id)){return Err(ErrorCode::ResourceLimit)}
        registry.references.try_reserve(catalogue.entries.len()).map_err(|_|ErrorCode::ResourceLimit)?;
        registry.cursors.try_reserve(catalogue.cursors.len()).map_err(|_|ErrorCode::ResourceLimit)?;
        for (id,directory) in &catalogue.entries {
            match registry.references.entry(id.clone()) {
                std::collections::hash_map::Entry::Vacant(entry)=>{entry.insert(Reference{context:catalogue.context.clone(),directory:directory.clone()});}
                std::collections::hash_map::Entry::Occupied(_)=>return Err(ErrorCode::ResourceLimit),
            }
        }
        for (index,id) in catalogue.cursors.iter().enumerate() {
            match registry.cursors.entry(id.clone()) {
                std::collections::hash_map::Entry::Vacant(entry)=>{entry.insert(Cursor{catalogue:catalogue.clone(),offset:(index+1)*catalogue.limit});}
                std::collections::hash_map::Entry::Occupied(_)=>return Err(ErrorCode::ResourceLimit),
            }
        }
        Ok(())
    }
    fn discard(&self,catalogue:Option<&Arc<Catalogue>>) {
        let Some(catalogue)=catalogue else{return};
        let mut registry=self.inner.lock().expect("creation directory leases");
        for (id,directory) in &catalogue.entries {
            if registry.references.get(id).is_some_and(|entry| {
                Arc::ptr_eq(&entry.context,&catalogue.context)&&Arc::ptr_eq(&entry.directory,directory)
            }) { registry.references.remove(id); }
        }
        registry.cursors.retain(|_,cursor|!Arc::ptr_eq(&cursor.catalogue,catalogue));
    }
}
pub(crate) struct Prepared {
    pub(crate) context:Arc<DirectoryContext>,
    directory:Arc<OwnedDirectory>,
    pub(crate) result:CreationDirectoriesResult,
    catalogue:Option<Arc<Catalogue>>,
}
fn page(catalogue:&Catalogue,offset:usize)->SecurityResult<CreationDirectoriesResult> {
    before(catalogue.context.expires)?;
    let end=offset.saturating_add(catalogue.limit).min(catalogue.entries.len());
    if offset>catalogue.entries.len(){return Err(ErrorCode::CursorExpired)}
    let mut entries=Vec::with_capacity(end-offset);
    for (reference,directory) in &catalogue.entries[offset..end] {
        let name=directory.value.name().ok_or(ErrorCode::NotReady)?;
        entries.push(CreationDirectoryEntry{reference:reference.clone(),name:Label::new(name).map_err(|_|ErrorCode::NotReady)?});
    }
    Ok(CreationDirectoriesResult{server_id:catalogue.context.physical.server_id.clone(),project_id:catalogue.context.project.clone(),
        source:catalogue.context.origin.as_ref().map(|origin|origin.source.clone()),entries,
        next_cursor:if end<catalogue.entries.len(){Some(catalogue.cursors[end/catalogue.limit-1].clone())}else{None}})
}
#[derive(PartialEq,Eq)]
struct CreatedRoot { workspace:Id,tab:Id,pane:Id,terminal:Id }
pub(crate) struct CreationSelection {
    context:Arc<DirectoryContext>,directory:Arc<OwnedDirectory>,reference:Option<Uuid>,placement:Option<Origin>,
    shell:Mutex<Option<ProcessEvidence>>,
    created:Mutex<Option<CreatedRoot>>,
    unproved:AtomicBool,
}
impl CreationSelection {
    fn remember<T>(&self,result:SecurityResult<T>)->SecurityResult<T> {
        if result.is_err(){self.unproved.store(true,Ordering::Release);}
        result
    }
    pub(crate) fn unproved(&self)->bool{self.unproved.load(Ordering::Acquire)}
    pub(crate) fn validate(&self,topology:&Topology,params:&SessionCreateParams)->SecurityResult<Source> {
        self.remember((|| {
        if !topology.config.allow_scoped_shell_probe{return Err(ErrorCode::NotReady);}
        let placement_matches=match (&self.placement,&params.placement) {
            (None,None)=>true,
            (Some(origin),Some(placement))=>matches!(&origin.source,ArtifactSource::Session{target} if target==&placement.source_target),
            _=>false,
        };
        if self.context.project!=params.project_id||self.context.physical.server_id!=params.server_id
            ||self.reference!=params.directory_ref
            ||!placement_matches {
            return Err(ErrorCode::StaleSession);
        }
        let source=topology.validate_directory_context(&self.context)?;
        if let Some(placement)=&self.placement {topology.validate_creation_origin(placement,&source)?;}
        if let Some(created)=self.created.lock().expect("created root").as_ref() {
            let pane=source.snapshot.pane(&created.pane).map_err(policy)?;
            if pane.workspace_id!=created.workspace||pane.tab_id!=created.tab||pane.terminal_id!=created.terminal {
                return Err(ErrorCode::StaleSession);
            }
        }
        self.checked_directory(&params.server_id,&params.project_id,params.directory_ref.as_ref())?;
        Ok(source)
        })())
    }
    fn checked_directory(&self,server:&Uuid,project:&Uuid,reference:Option<&Uuid>)->SecurityResult<&Path> {
        self.remember((|| {
            if &self.context.project!=project||&self.context.physical.server_id!=server||self.reference.as_ref()!=reference {
                return Err(ErrorCode::StaleSession);
            }
            let path=self.context.root.revalidate(&self.directory.value,self.context.expires).map_err(filesystem)?;
            if let Some(shell)=self.shell.lock().expect("creation shell").as_ref(){self.verify_process_cwd(shell)?;}
            Ok(path)
        })())
    }
    pub(crate) fn directory(&self,params:&SessionCreateParams)->SecurityResult<PathBuf> {
        self.checked_directory(&params.server_id,&params.project_id,params.directory_ref.as_ref()).map(Path::to_path_buf)
    }
    pub(crate) fn workspace(&self)->Option<&Id>{self.placement.as_ref().map(|origin|&origin.workspace)}
    pub(crate) fn retain_created(&self,resources:&CreationResources,tab:Id)->SecurityResult<()> {
        let created=CreatedRoot{
            workspace:resources.workspace_id.clone().ok_or(ErrorCode::NotReady)?,tab,
            pane:resources.pane_id.clone().ok_or(ErrorCode::NotReady)?,
            terminal:resources.terminal_id.clone().ok_or(ErrorCode::NotReady)?,
        };
        let mut held=self.created.lock().expect("created root");
        if held.as_ref().is_some_and(|previous|previous!=&created){return Err(ErrorCode::StaleSession)}
        *held=Some(created);Ok(())
    }
    pub(crate) fn matches_created(&self,pane:&Pane)->bool {
        self.created.lock().expect("created root").as_ref().is_some_and(|created| {
            pane.pane_id==created.pane&&pane.workspace_id==created.workspace
                &&pane.tab_id==created.tab&&pane.terminal_id==created.terminal
        })
    }
    fn verify_process_cwd(&self,shell:&ProcessEvidence)->SecurityResult<()> {
        let cwd=kernel_cwd(shell)?;
        if !self.context.root.matches_directory(&self.directory.value,&cwd,self.context.expires).map_err(filesystem)? {
            return Err(ErrorCode::NotReady);
        }
        Ok(())
    }
    pub(crate) fn retain_shell(&self,shell:ProcessEvidence)->SecurityResult<()> {
        self.remember((|| {
            self.verify_process_cwd(&shell)?;
            let mut held=self.shell.lock().expect("creation shell");
            if held.as_ref().is_some_and(|old|old!=&shell){return Err(ErrorCode::StaleSession)}
            *held=Some(shell);Ok(())
        })())
    }
    pub(crate) fn validate_terminal_proved(&self,topology:&Topology,params:&TerminalCreateParams)->SecurityResult<Source> {
        if self.shell.lock().expect("creation shell").is_none(){return Err(ErrorCode::NotReady);}
        self.validate_terminal(topology,params)
    }
    pub(crate) fn validate_terminal(&self,topology:&Topology,params:&TerminalCreateParams)->SecurityResult<Source> {
        self.remember((|| {
            topology.terminal_source_control(&params.server_id)?;
            if self.placement.is_some()
                ||self.context.origin.as_ref().map(|origin|&origin.source)!=params.source.as_ref(){return Err(ErrorCode::NotReady);}
            self.checked_directory(&params.server_id,&params.project_id,params.directory_ref.as_ref())?;
            let source=topology.validate_directory_context(&self.context)?;
            if !source.snapshot.workspaces.iter().any(|workspace|workspace.workspace_id==params.workspace_id)
                ||self.context.origin.as_ref().is_some_and(|origin|origin.workspace!=params.workspace_id){return Err(ErrorCode::StaleSession);}
            if let Some(created)=self.created.lock().expect("created root").as_ref() {
                let pane=source.snapshot.pane(&created.pane).map_err(policy)?;
                if pane.workspace_id!=params.workspace_id||pane.tab_id!=created.tab||pane.terminal_id!=created.terminal
                    ||!shells::ordinary_shell(&source.snapshot,pane){return Err(ErrorCode::StaleSession);}
            }
            Ok(source)
        })())
    }
    pub(crate) fn terminal_directory(&self,params:&TerminalCreateParams)->SecurityResult<PathBuf> {
        self.checked_directory(&params.server_id,&params.project_id,params.directory_ref.as_ref()).map(Path::to_path_buf)
    }
    pub(crate) fn retain_terminal_created(&self,resources:&ShellCreationResources)->SecurityResult<()> {
        if resources.server_id.as_ref()!=Some(&self.context.physical.server_id){return Err(ErrorCode::StaleSession);}
        let created=CreatedRoot{workspace:resources.workspace_id.clone().ok_or(ErrorCode::NotReady)?,
            tab:resources.tab_id.clone().ok_or(ErrorCode::NotReady)?,pane:resources.pane_id.clone().ok_or(ErrorCode::NotReady)?,
            terminal:resources.terminal_id.clone().ok_or(ErrorCode::NotReady)?};
        let mut held=self.created.lock().expect("created root");
        if held.as_ref().is_some_and(|previous|previous!=&created){return Err(ErrorCode::StaleSession);}
        *held=Some(created);Ok(())
    }
}
fn kernel_cwd(process:&ProcessEvidence)->SecurityResult<File> {
    if process_evidence(process.pid).map_err(policy)?!=*process{return Err(ErrorCode::StaleSession)}
    let proc=OpenOptions::new().read(true).custom_flags(libc::O_PATH|libc::O_DIRECTORY|libc::O_CLOEXEC|libc::O_NOFOLLOW).open("/proc").map_err(policy)?;
    let mut filesystem:libc::statfs=unsafe{std::mem::zeroed()};
    // SAFETY: held proc descriptor and correctly sized statfs storage.
    if unsafe{libc::fstatfs(proc.as_raw_fd(),&mut filesystem)}!=0||filesystem.f_type!=libc::PROC_SUPER_MAGIC{return Err(ErrorCode::NotReady)}
    let name=CString::new(process.pid.to_string()).map_err(|_|ErrorCode::NotReady)?;
    let fd=unsafe{libc::openat(proc.as_raw_fd(),name.as_ptr(),libc::O_PATH|libc::O_DIRECTORY|libc::O_CLOEXEC|libc::O_NOFOLLOW)};
    if fd<0{return Err(ErrorCode::NotReady)}
    let directory=unsafe{File::from_raw_fd(fd)};
    // Only the original proved kernel PID's cwd magic link is followed, never a phone path.
    let fd=unsafe{libc::openat(directory.as_raw_fd(),c"cwd".as_ptr(),libc::O_PATH|libc::O_DIRECTORY|libc::O_CLOEXEC)};
    if fd<0{return Err(ErrorCode::NotReady)}
    let cwd=unsafe{File::from_raw_fd(fd)};
    if process_evidence(process.pid).map_err(policy)?!=*process{return Err(ErrorCode::StaleSession)}
    Ok(cwd)
}

impl Topology {
    pub(crate) fn creation_origin(&self,server:&Uuid,input:&ArtifactSource)->SecurityResult<Origin> {
        let current=self.source(server)?;
        let origin=match input {
            ArtifactSource::Session{target}=>{
                let (key,epoch)={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&target.session_id)
                    .filter(|binding|binding.target==*target&&binding.key.server_id==*server).ok_or(ErrorCode::StaleSession)?;(binding.key.clone(),binding.source_epoch.clone())};
                let state=self.state.session_state(target,&epoch)?;
                if state.summary.connectivity!=SessionConnectivity::Ready{return Err(ErrorCode::NotReady)}
                let pane=current.snapshot.pane(&state.summary.pane_id).map_err(policy)?;
                if key.terminal_id!=pane.terminal_id{return Err(ErrorCode::StaleSession)}
                Origin{source:input.clone(),epoch,pane:pane.pane_id.clone(),terminal:pane.terminal_id.clone(),workspace:pane.workspace_id.clone(),tab:pane.tab_id.clone()}
            }
            ArtifactSource::Shell{target}=>{
                let inner=self.inner.lock().expect("topology mutex");
                let binding=inner.shell_bindings.get(&target.shell_id).filter(|binding|binding.target==*target).ok_or(ErrorCode::StaleSession)?;
                if !matches!(&binding.key,StableResourceKey::Shell{server_id,server_incarnation,..} if server_id==server&&server_incarnation==&current.incarnation){return Err(ErrorCode::StaleSession)}
                let identity=&binding.proof.identity;
                Origin{source:input.clone(),epoch:binding.proof.source_epoch.clone(),pane:identity.pane_id.clone(),terminal:identity.terminal_id.clone(),workspace:identity.workspace_id.clone(),tab:identity.tab_id.clone()}
            }
        };
        self.validate_creation_origin(&origin,&current)?;Ok(origin)
    }
    pub(crate) fn validate_creation_origin(&self,origin:&Origin,source:&Source)->SecurityResult<()> {
        match &origin.source {
            ArtifactSource::Session{target}=>{
                let inner=self.inner.lock().expect("topology mutex");
                let binding=inner.bindings.get(&target.session_id).filter(|binding|binding.target==*target&&binding.source_epoch==origin.epoch
                    &&binding.key.server_id==source.server_id&&binding.key.server_incarnation==source.incarnation).ok_or(ErrorCode::StaleSession)?;
                let state=self.state.session_state(target,&origin.epoch)?;
                if state.summary.connectivity!=SessionConnectivity::Ready||state.summary.pane_id!=origin.pane
                    ||state.summary.terminal_id!=origin.terminal||state.summary.workspace_id!=origin.workspace||binding.key.terminal_id!=origin.terminal{return Err(ErrorCode::StaleSession)}
            }
            ArtifactSource::Shell{target}=>{
                let inner=self.inner.lock().expect("topology mutex");
                let binding=inner.shell_bindings.get(&target.shell_id).filter(|binding|binding.target==*target&&binding.proof.source_epoch==origin.epoch).ok_or(ErrorCode::StaleSession)?;
                if !matches!(&binding.key,StableResourceKey::Shell{server_id,server_incarnation,..} if server_id==&source.server_id&&server_incarnation==&source.incarnation){return Err(ErrorCode::StaleSession)}
            }
        }
        let pane=source.snapshot.pane(&origin.pane).map_err(policy)?;
        if pane.terminal_id!=origin.terminal||pane.workspace_id!=origin.workspace||pane.tab_id!=origin.tab{return Err(ErrorCode::StaleSession)}
        if matches!(origin.source,ArtifactSource::Shell{..})&&(pane.agent.is_some()||pane.agent_session.is_some()
            ||source.snapshot.agents.iter().any(|agent|agent.terminal_id==origin.terminal)){return Err(ErrorCode::StaleSession)}
        source.recheck().map_err(policy)
    }
    fn directory_context(&self,auth:&Authorized,server:&Uuid,project:&Uuid,source:Option<&ArtifactSource>)->SecurityResult<Arc<DirectoryContext>> {
        if !self.config.allow_scoped_shell_probe&&!self.config.allow_terminal_control{return Err(ErrorCode::NotReady)}
        self.config.project(project)?;
        let physical=self.source(server)?;
        let server_process=process_evidence(physical.evidence.pid).map_err(policy)?;
        let origin=source.map(|source|self.creation_origin(server,source)).transpose()?;
        let root=self.creation_roots.get(project).and_then(Option::as_ref).ok_or(ErrorCode::NotReady)?.clone();
        Ok(Arc::new(DirectoryContext{peer:auth.proof.peer_id().clone(),peer_epoch:auth.proof.epoch(),connection:auth.connection_id.clone(),
            physical:OriginalSource::capture(physical),server_process,project:project.clone(),origin,root,expires:Instant::now()+LEASE}))
    }
    pub(crate) fn validate_directory_context(&self,context:&DirectoryContext)->SecurityResult<Source> {
        before(context.expires)?;
        if !self.config.allow_scoped_shell_probe&&!self.config.allow_terminal_control{return Err(ErrorCode::NotReady)}
        self.config.project(&context.project)?;
        if !self.creation_roots.get(&context.project).and_then(Option::as_ref).is_some_and(|root|Arc::ptr_eq(root,&context.root)){return Err(ErrorCode::StaleSession)}
        let current=self.source(&context.physical.server_id)?;
        if !context.physical.matches(&current)||process_evidence(context.server_process.pid).map_err(policy)?!=context.server_process{return Err(ErrorCode::StaleSession)}
        if let Some(origin)=&context.origin{self.validate_creation_origin(origin,&current)?;}
        current.recheck().map_err(policy)?;Ok(current)
    }
    pub(crate) fn validate_placement(&self,params:&SessionCreateParams)->SecurityResult<()> {
        if let Some(placement)=&params.placement {
            self.creation_origin(&params.server_id,&ArtifactSource::Session{target:placement.source_target.clone()})?;
        }
        Ok(())
    }
    pub(crate) async fn creation_directories(self:&Arc<Self>,auth:&Authorized,params:&CreationDirectoriesParams)->SecurityResult<Prepared> {
        params.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let leases=&self.state.creation_directories;
        if params.cursor.is_some() {
            let reservation=leases.reserve(auth.proof.peer_id(),0)?;
            let (catalogue,offset)=leases.cursor(&auth.proof,&auth.connection_id,params)?;
            self.validate_directory_context(&catalogue.context)?;
            let root=catalogue.context.root.clone();let directory=catalogue.directory.clone();let deadline=catalogue.context.expires.min(Instant::now()+WORK);
            tokio::task::spawn_blocking(move||{root.revalidate(&directory.value,deadline).map_err(filesystem)?;reservation.finish(0);Ok::<_,ErrorCode>(())}).await.map_err(|_|ErrorCode::NotReady)??;
            self.validate_directory_context(&catalogue.context)?;
            return Ok(Prepared{context:catalogue.context.clone(),directory:catalogue.directory.clone(),result:page(&catalogue,offset)?,catalogue:None});
        }
        let reservation=leases.reserve(auth.proof.peer_id(),1001)?;
        let (context,parent)=if let Some(parent)=&params.parent {
            let (context,directory)=leases.reference(&auth.proof,&auth.connection_id,parent,&params.server_id,&params.project_id)?;
            context.accepts_read(&auth.proof,&auth.connection_id,params)?;(context,Some(directory))
        } else {(self.directory_context(auth,&params.server_id,&params.project_id,params.source.as_ref())?,None)};
        self.validate_directory_context(&context)?;
        let root=context.root.clone();let deadline=context.expires.min(Instant::now()+WORK);
        let (directory,children)=tokio::task::spawn_blocking(move||{
            let root_selection=parent.is_none();
            let directory=match parent {Some(directory)=>directory,None=>Arc::new(OwnedDirectory{value:root.select(&[],deadline).map_err(filesystem)?,_reservation:reservation.clone()})};
            let children=root.list(&directory.value,deadline).map_err(filesystem)?;
            reservation.finish(children.len()+usize::from(root_selection));
            let children=children.into_iter().map(|value|Arc::new(OwnedDirectory{value,_reservation:reservation.clone()})).collect::<Vec<_>>();
            Ok::<_,ErrorCode>((directory,children))
        }).await.map_err(|_|ErrorCode::NotReady)??;
        self.validate_directory_context(&context)?;before(deadline)?;
        let limit=params.limit.map_or(100,|limit|limit.0 as usize);
        let entries=children.into_iter().map(|directory|(Uuid::random(),directory)).collect::<Vec<_>>();
        let cursors=(limit..entries.len()).step_by(limit).map(|_|Uuid::random()).collect();
        let catalogue=Arc::new(Catalogue{context:context.clone(),parent:params.parent.clone(),directory,entries,limit,cursors});
        let result=page(&catalogue,0)?;
        before(deadline)?;Ok(Prepared{context,directory:catalogue.as_ref().directory.clone(),result,catalogue:Some(catalogue)})
    }
    pub(crate) async fn enqueue_creation_directories(&self,auth:&Authorized,id:Uuid,prepared:Prepared)->SecurityResult<()> {
        let _update=self.updates.lock().await;
        prepared.context.accepts(&auth.proof,&auth.connection_id,&prepared.context.physical.server_id,&prepared.context.project)?;
        let source=self.validate_directory_context(&prepared.context)?;
        prepared.context.root.revalidate(&prepared.directory.value,prepared.context.expires.min(Instant::now()+WORK)).map_err(filesystem)?;
        let Prepared{context,result,catalogue,directory:_directory}=prepared;
        let outcome=self.state.creation_directories_response(&context,id,result,|| {
            // Atomic-only under State's lock; no native or filesystem I/O here.
            if !source.revision_current(){return Err(ErrorCode::NotReady)}
            self.state.creation_directories.publish(catalogue.as_ref())
        });
        if outcome.is_err(){self.state.creation_directories.discard(catalogue.as_ref());}
        outcome
    }
    pub(crate) async fn capture_creation(&self,auth:&Authorized,params:&SessionCreateParams)->SecurityResult<Arc<CreationSelection>> {
        if !self.config.allow_scoped_shell_probe{return Err(ErrorCode::NotReady);}
        let placement=params.placement.as_ref().map(|placement|self.creation_origin(&params.server_id,&ArtifactSource::Session{target:placement.source_target.clone()})).transpose()?;
        let (context,directory)=if let Some(reference)=&params.directory_ref {
            self.state.creation_directories.reference(&auth.proof,&auth.connection_id,reference,&params.server_id,&params.project_id)?
        } else {
            let reservation=self.state.creation_directories.reserve(auth.proof.peer_id(),1)?;
            let source=placement.as_ref().map(|origin|&origin.source);
            let context=self.directory_context(auth,&params.server_id,&params.project_id,source)?;
            let root=context.root.clone();let deadline=context.expires.min(Instant::now()+WORK);
            let directory=tokio::task::spawn_blocking(move||{
                let value=root.select(&[],deadline).map_err(filesystem)?;reservation.finish(1);
                Ok::<_,ErrorCode>(Arc::new(OwnedDirectory{value,_reservation:reservation}))
            }).await.map_err(|_|ErrorCode::NotReady)??;
            (context,directory)
        };
        if let (Some(placement),Some(origin))=(&placement,&context.origin)
            && placement.source!=origin.source {return Err(ErrorCode::StaleSession)}
        let selected=Arc::new(CreationSelection{context,directory,reference:params.directory_ref.clone(),placement,
            shell:Mutex::new(None),created:Mutex::new(None),unproved:AtomicBool::new(false)});
        selected.validate(self,params)?;Ok(selected)
    }
    pub(crate) async fn capture_terminal_creation(&self,auth:&Authorized,params:&TerminalCreateParams)->SecurityResult<Arc<CreationSelection>> {
        self.terminal_source_control(&params.server_id)?;
        let (context,directory)=if let Some(reference)=&params.directory_ref {
            self.state.creation_directories.reference(&auth.proof,&auth.connection_id,reference,&params.server_id,&params.project_id)?
        }else{
            let reservation=self.state.creation_directories.reserve(auth.proof.peer_id(),1)?;
            let context=self.directory_context(auth,&params.server_id,&params.project_id,params.source.as_ref())?;
            let root=context.root.clone();let deadline=context.expires.min(Instant::now()+WORK);
            let directory=tokio::task::spawn_blocking(move||{
                let value=root.select(&[],deadline).map_err(filesystem)?;reservation.finish(1);
                Ok::<_,ErrorCode>(Arc::new(OwnedDirectory{value,_reservation:reservation}))
            }).await.map_err(|_|ErrorCode::NotReady)??;
            (context,directory)
        };
        let selected=Arc::new(CreationSelection{context,directory,reference:params.directory_ref.clone(),placement:None,
            shell:Mutex::new(None),created:Mutex::new(None),unproved:AtomicBool::new(false)});
        selected.validate_terminal(self,params)?;Ok(selected)
    }
    pub(crate) fn execution_directory(&self,selection:&CreationSelection,params:&SessionCreateParams)->SecurityResult<PathBuf> {
        selection.directory(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt,PermissionsExt};

    struct Fixture {
        path:PathBuf,
        root:Arc<Root>,
        _socket:std::os::unix::net::UnixListener,
    }
    impl Fixture {
        fn new()->Self {
            let executable=fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
            let id=Uuid::random().0;
            let path=executable.parent().unwrap().join(format!("cd-{}{}{}",&id[..8],&id[9..13],&id[14..18]));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            fs::create_dir(path.join("alpha")).unwrap();
            fs::create_dir(path.join("empty")).unwrap();
            let socket_path=path.join("native.sock");
            assert!(socket_path.as_os_str().as_encoded_bytes().len()<108);
            let socket=std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
            fs::set_permissions(path.join("native.sock"),fs::Permissions::from_mode(0o600)).unwrap();
            let root=Arc::new(Root::open(&path,Instant::now()+WORK).unwrap());
            Self{path,root,_socket:socket}
        }
        fn context(&self,proof:&AuthenticatedPeer,connection:Uuid,expires:Instant)->Arc<DirectoryContext> {
            let process=process_evidence(std::process::id()).unwrap();
            let socket=fs::metadata(self.path.join("native.sock")).unwrap();
            Arc::new(DirectoryContext{
                peer:proof.peer_id().clone(),peer_epoch:proof.epoch(),connection,
                physical:OriginalSource{server_id:Uuid::random(),incarnation:Uuid::random(),socket_path:self.path.join("native.sock"),
                    evidence:ServerEvidence{linux_boot_id:process.linux_boot_id.clone(),pid:process.pid,pid_start_ticks:process.start_ticks,
                        socket_device:socket.dev(),socket_inode:socket.ino()},invalidation:Arc::new(AtomicU64::new(0)),
                    executable: InstalledExecutablePin::capture(
                        &fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
                        &ServerEvidence {
                            linux_boot_id: process.linux_boot_id.clone(),
                            pid: process.pid,
                            pid_start_ticks: process.start_ticks,
                            socket_device: socket.dev(),
                            socket_inode: socket.ino(),
                        },
                    )
                    .unwrap(),
                },
                server_process:process,project:Uuid::random(),origin:None,root:self.root.clone(),expires,
            })
        }
        fn catalogue(&self,leases:&Leases,context:Arc<DirectoryContext>)->Arc<Catalogue> {
            let reservation=leases.reserve(&context.peer,1001).unwrap();
            let directory=self.root.select(&[],context.expires).unwrap();
            let children=self.root.list(&directory,context.expires).unwrap();
            reservation.finish(children.len()+1);
            let directory=Arc::new(OwnedDirectory{value:directory,_reservation:reservation.clone()});
            let entries=children.into_iter().map(|value|(Uuid::random(),Arc::new(OwnedDirectory{value,_reservation:reservation.clone()}))).collect();
            Arc::new(Catalogue{context,parent:None,directory,entries,limit:1,cursors:vec![Uuid::random()]})
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self){fs::remove_dir_all(&self.path).unwrap();}
    }
    fn peers()->(AuthenticatedPeer,AuthenticatedPeer) {
        let mut store=Storage::memory([19;32],0).unwrap();
        let mut paired=|key| {
            let device=PairedDevice{device_id:Uuid::random(),endpoint_id:EndpointId(iroh::SecretKey::from_bytes(&[key;32]).public().to_string()),
                device_name:Label::new("directory fixture").unwrap(),paired_at:Uint(0),last_seen_at:None};
            let grant=store.pair_device(&device).unwrap();
            store.authenticated_hello(&device.device_id,grant).unwrap()
        };
        (paired(27),paired(28))
    }
    fn params(context:&DirectoryContext)->CreationDirectoriesParams {
        CreationDirectoriesParams{server_id:context.physical.server_id.clone(),project_id:context.project.clone(),
            source:context.origin.as_ref().map(|origin|origin.source.clone()),parent:None,cursor:None,limit:Some(Uint(1))}
    }
    #[test]
    fn delayed_creation_original_source_retains_the_configured_executable_pin() {
        let fixture = Fixture::new();
        let (peer, _) = peers();
        let context = fixture.context(&peer, Uuid::random(), Instant::now() + LEASE);
        let physical = &context.physical;
        let snapshot: Snapshot = serde_json::from_value(json!({"version":"0.9.1","protocol":22,
            "workspaces":[],"tabs":[],"panes":[],"agents":[]})).unwrap();
        let source = Source {
            server_id: physical.server_id.clone(),
            incarnation: physical.incarnation.clone(),
            socket_path: physical.socket_path.clone(),
            evidence: physical.evidence.clone(),
            executable: physical.executable.clone(),
            snapshot: Arc::new(snapshot),
            revision: 0,
            invalidation: physical.invalidation.clone(),
        };
        source.recheck().unwrap();
        let retained = OriginalSource::capture(source.clone());
        assert!(retained.matches(&source));
        let configured = fixture.path.join("configured-herdr");
        std::os::unix::fs::symlink(&source.executable.canonical_path, &configured).unwrap();
        let mut replacement = source.clone();
        replacement.executable =
            InstalledExecutablePin::capture(&configured, &source.evidence).unwrap();
        assert!(
            !retained.matches(&replacement),
            "a delayed chooser must not drop its original executable pin"
        );
        let mut refreshed = source.clone();
        refreshed.revision = 1;
        assert!(
            retained.matches(&refreshed),
            "inventory revision is not physical replacement"
        );
    }
    #[test]
    fn directory_pages_bind_real_names_to_peer_connection_project_parent_source_and_absolute_expiry() {
        let fixture=Fixture::new();let (peer,foreign)=peers();let connection=Uuid::random();
        let leases=Leases::default();
        let mut context=fixture.context(&peer,connection.clone(),Instant::now()+LEASE);
        let origin=Origin{source:ArtifactSource::Session{target:Target{session_id:Uuid::random(),generation:Uuid::random()}},
            epoch:Uuid::random(),pane:Id("w1:p1".into()),terminal:Id("terminal".into()),workspace:Id("w1".into()),tab:Id("w1:t1".into())};
        Arc::get_mut(&mut context).unwrap().origin=Some(origin);
        let catalogue=fixture.catalogue(&leases,context.clone());
        let first=page(&catalogue,0).unwrap();
        assert_eq!(first.entries.len(),1);
        let mut request=params(&context);request.cursor=first.next_cursor.clone();
        leases.publish(Some(&catalogue)).unwrap();
        let (_,offset)=leases.cursor(&peer,&connection,&request).unwrap();
        let second=page(&catalogue,offset).unwrap();
        assert_eq!(second.entries.len(),1);
        assert_ne!(first.entries[0].reference,second.entries[0].reference);
        let mut names=vec![first.entries[0].name.0.clone(),second.entries[0].name.0.clone()];
        names.sort();assert_eq!(names,vec!["alpha","empty"]);
        assert!(second.next_cursor.is_none());
        assert_eq!(catalogue.context.expires,context.expires,"paging must not renew the lease");
        for mixed in 0..5 {
            let mut bad=request.clone();
            match mixed {
                0=>bad.project_id=Uuid::random(),
                1=>bad.server_id=Uuid::random(),
                2=>bad.parent=Some(first.entries[0].reference.clone()),
                3=>bad.source=None,
                _=>bad.limit=Some(Uint(2)),
            }
            assert_eq!(leases.cursor(&peer,&connection,&bad).err(),Some(ErrorCode::CursorExpired));
        }
        assert_eq!(leases.cursor(&foreign,&connection,&request).err(),Some(ErrorCode::CursorExpired));
        assert_eq!(leases.cursor(&peer,&Uuid::random(),&request).err(),Some(ErrorCode::CursorExpired));
        let mut wrong_epoch=fixture.context(&peer,connection.clone(),Instant::now()+LEASE);
        Arc::get_mut(&mut wrong_epoch).unwrap().peer_epoch=peer.epoch()+1;
        assert_eq!(wrong_epoch.accepts(&peer,&connection,&wrong_epoch.physical.server_id,&wrong_epoch.project),Err(ErrorCode::CursorExpired));
        let expired=fixture.context(&peer,connection.clone(),Instant::now()-Duration::from_millis(1));
        assert_eq!(expired.accepts(&peer,&connection,&expired.physical.server_id,&expired.project),Err(ErrorCode::CursorExpired));
        leases.release_connection(&connection);
        assert_eq!(leases.reference(&peer,&connection,&first.entries[0].reference,&context.physical.server_id,&context.project).err(),Some(ErrorCode::CursorExpired));
    }
    #[test]
    fn retiring_metadata_keeps_descriptor_budget_until_the_real_creation_owner_drops() {
        let fixture=Fixture::new();let (peer,_)=peers();let connection=Uuid::random();
        let leases=Leases::default();let context=fixture.context(&peer,connection.clone(),Instant::now()+LEASE);
        let catalogue=fixture.catalogue(&leases,context.clone());
        let id=catalogue.entries[0].0.clone();leases.publish(Some(&catalogue)).unwrap();
        let (_,held)=leases.reference(&peer,&connection,&id,&context.physical.server_id,&context.project).unwrap();
        drop(catalogue);
        leases.release_peer(peer.peer_id());
        assert_eq!(leases.quota.lock().unwrap().fds,3);
        assert!(fixture.root.revalidate(&held.value,context.expires).is_ok());
        assert_eq!(leases.reference(&peer,&connection,&id,&context.physical.server_id,&context.project).err(),Some(ErrorCode::CursorExpired));
        drop(held);
        assert_eq!(leases.quota.lock().unwrap().fds,0);
        assert!(leases.quota.lock().unwrap().peers.is_empty());
    }
    #[test]
    fn selected_directory_replacement_refuses_without_a_pathname_or_root_fallback() {
        let fixture=Fixture::new();let (peer,_)=peers();let leases=Leases::default();
        let context=fixture.context(&peer,Uuid::random(),Instant::now()+LEASE);
        let reservation=leases.reserve(peer.peer_id(),1).unwrap();
        let value=fixture.root.select(&["alpha".to_owned()],context.expires).unwrap();reservation.finish(1);
        let reference=Uuid::random();
        let request=SessionCreateParams{boot_id:Uuid::random(),server_id:context.physical.server_id.clone(),project_id:context.project.clone(),
            provider:Provider::Omp,name:Label::new("explicit creation").unwrap(),directory_ref:Some(reference.clone()),placement:None};
        let selection=CreationSelection{context,directory:Arc::new(OwnedDirectory{value,_reservation:reservation}),
            reference:Some(reference),placement:None,shell:Mutex::new(None),created:Mutex::new(None),unproved:AtomicBool::new(false)};
        assert_eq!(selection.directory(&request).unwrap(),fixture.path.join("alpha"));
        fs::rename(fixture.path.join("alpha"),fixture.path.join("retained-alpha")).unwrap();
        fs::create_dir(fixture.path.join("alpha")).unwrap();
        assert!(selection.directory(&request).is_err());
        assert!(selection.unproved());
        assert!(selection.shell.lock().unwrap().is_none());
    }

    #[test]
    fn terminal_creation_accepts_an_empty_selected_directory_but_never_a_replacement() {
        let fixture=Fixture::new();let (peer,_)=peers();let leases=Leases::default();
        let context=fixture.context(&peer,Uuid::random(),Instant::now()+LEASE);
        let reservation=leases.reserve(peer.peer_id(),1).unwrap();
        let value=fixture.root.select(&["empty".to_owned()],context.expires).unwrap();reservation.finish(1);
        let reference=Uuid::random();
        let params=TerminalCreateParams{boot_id:Uuid::random(),server_id:context.physical.server_id.clone(),workspace_id:Id::new("w1").unwrap(),
            project_id:context.project.clone(),directory_ref:Some(reference.clone()),source:None};
        let selection=CreationSelection{context,directory:Arc::new(OwnedDirectory{value,_reservation:reservation}),reference:Some(reference),placement:None,
            shell:Mutex::new(None),created:Mutex::new(None),unproved:AtomicBool::new(false)};
        assert_eq!(selection.terminal_directory(&params).unwrap(),fixture.path.join("empty"));
        fs::rename(fixture.path.join("empty"),fixture.path.join("retained-empty")).unwrap();
        fs::create_dir(fixture.path.join("empty")).unwrap();
        assert!(selection.terminal_directory(&params).is_err());
        assert!(selection.unproved());assert!(selection.shell.lock().unwrap().is_none());
    }

    #[test]
    fn actual_kernel_cwd_must_match_the_selected_object_before_any_input_owner_exists() {
        let fixture=Fixture::new();let (peer,_)=peers();let leases=Leases::default();
        let context=fixture.context(&peer,Uuid::random(),Instant::now()+LEASE);
        let reservation=leases.reserve(peer.peer_id(),1).unwrap();
        let value=fixture.root.select(&[],context.expires).unwrap();reservation.finish(1);
        let selection=CreationSelection{context,directory:Arc::new(OwnedDirectory{value,_reservation:reservation}),
            reference:None,placement:None,shell:Mutex::new(None),created:Mutex::new(None),unproved:AtomicBool::new(false)};
        let terminal=TerminalCreateParams{boot_id:Uuid::random(),server_id:selection.context.physical.server_id.clone(),workspace_id:Id::new("w1").unwrap(),
            project_id:selection.context.project.clone(),directory_ref:None,source:None};
        assert_eq!(selection.terminal_directory(&terminal).unwrap(),fixture.path);
        let process=process_evidence(std::process::id()).unwrap();
        let actual=kernel_cwd(&process).unwrap();
        let actual_object=actual.metadata().unwrap();
        let current_object=fs::metadata(std::env::current_dir().unwrap()).unwrap();
        assert_eq!((actual_object.dev(),actual_object.ino()),(current_object.dev(),current_object.ino()));
        assert_eq!(selection.retain_shell(process),Err(ErrorCode::NotReady));
        assert!(selection.unproved());
        assert!(selection.shell.lock().unwrap().is_none(),"no input-sending prepared shell can be retained from a mismatched cwd");
    }
    #[test]
    fn serialized_publication_retires_source_connection_server_and_peer_without_late_page_grants() {
        for retirement in 0..4 {
            let fixture=Fixture::new();let (peer,_)=peers();let state=State::default();
            let connection=state.connect(&peer);
            let mut context=fixture.context(&peer,connection.clone(),Instant::now()+LEASE);
            let target=Target{session_id:Uuid::random(),generation:Uuid::random()};
            let display:SessionState=serde_json::from_value(json!({
                "summary":{"sessionId":target.session_id,"generation":target.generation,"serverId":context.physical.server_id,
                    "workspaceId":"w1","paneId":"w1:p1","terminalId":"terminal","provider":"omp","nativeId":"original",
                    "title":"directory source","status":"idle","connectivity":"ready",
                    "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},
                    "historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
                "desktopSelection":"unknown","activity":{"state":"idle"}
            })).unwrap();
            let epoch=state.begin_install(display.clone()).unwrap();
            state.finish_install(&epoch,ProviderSnapshot{state:display,history:vec![],pending:vec![],older_available:false,native_older:None}).unwrap();
            Arc::get_mut(&mut context).unwrap().origin=Some(Origin{
                source:ArtifactSource::Session{target:target.clone()},epoch:epoch.clone(),pane:Id("w1:p1".into()),
                terminal:Id("terminal".into()),workspace:Id("w1".into()),tab:Id("w1:t1".into()),
            });
            let server:Server=serde_json::from_value(json!({"serverId":context.physical.server_id,"name":"logical fixture",
                "connectivity":"online","canCreate":{"omp":true,"codex":true}})).unwrap();
            let project=Project{project_id:context.project.clone(),label:Label::new("authorized fixture").unwrap(),available:true,reason:None};
            let hierarchy:NativeHierarchy=serde_json::from_value(json!({
                "serverId":context.physical.server_id,"serverIncarnation":context.physical.incarnation,"freshness":"current",
                "workspaces":[{"workspaceId":"w1","number":1,"label":"workspace","focused":true,"activeTabId":"w1:t1","agentStatus":"idle"}],
                "tabs":[{"tabId":"w1:t1","workspaceId":"w1","number":1,"label":"tab","focused":true,"agentStatus":"idle"}],
                "panes":[{"paneId":"w1:p1","terminalId":"terminal","workspaceId":"w1","tabId":"w1:t1","label":"pane","title":"original","agent":"omp",
                    "focused":true,"agentStatus":"idle","target":target}]
            })).unwrap();
            let install=|hierarchy|state.replace_inventory(vec![server.clone()],vec![project.clone()],vec![],vec![],
                vec![hierarchy],&[(&target,&epoch)],|_|Ok(())).unwrap();
            install(hierarchy.clone());
            let leases=&state.creation_directories;
            let catalogue=fixture.catalogue(leases,context.clone());
            let first=page(&catalogue,0).unwrap();let reference=first.entries[0].reference.clone();
            state.creation_directories_response(&context,Uuid::random(),first.clone(),||leases.publish(Some(&catalogue))).unwrap();
            assert!(leases.reference(&peer,&connection,&reference,&context.physical.server_id,&context.project).is_ok());
            if retirement==0 {
                state.topology_synchronizing(&context.physical.server_id).unwrap();
                state.prune_creation_directories();
                assert!(leases.reference(&peer,&connection,&reference,&context.physical.server_id,&context.project).is_ok(),"a transient revision gap is not source retirement");
                let attempted=AtomicUsize::new(0);
                assert!(state.creation_directories_response(&context,Uuid::random(),first.clone(),||{attempted.fetch_add(1,Ordering::SeqCst);Ok(())}).is_err());
                assert_eq!(attempted.load(Ordering::SeqCst),0,"stale native view cannot publish handles");
                state.topology_revalidated(&target,&epoch,&Id("w1".into()),&Id("w1:p1".into()),||Ok(())).unwrap();
                install(hierarchy.clone());
            }
            match retirement {
                0=>{state.begin_source_install(&target).unwrap();}
                1=>{state.connect(&peer);}
                2=>{let mut replacement=hierarchy;replacement.server_incarnation=Uuid::random();install(replacement);}
                _=>state.revoke(peer.peer_id()),
            }
            assert_eq!(leases.reference(&peer,&connection,&reference,&context.physical.server_id,&context.project).err(),Some(ErrorCode::CursorExpired));
            let attempted=AtomicUsize::new(0);
            assert!(state.creation_directories_response(&context,Uuid::random(),first,||{attempted.fetch_add(1,Ordering::SeqCst);Ok(())}).is_err());
            assert_eq!(attempted.load(Ordering::SeqCst),0,"a late old-owner reply must not grant references");
        }
    }
}
