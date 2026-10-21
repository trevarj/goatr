// SPDX-License-Identifier: GPL-3.0-or-later
//! Authenticated public-extension channel, never an alternate provider/session.
use super::*;
use serde::{Deserialize,Serialize};
use std::{os::unix::fs::{PermissionsExt,DirBuilderExt},sync::{Weak,OnceLock},time::Instant};
use tokio::{net::{UnixListener,UnixStream,unix::{OwnedReadHalf,OwnedWriteHalf}},io::{AsyncBufReadExt,AsyncWriteExt,BufReader}};
const MODULE:&str=include_str!("omp_skills.ts");
const SOURCE:&str=include_str!("omp_skills_source.json");
const LIMITS:native_json::Limits=native_json::Limits {bytes:262144,depth:16,members:4096};
#[derive(Clone,Serialize,Deserialize)]#[serde(deny_unknown_fields)]
struct PinnedFile {path:String,sha256:String,size:u64}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct SourcePin {version:String,files:Vec<PinnedFile>}
#[derive(Serialize)]#[serde(rename_all="camelCase")]
struct OwnerRecord {version:u32,cap:String,endpoint:String,module_sha:String,package_root:String,files:Vec<PinnedFile>}
#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Hello {version:u32,cap:String,native_id:String,module_sha:String,package_version:String}
#[derive(Clone)]struct LaunchScope {source:Source,pane:Id,terminal:Id,workspace:Id,tab:Id,read:herdr::PaneReadIdentity}
impl LaunchScope {fn matches(&self,original:&Connection)->bool {
 if !self.source.same_physical_identity(&original.source)||original.key.terminal_id!=self.terminal{return false}
 original.source.snapshot.pane(&self.pane).ok().is_some_and(|pane|pane.terminal_id==self.terminal&&pane.workspace_id==self.workspace&&pane.tab_id==self.tab&&pane.read_identity.as_ref().is_some_and(|read|read.herdr_incarnation==self.read.herdr_incarnation&&read.runtime_epoch==self.read.runtime_epoch))
}}
#[derive(Clone)]struct Launch {cap:String,expires:Instant,used:bool,package:PathBuf,scope:LaunchScope}
struct Io {read:BufReader<OwnedReadHalf>,write:OwnedWriteHalf}
pub(super) struct Channel {
 runtime:Weak<Runtime>,process:ProcessEvidence,native:Id,launch:LaunchScope,alive:AtomicBool,fd:OwnedFd,io:tokio::sync::Mutex<Io>,
 generation:Mutex<Uuid>,catalogue:Mutex<Option<Uuid>>,permission:Mutex<Option<u64>>,bound:Mutex<Option<(Target,Uuid,Weak<Connection>)>>,expires:Instant,
}
impl Channel {
 fn close(&self){self.alive.store(false,Ordering::Release);unsafe{libc::shutdown(self.fd.as_raw_fd(),libc::SHUT_RDWR);}}
 fn current(&self)->SecurityResult<()> {
  if !self.alive.load(Ordering::Acquire)||Instant::now()>=self.expires||herdr::process_evidence(self.process.pid).map_err(policy)?!=self.process{return Err(ErrorCode::StaleSession)}
  let mut byte=0u8;let n=unsafe{libc::recv(self.fd.as_raw_fd(),(&mut byte as *mut u8).cast(),1,libc::MSG_PEEK|libc::MSG_DONTWAIT)};
  if n==0||n<0&&io::Error::last_os_error().kind()!=io::ErrorKind::WouldBlock{return Err(ErrorCode::StaleSession)}Ok(())
 }
 async fn call(self:&Arc<Self>,params:Value,deadline:tokio::time::Instant)->SecurityResult<Value>{self.call_owned(params,deadline,None).await}
 async fn authorized_call(self:&Arc<Self>,params:Value,deadline:tokio::time::Instant,context:&SkillsReadContext)->SecurityResult<Value>{
  self.call_owned(params,deadline,Some(context.clone())).await
 }
 async fn call_owned(self:&Arc<Self>,mut params:Value,deadline:tokio::time::Instant,authority:Option<SkillsReadContext>)->SecurityResult<Value>{
  self.current()?;let id=Uuid::random();params["id"]=json!(id);let mut bytes=serde_json::to_vec(&params).map_err(policy)?;bytes.push(b'\n');if bytes.len()>LIMITS.bytes{return Err(ErrorCode::ResourceLimit)}
  let runtime=self.runtime.upgrade().ok_or(ErrorCode::NotReady)?;let original_runtime=Arc::downgrade(&runtime);let channel=self.clone();let(send,receive)=oneshot::channel();
  // Runtime owns an admitted item's complete I/O, including draining its exact
  // reply after a phone waiter disappears. Queued canceled waiters write nothing.
  runtime.jobs.lock().spawn(async move{
   let mut started=false;let mut drained=false;let mut refused=false;
   let result=tokio::time::timeout_at(deadline,async{
    let mut io=channel.io.lock().await;
    if send.is_closed(){return Err(ErrorCode::NotReady)}channel.current()?;
    // Actual native grant admission follows the channel ownership wait.
    if let Some(context)=&authority{let runtime=original_runtime.upgrade().ok_or(ErrorCode::NotReady)?;context.scope.authorize(&runtime.topology).await?;context.validate_current(&runtime)?;}
    if send.is_closed(){return Err(ErrorCode::NotReady)}channel.current()?;
    started=true;io.write.write_all(&bytes).await.map_err(policy)?;
    let response=line(&mut io.read).await?;if response.get("id").and_then(Value::as_str)!=Some(id.0.as_str()){return Err(ErrorCode::StaleSession)}
    drained=true;
    if let Some(error)=response.get("error"){
     if !matches!(error.as_str(),Some("unavailable"|"busy"))||response.as_object().is_none_or(|object|object.len()!=2){drained=false;return Err(ErrorCode::StaleSession)}refused=true;return Err(ErrorCode::NotReady)
    }
    channel.current()?;if response.get("nativeId").and_then(Value::as_str).is_some_and(|native|native!=channel.native.0){drained=false;return Err(ErrorCode::StaleSession)}
    // Revoke/config changes may precede the next inventory observation.
    if let Some(context)=&authority{let runtime=original_runtime.upgrade().ok_or(ErrorCode::NotReady)?;context.scope.authorize(&runtime.topology).await?;context.validate_current(&runtime)?;}
    Ok(response)
   }).await.map_err(|_|ErrorCode::Timeout).and_then(|value|value);
   if result.is_err()&&started&&!drained&&!refused{channel.close()}
   let _=send.send(result);
  });
  receive.await.map_err(|_|ErrorCode::NotReady)?
 }

}
async fn line(reader:&mut BufReader<OwnedReadHalf>)->SecurityResult<Value>{
 let mut bytes=Vec::new();loop{
  let available=reader.fill_buf().await.map_err(policy)?;if available.is_empty(){return Err(ErrorCode::NotReady)}let end=available.iter().position(|b|*b==b'\n');let n=end.map_or(available.len(),|n|n+1);
  if bytes.len().checked_add(n).is_none_or(|n|n>LIMITS.bytes){return Err(ErrorCode::ResourceLimit)}bytes.extend_from_slice(&available[..n]);reader.consume(n);if end.is_some(){break}
 }native_json::parse(&bytes,LIMITS).map_err(policy)
}
fn ct_eq(a:&str,b:&str)->bool {if a.len()!=b.len(){return false}a.bytes().zip(b.bytes()).fold(0u8,|n,(a,b)|n|(a^b))==0}
fn pin_files(package:&Path,process:Option<&ProcessEvidence>)->io::Result<Vec<PinnedFile>>{
 let pin:SourcePin=serde_json::from_str(SOURCE).map_err(|_|invalid())?;
 if pin.version!="18.8.7"||pin.files.len()>16{return Err(invalid())}
 if let Some(process)=process{if herdr::process_evidence(process.pid)?!=*process{return Err(invalid())}}
 for expected in &pin.files{
  let path=package.join(&expected.path);let object=OpenOptions::new().read(true).custom_flags(libc::O_PATH|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&path)?;let before=object.metadata()?;
  if !before.is_file()||before.len()!=expected.size||before.len()>32*1024*1024{return Err(invalid())}
  // Public code provenance only. No skill document is read outside OMP.
  if let Some(process)=process{if herdr::process_object(process,&path)?!=(before.dev(),before.ino()){return Err(invalid())}}
  let mut file=File::open(format!("/proc/self/fd/{}",object.as_raw_fd()))?;let mut total=0u64;let mut digest=sha2::Sha256::new();let mut chunk=[0u8;65536];
  use sha2::Digest;
  loop{let count=file.read(&mut chunk)?;if count==0{break}total=total.checked_add(count as u64).ok_or_else(invalid)?;if total>expected.size{return Err(invalid())}digest.update(&chunk[..count]);}
  let after=file.metadata()?;
  if total!=expected.size||hex(&digest.finalize())!=expected.sha256||before.dev()!=after.dev()||before.ino()!=after.ino()||before.len()!=after.len()||before.mtime()!=after.mtime()||before.mtime_nsec()!=after.mtime_nsec()||before.ctime()!=after.ctime()||before.ctime_nsec()!=after.ctime_nsec(){return Err(invalid())}
  if let Some(process)=process{if herdr::process_object(process,&path)?!=(before.dev(),before.ino()){return Err(invalid())}}
 }
 if let Some(process)=process{if herdr::process_evidence(process.pid)?!=*process{return Err(invalid())}}
 Ok(pin.files)
}
fn exclusive(path:&Path,bytes:&[u8])->io::Result<()> {let mut f=OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;f.write_all(bytes)?;f.sync_all()}

