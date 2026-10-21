// SPDX-License-Identifier: GPL-3.0-or-later
//! Native-only independent user Skills-preview grant. No document path authority.
use super::*;
#[derive(Clone,Serialize,Deserialize,PartialEq,Eq)]#[serde(deny_unknown_fields)]
struct NativeSkillsScope {pane_id:Id,terminal_id:Id,workspace_id:Id,tab_id:Id,read_identity:PaneReadIdentity,permission_epoch:u64,provider_process:ProcessEvidence}
#[derive(Clone)]pub(crate) struct SkillsScopeContext {pub target:Target,pub epoch:Uuid,pub source:Source,scope:NativeSkillsScope}
impl SkillsScopeContext {
 pub(crate) fn permission_epoch(&self)->u64{self.scope.permission_epoch}
 pub(crate) async fn authorize(&self,topology:&Topology)->SecurityResult<()> {
  topology.validate_skills_scope(self)?;
  let client=Client::connect(&self.source.socket_path).await.map_err(policy)?;
  if client.evidence!=self.source.evidence{return Err(ErrorCode::StaleSession)}
  let result=client.request("skills.scope_bound",json!({"scope":self.scope}),"skills_scope_bound").await.map_err(api_io).map_err(policy)?;
  let scope:NativeSkillsScope=serde_json::from_value(result.get("scope").cloned().ok_or(ErrorCode::NotReady)?).map_err(|_|ErrorCode::NotReady)?;
  if scope!=self.scope{return Err(ErrorCode::StaleSession)}topology.validate_skills_scope(self)
 }
}
impl Topology {
 pub(crate) fn skills_scope_context(&self,target:&Target,owner:&ProcessEvidence)->SecurityResult<SkillsScopeContext>{
  let(server,epoch,pin)={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&target.session_id).filter(|binding|binding.target==*target).ok_or(ErrorCode::StaleSession)?;(binding.key.server_id.clone(),binding.source_epoch.clone(),binding.terminal_read_pin.clone().ok_or(ErrorCode::NotReady)?)};
  let source=self.source(&server)?;let state=self.state.session_state(target,&epoch)?;
  if state.summary.provider!=Provider::Omp{return Err(ErrorCode::NotReady)}let pane=source.snapshot.pane(&state.summary.pane_id).map_err(policy)?;
  let permission_epoch=source.snapshot.omp_skills_permission_epoch.filter(|epoch|*epoch>0).ok_or(ErrorCode::NotReady)?;
  let scope=NativeSkillsScope {pane_id:pane.pane_id.clone(),terminal_id:pane.terminal_id.clone(),workspace_id:pane.workspace_id.clone(),tab_id:pane.tab_id.clone(),read_identity:pin,permission_epoch,provider_process:owner.clone()};
  let context=SkillsScopeContext {target:target.clone(),epoch,source,scope};self.validate_skills_scope(&context)?;Ok(context)
 }
 pub(crate) fn with_skills_scope<T>(&self,context:&SkillsScopeContext,commit:impl FnOnce()->SecurityResult<T>)->SecurityResult<T>{
  self.validate_skills_scope(context)?;
  let inner=self.inner.lock().expect("topology mutex");
  let binding=inner.bindings.get(&context.target.session_id).filter(|binding|binding.target==context.target&&binding.source_epoch==context.epoch&&binding.terminal_read_pin.as_ref()==Some(&context.scope.read_identity)).ok_or(ErrorCode::StaleSession)?;
  let source=inner.servers.get(&binding.key.server_id).and_then(|record|record.source.as_ref()).ok_or(ErrorCode::NotReady)?;
  if source.server_id!=context.source.server_id||source.incarnation!=context.source.incarnation||source.evidence!=context.source.evidence||source.socket_path!=context.source.socket_path||!Arc::ptr_eq(&source.invalidation,&context.source.invalidation){return Err(ErrorCode::StaleSession)}
  if !source.revision_current()||!source.snapshot.omp_skills_preview_bound||source.snapshot.omp_skills_permission_epoch!=Some(context.scope.permission_epoch){return Err(ErrorCode::NotReady)}
  let pane=source.snapshot.pane(&context.scope.pane_id).map_err(policy)?;
  if pane.terminal_id!=context.scope.terminal_id||pane.workspace_id!=context.scope.workspace_id||pane.tab_id!=context.scope.tab_id||pane.read_identity.as_ref()!=Some(&context.scope.read_identity){return Err(ErrorCode::StaleSession)}
  commit()
 }
 pub(crate) fn validate_skills_scope(&self,context:&SkillsScopeContext)->SecurityResult<()> {
  let original=&context.source;original.recheck_identity().map_err(policy)?;
  let current={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&context.target.session_id).filter(|binding|binding.target==context.target&&binding.source_epoch==context.epoch&&binding.key.server_id==original.server_id&&binding.key.terminal_id==context.scope.terminal_id&&binding.terminal_read_pin.as_ref()==Some(&context.scope.read_identity)).ok_or(ErrorCode::StaleSession)?;
   let record=inner.servers.get(&binding.key.server_id).ok_or(ErrorCode::NotReady)?;let source=record.source.as_ref().ok_or(ErrorCode::NotReady)?;
   if source.server_id!=original.server_id||source.incarnation!=original.incarnation||source.evidence!=original.evidence||source.socket_path!=original.socket_path||!Arc::ptr_eq(&source.invalidation,&original.invalidation){return Err(ErrorCode::StaleSession)}source.clone()};
  current.recheck().map_err(policy)?;
  if !current.snapshot.omp_skills_preview_bound||current.snapshot.omp_skills_permission_epoch!=Some(context.scope.permission_epoch){return Err(ErrorCode::NotReady)}
  let pane=current.snapshot.pane(&context.scope.pane_id).map_err(policy)?;
  if pane.terminal_id!=context.scope.terminal_id||pane.workspace_id!=context.scope.workspace_id||pane.tab_id!=context.scope.tab_id||pane.read_identity.as_ref()!=Some(&context.scope.read_identity){return Err(ErrorCode::StaleSession)}
  let report=current.snapshot.agents.iter().find(|agent|agent.terminal_id==context.scope.terminal_id);let captured=original.snapshot.agents.iter().find(|agent|agent.terminal_id==context.scope.terminal_id);
  if report.map(|agent|(&agent.agent,&agent.agent_session))!=captured.map(|agent|(&agent.agent,&agent.agent_session))||!report.is_some_and(|agent|agent.agent.as_deref()==Some("omp")&&!agent.launch_pending){return Err(ErrorCode::StaleSession)}
  if process_evidence(context.scope.provider_process.pid).map_err(policy)?!=context.scope.provider_process{return Err(ErrorCode::StaleSession)}
  let state=self.state.session_state(&context.target,&context.epoch)?;
  if state.summary.connectivity!=SessionConnectivity::Ready||state.summary.provider!=Provider::Omp{return Err(ErrorCode::NotReady)}Ok(())
 }
}
