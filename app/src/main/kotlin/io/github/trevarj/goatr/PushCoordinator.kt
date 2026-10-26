// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Application
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/** One registration owner; cold callbacks open private storage, never an Iroh claim/FGS. */
@Suppress("DEPRECATION")
internal class PushCoordinator(private val app:Application,private val runtime:GoatrRuntime){
 private data class Confirmation(val generation:Uuid,val hello:HelloResult)
 private val verifiedGenerations=ConcurrentHashMap<String,Confirmation>()
 private val failedGenerations=ConcurrentHashMap<String,MutableSet<Uuid>>()
 private val synced=ConcurrentHashMap<String,HelloResult>();private val gates=ConcurrentHashMap<String,Mutex>();private val jobs=ConcurrentHashMap<String,Job>();private val dirty=ConcurrentHashMap.newKeySet<String>();private val hydrated=ConcurrentHashMap.newKeySet<String>()
 private val mutable=MutableStateFlow<Map<String,String>>(emptyMap());val health:StateFlow<Map<String,String>> = mutable
 private val random=SecureRandom()
 init{
  runtime.cleanupScope.launch{runtime.connection.state.collect{view->view.hello.forEach{(host,hello)->if(view.inventoryReady[host]!=true)synced.remove(host,hello)else if(synced[host]!==hello)schedule(host)}}}
  runtime.cleanupScope.launch{runtime.store.state.collect{view->
   gates.keys.filter{host->runtime.store.state.value.hosts.none{it.hostId==host}}.forEach{host->synchronized(jobs){if(runtime.store.state.value.hosts.none{it.hostId==host}){jobs.remove(host)?.cancel();dirty.remove(host);gates.remove(host);verifiedGenerations.remove(host);failedGenerations.remove(host);synced.remove(host);hydrated.remove(host);mutable.update{it-host}}}}
   view.hosts.filter{runtime.deliveryPreferences.mode(it.hostId)==DeliveryMode.Ntfy&&hydrated.add(it.hostId)}.forEach{metadata->runtime.cleanupScope.launch{
    try{gate(metadata.hostId).withLock{load(metadata.hostId)?.let{record->if(!verifiedGenerations.containsKey(metadata.hostId))mark(metadata.hostId,if(record.repair||!ntfyInstalled())"Repair required"else if(record.acknowledged)"Previously acknowledged · reconnect to verify"else"Verification pending")}}}catch(_:Exception){if(runtime.store.state.value.hosts.any{it.hostId==metadata.hostId&&it.deviceId==metadata.deviceId})mark(metadata.hostId,"Repair required")}
   }}
  }}
 }
 fun ntfyInstalled():Boolean=UnifiedPush.getDistributors(app).contains(NTFY)
 private fun gate(host:String)=gates.getOrPut(host){Mutex()}
 private fun instance(host:String)="goatr.${app.packageName}.$host"
 private fun host(connectorInstance:String):String?=runtime.store.state.value.hosts.singleOrNull{instance(it.hostId)==connectorInstance}?.hostId
 private fun mark(host:String,value:String){
  val exists=runtime.store.state.value.hosts.any{it.hostId==host};val before=mutable.value
  mutable.update{if(exists)it+(host to value)else it-host}
  if(before!=mutable.value)runtime.deliveryPreferences.pushHealthChanged()
 }
 private fun original(host:String,hello:HelloResult):Boolean=runtime.connection.state.value.hello[host]===hello&&runtime.connection.state.value.inventoryReady[host]==true&&runtime.store.state.value.hosts.any{it.hostId==host&&it.deviceId==hello.deviceId}
 private fun confirmed(host:String,record:LocalPushRegistration,hello:HelloResult){
  val before=verifiedGenerations[host]
  val installed=verifiedGenerations.compute(host){_,previous->
   if(!original(host,hello)||record.device!=hello.deviceId||!record.probeReceived||!record.acknowledged||record.repair||record.unregister||failedGenerations[host]?.contains(record.generation)==true)previous
   else previous?.takeIf{it.generation==record.generation&&it.hello===hello} ?: Confirmation(record.generation,hello)
  }
  if(installed!==before)runtime.deliveryPreferences.pushHealthChanged()
  if(installed!=null&&installed.hello===hello&&installed.generation==record.generation&&original(host,hello))mark(host,"Registration verified")
 }
 private fun unconfirm(record:LocalPushRegistration,hello:HelloResult?=null){
  val prior=verifiedGenerations[record.host] ?: return
  if(prior.generation==record.generation&&prior.hello.deviceId==record.device&&(hello==null||prior.hello===hello)&&verifiedGenerations.remove(record.host,prior))runtime.deliveryPreferences.pushHealthChanged()
 }
 internal fun label(host:String):String {
  val confirmed=verifiedGenerations[host] ?: return mutable.value[host] ?: "Not verified"
  if(failedGenerations[host]?.contains(confirmed.generation)==true)return "Repair required"
  val hello=runtime.connection.state.value.hello[host]
  return if(hello===confirmed.hello)"Registration verified"else if(hello==null)"Last verified · offline; current delivery unknown"else"Previously verified · checking current connection"
 }
 internal fun verified(host:String):Boolean {
  val confirmed=verifiedGenerations[host] ?: return false
  val hello=runtime.connection.state.value.hello[host]
  return runtime.deliveryPreferences.mode(host)==DeliveryMode.Ntfy&&failedGenerations[host]?.contains(confirmed.generation)!=true&&(hello==null||hello===confirmed.hello)
 }
 private suspend fun load(host:String):LocalPushRegistration?{
  val stored=runtime.store.encryptedRegistration(host) ?: return null
  val plain=runtime.secrets.pushOpen(host,stored.first,stored.second)
  return try{Protocol.strictJson.decodeFromString<LocalPushRegistration>(plain.toString(Charsets.UTF_8)).also{
   if(it.version!=1||it.createdAt<0||it.firstHostRegistrationAt?.let{at->at<0}==true||it.acknowledged&&!it.probeReceived||it.host!=host||it.generation!=stored.first||runtime.store.state.value.hosts.singleOrNull{row->row.hostId==host}?.deviceId!=it.device)throw ProtectedKeyUnavailable()
   it.generation.validateWire();it.probe.validateWire();Key32(it.key)
  }}finally{plain.fill(0)}
 }
 private suspend fun save(value:LocalPushRegistration,previousGeneration:Uuid?=value.generation,hello:HelloResult?=null){
  fun admitted()=runtime.store.state.value.hosts.any{it.hostId==value.host&&it.deviceId==value.device}&&(hello==null||original(value.host,hello))
  if(!admitted())throw StorePolicy(ErrorCode.NotReady)
  val plain=Protocol.strictJson.encodeToString(value).toByteArray(Charsets.UTF_8)
  try{runtime.store.encryptedRegistration(value.host,value.generation,value.device,previousGeneration,runtime.secrets.pushSeal(value.host,value.generation,plain),::admitted)}finally{plain.fill(0)}
  failedGenerations[value.host]?.removeIf{it!=value.generation}
 }
 suspend fun selectNtfy(host:String,repair:Boolean,active:()->Boolean){
  if(!active())throw StorePolicy(ErrorCode.NotReady)
  runtime.store.open();gate(host).withLock{
   if(!active())throw StorePolicy(ErrorCode.NotReady)
   val metadata=runtime.store.state.value.hosts.singleOrNull{it.hostId==host} ?: throw StorePolicy(ErrorCode.Unauthorized)
   val device=metadata.deviceId ?: throw StorePolicy(ErrorCode.NotReady)
   if(!ntfyInstalled())throw StorePolicy(ErrorCode.NotReady)
   if(repair){verifiedGenerations.remove(host);mark(host,"Repairing registration")}
   var registration=load(host)
   val previousGeneration=registration?.generation
   if(registration==null||registration.unregister||repair){
    verifiedGenerations.remove(host);mark(host,"Preparing registration")
    val alias=SecretStore.hostAlias(host,"push");val retained=runtime.store.ensurePushAlias(host,device,alias);runtime.secrets.material(alias,!retained).fill(0)
    val key=ByteArray(32).also(random::nextBytes)
    registration=try{LocalPushRegistration(1,host,device,Uuid.random(),Uuid.random(),PushHints.encode(key),System.currentTimeMillis())}finally{key.fill(0)}
    save(registration,previousGeneration)
   }
   if(!active())throw StorePolicy(ErrorCode.NotReady)
   runtime.deliveryPreferences.setMode(host,DeliveryMode.Ntfy)
   // Explicit selection only; never consult default/current distributor convenience APIs.
   UnifiedPush.saveDistributor(app,NTFY);UnifiedPush.register(app,instance(host),"Goatr paired desktop")
   if(!verifiedGenerations.containsKey(host))mark(host,if(registration.endpoint==null)"Waiting for endpoint"else if(registration.acknowledged)"Previously acknowledged · verifying current host"else"Waiting for probe")
  };schedule(host)
 }
 suspend fun disable(host:String){
  verifiedGenerations.remove(host)
  gate(host).withLock{load(host)?.let{save(it.copy(unregister=true,acknowledged=false))};mark(host,"Disabled")}
  try{UnifiedPush.unregister(app,instance(host))}catch(_:Exception){};schedule(host)
 }
 suspend fun selectPersistent(host:String){
  verifiedGenerations.remove(host)
  gate(host).withLock{load(host)?.let{save(it.copy(unregister=true,acknowledged=false))};runtime.deliveryPreferences.setMode(host,DeliveryMode.Persistent);mark(host,"Socket selected")}
  try{UnifiedPush.unregister(app,instance(host))}catch(_:Exception){}
  schedule(host)
 }
 suspend fun forget(host:String){verifiedGenerations.remove(host);failedGenerations.remove(host);synced.remove(host);hydrated.remove(host);synchronized(jobs){jobs.remove(host)?.cancel();dirty.remove(host)};try{UnifiedPush.unregister(app,instance(host))}catch(_:Exception){};gates.remove(host);mutable.update{it-host}}
 /** Receiver's bounded goAsync worker invokes these synchronously before SDK ACK. */
 private fun callback(instance:String,operation:suspend (String,LocalPushRegistration)->Unit){
  var capturedDevice:Uuid?=null
  var captured:LocalPushRegistration?=null
  try{runBlocking(Dispatchers.IO){withTimeout(2000){runtime.store.open();val host=host(instance) ?: return@withTimeout
   capturedDevice=runtime.store.state.value.hosts.singleOrNull{it.hostId==host}?.deviceId
   gate(host).withLock{val record=load(host) ?: return@withLock
    captured=record
    if(record.device!=capturedDevice||runtime.deliveryPreferences.mode(host)!=DeliveryMode.Ntfy||record.unregister||UnifiedPush.getSavedDistributor(app)!=NTFY)return@withLock
    operation(host,record)
   };schedule(host)
  }}}catch(_:Exception){host(instance)?.takeIf{host->runtime.store.state.value.hosts.any{it.hostId==host&&it.deviceId==capturedDevice}}?.let{host->
   captured?.let{record->failedGenerations.computeIfAbsent(host){ConcurrentHashMap.newKeySet<Uuid>()}.add(record.generation);unconfirm(record)}
    ?: verifiedGenerations[host]?.takeIf{it.hello.deviceId==capturedDevice}?.let{verifiedGenerations.remove(host,it)}
   mark(host,"Repair required")
  }}finally{host(instance)?.takeIf{host->runtime.store.state.value.hosts.any{it.hostId==host&&it.deviceId==capturedDevice}}?.let(::schedule)}
 }
 fun callbackEndpoint(endpoint:PushEndpoint,instance:String)=callback(instance){host,record->
  if(endpoint.temporary||endpoint.url.length>2048||endpoint.url.any{Character.isISOControl(it)}){unconfirm(record);save(record.copy(repair=true));mark(host,"Repair required");return@callback}
  val uri=try{java.net.URI(endpoint.url)}catch(_:Exception){throw StorePolicy(ErrorCode.PushRejected)}
  if(uri.scheme!="https"||uri.host==null||uri.userInfo!=null||uri.fragment!=null)throw StorePolicy(ErrorCode.PushRejected)
  if(record.endpoint!=null&&record.endpoint!=endpoint.url){unconfirm(record);save(record.copy(repair=true));mark(host,"Endpoint changed: repair required");return@callback}
  save(record.copy(endpoint=endpoint.url));if(!verifiedGenerations.containsKey(host))mark(host,if(record.acknowledged)"Previously acknowledged · verifying current host"else"Waiting for probe")
 }
 fun callbackMessage(message:PushMessage,instance:String)=callback(instance){host,record->
  val hint=try{PushHints.verify(message.content,record)}catch(_:Exception){return@callback}
  when(val verified=hint){
   is VerifiedPushHint.Probe->{save(record.copy(probeReceived=true));if(!verifiedGenerations.containsKey(host))mark(host,if(record.acknowledged)"Previously acknowledged · verifying current host"else"Confirming probe")}
   is VerifiedPushHint.Event->{runtime.store.pushHint(host,verified.hint,{runtime.notifications.health().allowsAttention(runtime.deliveryPreferences.hostEnabled(host))},{runtime.deliveryPreferences.notifyCompleted(host)},{event->runtime.suppressesAlert(host,event)})}
  }
 }
 fun callbackFailure(instance:String)=callback(instance){host,record->unconfirm(record);save(record.copy(repair=true));mark(host,"Repair required")}
 private fun schedule(host:String){
  if(runtime.connection.state.value.hello[host]==null||runtime.connection.state.value.inventoryReady[host]!=true)return
  synchronized(jobs){
   dirty.add(host)
   if(jobs[host]?.isActive==true)return
   val job=runtime.cleanupScope.launch(start=CoroutineStart.LAZY){
    try {
     do {
      synchronized(jobs){dirty.remove(host)}
      val hello=runtime.connection.state.value.hello[host] ?: break
      reconcile(host,hello)
     }while(isActive&&synchronized(jobs){dirty.remove(host)})
    }finally{
     val owner=currentCoroutineContext()[Job]
     val again=synchronized(jobs){if(jobs[host]===owner){jobs.remove(host);dirty.remove(host)}else false}
     if(again)schedule(host)
    }
   }
   jobs[host]=job;job.start()
  }
 }
 private suspend fun reconcile(host:String,hello:HelloResult){
  if(!original(host,hello))return
  try {
   while(currentCoroutineContext().isActive&&original(host,hello)){
    if(synced[host]!==hello) {
     try {runtime.connection.reconcilePush(host,hello);if(!original(host,hello))return;runtime.notifications.reconcile(host);if(!original(host,hello))return;synced[host]=hello;if(!original(host,hello)){synced.remove(host,hello);return}}
     catch(cancel:CancellationException){throw cancel}
     catch(failure:Exception){if(!original(host,hello))return;if(failure is RemoteFailure&&failure.code==ErrorCode.Unauthorized){mark(host,"Authorization required");return};mark(host,"Notification reconciliation pending");delay(5000);continue}
    }
    var record=gate(host).withLock{load(host)} ?: return
    if(!original(host,hello)||record.device!=hello.deviceId)return
    if(failedGenerations[host]?.contains(record.generation)==true){gate(host).withLock{load(host)?.takeIf{it.device==record.device&&it.generation==record.generation}?.let{save(it.copy(repair=true),hello=hello)}};if(original(host,hello))mark(host,"Repair required");return}
    if(record.unregister){
     runtime.connection.unregisterPush(host,hello,record.generation)
     gate(host).withLock{load(host)?.takeIf{it.device==record.device&&it.generation==record.generation&&it.unregister}?.let{
      runtime.store.retirePush(host,it.generation,it.device){original(host,hello)}
      if(original(host,hello))mark(host,"Socket selected")
     }};return
    }
    if(runtime.deliveryPreferences.mode(host)!=DeliveryMode.Ntfy||record.repair||record.endpoint==null)return
    if(verifiedGenerations[host]?.let{it.generation==record.generation&&it.hello===hello}==true)return
    if(!ntfyInstalled()){unconfirm(record,hello);if(original(host,hello))mark(host,"ntfy missing: repair required");return}
    var retry=true
    try {
     // The original gate never spans native network I/O: a real probe may precede its reply.
     record=gate(host).withLock{load(host)?.takeIf{it.device==record.device&&it.generation==record.generation&&it.probe==record.probe&&it.endpoint==record.endpoint&&!it.unregister&&!it.repair}?.let{current->
      if(current.firstHostRegistrationAt==null)current.copy(firstHostRegistrationAt=System.currentTimeMillis()).also{save(it,hello=hello)}else current
     }} ?: return
     if(!original(host,hello))return
     val registered=runtime.connection.registerPush(host,hello,record)
     val current=gate(host).withLock{load(host)?.takeIf{original(host,hello)&&it.device==record.device&&it.generation==record.generation&&it.probe==record.probe&&it.endpoint==record.endpoint&&!it.unregister&&!it.repair}?.also{
      // A later local generation cannot interleave between this read and confirmation.
      if(it.acknowledged&&registered.verified&&runtime.deliveryPreferences.mode(host)==DeliveryMode.Ntfy)confirmed(host,it,hello)
     }} ?: return
     if(runtime.deliveryPreferences.mode(host)!=DeliveryMode.Ntfy)return
     if(current.acknowledged){
      if(!registered.verified){gate(host).withLock{load(host)?.takeIf{it.generation==current.generation&&it.acknowledged}?.let{save(it.copy(repair=true),hello=hello)}};if(original(host,hello)){unconfirm(current,hello);mark(host,"Host registration rejected: repair required")}}
      retry=false
     }else if(current.probeReceived){
      runtime.connection.acknowledgePush(host,hello,current)
      gate(host).withLock{load(host)?.takeIf{original(host,hello)&&it.device==current.device&&it.generation==current.generation&&it.probe==current.probe&&it.probeReceived&&!it.unregister&&!it.repair}?.let{
       val committed=it.copy(acknowledged=true);save(committed,hello=hello);confirmed(host,committed,hello);retry=false
      }}
     }else if(System.currentTimeMillis()-(current.firstHostRegistrationAt ?: throw StorePolicy(ErrorCode.NotReady))>=45000){
      gate(host).withLock{load(host)?.takeIf{it.generation==current.generation&&!it.probeReceived}?.let{save(it.copy(repair=true),hello=hello)}}
      if(original(host,hello))mark(host,"Probe timed out: repair required");retry=false
     }else mark(host,"Waiting for probe")
    }catch(cancel:CancellationException){throw cancel}catch(failure:Exception){
     if(!original(host,hello))return
     if(failure is RemoteFailure&&failure.code in listOf(ErrorCode.StaleRegistration,ErrorCode.PushRejected)){
      gate(host).withLock{load(host)?.takeIf{it.generation==record.generation}?.let{save(it.copy(repair=true),hello=hello)}}
      if(original(host,hello)){unconfirm(record,hello);mark(host,"Repair required")};retry=false
     }else mark(host,"Verification pending: reconnect or repair")
    }
    if(!runtime.store.state.value.hosts.any{runtime.deliveryPreferences.requiresKeeper(it.hostId,verified(it.hostId))})DeliveryService.stopWhenUnused(app)
    if(!retry)return
    delay(5000)
   }
  }catch(cancel:CancellationException){throw cancel}catch(_:ProtectedKeyUnavailable){if(original(host,hello)){verifiedGenerations[host]?.takeIf{it.hello===hello}?.let{verifiedGenerations.remove(host,it)};mark(host,"Protected push material unavailable: repair required")}}catch(_:Exception){if(original(host,hello))mark(host,"Verification pending: reconnect or repair")}
 }
 companion object{const val NTFY="io.heckel.ntfy"}
}
