//! Host-only bound checkout authority. Filesystem proofs never enter phone parameters.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeObjectProof { pub path: String, pub device: Option<u64>, pub inode: Option<u64> }
impl NativeObjectProof {
    fn valid(&self) -> bool {
        self.path.starts_with('/') && self.path.len() <= 4096 && !self.path.chars().any(char::is_control)
            && self.device.is_some() && self.inode.is_some()
    }
    fn recheck(&self) -> SecurityResult<()> {
        if !self.valid() || !fs::symlink_metadata(&self.path).map_err(policy)?.is_dir() || object(Path::new(&self.path)).map_err(policy)? != (self.device.ok_or(ErrorCode::NotReady)?, self.inode.ok_or(ErrorCode::NotReady)?) { return Err(ErrorCode::StaleSession); }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeSourceProof {
    pub cwd: String, pub repo_root: NativeObjectProof, pub git_dir: NativeObjectProof,
    pub common_git_dir: NativeObjectProof, pub is_bare: bool,
}
impl NativeSourceProof {
    fn valid(&self) -> bool { self.cwd.starts_with('/') && self.cwd.len()<=4096 && !self.cwd.chars().any(char::is_control) && !self.is_bare && [&self.repo_root,&self.git_dir,&self.common_git_dir].iter().all(|proof|proof.valid()) }
    fn recheck(&self) -> SecurityResult<()> {
        if !self.valid() { return Err(ErrorCode::NotReady); }
        for proof in [&self.repo_root,&self.git_dir,&self.common_git_dir] { proof.recheck()?; }
        if configured_git(Path::new(&self.repo_root.path))?.as_ref() != Some(&(PathBuf::from(&self.git_dir.path),PathBuf::from(&self.common_git_dir.path))) { return Err(ErrorCode::StaleSession); }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(super) struct BoundCheckoutAdmission {
    target: Target, source_epoch: Uuid, server_id: Uuid, server_incarnation: Uuid,
    scope: WorktreeReadScope, source: NativeSourceProof, pub(super) project_id: Uuid,
    project_object: (u64,u64), evidence: ServerEvidence,
}
/// Durable host-only facts. Expected source authority remains separate from any
/// observed partial native output; neither becomes phone-controlled cwd authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionCheckout {
    pub action_id: Uuid, pub source_target: Target, pub source_epoch: Uuid,
    pub server_id: Uuid, pub server_incarnation: Uuid, pub evidence:ServerEvidence, pub scope: WorktreeReadScope,
    pub project_id: Uuid, pub project_object: (u64,u64), pub source: NativeSourceProof,
    pub checkout: CreationCheckout, pub resolved_commit: Option<String>,
    pub path: Option<String>, pub object: Option<NativeObjectProof>,
    pub checkout_source: Option<NativeSourceProof>,
    pub tab_id: Option<Id>,
}
impl ExecutionCheckout {
    fn intent(action: &Uuid, params: &SessionCreateParams, admission: &BoundCheckoutAdmission) -> Self {
        Self { action_id:action.clone(), source_target:admission.target.clone(), source_epoch:admission.source_epoch.clone(), server_id:admission.server_id.clone(), server_incarnation:admission.server_incarnation.clone(), evidence:admission.evidence.clone(), scope:admission.scope.clone(), project_id:admission.project_id.clone(), project_object:admission.project_object, source:admission.source.clone(), checkout:params.checkout.as_ref().expect("bound checkout").normalized(action), resolved_commit:None, path:None, object:None, checkout_source:None, tab_id:None }
    }
    pub(crate) fn retains(&self, old: &Self) -> bool {
        self.action_id==old.action_id && self.source_target==old.source_target
            && self.source_epoch==old.source_epoch && self.server_id==old.server_id
            && self.server_incarnation==old.server_incarnation && self.evidence==old.evidence && self.scope==old.scope
            && self.project_id==old.project_id && self.project_object==old.project_object
            && self.source==old.source && self.checkout==old.checkout
            && old.resolved_commit.as_ref().is_none_or(|value|self.resolved_commit.as_ref()==Some(value))
            && old.path.as_ref().is_none_or(|value|self.path.as_ref()==Some(value))
            && old.object.as_ref().is_none_or(|value|self.object.as_ref()==Some(value))
            && old.checkout_source.as_ref().is_none_or(|value|self.checkout_source.as_ref()==Some(value))
            && old.tab_id.as_ref().is_none_or(|value|self.tab_id.as_ref()==Some(value))
    }
    fn directory(&self) -> SecurityResult<PathBuf> {
        self.source.recheck()?;
        let path=self.path.as_ref().ok_or(ErrorCode::NotReady)?;
        let object=self.object.as_ref().ok_or(ErrorCode::NotReady)?;
        let source=self.checkout_source.as_ref().ok_or(ErrorCode::NotReady)?;
        if path != &object.path || source.cwd != *path || source.repo_root.device != object.device || source.repo_root.inode != object.inode || source.common_git_dir != self.source.common_git_dir { return Err(ErrorCode::StaleSession); }
        object.recheck()?; source.recheck()?;
        self.resolved_commit.as_ref().filter(|hash| valid_commit(hash)).ok_or(ErrorCode::NotReady)?;
        Ok(PathBuf::from(path))
    }
}
fn valid_commit(value:&str)->bool { [40,64].contains(&value.len()) && value.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)) }

/// Only configured checkout roots participate. No ancestor/project fallback can
/// broaden a configured subdirectory into authority for another repository.
fn configured_git(root:&Path)->SecurityResult<Option<(PathBuf,PathBuf)>> {
    let marker=root.join(".git");
    let metadata=match fs::metadata(&marker) { Ok(value)=>value,Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(error)=>return Err(policy(error)) };
    let git=if metadata.is_dir() { fs::canonicalize(&marker).map_err(policy)? }
    else {
        if !metadata.is_file() || metadata.len()>4096 { return Ok(None); }
        let value=fs::read_to_string(&marker).map_err(policy)?;
        let Some(value)=value.strip_prefix("gitdir: ").and_then(|line|line.strip_suffix('\n').or(Some(line))) else{return Ok(None)};
        if value.is_empty() || value.chars().any(char::is_control) {return Ok(None)}
        fs::canonicalize(root.join(value)).map_err(policy)?
    };
    let common=git.join("commondir");
    match fs::metadata(&common) {
        Err(error) if error.kind()==io::ErrorKind::NotFound=>Ok(Some((git.clone(),git))),
        Ok(metadata) if metadata.is_file() && metadata.len()<=4096=>{
            let value=fs::read_to_string(&common).map_err(policy)?;
            let value=value.strip_suffix('\n').unwrap_or(&value);
            if value.is_empty() || value.chars().any(char::is_control) {return Ok(None)}
            let common=fs::canonicalize(git.join(value)).map_err(policy)?;
            Ok(Some((git,common)))
        }
        _=>Ok(None),
    }
}

fn configured_common_git(root:&Path)->SecurityResult<Option<PathBuf>> {Ok(configured_git(root)?.map(|(_,common)|common))}

impl Topology {
    fn configured_project_for_source(&self, proof:&NativeSourceProof)->SecurityResult<Option<Uuid>> {
        proof.recheck()?;
        let mut matches=Vec::new();
        for project in &self.config.projects {
            let Some(common)=configured_common_git(&project.path)? else {continue};
            if object(&common).map_err(policy)? == (proof.common_git_dir.device.ok_or(ErrorCode::NotReady)?,proof.common_git_dir.inode.ok_or(ErrorCode::NotReady)?) { matches.push(project.id.clone()); }
        }
        Ok(if matches.len()==1 {matches.pop()} else {None})
    }
    pub(super) fn make_bound_checkout_admission(&self,target:&Target,source:&Source,context:&WorktreeReadContext,proof:Option<&NativeSourceProof>)->SecurityResult<Option<BoundCheckoutAdmission>> {
        if !source.snapshot.worktree_create_bound {return Ok(None)}
        let Some(proof)=proof else{return Ok(None)};
        let Some(project_id)=self.configured_project_for_source(proof)? else{return Ok(None)};
        let project=self.config.project(&project_id)?;
        Ok(Some(BoundCheckoutAdmission {target:target.clone(),source_epoch:context.source_epoch.clone(),server_id:source.server_id.clone(),server_incarnation:source.incarnation.clone(),scope:WorktreeReadScope {pane_id:context.pane.clone(),terminal_id:context.terminal.clone(),workspace_id:context.workspace.clone(),tab_id:context.tab.clone(),read_identity:context.read_identity.clone()},source:proof.clone(),project_id,project_object:object(&project.path).map_err(policy)?,evidence:source.evidence.clone()}))
    }
    pub(super) fn recheck_bound_checkout_admission(&self,admission:&BoundCheckoutAdmission)->SecurityResult<()> {
        let source=self.source(&admission.server_id)?;
        if source.incarnation!=admission.server_incarnation || source.evidence!=admission.evidence || !source.snapshot.worktree_create_bound {return Err(ErrorCode::StaleSession)}
        let epoch={let inner=self.inner.lock().expect("topology mutex");let binding=inner.bindings.get(&admission.target.session_id).filter(|binding|binding.target==admission.target && binding.source_epoch==admission.source_epoch && binding.terminal_read_pin.as_ref()==Some(&admission.scope.read_identity)).ok_or(ErrorCode::StaleSession)?;binding.source_epoch.clone()};
        let state=self.state.session_state(&admission.target,&epoch)?;
        if state.summary.server_id!=admission.server_id || state.summary.connectivity!=SessionConnectivity::Ready || !state.summary.capabilities.worktree_create {return Err(ErrorCode::NotReady)}
        let pane=source.snapshot.pane(&admission.scope.pane_id).map_err(policy)?;
        if pane.terminal_id!=admission.scope.terminal_id || pane.workspace_id!=admission.scope.workspace_id || pane.tab_id!=admission.scope.tab_id || pane.read_identity.as_ref()!=Some(&admission.scope.read_identity) || pane.cwd.as_deref()!=Some(admission.source.cwd.as_str()) {return Err(ErrorCode::StaleSession)}
        if self.configured_project_for_source(&admission.source)?.as_ref()!=Some(&admission.project_id) || object(&self.config.project(&admission.project_id)?.path).map_err(policy)?!=admission.project_object {return Err(ErrorCode::StaleSession)}
        source.recheck().map_err(policy)
    }
    pub(super) fn validate_bound_checkout_request(&self,params:&SessionCreateParams)->SecurityResult<Option<BoundCheckoutAdmission>> {
        let Some(target)=&params.source_target else {if params.checkout.is_some(){return Err(ErrorCode::InvalidRequest)}return Ok(None)};
        if params.checkout.is_none(){return Err(ErrorCode::InvalidRequest)}
        let admission=self.inner.lock().expect("topology mutex").worktree_admissions.get(&target.session_id).filter(|admission|&admission.target==target && admission.server_id==params.server_id && admission.project_id==params.project_id).cloned().ok_or(ErrorCode::NotReady)?;
        self.recheck_bound_checkout_admission(&admission)?;
        Ok(Some(admission))
    }
    fn recheck_execution_source(&self,proof:&ExecutionCheckout)->SecurityResult<()> {
        // Reconstruct only the action's retained admission, never the newest list cache.
        self.recheck_bound_checkout_admission(&BoundCheckoutAdmission {
            target:proof.source_target.clone(),source_epoch:proof.source_epoch.clone(),server_id:proof.server_id.clone(),
            server_incarnation:proof.server_incarnation.clone(),scope:proof.scope.clone(),source:proof.source.clone(),
            project_id:proof.project_id.clone(),project_object:proof.project_object,evidence:proof.evidence.clone(),
        })
    }
    pub(crate) fn execution_directory(&self,action:&Uuid,params:&SessionCreateParams)->SecurityResult<PathBuf> {
        self.config.project(&params.project_id)?;
        if params.checkout.is_none(){return Ok(self.config.project(&params.project_id)?.path.clone())}
        let proof=self.inner.lock().expect("topology mutex").execution_checkouts.get(action).filter(|proof|proof.action_id==*action && proof.server_id==params.server_id && Some(&proof.source_target)==params.source_target.as_ref() && proof.project_id==params.project_id && proof.checkout==params.checkout.as_ref().expect("checkout").normalized(action)).cloned().ok_or(ErrorCode::NotReady)?;
        self.recheck_execution_source(&proof)?;
        proof.directory()
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
enum NativeCheckoutOutcome { Created, RefusedBeforeEffect, Uncertain }
#[derive(Debug, Deserialize)]
struct NativeCheckoutResult {
    operation_id: Uuid, scope:WorktreeReadScope, outcome:NativeCheckoutOutcome,
    checkout:CreationCheckout, resolved_commit:Option<String>, source:Option<NativeSourceProof>,
    checkout_path:Option<String>, checkout_identity:Option<NativeObjectProof>, checkout_source:Option<NativeSourceProof>,
}
impl Topology {
    pub(super) async fn bound_workspace_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&SessionCreateParams,current:Receipt)->SecurityResult<Receipt> {
        let admission=self.validate_bound_checkout_request(params)?.ok_or(ErrorCode::NotReady)?;
        if self.inner.lock().expect("topology mutex").execution_checkouts.len()>=4096 {return Err(ErrorCode::ResourceLimit)}
        let source=self.source(&params.server_id)?;
        self.validate_topology_stage(&source,params,None)?;
        let progress=current.creation.as_ref().ok_or(ErrorCode::InvalidRequest)?;
        if progress.source_target!=params.source_target || progress.resources.checkout.as_ref()!=Some(&params.checkout.as_ref().ok_or(ErrorCode::InvalidRequest)?.resources(action)) {return Err(ErrorCode::InvalidRequest)}
        let client=Client::connect(&source.socket_path).await.map_err(policy)?;
        if client.evidence!=source.evidence {return Err(ErrorCode::StaleSession)}
        let fd=client.fence_socket().map_err(policy)?;
        let mut metadata=client.metadata();
        metadata.socket_epoch=Some(source.incarnation.clone());
        metadata.focused_workspace_id=source.snapshot.focused_workspace_id.clone();
        metadata.execution_checkout=Some(ExecutionCheckout::intent(action,params,&admission));
        let mut effect_metadata=metadata.clone();
        let mut resources=progress.resources.clone();
        let operation=action.clone();
        let intent=params.checkout.as_ref().ok_or(ErrorCode::InvalidRequest)?.normalized(action);
        let original=admission.clone();
        security.workspace_creation_boundary(auth,action.clone(),metadata,||{
            self.validate_topology_stage(&source,params,None)?;
            self.recheck_bound_checkout_admission(&admission)
        },fd,|checkpoint| async move {
            let response=client.request("worktree.create_bound",json!({
                "operation_id":operation,"pane_id":original.scope.pane_id,
                "expected_terminal_id":original.scope.terminal_id,
                "expected_read_identity":original.scope.read_identity,"checkout":intent,
            }),"worktree_create_bound").await;
            let mut state=StageState::Uncertain;
            let mut definitive=None;
            let changed=matches!(&response,Err(ApiError::Changed(_)));
            let value=match response {Ok(value)|Err(ApiError::Changed(value))=>Some(value),_=>None};
            if let Some(value)=value {
                let value=value.get("result").cloned().unwrap_or(Value::Null);
                let (known,complete)=workspace_resources(&value);
                if known.workspace_id.is_some(){resources.workspace_id=known.workspace_id}
                if known.pane_id.is_some(){resources.pane_id=known.pane_id}
                if known.terminal_id.is_some(){resources.terminal_id=known.terminal_id}
                // Even malformed or changed replies cannot erase already known native IDs.
                if let Some(proof)=effect_metadata.execution_checkout.as_mut() {
                    proof.path=value.get("checkout_path").and_then(Value::as_str).filter(|path|path.starts_with('/')&&path.len()<=4096&&!path.chars().any(char::is_control)).map(str::to_owned);
                    proof.object=value.get("checkout_identity").and_then(|value|serde_json::from_value::<NativeObjectProof>(value.clone()).ok()).filter(NativeObjectProof::valid);
                    proof.checkout_source=value.get("checkout_source").and_then(|value|serde_json::from_value::<NativeSourceProof>(value.clone()).ok()).filter(NativeSourceProof::valid);
                    proof.resolved_commit=value.get("resolved_commit").and_then(Value::as_str).filter(|value|valid_commit(value)).map(str::to_owned);
                    proof.tab_id=value.pointer("/tab/tab_id").and_then(Value::as_str).and_then(|value|Id::new(value).ok());
                    if let Some(checkout)=resources.checkout.as_mut(){checkout.resolved_commit=proof.resolved_commit.clone()}
                }
                if checkpoint.workspace(resources.clone(),effect_metadata.clone()).await.is_err() {
                    return CreationOutcome {resources,metadata:effect_metadata,state:StageState::Uncertain,definitive:None,at:pairing::now_ms()};
                }
                if let Ok(native)=serde_json::from_value::<NativeCheckoutResult>(value) {
                    let correlated=native.operation_id==operation && native.scope==original.scope && native.checkout==intent;
                    if correlated && !changed && native.outcome==NativeCheckoutOutcome::RefusedBeforeEffect
                        && resources.workspace_id.is_none() && resources.pane_id.is_none() && resources.terminal_id.is_none()
                        && native.checkout_identity.is_none() && native.checkout_source.is_none() {
                        state=StageState::Failed;definitive=Some(Id("Correlated native checkout refusal before effect".into()));
                    } else if correlated && !changed && native.outcome==NativeCheckoutOutcome::Created && complete
                        && native.source.as_ref()==Some(&original.source)
                        && native.resolved_commit.as_ref().is_some_and(|value|valid_commit(value))
                        && native.checkout_path==effect_metadata.execution_checkout.as_ref().and_then(|proof|proof.path.clone())
                        && native.checkout_identity==effect_metadata.execution_checkout.as_ref().and_then(|proof|proof.object.clone())
                        && native.checkout_source==effect_metadata.execution_checkout.as_ref().and_then(|proof|proof.checkout_source.clone())
                        && effect_metadata.execution_checkout.as_ref().is_some_and(|proof|proof.directory().is_ok())
                    {
                        // Expected creation events can invalidate a topology snapshot. Wait only
                        // for its existing authoritative follower, never retarget or issue a probe.
                        let until=tokio::time::Instant::now()+Duration::from_secs(2);
                        loop {
                            match self.recheck_bound_checkout_admission(&original) {
                                Ok(())=>{state=StageState::Confirmed;definitive=Some(Id("Correlated original bound checkout and complete native root".into()));break},
                                Err(ErrorCode::NotReady) if tokio::time::Instant::now()<until=>tokio::time::sleep(Duration::from_millis(20)).await,
                                _=>break,
                            }
                        }
                    }
                }
            }
            CreationOutcome {resources,metadata:effect_metadata,state,definitive,at:pairing::now_ms()}
        }).await?;
        let receipt=self.receipt(security,auth,action).await?;
        if receipt.creation.as_ref().is_some_and(|progress|progress.stage_state==StageState::Confirmed) {
            let proof=auth.proof.clone();let id=action.clone();
            let local=self.storage.call(move|store|store.creation_local(&proof,&id)).await.map_err(pairing::store_error)?.ok_or(ErrorCode::NotReady)?;
            let execution=local.stages.into_iter().find(|stage|stage.stage==CreationStage::Workspace && stage.state==StageState::Confirmed && stage.intent.workspace_operation==WorkspaceOperation::Create).and_then(|stage|stage.result.and_then(|metadata|metadata.execution_checkout)).ok_or(ErrorCode::NotReady)?;
            if execution.directory().is_err() || self.recheck_execution_source(&execution).is_err() {
                let proof=auth.proof.clone();let id=action.clone();
                self.storage.call(move|store|store.transition(&proof,&id,ReceiptTransition {state:ReceiptState::Uncertain,proof:TransitionProof::LostOutcome,result:None,error:None,at:pairing::now_ms()})).await.map_err(pairing::store_error)?;
                return self.receipt(security,auth,action).await;
            }
            self.inner.lock().expect("topology mutex").execution_checkouts.insert(action.clone(),execution);
            let resources=&receipt.creation.as_ref().ok_or(ErrorCode::NotReady)?.resources;
            if self.confirm_workspace(&source,resources,params.provider).await.is_err() {
                let proof=auth.proof.clone();let id=action.clone();
                self.storage.call(move|store|store.transition(&proof,&id,ReceiptTransition {state:ReceiptState::Uncertain,proof:TransitionProof::LostOutcome,result:None,error:None,at:pairing::now_ms()})).await.map_err(pairing::store_error)?;
                return self.receipt(security,auth,action).await;
            }
        }
        Ok(receipt)
    }
}
