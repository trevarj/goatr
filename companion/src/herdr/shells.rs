// SPDX-License-Identifier: GPL-3.0-or-later
//! Positive native ordinary-shell authority. No provider, cwd or pane fallback.
use super::*;

pub(super) const CREATION_JOBS:u32=64;

/// Negative admission facts owned by the original creation receipt, not another job queue.
/// Unknown entered outcomes retain the original baseline: only previously observed panes
/// remain eligible. A correlated complete response narrows this to its exact new root.
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub(crate) struct CreationShellFence {
    pub server:Uuid,pub incarnation:Option<Uuid>,pub baseline:HashMap<Id,Id>,pub created:Option<(Id,Id)>,
}
impl CreationShellFence {
    pub(crate) fn validate(&self)->crate::protocol::Validation {
        self.server.validate()?;self.incarnation.validate()?;
        if self.baseline.len()>512||self.created.is_none()&&self.incarnation.is_none(){return Err(crate::protocol::ProtocolFailure{kind:crate::protocol::FailureKind::InvalidRequest,request_id:None,close:true});}
        for (pane,terminal) in &self.baseline {pane.validate()?;terminal.validate()?;}
        if let Some((pane,terminal))=&self.created {pane.validate()?;terminal.validate()?;}
        Ok(())
    }
    pub(crate) fn blocks(&self,server:&Uuid,incarnation:&Uuid,pane:&Id,terminal:&Id)->bool {
        self.server==*server&&self.incarnation.as_ref().is_none_or(|known|known==incarnation)&&match &self.created {
            Some((original_pane,original_terminal))=>original_pane==pane&&original_terminal==terminal,
            None=>self.baseline.get(pane)!=Some(terminal),
        }
    }
}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeShellScope {
    pub pane_id:Id,pub terminal_id:Id,pub workspace_id:Id,pub tab_id:Id,
    pub read_identity:PaneReadIdentity,pub permission_epoch:u64,
}
impl NativeShellScope {
    pub(super) fn validate(&self)->io::Result<()> {
        id_valid(&self.pane_id)?;id_valid(&self.terminal_id)?;id_valid(&self.workspace_id)?;id_valid(&self.tab_id)?;
        self.read_identity.validate()?;
        if self.permission_epoch==0 || self.read_identity.runtime_epoch==0 || self.read_identity.binding_epoch==0 {return Err(invalid("invalid shell permission epoch"))}Ok(())
    }
}
#[derive(Clone,Debug,Deserialize,PartialEq,Eq)]
pub struct NativeShellInfo {pub scope:NativeShellScope,pub read:bool,pub create:bool,pub input:bool,pub close:bool}
/// Logical routes carry no invented native epoch. Optional identity decodes historical proofs only.
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellIdentity {
    pub pane_id:Id,pub terminal_id:Id,pub workspace_id:Id,pub tab_id:Id,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub read_identity:Option<PaneReadIdentity>,
}
#[derive(Clone,Debug,Serialize,PartialEq,Eq)]
pub struct ShellProof {
    pub identity:ShellIdentity,
    #[serde(skip_serializing_if="Option::is_none")]pub scope:Option<NativeShellScope>,
    #[serde(skip_serializing_if="Option::is_none")]pub artifact_scope:Option<super::artifacts::NativeArtifactScope>,
    #[serde(default,skip_serializing_if="std::ops::Not::not")]pub logical:bool,
    pub source_epoch:Uuid,pub title:Label,
}
impl<'de> Deserialize<'de> for ShellProof {
    fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored {
            #[serde(default)]identity:Option<ShellIdentity>,
            #[serde(default)]scope:Option<NativeShellScope>,
            #[serde(default)]artifact_scope:Option<super::artifacts::NativeArtifactScope>,
            #[serde(default)]logical:bool,source_epoch:Uuid,title:Label,
        }
        let stored=Stored::deserialize(deserializer)?;
        let identity=stored.identity.or_else(||stored.scope.as_ref().map(shell_identity)).ok_or_else(||serde::de::Error::custom("shell identity unavailable"))?;
        Ok(Self{identity,scope:stored.scope,artifact_scope:stored.artifact_scope,logical:stored.logical,source_epoch:stored.source_epoch,title:stored.title})
    }
}
fn shell_identity(scope:&NativeShellScope)->ShellIdentity {
    ShellIdentity{pane_id:scope.pane_id.clone(),terminal_id:scope.terminal_id.clone(),workspace_id:scope.workspace_id.clone(),tab_id:scope.tab_id.clone(),read_identity:Some(scope.read_identity.clone())}
}
impl ShellProof {
    pub(crate) fn same_authority(&self,other:&Self)->bool {
        self.identity==other.identity&&self.scope==other.scope&&self.artifact_scope==other.artifact_scope&&self.logical==other.logical&&self.source_epoch==other.source_epoch
    }
    pub(crate) fn validate_for(&self,key:&StableResourceKey)->io::Result<()> {
        if let Some(identity)=&self.identity.read_identity {identity.validate()?;}
        for id in [&self.identity.pane_id,&self.identity.terminal_id,&self.identity.workspace_id,&self.identity.tab_id]{id_valid(id)?;}
        self.source_epoch.validate().map_err(|_|invalid("invalid shell source"))?;self.title.validate().map_err(|_|invalid("invalid shell title"))?;
        if self.logical {
            if self.identity.read_identity.is_some()||self.scope.is_some()||self.artifact_scope.is_some(){return Err(invalid("mixed logical shell authority"));}
        } else {
            if self.scope.is_none()&&self.artifact_scope.is_none(){return Err(invalid("shell proof missing"));}
            if let Some(scope)=&self.scope {scope.validate()?;if shell_identity(scope)!=self.identity{return Err(invalid("shell scope mismatch"));}}
            if let Some(scope)=&self.artifact_scope {
                scope.validate()?;let original=scope.identity();
                if original.pane_id!=self.identity.pane_id||original.terminal_id!=self.identity.terminal_id||original.workspace_id!=self.identity.workspace_id||original.tab_id!=self.identity.tab_id||Some(&original.read_identity)!=self.identity.read_identity.as_ref()||!matches!(scope.source,super::artifacts::NativeArtifactKind::Shell){return Err(invalid("artifact shell scope mismatch"));}
            }
        }
        let StableResourceKey::Shell{terminal_id,..}=key else{return Err(invalid("not a shell mapping"));};
        if terminal_id!=&self.identity.terminal_id{return Err(invalid("shell mapping mismatch"));}Ok(())
    }
}
pub(super) struct ShellBinding {
    pub(super) key:StableResourceKey,pub(super) target:ShellTarget,pub(super) proof:ShellProof,pub(super) source:Option<Source>,pub(super) read:bool,create:bool,input:bool,
    pub(super) artifacts:bool,
}
#[derive(Clone,Copy,PartialEq,Eq)]
pub(crate) enum ShellAccess {Read,Input}
#[derive(Clone)]
pub(crate) struct LogicalShellContext {
    pub(super) target:ShellTarget,
    pub(super) proof:ShellProof,
    pub(super) source:Source,
}
pub(super) fn ordinary_shell(snapshot:&Snapshot,pane:&Pane)->bool {
    pane.agent.is_none()&&pane.display_agent.is_none()&&pane.agent_session.is_none()&&pane.restore_error.is_none()
        &&!snapshot.agents.iter().any(|agent|agent.terminal_id==pane.terminal_id||agent.pane_id==pane.pane_id)
}

