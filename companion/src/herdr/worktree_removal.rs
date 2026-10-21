use super::*;
#[derive(Clone)]
pub(super) struct RemoveAdmission {target:Target,candidate:Uuid,preview:Option<Uuid>,source:Source,context:WorktreeReadContext,expires:std::time::Instant}
pub(crate) struct RemovePreviewObservation {admission:RemoveAdmission,pub(crate) result:WorktreesRemovePreviewResult}
fn scope_matches(value:&Value,context:&WorktreeReadContext)->bool {
    serde_json::from_value::<WorktreeReadScope>(value.clone()).is_ok_and(|scope|scope.pane_id==context.pane && scope.terminal_id==context.terminal && scope.workspace_id==context.workspace && scope.tab_id==context.tab && scope.read_identity==context.read_identity)
}
impl Topology {
    fn validate_remove_context(&self,admission:&RemoveAdmission,ack:bool)->SecurityResult<()> {
        admission.source.recheck_identity().map_err(policy)?;
        let source={let inner=self.inner.lock().expect("topology mutex");
            let binding=inner.bindings.get(&admission.target.session_id).filter(|binding|binding.target==admission.target && binding.source_epoch==admission.context.source_epoch && binding.key.server_id==admission.source.server_id && binding.key.terminal_id==admission.context.terminal && binding.terminal_read_pin.as_ref()==Some(&admission.context.read_identity)).ok_or(ErrorCode::StaleSession)?;
            let current=inner.servers.get(&binding.key.server_id).and_then(|record|record.source.as_ref()).ok_or(ErrorCode::NotReady)?;
            if current.incarnation!=admission.source.incarnation || current.evidence!=admission.source.evidence || current.socket_path!=admission.source.socket_path || !Arc::ptr_eq(&current.invalidation,&admission.source.invalidation){return Err(ErrorCode::StaleSession)}current.clone()
        };
        let state=self.state.session_state(&admission.target,&admission.context.source_epoch)?;
        if !ack {
            if std::time::Instant::now()>=admission.expires{return Err(ErrorCode::NotReady)}
            admission.source.recheck().map_err(policy)?;source.recheck().map_err(policy)?;
            if state.summary.connectivity!=SessionConnectivity::Ready || !state.summary.capabilities.worktree_remove{return Err(ErrorCode::NotReady)}
        }
        let context=&admission.context;let pane=source.snapshot.pane(&context.pane).map_err(policy)?;
        if !source.snapshot.worktree_remove_bound || state.summary.pane_id!=context.pane || state.summary.terminal_id!=context.terminal || state.summary.workspace_id!=context.workspace || pane.terminal_id!=context.terminal || pane.workspace_id!=context.workspace || pane.tab_id!=context.tab || pane.read_identity.as_ref()!=Some(&context.read_identity){return Err(ErrorCode::StaleSession)}Ok(())
    }
    pub(crate) async fn remove_preview(&self,params:&WorktreesRemovePreviewParams)->SecurityResult<RemovePreviewObservation> {
        let mut admission={let _update=self.updates.lock().await;
            let (server,epoch,pin)={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&params.target.session_id).filter(|binding|binding.target==params.target).ok_or(ErrorCode::StaleSession)?;(binding.key.server_id.clone(),binding.source_epoch.clone(),binding.terminal_read_pin.clone().ok_or(ErrorCode::NotReady)?)};
            let source=self.source(&server)?;let state=self.state.session_state(&params.target,&epoch)?;let pane=source.snapshot.pane(&state.summary.pane_id).map_err(policy)?;
            let context=WorktreeReadContext {source_epoch:epoch,pane:state.summary.pane_id,terminal:state.summary.terminal_id,workspace:state.summary.workspace_id,tab:pane.tab_id.clone(),read_identity:pin,creation:None};
            let admission=RemoveAdmission {target:params.target.clone(),candidate:params.candidate_id.clone(),preview:None,source,context,expires:std::time::Instant::now()+Duration::from_secs(50)};
            self.validate_remove_context(&admission,false)?;admission
        };
        let client=Client::connect(&admission.source.socket_path).await.map_err(policy)?;
        if client.evidence!=admission.source.evidence{return Err(ErrorCode::StaleSession)}self.validate_remove_context(&admission,false)?;
        let context=&admission.context;
        let value=client.request("worktree.remove_preview_bound",json!({"pane_id":context.pane,"expected_terminal_id":context.terminal,"expected_read_identity":context.read_identity,"candidate_id":params.candidate_id}),"worktree_remove_preview_bound").await.map_err(api_io).map_err(policy)?;
        let body=value.get("preview").ok_or(ErrorCode::NotReady)?;
        if !body.get("scope").is_some_and(|scope|scope_matches(scope,context)) || body.get("candidate_id").and_then(Value::as_str)!=Some(params.candidate_id.0.as_str()){return Err(ErrorCode::StaleSession)}
        let result=WorktreesRemovePreviewResult {target:params.target.clone(),candidate_id:params.candidate_id.clone(),preview_id:serde_json::from_value(body.get("preview_id").cloned().ok_or(ErrorCode::NotReady)?).map_err(|_| ErrorCode::NotReady)?,path:serde_json::from_value(body.get("path").cloned().ok_or(ErrorCode::NotReady)?).map_err(|_| ErrorCode::NotReady)?,branch:body.get("branch").map(|value|serde_json::from_value(value.clone()).map_err(|_| ErrorCode::NotReady)).transpose()?,head:body.get("head").map(|value|serde_json::from_value(value.clone()).map_err(|_| ErrorCode::NotReady)).transpose()?,removable:body.get("removable").and_then(Value::as_bool).ok_or(ErrorCode::NotReady)?,reason:body.get("reason").map(|value|serde_json::from_value(value.clone()).map_err(|_| ErrorCode::NotReady)).transpose()?};
        result.validate().map_err(|_| ErrorCode::NotReady)?;admission.preview=Some(result.preview_id.clone());self.validate_remove_context(&admission,false)?;
        Ok(RemovePreviewObservation {admission,result})
    }
    pub(crate) async fn enqueue_remove_preview(&self,connection:&Uuid,response_id:Uuid,observation:RemovePreviewObservation)->SecurityResult<()> {
        let _update=self.updates.lock().await;self.validate_remove_context(&observation.admission,false)?;
        let mut inner=self.inner.lock().expect("topology mutex");inner.removal_previews.retain(|_,preview|std::time::Instant::now()<preview.expires);
        if observation.result.removable && inner.removal_previews.len()>=64{return Err(ErrorCode::ResourceLimit)}
        let target=observation.admission.target.clone();let epoch=observation.admission.context.source_epoch.clone();
        self.state.worktrees_remove_preview_response(connection,&target,&epoch,response_id,observation.result.clone())?;
        if observation.result.removable{inner.removal_previews.insert(observation.result.preview_id.clone(),observation.admission);}Ok(())
    }
    pub(crate) async fn remove_worktree_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&WorktreesRemoveParams)->SecurityResult<()> {
        let admission={let _update=self.updates.lock().await;let admission=self.inner.lock().expect("topology mutex").removal_previews.remove(&params.preview_id).filter(|preview|preview.target==params.target && preview.candidate==params.candidate_id && preview.preview.as_ref()==Some(&params.preview_id)).ok_or(ErrorCode::NotReady)?;self.validate_remove_context(&admission,false)?;admission};
        let client=Client::connect(&admission.source.socket_path).await.map_err(policy)?;
        if client.evidence!=admission.source.evidence{return Err(ErrorCode::StaleSession)}self.validate_remove_context(&admission,false)?;
        let fd=client.fence_socket().map_err(policy)?;let original=admission.clone();
        security.worktree_removal_boundary(auth,action.clone(),params.clone(),admission.context.source_epoch.clone(),fd,||{
            let _update=self.updates.try_lock().map_err(|_|ErrorCode::Busy)?;self.validate_remove_context(&admission,false)?;
            security.state.validate_session_mutation(&Request {id:action.clone(),body:RequestBody::WorktreesRemove(params.clone())})
        },|checkpoint|async move {
            let transition=|progress,state,proof|crate::storage::ReceiptTransition {state,proof,result:Some(ActionResult::WorktreeRemoval(ActionResultWorktreeRemoval {target:params.target.clone(),candidate_id:params.candidate_id.clone(),preview_id:params.preview_id.clone(),progress})),error:None,at:pairing::now_ms()};
            let context=&original.context;
            let response=client.request("worktree.remove_bound",json!({"operation_id":action,"pane_id":context.pane,"expected_terminal_id":context.terminal,"expected_read_identity":context.read_identity,"candidate_id":params.candidate_id,"preview_id":params.preview_id}),"worktree_remove_bound").await;
            let (value,normal)=match response {Ok(value)=>(Some(value),true),Err(ApiError::Changed(value))=>(Some(value),false),Err(ApiError::Rejected(code)) if code=="worktree_remove_bound_unavailable"=>return transition(WorktreeRemovalProgress::default(),ReceiptState::Failed,TransitionProof::DefinitiveNoEffect(Id("native removal refused before admission".into()))),Err(_)=>(None,false)};
            let Some(completion)=value.as_ref().and_then(|value|completion(value,&original,action))else{return transition(WorktreeRemovalProgress::default(),ReceiptState::Uncertain,TransitionProof::LostOutcome)};
            let progress=completion.progress;
            if checkpoint.retain(&params.target,&original.context.source_epoch,&params.candidate_id,&params.preview_id,&progress).is_err() || checkpoint.checkpoint().await.is_err(){return transition(progress,ReceiptState::Uncertain,TransitionProof::LostOutcome)}
            if normal && completion.consistent && completion.removed && progress.complete() && self.validate_remove_context(&original,true).is_ok(){return transition(progress,ReceiptState::Succeeded,TransitionProof::CorrelatedAcknowledgement(Id("native closed checkout removed".into())))}
            if normal && completion.consistent && completion.refused && progress.effect_started==Some(false) && progress.git_remove_succeeded==Some(false) && progress.checkout_absent==Some(false) && progress.registration_absent==Some(false){return transition(progress,ReceiptState::Failed,TransitionProof::DefinitiveNoEffect(Id("native removal refused before effect".into())))}
            transition(progress,ReceiptState::Uncertain,TransitionProof::LostOutcome)
        }).await
    }
}
struct Completion {progress:WorktreeRemovalProgress,consistent:bool,removed:bool,refused:bool}
fn completion(value:&Value,admission:&RemoveAdmission,action:&Uuid)->Option<Completion> {
    let body=value.get("result")?;
    if body.get("operation_id").and_then(Value::as_str)!=Some(action.0.as_str()) || body.get("candidate_id").and_then(Value::as_str)!=Some(admission.candidate.0.as_str()) || body.get("preview_id").and_then(Value::as_str)!=Some(admission.preview.as_ref()?.0.as_str()) || !body.get("scope").is_some_and(|scope|scope_matches(scope,&admission.context)){return None}
    let flag=|key|body.get(key).and_then(Value::as_bool);
    let mut progress=WorktreeRemovalProgress {effect_started:flag("effect_started"),git_remove_succeeded:flag("git_remove_succeeded"),checkout_absent:flag("checkout_absent"),registration_absent:flag("registration_absent")};
    let valid_progress=progress.validate().is_ok();if !valid_progress{progress.git_remove_succeeded=None;}
    let reason=body.get("reason").and_then(|value|serde_json::from_value::<WorktreeRemoveReason>(value.clone()).ok());
    let outcome=body.get("outcome").and_then(Value::as_str);
    let consistent=valid_progress && [flag("effect_started"),flag("git_remove_succeeded"),flag("checkout_absent"),flag("registration_absent")].iter().all(Option::is_some) && reason.is_some() && matches!(outcome,Some("removed"|"refused_before_effect"|"uncertain"));
    Some(Completion {progress,consistent,removed:outcome==Some("removed") && reason==Some(WorktreeRemoveReason::Removed),refused:outcome==Some("refused_before_effect") && reason.is_some_and(|reason|!matches!(reason,WorktreeRemoveReason::Removed|WorktreeRemoveReason::OperationAlreadyConsumed|WorktreeRemoveReason::PreparationPending))})
}
