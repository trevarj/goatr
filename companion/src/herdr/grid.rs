// SPDX-License-Identifier: GPL-3.0-or-later
//! One original stock endpoint, one outstanding public envelope, no terminal history.
use super::*;
use tokio::{sync::{mpsc,oneshot,watch},time::{Instant,timeout_at}};

pub(crate) fn before(deadline:Instant)->SecurityResult<()> {
    if Instant::now()>=deadline {Err(ErrorCode::Timeout)}else{Ok(())}
}
// ponytail: conservative 64MiB per actual reader/decode/replacement owner; two owners share
// 128MiB. Metadata removal never releases a permit while its native owner is still alive.
fn decode_slots()->Arc<tokio::sync::Semaphore> {
    static SLOTS:std::sync::LazyLock<Arc<tokio::sync::Semaphore>>=std::sync::LazyLock::new(||Arc::new(tokio::sync::Semaphore::new(2)));
    SLOTS.clone()
}
pub(crate) struct GridLease {
    pub id:Uuid,
    stopped:watch::Sender<bool>,
    source_changed:watch::Sender<u64>,
    graceful:AtomicBool,
    popup:AtomicBool,
    coherent:AtomicBool,
    topology:Arc<Topology>,
    context:shells::LogicalShellContext,
    socket:Mutex<Option<OwnedFd>>,
    commands:mpsc::Sender<Command>,
}
struct PendingGrid {state:Arc<State>,connection:Uuid,subscription:Uuid,published:bool}
impl Drop for PendingGrid {
    fn drop(&mut self){if !self.published{self.state.grid_failed(&self.connection,&self.subscription);}}
}
enum ResizeCompletion {Confirmed,Refused(ErrorCode),Uncertain}
struct ResizeRequest {columns:u16,rows:u16,deadline:Instant,reply:oneshot::Sender<ResizeCompletion>}
struct PendingResize {columns:u16,rows:u16,after:u64,deadline:Instant,reply:oneshot::Sender<ResizeCompletion>}
#[derive(Clone,Copy)]
pub(super) enum InputCompletion {Written,Refused(ErrorCode),Uncertain}
struct InputRequest {bytes:Vec<u8>,progress:Arc<pairing::ShellInputProgressOwner>,deadline:Instant,reply:oneshot::Sender<InputCompletion>}
enum Command {Resize(ResizeRequest),Input(InputRequest)}
impl GridLease {
    fn new(topology:Arc<Topology>,context:shells::LogicalShellContext)->(Arc<Self>,mpsc::Receiver<Command>) {
        let (stopped,_)=watch::channel(false);let (source_changed,_)=watch::channel(context.source.revision);
        let (commands,receive)=mpsc::channel(1);
        (Arc::new(Self{id:Uuid::random(),stopped,source_changed,graceful:AtomicBool::new(false),popup:AtomicBool::new(false),coherent:AtomicBool::new(false),topology,context,socket:Mutex::new(None),commands}),receive)
    }
    pub(crate) async fn authorize(&self,security:&Security,auth:&Authorized,deadline:Instant)->SecurityResult<tokio::sync::OwnedMutexGuard<()>> {
        before(deadline)?;
        let guard=tokio::select!{biased;_=self.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,security.guard(auth))=>result.map_err(|_|ErrorCode::Timeout)??};
        before(deadline)?;if !self.active(){return Err(ErrorCode::StaleSession);}Ok(guard)
    }
    pub fn cancel(&self){self.stopped.send_replace(true);}
    pub(crate) fn release(&self){self.graceful.store(true,Ordering::Release);self.cancel();}
    pub fn active(&self)->bool{!*self.stopped.borrow()}
    pub async fn cancelled(&self){let mut changed=self.stopped.subscribe();while !*changed.borrow_and_update(){if changed.changed().await.is_err(){break;}}}
    pub(crate) fn check_current(&self)->SecurityResult<()> {
        if !self.active(){return Err(ErrorCode::StaleSession);}
        self.topology.grid_source(&self.context).map(|_|())
    }
    pub async fn current(&self,deadline:Instant)->SecurityResult<()> {
        let mut changed=self.source_changed.subscribe();
        loop {
            before(deadline)?;
            let update=tokio::select!{biased;_=self.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,self.topology.updates.lock())=>result.map_err(|_|ErrorCode::Timeout)?};
            let result=self.check_current();drop(update);
            match result {
                Ok(())=>return before(deadline),
                Err(ErrorCode::NotReady)=>{
                    tokio::select!{biased;_=self.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,changed.changed())=>result.map_err(|_|ErrorCode::Timeout)?.map_err(|_|ErrorCode::StaleSession)?};
                }
                Err(code)=>return Err(code),
            }
        }
    }
    pub(crate) fn server(&self)->&Uuid{&self.context.source.server_id}
    pub(crate) fn shell(&self)->(&ShellTarget,&Uuid){(&self.context.target,&self.context.proof.source_epoch)}
    pub(crate) fn accepts_source(&self,current:&Source)->bool {
        let route=&self.context.proof.identity;
        self.context.source.same_physical_identity(current)&&current.snapshot.pane(&route.pane_id).is_ok_and(|pane|
            pane.terminal_id==route.terminal_id&&pane.workspace_id==route.workspace_id&&pane.tab_id==route.tab_id&&shells::ordinary_shell(&current.snapshot,pane))
    }
    pub(crate) fn retains_inventory(&self,summary:&ShellSummary,graph:&crate::protocol::NativeHierarchy)->bool {
        let route=&self.context.proof.identity;
        summary.target==self.context.target&&summary.server_id==self.context.source.server_id
            &&summary.workspace_id==route.workspace_id&&summary.tab_id==route.tab_id&&summary.pane_id==route.pane_id&&summary.terminal_id==route.terminal_id
            &&graph.server_id==self.context.source.server_id&&graph.server_incarnation==self.context.source.incarnation
            &&graph.panes.iter().any(|pane|pane.pane_id==route.pane_id&&pane.terminal_id==route.terminal_id
                &&pane.workspace_id==route.workspace_id&&pane.tab_id==route.tab_id&&pane.agent.is_none()
                &&pane.target.is_none()&&pane.unbound_row_id.is_none()
                &&if graph.freshness==NativeFreshness::Stale {pane.shell_target.as_ref().is_none_or(|target|target==&self.context.target)}
                    else{pane.shell_target.as_ref()==Some(&self.context.target)})
    }
    pub(crate) fn source_updated(&self,source:&Source){self.source_changed.send_replace(source.revision);}
    pub(crate) fn inventory_updated(&self){let revision=*self.source_changed.borrow();self.source_changed.send_replace(revision);}
    fn fence_socket(&self)->SecurityResult<OwnedFd> {
        if !self.active(){return Err(ErrorCode::StaleSession);}
        self.socket.lock().map_err(|_|ErrorCode::NotReady)?.as_ref().ok_or(ErrorCode::NotReady)?.as_fd().try_clone_to_owned().map_err(policy)
    }
    pub(super) fn screen(&self)->SecurityResult<()> {
        let current=self.topology.grid_source(&self.context)?;
        if current.snapshot.agents.iter().any(|agent|agent.launch_pending)
            ||current.snapshot.panes.iter().any(|pane|pane.restore_error.is_some())
        {return Err(ErrorCode::NotReady);}Ok(())
    }
    pub(super) fn input_ready(&self)->SecurityResult<()> {
        if !self.coherent.load(Ordering::Acquire)||self.popup.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        self.screen()?;
        self.topology.state.validate_shell(&self.context.target,&self.context.proof.source_epoch,shells::ShellAccess::Input)
    }
}
impl Topology {
    fn grid_source(&self,context:&shells::LogicalShellContext)->SecurityResult<Source> {
        context.source.recheck_identity().map_err(|_|ErrorCode::StaleSession)?;
        self.terminal_source_control(&context.source.server_id)?;
        let inner=self.inner.lock().expect("topology mutex");
        let binding=inner.shell_bindings.get(&context.target.shell_id).filter(|binding|binding.target==context.target&&binding.proof.logical&&binding.read&&binding.proof.same_authority(&context.proof)).ok_or(ErrorCode::StaleSession)?;
        let current=inner.servers.get(&context.source.server_id).and_then(|record|record.source.as_ref()).ok_or(ErrorCode::StaleSession)?;
        if !context.source.same_physical_identity(current){return Err(ErrorCode::StaleSession);}
        if !current.recheck_readiness().map_err(|_|ErrorCode::StaleSession)?{return Err(ErrorCode::NotReady);}
        let route=&context.proof.identity;
        let pane=current.snapshot.pane(&route.pane_id).map_err(|_|ErrorCode::StaleSession)?;
        if pane.terminal_id!=route.terminal_id||pane.workspace_id!=route.workspace_id||pane.tab_id!=route.tab_id||!shells::ordinary_shell(&current.snapshot,pane){return Err(ErrorCode::StaleSession);}
        if binding.source.as_ref().is_none_or(|source|!source.same_physical_identity(current)){return Err(ErrorCode::StaleSession);}
        if self.state.creation_shell_blocked(&current.server_id,&current.incarnation,&route.pane_id,&route.terminal_id){return Err(ErrorCode::StaleSession);}
        self.state.validate_shell(&context.target,&context.proof.source_epoch,shells::ShellAccess::Read)?;
        Ok(current.clone())
    }
    pub(crate) async fn grid_open(self:&Arc<Self>,security:Arc<Security>,auth:Arc<Authorized>,request:Uuid,body:RequestBody,deadline:Instant)->SecurityResult<()> {
        before(deadline)?;security.state.grid_available(&auth.connection_id)?;
        let RequestBody::ShellGridSubscribe(params)=body else{return Err(ErrorCode::NotReady);};
        let context={let _update=timeout_at(deadline,self.updates.lock()).await.map_err(|_|ErrorCode::Timeout)?;self.logical_shell_context(&params.target)?};
        let current=self.grid_source(&context)?;
        let pane=current.snapshot.pane(&context.proof.identity.pane_id).map_err(policy)?;
        let (columns,rows)=current.snapshot.layout_seed(pane).map_err(policy)?;
        let memory=decode_slots().try_acquire_owned().map_err(|_|ErrorCode::ResourceLimit)?;
        let (lease,commands)=GridLease::new(self.clone(),context);
        {let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline)?;}
        let mut reservation=PendingGrid{state:security.state.clone(),connection:auth.connection_id.clone(),subscription:lease.id.clone(),published:false};
        let mut session=tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,endpoint::Session::connect(&lease.context,Arc::downgrade(&lease),auth.clone()))=>result.map_err(|_|ErrorCode::Timeout)??};
        *lease.socket.lock().map_err(|_|ErrorCode::NotReady)?=Some(session.client.fence_socket().map_err(policy)?);
        let opened=async {
            ready(&lease,&security,&auth,deadline).await?;
            session.negotiate(columns,rows).await?;
            ready(&lease,&security,&auth,deadline).await?;lease.screen()?;session.activate().await?;
            ready(&lease,&security,&auth,deadline).await?;session.focus_workspace().await?;
            ready(&lease,&security,&auth,deadline).await?;session.focus_tab().await?;
            ready(&lease,&security,&auth,deadline).await?;session.focus_pane().await?;
            while !session.coherent(){session.next().await?;}
            ready(&lease,&security,&auth,deadline).await?;
            let surface=session.surface.as_ref().ok_or(ErrorCode::NotReady)?;
            lease.popup.store(surface.popup(),Ordering::Release);
            lease.coherent.store(true,Ordering::Release);
            surface.projection(0)
        };
        let projection=tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,opened)=>result.map_err(|_|ErrorCode::Timeout)??};
        let mut bytes=0;
        let response=ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{target:params.target,subscription:lease.id.clone(),seq:ZeroValue(0),frame:projection.begin.clone()});
        let receiver={let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;security.state.grid_open_response(&auth.connection_id,request,response,lease.clone(),deadline,&mut bytes)?};
        reservation.published=true;
        tokio::spawn(async move {
            let _memory=memory;
            let outcome=async {
                written(&lease,receiver,deadline).await?;
                publish_cells(&lease,&security,&auth,projection,deadline,&mut bytes).await?;
                run(&lease,&security,&auth,&mut session,commands).await
            }.await;
            lease.socket.lock().expect("grid socket mutex").take();
            if lease.graceful.load(Ordering::Acquire){session.deactivate().await;}
            if outcome.is_err(){security.state.grid_failed(&auth.connection_id,&lease.id);}
            lease.cancel();
        });
        Ok(())
    }
    pub(crate) fn validate_shell_resize(&self,state:&State,connection:&Uuid,params:&ShellResizeParams)->SecurityResult<Arc<GridLease>> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        if !lease.coherent.load(Ordering::Acquire)||lease.popup.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        lease.screen()?;Ok(lease)
    }
    pub(crate) async fn shell_resize_once(&self,security:&Security,auth:&Authorized,request:&Uuid,params:&ShellResizeParams,deadline:Instant)->SecurityResult<()> {
        params.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let lease=self.validate_shell_resize(&security.state,&auth.connection_id,params)?;
        let fd=lease.fence_socket()?;
        let columns=params.columns.0 as u16;let rows=params.rows.0 as u16;
        security.boundary(auth,request.clone(),||self.validate_shell_resize(&security.state,&auth.connection_id,params).map(|_|()),fd,async {
            let (reply,receive)=oneshot::channel();
            let sent=lease.commands.try_send(Command::Resize(ResizeRequest{columns,rows,deadline,reply}));
            let completion=match sent {Ok(())=>receive.await.unwrap_or(ResizeCompletion::Uncertain),Err(mpsc::error::TrySendError::Full(_))=>ResizeCompletion::Refused(ErrorCode::Busy),Err(mpsc::error::TrySendError::Closed(_))=>ResizeCompletion::Refused(ErrorCode::NotReady)};
            let (state,proof,error,result)=match completion {
                ResizeCompletion::Confirmed=>(ReceiptState::Succeeded,crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("original native surface dimensions confirmed".into())),None,Some(ActionResult::ShellResize(ActionResultShellResize{columns:Uint(u64::from(columns)),rows:Uint(u64::from(rows))}))),
                ResizeCompletion::Refused(code)=>(ReceiptState::Failed,crate::storage::TransitionProof::DefinitiveNoEffect(Id("original resize refused before dispatch".into())),Some(WireError{code,message:SafeMessage("Terminal resize was refused before dispatch".into())}),None),
                ResizeCompletion::Uncertain=>{lease.cancel();(ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome,None,None)},
            };
            crate::storage::ReceiptTransition{state,proof,result,error,at:pairing::now_ms()}
        }).await
    }
    pub(crate) fn validate_shell_input(&self,state:&State,connection:&Uuid,params:&ShellInputParams)->SecurityResult<Arc<GridLease>> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        lease.input_ready()?;Ok(lease)
    }
    pub(crate) async fn await_shell_input(&self,state:&State,connection:&Uuid,params:&ShellInputParams,deadline:Instant)->SecurityResult<()> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        // Subscribe-before-recheck is owned by this exact lease; no authorization/State lock
        // is held while an ordinary same-original topology observation completes.
        lease.current(deadline).await?;lease.input_ready()
    }
    pub(crate) async fn shell_input_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&ShellInputParams,deadline:Instant)->SecurityResult<()> {
        params.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let lease=self.validate_shell_input(&security.state,&auth.connection_id,params)?;
        // The existing single native owner reserves its command slot BEFORE frame allocation.
        let permit=lease.commands.clone().try_reserve_owned().map_err(|error|match error {
            mpsc::error::TrySendError::Full(_)=>ErrorCode::Busy,mpsc::error::TrySendError::Closed(_)=>ErrorCode::NotReady,
        })?;
        let bytes=endpoint::pane_input(&lease.context.proof.identity.pane_id,&params.input)?;
        let fd=lease.fence_socket()?;
        security.shell_input_boundary(auth,action.clone(),params.target.clone(),lease.context.proof.source_epoch.clone(),fd,
            ||self.validate_shell_input(&security.state,&auth.connection_id,params).map(|_|()),|checkpoint|async move {
                let progress=ShellDispatchProgress{frame_bytes:Some(bytes.len() as u32),socket_bytes_written:Some(0),frame_complete:Some(false)};
                let owner=match checkpoint.owner(&params.target,&lease.context.proof.source_epoch){
                    Ok(owner)=>Arc::new(owner),Err(_)=>return input_transition(ShellDispatchProgress::default(),InputCompletion::Uncertain),
                };
                if owner.retain(&progress).is_err(){return input_transition(progress,InputCompletion::Uncertain);}
                if checkpoint.checkpoint().await.is_err(){return input_transition(progress,InputCompletion::Refused(ErrorCode::NotReady));}
                let (reply,receive)=oneshot::channel();
                permit.send(Command::Input(InputRequest{bytes,progress:owner.clone(),deadline,reply}));
                let completion=receive.await.unwrap_or(InputCompletion::Uncertain);
                let progress=owner.progress().unwrap_or(progress);
                if checkpoint.checkpoint().await.is_err(){lease.cancel();return input_transition(progress,InputCompletion::Uncertain);}
                if matches!(completion,InputCompletion::Uncertain){lease.cancel();}
                input_transition(progress,completion)
            }).await
    }
}
fn input_transition(progress:ShellDispatchProgress,completion:InputCompletion)->crate::storage::ReceiptTransition {
    let (state,proof,error)=match completion {
        InputCompletion::Written if progress.complete()=>(ReceiptState::Dispatched,crate::storage::TransitionProof::WriteCompleted,None),
        InputCompletion::Refused(code)=>(ReceiptState::Failed,crate::storage::TransitionProof::DefinitiveNoEffect(Id("original terminal input refused before native write".into())),
            Some(WireError{code,message:SafeMessage("Terminal input was refused before dispatch".into())})),
        _=>(ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome,None),
    };
    crate::storage::ReceiptTransition{state,proof,result:Some(ActionResult::ShellDispatch(ActionResultShellDispatch{progress})),error,at:pairing::now_ms()}
}
async fn ready(lease:&GridLease,security:&Security,auth:&Authorized,deadline:Instant)->SecurityResult<()> {
    lease.current(deadline).await?;
    let _guard=lease.authorize(security,auth,deadline).await?;lease.check_current()?;before(deadline)
}
async fn written(lease:&GridLease,receiver:oneshot::Receiver<()>,deadline:Instant)->SecurityResult<()> {
    before(deadline)?;
    tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,receiver)=>result.map_err(|_|ErrorCode::Timeout)?.map_err(|_|ErrorCode::NotReady)?};before(deadline)
}
async fn event(lease:&Arc<GridLease>,security:&Security,auth:&Authorized,body:EventBody,deadline:Instant,bytes:&mut usize)->SecurityResult<()> {
    lease.current(deadline).await?;
    let receiver={let _guard=lease.authorize(security,auth,deadline).await?;lease.check_current()?;security.state.grid_event(&auth.connection_id,lease,body,deadline,bytes)?};
    written(lease,receiver,deadline).await
}
async fn publish_cells(lease:&Arc<GridLease>,security:&Security,auth:&Authorized,projection:endpoint::Projection,deadline:Instant,bytes:&mut usize)->SecurityResult<()> {
    let revision=projection.begin.revision;let columns=projection.begin.columns.0 as usize;
    let mut cells=projection.cells.into_iter();let mut position=0usize;let mut index=0u64;
    while cells.len()>0 {
        let count=cells.len().min(512);let mut spans=Vec::new();let mut remaining=count;
        while remaining>0 {
            let column=position%columns;let take=remaining.min(columns-column);
            spans.push(GridSpan{row:Uint((position/columns) as u64),column:Uint(column as u64),cells:cells.by_ref().take(take).collect()});
            position+=take;remaining-=take;
        }
        event(lease,security,auth,EventBody::GridPart(GridPart{revision,index:Uint(index),spans}),deadline,bytes).await?;index+=1;
    }
    event(lease,security,auth,EventBody::GridCommit(GridCommit{revision}),deadline,bytes).await
}
async fn run(lease:&Arc<GridLease>,security:&Security,auth:&Authorized,session:&mut endpoint::Session,mut commands:mpsc::Receiver<Command>)->SecurityResult<()> {
    let mut revision=0u64;let mut pending:Option<PendingResize>=None;let mut coherence_deadline=None;let mut input:Option<InputRequest>=None;
    loop {
        let deadline=pending.as_ref().map(|value|value.deadline).into_iter().chain(coherence_deadline).chain(input.as_ref().map(|value|value.deadline)).min();
        enum Wake {Command(Command),Native(endpoint::Incoming),InputReady}
        let can_input=input.is_some()&&pending.is_none()&&session.coherent()&&!session.read_in_progress();
        let accept_command=input.is_none();
        let next=async {match deadline {Some(deadline)=>timeout_at(deadline,session.next()).await.map_err(|_|ErrorCode::Timeout)?,None=>session.next().await}};
        let abandoned=async {match (pending.as_mut(),input.as_mut()) {
            (Some(resize),Some(input))=>tokio::select!{_=resize.reply.closed()=>(),_=input.reply.closed()=>()},
            (Some(resize),None)=>resize.reply.closed().await,
            (None,Some(input))=>input.reply.closed().await,
            (None,None)=>std::future::pending::<()>().await,
        }};
        let wake=tokio::select!{biased;
            _=lease.cancelled()=>return Ok(()),
            _=abandoned=>{lease.cancel();return Err(ErrorCode::StaleSession);},
            result=next=>Wake::Native(result?),
            _=async {},if can_input=>Wake::InputReady,
            request=commands.recv(),if accept_command=>Wake::Command(request.ok_or(ErrorCode::StaleSession)?),
        };
        match wake {
            Wake::Command(Command::Resize(mut request))=>{
                if request.reply.is_closed(){continue;}
                let checked=if pending.is_some(){Err(ErrorCode::Busy)}else if !session.coherent()||session.read_in_progress(){Err(ErrorCode::NotReady)}else{ready(lease,security,auth,request.deadline).await.and_then(|()|lease.screen()).and_then(|()|if lease.popup.load(Ordering::Acquire){Err(ErrorCode::NotReady)}else{Ok(())})};
                if let Err(code)=checked {let _=request.reply.send(ResizeCompletion::Refused(code));continue;}
                let after=session.surface.as_ref().ok_or(ErrorCode::NotReady)?.revisions().2;
                let bytes=endpoint::resize(request.columns,request.rows)?;
                let sent=tokio::select!{biased;
                    _=lease.cancelled()=>Err(ErrorCode::StaleSession),
                    _=request.reply.closed()=>Err(ErrorCode::StaleSession),
                    result=timeout_at(request.deadline,session.send(&bytes))=>result.map_err(|_|ErrorCode::Timeout)?,
                };
                if sent.is_err(){let _=request.reply.send(ResizeCompletion::Uncertain);return Err(ErrorCode::NotReady);}
                pending=Some(PendingResize{columns:request.columns,rows:request.rows,after,deadline:request.deadline,reply:request.reply});
            }
            Wake::Command(Command::Input(request))=>{
                if request.reply.is_closed(){continue;}
                input=Some(request);
            }
            Wake::InputReady=>{
                // The preferred native poll may just have consumed a prefix/partial payload.
                // Keep this original bounded command until that SAME transaction is coherent.
                if pending.is_some()||!session.coherent()||session.read_in_progress(){continue;}
                let request=input.take().expect("selected original input");
                if request.reply.is_closed(){continue;}
                let checked=ready(lease,security,auth,request.deadline).await.and_then(|()|lease.input_ready());
                if let Err(code)=checked {let _=request.reply.send(InputCompletion::Refused(code));continue;}
                let sent=tokio::select!{biased;
                    _=lease.cancelled()=>Err(ErrorCode::StaleSession),
                    result=timeout_at(request.deadline,session.send_input(&request.bytes,&request.progress,&request.reply))=>result.map_err(|_|ErrorCode::Timeout)?,
                };
                if sent.is_err(){let _=request.reply.send(InputCompletion::Uncertain);lease.cancel();return Err(ErrorCode::NotReady);}
                let _=request.reply.send(InputCompletion::Written);
            }
            Wake::Native(incoming)=>{
                if let Some(surface)=&session.surface{lease.popup.store(surface.popup(),Ordering::Release);}
                lease.coherent.store(session.coherent(),Ordering::Release);
                if matches!(incoming,endpoint::Incoming::Chunk{..}){return Err(ErrorCode::NotReady);}
                if !session.coherent(){if coherence_deadline.is_none(){coherence_deadline=Some(session.transaction_deadline());}continue;}
                coherence_deadline=None;
                if !matches!(incoming,endpoint::Incoming::Surface){continue;}
                let deadline=session.transaction_deadline();
                ready(lease,security,auth,deadline).await?;
                let surface=session.surface.as_ref().ok_or(ErrorCode::NotReady)?;
                if pending.as_ref().is_some_and(|resize|surface.revisions().2>resize.after&&surface.dimensions()==(resize.columns,resize.rows)) {
                    let resize=pending.take().expect("matching resize");let _=resize.reply.send(ResizeCompletion::Confirmed);
                }
                revision=revision.checked_add(1).filter(|value|*value<=UINT_MAX).ok_or(ErrorCode::ResourceLimit)?;
                let projection=surface.projection(revision)?;let mut bytes=0;
                event(lease,security,auth,EventBody::GridBegin(GridBeginEvent{frame:projection.begin.clone()}),deadline,&mut bytes).await?;
                publish_cells(lease,security,auth,projection,deadline,&mut bytes).await?;
            }
        }
    }
}