fn shell_summary(binding:&ShellBinding,ready:bool)->ShellSummary {
    let StableResourceKey::Shell {server_id,..}=&binding.key else {unreachable!("typed shell binding")};
    let scope=&binding.proof.identity;
    ShellSummary {target:binding.target.clone(),server_id:server_id.clone(),workspace_id:scope.workspace_id.clone(),tab_id:scope.tab_id.clone(),pane_id:scope.pane_id.clone(),terminal_id:scope.terminal_id.clone(),title:binding.proof.title.clone(),connectivity:if ready {SessionConnectivity::Ready}else{SessionConnectivity::Offline},capabilities:ShellCapabilities {read:ready&&binding.read,create:ready&&binding.create,input:ready&&binding.input,close:false,artifacts:ready&&binding.artifacts,grid:ready && binding.read && binding.proof.logical && binding.source.as_ref().is_some_and(|source|source.snapshot.pane(&scope.pane_id).is_ok_and(|pane|source.snapshot.layout_seed(pane).is_ok()))}}
}
impl Topology {
    pub(super) fn restore_shells(&self,shells:Vec<PersistedShell>)->SecurityResult<()> {
        let mut inner=self.inner.lock().expect("topology mutex");
        for shell in shells {
            if !matches!(&shell.key,StableResourceKey::Shell {host_id,..} if host_id==&self.host_id) {return Err(ErrorCode::NotReady)}
            inner.shell_bindings.insert(shell.target.shell_id.clone(),ShellBinding {key:shell.key,target:shell.target,proof:shell.proof,source:None,read:false,create:false,input:false,artifacts:false});
        }Ok(())
    }
    pub(super) async fn update_shells(&self,source:&Source)->SecurityResult<()> {
        // Stored proofs retain display/history only. Current source and the immutable local grant
        // independently admit the new logical viewer; private native grants never enable input.
        if self.state.has_creation_shell_fences(&source.server_id) {
            let server=source.server_id.clone();let incarnation=source.incarnation.clone();let snapshot=source.snapshot.clone();let state=self.state.clone();
            // Only conclusive success plus observed agent/loss retires this admission metadata.
            // Uncertain roots and all native resources/receipts remain untouched.
            self.storage.call(move|store|{
                for(peer,action)in store.retire_confirmed_creation_shell_fences(&server,&incarnation,&snapshot)? {state.creation_shell_fence(peer,action,None);}
                Ok(())
            }).await.map_err(pairing::store_error)?;
        }
        let old:Vec<_>={let mut inner=self.inner.lock().expect("topology mutex");inner.shell_bindings.values_mut().filter_map(|binding| {
            if binding.key.server()==&source.server_id {let prior=binding.source.take();Some((binding.key.clone(),binding.target.clone(),binding.proof.clone(),prior))}else{None}
        }).collect()};
        for pane in &source.snapshot.panes {
            if !ordinary_shell(&source.snapshot,pane)||self.state.creation_shell_blocked(&source.server_id,&source.incarnation,&pane.pane_id,&pane.terminal_id){continue;}
            let identity=ShellIdentity{pane_id:pane.pane_id.clone(),terminal_id:pane.terminal_id.clone(),workspace_id:pane.workspace_id.clone(),tab_id:pane.tab_id.clone(),read_identity:None};
            let key=StableResourceKey::Shell {host_id:self.host_id.clone(),server_id:source.server_id.clone(),server_incarnation:source.incarnation.clone(),terminal_id:pane.terminal_id.clone()};
            let prior=old.iter().find(|(known,_,_,_)|*known==key);
            let epoch=prior.filter(|(_,_,proof,original)|proof.logical&&proof.identity==identity&&original.as_ref().is_some_and(|original|original.same_physical_identity(source)))
                .map(|(_,_,proof,_)|proof.source_epoch.clone()).or_else(||{
                    // A refresh may have captured `old` before a creation committed. The
                    // fence clears only AFTER that live binding is installed under inner;
                    // rejoin it here before storage can remint its receipt's generation.
                    self.inner.lock().expect("topology mutex").shell_bindings.values()
                        .find(|binding|binding.key==key&&binding.proof.logical&&binding.proof.identity==identity
                            &&binding.source.as_ref().is_some_and(|original|original.same_physical_identity(source)))
                        .map(|binding|binding.proof.source_epoch.clone())
                }).unwrap_or_else(Uuid::random);
            let title=pane.label.as_deref().or(pane.title.as_deref()).and_then(|text|Label::new(text).ok()).unwrap_or_else(||Label("Terminal".into()));
            let proof=ShellProof {identity,scope:None,artifact_scope:None,logical:true,source_epoch:epoch,title};
            let stored_key=key.clone();let stored_proof=proof.clone();
            let target=self.storage.call(move|store|store.shell_identity(&stored_key,&stored_proof,false)).await.map_err(pairing::store_error)?;
            source.recheck().map_err(policy)?;
            let granted=self.terminal_source_control(&source.server_id).is_ok();
            let artifacts=self.config.projects.iter().any(|project|project.allow_read_tools&&self.creation_roots.get(&project.id).is_some_and(Option::is_some));
            self.inner.lock().expect("topology mutex").shell_bindings.insert(target.shell_id.clone(),ShellBinding {key,target,proof,source:Some(source.clone()),read:granted,create:granted&&self.creation_roots.values().any(Option::is_some),input:granted,artifacts});
        }Ok(())
    }
    pub(super) fn shell_inventory(&self,inner:&Inner)->Vec<(ShellSummary,Uuid)> {
        let mut shells:Vec<_>=inner.shell_bindings.values().map(|binding| {
            let ready=binding.source.as_ref().is_some_and(|source|source.recheck().is_ok() && inner.servers.get(&source.server_id).and_then(|record|record.source.as_ref()).is_some_and(|current|current.revision==source.revision && Arc::ptr_eq(&current.invalidation,&source.invalidation)));
            (shell_summary(binding,ready),binding.proof.source_epoch.clone())
        }).collect();shells.sort_by(|(left,_),(right,_)|left.target.shell_id.0.cmp(&right.target.shell_id.0));shells
    }
    pub(crate) fn logical_shell_context(&self,target:&ShellTarget)->SecurityResult<LogicalShellContext> {
        self.terminal_control()?;
        let inner=self.inner.lock().expect("topology mutex");
        let binding=inner.shell_bindings.get(&target.shell_id).filter(|binding|binding.target==*target&&binding.proof.logical).ok_or(ErrorCode::StaleSession)?;
        let source=binding.source.clone().ok_or(ErrorCode::NotReady)?;
        self.terminal_source_control(&source.server_id)?;
        if !binding.read{return Err(ErrorCode::NotReady);}
        source.recheck().map_err(policy)?;
        let route=&binding.proof.identity;
        let pane=source.snapshot.pane(&route.pane_id).map_err(policy)?;
        if pane.terminal_id!=route.terminal_id||pane.workspace_id!=route.workspace_id||pane.tab_id!=route.tab_id||!ordinary_shell(&source.snapshot,pane){return Err(ErrorCode::StaleSession);}
        if self.state.creation_shell_blocked(&source.server_id,&source.incarnation,&route.pane_id,&route.terminal_id){return Err(ErrorCode::StaleSession);}
        self.state.validate_shell(target,&binding.proof.source_epoch,ShellAccess::Read)?;
        Ok(LogicalShellContext{target:target.clone(),proof:binding.proof.clone(),source})
    }
}

