// SPDX-License-Identifier: GPL-3.0-or-later
//! Positive native ordinary-shell authority. No provider, cwd or pane fallback.
use super::*;

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
#[derive(Clone,Debug,Serialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellProof {pub identity:WorktreeReadScope,#[serde(skip_serializing_if="Option::is_none")]pub scope:Option<NativeShellScope>,#[serde(skip_serializing_if="Option::is_none")]pub artifact_scope:Option<super::artifacts::NativeArtifactScope>,pub source_epoch:Uuid,pub title:Label}
impl<'de> Deserialize<'de> for ShellProof {
 fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
  #[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Stored{#[serde(default)]identity:Option<WorktreeReadScope>,#[serde(default)]scope:Option<NativeShellScope>,#[serde(default)]artifact_scope:Option<super::artifacts::NativeArtifactScope>,source_epoch:Uuid,title:Label}
  let stored=Stored::deserialize(deserializer)?;let identity=stored.identity.or_else(||stored.scope.as_ref().map(shell_identity)).ok_or_else(||serde::de::Error::custom("shell identity unavailable"))?;
  Ok(Self{identity,scope:stored.scope,artifact_scope:stored.artifact_scope,source_epoch:stored.source_epoch,title:stored.title})
 }
}
fn shell_identity(scope:&NativeShellScope)->WorktreeReadScope{WorktreeReadScope{pane_id:scope.pane_id.clone(),terminal_id:scope.terminal_id.clone(),workspace_id:scope.workspace_id.clone(),tab_id:scope.tab_id.clone(),read_identity:scope.read_identity.clone()}}
impl ShellProof {
 pub(crate) fn same_authority(&self,other:&Self)->bool{self.identity==other.identity&&self.scope==other.scope&&self.artifact_scope==other.artifact_scope&&self.source_epoch==other.source_epoch}
 pub(crate) fn validate_for(&self,key:&StableResourceKey)->io::Result<()>{
  self.identity.read_identity.validate()?;for id in [&self.identity.pane_id,&self.identity.terminal_id,&self.identity.workspace_id,&self.identity.tab_id]{id_valid(id)?}
  self.source_epoch.validate().map_err(|_|invalid("invalid shell source"))?;self.title.validate().map_err(|_|invalid("invalid shell title"))?;
  if self.scope.is_none()&&self.artifact_scope.is_none(){return Err(invalid("shell proof missing"))}
  if let Some(scope)=&self.scope{scope.validate()?;if shell_identity(scope)!=self.identity{return Err(invalid("shell scope mismatch"))}}
  if let Some(scope)=&self.artifact_scope{scope.validate()?;if scope.identity()!=self.identity||!matches!(scope.source,super::artifacts::NativeArtifactKind::Shell){return Err(invalid("artifact shell scope mismatch"))}}
  let StableResourceKey::Shell{terminal_id,..}=key else{return Err(invalid("not a shell mapping"))};if terminal_id!=&self.identity.terminal_id{return Err(invalid("shell mapping mismatch"))}Ok(())
 }
}
pub(super) struct ShellBinding {
    pub(super) key:StableResourceKey,pub(super) target:ShellTarget,pub(super) proof:ShellProof,pub(super) source:Option<Source>,read:bool,create:bool,input:bool,artifacts:bool,
}
#[derive(Clone,Copy,PartialEq,Eq)]
pub(crate) enum ShellAccess {Read,Create,Input}
#[derive(Clone)]
pub(crate) struct ShellReadContext {target:ShellTarget,proof:ShellProof,scope:NativeShellScope,source:Source}