#[cfg(test)]
pub(crate) struct GridQueueFixture {pub lease:Arc<GridLease>,pub source:Source,root:PathBuf,_listener:tokio::net::UnixListener,commands:Option<mpsc::Receiver<Command>>}
#[cfg(test)]
impl GridQueueFixture {
    pub fn topology(&self)->Arc<Topology>{self.lease.topology.clone()}
    pub fn opening(&self)->ResultBody {
        ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{target:self.lease.context.target.clone(),subscription:self.lease.id.clone(),seq:ZeroValue(0),frame:GridBegin{mode:GridMode::Replace,revision:Uint(0),columns:Uint(1),rows:Uint(1),cursor:None,alternate_screen:false,graphics_omitted:false,popup_blocked:false,part_count:Uint(1)}})
    }
}
#[cfg(test)]
impl Drop for GridQueueFixture {fn drop(&mut self){fs::remove_dir_all(&self.root).unwrap();}}
/// Real State/Topology/process/socket proof. Only the provider-specific old grid fixture is retired.
#[cfg(test)]
pub(crate) async fn queue_fixture(state:Arc<State>,server:Uuid)->GridQueueFixture {
    queue_fixture_on(state,server,None).await
}
#[cfg(test)]
async fn queue_fixture_on(state:Arc<State>,server:Uuid,existing:Option<Arc<Topology>>)->GridQueueFixture {
    use std::os::unix::fs::{DirBuilderExt,PermissionsExt};
    let root=fs::canonicalize("/tmp").unwrap().join(format!("goatr-grid-{}",Uuid::random().0));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let path=root.join("native.sock");let listener=tokio::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
    let evidence=Client::connect(&path).await.unwrap().evidence;
    let exe=fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let config=Config::parse(&format!("version=1\nruntime_dir={root:?}\nruntime_dir_agent={root:?}\nallow_terminal_control=true\n[executables]\nherdr={exe:?}\n")).unwrap();
    let storage=Arc::new(StorageOwner::new(Storage::memory([49;32],0).unwrap()).unwrap());
    let host=EndpointId(iroh::SecretKey::from_bytes(&[49;32]).public().to_string());
    let topology=match existing{Some(topology)=>topology,None=>Topology::new(config,storage,state.clone(),host).await.unwrap()};
    let snapshot:Snapshot=serde_json::from_value(json!({"version":"0.9.1","protocol":22,"focused_workspace_id":"w",
        "workspaces":[{"workspace_id":"w","number":1,"label":"fixture","focused":true,"active_tab_id":"t","agent_status":"idle"}],
        "tabs":[{"tab_id":"t","workspace_id":"w","number":1,"label":"fixture","focused":true,"agent_status":"idle"}],
        "panes":[{"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t","focused":true,"agent_status":"idle"}],"agents":[],
        "layouts":[{"workspace_id":"w","tab_id":"t","area":{"x":0,"y":0,"width":1,"height":1},"focused_pane_id":"p","panes":[{"pane_id":"p","rect":{"x":0,"y":0,"width":1,"height":1}}]}]})).unwrap();
    let executable = InstalledExecutablePin::capture(&exe, &evidence).unwrap();
    let source = Source {
        server_id: server.clone(),
        incarnation: Uuid::random(),
        socket_path: path.clone(),
        evidence,
        executable,
        snapshot: Arc::new(snapshot),
        revision: 0,
        invalidation: Arc::new(AtomicU64::new(0)),
    };
    topology.inner.lock().unwrap().servers.insert(server.clone(),ServerRecord{slot:"fixture".into(),name:Label("Grid fixture".into()),path,server_id:server.clone(),source:Some(source.clone()),reason:None,coarse_shell:None});
    topology.update_shells(&source).await.unwrap();
    topology.publish().unwrap();
    let context={let inner=topology.inner.lock().unwrap();let binding=inner.shell_bindings.values().find(|binding|binding.key.server()==&server).unwrap();shells::LogicalShellContext{target:binding.target.clone(),proof:binding.proof.clone(),source:source.clone()}};
    let (lease,commands)=GridLease::new(topology,context);
    GridQueueFixture{lease,source,root,_listener:listener,commands:Some(commands)}
}
#[cfg(test)]
mod boundary_tests {
    use super::*;
    fn connection(state:&State)->Uuid {
        let mut storage=Storage::memory([50;32],0).unwrap();
        let device=PairedDevice{device_id:Uuid::random(),endpoint_id:EndpointId(iroh::SecretKey::from_bytes(&[50;32]).public().to_string()),device_name:Label("Grid peer".into()),paired_at:Uint(0),last_seen_at:None};
        let epoch=storage.pair_device(&device).unwrap();state.connect(&storage.authenticated_hello(&device.device_id,epoch).unwrap())
    }
    #[tokio::test]
    async fn successful_creation_dirty_during_commit_keeps_the_exact_target_enrollable() {
        let state=Arc::new(State::default());let fixture=queue_fixture(state.clone(),Uuid::random()).await;
        let peer=Uuid::random();let action=Uuid::random();let target=fixture.lease.context.target.clone();
        let identity=&fixture.lease.context.proof.identity;let epoch=fixture.lease.context.proof.source_epoch.clone();
        let resources=ShellCreationResources{server_id:Some(fixture.source.server_id.clone()),workspace_id:Some(identity.workspace_id.clone()),
            tab_id:Some(identity.tab_id.clone()),pane_id:Some(identity.pane_id.clone()),terminal_id:Some(identity.terminal_id.clone()),created_target:Some(target.clone())};
        let fence=||shells::CreationShellFence{server:fixture.source.server_id.clone(),incarnation:Some(fixture.source.incarnation.clone()),
            baseline:HashMap::new(),created:Some((identity.pane_id.clone(),identity.terminal_id.clone()))};
        state.creation_shell_fence(peer.clone(),action.clone(),Some(fence()));let mut commits=0;
        fixture.lease.topology.commit_created_terminal(fixture.source.clone(),&peer,&action,&resources,&epoch,&mut ||{
            commits+=1;fixture.source.invalidation.store(1,Ordering::Release);state.topology_synchronizing(&fixture.source.server_id)?;Ok(())
        }).unwrap();
        assert_eq!(commits,1);
        assert!(!state.creation_shell_blocked(&fixture.source.server_id,&fixture.source.incarnation,&identity.pane_id,&identity.terminal_id));
        assert_eq!(fixture.lease.check_current(),Err(ErrorCode::NotReady));
        let mut current=fixture.source.clone();current.revision=1;
        fixture.lease.topology.inner.lock().unwrap().servers.get_mut(&current.server_id).unwrap().source=Some(current.clone());
        fixture.lease.topology.update_shells(&current).await.unwrap();fixture.lease.topology.publish().unwrap();
        assert_eq!(fixture.lease.topology.logical_shell_context(&target).unwrap().target,target);
        state.creation_shell_fence(peer.clone(),action.clone(),Some(fence()));
        assert_eq!(fixture.lease.topology.commit_created_terminal(current,&peer,&action,&resources,&epoch,&mut ||Err(ErrorCode::NotReady)),Err(ErrorCode::NotReady));
        assert!(state.creation_shell_blocked(&fixture.source.server_id,&fixture.source.incarnation,&identity.pane_id,&identity.terminal_id));
    }