pub(super) fn shell_transition(resources:ShellCreationResources,state:ReceiptState,proof:crate::storage::TransitionProof)->crate::storage::ReceiptTransition {
    crate::storage::ReceiptTransition {state,proof,result:Some(ActionResult::ShellCreation(ActionResultShellCreation {resources})),error:None,at:pairing::now_ms()}
}

impl Topology {
    pub(crate) fn terminal_control(&self)->SecurityResult<()> {
        if !self.config.allow_terminal_control{return Err(ErrorCode::NotReady);}
        self.terminal_safety
    }
    pub(crate) fn terminal_source_control(&self,server:&Uuid)->SecurityResult<()> {
        self.terminal_control()?;
        if self.terminal_unproved_servers.contains(server){return Err(ErrorCode::NotReady);}
        Ok(())
    }
    pub(crate) fn terminal_creation_available(&self,source:&Source,workspace:&Id)->bool {
        self.terminal_source_control(&source.server_id).is_ok()&&self.creation_roots.values().any(Option::is_some)
            &&source.recheck().is_ok()&&source.snapshot.workspaces.iter().any(|row|row.workspace_id==*workspace)
    }
    pub(super) async fn begin_creation_shell_fence(&self,auth:&Authorized,action:&Uuid,source:&Source)->SecurityResult<()> {
        source.recheck().map_err(policy)?;
        let fence=CreationShellFence{server:source.server_id.clone(),incarnation:Some(source.incarnation.clone()),
            baseline:source.snapshot.panes.iter().map(|pane|(pane.pane_id.clone(),pane.terminal_id.clone())).collect(),created:None};
        fence.validate().map_err(|_|ErrorCode::ResourceLimit)?;
        let proof=auth.proof.clone();let id=action.clone();let state=self.state.clone();
        self.storage.call(move|store|{
            store.retain_creation_shell_fence(&proof,&id,fence.clone())?;
            state.creation_shell_fence(proof.peer_id().clone(),id,Some(fence));Ok(())
        }).await.map_err(pairing::store_error)
    }
    pub(super) async fn narrow_creation_shell_fence(&self,auth:&Authorized,action:&Uuid,pane:&Id,terminal:&Id)->SecurityResult<()> {
        let proof=auth.proof.clone();let id=action.clone();let state=self.state.clone();let pane=pane.clone();let terminal=terminal.clone();
        self.storage.call(move|store|{
            let fence=store.narrow_creation_shell_fence(&proof,&id,&pane,&terminal)?;
            state.creation_shell_fence(proof.peer_id().clone(),id,Some(fence));Ok(())
        }).await.map_err(pairing::store_error)
    }
    pub(super) async fn release_creation_shell_fence(&self,auth:&Authorized,action:&Uuid)->SecurityResult<()> {
        let proof=auth.proof.clone();let id=action.clone();let state=self.state.clone();
        self.storage.call(move|store|{
            store.release_creation_shell_fence(&proof,&id)?;
            state.creation_shell_fence(proof.peer_id().clone(),id,None);Ok(())
        }).await.map_err(pairing::store_error)
    }
}