fn positive_shell<'a>(snapshot:&Snapshot,pane:&'a Pane)->Option<&'a NativeShellInfo> {
    let shell=pane.shell.as_ref()?;
    if !snapshot.shell_read_bound || !shell.read || shell.close || shell.create && !snapshot.shell_create_bound
        || pane.agent.is_some() || pane.agent_session.is_some() || snapshot.agents.iter().any(|agent|agent.terminal_id==pane.terminal_id)
        || shell.scope.pane_id!=pane.pane_id || shell.scope.terminal_id!=pane.terminal_id || shell.scope.workspace_id!=pane.workspace_id || shell.scope.tab_id!=pane.tab_id
        || pane.read_identity.as_ref()!=Some(&shell.scope.read_identity) || shell.scope.validate().is_err() {return None}
    Some(shell)
}
fn shell_summary(binding:&ShellBinding,ready:bool)->ShellSummary {
    let StableResourceKey::Shell {server_id,..}=&binding.key else {unreachable!("typed shell binding")};
    let scope=&binding.proof.identity;
    ShellSummary {target:binding.target.clone(),server_id:server_id.clone(),workspace_id:scope.workspace_id.clone(),tab_id:scope.tab_id.clone(),pane_id:scope.pane_id.clone(),terminal_id:scope.terminal_id.clone(),title:binding.proof.title.clone(),connectivity:if ready {SessionConnectivity::Ready}else{SessionConnectivity::Offline},capabilities:ShellCapabilities {read:ready && binding.read,create:ready && binding.create,input:ready && binding.input,close:false,artifacts:ready && binding.artifacts}}
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
        // Called under the existing topology update owner. All persistence remains
        // on StorageOwner; a cached proof never becomes source readiness.
        let old:Vec<_>={let mut inner=self.inner.lock().expect("topology mutex");inner.shell_bindings.values_mut().filter_map(|binding| {
            if binding.key.server()==&source.server_id {let prior=binding.source.take();Some((binding.key.clone(),binding.target.clone(),binding.proof.clone(),prior))}else{None}
        }).collect()};
        for pane in &source.snapshot.panes {
            let native=positive_shell(&source.snapshot,pane);let artifact=super::artifacts::positive_artifact_shell(&source.snapshot,pane);
            if native.is_none()&&artifact.is_none(){continue}let identity=if let Some(shell)=native{shell_identity(&shell.scope)}else{artifact.expect("positive artifact proof").identity()};
            let key=StableResourceKey::Shell {host_id:self.host_id.clone(),server_id:source.server_id.clone(),server_incarnation:source.incarnation.clone(),terminal_id:pane.terminal_id.clone()};
            let prior=old.iter().find(|(known,_,_,_)|*known==key);
            let epoch=prior.filter(|(_,_,proof,original)|proof.identity==identity && proof.scope.as_ref()==native.map(|n|&n.scope) && proof.artifact_scope.as_ref()==artifact && original.as_ref().is_some_and(|original|original.evidence==source.evidence && original.incarnation==source.incarnation && Arc::ptr_eq(&original.invalidation,&source.invalidation))).map(|(_,_,proof,_)|proof.source_epoch.clone()).unwrap_or_else(Uuid::random);
            let title=pane.label.as_deref().or(pane.title.as_deref()).and_then(|text|Label::new(text).ok()).unwrap_or_else(||Label("Terminal".into()));
            let proof=ShellProof {identity,scope:native.map(|n|n.scope.clone()),artifact_scope:artifact.cloned(),source_epoch:epoch,title};
            let stored_key=key.clone();let stored_proof=proof.clone();
            let target=self.storage.call(move|store|store.shell_identity(&stored_key,&stored_proof,false)).await.map_err(pairing::store_error)?;
            source.recheck().map_err(policy)?;
            self.inner.lock().expect("topology mutex").shell_bindings.insert(target.shell_id.clone(),ShellBinding {key,target,proof,source:Some(source.clone()),read:native.is_some(),create:native.is_some_and(|n|n.create),input:native.is_some_and(|n|n.input),artifacts:artifact.is_some()});
        }Ok(())
    }
    pub(super) fn shell_inventory(&self,inner:&Inner)->Vec<(ShellSummary,Uuid)> {
        let mut shells:Vec<_>=inner.shell_bindings.values().map(|binding| {
            let ready=binding.source.as_ref().is_some_and(|source|source.recheck().is_ok() && inner.servers.get(&source.server_id).and_then(|record|record.source.as_ref()).is_some_and(|current|current.revision==source.revision && Arc::ptr_eq(&current.invalidation,&source.invalidation)));
            (shell_summary(binding,ready),binding.proof.source_epoch.clone())
        }).collect();shells.sort_by(|(left,_),(right,_)|left.target.shell_id.0.cmp(&right.target.shell_id.0));shells
    }
    pub(crate) fn shell_context(&self,target:&ShellTarget,access:ShellAccess)->SecurityResult<ShellReadContext> {
        let inner=self.inner.lock().expect("topology mutex");
        let binding=inner.shell_bindings.get(&target.shell_id).filter(|binding|binding.target==*target).ok_or(ErrorCode::StaleSession)?;
        let source=binding.source.clone().ok_or(ErrorCode::NotReady)?;
        source.recheck().map_err(policy)?;
        let scope=binding.proof.scope.as_ref().ok_or(ErrorCode::NotReady)?;let native=source.snapshot.pane(&scope.pane_id).ok().and_then(|pane|positive_shell(&source.snapshot,pane)).filter(|shell|shell.scope==*scope).ok_or(ErrorCode::NotReady)?;
        if access==ShellAccess::Create && (!binding.create || !native.create) || access==ShellAccess::Input && (!binding.input || !native.input) {return Err(ErrorCode::NotReady)}
        self.state.validate_shell(target,&binding.proof.source_epoch,access)?;
        Ok(ShellReadContext {target:target.clone(),proof:binding.proof.clone(),scope:scope.clone(),source})
    }
    pub(crate) fn validate_shell_context(&self,context:&ShellReadContext,access:ShellAccess)->SecurityResult<()> {
        context.source.recheck().map_err(policy)?;
        let inner=self.inner.lock().expect("topology mutex");
        let original=inner.shell_bindings.get(&context.target.shell_id).filter(|binding|binding.target==context.target && binding.proof.same_authority(&context.proof)).ok_or(ErrorCode::StaleSession)?;
        let source=original.source.as_ref().ok_or(ErrorCode::NotReady)?;
        if source.evidence!=context.source.evidence || source.incarnation!=context.source.incarnation || source.socket_path!=context.source.socket_path || source.revision!=context.source.revision || !Arc::ptr_eq(&source.invalidation,&context.source.invalidation) || access==ShellAccess::Create && !original.create || access==ShellAccess::Input && !original.input {return Err(ErrorCode::StaleSession)}
        source.recheck().map_err(policy)?;
        self.state.validate_shell(&context.target,&context.proof.source_epoch,access)
    }
    pub(crate) async fn shell_read(&self,target:&ShellTarget)->SecurityResult<(ShellReadContext,ShellReadResult)> {
        let context={let _update=self.updates.lock().await;self.shell_context(target,ShellAccess::Read)?};
        let client=Client::connect(&context.source.socket_path).await.map_err(policy)?;
        if client.evidence!=context.source.evidence {return Err(ErrorCode::StaleSession)}
        self.validate_shell_context(&context,ShellAccess::Read)?;
        let mut response=client.request("shell.read_bound",json!({"scope":context.scope}),"shell_read_bound").await.map_err(api_io).map_err(policy)?;
        let read=response.get_mut("read").ok_or(ErrorCode::NotReady)?.take();
        #[derive(Deserialize)]struct Read {scope:NativeShellScope,text:String,truncated:bool}
        let read:Read=serde_json::from_value(read).map_err(|_|ErrorCode::NotReady)?;
        if read.scope!=context.scope || read.text.len()>16*1024 {return Err(ErrorCode::StaleSession)}
        let result=ShellReadResult {target:target.clone(),text:read.text,truncated:read.truncated};result.validate().map_err(|_|ErrorCode::NotReady)?;
        Ok((context,result))
    }
    pub(crate) async fn enqueue_shell_read(&self,connection:&Uuid,response:Uuid,context:&ShellReadContext,result:ShellReadResult)->SecurityResult<()> {
        let _update=self.updates.lock().await;self.validate_shell_context(context,ShellAccess::Read)?;
        if result.target!=context.target {return Err(ErrorCode::StaleSession)}
        self.state.shell_read_response(connection,&context.target,&context.proof.source_epoch,response,result,||context.source.revision_current())
    }
}