pub(super) struct Manager {
 config:Config,ready:AtomicBool,launches:Mutex<HashMap<Uuid,Launch>>,channels:Mutex<Vec<Arc<Channel>>>,
 runtime:OnceLock<Weak<Runtime>>,previews:Mutex<HashMap<Uuid,Preview>>,
}
impl Manager {
 pub(super) fn new(config:&Config)->Arc<Self>{Arc::new(Self {config:config.clone(),ready:AtomicBool::new(false),launches:Mutex::new(HashMap::new()),channels:Mutex::new(Vec::new()),runtime:OnceLock::new(),previews:Mutex::new(HashMap::new())})}
 fn base(&self)->PathBuf{self.config.runtime_dir.join("skills")}
 fn endpoint(&self)->PathBuf{self.base().join("channel")}
 pub(super) fn prepare(&self,source:Source,pane:&Id)->io::Result<Vec<String>>{
  let native=source.snapshot.pane(pane).map_err(|_|invalid())?;let scope=LaunchScope{source:source.clone(),pane:native.pane_id.clone(),terminal:native.terminal_id.clone(),workspace:native.workspace_id.clone(),tab:native.tab_id.clone(),read:native.read_identity.clone().ok_or_else(invalid)?};
  if !self.ready.load(Ordering::Acquire){return Err(invalid())}let package=self.config.omp_skills_package_root.as_ref().ok_or_else(invalid)?;let files=pin_files(package,None)?;
  let mut launches=self.launches.lock();if launches.len()>=32{return Err(io::Error::new(io::ErrorKind::OutOfMemory,"OMP Skills launch resource limit"))}let id=Uuid::random();let cap=URL_SAFE_NO_PAD.encode(pairing::random32()?);
  // Consume a finite slot before any exclusive file allocation. Failed prepares
  // cannot create an unbounded succession of abandoned private launch objects.
  launches.insert(id.clone(),Launch{cap:cap.clone(),expires:Instant::now(),used:true,package:package.clone(),scope:scope.clone()});
  let dir=self.base().join(&id.0);fs::DirBuilder::new().mode(0o700).create(&dir)?;pairing::private_dir(&dir,false)?;
  let agent=self.config.runtime_dir_agent.join("skills").join(&id.0);let module=dir.join("extension.ts");exclusive(&module,MODULE.as_bytes())?;
  let endpoint=self.config.runtime_dir_agent.join("skills/channel");let endpoint=endpoint.to_str().filter(|s|s.len()<=107).ok_or_else(invalid)?.to_owned();
  let owner=OwnerRecord {version:1,cap:cap.clone(),endpoint,module_sha:hex(&sha256(MODULE.as_bytes())),package_root:package.to_str().ok_or_else(invalid)?.to_owned(),files};exclusive(&dir.join("owner.json"),&serde_json::to_vec(&owner).map_err(|_|invalid())?)?;
  launches.insert(id,Launch {cap,expires:Instant::now()+Duration::from_secs(30),used:false,package:package.clone(),scope});
  Ok(vec!["--extension".into(),agent.join("extension.ts").to_str().ok_or_else(invalid)?.to_owned()])
 }
 pub(super) fn channel(&self,original:&Arc<Connection>)->Option<Arc<Channel>>{
  let channel=self.channels.lock().iter().find(|channel|channel.process==original.owner&&channel.native==original.key.native_conversation_id&&channel.launch.matches(original)&&channel.current().is_ok()).cloned()?;
  {let mut bound=channel.bound.lock();match bound.as_ref(){
   Some((target,epoch,owner))=>{if target!=&original.target||epoch!=&original.epoch||!owner.upgrade().is_some_and(|owner|Arc::ptr_eq(&owner,original)){channel.close();return None}},
   None=>*bound=Some((original.target.clone(),original.epoch.clone(),Arc::downgrade(original))),
  }}Some(channel)
 }
 fn changed(&self){if let Some(runtime)=self.runtime.get().and_then(Weak::upgrade){runtime.refresh_skills_capabilities();}}
 pub(super) async fn run(self:Arc<Self>,runtime:Weak<Runtime>,mut stop:watch::Receiver<bool>){
  let _=self.runtime.set(runtime);if self.config.omp_skills_package_root.is_none(){return}if pairing::private_dir(&self.base(),true).is_err(){return}let endpoint=self.endpoint();if endpoint.as_os_str().len()>107{return}
  let Ok(listener)=UnixListener::bind(&endpoint)else{return};if fs::set_permissions(&endpoint,fs::Permissions::from_mode(0o600)).is_err(){return}self.ready.store(true,Ordering::Release);
  let permits=Arc::new(tokio::sync::Semaphore::new(8));let mut tasks=JoinSet::new();
  loop{tokio::select!{biased;_=stop.changed()=>break,_=tasks.join_next(),if !tasks.is_empty()=>{},accepted=listener.accept()=>{
   let Ok((stream,_))=accepted else{break};let Ok(permit)=permits.clone().try_acquire_owned()else{drop(stream);continue};let owner=self.clone();tasks.spawn(async move{let _permit=permit;let _=tokio::time::timeout(Duration::from_secs(30),owner.register(stream)).await;});
  }}}
  self.ready.store(false,Ordering::Release);for channel in self.channels.lock().iter(){channel.close()}self.changed();tasks.abort_all();while tasks.join_next().await.is_some(){}
 }
 async fn register(self:Arc<Self>,stream:UnixStream)->SecurityResult<()>{
  let cred=stream.peer_cred().map_err(policy)?;if cred.uid()!=pairing::uid(){return Err(ErrorCode::Unauthorized)}let pid=cred.pid().and_then(|pid|u32::try_from(pid).ok()).ok_or(ErrorCode::Unauthorized)?;let process=herdr::process_evidence(pid).map_err(policy)?;
  let fd=stream.as_fd().try_clone_to_owned().map_err(policy)?;let(read,write)=stream.into_split();let mut read=BufReader::new(read);let hello:Hello=serde_json::from_value(line(&mut read).await?).map_err(policy)?;
  if hello.version!=1||hello.module_sha!=hex(&sha256(MODULE.as_bytes()))||hello.package_version!="18.8.7"{return Err(ErrorCode::Unauthorized)}let native=Id::new(hello.native_id).map_err(|_|ErrorCode::Unauthorized)?;
  let launch={let mut launches=self.launches.lock();let matching=launches.values_mut().find(|launch|!launch.used&&Instant::now()<launch.expires&&ct_eq(&launch.cap,&hello.cap)).ok_or(ErrorCode::Unauthorized)?;matching.used=true;matching.clone()};
  pin_files(&launch.package,Some(&process)).map_err(policy)?;
  if herdr::process_evidence(pid).map_err(policy)?!=process{return Err(ErrorCode::StaleSession)}
  let mut io=Io{read,write};io.write.write_all(&serde_json::to_vec(&json!({"ready":true,"nativeId":native})).map_err(policy)?).await.map_err(policy)?;io.write.write_all(b"\n").await.map_err(policy)?;
  let channel=Arc::new(Channel {runtime:self.runtime.get().cloned().unwrap_or_default(),process,native,launch:launch.scope,alive:AtomicBool::new(true),fd,io:tokio::sync::Mutex::new(io),generation:Mutex::new(Uuid::random()),catalogue:Mutex::new(None),permission:Mutex::new(None),bound:Mutex::new(None),expires:Instant::now()+Duration::from_secs(3600)});
  {let mut channels=self.channels.lock();if channels.len()>=32||channels.iter().any(|old|old.process==channel.process){channel.close();return Err(ErrorCode::ResourceLimit)}channels.push(channel)}self.changed();Ok(())
 }
}

