// SPDX-License-Identifier: GPL-3.0-or-later
//! Provider-process directory creation, distinct from ordinary-shell authority.
use super::*;
use super::shells::{NativeShellScope,NativeShellCompletion,shell_transition};
#[derive(Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct NativeAgentShellScope {pane_id:Id,terminal_id:Id,workspace_id:Id,tab_id:Id,read_identity:PaneReadIdentity,permission_epoch:u64,provider:Provider,provider_process:ProcessEvidence}
#[derive(Clone)]
pub(crate) struct AgentTerminalContext {pub target:Target,pub epoch:Uuid,source:Source,scope:NativeAgentShellScope,guard:Arc<dyn Fn(bool,&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()>+Send+Sync>}
pub(crate) fn agent_terminal_available(source:&Source,agent:&Agent,process:&ProcessEvidence)->bool {
    source.snapshot.agent_shell_create_bound&&source.snapshot.agent_shell_permission_epoch.is_some_and(|epoch|epoch>0)&&source.snapshot.pane(&agent.pane_id).ok().is_some_and(|pane|pane.terminal_id==agent.terminal_id&&pane.read_identity.is_some())&&process_evidence(process.pid).is_ok_and(|current|current==*process)
}
impl AgentTerminalContext {
    pub(crate) fn validate(&self,topology:&Topology,ack:bool)->SecurityResult<()> {(self.guard)(ack,&mut ||topology.validate_agent_terminal_context(self,ack))}
}
impl Topology {
    pub(crate) fn agent_terminal_context(&self,target:&Target,process:ProcessEvidence,provider:Provider,guard:Arc<dyn Fn(bool,&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()>+Send+Sync>)->SecurityResult<AgentTerminalContext> {
        let (server,epoch,pin)={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&target.session_id).filter(|binding|binding.target==*target).ok_or(ErrorCode::StaleSession)?;(binding.key.server_id.clone(),binding.source_epoch.clone(),binding.terminal_read_pin.clone().ok_or(ErrorCode::NotReady)?)};
        let source=self.source(&server)?;let state=self.state.session_state(target,&epoch)?;let pane=source.snapshot.pane(&state.summary.pane_id).map_err(policy)?;
        let permission_epoch=source.snapshot.agent_shell_permission_epoch.filter(|epoch|*epoch>0).ok_or(ErrorCode::NotReady)?;
        let scope=NativeAgentShellScope {pane_id:pane.pane_id.clone(),terminal_id:pane.terminal_id.clone(),workspace_id:pane.workspace_id.clone(),tab_id:pane.tab_id.clone(),read_identity:pin,permission_epoch,provider,provider_process:process};
        let context=AgentTerminalContext {target:target.clone(),epoch,source,scope,guard};context.validate(self,false)?;Ok(context)
    }
    fn validate_agent_terminal_context(&self,context:&AgentTerminalContext,ack:bool)->SecurityResult<()> {
        let original=&context.source;original.recheck_identity().map_err(policy)?;
        let current={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&context.target.session_id).filter(|binding|binding.target==context.target&&binding.source_epoch==context.epoch&&binding.key.server_id==original.server_id&&binding.key.terminal_id==context.scope.terminal_id&&binding.terminal_read_pin.as_ref()==Some(&context.scope.read_identity)).ok_or(ErrorCode::StaleSession)?;
            let source=inner.servers.get(&binding.key.server_id).and_then(|record|record.source.as_ref()).ok_or(ErrorCode::NotReady)?;
            if source.server_id!=original.server_id||source.incarnation!=original.incarnation||source.evidence!=original.evidence||source.socket_path!=original.socket_path||!Arc::ptr_eq(&source.invalidation,&original.invalidation){return Err(ErrorCode::StaleSession)}source.clone()};
        let state=self.state.session_state(&context.target,&context.epoch)?;
        if !ack {original.recheck().map_err(policy)?;current.recheck().map_err(policy)?;if state.summary.connectivity!=SessionConnectivity::Ready||!state.summary.capabilities.terminal_create{return Err(ErrorCode::NotReady)}}
        let pane=current.snapshot.pane(&context.scope.pane_id).map_err(policy)?;let before=original.snapshot.pane(&context.scope.pane_id).map_err(policy)?;
        if !current.snapshot.agent_shell_create_bound||current.snapshot.agent_shell_permission_epoch!=Some(context.scope.permission_epoch)||!positive_agent_alias(&current.snapshot,pane,context.scope.provider)||pane.agent_session!=before.agent_session||pane.read_identity.as_ref()!=Some(&context.scope.read_identity)||pane.terminal_id!=context.scope.terminal_id||pane.workspace_id!=context.scope.workspace_id||pane.tab_id!=context.scope.tab_id||state.summary.provider!=context.scope.provider||state.summary.terminal_id!=context.scope.terminal_id||state.summary.pane_id!=context.scope.pane_id{return Err(ErrorCode::StaleSession)}
        if process_evidence(context.scope.provider_process.pid).map_err(policy)?!=context.scope.provider_process{return Err(ErrorCode::StaleSession)}Ok(())
    }
    pub(crate) async fn agent_terminal_create_once(self:&Arc<Self>,security:&Security,auth:&Authorized,action:&Uuid,context:AgentTerminalContext)->SecurityResult<()> {
        context.validate(self,false)?;let client=Client::connect(&context.source.socket_path).await.map_err(policy)?;if client.evidence!=context.source.evidence{return Err(ErrorCode::StaleSession)}context.validate(self,false)?;
        let fd=client.fence_socket().map_err(policy)?;let effect_action=action.clone();let original=context.clone();
        let settlement_context=context.clone();let settlement_topology=self.clone();
        let acknowledge:crate::pairing::AgentTerminalAckGuard=Arc::new(move|commit|{(settlement_context.guard)(true,&mut ||{settlement_topology.validate_agent_terminal_context(&settlement_context,true)?;commit()})});
        security.terminal_creation_boundary(auth,action.clone(),context.target.clone(),context.epoch.clone(),fd,acknowledge,||context.validate(self,false),|checkpoint|async move {
            let response=client.request("shell.create_agent_bound",json!({"operation_id":effect_action,"scope":original.scope}),"shell_create_agent_bound").await;
            let (value,normal)=match response {Ok(value)=>(Some(value),true),Err(ApiError::Changed(value))=>(Some(value),false),Err(_)=>(None,false)};
            let Some(completion)=value.as_ref().and_then(|value|native_agent_terminal_completion(value,&original,&effect_action))else{return shell_transition(ShellCreationResources::default(),ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)};
            let mut resources=completion.resources;
            if checkpoint.retain(&original.target,&original.epoch,&resources).is_err(){return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
            if checkpoint.checkpoint().await.is_err(){return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
            if normal&&completion.created&&completion.consistent&&resources.complete()&&completion.created_scope.is_some()&&original.validate(self,true).is_ok(){
                if let Some(scope)=&completion.created_scope{resources.created_target=self.existing_created_shell_scope(&original.source,scope)}
                if resources.created_target.is_some()&&(checkpoint.retain(&original.target,&original.epoch,&resources).is_err()||checkpoint.checkpoint().await.is_err()||original.validate(self,true).is_err()){return shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)}
                return shell_transition(resources,ReceiptState::Succeeded,crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("native agent process terminal created".into())))}
            if normal&&completion.refused&&completion.consistent&&resources.workspace_id.is_none()&&resources.tab_id.is_none()&&resources.pane_id.is_none()&&resources.terminal_id.is_none()&&completion.created_scope.is_none(){return shell_transition(resources,ReceiptState::Failed,crate::storage::TransitionProof::DefinitiveNoEffect(Id("native agent process terminal refused before effect".into())))}
            shell_transition(resources,ReceiptState::Uncertain,crate::storage::TransitionProof::LostOutcome)
        }).await
    }
}
fn native_agent_terminal_completion(value:&Value,context:&AgentTerminalContext,action:&Uuid)->Option<NativeShellCompletion> {
    let body=value.get("result")?.as_object()?;
    if body.get("operation_id")?.as_str()?!=action.0 {return None}
    let echoed:NativeAgentShellScope=serde_json::from_value(body.get("scope")?.clone()).ok()?;
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
