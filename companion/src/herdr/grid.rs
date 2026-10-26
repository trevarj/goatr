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
/// Original provider and route pin. Source revision is retained, never adopted from a frame.
pub(super) struct AgentObserveContext {
    pub target:Target,pub source_epoch:Uuid,pub key:StableSessionKey,pub provider:Provider,
    pub report:Option<(Option<String>,Option<NativeSession>)>,
    pub source:Source,pub route:shells::ShellIdentity,pub columns:u16,pub rows:u16,
}
enum GridContext {Shell(shells::LogicalShellContext),Agent(AgentObserveContext)}
impl GridContext {
    fn source(&self)->&Source {match self {Self::Shell(c)=>&c.source,Self::Agent(c)=>&c.source}}
    fn shell(&self)->SecurityResult<&shells::LogicalShellContext> {match self {Self::Shell(c)=>Ok(c),Self::Agent(_)=>Err(ErrorCode::NotReady)}}
}
pub(crate) struct GridLease {
    pub id:Uuid,
    stopped:watch::Sender<bool>,
    source_changed:watch::Sender<u64>,
    graceful:AtomicBool,
    popup:AtomicBool,
    coherent:AtomicBool,
    topology:Arc<Topology>,
    context:GridContext,
    socket:Mutex<Option<OwnedFd>>,
    commands:Option<mpsc::Sender<Command>>,
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
        (Arc::new(Self{id:Uuid::random(),stopped,source_changed,graceful:AtomicBool::new(false),popup:AtomicBool::new(false),coherent:AtomicBool::new(false),topology,context:GridContext::Shell(context),socket:Mutex::new(None),commands:Some(commands)}),receive)
    }
    fn agent(topology:Arc<Topology>,context:AgentObserveContext)->Arc<Self> {
        let (stopped,_)=watch::channel(false);let (source_changed,_)=watch::channel(context.source.revision);
        Arc::new(Self{id:Uuid::random(),stopped,source_changed,graceful:AtomicBool::new(false),popup:AtomicBool::new(false),coherent:AtomicBool::new(false),topology,context:GridContext::Agent(context),socket:Mutex::new(None),commands:None})
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
        match &self.context {GridContext::Shell(c)=>self.topology.grid_source(c),GridContext::Agent(c)=>self.topology.agent_grid_source(c)}.map(|_|())
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
    pub(crate) fn server(&self)->&Uuid{&self.context.source().server_id}
    pub(crate) fn shell(&self)->Option<(&ShellTarget,&Uuid)>{match &self.context {GridContext::Shell(c)=>Some((&c.target,&c.proof.source_epoch)),GridContext::Agent(_)=>None}}
    pub(crate) fn agent_target(&self)->Option<&Target>{match &self.context {GridContext::Agent(c)=>Some(&c.target),GridContext::Shell(_)=>None}}
    pub(crate) fn retains_agent_inventory(&self,sessions:&[SessionSummary],graphs:&[NativeHierarchy])->bool {
        let GridContext::Agent(c)=&self.context else{return false};
        let valid=sessions.iter().any(|s|s.session_id==c.target.session_id&&s.generation==c.target.generation&&s.provider==c.provider
            &&s.connectivity==SessionConnectivity::Ready&&s.server_id==c.source.server_id&&s.workspace_id==c.route.workspace_id&&s.pane_id==c.route.pane_id&&s.terminal_id==c.route.terminal_id)
            &&graphs.iter().any(|g|g.server_id==c.source.server_id&&g.server_incarnation==c.source.incarnation&&g.freshness==NativeFreshness::Current
                &&g.panes.iter().any(|p|p.pane_id==c.route.pane_id&&p.terminal_id==c.route.terminal_id&&p.workspace_id==c.route.workspace_id&&p.tab_id==c.route.tab_id&&p.target.as_ref()==Some(&c.target)));
        if valid{self.inventory_updated();}valid
    }
    pub(crate) fn accepts_source(&self,current:&Source)->bool {
        match &self.context {
            GridContext::Shell(c)=>{let route=&c.proof.identity;c.source.same_physical_identity(current)&&current.snapshot.pane(&route.pane_id).is_ok_and(|pane|
                pane.terminal_id==route.terminal_id&&pane.workspace_id==route.workspace_id&&pane.tab_id==route.tab_id&&shells::ordinary_shell(&current.snapshot,pane))},
            GridContext::Agent(c)=>c.source.same_physical_identity(current)&&current.snapshot.pane(&c.route.pane_id).is_ok_and(|p|
                p.terminal_id==c.route.terminal_id&&p.workspace_id==c.route.workspace_id&&p.tab_id==c.route.tab_id),
        }
    }
    pub(crate) fn retains_inventory(&self,summary:&ShellSummary,graph:&crate::protocol::NativeHierarchy)->bool {
        let Ok(context)=self.context.shell() else{return false};
        let route=&context.proof.identity;
        summary.target==context.target&&summary.server_id==context.source.server_id
            &&summary.workspace_id==route.workspace_id&&summary.tab_id==route.tab_id&&summary.pane_id==route.pane_id&&summary.terminal_id==route.terminal_id
            &&graph.server_id==context.source.server_id&&graph.server_incarnation==context.source.incarnation
            &&graph.panes.iter().any(|pane|pane.pane_id==route.pane_id&&pane.terminal_id==route.terminal_id
                &&pane.workspace_id==route.workspace_id&&pane.tab_id==route.tab_id&&pane.agent.is_none()
                &&pane.target.is_none()&&pane.unbound_row_id.is_none()
                &&if graph.freshness==NativeFreshness::Stale {pane.shell_target.as_ref().is_none_or(|target|target==&context.target)}
                    else{pane.shell_target.as_ref()==Some(&context.target)})
    }
    pub(crate) fn source_updated(&self,source:&Source){self.source_changed.send_replace(source.revision);}
    pub(crate) fn inventory_updated(&self){let revision=*self.source_changed.borrow();self.source_changed.send_replace(revision);}
    fn fence_socket(&self)->SecurityResult<OwnedFd> {
        if !self.active(){return Err(ErrorCode::StaleSession);}
        self.socket.lock().map_err(|_|ErrorCode::NotReady)?.as_ref().ok_or(ErrorCode::NotReady)?.as_fd().try_clone_to_owned().map_err(policy)
    }
    pub(super) fn screen(&self)->SecurityResult<()> {
        let current=self.topology.grid_source(self.context.shell()?)?;
        if current.snapshot.agents.iter().any(|agent|agent.launch_pending)
            ||current.snapshot.panes.iter().any(|pane|pane.restore_error.is_some())
        {return Err(ErrorCode::NotReady);}Ok(())
    }
    pub(super) fn input_ready(&self)->SecurityResult<()> {
        if !self.coherent.load(Ordering::Acquire)||self.popup.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        self.screen()?;
        let context=self.context.shell()?;
        self.topology.state.validate_shell(&context.target,&context.proof.source_epoch,shells::ShellAccess::Input)
    }
}
impl Topology {
    fn agent_observe_context(&self,target:&Target)->SecurityResult<AgentObserveContext> {
        let (source,original,summary)=self.logical_rename_context(target)?;
        let (key,epoch,report)={
            let inner=self.inner.lock().expect("topology mutex");
            let binding=inner.bindings.get(&target.session_id).filter(|b|b.target==*target&&b.source_epoch==original.source_epoch).ok_or(ErrorCode::StaleSession)?;
            (binding.key.clone(),binding.source_epoch.clone(),binding.herdr_report.clone())
        };
        let route=shells::ShellIdentity{pane_id:original.pane,terminal_id:original.terminal,workspace_id:original.workspace,tab_id:original.tab,read_identity:None};
        let pane=source.snapshot.pane(&route.pane_id).map_err(policy)?;
        let (columns,rows)=source.snapshot.layout_seed(pane).map_err(policy)?;
        let (columns,rows)=ansi::viewport(columns,rows);
        let context=AgentObserveContext{target:target.clone(),source_epoch:epoch,key,provider:summary.provider,report,source,route,columns,rows};
        self.agent_grid_source(&context)?;Ok(context)
    }
    fn agent_grid_source(&self,context:&AgentObserveContext)->SecurityResult<Source> {
        context.source.recheck_identity().map_err(|_|ErrorCode::StaleSession)?;
        let current={
            let inner=self.inner.lock().expect("topology mutex");
            let binding=inner.bindings.get(&context.target.session_id).filter(|b|b.target==context.target&&b.key==context.key&&b.source_epoch==context.source_epoch&&b.herdr_report==context.report).ok_or(ErrorCode::StaleSession)?;
            let source=inner.servers.get(&binding.key.server_id).and_then(|r|r.source.as_ref()).ok_or(ErrorCode::StaleSession)?;
            if !context.source.same_physical_identity(source){return Err(ErrorCode::StaleSession);}
            source.clone()
        };
        if !current.recheck_readiness().map_err(|_|ErrorCode::StaleSession)?{return Err(ErrorCode::NotReady);}
        self.state.validate_agent_observer(&context.target,&context.source_epoch,context.provider,&current.server_id,&context.route)?;
        let route=&context.route;let pane=current.snapshot.pane(&route.pane_id).map_err(|_|ErrorCode::StaleSession)?;
        if pane.terminal_id!=route.terminal_id||pane.workspace_id!=route.workspace_id||pane.tab_id!=route.tab_id
        {return Err(ErrorCode::StaleSession);}
        Ok(current)
    }
    async fn agent_grid_open(self:&Arc<Self>,security:Arc<Security>,auth:Arc<Authorized>,request:Uuid,params:TerminalGridSubscribeParams,deadline:Instant)->SecurityResult<()> {
        let context={let _update=timeout_at(deadline,self.updates.lock()).await.map_err(|_|ErrorCode::Timeout)?;self.agent_observe_context(&params.target)?};
        let memory=decode_slots().try_acquire_owned().map_err(|_|ErrorCode::ResourceLimit)?;
        let lease=GridLease::agent(self.clone(),context);
        {let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline)?;}
        let mut reservation=PendingGrid{state:security.state.clone(),connection:auth.connection_id.clone(),subscription:lease.id.clone(),published:false};
        let GridContext::Agent(context)=&lease.context else{unreachable!()};
        let mut session=tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,ansi::Session::connect(context,Arc::downgrade(&lease),auth.clone()))=>result.map_err(|_|ErrorCode::Timeout)??};
        *lease.socket.lock().map_err(|_|ErrorCode::NotReady)?=Some(session.client.fence_socket().map_err(policy)?);
        let opened=async {
            ready(&lease,&security,&auth,deadline).await?;session.negotiate(&context.route.terminal_id).await?;
            let projection=session.next().await?;
            // Stock resolves ObserveTerminal once, but may fall back to an agent name/label.
            // Hold this decoded frame and its permit until a fresh exact-ID admission proof.
            {let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;}
            let client=Client::connect_source(&context.source).await.map_err(policy)?;
            {let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;}
            let (_,snapshot)=client.snapshot().await.map_err(policy)?;
            {
                let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;
                session.client.recheck().map_err(policy)?;
                let pane=snapshot.pane(&context.route.pane_id).map_err(|_|ErrorCode::StaleSession)?;
                let reported=snapshot.agents.iter().find(|a|a.terminal_id==context.route.terminal_id).map(|a|(&a.agent,&a.agent_session));
                if pane.terminal_id!=context.route.terminal_id||pane.workspace_id!=context.route.workspace_id||pane.tab_id!=context.route.tab_id
                    ||reported!=context.report.as_ref().map(|(kind,native)|(kind,native))
                {return Err(ErrorCode::StaleSession);}
            }
            Ok::<_,ErrorCode>(projection)
        };
        let projection=tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,opened)=>result.map_err(|_|ErrorCode::Timeout)??};
        let mut bytes=0;
        let response=ResultBody::TerminalGridSubscribe(TerminalGridSubscribeResult{target:params.target,subscription:lease.id.clone(),seq:ZeroValue(0),frame:projection.begin.clone()});
        let receiver={let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;security.state.grid_open_response(&auth.connection_id,request,response,lease.clone(),deadline,&mut bytes)?};
        reservation.published=true;
        tokio::spawn(async move {
            let _memory=memory;
            let outcome=async {
                written(&lease,receiver,deadline).await?;publish_cells(&lease,&security,&auth,projection,deadline,&mut bytes).await?;
                loop {
                    let projection=tokio::select!{biased;_=lease.cancelled()=>return Ok::<(),ErrorCode>(()),result=session.next()=>result?};
                    let deadline=session.deadline();let mut bytes=0;
                    event(&lease,&security,&auth,EventBody::GridBegin(GridBeginEvent{frame:projection.begin.clone()}),deadline,&mut bytes).await?;
                    publish_cells(&lease,&security,&auth,projection,deadline,&mut bytes).await?;
                }
            }.await;
            lease.socket.lock().expect("grid socket mutex").take();
            if outcome.is_err(){security.state.grid_failed(&auth.connection_id,&lease.id);}
            lease.cancel();
        });
        Ok(())
    }
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
        if let RequestBody::TerminalGridSubscribe(params)=body {
            if !params.ansi_observer{return Err(ErrorCode::NotReady);}
            return self.agent_grid_open(security,auth,request,params,deadline).await;
        }
        let RequestBody::ShellGridSubscribe(params)=body else{return Err(ErrorCode::NotReady);};
        let context={let _update=timeout_at(deadline,self.updates.lock()).await.map_err(|_|ErrorCode::Timeout)?;self.logical_shell_context(&params.target)?};
        let current=self.grid_source(&context)?;
        let pane=current.snapshot.pane(&context.proof.identity.pane_id).map_err(policy)?;
        let (columns,rows)=current.snapshot.layout_seed(pane).map_err(policy)?;
        let memory=decode_slots().try_acquire_owned().map_err(|_|ErrorCode::ResourceLimit)?;
        let (lease,commands)=GridLease::new(self.clone(),context);
        {let _guard=lease.authorize(&security,&auth,deadline).await?;lease.check_current()?;security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline)?;}
        let mut reservation=PendingGrid{state:security.state.clone(),connection:auth.connection_id.clone(),subscription:lease.id.clone(),published:false};
        let mut session=tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,endpoint::Session::connect(lease.context.shell()?,Arc::downgrade(&lease),auth.clone()))=>result.map_err(|_|ErrorCode::Timeout)??};
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
        if lease.context.shell()?.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        if !lease.coherent.load(Ordering::Acquire)||lease.popup.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        lease.screen()?;Ok(lease)
    }
    pub(crate) async fn shell_resize_once(&self,security:&Security,auth:&Authorized,request:&Uuid,params:&ShellResizeParams,deadline:Instant)->SecurityResult<()> {
        params.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let lease=self.validate_shell_resize(&security.state,&auth.connection_id,params)?;
        let commands=lease.commands.as_ref().ok_or(ErrorCode::NotReady)?;
        let fd=lease.fence_socket()?;
        let columns=params.columns.0 as u16;let rows=params.rows.0 as u16;
        security.boundary(auth,request.clone(),||self.validate_shell_resize(&security.state,&auth.connection_id,params).map(|_|()),fd,async {
            let (reply,receive)=oneshot::channel();
            let sent=commands.try_send(Command::Resize(ResizeRequest{columns,rows,deadline,reply}));
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
        if lease.context.shell()?.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        lease.input_ready()?;Ok(lease)
    }
    pub(crate) async fn await_shell_input(&self,state:&State,connection:&Uuid,params:&ShellInputParams,deadline:Instant)->SecurityResult<()> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.shell()?.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        // Subscribe-before-recheck is owned by this exact lease; no authorization/State lock
        // is held while an ordinary same-original topology observation completes.
        lease.current(deadline).await?;lease.input_ready()
    }
    pub(crate) async fn shell_input_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&ShellInputParams,deadline:Instant)->SecurityResult<()> {
        params.validate().map_err(|_|ErrorCode::InvalidRequest)?;
        let lease=self.validate_shell_input(&security.state,&auth.connection_id,params)?;
        // The existing single native owner reserves its command slot BEFORE frame allocation.
        let permit=lease.commands.as_ref().ok_or(ErrorCode::NotReady)?.clone().try_reserve_owned().map_err(|error|match error {
            mpsc::error::TrySendError::Full(_)=>ErrorCode::Busy,mpsc::error::TrySendError::Closed(_)=>ErrorCode::NotReady,
        })?;
        let context=lease.context.shell()?;
        let source_epoch=context.proof.source_epoch.clone();
        let bytes=endpoint::pane_input(&context.proof.identity.pane_id,&params.input)?;
        let fd=lease.fence_socket()?;
        security.shell_input_boundary(auth,action.clone(),params.target.clone(),source_epoch.clone(),fd,
            ||self.validate_shell_input(&security.state,&auth.connection_id,params).map(|_|()),|checkpoint|async move {
                let progress=ShellDispatchProgress{frame_bytes:Some(bytes.len() as u32),socket_bytes_written:Some(0),frame_complete:Some(false)};
                let owner=match checkpoint.owner(&params.target,&source_epoch){
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
        ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{target:self.lease.context.shell().unwrap().target.clone(),subscription:self.lease.id.clone(),seq:ZeroValue(0),frame:GridBegin{mode:GridMode::Replace,revision:Uint(0),columns:Uint(1),rows:Uint(1),cursor:None,alternate_screen:false,graphics_omitted:false,popup_blocked:false,part_count:Uint(1)}})
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
        let peer=Uuid::random();let action=Uuid::random();let target=fixture.lease.context.shell().unwrap().target.clone();
        let identity=&fixture.lease.context.shell().unwrap().proof.identity;let epoch=fixture.lease.context.shell().unwrap().proof.source_epoch.clone();
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
        let owner=fixture.lease.topology.clone();let original=fixture.lease.context.shell().unwrap().target.clone();let epoch=Uuid::random();
        let mut current=fixture.source.clone();let mut snapshot=(*current.snapshot).clone();
        let mut created=snapshot.panes[0].clone();created.pane_id=Id("p2".into());created.terminal_id=Id("n2".into());created.focused=false;
        snapshot.panes.push(created);
        snapshot.layouts[0].area.width=2;
        snapshot.layouts[0].panes.push(NativeLayoutPane{pane_id:Id("p2".into()),rect:endpoint::Rect{x:1,y:0,width:1,height:1}});
        current.snapshot=Arc::new(snapshot);current.revision=1;current.invalidation.store(1,Ordering::Release);
        owner.inner.lock().unwrap().servers.get_mut(&current.server_id).unwrap().source=Some(current.clone());
        let mut proof=fixture.lease.context.shell().unwrap().proof.clone();proof.identity.pane_id=Id("p2".into());proof.identity.terminal_id=Id("n2".into());proof.source_epoch=epoch.clone();
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
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
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
            let input=ShellInput::Text{text:"x".into()};let bytes=endpoint::pane_input(&lease.context.shell().unwrap().proof.identity.pane_id,&input).unwrap();
            let action=Uuid::random();let params=ShellInputParams{boot_id:security.boot.clone(),target:lease.context.shell().unwrap().target.clone(),subscription:lease.id.clone(),input};
            security.reserve(&auth,Request{id:action.clone(),body:RequestBody::ShellInput(params.clone())},|_|Ok(()),|_|{}).await.unwrap();
            let commands=fixture.commands.take().unwrap();let fd=session.client.fence_socket().unwrap();
            let source=lease.context.shell().unwrap().proof.source_epoch.clone();let target=params.target.clone();
            security.shell_input_boundary(&auth,action.clone(),target.clone(),source,fd,||Ok(()),|checkpoint|{
                let secured=security.clone();let authorization=auth.clone();let lease=lease.clone();
                async move {
                    let owner=Arc::new(checkpoint.owner(&target,&lease.context.shell().unwrap().proof.source_epoch).unwrap());
                    owner.retain(&ShellDispatchProgress{frame_bytes:Some(bytes.len() as u32),socket_bytes_written:Some(0),frame_complete:Some(false)}).unwrap();
                    checkpoint.checkpoint().await.unwrap();
                    let (send,mut receive)=oneshot::channel();
                    lease.commands.as_ref().unwrap().try_send(Command::Input(InputRequest{bytes:bytes.clone(),progress:owner.clone(),deadline,reply:send})).unwrap_or_else(|_|panic!("original bounded input slot"));
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
        let original_shell=before.shells.iter().find(|shell|shell.target==fixture.lease.context.shell().unwrap().target).unwrap();
        let sibling_shell=before.shells.iter().find(|shell|shell.target==sibling.lease.context.shell().unwrap().target).unwrap();
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
        let restored=fixture.lease.topology.logical_shell_context(&fixture.lease.context.shell().unwrap().target).unwrap();
        assert_eq!(restored.target,fixture.lease.context.shell().unwrap().target);
        assert_eq!(restored.proof.source_epoch,fixture.lease.context.shell().unwrap().proof.source_epoch);
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
        let mut shells=inventory.shells.into_iter().map(|summary|(summary,fixture.lease.context.shell().unwrap().proof.source_epoch.clone())).collect::<Vec<_>>();
        let selected=shells.iter_mut().find(|(summary,_)|summary.target==fixture.lease.context.shell().unwrap().target).unwrap();
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
    fn observer_snapshot()->Value {
        json!({"version":"0.9.1","protocol":22,"focused_workspace_id":"w",
            "workspaces":[{"workspace_id":"w","number":1,"label":"fixture","focused":true,"active_tab_id":"t","agent_status":"idle"}],
            "tabs":[{"tab_id":"t","workspace_id":"w","number":1,"label":"fixture","focused":true,"agent_status":"idle"}],
            "panes":[{"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t","focused":true,"agent_status":"idle","agent":"omp"}],
            "agents":[{"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t","agent":"omp","name":"original",
                "agent_session":{"source":"test","agent":"omp","kind":"path","value":"/fixture/original.jsonl"}}],
            "layouts":[{"workspace_id":"w","tab_id":"t","area":{"x":0,"y":0,"width":1,"height":1},"focused_pane_id":"p",
                "panes":[{"pane_id":"p","rect":{"x":0,"y":0,"width":1,"height":1}}]}]})
    }
    async fn agent_fixture(state:Arc<State>)->(GridQueueFixture,Arc<GridLease>,Target) {
        let mut fixture=queue_fixture(state.clone(),Uuid::random()).await;
        let topology=fixture.topology();
        fixture.source.snapshot=Arc::new(serde_json::from_value(observer_snapshot()).unwrap());
        let source=fixture.source.clone();
        topology.inner.lock().unwrap().servers.get_mut(&source.server_id).unwrap().source=Some(source.clone());
        let value:SessionState=serde_json::from_value(json!({
            "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":source.server_id,
                "workspaceId":"w","terminalId":"n","paneId":"p","provider":"omp","nativeId":"native","title":"Agent",
                "status":"idle","connectivity":"ready","capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                "promptMode":"submit_or_steer","cancelMode":"current_work","terminalRead":true},"historyCoverage":{"scope":"all_branches",
                "source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
            "desktopSelection":"unknown","activity":{"state":"idle"}
        })).unwrap();
        let target=Target{session_id:value.summary.session_id.clone(),generation:value.summary.generation.clone()};
        let epoch=state.begin_install(value.clone()).unwrap();
        state.finish_install(&epoch,ProviderSnapshot{state:value,pending:vec![],history:vec![],older_available:false,native_older:None}).unwrap();
        assert!(!state.session_state(&target,&epoch).unwrap().summary.capabilities.terminal_read);
        let key=StableSessionKey{host_id:topology.host_id.clone(),server_id:source.server_id.clone(),server_incarnation:source.incarnation.clone(),
            terminal_id:Id("n".into()),provider_owner_identity:Id("provider-owner".into()),native_conversation_id:Id("native".into())};
        let agent=&source.snapshot.agents[0];let report=Some((agent.agent.clone(),agent.agent_session.clone()));
        topology.inner.lock().unwrap().bindings.insert(target.session_id.clone(),Binding{key,target:target.clone(),source_epoch:epoch,herdr_report:report,terminal_read_pin:None});
        let context=topology.agent_observe_context(&target).unwrap();
        let lease=GridLease::agent(topology,context);
        (fixture,lease,target)
    }
    #[tokio::test]
    async fn agent_has_no_shell_command_lane_and_old_phone_cannot_open_frames() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let (fixture,lease,target)=agent_fixture(security.state.clone()).await;
            let topology=fixture.topology();let deadline=Instant::now()+Duration::from_secs(5);
            assert!(lease.commands.is_none());assert!(lease.shell().is_none());assert!(lease.check_current().is_ok());
            assert_eq!(topology.grid_open(security.clone(),auth.clone(),Uuid::random(),RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams{target,ansi_observer:false}),deadline).await,Err(ErrorCode::NotReady));
            assert!(security.state.grid_available(&auth.connection_id).is_ok());
            security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let shell=fixture.lease.context.shell().unwrap().target.clone();
            assert_eq!(topology.validate_shell_resize(&security.state,&auth.connection_id,&ShellResizeParams{boot_id:security.boot.clone(),target:shell.clone(),subscription:lease.id.clone(),columns:Uint(1),rows:Uint(1)}).err(),Some(ErrorCode::NotReady));
            assert_eq!(topology.validate_shell_input(&security.state,&auth.connection_id,&ShellInputParams{boot_id:security.boot.clone(),target:shell,subscription:lease.id.clone(),input:ShellInput::Text{text:"never dispatched".into()}}).err(),Some(ErrorCode::NotReady));
            assert!(lease.active());security.state.unsubscribe(&auth.connection_id,&lease.id).unwrap();assert!(!lease.active());
        }
        security.state.disconnect(&auth.connection_id);drop(auth);drop(security);drop(host);drop(phone);fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn agent_original_binding_route_source_and_generation_retire_queued_cells() {
        for change in 0..9 {
            let state=Arc::new(State::default());let connection=connection(&state);
            let (fixture,lease,target)=agent_fixture(state.clone()).await;
            let deadline=Instant::now()+Duration::from_secs(5);
            state.reserve_grid(&connection,lease.clone(),deadline).unwrap();
            let opening=ResultBody::TerminalGridSubscribe(TerminalGridSubscribeResult{target:target.clone(),subscription:lease.id.clone(),seq:ZeroValue(0),
                frame:GridBegin{mode:GridMode::Replace,revision:Uint(0),columns:Uint(1),rows:Uint(1),cursor:None,alternate_screen:false,graphics_omitted:true,popup_blocked:false,part_count:Uint(1)}});
            let pending=state.grid_open_response(&connection,Uuid::random(),opening,lease.clone(),deadline,&mut 0).unwrap();
            match change {
                0..=3=>{
                    let mut current=fixture.source.clone();let mut snapshot=(*current.snapshot).clone();
                    match change {0=>snapshot.panes[0].terminal_id=Id("new-terminal".into()),1=>snapshot.panes[0].workspace_id=Id("new-workspace".into()),2=>snapshot.panes[0].tab_id=Id("new-tab".into()),_=>current.incarnation=Uuid::random()}
                    current.snapshot=Arc::new(snapshot);state.reconcile_grid_source(&current);
                },
                4=>{fixture.topology().inner.lock().unwrap().bindings.get_mut(&target.session_id).unwrap().key.native_conversation_id=Id("replacement-conversation".into());
                    assert_eq!(lease.check_current(),Err(ErrorCode::StaleSession));state.grid_failed(&connection,&lease.id);},
                5=>{fixture.topology().inner.lock().unwrap().bindings.get_mut(&target.session_id).unwrap().source_epoch=Uuid::random();
                    assert_eq!(lease.check_current(),Err(ErrorCode::StaleSession));state.grid_failed(&connection,&lease.id);},
                6=>state.retire_session(&target.session_id),
                7=>{state.begin_source_install(&target).unwrap();},
                _=>{state.update(&target,crate::state::SessionUpdate::SourceGap).unwrap();},
            }
            assert!(!lease.active());assert!(pending.await.is_err());assert!(state.pop_output(&connection).unwrap().0.is_none());
            assert_eq!(lease.check_current(),Err(ErrorCode::StaleSession));state.disconnect(&connection);
        }
    }
    #[tokio::test]
    async fn observer_native_socket_writes_only_frozen_hello_and_exact_terminal_observe() {
        use std::os::unix::fs::PermissionsExt;
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let (fixture,lease,_)=agent_fixture(security.state.clone()).await;
            let path=fixture.source.socket_path.with_file_name("native-client.sock");
            let listener=tokio::net::UnixListener::bind(&path).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
            let GridContext::Agent(context)=&lease.context else{panic!()};
            let (session,peer)=tokio::join!(ansi::Session::connect(context,Arc::downgrade(&lease),auth.clone()),listener.accept());
            let mut session=session.unwrap();let (mut peer,_)=peer.unwrap();
            let producer=async {
                let mut header=[0;4];peer.read_exact(&mut header).await.unwrap();assert_eq!(u32::from_le_bytes(header),7);
                let mut hello=[0;7];peer.read_exact(&mut hello).await.unwrap();assert_eq!(hello,[0,22,1,1,0,0,0]);
                for byte in [4,0,0,0,0,22,1,0] {peer.write_all(&[byte]).await.unwrap();tokio::task::yield_now().await;}
                let mut observe=[0;7];peer.read_exact(&mut observe).await.unwrap();assert_eq!(observe,[3,0,0,0,7,1,b'n']);
                peer.write_all(&endpoint::fixture::packet(&[7])).await.unwrap();
                let ansi=b"\x1b[?2026h\x1b[?25l\x1b]8;;\x1b\\\x1b[2J\x1b[1;1H\x1b[0;1;38;5;196;49mA\x1b[0m\x1b[1;1H\x1b[2 q\x1b[?25h\x1b[?2026l\x1b[1;1H\x1b[?25h";
                assert!(ansi.len()<=250);let mut native=vec![1,1,1,1,1,ansi.len() as u8];native.extend_from_slice(ansi);
                for chunk in endpoint::fixture::packet(&native).chunks(3) {peer.write_all(chunk).await.unwrap();tokio::task::yield_now().await;}
                peer.write_all(&endpoint::fixture::packet(&[3,0])).await.unwrap();
                let mut unexpected=[0];assert_eq!(peer.read(&mut unexpected).await.unwrap(),0,"dismissal must not write control, detach, resize, input or endpoint commands");
            };
            let consumer=async {
                session.negotiate(&Id("n".into())).await.unwrap();let projection=session.next().await.unwrap();
                assert_eq!(session.next().await.err(),Some(ErrorCode::StaleSession));drop(session);projection
            };
            let ((),projection)=tokio::time::timeout(Duration::from_secs(5),async {tokio::join!(producer,consumer)}).await.unwrap();
            assert_eq!(projection.cells.len(),1);assert_eq!(projection.cells[0].text,"A");assert!(projection.cells[0].attributes.bold);
            assert_eq!(projection.cells[0].foreground,GridColor::Indexed{index:Uint(196)});assert_eq!(projection.begin.cursor.unwrap().shape,Uint(2));
            assert!(projection.begin.graphics_omitted);assert!(!projection.begin.alternate_screen);
        }
        security.state.disconnect(&auth.connection_id);drop(auth);drop(security);drop(host);drop(phone);fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn observer_refuses_bad_welcome_or_frames_before_hello_without_sending_observe() {
        use std::os::unix::fs::PermissionsExt;
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        for invalid in [&[0,21,1,0][..],&[0,22,0,0][..],&[0,22,1,1,1,b'x'][..],&[1,1,1,1,1,0][..],&[3,0][..]] {
            let (fixture,lease,_)=agent_fixture(security.state.clone()).await;
            let path=fixture.source.socket_path.with_file_name("native-client.sock");
            let listener=tokio::net::UnixListener::bind(&path).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
            let GridContext::Agent(context)=&lease.context else{panic!()};
            let (session,peer)=tokio::join!(ansi::Session::connect(context,Arc::downgrade(&lease),auth.clone()),listener.accept());
            let mut session=session.unwrap();let (mut peer,_)=peer.unwrap();
            let producer=async {
                let mut hello=[0;11];peer.read_exact(&mut hello).await.unwrap();assert_eq!(hello,[7,0,0,0,0,22,1,1,0,0,0]);
                peer.write_all(&endpoint::fixture::packet(invalid)).await.unwrap();
                let mut unexpected=[0];assert_eq!(peer.read(&mut unexpected).await.unwrap(),0);
            };
            let consumer=async {assert_eq!(session.negotiate(&Id("n".into())).await,Err(ErrorCode::NotReady));drop(session);};
            tokio::time::timeout(Duration::from_secs(5),async {tokio::join!(producer,consumer)}).await.unwrap();
        }
        security.state.disconnect(&auth.connection_id);drop(auth);drop(security);drop(host);drop(phone);fs::remove_dir_all(root).unwrap();
    }
    // These end-to-end openings share the production two-reader quota with the Shell test.
    static OBSERVER_OPENS:tokio::sync::Mutex<()>=tokio::sync::Mutex::const_new(());
    async fn observer_fixture(state:Arc<State>)->(GridQueueFixture,Target,tokio::net::UnixListener) {
        use std::os::unix::fs::PermissionsExt;
        let (fixture,_,target)=agent_fixture(state).await;
        let (mut probe,_)=fixture._listener.accept().await.unwrap();
        assert_eq!(probe.read(&mut [0]).await.unwrap(),0);
        let path=fixture.source.socket_path.with_file_name("native-client.sock");
        let listener=tokio::net::UnixListener::bind(&path).unwrap();
        fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
        (fixture,target,listener)
    }
    fn observer_frame(seq:u8,text:&str)->Vec<u8> {
        let clear=if seq==1 {"\x1b[2J"}else{""};
        let ansi=format!("\x1b[?2026h\x1b[?25l\x1b]8;;\x1b\\{clear}\x1b[1;1H\x1b[0;39;49m{text}\x1b[0m\x1b[1;1H\x1b[?25h\x1b[?2026l\x1b[1;1H\x1b[?25h");
        assert!(ansi.len()<=250);
        let mut bytes=vec![1,seq,1,1,u8::from(seq==1),ansi.len() as u8];bytes.extend_from_slice(ansi.as_bytes());
        endpoint::fixture::packet(&bytes)
    }
    async fn observer_first_frame(fixture:&GridQueueFixture,listener:&tokio::net::UnixListener,calls:&AtomicU64)->(UnixStream,BufReader<UnixStream>,Value) {
        let (mut native,_)=listener.accept().await.unwrap();
        let mut hello=[0;11];native.read_exact(&mut hello).await.unwrap();assert_eq!(hello,[7,0,0,0,0,22,1,1,0,0,0]);
        native.write_all(&endpoint::fixture::packet(&[0,22,1,0])).await.unwrap();
        let mut observe=[0;7];native.read_exact(&mut observe).await.unwrap();assert_eq!(observe,[3,0,0,0,7,1,b'n']);
        native.write_all(&observer_frame(1,"A")).await.unwrap();
        let (stream,_)=fixture._listener.accept().await.unwrap();let mut public=BufReader::new(stream);
        let request:Value=serde_json::from_slice(&read_line(&mut public).await.unwrap()).unwrap();
        calls.fetch_add(1,Ordering::SeqCst);
        assert_eq!(request["method"],"session.snapshot");assert_eq!(request["params"],json!({}));
        (native,public,request)
    }
    async fn observer_snapshot_reply(public:&mut BufReader<UnixStream>,request:&Value,snapshot:Value)->io::Result<()> {
        let mut bytes=serde_json::to_vec(&json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":snapshot}})).unwrap();
        bytes.push(b'\n');public.get_mut().write_all(&bytes).await
    }
    async fn observer_output(security:&Security,auth:&Authorized,deadline:Instant)->Envelope {
        timeout_at(deadline,async {
            loop {
                let (queued,closed)=security.state.pop_tagged_output(&auth.connection_id).unwrap();assert!(!closed);
                if let Some(queued)=queued {
                    let (lease,written,until)=queued.grid.expect("actual observer publication owner");
                    let _guard=lease.authorize(security,auth,until).await.unwrap();lease.check_current().unwrap();
                    let envelope=decode_payload(&queued.bytes,Some(Method::TerminalGridSubscribe)).unwrap();
                    written.send(()).unwrap();return envelope;
                }
                tokio::task::yield_now().await;
            }
        }).await.unwrap()
    }
    #[tokio::test]
    async fn observer_admits_held_first_frame_with_one_snapshot_and_no_later_rpcs() {
        let _serial=OBSERVER_OPENS.lock().await;
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let (fixture,target,listener)=observer_fixture(security.state.clone()).await;
            let topology=fixture.topology();let deadline=Instant::now()+Duration::from_secs(5);let calls=AtomicU64::new(0);
            let producer=async {
                let (mut native,mut public,request)=observer_first_frame(&fixture,&listener,&calls).await;
                assert!(security.state.pop_output(&auth.connection_id).unwrap().0.is_none(),"first frame must remain private while admission is pending");
                assert_eq!(security.state.grid_available(&auth.connection_id),Err(ErrorCode::Busy));
                // Later native data may queue during the round trip, never replace held frame A.
                native.write_all(&observer_frame(2,"B")).await.unwrap();native.write_all(&observer_frame(3,"C")).await.unwrap();
                let mut snapshot=observer_snapshot();
                snapshot["panes"][0]["title"]=json!("new terminal title");snapshot["panes"][0]["agent_status"]=json!("working");
                snapshot["agents"][0]["title"]=json!("new agent title");snapshot["agents"][0]["name"]=json!("new alias");
                snapshot["workspaces"][0]["label"]=json!("new label");snapshot["tabs"][0]["agent_status"]=json!("working");
                observer_snapshot_reply(&mut public,&request,snapshot).await.unwrap();
                let mut unexpected=[0];
                tokio::select! {
                    result=native.read(&mut unexpected)=>assert_eq!(result.unwrap(),0,"observer must never write input, resize, focus or control"),
                    result=fixture._listener.accept()=>{
                        let (stream,_)=result.unwrap();let mut extra=BufReader::new(stream);
                        read_line(&mut extra).await.unwrap();calls.fetch_add(1,Ordering::SeqCst);
                        panic!("later frames must not request another native snapshot");
                    }
                }
            };
            let consumer=async {
                topology.grid_open(security.clone(),auth.clone(),Uuid::random(),RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams{target:target.clone(),ansi_observer:true}),deadline).await.unwrap();
                let Envelope::Response(Response{outcome:Ok(ResultBody::TerminalGridSubscribe(opened)),..})=observer_output(&security,&auth,deadline).await else{panic!("observer opening missing")};
                assert_eq!(opened.target,target);assert_eq!(opened.frame.revision,Uint(0));
                for (revision,text) in ["A","B","C"].into_iter().enumerate() {
                    if revision>0 {assert!(matches!(observer_output(&security,&auth,deadline).await,Envelope::Event(Event{body:EventBody::GridBegin(GridBeginEvent{frame}),..})if frame.revision==Uint(revision as u64)));}
                    let Envelope::Event(Event{body:EventBody::GridPart(part),..})=observer_output(&security,&auth,deadline).await else{panic!("observer cells missing")};
                    assert_eq!(part.revision,Uint(revision as u64));assert_eq!(part.spans[0].cells[0].text,text);
                    assert!(matches!(observer_output(&security,&auth,deadline).await,Envelope::Event(Event{body:EventBody::GridCommit(commit),..})if commit.revision==Uint(revision as u64)));
                }
                assert!(Arc::ptr_eq(&topology.source(&fixture.source.server_id).unwrap().snapshot,&fixture.source.snapshot),"admission must not apply/adopt its snapshot");
                security.state.unsubscribe(&auth.connection_id,&opened.subscription).unwrap();
            };
            timeout_at(deadline,async {tokio::join!(producer,consumer)}).await.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
    #[tokio::test]
    async fn observer_alias_fallback_or_native_report_change_publishes_no_opening_or_cells() {
        let _serial=OBSERVER_OPENS.lock().await;
        for change in 0..4 {
            let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
            {
                let (fixture,target,listener)=observer_fixture(security.state.clone()).await;
                let topology=fixture.topology();let deadline=Instant::now()+Duration::from_secs(5);let calls=AtomicU64::new(0);
                let producer=async {
                    let (mut native,mut public,request)=observer_first_frame(&fixture,&listener,&calls).await;
                    assert!(security.state.pop_output(&auth.connection_id).unwrap().0.is_none());
                    let mut snapshot=observer_snapshot();
                    match change {
                        0=>{
                            // A vanished; stock's one-time alias fallback resolved B named/labeled A-ID.
                            snapshot["panes"][0]["terminal_id"]=json!("b");snapshot["panes"][0]["label"]=json!("n");
                            snapshot["agents"][0]["terminal_id"]=json!("b");snapshot["agents"][0]["name"]=json!("n");
                        },
                        1=>{snapshot["agents"][0]["agent"]=json!("codex");snapshot["panes"][0]["agent"]=json!("codex");},
                        2=>snapshot["agents"][0]["agent_session"]["value"]=json!("/fixture/replacement.jsonl"),
                        _=>{
                            snapshot["panes"][0]["pane_id"]=json!("replacement-pane");snapshot["agents"][0]["pane_id"]=json!("replacement-pane");
                            snapshot["layouts"]=json!([]);
                        },
                    }
                    observer_snapshot_reply(&mut public,&request,snapshot).await.unwrap();
                    assert_eq!(native.read(&mut [0]).await.unwrap(),0);
                };
                let consumer=async {
                    assert_eq!(topology.grid_open(security.clone(),auth.clone(),Uuid::random(),RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams{target,ansi_observer:true}),deadline).await,Err(ErrorCode::StaleSession));
                    let (output,closed)=security.state.pop_output(&auth.connection_id).unwrap();assert!(output.is_none()&&closed,"no opening or staged cells may escape failed admission");
                };
                timeout_at(deadline,async {tokio::join!(producer,consumer)}).await.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
            }
            pairing::tests::review_close(host,phone,security,auth,root).await;
        }
    }
    #[tokio::test]
    async fn observer_admission_roundtrip_rejects_binding_source_auth_and_deadline_races() {
        let _serial=OBSERVER_OPENS.lock().await;
        for change in 0..10 {
            let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
            {
                let (fixture,target,listener)=observer_fixture(security.state.clone()).await;
                let topology=fixture.topology();let deadline=Instant::now()+Duration::from_secs(5);let calls=AtomicU64::new(0);
                let (cutoff,cutoff_wait)=oneshot::channel();let (released,release_wait)=oneshot::channel();
                let producer=async {
                    let (mut native,mut public,request)=observer_first_frame(&fixture,&listener,&calls).await;
                    assert!(security.state.pop_output(&auth.connection_id).unwrap().0.is_none());
                    cutoff.send(()).unwrap();release_wait.await.unwrap();
                    // Cancellation/deadline may already have dropped the original public client.
                    let _=observer_snapshot_reply(&mut public,&request,observer_snapshot()).await;
                    assert_eq!(native.read(&mut [0]).await.unwrap(),0);
                };
                let racing=async {
                    cutoff_wait.await.unwrap();
                    match change {
                        0=>topology.inner.lock().unwrap().bindings.get_mut(&target.session_id).unwrap().key.native_conversation_id=Id("replacement".into()),
                        1=>topology.inner.lock().unwrap().bindings.get_mut(&target.session_id).unwrap().source_epoch=Uuid::random(),
                        2=>topology.inner.lock().unwrap().bindings.get_mut(&target.session_id).unwrap().target.generation=Uuid::random(),
                        3=>topology.inner.lock().unwrap().bindings.get_mut(&target.session_id).unwrap().herdr_report.as_mut().unwrap().0=Some("codex".into()),
                        4=>topology.inner.lock().unwrap().servers.get_mut(&fixture.source.server_id).unwrap().source.as_mut().unwrap().incarnation=Uuid::random(),
                        5=>{
                            use std::os::unix::fs::PermissionsExt;
                            fs::remove_file(&fixture.source.socket_path).unwrap();
                            let _replacement=tokio::net::UnixListener::bind(&fixture.source.socket_path).unwrap();
                            fs::set_permissions(&fixture.source.socket_path,fs::Permissions::from_mode(0o600)).unwrap();
                        },
                        6=>{security.state.begin_source_install(&target).unwrap();},
                        7=>{security.revoke(auth.proof.peer_id().clone(),None).await.unwrap();},
                        8=>tokio::time::sleep_until(deadline).await,
                        _=>{
                            let mut changed=fixture.source.clone();let mut snapshot=(*changed.snapshot).clone();
                            snapshot.panes[0].terminal_id=Id("replacement".into());changed.snapshot=Arc::new(snapshot);
                            security.state.reconcile_grid_source(&changed);
                        },
                    }
                    released.send(()).unwrap();
                };
                let consumer=async {
                    assert!(topology.grid_open(security.clone(),auth.clone(),Uuid::random(),RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams{target:target.clone(),ansi_observer:true}),deadline).await.is_err());
                    if change==7 {assert!(matches!(security.state.pop_output(&auth.connection_id),Err(ErrorCode::Unauthorized)));}
                    else {let (output,closed)=security.state.pop_output(&auth.connection_id).unwrap();assert!(output.is_none()&&closed,"late snapshot must not revive an invalidated opening");}
                };
                timeout_at(deadline+Duration::from_secs(1),async {tokio::join!(producer,racing,consumer)}).await.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
            }
            pairing::tests::review_close(host,phone,security,auth,root).await;
        }
    }
    #[tokio::test]
    async fn observer_proven_but_queued_opening_cannot_outlive_deadline_or_revocation() {
        let _serial=OBSERVER_OPENS.lock().await;
        for revoke in [false,true] {
            let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
            {
                let (fixture,target,listener)=observer_fixture(security.state.clone()).await;
                let topology=fixture.topology();let deadline=Instant::now()+Duration::from_secs(5);let calls=AtomicU64::new(0);
                let producer=async {
                    let (mut native,mut public,request)=observer_first_frame(&fixture,&listener,&calls).await;
                    observer_snapshot_reply(&mut public,&request,observer_snapshot()).await.unwrap();
                    assert_eq!(native.read(&mut [0]).await.unwrap(),0);
                };
                let consumer=async {
                    topology.grid_open(security.clone(),auth.clone(),Uuid::random(),RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams{target,ansi_observer:true}),deadline).await.unwrap();
                    // Do not acknowledge the queued response: its held cells must stay private.
                    if revoke {
                        security.revoke(auth.proof.peer_id().clone(),None).await.unwrap();
                        assert!(matches!(security.state.pop_output(&auth.connection_id),Err(ErrorCode::Unauthorized)));
                    }else{
                        tokio::time::sleep_until(deadline).await;
                        assert!(matches!(security.state.pop_tagged_output(&auth.connection_id),Err(ErrorCode::Timeout)|Ok((None,true))));
                        let (output,closed)=security.state.pop_output(&auth.connection_id).unwrap();assert!(output.is_none()&&closed);
                    }
                };
                timeout_at(deadline+Duration::from_secs(1),async {tokio::join!(producer,consumer)}).await.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
            }
            pairing::tests::review_close(host,phone,security,auth,root).await;
        }
    }
}
