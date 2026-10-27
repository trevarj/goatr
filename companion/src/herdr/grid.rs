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
    history:Mutex<HistoryState>,
    held:Mutex<HashMap<u64,ShellKeyEvent>>,
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
fn input_keys(input:&ShellInput)->&[ShellKeyEvent] {match input{ShellInput::Keys{events}=>events,_=>&[]}}
struct InputRequest {bytes:Vec<u8>,keys:Vec<ShellKeyEvent>,progress:Arc<pairing::ShellInputProgressOwner>,deadline:Instant,reply:oneshot::Sender<InputCompletion>}
pub(super) enum InputPlan {Original,Cleanup{input:ShellInput,bytes:Vec<u8>}}
impl InputPlan {
    pub(super) fn bytes<'a>(&'a self,original:&'a [u8])->&'a [u8] {match self{Self::Original=>original,Self::Cleanup{bytes,..}=>bytes}}
    pub(super) fn keys<'a>(&'a self,original:&'a [ShellKeyEvent])->&'a [ShellKeyEvent] {match self{Self::Original=>original,Self::Cleanup{input,..}=>input_keys(input)}}
}
enum Command {Resize(ResizeRequest),Input(InputRequest),Scroll(ScrollRequest)}
enum ScrollCompletion {Confirmed(GridScroll),Refused(ErrorCode),Uncertain}
struct ScrollRequest {client:Client,offset:u64,deadline:Instant,reply:oneshot::Sender<ScrollCompletion>}
#[derive(Clone,Copy,PartialEq,Eq)]
enum HistoryMode {Live,Browsing,Pending,Uncertain}
struct HistoryState {enabled:bool,mode:HistoryMode,prior:HistoryMode,available:Option<GridScroll>,committed:Option<GridScroll>,bottom_ack:bool}
impl Default for HistoryState {fn default()->Self {Self{enabled:false,mode:HistoryMode::Live,prior:HistoryMode::Live,available:None,committed:None,bottom_ack:false}}}
impl HistoryState {
    fn observe(&mut self,metrics:Option<GridScroll>) {
        if metrics.as_ref().is_some_and(|m|m.offset_from_bottom.0>0)&&self.mode==HistoryMode::Live{self.mode=HistoryMode::Browsing;self.bottom_ack=false;}
        self.available=metrics;
    }
    fn commit(&mut self){
        self.committed=self.available.clone();
        if self.mode==HistoryMode::Live&&self.committed.as_ref().is_some_and(|metrics|metrics.offset_from_bottom.0>0) {
            self.mode=HistoryMode::Browsing;self.bottom_ack=false;
        }
        self.resume();
    }
    fn resume(&mut self){
        if self.bottom_ack&&self.committed.as_ref().is_some_and(|m|m.offset_from_bottom.0==0) {
            self.mode=HistoryMode::Live;self.bottom_ack=false;
        }
    }
    fn pending(&mut self)->SecurityResult<()> {
        if self.mode==HistoryMode::Pending{return Err(ErrorCode::Busy);}
        self.prior=if self.mode==HistoryMode::Live&&self.committed.as_ref().is_some_and(|m|m.offset_from_bottom.0==0){HistoryMode::Live}
            else if self.mode==HistoryMode::Uncertain{HistoryMode::Uncertain}else{HistoryMode::Browsing};
        self.mode=HistoryMode::Pending;self.bottom_ack=false;Ok(())
    }
    fn finish(&mut self,result:&ScrollCompletion,bottom:bool) {
        match result {
            ScrollCompletion::Confirmed(metrics)=>{self.mode=HistoryMode::Browsing;self.bottom_ack=bottom&&metrics.offset_from_bottom.0==0;self.resume();},
            ScrollCompletion::Refused(_)=>{self.mode=if self.prior==HistoryMode::Live&&!self.committed.as_ref().is_some_and(|m|m.offset_from_bottom.0==0){HistoryMode::Browsing}else{self.prior};},
            ScrollCompletion::Uncertain=>{self.mode=HistoryMode::Uncertain;self.bottom_ack=false;},
        }
    }
}
struct ScrollIntent {lease:Arc<GridLease>,entered:bool,finished:bool}
impl Drop for ScrollIntent {fn drop(&mut self){if !self.finished {self.lease.history.lock().expect("history mutex").finish(&if self.entered{ScrollCompletion::Uncertain}else{ScrollCompletion::Refused(ErrorCode::NotReady)},false);}}}
impl GridLease {
    fn new(topology:Arc<Topology>,context:shells::LogicalShellContext)->(Arc<Self>,mpsc::Receiver<Command>) {
        let (stopped,_)=watch::channel(false);let (source_changed,_)=watch::channel(context.source.revision);
        let (commands,receive)=mpsc::channel(1);
        (Arc::new(Self{id:Uuid::random(),stopped,source_changed,graceful:AtomicBool::new(false),popup:AtomicBool::new(false),coherent:AtomicBool::new(false),topology,context:GridContext::Shell(context),socket:Mutex::new(None),commands:Some(commands),history:Mutex::new(HistoryState::default()),held:Mutex::new(HashMap::new())}),receive)
    }
    fn agent(topology:Arc<Topology>,context:AgentObserveContext)->Arc<Self> {
        let (stopped,_)=watch::channel(false);let (source_changed,_)=watch::channel(context.source.revision);
        Arc::new(Self{id:Uuid::random(),stopped,source_changed,graceful:AtomicBool::new(false),popup:AtomicBool::new(false),coherent:AtomicBool::new(false),topology,context:GridContext::Agent(context),socket:Mutex::new(None),commands:None,history:Mutex::new(HistoryState::default()),held:Mutex::new(HashMap::new())})
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
        match &self.context {
            GridContext::Shell(c)=>{self.topology.grid_source(c)?;if self.history.lock().map_err(|_|ErrorCode::NotReady)?.enabled{self.topology.state.validate_shell_history(&c.target,&c.proof.source_epoch)?;}Ok(())},
            GridContext::Agent(c)=>self.topology.agent_grid_source(c).map(|_|()),
        }
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
    fn input_admission<T>(&self,keys:&[ShellKeyEvent],admitted:impl FnOnce(Option<&HashMap<u64,ShellKeyEvent>>)->T)->SecurityResult<T> {
        self.input_authority()?;
        // Always history -> held, including the synchronous first-byte write callback.
        let history=self.history.lock().map_err(|_|ErrorCode::NotReady)?;
        if !history.enabled{return Ok(admitted(None));}
        let held=self.held.lock().map_err(|_|ErrorCode::NotReady)?;
        if history.mode!=HistoryMode::Live {
            if keys.is_empty()||keys.iter().enumerate().any(|(index,key)|key.phase!=ShellKeyPhase::Release
                ||key.physical_key_id.as_ref().is_none_or(|id|!held.contains_key(&id.0)||keys[..index].iter().any(|old|old.physical_key_id.as_ref()==Some(id))))
            {return Err(ErrorCode::NotReady);}
            return Ok(admitted(Some(&held)));
        }
        let added=keys.iter().enumerate().filter(|(index,key)|key.phase==ShellKeyPhase::Press&&key.physical_key_id.as_ref().is_some_and(|id|
            !held.contains_key(&id.0)&&!keys[..*index].iter().any(|old|old.phase==ShellKeyPhase::Press&&old.physical_key_id.as_ref()==Some(id)))).count();
        if held.len()+added>128{return Err(ErrorCode::ResourceLimit);}Ok(admitted(None))
    }
    pub(super) fn input_ready(&self,keys:&[ShellKeyEvent])->SecurityResult<()> {self.input_admission(keys,|_|())}
    pub(super) fn input_plan(&self,keys:&[ShellKeyEvent])->SecurityResult<InputPlan> {
        self.input_admission(keys,|held| {
            let Some(held)=held else{return Ok(InputPlan::Original)};
            let input=ShellInput::Keys{events:keys.iter().map(|key|{
                let mut release=held[&key.physical_key_id.as_ref().expect("validated physical ID").0].clone();
                release.phase=ShellKeyPhase::Release;release
            }).collect()};
            let bytes=endpoint::pane_input(&self.context.shell()?.proof.identity.pane_id,&input)?;
            Ok(InputPlan::Cleanup{input,bytes})
        })?
    }
    pub(super) fn poll_input<T>(&self,keys:&[ShellKeyEvent],plan:&InputPlan,write:impl FnOnce()->T)->SecurityResult<Option<T>> {
        self.input_admission(keys,|held| {
            let current=match (held,plan) {
                (None,InputPlan::Original)=>true,
                (Some(held),InputPlan::Cleanup{input:ShellInput::Keys{events},..})=>events.len()==keys.len()&&keys.iter().zip(events).all(|(requested,effective)| {
                    effective.phase==ShellKeyPhase::Release&&effective.physical_key_id==requested.physical_key_id
                        &&effective.physical_key_id.as_ref().and_then(|id|held.get(&id.0)).is_some_and(|press|press.key==effective.key&&press.modifiers==effective.modifiers)
                }),
                _=>false,
            };
            if current{Some(write())}else{None}
        })
    }
    fn input_written(&self,keys:&[ShellKeyEvent]) {
        if !self.history.lock().expect("history mutex").enabled{return;}
        let mut held=self.held.lock().expect("held key mutex");
        for key in keys {if let Some(id)=&key.physical_key_id {match key.phase{ShellKeyPhase::Press=>{held.insert(id.0,key.clone());},ShellKeyPhase::Release=>{held.remove(&id.0);},ShellKeyPhase::Repeat=>{}}}}
    }
    pub(super) fn input_authority(&self)->SecurityResult<()> {
        if !self.coherent.load(Ordering::Acquire)||self.popup.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        self.screen()?;
        let context=self.context.shell()?;
        if self.history.lock().map_err(|_|ErrorCode::NotReady)?.enabled{self.topology.state.validate_shell_history(&context.target,&context.proof.source_epoch)}
        else{self.topology.state.validate_shell(&context.target,&context.proof.source_epoch,shells::ShellAccess::Input)}
    }
    fn scroll_ready(&self)->SecurityResult<()> {
        self.check_current()?;self.screen()?;
        let context=self.context.shell()?;
        self.topology.state.validate_shell_history(&context.target,&context.proof.source_epoch)?;
        let history=self.history.lock().map_err(|_|ErrorCode::NotReady)?;
        if !history.enabled||history.available.is_none()||history.committed.is_none()||!self.coherent.load(Ordering::Acquire)||self.popup.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        Ok(())
    }
    fn projection(&self,surface:&endpoint::Surface,revision:u64)->SecurityResult<endpoint::Projection> {
        let mut projection=surface.projection(revision)?;
        let mut history=self.history.lock().map_err(|_|ErrorCode::NotReady)?;
        if history.enabled {
            projection.begin.scroll=surface.scroll().map(NativeScroll::metrics).transpose()?;
            history.observe(projection.begin.scroll.clone());
        }
        Ok(projection)
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
        if params.history {
            let context=lease.context.shell()?;
            self.state.validate_shell_history(&context.target,&context.proof.source_epoch)?;
            lease.history.lock().map_err(|_|ErrorCode::NotReady)?.enabled=true;
        }
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
            lease.projection(surface,0)
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
    pub(crate) fn validate_shell_scroll(&self,state:&State,connection:&Uuid,params:&ShellScrollParams)->SecurityResult<Arc<GridLease>> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.shell()?.target!=params.target{return Err(ErrorCode::StaleSession);}
        lease.scroll_ready()?;
        if lease.history.lock().map_err(|_|ErrorCode::NotReady)?.mode==HistoryMode::Pending{return Err(ErrorCode::Busy);}
        Ok(lease)
    }
    pub(crate) async fn shell_scroll_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&ShellScrollParams,deadline:Instant)->SecurityResult<()> {
        params.validate().map_err(|_|ErrorCode::InvalidRequest)?;before(deadline)?;
        let lease=self.validate_shell_scroll(&security.state,&auth.connection_id,params)?;
        let source=&lease.context.shell()?.source;
        let client=tokio::select!{biased;_=lease.cancelled()=>return Err(ErrorCode::StaleSession),result=timeout_at(deadline,Client::connect_source(source))=>result.map_err(|_|ErrorCode::Timeout)?.map_err(policy)?};
        before(deadline)?;lease.scroll_ready()?;
        let fd=client.fence_socket().map_err(policy)?;
        lease.history.lock().map_err(|_|ErrorCode::NotReady)?.pending()?;
        let mut intent=ScrollIntent{lease:lease.clone(),entered:false,finished:false};
        security.boundary(auth,action.clone(),||{before(deadline)?;lease.scroll_ready()},fd,async {
            intent.entered=true;
            let (reply,receive)=oneshot::channel();
            let mut completion=match lease.commands.as_ref().ok_or(ErrorCode::NotReady).and_then(|commands| {
                before(deadline)?;
                commands.try_send(Command::Scroll(ScrollRequest{client,offset:params.offset_from_bottom.0,deadline,reply})).map_err(|error|match error{mpsc::error::TrySendError::Full(_)=>ErrorCode::Busy,mpsc::error::TrySendError::Closed(_)=>ErrorCode::NotReady})
            }) {
                Ok(())=>receive.await.unwrap_or(ScrollCompletion::Uncertain),
                Err(code)=>ScrollCompletion::Refused(code),
            };
            if matches!(&completion,ScrollCompletion::Confirmed(_))&&(before(deadline).is_err()||auth.live_connection().is_err()||lease.check_current().is_err()) {completion=ScrollCompletion::Uncertain;}
            lease.history.lock().expect("history mutex").finish(&completion,params.offset_from_bottom.0==0);intent.finished=true;
            let (state,proof,result,error)=match completion {
                ScrollCompletion::Confirmed(metrics)=>(ReceiptState::Succeeded,crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("stock pane.scroll correlated original route".into())),Some(ActionResult::ShellScroll(metrics)),None),
                ScrollCompletion::Refused(code)=>(ReceiptState::Failed,crate::storage::TransitionProof::DefinitiveNoEffect(Id("stock scroll refused without effect".into())),None,Some(WireError{code,message:SafeMessage("History movement was refused before effect".into())})),
                ScrollCompletion::Uncertain=>(ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome,None,None),
            };
            crate::storage::ReceiptTransition{state,proof,result,error,at:pairing::now_ms()}
        }).await
    }
    pub(crate) fn validate_shell_input(&self,state:&State,connection:&Uuid,params:&ShellInputParams)->SecurityResult<Arc<GridLease>> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.shell()?.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        lease.input_ready(input_keys(&params.input))?;Ok(lease)
    }
    pub(crate) async fn await_shell_input(&self,state:&State,connection:&Uuid,params:&ShellInputParams,deadline:Instant)->SecurityResult<()> {
        let lease=state.grid_lease(connection,&params.subscription)?;
        if lease.context.shell()?.target!=params.target||!lease.active(){return Err(ErrorCode::StaleSession);}
        // Subscribe-before-recheck is owned by this exact lease; no authorization/State lock
        // is held while an ordinary same-original topology observation completes.
        lease.current(deadline).await?;lease.input_ready(input_keys(&params.input))
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
        let keys=input_keys(&params.input).to_vec();
        let fd=lease.fence_socket()?;
        security.shell_input_boundary(auth,action.clone(),params.target.clone(),source_epoch.clone(),fd,
            ||self.validate_shell_input(&security.state,&auth.connection_id,params).map(|_|()),|checkpoint|async move {
                let progress=ShellDispatchProgress::default();
                let owner=match checkpoint.owner(&params.target,&source_epoch){
                    Ok(owner)=>Arc::new(owner),Err(_)=>return input_transition(ShellDispatchProgress::default(),InputCompletion::Uncertain),
                };
                if checkpoint.checkpoint().await.is_err(){return input_transition(progress,InputCompletion::Refused(ErrorCode::NotReady));}
                let (reply,receive)=oneshot::channel();
                permit.send(Command::Input(InputRequest{bytes,keys,progress:owner.clone(),deadline,reply}));
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
    event(lease,security,auth,EventBody::GridCommit(GridCommit{revision}),deadline,bytes).await?;
    if matches!(&lease.context,GridContext::Shell(_)){let mut history=lease.history.lock().map_err(|_|ErrorCode::NotReady)?;if history.enabled{history.commit();}}
    Ok(())
}
async fn run(lease:&Arc<GridLease>,security:&Security,auth:&Authorized,session:&mut endpoint::Session,mut commands:mpsc::Receiver<Command>)->SecurityResult<()> {
    let mut revision=0u64;let mut pending:Option<PendingResize>=None;let mut coherence_deadline=None;let mut input:Option<InputRequest>=None;let mut scroll:Option<ScrollRequest>=None;
    loop {
        let deadline=pending.as_ref().map(|value|value.deadline).into_iter().chain(coherence_deadline).chain(input.as_ref().map(|value|value.deadline)).min();
        enum Wake {Command(Command),Native(endpoint::Incoming),InputReady,ScrollReady,ScrollExpired,ScrollAbandoned}
        let can_input=input.is_some()&&pending.is_none()&&scroll.is_none()&&session.coherent()&&!session.read_in_progress();
        let can_scroll=scroll.is_some()&&input.is_none()&&pending.is_none()&&session.coherent()&&!session.read_in_progress();
        let accept_command=input.is_none()&&pending.is_none()&&scroll.is_none();
        let scroll_deadline=scroll.as_ref().map(|request|request.deadline);
        let scroll_expired=async{match scroll_deadline{Some(deadline)=>tokio::time::sleep_until(deadline).await,None=>std::future::pending::<()>().await}};
        let scroll_abandoned=async{match scroll.as_mut(){Some(request)=>request.reply.closed().await,None=>std::future::pending::<()>().await}};
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
            _=scroll_abandoned=>Wake::ScrollAbandoned,
            _=scroll_expired=>Wake::ScrollExpired,
            result=next=>Wake::Native(result?),
            _=async {},if can_input=>Wake::InputReady,
            _=async {},if can_scroll=>Wake::ScrollReady,
            request=commands.recv(),if accept_command=>Wake::Command(request.ok_or(ErrorCode::StaleSession)?),
        };
        match wake {
            Wake::Command(Command::Scroll(request))=>{if !request.reply.is_closed(){scroll=Some(request);}},
            Wake::ScrollAbandoned=>{scroll.take();},
            Wake::ScrollExpired=>{if let Some(request)=scroll.take(){let _=request.reply.send(ScrollCompletion::Refused(ErrorCode::Timeout));}},
            Wake::ScrollReady=>{
                if !session.coherent()||session.read_in_progress(){continue;}
                let request=scroll.take().expect("selected original scroll");
                if request.reply.is_closed(){continue;}
                let checked=ready(lease,security,auth,request.deadline).await.and_then(|()|lease.scroll_ready()).and_then(|()|before(request.deadline));
                if let Err(code)=checked{let _=request.reply.send(ScrollCompletion::Refused(code));continue;}
                let context=lease.context.shell()?;let route=&context.proof.identity;
                let mut started=false;
                let response=tokio::select!{biased;_=lease.cancelled()=>Err(ApiError::Unavailable(invalid("original history lease retired"))),
                    result=timeout_at(request.deadline,request.client.pane_scroll(&route.pane_id,request.offset,&mut started,||{if request.reply.is_closed(){return Err(ErrorCode::StaleSession);}auth.live_connection()?;before(request.deadline)?;lease.scroll_ready()}))=>result.unwrap_or_else(|_|Err(ApiError::Unavailable(invalid("history deadline"))))};
                let completion=match response {
                    Ok(pane)=>{
                        let checked=ready(lease,security,auth,request.deadline).await.and_then(|()|lease.scroll_ready()).and_then(|()|{
                            if pane.pane_id!=route.pane_id||pane.terminal_id!=route.terminal_id||pane.workspace_id!=route.workspace_id||pane.tab_id!=route.tab_id{return Err(ErrorCode::StaleSession);}
                            pane.scroll.as_ref().ok_or(ErrorCode::NotReady)?.metrics()
                        });
                        match checked{Ok(metrics)=>ScrollCompletion::Confirmed(metrics),Err(_)=>ScrollCompletion::Uncertain}
                    },
                    Err(_) if !started=>ScrollCompletion::Refused(ErrorCode::NotReady),
                    Err(ApiError::Rejected(code)) if matches!(code.as_str(),"invalid_request"|"unknown_method")=>if lease.scroll_ready().is_ok()&&before(request.deadline).is_ok(){ScrollCompletion::Refused(ErrorCode::NotReady)}else{ScrollCompletion::Uncertain},
                    Err(_)=>ScrollCompletion::Uncertain,
                };
                let _=request.reply.send(completion);
            }
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
                let checked=ready(lease,security,auth,request.deadline).await.and_then(|()|lease.input_ready(&request.keys));
                if let Err(code)=checked {let _=request.reply.send(InputCompletion::Refused(code));continue;}
                let mut started=false;
                let sent=tokio::select!{biased;
                    _=lease.cancelled()=>Err(ErrorCode::StaleSession),
                    result=timeout_at(request.deadline,session.send_input(&request.bytes,&request.keys,&request.progress,&request.reply,request.deadline,&mut started))=>result.unwrap_or(Err(ErrorCode::Timeout)),
                };
                let plan=match sent {
                    Ok(plan)=>plan,
                    Err(code)=>{
                        if !started{let _=request.reply.send(InputCompletion::Refused(code));continue;}
                        let _=request.reply.send(InputCompletion::Uncertain);lease.cancel();return Err(ErrorCode::NotReady);
                    },
                };
                lease.input_written(plan.keys(&request.keys));
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
                let projection=lease.projection(surface,revision)?;let mut bytes=0;
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
        ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{target:self.lease.context.shell().unwrap().target.clone(),subscription:self.lease.id.clone(),seq:ZeroValue(0),frame:GridBegin { scroll: None, mode:GridMode::Replace,revision:Uint(0),columns:Uint(1),rows:Uint(1),cursor:None,alternate_screen:false,graphics_omitted:false,popup_blocked:false,part_count:Uint(1) }})
    }
}
#[cfg(test)]
impl Drop for GridQueueFixture {fn drop(&mut self){fs::remove_dir_all(&self.root).unwrap();}}
/// Real State/Topology/process/socket proof. Only the provider-specific old grid fixture is retired.
#[cfg(test)]
pub(crate) async fn queue_fixture(state:Arc<State>,server:Uuid)->GridQueueFixture {
    queue_fixture_on(state,server,None,false).await
}
#[cfg(test)]
async fn queue_fixture_on(state:Arc<State>,server:Uuid,existing:Option<Arc<Topology>>,private_executable:bool)->GridQueueFixture {
    use std::os::unix::fs::{DirBuilderExt,PermissionsExt};
    let native_exe=fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let id=Uuid::random().0;
    let root=native_exe.parent().unwrap().join(format!("gr-{}{}{}",&id[..8],&id[9..13],&id[14..18]));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let path=root.join("native.sock");
    assert!(path.as_os_str().as_encoded_bytes().len()<108);
    let listener=tokio::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
    let evidence=Client::connect(&path).await.unwrap().evidence;
    let exe=if private_executable {
        // Own only a configured link; never replace or execute a substitute process binary.
        let path=root.join("herdr");std::os::unix::fs::symlink(native_exe,&path).unwrap();path
    }else{native_exe};
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
                    lease.commands.as_ref().unwrap().try_send(Command::Input(InputRequest{bytes:bytes.clone(),keys:vec![],progress:owner.clone(),deadline,reply:send})).unwrap_or_else(|_|panic!("original bounded input slot"));
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
        let first=queue_fixture(state.clone(),Uuid::random()).await;let sibling=queue_fixture_on(state.clone(),Uuid::random(),Some(first.lease.topology.clone()),false).await;
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
        let sibling=queue_fixture_on(state.clone(),Uuid::random(),Some(fixture.lease.topology.clone()),false).await;
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
            assert_eq!(topology.validate_shell_scroll(&security.state,&auth.connection_id,&ShellScrollParams{boot_id:security.boot.clone(),target:shell.clone(),subscription:lease.id.clone(),offset_from_bottom:Uint(0)}).err(),Some(ErrorCode::NotReady));
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
                frame:GridBegin { scroll: None, mode:GridMode::Replace,revision:Uint(0),columns:Uint(1),rows:Uint(1),cursor:None,alternate_screen:false,graphics_omitted:true,popup_blocked:false,part_count:Uint(1) }});
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
    fn history_metrics(offset:u64)->GridScroll {GridScroll{offset_from_bottom:Uint(offset),max_offset_from_bottom:Uint(20),viewport_rows:Uint(1)}}
    #[test]
    fn history_bottom_ack_and_current_commit_are_both_required_in_either_order() {
        for ack_first in [false,true] {
            let mut history=HistoryState::default();history.observe(Some(history_metrics(7)));history.commit();
            assert!(history.mode==HistoryMode::Browsing);
            history.observe(Some(history_metrics(0)));history.commit();assert!(history.mode==HistoryMode::Browsing);
            history.observe(None);history.commit();assert!(history.mode==HistoryMode::Browsing);
            history.observe(Some(history_metrics(7)));history.commit();history.pending().unwrap();
            if ack_first {history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);assert!(history.mode==HistoryMode::Browsing);history.observe(Some(history_metrics(0)));history.commit();}
            else {history.observe(Some(history_metrics(0)));history.commit();assert!(history.mode==HistoryMode::Pending);history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);}
            assert!(history.mode==HistoryMode::Live);
            history.observe(Some(history_metrics(4)));history.commit();history.observe(Some(history_metrics(0)));history.commit();
            history.pending().unwrap();history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);
            assert!(history.mode==HistoryMode::Live,"no-op bottom needs no new revision");
            history.pending().unwrap();history.finish(&ScrollCompletion::Uncertain,false);
            history.observe(None);history.commit();history.observe(Some(history_metrics(0)));history.commit();
            assert!(history.mode==HistoryMode::Uncertain);
            history.pending().unwrap();history.finish(&ScrollCompletion::Refused(ErrorCode::NotReady),false);assert!(history.mode==HistoryMode::Uncertain);
            history.pending().unwrap();history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),false);
            assert!(history.mode==HistoryMode::Browsing,"an older intent clamped to zero is not explicit bottom");
        }
    }
    #[test]
    fn history_committing_nonbottom_after_pending_bottom_ack_never_leaves_live() {
        for ack_before_commit in [false,true] {
            let mut history=HistoryState::default();history.observe(Some(history_metrics(0)));history.commit();
            history.pending().unwrap();history.observe(Some(history_metrics(9)));
            if ack_before_commit{history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);assert!(history.mode==HistoryMode::Live);}
            history.commit();
            if !ack_before_commit{history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);}
            assert!(history.committed.as_ref().unwrap().offset_from_bottom.0>0&&history.mode==HistoryMode::Browsing);
            assert!(!history.bottom_ack||!ack_before_commit);
            history.pending().unwrap();history.observe(Some(history_metrics(0)));
            if ack_before_commit{history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);history.commit();}
            else{history.commit();history.finish(&ScrollCompletion::Confirmed(history_metrics(0)),true);}
            assert!(history.mode==HistoryMode::Live&&history.committed.as_ref().unwrap().offset_from_bottom.0==0);
        }
    }
    async fn history_output(security:&Security,auth:&Authorized,deadline:Instant)->Envelope {
        timeout_at(deadline,async {
            loop {
                let (queued,closed)=security.state.pop_tagged_output(&auth.connection_id).unwrap();assert!(!closed);
                if let Some(queued)=queued {
                    let (lease,written,until)=queued.grid.expect("original shell publication");
                    let _guard=lease.authorize(security,auth,until).await.unwrap();lease.check_current().unwrap();
                    let envelope=decode_payload(&queued.bytes,Some(Method::ShellGridSubscribe)).unwrap();
                    written.send(()).unwrap();return envelope;
                }
                tokio::task::yield_now().await;
            }
        }).await.unwrap()
    }
    async fn history_first_commit(lease:&Arc<GridLease>,security:&Security,auth:&Authorized,session:&mut endpoint::Session,peer:&mut UnixStream,deadline:Instant) {
        lease.history.lock().unwrap().enabled=true;
        let crop=endpoint::Rect{x:0,y:0,width:1,height:1};
        peer.write_all(&endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(1,&[],crop,Some([0,20,1]),None))).await.unwrap();session.next().await.unwrap();
        lease.coherent.store(session.coherent(),Ordering::Release);
        let projection=lease.projection(session.surface.as_ref().unwrap(),0).unwrap();let mut bytes=0;
        assert_eq!(lease.scroll_ready(),Err(ErrorCode::NotReady),"staging metrics before GridCommit cannot admit history");
        let response=ResultBody::ShellGridSubscribe(ShellGridSubscribeResult{target:lease.context.shell().unwrap().target.clone(),subscription:lease.id.clone(),seq:ZeroValue(0),frame:projection.begin.clone()});
        let receiver=security.state.grid_open_response(&auth.connection_id,Uuid::random(),response,lease.clone(),deadline,&mut bytes).unwrap();
        let publishing=async{written(lease,receiver,deadline).await.unwrap();publish_cells(lease,security,auth,projection,deadline,&mut bytes).await.unwrap();};
        let consuming=async{
            let Envelope::Response(Response{outcome:Ok(ResultBody::ShellGridSubscribe(opened)),..})=history_output(security,auth,deadline).await else{panic!("real history opening")};
            assert_eq!(opened.frame.scroll,Some(history_metrics(0)));
            assert!(matches!(history_output(security,auth,deadline).await,Envelope::Event(Event{body:EventBody::GridPart(_),..})));
            assert!(matches!(history_output(security,auth,deadline).await,Envelope::Event(Event{body:EventBody::GridCommit(_),..})));
        };
        tokio::join!(publishing,consuming);
        assert!(lease.scroll_ready().is_ok(),"the actual committed initial metrics admit history");
    }
    async fn history_input_action(security:&Security,auth:&Authorized,lease:&Arc<GridLease>,input:ShellInput,deadline:Instant)->Receipt {
        let target=lease.context.shell().unwrap().target.clone();
        let params=ShellInputParams{boot_id:security.boot.clone(),target,subscription:lease.id.clone(),input};
        let action=Uuid::random();let request=Request{id:action.clone(),body:RequestBody::ShellInput(params.clone())};
        let topology=lease.topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
        security.reserve(auth,request,move|request|{let RequestBody::ShellInput(p)=&request.body else{panic!()};topology.validate_shell_input(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
        lease.topology.shell_input_once(security,auth,&action,&params,deadline).await.unwrap();
        let proof=auth.proof.clone();security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap()
    }
    #[tokio::test]
    async fn history_releases_only_successfully_written_physical_holds_on_the_same_lease() {
        for forged_key in [false,true] {
            let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
            {
                let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();
                let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
                let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
                *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());lease.coherent.store(true,Ordering::Release);lease.history.lock().unwrap().enabled=true;
                let event=|phase,id|ShellKeyEvent{key:ShellKey::Left,modifiers:ShellKeyModifiers{shift:true,ctrl:true,alt:false,super_key:false},phase,physical_key_id:Some(Uint(id))};
                let scenario=async {
                    let press=ShellInput::Keys{events:vec![event(ShellKeyPhase::Press,42)]};
                    let expected=endpoint::pane_input(&Id("p".into()),&press).unwrap();
                    let reading=async{let mut actual=vec![0;expected.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);};
                    let (receipt,())=tokio::join!(history_input_action(&security,&auth,&lease,press,deadline),reading);
                    assert_eq!(receipt.state,ReceiptState::Dispatched);
                    let crop=endpoint::Rect{x:0,y:0,width:1,height:1};
                    peer.write_all(&endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(1,&[],crop,Some([7,20,1]),None))).await.unwrap();
                    for _ in 0..3 {history_output(&security,&auth,deadline).await;}
                    assert!(lease.history.lock().unwrap().mode==HistoryMode::Browsing);assert_eq!(lease.input_ready(&[]),Err(ErrorCode::NotReady));
                    for keys in [vec![event(ShellKeyPhase::Press,42)],vec![event(ShellKeyPhase::Repeat,42)],vec![event(ShellKeyPhase::Release,7)],vec![event(ShellKeyPhase::Release,42),event(ShellKeyPhase::Release,42)]] {
                        assert_eq!(lease.input_ready(&keys),Err(ErrorCode::NotReady));
                    }
                    let canonical=ShellInput::Keys{events:vec![event(ShellKeyPhase::Release,42)]};
                    let mut changed=event(ShellKeyPhase::Release,42);changed.modifiers=ShellKeyModifiers{shift:false,ctrl:false,alt:true,super_key:true};
                    if forged_key{changed.key=ShellKey::Character('𝛌');}
                    let release=ShellInput::Keys{events:vec![changed]};
                    let expected=endpoint::pane_input(&Id("p".into()),&canonical).unwrap();
                    assert_ne!(endpoint::pane_input(&Id("p".into()),&release).unwrap(),expected);
                    let reading=async{let mut actual=vec![0;expected.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);};
                    let (receipt,())=tokio::join!(history_input_action(&security,&auth,&lease,release,deadline),reading);
                    assert_eq!(receipt.state,ReceiptState::Dispatched);
                    let Some(ActionResult::ShellDispatch(result))=receipt.result else{panic!("actual canonical frame facts")};
                    assert_eq!(result.progress.frame_bytes,Some(expected.len() as u32));assert_eq!(result.progress.socket_bytes_written,Some(expected.len() as u32));assert!(result.progress.complete());
                    assert!(lease.held.lock().unwrap().is_empty());assert_eq!(lease.input_ready(&[event(ShellKeyPhase::Release,42)]),Err(ErrorCode::NotReady));
                    assert_eq!(peer.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock);lease.cancel();
                };
                let running=run(&lease,&security,&auth,&mut session,fixture.commands.take().unwrap());
                let (result,())=timeout_at(deadline,async {tokio::join!(running,scenario)}).await.unwrap();result.unwrap();
                lease.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&lease.id).unwrap();
            }
            pairing::tests::review_close(host,phone,security,auth,root).await;
        }
    }
    #[tokio::test]
    async fn partial_native_input_finishes_across_history_pending_without_a_second_frame() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();
            let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());lease.coherent.store(true,Ordering::Release);
            history_first_commit(&lease,&security,&auth,&mut session,&mut peer,deadline).await;
            let buffer:libc::c_int=4096;
            // SAFETY: live socket FD and correctly sized borrowed integer option.
            assert_eq!(unsafe{libc::setsockopt(session.client.reader.get_ref().as_raw_fd(),libc::SOL_SOCKET,libc::SO_SNDBUF,(&buffer as *const libc::c_int).cast(),std::mem::size_of_val(&buffer) as libc::socklen_t)},0);
            let input=ShellInput::Text{text:"x".repeat(16384)};let bytes=endpoint::pane_input(&Id("p".into()),&input).unwrap();
            let action=Uuid::random();let params=ShellInputParams{boot_id:security.boot.clone(),target:lease.context.shell().unwrap().target.clone(),subscription:lease.id.clone(),input};
            security.reserve(&auth,Request{id:action.clone(),body:RequestBody::ShellInput(params.clone())},|_|Ok(()),|_|{}).await.unwrap();
            let fd=session.client.fence_socket().unwrap();let epoch=lease.context.shell().unwrap().proof.source_epoch.clone();
            let retained=lease.clone();
            security.shell_input_boundary(&auth,action,params.target.clone(),epoch.clone(),fd,||Ok(()),|checkpoint|async move {
                let owner=Arc::new(checkpoint.owner(&params.target,&epoch).unwrap());
                checkpoint.checkpoint().await.unwrap();let (reply,_receive)=oneshot::channel();let mut started=false;
                {
                    let sending=session.send_input(&bytes,&[],&owner,&reply,deadline,&mut started);tokio::pin!(sending);
                    timeout_at(deadline,async{loop{assert!(futures_util::poll!(&mut sending).is_pending());if owner.progress().unwrap().socket_bytes_written.unwrap()>0{break;}tokio::task::yield_now().await;}}).await.unwrap();
                    assert!(!owner.progress().unwrap().complete());
                    lease.history.lock().unwrap().pending().unwrap();
                    let reading=async{let mut actual=vec![0;bytes.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,bytes);};
                    let (result,())=tokio::join!(&mut sending,reading);assert!(matches!(result.unwrap(),InputPlan::Original));
                }
                assert!(started&&owner.progress().unwrap().complete());assert_eq!(lease.input_ready(&[]),Err(ErrorCode::NotReady));
                assert_eq!(peer.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock);
                checkpoint.checkpoint().await.unwrap();input_transition(owner.progress().unwrap(),InputCompletion::Written)
            }).await.unwrap();
            retained.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&retained.id).unwrap();
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
    #[tokio::test]
    async fn stock_scroll_public_request_is_once_correlated_clamped_and_never_replayed() {
        for change in 0..18 {
            let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
            {
                let mut fixture=queue_fixture_on(security.state.clone(),Uuid::random(),None,true).await;let lease=fixture.lease.clone();let topology=fixture.topology();
                let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
                let (mut probe,_)=fixture._listener.accept().await.unwrap();assert_eq!(probe.read(&mut [0]).await.unwrap(),0);
                let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
                *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());lease.coherent.store(true,Ordering::Release);
                history_first_commit(&lease,&security,&auth,&mut session,&mut peer,deadline).await;
                let params=ShellScrollParams{boot_id:security.boot.clone(),target:lease.context.shell().unwrap().target.clone(),subscription:lease.id.clone(),offset_from_bottom:Uint(999)};
                let request=Request{id:Uuid::random(),body:RequestBody::ShellScroll(params.clone())};let calls=AtomicU64::new(0);
                let action_deadline=if change==16{Instant::now()+Duration::from_secs(1)}else{deadline};
                let producer=async {
                    let (stream,_)=fixture._listener.accept().await.unwrap();let mut public=BufReader::new(stream);
                    let native:Value=serde_json::from_slice(&read_line(&mut public).await.unwrap()).unwrap();calls.fetch_add(1,Ordering::SeqCst);
                    assert_eq!(native["method"],"pane.scroll");assert_eq!(native["params"],json!({"pane_id":"p","offset_from_bottom":999}));assert_eq!(native.as_object().unwrap().len(),3);
                    let mut pane=json!({"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t","focused":true,"agent_status":"idle","scroll":{"offset_from_bottom":20,"max_offset_from_bottom":20,"viewport_rows":1}});
                    let mut id=native["id"].clone();let mut kind="pane_info";
                    match change {
                        1=>id=json!(Uuid::random()),2=>kind="workspace_info",3=>pane["pane_id"]=json!("replacement"),4=>pane["terminal_id"]=json!("replacement"),
                        5=>pane["workspace_id"]=json!("replacement"),6=>pane["tab_id"]=json!("replacement"),7=>pane["scroll"]["viewport_rows"]=json!(0),
                        8=>topology.inner.lock().unwrap().servers.get_mut(&fixture.source.server_id).unwrap().source.as_mut().unwrap().incarnation=Uuid::random(),
                        9=>topology.inner.lock().unwrap().shell_bindings.get_mut(&params.target.shell_id).unwrap().proof.source_epoch=Uuid::random(),
                        12=>topology.inner.lock().unwrap().shell_bindings.get_mut(&params.target.shell_id).unwrap().target.generation=Uuid::random(),
                        13=>{
                            use std::os::unix::fs::PermissionsExt;
                            fs::remove_file(&fixture.source.socket_path).unwrap();
                            let _replacement=tokio::net::UnixListener::bind(&fixture.source.socket_path).unwrap();
                            fs::set_permissions(&fixture.source.socket_path,fs::Permissions::from_mode(0o600)).unwrap();
                        },
                        14=>{
                            let configured=&fixture.source.executable.configured_path;
                            assert!(configured.starts_with(&fixture.root));fs::remove_file(configured).unwrap();
                            fs::write(configured,b"owned replacement executable object, never executed").unwrap();
                        },
                        15=>{security.revoke(auth.proof.peer_id().clone(),None).await.unwrap();},
                        16=>tokio::time::sleep_until(action_deadline+Duration::from_millis(20)).await,
                        17=>return,
                        _=>{},
                    }
                    let response=if change==10 {json!({"id":id,"error":{"code":"invalid_request","message":"request rejected before dispatch"}})}
                        else if change==11 {json!({"id":id,"error":{"code":"pane_not_found","message":"ambiguous route disappearance"}})}
                        else {json!({"id":id,"result":{"type":kind,"pane":pane}})};
                    let mut bytes=serde_json::to_vec(&response).unwrap();bytes.push(b'\n');
                    if matches!(change,15|16){let _=public.get_mut().write_all(&bytes).await;}else{public.get_mut().write_all(&bytes).await.unwrap();}
                };
                let client=async {
                    let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
                    security.reserve(&auth,request.clone(),move|request|{let RequestBody::ShellScroll(p)=&request.body else{panic!()};validator.validate_shell_scroll(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
                    let outcome=topology.shell_scroll_once(&security,&auth,&request.id,&params,action_deadline).await;
                    if change==15 {
                        assert!(auth.live_connection().is_err());assert!(matches!(security.state.pop_output(&auth.connection_id),Err(ErrorCode::Unauthorized)));
                        assert!(lease.history.lock().unwrap().mode==HistoryMode::Uncertain);lease.cancel();return;
                    }
                    outcome.unwrap();
                    let proof=auth.proof.clone();let action=request.id.clone();
                    let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
                    assert_eq!(receipt.state,if change==0{ReceiptState::Succeeded}else if change==10{ReceiptState::Failed}else{ReceiptState::Uncertain});
                    if change==0 {assert_eq!(receipt.result,Some(ActionResult::ShellScroll(history_metrics(20))));}else{assert!(receipt.result.is_none());}
                    lease.cancel();
                    let repeated=security.reserve(&auth,request.clone(),|_|panic!("duplicate must return its old receipt before a stale lease check"),|_|panic!("duplicate cannot enter another effect")).await.unwrap();
                    let crate::storage::Reservation::Existing(old)=repeated else{panic!("duplicate action reminted")};assert_eq!(old,receipt);
                };
                let exercise=async{tokio::join!(producer,client);lease.cancel();};
                let running=run(&lease,&security,&auth,&mut session,fixture.commands.take().unwrap());
                let (outcome,())=timeout_at(deadline,async{tokio::join!(running,exercise)}).await.unwrap();outcome.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst),1);
                {let extra=fixture._listener.accept();tokio::pin!(extra);assert!(futures_util::poll!(&mut extra).is_pending());}
                lease.socket.lock().unwrap().take();if change!=15{security.state.unsubscribe(&auth.connection_id,&lease.id).unwrap();}
            }
            pairing::tests::review_close(host,phone,security,auth,root).await;
        }
    }
    #[tokio::test]
    async fn legacy_history_false_nonbottom_preserves_real_input_resize_and_old_grid_fields() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();let topology=fixture.topology();
            let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());lease.coherent.store(true,Ordering::Release);
            assert!(!lease.history.lock().unwrap().enabled);
            let scenario=async {
                let crop=endpoint::Rect{x:0,y:0,width:1,height:1};
                peer.write_all(&endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(1,&[],crop,Some([7,20,1]),None))).await.unwrap();
                let Envelope::Event(Event{body:EventBody::GridBegin(GridBeginEvent{frame}),..})=history_output(&security,&auth,deadline).await else{panic!("legacy replacement Begin")};
                assert!(frame.scroll.is_none());assert!(serde_json::to_value(&frame).unwrap().get("scroll").is_none());
                for _ in 0..2{history_output(&security,&auth,deadline).await;}
                {let history=lease.history.lock().unwrap();assert!(history.mode==HistoryMode::Live&&history.available.is_none()&&history.committed.is_none());}
                let input=ShellInput::Text{text:"legacy input".into()};let expected=endpoint::pane_input(&Id("p".into()),&input).unwrap();
                let reading=async{let mut actual=vec![0;expected.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);};
                let (receipt,())=tokio::join!(history_input_action(&security,&auth,&lease,input,deadline),reading);assert_eq!(receipt.state,ReceiptState::Dispatched);
                let params=ShellResizeParams{boot_id:security.boot.clone(),target:lease.context.shell().unwrap().target.clone(),subscription:lease.id.clone(),columns:Uint(1),rows:Uint(1)};
                let action=Uuid::random();let request=Request{id:action.clone(),body:RequestBody::ShellResize(params.clone())};
                let resizing=async {
                    let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
                    security.reserve(&auth,request,move|request|{let RequestBody::ShellResize(p)=&request.body else{panic!()};validator.validate_shell_resize(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
                    topology.shell_resize_once(&security,&auth,&action,&params,deadline).await.unwrap();
                    let proof=auth.proof.clone();let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
                    assert_eq!(receipt.state,ReceiptState::Succeeded);
                };
                let native=async {
                    let expected=endpoint::resize(1,1).unwrap();let mut actual=vec![0;expected.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);
                    peer.write_all(&endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(2,&[],crop,Some([8,20,1]),None))).await.unwrap();
                    let Envelope::Event(Event{body:EventBody::GridBegin(GridBeginEvent{frame}),..})=history_output(&security,&auth,deadline).await else{panic!("legacy resize Begin")};assert!(frame.scroll.is_none());
                    for _ in 0..2{history_output(&security,&auth,deadline).await;}
                };
                tokio::join!(resizing,native);assert!(lease.history.lock().unwrap().mode==HistoryMode::Live);lease.cancel();
            };
            let running=run(&lease,&security,&auth,&mut session,fixture.commands.take().unwrap());
            // All consumer assertions finish before cancellation; shutdown may win the final write acknowledgement.
            let (_,())=timeout_at(deadline,async{tokio::join!(running,scenario)}).await.unwrap();
            lease.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&lease.id).unwrap();
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
    #[tokio::test]
    async fn queued_scroll_waits_for_actual_resize_confirmation_and_publication() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();let topology=fixture.topology();
            let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut probe,_)=fixture._listener.accept().await.unwrap();assert_eq!(probe.read(&mut [0]).await.unwrap(),0);
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());
            history_first_commit(&lease,&security,&auth,&mut session,&mut peer,deadline).await;
            let target=lease.context.shell().unwrap().target.clone();
            let resize=ShellResizeParams{boot_id:security.boot.clone(),target:target.clone(),subscription:lease.id.clone(),columns:Uint(1),rows:Uint(1)};
            let scroll=ShellScrollParams{boot_id:security.boot.clone(),target,subscription:lease.id.clone(),offset_from_bottom:Uint(8)};
            let (entered,waiting)=oneshot::channel();let calls=AtomicU64::new(0);
            let resizing=async {
                let request=Request{id:Uuid::random(),body:RequestBody::ShellResize(resize.clone())};let action=request.id.clone();
                let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
                security.reserve(&auth,request,move|request|{let RequestBody::ShellResize(p)=&request.body else{panic!()};validator.validate_shell_resize(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
                topology.shell_resize_once(&security,&auth,&action,&resize,deadline).await.unwrap();
            };
            let scrolling=async {
                waiting.await.unwrap();
                let request=Request{id:Uuid::random(),body:RequestBody::ShellScroll(scroll.clone())};let action=request.id.clone();
                let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
                security.reserve(&auth,request,move|request|{let RequestBody::ShellScroll(p)=&request.body else{panic!()};validator.validate_shell_scroll(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
                topology.shell_scroll_once(&security,&auth,&action,&scroll,deadline).await.unwrap();
                let proof=auth.proof.clone();let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
                assert_eq!(receipt.state,ReceiptState::Succeeded);assert_eq!(receipt.result,Some(ActionResult::ShellScroll(history_metrics(7))));
                lease.cancel();
            };
            let producer=async {
                let expected=endpoint::resize(1,1).unwrap();let mut actual=vec![0;expected.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);entered.send(()).unwrap();
                let (public,_)=fixture._listener.accept().await.unwrap();
                timeout_at(deadline,async{while lease.commands.as_ref().unwrap().capacity()!=0||lease.history.lock().unwrap().mode!=HistoryMode::Pending{tokio::task::yield_now().await;}}).await.unwrap();
                assert_eq!(public.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock,"capturing/queueing is not prior resize completion");
                let crop=endpoint::Rect{x:0,y:0,width:1,height:1};
                peer.write_all(&endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(2,&[],crop,Some([0,20,1]),None))).await.unwrap();
                for _ in 0..3{history_output(&security,&auth,deadline).await;}
                let mut public=BufReader::new(public);let request:Value=serde_json::from_slice(&read_line(&mut public).await.unwrap()).unwrap();calls.fetch_add(1,Ordering::SeqCst);
                assert_eq!(request["method"],"pane.scroll");assert_eq!(request["params"],json!({"pane_id":"p","offset_from_bottom":8}));
                let mut bytes=serde_json::to_vec(&json!({"id":request["id"],"result":{"type":"pane_info","pane":{"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t","focused":true,"agent_status":"idle","scroll":{"offset_from_bottom":7,"max_offset_from_bottom":20,"viewport_rows":1}}}})).unwrap();bytes.push(b'\n');public.get_mut().write_all(&bytes).await.unwrap();
            };
            let exercise=async{tokio::join!(resizing,scrolling,producer);};
            let running=run(&lease,&security,&auth,&mut session,fixture.commands.take().unwrap());
            let (outcome,())=timeout_at(deadline,async{tokio::join!(running,exercise)}).await.unwrap();outcome.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
            lease.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&lease.id).unwrap();
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
    #[tokio::test]
    async fn pending_history_refuses_previously_queued_input_before_one_public_scroll() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();let topology=fixture.topology();
            let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut probe,_)=fixture._listener.accept().await.unwrap();assert_eq!(probe.read(&mut [0]).await.unwrap(),0);
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());
            history_first_commit(&lease,&security,&auth,&mut session,&mut peer,deadline).await;
            let crop=endpoint::Rect{x:0,y:0,width:1,height:1};
            let fragmented=endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(2,&[],crop,Some([0,20,1]),None));peer.write_all(&fragmented[..2]).await.unwrap();
            let input=ShellInput::Text{text:"must remain unsent".into()};let bytes=endpoint::pane_input(&Id("p".into()),&input).unwrap();
            let target=lease.context.shell().unwrap().target.clone();let epoch=lease.context.shell().unwrap().proof.source_epoch.clone();
            let action=Uuid::random();let params=ShellInputParams{boot_id:security.boot.clone(),target:target.clone(),subscription:lease.id.clone(),input};
            security.reserve(&auth,Request{id:action.clone(),body:RequestBody::ShellInput(params)},|_|Ok(()),|_|{}).await.unwrap();
            let fd=session.client.fence_socket().unwrap();let commands=fixture.commands.take().unwrap();let retained=lease.clone();
            let secured=security.clone();let authorization=auth.clone();let listener=&fixture._listener;
            security.shell_input_boundary(&auth,action,target.clone(),epoch.clone(),fd,||Ok(()),|checkpoint|async move {
                let owner=Arc::new(checkpoint.owner(&target,&epoch).unwrap());
                owner.retain(&ShellDispatchProgress{frame_bytes:Some(bytes.len() as u32),socket_bytes_written:Some(0),frame_complete:Some(false)}).unwrap();checkpoint.checkpoint().await.unwrap();
                let (reply,receive)=oneshot::channel();lease.commands.as_ref().unwrap().try_send(Command::Input(InputRequest{bytes,keys:vec![],progress:owner.clone(),deadline,reply})).unwrap_or_else(|_|panic!("one original command slot"));
                let running=run(&lease,&secured,&authorization,&mut session,commands);tokio::pin!(running);assert!(futures_util::poll!(&mut running).is_pending());
                assert_eq!(lease.commands.as_ref().unwrap().capacity(),1,"the old input is owned by run, not merely in the queue");
                assert_eq!(peer.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock);
                let scroll=ShellScrollParams{boot_id:secured.boot.clone(),target:target.clone(),subscription:lease.id.clone(),offset_from_bottom:Uint(7)};
                let scrolling=async {
                    let request=Request{id:Uuid::random(),body:RequestBody::ShellScroll(scroll.clone())};let action=request.id.clone();
                    let validator=topology.clone();let state=secured.state.clone();let connection=authorization.connection_id.clone();
                    secured.reserve(&authorization,request,move|request|{let RequestBody::ShellScroll(p)=&request.body else{panic!()};validator.validate_shell_scroll(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
                    topology.shell_scroll_once(&secured,&authorization,&action,&scroll,deadline).await.unwrap();
                };
                let producer=async {
                    let (stream,_)=listener.accept().await.unwrap();let mut public=BufReader::new(stream);
                    timeout_at(deadline,async{while lease.history.lock().unwrap().mode!=HistoryMode::Pending{tokio::task::yield_now().await;}}).await.unwrap();
                    peer.write_all(&fragmented[2..]).await.unwrap();for _ in 0..3{history_output(&secured,&authorization,deadline).await;}
                    let request:Value=serde_json::from_slice(&read_line(&mut public).await.unwrap()).unwrap();assert_eq!(request["method"],"pane.scroll");assert_eq!(request["params"],json!({"pane_id":"p","offset_from_bottom":7}));
                    assert_eq!(peer.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock,"previously queued text cannot precede the history movement");
                    let mut response=serde_json::to_vec(&json!({"id":request["id"],"result":{"type":"pane_info","pane":{"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t","focused":true,"agent_status":"idle","scroll":{"offset_from_bottom":7,"max_offset_from_bottom":20,"viewport_rows":1}}}})).unwrap();response.push(b'\n');public.get_mut().write_all(&response).await.unwrap();
                };
                let exercise=async{tokio::join!(scrolling,producer);lease.cancel();};
                let (outcome,())=timeout_at(deadline,async{tokio::join!(&mut running,exercise)}).await.unwrap();outcome.unwrap();
                let completion=receive.await.unwrap();assert!(matches!(completion,InputCompletion::Refused(ErrorCode::NotReady)));
                assert_eq!(owner.progress().unwrap().socket_bytes_written,Some(0));checkpoint.checkpoint().await.unwrap();
                input_transition(owner.progress().unwrap(),completion)
            }).await.unwrap();
            retained.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&retained.id).unwrap();
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
    #[tokio::test]
    async fn queued_scroll_deadline_refuses_without_one_native_byte_or_input_replay() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();let topology=fixture.topology();
            let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut probe,_)=fixture._listener.accept().await.unwrap();assert_eq!(probe.read(&mut [0]).await.unwrap(),0);
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());
            history_first_commit(&lease,&security,&auth,&mut session,&mut peer,deadline).await;
            let crop=endpoint::Rect{x:0,y:0,width:1,height:1};
            let pending_frame=endpoint::fixture::packet(&endpoint::fixture::patch_with_scroll(2,&[],crop,Some([0,20,1]),None));peer.write_all(&pending_frame[..2]).await.unwrap();
            let action_deadline=Instant::now()+Duration::from_secs(1);
            let params=ShellScrollParams{boot_id:security.boot.clone(),target:lease.context.shell().unwrap().target.clone(),subscription:lease.id.clone(),offset_from_bottom:Uint(7)};
            let request=Request{id:Uuid::random(),body:RequestBody::ShellScroll(params.clone())};let action=request.id.clone();let (finished,waiting)=oneshot::channel();
            let client=async {
                let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
                security.reserve(&auth,request,move|request|{let RequestBody::ShellScroll(p)=&request.body else{panic!()};validator.validate_shell_scroll(&state,&connection,p).map(|_|())},|_|{}).await.unwrap();
                topology.shell_scroll_once(&security,&auth,&action,&params,action_deadline).await.unwrap();
                let proof=auth.proof.clone();let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
                assert_eq!(receipt.state,ReceiptState::Failed);assert_eq!(receipt.error.as_ref().unwrap().code,ErrorCode::Timeout);assert!(receipt.result.is_none());
                assert!(lease.history.lock().unwrap().mode==HistoryMode::Live,"definitive zero-effect refusal restores the prior committed-bottom Live mode");
                finished.send(()).unwrap();
            };
            let producer=async {
                let (mut public,_)=fixture._listener.accept().await.unwrap();waiting.await.unwrap();
                let mut byte=[0];let observed=tokio::time::timeout(Duration::from_millis(50),public.read(&mut byte)).await;
                assert!(matches!(observed,Ok(Ok(0))|Err(_)),"an expired queued request must never write a native NDJSON prefix");
                assert_eq!(peer.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock);lease.cancel();
            };
            let exercise=async{tokio::join!(client,producer);};
            let running=run(&lease,&security,&auth,&mut session,fixture.commands.take().unwrap());
            let (outcome,())=timeout_at(deadline,async{tokio::join!(running,exercise)}).await.unwrap();outcome.unwrap();
            lease.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&lease.id).unwrap();
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
    async fn direct_input_frame(security:&Security,auth:&Authorized,lease:&Arc<GridLease>,session:&mut endpoint::Session,input:ShellInput,deadline:Instant)->Receipt {
        let target=lease.context.shell().unwrap().target.clone();let epoch=lease.context.shell().unwrap().proof.source_epoch.clone();
        let params=ShellInputParams{boot_id:security.boot.clone(),target:target.clone(),subscription:lease.id.clone(),input};
        let action=Uuid::random();let request=Request{id:action.clone(),body:RequestBody::ShellInput(params.clone())};
        let validator=lease.topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
        security.reserve(auth,request,move|request|{let RequestBody::ShellInput(params)=&request.body else{panic!()};validator.validate_shell_input(&state,&connection,params).map(|_|())},|_|{}).await.unwrap();
        let keys=input_keys(&params.input).to_vec();let bytes=endpoint::pane_input(&lease.context.shell().unwrap().proof.identity.pane_id,&params.input).unwrap();
        let fd=session.client.fence_socket().unwrap();let effect_target=target.clone();let effect_epoch=epoch.clone();
        security.shell_input_boundary(auth,action.clone(),target,epoch,fd,||lease.input_ready(input_keys(&params.input)),|checkpoint|async move {
            let owner=checkpoint.owner(&effect_target,&effect_epoch).unwrap();checkpoint.checkpoint().await.unwrap();
            let (reply,_receive)=oneshot::channel();let mut started=false;
            let plan=session.send_input(&bytes,&keys,&owner,&reply,deadline,&mut started).await.unwrap();
            assert!(started);lease.input_written(plan.keys(&keys));checkpoint.checkpoint().await.unwrap();
            input_transition(owner.progress().unwrap(),InputCompletion::Written)
        }).await.unwrap();
        let proof=auth.proof.clone();security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap()
    }
    #[tokio::test]
    async fn zero_byte_pending_write_replans_cleanup_and_live_caller_bytes_with_exact_progress() {
        for back_to_live in [false,true] {
            let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
            {
                let fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();
                let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
                let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
                *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());
                history_first_commit(&lease,&security,&auth,&mut session,&mut peer,deadline).await;
                let press=ShellKeyEvent{key:ShellKey::Left,modifiers:ShellKeyModifiers{shift:true,ctrl:true,alt:false,super_key:false},phase:ShellKeyPhase::Press,physical_key_id:Some(Uint(42))};
                let input=ShellInput::Keys{events:vec![press.clone()]};let encoded=endpoint::pane_input(&Id("p".into()),&input).unwrap();
                direct_input_frame(&security,&auth,&lease,&mut session,input,deadline).await;
                let mut observed=vec![0;encoded.len()];peer.read_exact(&mut observed).await.unwrap();assert_eq!(observed,encoded);
                let buffer:libc::c_int=65536;
                // SAFETY: this original live socket and integer option remain owned by the fixture.
                assert_eq!(unsafe{libc::setsockopt(session.client.reader.get_ref().as_raw_fd(),libc::SOL_SOCKET,libc::SO_SNDBUF,(&buffer as *const libc::c_int).cast(),std::mem::size_of_val(&buffer) as libc::socklen_t)},0);
                let filler=ShellInput::Text{text:"x".repeat(16384)};let filler_bytes=endpoint::pane_input(&Id("p".into()),&filler).unwrap();
                assert_eq!(direct_input_frame(&security,&auth,&lease,&mut session,filler,deadline).await.state,ReceiptState::Dispatched);
                let buffer:libc::c_int=4096;
                // Shrinking below the already queued, fully written text frame creates real byte0 backpressure.
                assert_eq!(unsafe{libc::setsockopt(session.client.reader.get_ref().as_raw_fd(),libc::SOL_SOCKET,libc::SO_SNDBUF,(&buffer as *const libc::c_int).cast(),std::mem::size_of_val(&buffer) as libc::socklen_t)},0);
                let mut canonical=press;canonical.phase=ShellKeyPhase::Release;
                let mut caller=canonical.clone();caller.key=ShellKey::Character('𝛌');caller.modifiers=ShellKeyModifiers{shift:false,ctrl:false,alt:true,super_key:true};
                let input=ShellInput::Keys{events:vec![caller.clone()]};let bytes=endpoint::pane_input(&Id("p".into()),&input).unwrap();
                let canonical_bytes=endpoint::pane_input(&Id("p".into()),&ShellInput::Keys{events:vec![canonical.clone()]}).unwrap();assert_ne!(bytes.len(),canonical_bytes.len());
                if back_to_live{lease.history.lock().unwrap().pending().unwrap();}
                let expected=if back_to_live{bytes.clone()}else{canonical_bytes};
                let target=lease.context.shell().unwrap().target.clone();let epoch=lease.context.shell().unwrap().proof.source_epoch.clone();
                let action=Uuid::random();let params=ShellInputParams{boot_id:security.boot.clone(),target:target.clone(),subscription:lease.id.clone(),input};
                security.reserve(&auth,Request{id:action.clone(),body:RequestBody::ShellInput(params)},|_|Ok(()),|_|{}).await.unwrap();
                let retained=lease.clone();let fd=session.client.fence_socket().unwrap();
                security.shell_input_boundary(&auth,action.clone(),target.clone(),epoch.clone(),fd,||Ok(()),|checkpoint|async move {
                    let owner=checkpoint.owner(&target,&epoch).unwrap();checkpoint.checkpoint().await.unwrap();
                    let (reply,_receive)=oneshot::channel();let mut started=false;let keys=vec![caller];
                    let plan={
                        let sending=session.send_input(&bytes,&keys,&owner,&reply,deadline,&mut started);tokio::pin!(sending);
                        assert!(futures_util::poll!(&mut sending).is_pending());assert_eq!(owner.progress().unwrap(),ShellDispatchProgress::default());
                        assert!(lease.held.lock().unwrap().contains_key(&42));
                        if back_to_live{lease.history.lock().unwrap().finish(&ScrollCompletion::Refused(ErrorCode::NotReady),false);}
                        else{lease.history.lock().unwrap().pending().unwrap();}
                        let reading=async {
                            let mut prelude=vec![0;filler_bytes.len()];peer.read_exact(&mut prelude).await.unwrap();assert_eq!(prelude,filler_bytes);
                            let mut actual=vec![0;expected.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);
                        };
                        let (sent,())=tokio::join!(&mut sending,reading);sent.unwrap()
                    };
                    assert!(started);assert_eq!(owner.progress().unwrap().frame_bytes,Some(expected.len() as u32));assert!(owner.progress().unwrap().complete());
                    assert!(lease.held.lock().unwrap().contains_key(&42),"the ID is not consumed while planning or waiting at byte0");
                    assert_eq!(plan.keys(&keys),if back_to_live{keys.as_slice()}else{std::slice::from_ref(&canonical)});
                    lease.input_written(plan.keys(&keys));assert!(lease.held.lock().unwrap().is_empty());
                    assert_eq!(peer.try_read(&mut [0]).unwrap_err().kind(),io::ErrorKind::WouldBlock);
                    checkpoint.checkpoint().await.unwrap();input_transition(owner.progress().unwrap(),InputCompletion::Written)
                }).await.unwrap();
                let proof=auth.proof.clone();let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
                assert_eq!(receipt.state,ReceiptState::Dispatched);
                retained.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&retained.id).unwrap();
            }
            pairing::tests::review_close(host,phone,security,auth,root).await;
        }
    }
    #[tokio::test]
    async fn input_progress_retention_failure_after_kernel_write_is_uncertain() {
        let (host,phone,security,auth,root)=pairing::tests::review_authorized().await;
        {
            let mut fixture=queue_fixture(security.state.clone(),Uuid::random()).await;let lease=fixture.lease.clone();
            let deadline=Instant::now()+Duration::from_secs(5);security.state.reserve_grid(&auth.connection_id,lease.clone(),deadline).unwrap();
            let (mut session,mut peer)=endpoint::fixture::established(lease.context.shell().unwrap(),Arc::downgrade(&lease),auth.clone()).await;
            *lease.socket.lock().unwrap()=Some(session.client.fence_socket().unwrap());lease.coherent.store(true,Ordering::Release);
            let input=ShellInput::Text{text:"written before fact retention failure".into()};let bytes=endpoint::pane_input(&Id("p".into()),&input).unwrap();
            let action=Uuid::random();let target=lease.context.shell().unwrap().target.clone();let epoch=lease.context.shell().unwrap().proof.source_epoch.clone();
            let params=ShellInputParams{boot_id:security.boot.clone(),target:target.clone(),subscription:lease.id.clone(),input};
            security.reserve(&auth,Request{id:action.clone(),body:RequestBody::ShellInput(params)},|_|Ok(()),|_|{}).await.unwrap();
            let fd=session.client.fence_socket().unwrap();let commands=fixture.commands.take().unwrap();let retained=lease.clone();
            let secured=security.clone();let authorization=auth.clone();
            security.shell_input_boundary(&auth,action.clone(),target.clone(),epoch.clone(),fd,||Ok(()),|checkpoint|async move {
                let owner=Arc::new(checkpoint.owner(&target,&epoch).unwrap());
                owner.retain(&ShellDispatchProgress{frame_bytes:Some(bytes.len() as u32+1),socket_bytes_written:Some(0),frame_complete:Some(false)}).unwrap();checkpoint.checkpoint().await.unwrap();
                let (reply,receive)=oneshot::channel();lease.commands.as_ref().unwrap().try_send(Command::Input(InputRequest{bytes:bytes.clone(),keys:vec![],progress:owner.clone(),deadline,reply})).unwrap_or_else(|_|panic!("one bounded input slot"));
                let reading=async{let mut actual=vec![0;bytes.len()];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual,bytes);};
                let running=run(&lease,&secured,&authorization,&mut session,commands);
                let (outcome,())=tokio::join!(running,reading);assert!(outcome.is_err());
                let completion=receive.await.unwrap();assert!(matches!(completion,InputCompletion::Uncertain));assert_eq!(owner.progress().unwrap().socket_bytes_written,Some(0));
                checkpoint.checkpoint().await.unwrap();input_transition(owner.progress().unwrap(),completion)
            }).await.unwrap();
            let proof=auth.proof.clone();let receipt=security.storage.call(move|store|store.status(&proof,&action,pairing::now_ms())).await.unwrap().unwrap();
            assert_eq!(receipt.state,ReceiptState::Uncertain);retained.socket.lock().unwrap().take();security.state.unsubscribe(&auth.connection_id,&retained.id).unwrap();
        }
        pairing::tests::review_close(host,phone,security,auth,root).await;
    }
}