#[derive(Clone)]pub(crate) struct SkillsReadContext {original:Arc<Connection>,channel:Arc<Channel>,scope:herdr::SkillsScopeContext,generation:Uuid}
impl SkillsReadContext {
 fn validate_current(&self,runtime:&Runtime)->SecurityResult<()> {
  // Removal from the original backend owner is identity loss even while its
  // ordered durable retirement has not yet reached topology/State publication.
  let connections=runtime.connections.lock();
  if !connections.get(&self.original.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&self.original)){return Err(ErrorCode::StaleSession)}
  self.validate(runtime)
 }
 fn validate(&self,runtime:&Runtime)->SecurityResult<()> {
  self.original.validate(&runtime.security.state,&runtime.topology)?;self.channel.current()?;
  if self.channel.process!=self.original.owner||self.channel.native!=self.original.key.native_conversation_id||*self.channel.generation.lock()!=self.generation||*self.channel.permission.lock()!=Some(self.scope.permission_epoch()){return Err(ErrorCode::StaleSession)}
  runtime.topology.validate_skills_scope(&self.scope)
 }
}
struct Preview {peer:Uuid,context:SkillsReadContext,document:Uuid,native_catalogue:Uuid,expires:Instant,size:Option<u64>,sha256:String}
pub(crate) struct PendingPreview {manager:Arc<Manager>,id:Uuid,published:bool,cancel_channel:bool}
impl PendingPreview {pub(crate) fn published(&mut self){self.published=true}}
impl Drop for PendingPreview {fn drop(&mut self){
 if self.published{return}
 let channel=self.manager.previews.lock().get(&self.id).map(|preview|preview.context.channel.clone());let Some(channel)=channel else{return};
 if !self.cancel_channel{self.manager.previews.lock().remove(&self.id);return}
 let manager=self.manager.clone();let id=self.id.clone();
 if let Some(runtime)=manager.runtime.get().and_then(Weak::upgrade){
  // The retained quota slot belongs to this exact read until cleanup finishes.
  // Reuse the existing runtime job owner, never reopen or adopt another source.
  runtime.jobs.lock().spawn(async move{
   let reply=channel.call(json!({"method":"close","readId":id}),tokio::time::Instant::now()+Duration::from_secs(5)).await;
   let proved=reply.as_ref().ok().is_some_and(|reply|reply.as_object().is_some_and(|object|object.len()==2)&&reply.get("closed").and_then(Value::as_bool).is_some());
   if !proved{channel.close()}manager.previews.lock().remove(&id);manager.changed();
  });
 }else{channel.close();manager.previews.lock().remove(&id);}
}}
#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
struct NativeEntry {name:String,description:String,#[serde(default)]document_ref:Option<Uuid>}
#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
struct NativeList {id:Uuid,native_id:Id,catalogue:Uuid,entries:Vec<NativeEntry>}
#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
struct NativeOpen {id:Uuid,native_id:Id,catalogue:Uuid,document_ref:Uuid,read_id:Uuid,sha256:String,size:u64}
#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
struct NativeRead {id:Uuid,native_id:Id,catalogue:Uuid,read_id:Uuid,offset:u64,data:String,eof:bool}
impl Runtime {
 pub(super) fn refresh_skills_capabilities(&self){
  self.skills.previews.lock().retain(|_,preview|preview.size.is_none()||Instant::now()<preview.expires);
  let connections=self.connections.lock();
  for original in connections.values(){
   let channel=self.skills.channel(original);let generation=channel.and_then(|channel|{
    let current=self.topology.source_readiness(&original.key.server_id).ok().flatten()?;
    if current.recheck_readiness().ok()!=Some(true){return None}
    let epoch=current.snapshot.omp_skills_permission_epoch.filter(|epoch|*epoch>0);
    if !current.snapshot.omp_skills_preview_bound||epoch.is_none(){channel.close();return None}
    {let mut pinned=channel.permission.lock();if pinned.is_some()&&*pinned!=epoch{channel.close();return None}*pinned=epoch;}
    if original.validate(&self.security.state,&self.topology).is_err()||self.topology.skills_scope_context(&original.target,&original.owner).is_err(){return None}
    Some(channel.generation.lock().clone())
   });
   let _=self.security.state.skills_generation(&original.target,&original.epoch,generation);
  }
 }
 fn skills_context(&self,target:&Target)->SecurityResult<SkillsReadContext>{
  let original=self.connections.lock().get(&target.session_id).filter(|original|original.target==*target).cloned().ok_or(ErrorCode::NotReady)?;
  original.validate(&self.security.state,&self.topology)?;let channel=self.skills.channel(&original).ok_or(ErrorCode::NotReady)?;let scope=self.topology.skills_scope_context(target,&original.owner)?;
  let generation=channel.generation.lock().clone();let context=SkillsReadContext{original,channel,scope,generation};context.validate(self)?;Ok(context)
 }
 pub(crate) async fn skills_list(&self,target:&Target)->SecurityResult<(SkillsReadContext,SkillsListResult)>{
  let mut context=self.skills_context(target)?;context.scope.authorize(&self.topology).await?;context.validate(self)?;
  let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
  let native:NativeList=serde_json::from_value(context.channel.authorized_call(json!({"method":"list"}),deadline,&context).await?).map_err(policy)?;context.validate(self)?;
  if native.native_id!=context.original.key.native_conversation_id||native.entries.len()>128{return Err(ErrorCode::StaleSession)}
  let mut names=HashSet::new();let mut skills=Vec::new();
  for row in native.entries{
   if !row.name.starts_with("skill:")||!names.insert(row.name.clone())||row.name.chars().any(char::is_control){return Err(ErrorCode::NotReady)}
   skills.push(SkillEntry{name:row.name.strip_prefix("skill:").ok_or(ErrorCode::NotReady)?.to_owned(),description:row.description,path:None,scope:"enabled".into(),enabled:true,display_name:None,short_description:None,command_name:Some(row.name),document_ref:row.document_ref});
  }
  // This explicit fresh catalogue read may advance only its catalogue generation,
  // never original provider/Target/process/native scope authority.
  {let mut catalogue=context.channel.catalogue.lock();if catalogue.as_ref().is_some_and(|old|old!=&native.catalogue){*context.channel.generation.lock()=Uuid::random()}*catalogue=Some(native.catalogue)}
  context.generation=context.channel.generation.lock().clone();self.refresh_skills_capabilities();
  let result=SkillsListResult{target:target.clone(),catalogue_generation:context.generation.clone(),skills,errors:Vec::new(),provider:Provider::Omp};result.validate().map_err(policy)?;context.validate(self)?;Ok((context,result))
 }
 pub(crate) fn enqueue_skills_list(&self,connection:&Uuid,response:Uuid,context:&SkillsReadContext,result:SkillsListResult)->SecurityResult<()>{
  let connections=self.connections.lock();if !connections.get(&context.original.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&context.original)){return Err(ErrorCode::StaleSession)}context.validate(self)?;
  self.topology.with_skills_scope(&context.scope,||self.security.state.skills_list_response(connection,&context.original.target,&context.original.epoch,response,result))
 }
 pub(crate) async fn skills_preview_open(&self,peer:&Uuid,params:&SkillsPreviewOpenParams)->SecurityResult<(SkillsReadContext,SkillsPreviewOpenResult,PendingPreview)>{
  let context=self.skills_context(&params.target)?;if context.generation!=params.catalogue_generation{return Err(ErrorCode::StaleSession)}context.scope.authorize(&self.topology).await?;context.validate(self)?;
  let native_catalogue=context.channel.catalogue.lock().clone().ok_or(ErrorCode::NotReady)?;let id=Uuid::random();
  {let mut previews=self.skills.previews.lock();previews.retain(|_,preview|preview.size.is_none()||Instant::now()<preview.expires);if previews.len()>=16||previews.values().any(|preview|preview.peer==*peer){return Err(ErrorCode::ResourceLimit)}
   previews.insert(id.clone(),Preview{peer:peer.clone(),context:context.clone(),document:params.document_ref.clone(),native_catalogue:native_catalogue.clone(),expires:Instant::now()+Duration::from_secs(120),size:None,sha256:String::new()});}
  let mut pending=PendingPreview {manager:self.skills.clone(),id:id.clone(),published:false,cancel_channel:true};let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
  let response=match context.channel.authorized_call(json!({"method":"open","catalogue":native_catalogue,"documentRef":params.document_ref,"readId":id}),deadline,&context).await{
   Ok(response)=>response,Err(error)=>return Err(error)
  };
  let opened:NativeOpen=serde_json::from_value(response).map_err(policy)?;context.validate(self)?;
  if opened.native_id!=context.original.key.native_conversation_id||opened.catalogue!=native_catalogue||opened.document_ref!=params.document_ref||opened.read_id!=id||opened.size>1048576||opened.sha256.len()!=64||!opened.sha256.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){return Err(ErrorCode::StaleSession)}
  let result=SkillsPreviewOpenResult{target:params.target.clone(),catalogue_generation:context.generation.clone(),document_ref:params.document_ref.clone(),read_id:id.clone(),size:Uint(opened.size),sha256:opened.sha256.clone()};result.validate().map_err(policy)?;
  {let mut previews=self.skills.previews.lock();let preview=previews.get_mut(&id).ok_or(ErrorCode::NotReady)?;if Instant::now()>=preview.expires{return Err(ErrorCode::NotReady)}preview.sha256=opened.sha256;preview.size=Some(opened.size);}
  Ok((context,result,pending))
 }
 pub(crate) async fn skills_preview_read(&self,peer:&Uuid,params:&SkillsPreviewReadParams)->SecurityResult<(SkillsReadContext,SkillsPreviewReadResult,PendingPreview)>{
  params.validate().map_err(policy)?;let(context,catalogue,size)={let previews=self.skills.previews.lock();let preview=previews.get(&params.read_id).filter(|preview|preview.peer==*peer&&preview.context.original.target==params.target&&preview.context.generation==params.catalogue_generation&&Instant::now()<preview.expires).ok_or(ErrorCode::NotReady)?;(preview.context.clone(),preview.native_catalogue.clone(),preview.size.ok_or(ErrorCode::NotReady)?)};
  let pending=PendingPreview{manager:self.skills.clone(),id:params.read_id.clone(),published:false,cancel_channel:true};
  context.validate(self)?;
  let chunk:NativeRead=serde_json::from_value(context.channel.authorized_call(json!({"method":"read","catalogue":catalogue,"readId":params.read_id,"offset":params.offset.0,"limit":params.limit.0}),tokio::time::Instant::now()+Duration::from_secs(5),&context).await?).map_err(policy)?;context.validate(self)?;
  let data=base64::engine::general_purpose::STANDARD.decode(&chunk.data).map_err(policy)?;
  let end=chunk.offset.checked_add(data.len()as u64).ok_or(ErrorCode::NotReady)?;
  if chunk.native_id!=context.original.key.native_conversation_id||chunk.catalogue!=catalogue||chunk.read_id!=params.read_id||chunk.offset!=params.offset.0||data.len()>params.limit.0 as usize||data.is_empty()&&!chunk.eof||end>size||chunk.eof!=(end==size)||base64::engine::general_purpose::STANDARD.encode(&data)!=chunk.data{return Err(ErrorCode::StaleSession)}
  let result=SkillsPreviewReadResult{target:params.target.clone(),catalogue_generation:params.catalogue_generation.clone(),read_id:params.read_id.clone(),offset:params.offset,data:Base64Bytes(chunk.data),eof:chunk.eof};result.validate().map_err(policy)?;Ok((context,result,pending))
 }
 pub(crate) async fn skills_preview_close(&self,peer:&Uuid,params:&SkillsPreviewCloseParams)->SecurityResult<SkillsPreviewCloseResult>{
  let owned={let previews=self.skills.previews.lock();previews.get(&params.read_id).filter(|preview|preview.peer==*peer&&preview.context.original.target==params.target&&preview.context.generation==params.catalogue_generation).map(|preview|preview.context.channel.clone())};
  let Some(channel)=owned else{return Ok(SkillsPreviewCloseResult{closed:false})};
  // Only exact owned memory cleanup may outlive read readiness. No file read,
  // replacement source lookup or reopening is permitted on this path.
  let reply=channel.call(json!({"method":"close","readId":params.read_id}),tokio::time::Instant::now()+Duration::from_secs(5)).await?;
  if reply.as_object().is_none_or(|object|object.len()!=2)||reply.get("closed").and_then(Value::as_bool).is_none(){return Err(ErrorCode::NotReady)}
  self.skills.previews.lock().remove(&params.read_id);Ok(SkillsPreviewCloseResult{closed:true})
 }
 pub(crate) fn enqueue_skills_preview(&self,connection:&Uuid,id:Uuid,context:&SkillsReadContext,result:ResultBody)->SecurityResult<()>{
  let connections=self.connections.lock();if !connections.get(&context.original.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&context.original)){return Err(ErrorCode::StaleSession)}context.validate(self)?;
  self.topology.with_skills_scope(&context.scope,||self.security.state.skills_preview_response(connection,&context.original.target,&context.original.epoch,&context.generation,id,result))
 }
}