impl Topology {
    /// The existing reservation callback registers exactly one bounded original job.
    /// A matching receipt wins before directory, source, lease or task-capacity admission.
    pub(crate) async fn terminal_create(self:&Arc<Self>,security:&Arc<Security>,auth:&Arc<Authorized>,request:Request,deadline:tokio::time::Instant)->SecurityResult<Receipt> {
        let (request,existing)=security.preflight_request(auth,request).await?;
        if let Some(receipt)=existing{return Ok(receipt);}
        let RequestBody::TerminalCreate(params)=&request.body else{return Err(ErrorCode::InvalidRequest)};
        let permit=if self.terminal_stopping.load(Ordering::Acquire){Err(ErrorCode::NotReady)}
            else{self.terminal_slots.clone().try_acquire_owned().map_err(|_|ErrorCode::Busy)};
        let selection=match &permit {
            Ok(_)=>self.capture_terminal_creation(auth,params).await,
            Err(code)=>Err(*code),
        };
        let checked=selection.clone();let validator=self.clone();
        let worker=self.clone();let authorization=auth.clone();let secured=security.clone();let original=params.clone();
        let handle=tokio::runtime::Handle::current();
        let reservation=security.reserve(auth,request,move|request|{
            grid::before(deadline)?;
            if validator.terminal_stopping.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
            let RequestBody::TerminalCreate(params)=&request.body else{return Err(ErrorCode::InvalidRequest)};
            checked.as_ref().map_err(|code|*code)?.validate_terminal(&validator,params).map(|_|())
        },move|committed|{
            let permit=permit.expect("creation reservation validated its task permit");
            let selection=selection.expect("creation reservation validated its held directory");
            let owner=worker.clone();
            let mut jobs=worker.terminal_jobs.lock().expect("terminal creation jobs");
            while jobs.try_join_next().is_some(){}
            jobs.spawn_on(async move {
                let _permit=permit;let action=committed.action_id;
                let outcome=tokio::time::timeout_at(deadline,owner.terminal_create_once(&secured,&authorization,&action,&original,selection,deadline)).await
                    .unwrap_or(Err(ErrorCode::Timeout));
                let proof=authorization.proof.clone();let id=action.clone();
                let receipt=secured.storage.call(move|store|{
                    let mut receipt=store.status(&proof,&id,pairing::now_ms())?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
                    if receipt.state==ReceiptState::Reserved {
                        let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=store.transition(&proof,&id,ReceiptTransition{state:ReceiptState::Failed,
                            proof:TransitionProof::DefinitiveNoEffect(Id("configured terminal factory was not entered".into())),
                            result:None,error:Some(WireError{code,message:SafeMessage("Terminal creation was refused before dispatch".into())}),at:pairing::now_ms()})?;
                    }Ok(receipt)
                }).await;
                if let Ok(receipt)=receipt {
                    if receipt.state==ReceiptState::Failed&&(receipt.result.as_ref().is_none_or(|result|matches!(result,ActionResult::ShellCreation(result) if !result.resources.complete()))) {
                        let _=owner.release_creation_shell_fence(&authorization,&action).await;
                    }
                    let _=secured.state.peer_event(&authorization.proof,EventBody::ActionStatus(ActionStatusEvent{receipt}));
                }
            },&handle);
        }).await?;
        match reservation {Reservation::Existing(receipt)|Reservation::Reserved(receipt)=>Ok(receipt)}
    }
}

