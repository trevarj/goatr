//! Ephemeral, peer-scoped artifact authority. No paths or payload enter storage.
use super::*;
use base64::Engine;
use std::time::{Duration,Instant};
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum NativeArtifactKind{Agent{provider:Provider,provider_process:ProcessEvidence},Shell}
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeArtifactScope{pub pane_id:Id,pub terminal_id:Id,pub workspace_id:Id,pub tab_id:Id,pub read_identity:PaneReadIdentity,pub permission_epoch:u64,pub source:NativeArtifactKind}
impl std::fmt::Debug for NativeArtifactScope{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("NativeArtifactScope(redacted)")}}
impl NativeArtifactScope{
 pub(super) fn identity(&self)->WorktreeReadScope{WorktreeReadScope{pane_id:self.pane_id.clone(),terminal_id:self.terminal_id.clone(),workspace_id:self.workspace_id.clone(),tab_id:self.tab_id.clone(),read_identity:self.read_identity.clone()}}
 pub(super) fn validate(&self)->io::Result<()>{for id in [&self.pane_id,&self.terminal_id,&self.workspace_id,&self.tab_id]{id_valid(id)?}self.read_identity.validate()?;if self.permission_epoch==0||self.read_identity.runtime_epoch==0||self.read_identity.binding_epoch==0{return Err(invalid("artifact scope unavailable"))}Ok(())}
}
pub(super) fn positive_artifact_shell<'a>(snapshot:&Snapshot,pane:&'a Pane)->Option<&'a NativeArtifactScope>{
 let scope=pane.artifact_shell_scope.as_ref()?;
 if !snapshot.artifact_read_bound||snapshot.artifact_permission_epoch!=Some(scope.permission_epoch)||!matches!(scope.source,NativeArtifactKind::Shell)||scope.validate().is_err()||pane.agent.is_some()||pane.agent_session.is_some()||snapshot.agents.iter().any(|agent|agent.terminal_id==pane.terminal_id)||scope.pane_id!=pane.pane_id||scope.terminal_id!=pane.terminal_id||scope.workspace_id!=pane.workspace_id||scope.tab_id!=pane.tab_id||pane.read_identity.as_ref()!=Some(&scope.read_identity){return None}Some(scope)
}
type Guard=Arc<dyn Fn(&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()>+Send+Sync>;
#[derive(Clone)]
pub(crate) struct ArtifactContext{pub source:ArtifactSource,pub epoch:Uuid,original:Source,scope:NativeArtifactScope,guard:Option<Guard>}
pub(crate) fn agent_artifacts_available(source:&Source,agent:&Agent,process:&ProcessEvidence)->bool{source.snapshot.artifact_read_bound&&source.snapshot.artifact_permission_epoch.is_some_and(|epoch|epoch>0)&&source.snapshot.pane(&agent.pane_id).ok().is_some_and(|pane|pane.terminal_id==agent.terminal_id&&pane.read_identity.is_some())&&process_evidence(process.pid).is_ok_and(|current|current==*process)}
impl Topology{
 pub(crate) fn agent_artifact_context(&self,target:&Target,process:ProcessEvidence,provider:Provider,guard:Guard)->SecurityResult<ArtifactContext>{
  let (server,epoch,pin)={let inner=self.inner.lock().expect("topology mutex");let b=inner.bindings.get(&target.session_id).filter(|b|b.target==*target).ok_or(ErrorCode::StaleSession)?;(b.key.server_id.clone(),b.source_epoch.clone(),b.terminal_read_pin.clone().ok_or(ErrorCode::NotReady)?)};
  let original=self.source(&server)?;let state=self.state.session_state(target,&epoch)?;let pane=original.snapshot.pane(&state.summary.pane_id).map_err(policy)?;let permission_epoch=original.snapshot.artifact_permission_epoch.ok_or(ErrorCode::NotReady)?;
  let context=ArtifactContext{source:ArtifactSource::Session{target:target.clone()},epoch,scope:NativeArtifactScope{pane_id:pane.pane_id.clone(),terminal_id:pane.terminal_id.clone(),workspace_id:pane.workspace_id.clone(),tab_id:pane.tab_id.clone(),read_identity:pin,permission_epoch,source:NativeArtifactKind::Agent{provider,provider_process:process}},original,guard:Some(guard)};self.with_artifact_context(&context,&mut ||Ok(()))?;Ok(context)
 }
 pub(crate) fn shell_artifact_context(&self,target:&ShellTarget)->SecurityResult<ArtifactContext>{
  let context={let inner=self.inner.lock().expect("topology mutex");let b=inner.shell_bindings.get(&target.shell_id).filter(|b|b.target==*target).ok_or(ErrorCode::StaleSession)?;ArtifactContext{source:ArtifactSource::Shell{target:target.clone()},epoch:b.proof.source_epoch.clone(),original:b.source.clone().ok_or(ErrorCode::NotReady)?,scope:b.proof.artifact_scope.clone().ok_or(ErrorCode::NotReady)?,guard:None}};self.with_artifact_context(&context,&mut ||Ok(()))?;Ok(context)
 }
 fn with_artifact_context(&self,context:&ArtifactContext,operation:&mut dyn FnMut()->SecurityResult<()>)->SecurityResult<()>{
  let mut validate=||{
   context.original.recheck_identity().map_err(policy)?;context.scope.validate().map_err(policy)?;self.state.validate_artifact_source(&context.source,&context.epoch)?;
   if let NativeArtifactKind::Agent{provider_process,..}=&context.scope.source{if process_evidence(provider_process.pid).map_err(policy)?!=*provider_process{return Err(ErrorCode::StaleSession)}}
   let inner=self.inner.lock().expect("topology mutex");let record=inner.servers.get(&context.original.server_id).and_then(|s|s.source.as_ref()).ok_or(ErrorCode::StaleSession)?;
   if record.incarnation!=context.original.incarnation||record.evidence!=context.original.evidence||record.socket_path!=context.original.socket_path||!Arc::ptr_eq(&record.invalidation,&context.original.invalidation)||!record.snapshot.artifact_read_bound||record.snapshot.artifact_permission_epoch!=Some(context.scope.permission_epoch){return Err(ErrorCode::StaleSession)}
   if !record.recheck_readiness().map_err(policy)?{return Err(ErrorCode::NotReady)}
   let pane=record.snapshot.pane(&context.scope.pane_id).map_err(policy)?;
   if pane.terminal_id!=context.scope.terminal_id||pane.workspace_id!=context.scope.workspace_id||pane.tab_id!=context.scope.tab_id||pane.read_identity.as_ref()!=Some(&context.scope.read_identity){return Err(ErrorCode::StaleSession)}
   match &context.source{
    ArtifactSource::Session{target}=>{let b=inner.bindings.get(&target.session_id).filter(|b|b.target==*target&&b.source_epoch==context.epoch&&b.terminal_read_pin.as_ref()==Some(&context.scope.read_identity)).ok_or(ErrorCode::StaleSession)?;let NativeArtifactKind::Agent{provider,..}=&context.scope.source else{return Err(ErrorCode::NotReady)};if b.key.server_id!=record.server_id||!positive_agent_alias(&record.snapshot,pane,*provider){return Err(ErrorCode::StaleSession)}},
    ArtifactSource::Shell{target}=>{let b=inner.shell_bindings.get(&target.shell_id).filter(|b|b.target==*target&&b.proof.source_epoch==context.epoch&&b.proof.artifact_scope.as_ref()==Some(&context.scope)).ok_or(ErrorCode::StaleSession)?;if b.key.server()!=&record.server_id||positive_artifact_shell(&record.snapshot,pane)!=Some(&context.scope){return Err(ErrorCode::StaleSession)}},
   }
   // The original backend guard and topology membership stay held through the
   // final State enqueue; no native I/O or await occurs under either guard.
   operation()
  };
  if let Some(guard)=&context.guard{guard(&mut validate)}else{validate()}
 }
}
struct Authority{peer:Uuid,owner:Uuid,context:ArtifactContext,expires:Instant,refs:HashMap<Uuid,(Uuid,ArtifactEntry)>,cursors:HashMap<Uuid,Uuid>,directories:usize,bytes:usize}
struct ReadLease{peer:Uuid,authority:Arc<std::sync::Mutex<Authority>>,native:Option<Uuid>,size:u64,expires:Instant}
#[derive(Default)]
pub(super) struct ArtifactLeases{authorities:Vec<Arc<std::sync::Mutex<Authority>>>,reads:HashMap<Uuid,ReadLease>}
impl ArtifactLeases{
 pub(super) fn maintain(&mut self){self.expire();}
 fn expire(&mut self){let now=Instant::now();self.reads.retain(|_,r|now<r.expires);for authority in &self.authorities{if let Ok(mut authority)=authority.lock(){if now>=authority.expires{authority.refs.clear();authority.cursors.clear();authority.bytes=0;authority.directories=0}}}self.authorities.retain(|a|a.lock().is_ok_and(|a|now<a.expires)||self.reads.values().any(|r|Arc::ptr_eq(&r.authority,a)));}
}
pub(crate) struct PendingOpen{topology:Arc<Topology>,public:Uuid,context:ArtifactContext,owner:Uuid,native:Uuid,armed:bool}
impl PendingOpen{pub(crate) fn published(&mut self){self.armed=false;}}
impl Drop for PendingOpen{fn drop(&mut self){if !self.armed{return}let topology=self.topology.clone();let public=self.public.clone();let context=self.context.clone();let owner=self.owner.clone();let native=self.native.clone();tokio::spawn(async move{topology.close_retained_artifact(&public,&context,&owner,&native).await;});}}
#[derive(Deserialize)]struct NativeEntry{reference:Uuid,name:String,kind:ArtifactKind,size:Option<u64>}
#[derive(Deserialize)]struct NativeList{scope:NativeArtifactScope,entries:Vec<NativeEntry>,next_cursor:Option<Uuid>}
#[derive(Deserialize)]struct NativeOpen{scope:NativeArtifactScope,read_id:Uuid,name:String,size:u64,sha256:String,preview_kind:ArtifactPreviewKind}
#[derive(Deserialize)]struct NativeRead{scope:NativeArtifactScope,read_id:Uuid,offset:u64,data:Base64Bytes,eof:bool}
impl Topology{
 pub(crate) async fn artifact_request(self:&Arc<Self>,peer:&Uuid,body:&RequestBody,initial:Option<ArtifactContext>)->SecurityResult<(ArtifactContext,ResultBody,Option<PendingOpen>)>{
  let source=match body{RequestBody::ArtifactsList(p)=>&p.source,RequestBody::ArtifactsOpen(p)=>&p.source,RequestBody::ArtifactsRead(p)=>&p.source,RequestBody::ArtifactsClose(p)=>&p.source,_=>return Err(ErrorCode::InvalidRequest)};
  let mut native_read=None;let mut reservation=None;let mut native_file=None;let mut native_cursor=None;let mut native_parent=None;
  let authority={let mut leases=self.artifact_leases.lock().expect("artifact leases");leases.expire();match body{
   RequestBody::ArtifactsList(p)if p.parent.is_none()&&p.cursor.is_none()=>{
    let context=initial.ok_or(ErrorCode::NotReady)?;if context.source!=*source{return Err(ErrorCode::StaleSession)}
    let matching=leases.authorities.iter().find(|a|a.lock().is_ok_and(|a|a.peer==*peer&&Instant::now()<a.expires&&a.context.source==context.source&&a.context.epoch==context.epoch&&a.context.scope==context.scope&&a.context.original.server_id==context.original.server_id&&a.context.original.incarnation==context.original.incarnation&&a.context.original.evidence==context.original.evidence&&a.context.original.socket_path==context.original.socket_path&&Arc::ptr_eq(&a.context.original.invalidation,&context.original.invalidation))).cloned();
    if let Some(matching)=matching{matching}else{
    if leases.authorities.len()>=128||leases.authorities.iter().filter(|a|a.lock().is_ok_and(|a|a.peer==*peer)).count()>=32{return Err(ErrorCode::ResourceLimit)}
    let authority=Arc::new(std::sync::Mutex::new(Authority{peer:peer.clone(),owner:Uuid::random(),context,expires:Instant::now()+Duration::from_secs(300),refs:HashMap::new(),cursors:HashMap::new(),directories:0,bytes:0}));leases.authorities.push(authority.clone());authority}
   },
   RequestBody::ArtifactsList(p)=>leases.authorities.iter().find(|a|a.lock().is_ok_and(|a|a.peer==*peer&&a.context.source==*source&&Instant::now()<a.expires&&p.parent.as_ref().is_some_and(|r|a.refs.contains_key(r))||a.peer==*peer&&a.context.source==*source&&Instant::now()<a.expires&&p.cursor.as_ref().is_some_and(|r|a.cursors.contains_key(r)))).cloned().ok_or(ErrorCode::NotReady)?,
   RequestBody::ArtifactsOpen(p)=>leases.authorities.iter().find(|a|a.lock().is_ok_and(|a|a.peer==*peer&&a.context.source==*source&&Instant::now()<a.expires&&a.refs.contains_key(&p.file))).cloned().ok_or(ErrorCode::NotReady)?,
   RequestBody::ArtifactsRead(p)=>{let read=leases.reads.get(&p.read_id).filter(|r|r.peer==*peer).ok_or(ErrorCode::NotReady)?;native_read=Some(read.native.clone().ok_or(ErrorCode::NotReady)?);read.authority.clone()},
   RequestBody::ArtifactsClose(p)=>{let read=leases.reads.get(&p.read_id).filter(|r|r.peer==*peer).ok_or(ErrorCode::NotReady)?;native_read=Some(read.native.clone().ok_or(ErrorCode::NotReady)?);read.authority.clone()},
   _=>return Err(ErrorCode::InvalidRequest),
  }};
  let (context,owner)={let a=authority.lock().map_err(|_|ErrorCode::NotReady)?;if a.peer!=*peer||a.context.source!=*source{return Err(ErrorCode::StaleSession)}
   match body{
    RequestBody::ArtifactsList(p)=>{if let Some(parent)=&p.parent{let (native,entry)=a.refs.get(parent).ok_or(ErrorCode::NotReady)?;if entry.kind!=ArtifactKind::Directory{return Err(ErrorCode::InvalidRequest)}native_parent=Some(native.clone())}if let Some(cursor)=&p.cursor{native_cursor=Some(a.cursors.get(cursor).ok_or(ErrorCode::NotReady)?.clone())}},
    RequestBody::ArtifactsOpen(p)=>{let (native,entry)=a.refs.get(&p.file).ok_or(ErrorCode::NotReady)?;if entry.kind!=ArtifactKind::File||entry.size.is_none_or(|s|s.0>32*1024*1024){return Err(ErrorCode::ResourceLimit)}native_file=Some((native.clone(),entry.clone()))},_=>{},
   }(a.context.clone(),a.owner.clone())};
  if !matches!(body,RequestBody::ArtifactsClose(_)){self.with_artifact_context(&context,&mut ||Ok(()))?}
  if let Some((_,entry))=&native_file{let size=entry.size.ok_or(ErrorCode::NotReady)?.0;let mut leases=self.artifact_leases.lock().expect("artifact leases");leases.expire();let owned:Vec<_>=leases.reads.values().filter(|r|r.peer==*peer).collect();if leases.reads.len()>=32||owned.len()>=1||owned.iter().map(|r|r.size).sum::<u64>().checked_add(size).is_none_or(|n|n>64*1024*1024)||leases.reads.values().map(|r|r.size).sum::<u64>().checked_add(size).is_none_or(|n|n>256*1024*1024){return Err(ErrorCode::ResourceLimit)}let id=Uuid::random();leases.reads.insert(id.clone(),ReadLease{peer:peer.clone(),authority:authority.clone(),native:Some(Uuid::random()),size,expires:Instant::now()+Duration::from_secs(615)});reservation=Some(id);}
  if let RequestBody::ArtifactsList(p)=body{let leases=self.artifact_leases.lock().expect("artifact leases");let mut total=0usize;let mut peer_total=0usize;let mut directories=0usize;let mut peer_directories=0usize;for a in &leases.authorities{let a=a.lock().map_err(|_|ErrorCode::NotReady)?;total+=a.bytes;directories+=a.directories;if a.peer==*peer{peer_total+=a.bytes;peer_directories+=a.directories}}
   let a=authority.lock().map_err(|_|ErrorCode::NotReady)?;let replacing=p.parent.is_none()&&p.cursor.is_none();if replacing{total-=a.bytes;peer_total-=a.bytes;directories-=a.directories;peer_directories-=a.directories}if total+256*1024>16*1024*1024||peer_total+256*1024>4*1024*1024||p.cursor.is_none()&&(!replacing&&a.directories>=16||directories>=256||peer_directories>=64)||(!replacing&&a.refs.len()+p.limit.0 as usize>4096){return Err(ErrorCode::ResourceLimit)}}
  let pending_open=if let Some(public)=&reservation{let native=self.artifact_leases.lock().expect("artifact leases").reads.get(public).and_then(|r|r.native.clone()).ok_or(ErrorCode::NotReady)?;Some(PendingOpen{topology:self.clone(),public:public.clone(),context:context.clone(),owner:owner.clone(),native,armed:true})}else{None};
  context.original.recheck_identity().map_err(policy)?;
  let client=Client::connect(&context.original.socket_path).await.map_err(policy)?;if client.evidence!=context.original.evidence{return Err(ErrorCode::StaleSession)}if !matches!(body,RequestBody::ArtifactsClose(_)){self.with_artifact_context(&context,&mut ||Ok(()))?}
  let (method,params,kind)=match body{
   RequestBody::ArtifactsList(p)=>("artifact.list_bound",json!({"scope":context.scope,"owner_id":owner,"parent":native_parent,"cursor":native_cursor,"limit":p.limit}),"artifact_list_bound"),
   RequestBody::ArtifactsOpen(_)=>("artifact.open_bound",json!({"scope":context.scope,"owner_id":owner,"read_id":pending_open.as_ref().ok_or(ErrorCode::NotReady)?.native,"file":native_file.as_ref().ok_or(ErrorCode::NotReady)?.0}),"artifact_open_bound"),
   RequestBody::ArtifactsRead(p)=>("artifact.read_bound",json!({"scope":context.scope,"owner_id":owner,"read_id":native_read,"offset":p.offset,"limit":p.limit}),"artifact_read_bound"),
   RequestBody::ArtifactsClose(_)=>("artifact.close_bound",json!({"scope":context.scope,"owner_id":owner,"read_id":native_read}),"artifact_close_bound"),_=>return Err(ErrorCode::InvalidRequest)};
  let reply=client.request(method,params,kind).await.map_err(|error|match error{ApiError::Rejected(code)if code=="artifact_resource_limit"||code=="artifact_too_large"=>ErrorCode::ResourceLimit,error=>policy(api_io(error))})?;let result=reply.get("result").ok_or(ErrorCode::NotReady)?.clone();
  if !matches!(body,RequestBody::ArtifactsClose(_)){self.with_artifact_context(&context,&mut ||Ok(()))?}
  let result=match body{
   RequestBody::ArtifactsList(p)=>{let list:NativeList=serde_json::from_value(result).map_err(|_|ErrorCode::NotReady)?;if list.scope!=context.scope||list.entries.len()>p.limit.0 as usize{return Err(ErrorCode::NotReady)}let mut a=authority.lock().map_err(|_|ErrorCode::NotReady)?;if Instant::now()>=a.expires{return Err(ErrorCode::NotReady)}if p.parent.is_none()&&p.cursor.is_none(){a.refs.clear();a.cursors.clear();a.bytes=0;a.directories=0;}let mut entries=Vec::new();for item in list.entries{let existing=a.refs.iter().find(|(_,v)|v.0==item.reference).map(|(public,_)|public.clone());let public=existing.unwrap_or_else(Uuid::random);let entry=ArtifactEntry{reference:public.clone(),name:item.name,kind:item.kind,size:item.size.map(Uint::new).transpose().map_err(|_|ErrorCode::NotReady)?};entry.validate().map_err(|_|ErrorCode::NotReady)?;if !a.refs.contains_key(&public){let bytes=a.bytes.checked_add(entry.name.len()+160).filter(|bytes|*bytes<=256*1024).ok_or(ErrorCode::ResourceLimit)?;a.bytes=bytes;a.refs.insert(public,(item.reference,entry.clone()));}entries.push(entry)}let next_cursor=if let Some(native)=list.next_cursor{let known=a.cursors.iter().find(|(_,value)|**value==native).map(|(public,_)|public.clone());let public=if let Some(known)=known{known}else{a.bytes=a.bytes.checked_add(128).filter(|bytes|*bytes<=256*1024).ok_or(ErrorCode::ResourceLimit)?;Uuid::random()};a.cursors.insert(public.clone(),native);Some(public)}else{None};if p.cursor.is_none(){a.directories+=1}let result=ArtifactsListResult{source:source.clone(),entries,next_cursor};result.validate().map_err(|_|ErrorCode::NotReady)?;ResultBody::ArtifactsList(result)},
   RequestBody::ArtifactsOpen(_)=>{let open:NativeOpen=serde_json::from_value(result).map_err(|_|ErrorCode::NotReady)?;let (_,entry)=native_file.as_ref().ok_or(ErrorCode::NotReady)?;if open.scope!=context.scope||pending_open.as_ref().is_none_or(|p|p.native!=open.read_id)||entry.size.map(|s|s.0)!=Some(open.size)||entry.name!=open.name{return Err(ErrorCode::StaleSession)}let public=reservation.ok_or(ErrorCode::NotReady)?;self.artifact_leases.lock().expect("artifact leases").reads.get_mut(&public).ok_or(ErrorCode::NotReady)?.native=Some(open.read_id);let result=ArtifactsOpenResult{source:source.clone(),read_id:public,name:open.name,size:Uint::new(open.size).map_err(|_|ErrorCode::NotReady)?,sha256:open.sha256,preview_kind:open.preview_kind};result.validate().map_err(|_|ErrorCode::NotReady)?;ResultBody::ArtifactsOpen(result)},
   RequestBody::ArtifactsRead(p)=>{let read:NativeRead=serde_json::from_value(result).map_err(|_|ErrorCode::NotReady)?;let count=base64::engine::general_purpose::STANDARD.decode(&read.data.0).map_err(|_|ErrorCode::NotReady)?.len()as u64;let expected=self.artifact_leases.lock().expect("artifact leases").reads.get(&p.read_id).map(|r|r.size).ok_or(ErrorCode::NotReady)?;if read.scope!=context.scope||Some(read.read_id)!=native_read||read.offset!=p.offset.0||count>p.limit.0||read.offset.checked_add(count).is_none_or(|end|end>expected||read.eof!=(end==expected))||count==0&&!read.eof{return Err(ErrorCode::StaleSession)}let result=ArtifactsReadResult{source:source.clone(),read_id:p.read_id.clone(),offset:p.offset,data:read.data,eof:read.eof};result.validate().map_err(|_|ErrorCode::NotReady)?;ResultBody::ArtifactsRead(result)},
   RequestBody::ArtifactsClose(p)=>{let closed=result.get("closed").and_then(Value::as_bool).ok_or(ErrorCode::NotReady)?;if closed{self.artifact_leases.lock().expect("artifact leases").reads.remove(&p.read_id);}ResultBody::ArtifactsClose(ArtifactsCloseResult{source:source.clone(),read_id:p.read_id.clone(),closed})},_=>return Err(ErrorCode::InvalidRequest)};Ok((context,result,pending_open))
 }
 async fn close_retained_artifact(&self,public:&Uuid,context:&ArtifactContext,owner:&Uuid,native:&Uuid){
  let closed=async{context.original.recheck_identity().map_err(policy)?;let client=Client::connect(&context.original.socket_path).await.map_err(policy)?;if client.evidence!=context.original.evidence{return Err(ErrorCode::StaleSession)}context.original.recheck_identity().map_err(policy)?;let reply=client.request("artifact.close_bound",json!({"scope":context.scope,"owner_id":owner,"read_id":native}),"artifact_close_bound").await.map_err(api_io).map_err(policy)?;if reply.get("result").and_then(|v|v.get("closed")).and_then(Value::as_bool)!=Some(true){return Err(ErrorCode::NotReady)}Ok(())}.await;
  if closed.is_ok(){let mut leases=self.artifact_leases.lock().expect("artifact leases");if leases.reads.get(public).is_some_and(|r|r.native.as_ref()==Some(native)){leases.reads.remove(public);}}
 }
 pub(crate) fn enqueue_artifact(&self,connection:&Uuid,id:Uuid,context:&ArtifactContext,result:ResultBody)->SecurityResult<()>{if matches!(&result,ResultBody::ArtifactsClose(_)){return self.state.response(connection,Response{id,outcome:Ok(result)})}let mut result=Some(result);self.with_artifact_context(context,&mut ||self.state.artifact_response(connection,id.clone(),&context.source,&context.epoch,result.take().ok_or(ErrorCode::NotReady)?))}
}