    #[tokio::test]
    async fn refresh_captured_before_enrollment_preserves_the_receipt_target_when_that_same_refresh_resumes() {
        let state=Arc::new(State::default());let fixture=queue_fixture(state.clone(),Uuid::random()).await;
        let owner=fixture.lease.topology.clone();let original=fixture.lease.context.target.clone();let epoch=Uuid::random();
        let mut current=fixture.source.clone();let mut snapshot=(*current.snapshot).clone();
        let mut created=snapshot.panes[0].clone();created.pane_id=Id("p2".into());created.terminal_id=Id("n2".into());created.focused=false;
        snapshot.panes.push(created);
        snapshot.layouts[0].area.width=2;
        snapshot.layouts[0].panes.push(NativeLayoutPane{pane_id:Id("p2".into()),rect:endpoint::Rect{x:1,y:0,width:1,height:1}});
        current.snapshot=Arc::new(snapshot);current.revision=1;current.invalidation.store(1,Ordering::Release);
        owner.inner.lock().unwrap().servers.get_mut(&current.server_id).unwrap().source=Some(current.clone());
        let mut proof=fixture.lease.context.proof.clone();proof.identity.pane_id=Id("p2".into());proof.identity.terminal_id=Id("n2".into());proof.source_epoch=epoch.clone();
        let key=StableResourceKey::Shell{host_id:owner.host_id.clone(),server_id:current.server_id.clone(),server_incarnation:current.incarnation.clone(),terminal_id:Id("n2".into())};
        let (stored_key,stored_proof)=(key.clone(),proof.clone());
        let receipt_target=owner.storage.call(move|store|store.shell_identity(&stored_key,&stored_proof,false)).await.unwrap();
        let resources=ShellCreationResources{server_id:Some(current.server_id.clone()),workspace_id:Some(Id("w".into())),tab_id:Some(Id("t".into())),
            pane_id:Some(Id("p2".into())),terminal_id:Some(Id("n2".into())),created_target:Some(receipt_target.clone())};
        let (release,waiting)=std::sync::mpsc::channel::<()>();
        let (started,blocked)=oneshot::channel();let storage=owner.storage.clone();
        let blocker=tokio::spawn(async move {storage.call(move|_|{let _=started.send(());let _=waiting.recv();Ok(())}).await});
        blocked.await.unwrap();
        let refreshing={
            let owner=owner.clone();let source=current.clone();
            tokio::spawn(async move {let _update=owner.updates.lock().await;owner.update_shells(&source).await?;owner.publish()})
        };
        let deadline=Instant::now()+Duration::from_secs(5);
        timeout_at(deadline,async {
            loop {
                if owner.inner.lock().unwrap().shell_bindings.get(&original.shell_id).unwrap().source.is_none(){break;}
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert!(!refreshing.is_finished(),"the SAME refresh must remain paused after old capture and before its storage observation");
        let peer=Uuid::random();let action=Uuid::random();
        state.creation_shell_fence(peer.clone(),action.clone(),Some(shells::CreationShellFence{server:current.server_id.clone(),
            incarnation:Some(current.incarnation.clone()),baseline:HashMap::new(),created:Some((Id("p2".into()),Id("n2".into())))}));
        let mut commits=0;
        owner.commit_created_terminal(current.clone(),&peer,&action,&resources,&epoch,&mut ||{commits+=1;Ok(())}).unwrap();
        assert_eq!(commits,1);assert!(!refreshing.is_finished());
        drop(release);blocker.await.unwrap().unwrap();
        refreshing.await.unwrap().unwrap();
        assert_eq!(owner.logical_shell_context(resources.created_target.as_ref().unwrap()).unwrap().target,receipt_target);
        assert_eq!(owner.logical_shell_context(&original).unwrap().target,original);
        let id=receipt_target.shell_id.clone();
        let persisted=owner.storage.call(move|store|Ok(store.persisted_shells()?.into_iter().find(|row|row.target.shell_id==id).unwrap().target)).await.unwrap();
        assert_eq!(persisted,receipt_target,"that resumed refresh must not remint the receipt's stored generation");
    }

    #[tokio::test]
    async fn queued_input_waits_for_the_same_fragmented_native_frame_then_writes_once() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;
            let lease=fixture.lease.clone();let deadline=Instant::now()+Duration::from_secs(5);
            security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut session,mut peer)=endpoint::fixture::established(&lease.context,Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());
            lease.coherent.store(true,Ordering::Release);
            let incoming=endpoint::fixture::packet(&[17,0]);
            peer.write_all(&incoming[..2]).await.unwrap();
            timeout_at(deadline,async {
                loop {
                    {let next=session.next();tokio::pin!(next);assert!(futures_util::poll!(&mut next).is_pending());}
                    if session.read_in_progress(){break;}
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
            let input=ShellInput::Text{text:"x".into()};let bytes=endpoint::pane_input(&lease.context.proof.identity.pane_id,&input).unwrap();
            let action=Uuid::random();let params=ShellInputParams{boot_id:security.boot.clone(),target:lease.context.target.clone(),subscription:lease.id.clone(),input};
            security.reserve(&auth,Request{id:action.clone(),body:RequestBody::ShellInput(params.clone())},|_|Ok(()),|_|{}).await.unwrap();
            let commands=fixture.commands.take().unwrap();let fd=session.client.fence_socket().unwrap();
            let source=lease.context.proof.source_epoch.clone();let target=params.target.clone();
            security.shell_input_boundary(&auth,action.clone(),target.clone(),source,fd,||Ok(()),|checkpoint|{
                let secured=security.clone();let authorization=auth.clone();let lease=lease.clone();
                async move {
                    let owner=Arc::new(checkpoint.owner(&target,&lease.context.proof.source_epoch).unwrap());
                    owner.retain(&ShellDispatchProgress{frame_bytes:Some(bytes.len() as u32),socket_bytes_written:Some(0),frame_complete:Some(false)}).unwrap();
                    checkpoint.checkpoint().await.unwrap();
                    let (send,mut receive)=oneshot::channel();
                    lease.commands.try_send(Command::Input(InputRequest{bytes:bytes.clone(),progress:owner.clone(),deadline,reply:send})).unwrap_or_else(|_|panic!("original bounded input slot"));
                    let running=run(&lease,&secured,&authorization,&mut session,commands);tokio::pin!(running);
                    assert!(futures_util::poll!(&mut running).is_pending());
                    assert!(matches!(receive.try_recv(),Err(oneshot::error::TryRecvError::Empty)));
                    let mut probe=[0;1];assert_eq!(peer.try_read(&mut probe).unwrap_err().kind(),io::ErrorKind::WouldBlock);
                    peer.write_all(&incoming[2..]).await.unwrap();
                    let completion=tokio::select!{value=&mut receive=>value.unwrap(),result=&mut running=>panic!("native owner ended before handoff: {result:?}")};
                    assert!(matches!(completion,InputCompletion::Written));
                    let mut actual=vec![0;bytes.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,bytes);
                    assert_eq!(peer.try_read(&mut probe).unwrap_err().kind(),io::ErrorKind::WouldBlock);
                    lease.cancel();running.await.unwrap();lease.socket.lock().unwrap().take();
                    checkpoint.checkpoint().await.unwrap();
                    input_transition(owner.progress().unwrap(),completion)
                }
            }).await.unwrap();
            let proof=auth.proof.clone();let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
            assert_eq!(receipt.state,ReceiptState::Dispatched);
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }

    #[tokio::test]
    async fn expired_queued_opening_and_commit_are_rejected_without_a_waiter_poll() {
        for opening in [true,false] {
            let state=Arc::new(State::default());let conn=connection(&state);let fixture=queue_fixture(state.clone(),Uuid::random()).await;
            let deadline=Instant::now()+Duration::from_millis(100);state.reserve_grid(&conn,fixture.lease.clone(),deadline).unwrap();let mut bytes=0;
            let receiver=if opening {state.grid_open_response(&conn,Uuid::random(),fixture.opening(),fixture.lease.clone(),deadline,&mut bytes).unwrap()}else{state.grid_event(&conn,&fixture.lease,EventBody::GridCommit(GridCommit{revision:Uint(0)}),deadline,&mut bytes).unwrap()};
            tokio::time::sleep_until(deadline).await;assert!(matches!(state.pop_tagged_output(&conn),Err(ErrorCode::Timeout)));assert!(receiver.await.is_err());assert!(!fixture.lease.active());
            let (output,closed)=state.pop_tagged_output(&conn).unwrap();assert!(output.is_none()&&closed);state.disconnect(&conn);
        }
    }
    #[tokio::test]
    async fn quiet_agent_transition_closes_exact_reader_queue_and_phone_but_not_sibling() {
        let state=Arc::new(State::default());let a=connection(&state);let b=connection(&state);
        let first=queue_fixture(state.clone(),Uuid::random()).await;let sibling=queue_fixture_on(state.clone(),Uuid::random(),Some(first.lease.topology.clone())).await;
        let deadline=Instant::now()+Duration::from_secs(2);let mut bytes=0;
        state.reserve_grid(&a,first.lease.clone(),deadline).unwrap();
        let completion=state.grid_open_response(&a,Uuid::random(),first.opening(),first.lease.clone(),deadline,&mut bytes).unwrap();
        state.reserve_grid(&b,sibling.lease.clone(),deadline).unwrap();
        let (reader,mut peer)=UnixStream::pair().unwrap();let lease=first.lease.clone();
        let quiet=tokio::spawn(async move {let mut decoder=endpoint::Reader::new();let mut reader=BufReader::new(reader);tokio::select!{biased;_=lease.cancelled()=>Err(ErrorCode::StaleSession),result=decoder.read(&mut reader)=>result.map(|_|())}});
        let mut changed=first.source.clone();let mut snapshot=(*changed.snapshot).clone();snapshot.panes[0].agent=Some("omp".into());changed.snapshot=Arc::new(snapshot);state.reconcile_grid_source(&changed);
        assert!(matches!(tokio::time::timeout(Duration::from_secs(1),quiet).await.unwrap().unwrap(),Err(ErrorCode::StaleSession)));
        assert_eq!(peer.read(&mut [0]).await.unwrap(),0);assert!(completion.await.is_err());assert!(!first.lease.active());assert!(sibling.lease.active());
        let (output,closed)=state.pop_tagged_output(&a).unwrap();assert!(output.is_none()&&closed);state.unsubscribe(&b,&sibling.lease.id).unwrap();assert!(!state.pop_tagged_output(&b).unwrap().1);state.disconnect(&a);state.disconnect(&b);
    }
    #[tokio::test]
    async fn update_lock_past_cutoff_never_admits_a_late_handoff() {
        let state=Arc::new(State::default());let fixture=queue_fixture(state,Uuid::random()).await;
        let held=fixture.lease.topology.updates.lock().await;let deadline=Instant::now()+Duration::from_millis(100);
        let release=async{tokio::time::sleep_until(deadline+Duration::from_millis(20)).await;drop(held);};
        let (result,())=tokio::join!(fixture.lease.current(deadline),release);assert_eq!(result,Err(ErrorCode::Timeout));
    }
    #[tokio::test]
    async fn same_original_freshness_wait_is_peer_scoped_and_never_revives_a_retired_lease() {
        let state=Arc::new(State::default());let owner=connection(&state);let other=connection(&state);
        let fixture=queue_fixture(state.clone(),Uuid::random()).await;
        let sibling=queue_fixture_on(state.clone(),Uuid::random(),Some(fixture.lease.topology.clone())).await;
        let inventory=|changed| {
            let request=Uuid::random();
            state.host_snapshot(&owner,request.clone()).expect("host snapshot must remain valid while the original source is dirty");
            let mut invalidated=false;
            loop {
                let (bytes,closed)=state.pop_output(&owner).unwrap();
                assert!(!closed);
                match decode_payload(&bytes.expect("host snapshot response missing"),Some(Method::HostSnapshot)).unwrap() {
                    Envelope::Event(event)=>{
                        assert!(!invalidated);
                        assert!(matches!(event.body,EventBody::HostChanged(_)));
                        invalidated=true;
                    }
                    Envelope::Response(response)=>{
                        assert_eq!(invalidated,changed,"host invalidation must precede its replacement snapshot");
                        assert_eq!(response.id,request);
                        let Ok(ResultBody::HostSnapshot(snapshot))=response.outcome else {panic!("host snapshot response missing")};
                        return snapshot;
                    }
                    _=>panic!("unexpected host snapshot frame"),
                }
            }
        };
        let deadline=Instant::now()+Duration::from_secs(5);
        state.reserve_grid(&owner,fixture.lease.clone(),deadline).unwrap();
        state.reserve_grid(&other,sibling.lease.clone(),deadline).unwrap();
        assert!(matches!(state.grid_lease(&other,&fixture.lease.id),Err(ErrorCode::StaleSession)));
        let before=inventory(false);
        let original_shell=before.shells.iter().find(|shell|shell.target==fixture.lease.context.target).unwrap();
        let sibling_shell=before.shells.iter().find(|shell|shell.target==sibling.lease.context.target).unwrap();
        let original_graph=before.hierarchy.iter().find(|graph|graph.server_id==fixture.source.server_id).unwrap();
        let sibling_graph=before.hierarchy.iter().find(|graph|graph.server_id==sibling.source.server_id).unwrap();
        assert_eq!(original_shell.connectivity,SessionConnectivity::Ready);
        assert_eq!(sibling_shell.connectivity,SessionConnectivity::Ready);
        assert_eq!(original_graph.freshness,NativeFreshness::Current);
        assert_eq!(sibling_graph.freshness,NativeFreshness::Current);
        assert!(original_graph.panes.iter().any(|pane|pane.shell_target.as_ref()==Some(&original_shell.target)));
        fixture.source.invalidation.store(1,Ordering::Release);
        state.topology_synchronizing(&fixture.source.server_id).unwrap();
        let stale=inventory(true);
        let stale_graph=stale.hierarchy.iter().find(|graph|graph.server_id==fixture.source.server_id).unwrap();
        assert_eq!(stale_graph.freshness,NativeFreshness::Stale);
        assert_eq!(stale_graph.reason,Some(NativeUnavailableReason::SourceUnavailable));
        assert!(stale_graph.panes.iter().all(|pane|pane.target.is_none()&&pane.unbound_row_id.is_none()&&pane.shell_target.is_none()));
        let stale_shell=stale.shells.iter().find(|shell|shell.target==original_shell.target).unwrap();
        assert_eq!(stale_shell.connectivity,SessionConnectivity::Offline);
        assert_eq!(stale_shell.capabilities,ShellCapabilities{read:false,create:false,input:false,close:false,artifacts:false,grid:false});
        assert_eq!(
            (&stale_shell.server_id,&stale_shell.workspace_id,&stale_shell.tab_id,&stale_shell.pane_id,&stale_shell.terminal_id,&stale_shell.title),
            (&original_shell.server_id,&original_shell.workspace_id,&original_shell.tab_id,&original_shell.pane_id,&original_shell.terminal_id,&original_shell.title),
        );
        assert_eq!(stale.shells.iter().find(|shell|shell.target==sibling_shell.target),Some(sibling_shell));
        assert_eq!(stale.hierarchy.iter().find(|graph|graph.server_id==sibling_graph.server_id),Some(sibling_graph));
        assert!(sibling.lease.active());sibling.lease.check_current().unwrap();
        assert!(fixture.lease.active(),"an ordinary dirty layout must pause, not retire, the original owner");
        assert!(Arc::ptr_eq(&state.grid_lease(&owner,&fixture.lease.id).unwrap(),&fixture.lease));
        assert_eq!(fixture.lease.check_current(),Err(ErrorCode::NotReady));
        let pending=fixture.lease.current(deadline);tokio::pin!(pending);
        assert!(futures_util::poll!(&mut pending).is_pending());assert!(fixture.lease.active());
        let mut current=fixture.source.clone();current.revision=1;
        let mut layout=(*current.snapshot).clone();layout.layouts[0].area.width=2;layout.layouts[0].panes[0].rect.width=2;
        current.snapshot=Arc::new(layout);
        fixture.lease.topology.inner.lock().unwrap().servers.get_mut(&current.server_id).unwrap().source=Some(current.clone());
        fixture.lease.topology.update_shells(&current).await.unwrap();fixture.lease.topology.publish().unwrap();
        state.reconcile_grid_source(&current);pending.await.unwrap();
        assert!(fixture.lease.active());
        let restored=fixture.lease.topology.logical_shell_context(&fixture.lease.context.target).unwrap();
        assert_eq!(restored.target,fixture.lease.context.target);
        assert_eq!(restored.proof.source_epoch,fixture.lease.context.proof.source_epoch);
        let fresh=inventory(true);
        assert_eq!(fresh.shells.iter().find(|shell|shell.target==original_shell.target),Some(original_shell));
        assert_eq!(fresh.hierarchy.iter().find(|graph|graph.server_id==original_graph.server_id),Some(original_graph));
        assert_eq!(fresh.shells.iter().find(|shell|shell.target==sibling_shell.target),Some(sibling_shell));
        assert_eq!(fresh.hierarchy.iter().find(|graph|graph.server_id==sibling_graph.server_id),Some(sibling_graph));
        state.disconnect(&owner);assert!(!fixture.lease.active());
        assert!(sibling.lease.active());
        state.reconcile_grid_source(&current);assert_eq!(fixture.lease.current(deadline).await,Err(ErrorCode::StaleSession));
        state.disconnect(&other);
        assert!(!sibling.lease.active());
    }
    #[tokio::test]
    async fn current_inventory_input_permission_loss_retires_without_reviving_the_original_lease() {
        let state=Arc::new(State::default());let owner=connection(&state);
        let fixture=queue_fixture(state.clone(),Uuid::random()).await;
        let deadline=Instant::now()+Duration::from_secs(5);state.reserve_grid(&owner,fixture.lease.clone(),deadline).unwrap();
        let request=Uuid::random();state.host_snapshot(&owner,request.clone()).unwrap();
        let inventory=loop {
            let (Some(bytes),_)=state.pop_output(&owner).unwrap() else{panic!("current fixture inventory missing")};
            if let Envelope::Response(response)=decode_payload(&bytes,Some(Method::HostSnapshot)).unwrap()
                &&response.id==request&&let Ok(ResultBody::HostSnapshot(inventory))=response.outcome {break inventory;}
        };
        let mut shells=inventory.shells.into_iter().map(|summary|(summary,fixture.lease.context.proof.source_epoch.clone())).collect::<Vec<_>>();
        let selected=shells.iter_mut().find(|(summary,_)|summary.target==fixture.lease.context.target).unwrap();
        assert!(selected.0.capabilities.read&&selected.0.capabilities.grid);selected.0.capabilities.input=false;
        state.replace_inventory(inventory.servers,inventory.projects,shells,inventory.unbound,inventory.hierarchy,&[],|_|Ok(())).unwrap();
        assert!(!fixture.lease.active());state.reconcile_grid_source(&fixture.source);assert!(!fixture.lease.active());
        assert_eq!(fixture.lease.current(deadline).await,Err(ErrorCode::StaleSession));state.disconnect(&owner);
    }

    #[tokio::test]
    async fn missing_replaced_and_unavailable_sources_retire_instead_of_becoming_refresh_waits() {
        for loss in 0..5 {
            let state=Arc::new(State::default());let owner=connection(&state);
            let fixture=queue_fixture(state.clone(),Uuid::random()).await;
            let deadline=Instant::now()+Duration::from_secs(5);state.reserve_grid(&owner,fixture.lease.clone(),deadline).unwrap();
            if loss==4 {
                state.topology_offline(&fixture.source.server_id,"original native source unavailable").unwrap();
            }else{
                let mut changed=fixture.source.clone();let mut snapshot=(*changed.snapshot).clone();
                match loss {
                    0=>snapshot.panes.clear(),
                    1=>snapshot.panes[0].terminal_id=Id("replacement-terminal".into()),
                    2=>snapshot.panes[0].agent=Some("omp".into()),
                    _=>changed.incarnation=Uuid::random(),
                }
                changed.snapshot=Arc::new(snapshot);
                state.reconcile_grid_source(&changed);
            }
            assert!(!fixture.lease.active());
            state.reconcile_grid_source(&fixture.source);
            assert!(!fixture.lease.active(),"a later original-looking snapshot never revives a retired lease");
            assert_eq!(fixture.lease.current(deadline).await,Err(ErrorCode::StaleSession));
            state.disconnect(&owner);
        }
    }
}