impl Topology {
    async fn terminal_create_once(self:&Arc<Self>,security:&Security,auth:&Authorized,action:&Uuid,params:&TerminalCreateParams,selection:Arc<CreationSelection>,deadline:tokio::time::Instant)->SecurityResult<()> {
        grid::before(deadline)?;
        if self.terminal_stopping.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
        let source=selection.validate_terminal(self,params)?;
        let directory=selection.terminal_directory(params)?;
        let client=Client::connect(&source.socket_path).await.map_err(policy)?;
        if client.evidence!=source.evidence{return Err(ErrorCode::StaleSession);}
        let fd=client.fence_socket().map_err(policy)?;
        self.begin_creation_shell_fence(auth,action,&source).await?;
        let epoch=Uuid::random();
        let original_selection=selection.clone();let original_params=params.clone();let owner=self.clone();
        let peer=auth.proof.peer_id().clone();let original_action=action.clone();let original_epoch=epoch.clone();
        let acknowledge:pairing::TerminalCreationAckGuard=Arc::new(move|resources,commit|{
            if owner.terminal_stopping.load(Ordering::Acquire)||original_selection.unproved(){return Err(ErrorCode::NotReady);}
            let source=original_selection.validate_terminal_proved(&owner,&original_params)?;
            owner.commit_created_terminal(source,&peer,&original_action,resources,&original_epoch,commit)
        });
        let entered=security.terminal_creation_boundary(auth,action.clone(),source.incarnation.clone(),fd,acknowledge,
            ||{grid::before(deadline)?;if self.terminal_stopping.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
                selection.validate_terminal(self,params).map(|_|())},
            |checkpoint|{let source=&source;let selection=&selection;let epoch=&epoch;async move {
                let mut resources=ShellCreationResources::default();
                let response=client.request("tab.create",json!({"workspace_id":params.workspace_id,"cwd":directory,"focus":false}),"tab_created").await;
                let (value,normal)=match response {
                    Ok(value)=>(Some(value),true),Err(ApiError::Changed(value))=>(Some(value),false),
                    Err(ApiError::Rejected(code)) if matches!(code.as_str(),"invalid_request"|"invalid_params"|"invalid_env")=>{
                        return shell_transition(resources,ReceiptState::Failed,TransitionProof::DefinitiveNoEffect(Id("native tab factory rejected before effect".into())));
                    }
                    Err(_)=>(None,false),
                };
                let mut complete=false;
                if let Some(value)=value {
                    let (known,tab,consistent)=workspace_resources(&value,Some(&params.workspace_id));
                    resources=ShellCreationResources{server_id:if known.workspace_id.is_some()||known.pane_id.is_some()||known.terminal_id.is_some()||tab.is_some(){Some(source.server_id.clone())}else{None},
                        workspace_id:known.workspace_id,tab_id:tab,pane_id:known.pane_id,terminal_id:known.terminal_id,created_target:None};
                    complete=consistent&&resources.complete()
                        &&!source.snapshot.panes.iter().any(|pane|resources.pane_id.as_ref()==Some(&pane.pane_id)||resources.terminal_id.as_ref()==Some(&pane.terminal_id))
                        &&!source.snapshot.tabs.iter().any(|tab|resources.tab_id.as_ref()==Some(&tab.tab_id));
                }
                if checkpoint.retain(&source.incarnation,&resources).is_err()||checkpoint.checkpoint().await.is_err(){
                    return shell_transition(resources,ReceiptState::Uncertain,TransitionProof::LostOutcome);
                }
                if !normal||!complete||selection.retain_terminal_created(&resources).is_err(){
                    return shell_transition(resources,ReceiptState::Uncertain,TransitionProof::LostOutcome);
                }
                let confirmed:SecurityResult<()>=async {
                    self.narrow_creation_shell_fence(auth,action,resources.pane_id.as_ref().ok_or(ErrorCode::NotReady)?,resources.terminal_id.as_ref().ok_or(ErrorCode::NotReady)?).await?;
                    let current=self.confirm_created_terminal(&source,&selection,params,&resources,deadline).await?;
                    let pane=current.snapshot.pane(resources.pane_id.as_ref().ok_or(ErrorCode::NotReady)?).map_err(policy)?;
                    let (key,proof)=self.created_shell_identity(&current,pane,&epoch);
                    let target=self.storage.call(move|store|store.shell_identity(&key,&proof,false)).await.map_err(pairing::store_error)?;
                    // Cwd proof precedes even provisional identity enrollment; IDs alone never
                    // enter inventory. Final same-root/cwd proof also guards the receipt commit.
                    selection.validate_terminal_proved(self,params)?;
                    resources.created_target=Some(target);
                    checkpoint.retain(&source.incarnation,&resources)?;
                    checkpoint.checkpoint().await?;
                    Ok(())
                }.await;
                if confirmed.is_err(){return shell_transition(resources,ReceiptState::Uncertain,TransitionProof::LostOutcome);}
                shell_transition(resources,ReceiptState::Succeeded,TransitionProof::CorrelatedAcknowledgement(Id("original created shell kernel cwd matched selected directory".into())))
            }}).await;
        let receipt=self.receipt(security,auth,action).await?;
        if receipt.state==ReceiptState::Succeeded {
            // Publish only after any complete refresh finishes; metadata enrollment already
            // committed, so a normal dirty pause cannot strand its exact target.
            let _update=tokio::time::timeout_at(deadline,self.updates.lock()).await.map_err(|_|ErrorCode::Timeout)?;
            grid::before(deadline)?;self.publish()?;
        }
        else if matches!(receipt.state,ReceiptState::Reserved|ReceiptState::Failed)
            &&receipt.result.as_ref().is_none_or(|result|matches!(result,ActionResult::ShellCreation(result) if result.resources.workspace_id.is_none()&&result.resources.tab_id.is_none()&&result.resources.pane_id.is_none()&&result.resources.terminal_id.is_none()))
        {self.release_creation_shell_fence(auth,action).await?;}
        entered
    }
    fn created_shell_identity(&self,source:&Source,pane:&Pane,epoch:&Uuid)->(StableResourceKey,ShellProof) {
        let key=StableResourceKey::Shell{host_id:self.host_id.clone(),server_id:source.server_id.clone(),server_incarnation:source.incarnation.clone(),terminal_id:pane.terminal_id.clone()};
        let identity=ShellIdentity{pane_id:pane.pane_id.clone(),terminal_id:pane.terminal_id.clone(),workspace_id:pane.workspace_id.clone(),tab_id:pane.tab_id.clone(),read_identity:None};
        let title=pane.label.as_deref().or(pane.title.as_deref()).and_then(|text|Label::new(text).ok()).unwrap_or_else(||Label("Terminal".into()));
        (key,ShellProof{identity,scope:None,artifact_scope:None,logical:true,source_epoch:epoch.clone(),title})
    }
    async fn confirm_created_terminal(&self,source:&Source,selection:&CreationSelection,params:&TerminalCreateParams,resources:&ShellCreationResources,deadline:tokio::time::Instant)->SecurityResult<Source> {
        let pane_id=resources.pane_id.as_ref().ok_or(ErrorCode::NotReady)?;
        let reporter=process_evidence(source.evidence.pid).map_err(policy)?;
        let client=Client::connect(&source.socket_path).await.map_err(policy)?;
        if client.evidence!=source.evidence{return Err(ErrorCode::StaleSession);}
        let (_,snapshot)=client.snapshot().await.map_err(policy)?;
        let pane=snapshot.pane(pane_id).map_err(policy)?;
        if !selection.matches_created(pane)||!ordinary_shell(&snapshot,pane)
            ||snapshot.focused_workspace_id!=source.snapshot.focused_workspace_id{return Err(ErrorCode::StaleSession);}
        loop {
            grid::before(deadline)?;source.recheck_identity().map_err(policy)?;selection.terminal_directory(params)?;
            if self.terminal_stopping.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
            let client=Client::connect(&source.socket_path).await.map_err(policy)?;
            if client.evidence!=source.evidence{return Err(ErrorCode::StaleSession);}
            let info=client.process_info(pane_id).await.map_err(policy)?;
            if let Some(pid)=info.shell_pid {
                let shell=reported_process(&reporter,pid).map_err(policy)?;
                selection.retain_shell(shell)?;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        loop {
            grid::before(deadline)?;source.recheck_identity().map_err(policy)?;selection.terminal_directory(params)?;
            if self.terminal_stopping.load(Ordering::Acquire){return Err(ErrorCode::NotReady);}
            let current={let inner=self.inner.lock().expect("topology mutex");
                inner.servers.get(&source.server_id).and_then(|server|server.source.as_ref()).cloned().ok_or(ErrorCode::StaleSession)?};
            if !source.same_physical_identity(&current){return Err(ErrorCode::StaleSession);}
            if current.recheck_readiness().map_err(policy)? {
                if let Ok(pane)=current.snapshot.pane(pane_id) {
                    if !selection.matches_created(pane)||!ordinary_shell(&current.snapshot,pane){return Err(ErrorCode::StaleSession);}
                    return selection.validate_terminal_proved(self,params);
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    pub(super) fn commit_created_terminal(&self,source:Source,peer:&Uuid,action:&Uuid,resources:&ShellCreationResources,epoch:&Uuid,commit:&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()> {
        let target=resources.created_target.as_ref().ok_or(ErrorCode::NotReady)?;
        let pane=source.snapshot.pane(resources.pane_id.as_ref().ok_or(ErrorCode::NotReady)?).map_err(policy)?;
        let (key,proof)=self.created_shell_identity(&source,pane,epoch);
        let id=target.shell_id.clone();
        let binding=ShellBinding{key,target:target.clone(),proof,source:Some(source.clone()),
            read:true,create:self.creation_roots.values().any(Option::is_some),input:true,artifacts:false};
        // SQLite already owns the transaction. This short existing-owner lock spans its
        // successful commit and metadata installation; no await/native request is permitted.
        let mut inner=self.inner.lock().expect("topology mutex");
        let current=inner.servers.get(&source.server_id).and_then(|server|server.source.as_ref()).ok_or(ErrorCode::StaleSession)?;
        if !source.same_physical_identity(current)||source.revision!=current.revision||!source.revision_current(){return Err(ErrorCode::NotReady);}
        let route=&binding.proof.identity;let current_pane=current.snapshot.pane(&route.pane_id).map_err(policy)?;
        if current_pane.terminal_id!=route.terminal_id||current_pane.workspace_id!=route.workspace_id||current_pane.tab_id!=route.tab_id
            ||!ordinary_shell(&current.snapshot,current_pane){return Err(ErrorCode::StaleSession);}
        inner.shell_bindings.try_reserve(1).map_err(|_|ErrorCode::ResourceLimit)?;
        commit()?;
        inner.shell_bindings.insert(id,binding);
        self.state.creation_shell_fence(peer.clone(),action.clone(),None);
        Ok(())
    }
}