pub(super) struct NativeShellCompletion {pub(super) resources:ShellCreationResources,pub(super) created_scope:Option<NativeShellScope>,pub(super) created:bool,pub(super) refused:bool,pub(super) consistent:bool}
fn native_shell_completion(value:&Value,context:&ShellReadContext,action:&Uuid)->Option<NativeShellCompletion> {
    let body=value.get("result")?.as_object()?;
    if body.get("operation_id")?.as_str()?!=action.0 {return None}
    let echoed:NativeShellScope=serde_json::from_value(body.get("scope")?.clone()).ok()?;
    if echoed!=context.scope {return None}
    let field=|value:Option<&Value>|value.and_then(Value::as_str).and_then(|value|Id::new(value).ok());
    let tab=body.get("tab").and_then(Value::as_object);
    let pane=body.get("root_pane").and_then(Value::as_object);
    let mut resources=ShellCreationResources {server_id:None,workspace_id:field(body.get("workspace_id")),tab_id:field(tab.and_then(|tab|tab.get("tab_id"))),pane_id:field(pane.and_then(|pane|pane.get("pane_id"))),terminal_id:field(pane.and_then(|pane|pane.get("terminal_id"))),created_target:None};
    let any_fact=resources.workspace_id.is_some() || resources.tab_id.is_some() || resources.pane_id.is_some() || resources.terminal_id.is_some();
    if any_fact {resources.server_id=Some(context.source.server_id.clone())}
    let created_scope=body.get("created_scope").and_then(|scope|serde_json::from_value::<NativeShellScope>(scope.clone()).ok()).filter(|scope|scope.validate().is_ok());
    let reasons=["created","invalid_operation","operation_already_consumed","operation_capacity","operation_in_progress","source_unavailable_before_effect","directory_unavailable_before_effect","identity_unavailable_before_effect","source_changed_before_effect","source_changed_after_effect","deadline_before_effect","deadline_after_effect","worker_unavailable_before_effect","factory_uncertain","factory_scope_uncertain","final_scope_unavailable","preparation_pending","factory_created"];
    let mut consistent=body.get("reason").and_then(Value::as_str).is_some_and(|reason|reasons.contains(&reason))
        && body.get("workspace_id").is_none_or(|value|field(Some(value)).is_some())
        && body.get("tab").is_none_or(Value::is_object) && body.get("root_pane").is_none_or(Value::is_object)
        && body.get("created_scope").is_none_or(|_|created_scope.is_some())
        && resources.workspace_id.as_ref().is_none_or(|workspace|workspace==&context.scope.workspace_id);
    if let Some(tab)=tab {consistent&=field(tab.get("tab_id")).is_some();}
    if let Some(pane)=pane {consistent&=field(pane.get("pane_id")).is_some() && field(pane.get("terminal_id")).is_some();}
    if let Some(tab)=tab {consistent&=field(tab.get("workspace_id")).as_ref()==resources.workspace_id.as_ref();}
    if let Some(pane)=pane {
        consistent&=field(pane.get("workspace_id")).as_ref()==resources.workspace_id.as_ref() && field(pane.get("tab_id")).as_ref()==resources.tab_id.as_ref();
    }
    if let Some(scope)=&created_scope {
        let read=pane.and_then(|pane|pane.get("read_identity")).and_then(|value|serde_json::from_value::<PaneReadIdentity>(value.clone()).ok());
        let shell=pane.and_then(|pane|pane.get("shell")).and_then(|value|serde_json::from_value::<NativeShellInfo>(value.clone()).ok());
        consistent&=resources.workspace_id.as_ref()==Some(&scope.workspace_id) && resources.tab_id.as_ref()==Some(&scope.tab_id) && resources.pane_id.as_ref()==Some(&scope.pane_id) && resources.terminal_id.as_ref()==Some(&scope.terminal_id)
            && scope.workspace_id==context.scope.workspace_id && scope.pane_id!=context.scope.pane_id && scope.terminal_id!=context.scope.terminal_id && scope.tab_id!=context.scope.tab_id
            && scope.permission_epoch==context.scope.permission_epoch && scope.read_identity.herdr_incarnation==context.scope.read_identity.herdr_incarnation && scope.read_identity.runtime_epoch!=context.scope.read_identity.runtime_epoch
            && read.as_ref()==Some(&scope.read_identity) && shell.is_some_and(|shell|shell.scope==*scope && shell.read && !shell.close);
    }
    let outcome=body.get("outcome").and_then(Value::as_str);
    Some(NativeShellCompletion {resources,created_scope,created:outcome==Some("created"),refused:outcome==Some("refused_before_effect"),consistent})
}
pub(super) fn shell_transition(resources:ShellCreationResources,state:ReceiptState,proof:crate::storage::TransitionProof)->crate::storage::ReceiptTransition {
    crate::storage::ReceiptTransition {state,proof,result:Some(ActionResult::ShellCreation(ActionResultShellCreation {resources})),error:None,at:pairing::now_ms()}
}
impl Topology {
    /// Receipt proof only: native's actor has already admitted and performed this
    /// exact action. Its own tab event may advance inventory revision/readiness.
    fn validate_shell_ack(&self,context:&ShellReadContext,access:ShellAccess)->SecurityResult<()> {
        context.source.recheck_identity().map_err(policy)?;
        let inner=self.inner.lock().expect("topology mutex");
        let binding=inner.shell_bindings.get(&context.target.shell_id).filter(|binding|binding.target==context.target && binding.proof.same_authority(&context.proof)).ok_or(ErrorCode::StaleSession)?;
        let current=inner.servers.get(&context.source.server_id).and_then(|record|record.source.as_ref()).ok_or(ErrorCode::NotReady)?;
        if current.server_id!=context.source.server_id || current.incarnation!=context.source.incarnation || current.evidence!=context.source.evidence || current.socket_path!=context.source.socket_path || !Arc::ptr_eq(&current.invalidation,&context.source.invalidation) {return Err(ErrorCode::StaleSession)}
        // A newer available native snapshot can disprove the original authority;
        // never require the old global revision or temporary binding.source Option.
        let original=current.snapshot.pane(&context.scope.pane_id).ok().and_then(|pane|positive_shell(&current.snapshot,pane));
        if original.is_none_or(|shell|shell.scope!=context.scope || access==ShellAccess::Create && !shell.create || access==ShellAccess::Input && !shell.input) {return Err(ErrorCode::StaleSession)}
        self.state.validate_shell_identity(&binding.target,&binding.proof.source_epoch)
    }
    fn existing_created_shell(&self,context:&ShellReadContext,scope:&NativeShellScope)->Option<ShellTarget> {self.existing_created_shell_scope(&context.source,scope)}
    pub(super) fn existing_created_shell_scope(&self,source:&Source,scope:&NativeShellScope)->Option<ShellTarget> {
        let inner=self.inner.lock().ok()?;
        inner.shell_bindings.values().find(|binding| {
            binding.proof.scope.as_ref()==Some(scope) && binding.key.server()==&source.server_id && binding.source.as_ref().is_some_and(|current|current.incarnation==source.incarnation && current.evidence==source.evidence && current.recheck().is_ok() && current.snapshot.pane(&scope.pane_id).ok().and_then(|pane|positive_shell(&current.snapshot,pane)).is_some_and(|shell|shell.scope==*scope))
        }).map(|binding|binding.target.clone())
    }
    pub(crate) async fn shell_create_once(&self,security:&Security,auth:&Authorized,action:&Uuid,target:&ShellTarget)->SecurityResult<()> {
        let context={let _update=self.updates.lock().await;self.shell_context(target,ShellAccess::Create)?};
        let client=Client::connect(&context.source.socket_path).await.map_err(policy)?;
        if client.evidence!=context.source.evidence {return Err(ErrorCode::StaleSession)}
        self.validate_shell_context(&context,ShellAccess::Create)?;
        let fd=client.fence_socket().map_err(policy)?;
        let original=context.clone();let effect_action=action.clone();
        security.shell_creation_boundary(auth,action.clone(),target.clone(),context.proof.source_epoch.clone(),fd,||self.validate_shell_context(&context,ShellAccess::Create),|checkpoint|async move {
            let response=client.request("shell.create_bound",json!({"operation_id":effect_action,"scope":original.scope}),"shell_create_bound").await;
            let (value,normal)=match response {Ok(value)=>(Some(value),true),Err(ApiError::Changed(value))=>(Some(value),false),Err(_)=>(None,false)};
            let Some(completion)=value.as_ref().and_then(|value|native_shell_completion(value,&original,&effect_action)) else {return shell_transition(ShellCreationResources::default(),ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)};
            let mut resources=completion.resources;
            // Correlated validated native IDs survive every await and Changed body.
            if checkpoint.retain(&original.target,&original.proof.source_epoch,&resources).is_err() {return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
            if checkpoint.checkpoint().await.is_err() {return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
            if normal && completion.created && completion.consistent && resources.complete() && completion.created_scope.is_some() && self.validate_shell_ack(&original,ShellAccess::Create).is_ok() {
                if let Some(scope)=&completion.created_scope {resources.created_target=self.existing_created_shell(&original,scope);}
                if resources.created_target.is_some() {
                    if checkpoint.retain(&original.target,&original.proof.source_epoch,&resources).is_err() || checkpoint.checkpoint().await.is_err() {return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
                    if self.validate_shell_ack(&original,ShellAccess::Create).is_err() {return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
                }
                return shell_transition(resources,ReceiptState::Succeeded,crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("native shell creation acknowledged".into())));
            }
            if normal && completion.refused && completion.consistent && resources.workspace_id.is_none() && resources.tab_id.is_none() && resources.pane_id.is_none() && resources.terminal_id.is_none() && completion.created_scope.is_none() {
                return shell_transition(resources,ReceiptState::Failed,crate::storage::TransitionProof::DefinitiveNoEffect(Id("native shell creation refused before effect".into())));
            }
            shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)
        }).await
    }
}

struct NativeShellInputCompletion {progress:ShellInputProgress,accepted:bool,refused:bool,consistent:bool}
fn native_shell_input_completion(value:&Value,context:&ShellReadContext,action:&Uuid)->Option<NativeShellInputCompletion> {
 let body=value.get("result")?.as_object()?;
 if body.get("operation_id")?.as_str()?!=action.0 {return None}
 let echoed:NativeShellScope=serde_json::from_value(body.get("scope")?.clone()).ok()?;
 if echoed!=context.scope {return None}
 let encoded=body.get("encoded_bytes").and_then(Value::as_u64);
 let written=body.get("written_bytes").and_then(Value::as_u64);
 let flushed=body.get("flush_completed").and_then(Value::as_bool);
 let total=encoded.filter(|n|*n>0 && *n<=16396).map(|n|n as u32);
 let written_bytes=written.filter(|n|total.is_some_and(|total|*n<=u64::from(total))).map(|n|n as u32);
 // Keep each independently proven bounded fact even when another result field
 // is malformed. Zero total never establishes the earlier operation's total.
 let mut progress=ShellInputProgress {encoded_bytes:total,written_bytes,flush_completed:None};
 progress.flush_completed=match flushed {Some(false) if total.is_some()=>Some(false),Some(true) if total.is_some() && written_bytes==total=>Some(true),_=>None};
 let reasons=["accepted","invalid_operation","operation_already_consumed","operation_capacity","invalid_input","source_unavailable_before_effect","source_changed","deadline","queue_unavailable","actor_unavailable","write_failed","flush_unproved","cancelled"];
 let reason=body.get("reason").and_then(Value::as_str).filter(|reason|reasons.contains(reason));
 let outcome=body.get("outcome").and_then(Value::as_str);
 let counts_valid=encoded.is_some_and(|n|n<=16396) && written.is_some_and(|n|encoded.is_some_and(|total|n<=total)) && flushed.is_some() && (flushed!=Some(true) || progress.accepted());
 Some(NativeShellInputCompletion {progress,consistent:counts_valid && reason.is_some() && matches!(outcome,Some("accepted"|"refused_before_effect"|"uncertain")),accepted:outcome==Some("accepted") && reason==Some("accepted"),refused:outcome==Some("refused_before_effect") && reason.is_some_and(|reason|reason!="accepted" && reason!="operation_already_consumed") && written==Some(0) && flushed==Some(false)})
}
fn shell_input_transition(progress:ShellInputProgress,state:ReceiptState,proof:crate::storage::TransitionProof)->crate::storage::ReceiptTransition {
 crate::storage::ReceiptTransition {state,proof,result:Some(ActionResult::ShellInput(ActionResultShellInput {progress})),error:None,at:pairing::now_ms()}
}
impl Topology {
 pub(crate) async fn shell_input_once(&self,security:&Security,auth:&Authorized,action:&Uuid,target:&ShellTarget,input:&ShellInput)->SecurityResult<()> {
  input.validate().map_err(|_|ErrorCode::InvalidRequest)?;
  let context={let _update=self.updates.lock().await;self.shell_context(target,ShellAccess::Input)?};
  let client=Client::connect(&context.source.socket_path).await.map_err(policy)?;
  if client.evidence!=context.source.evidence {return Err(ErrorCode::StaleSession)}
  self.validate_shell_context(&context,ShellAccess::Input)?;
  let fd=client.fence_socket().map_err(policy)?;let original=context.clone();let effect_action=action.clone();
  security.shell_input_boundary(auth,action.clone(),target.clone(),context.proof.source_epoch.clone(),fd,||self.validate_shell_context(&context,ShellAccess::Input),|checkpoint|async move {
   let response=client.request("shell.input_bound",json!({"operation_id":effect_action,"scope":original.scope,"input":input}),"shell_input_bound").await;
   let (value,normal)=match response {Ok(value)=>(Some(value),true),Err(ApiError::Changed(value))=>(Some(value),false),Err(_)=>(None,false)};
   let completion=value.as_ref().and_then(|value|native_shell_input_completion(value,&original,&effect_action));
   let Some(result)=completion else {return shell_input_transition(ShellInputProgress::default(),ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)};
   let progress=result.progress;
   if checkpoint.retain(&original.target,&original.proof.source_epoch,&progress).is_err() || checkpoint.checkpoint().await.is_err() {return shell_input_transition(progress,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
   if normal && result.consistent && result.accepted && progress.accepted() && self.validate_shell_ack(&original,ShellAccess::Input).is_ok() {
    return shell_input_transition(progress,ReceiptState::Succeeded,crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("native shell input accepted".into())));
   }
   if normal && result.consistent && result.refused {
    return shell_input_transition(progress,ReceiptState::Failed,crate::storage::TransitionProof::DefinitiveNoEffect(Id("native shell input refused before effect".into())));
   }
   shell_input_transition(progress,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)
  }).await
 }
}
