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
 private val verifiedGenerations=ConcurrentHashMap<String,Uuid>();private val synced=ConcurrentHashMap<String,HelloResult>();private val gates=ConcurrentHashMap<String,Mutex>();private val jobs=ConcurrentHashMap<String,Job>();private val hydrated=ConcurrentHashMap.newKeySet<String>()
 private val mutable=MutableStateFlow<Map<String,String>>(emptyMap());val health:StateFlow<Map<String,String>> = mutable
 private val random=SecureRandom()
 init{
  runtime.cleanupScope.launch{runtime.connection.state.collect{view->view.hello.keys.filter{view.inventoryReady[it]==true}.forEach(::schedule)}}
  runtime.cleanupScope.launch{runtime.store.state.collect{view->
   gates.keys.filter{host->view.hosts.none{it.hostId==host}}.forEach{host->jobs.remove(host)?.cancel();gates.remove(host);verifiedGenerations.remove(host);synced.remove(host);hydrated.remove(host);mutable.update{it-host}}
   view.hosts.filter{runtime.deliveryPreferences.mode(it.hostId)==DeliveryMode.Ntfy&&hydrated.add(it.hostId)}.forEach{metadata->runtime.cleanupScope.launch{
    try{gate(metadata.hostId).withLock{load(metadata.hostId)?.let{record->if(record.acknowledged&&!record.repair&&ntfyInstalled())confirmed(metadata.hostId,record)else mark(metadata.hostId,if(record.repair||!ntfyInstalled())"Repair required"else"Verification pending")}}}catch(_:Exception){mark(metadata.hostId,"Repair required")}
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
 private fun confirmed(host:String,record:LocalPushRegistration){
  if(!record.acknowledged||record.repair||record.unregister)return
  verifiedGenerations[host]=record.generation;mark(host,"Verified")
 }
 internal fun label(host:String):String=mutable.value[host] ?: "Not verified"
 internal fun verified(host:String):Boolean=runtime.deliveryPreferences.mode(host)==DeliveryMode.Ntfy&&verifiedGenerations.containsKey(host)&&label(host)=="Verified"
 private suspend fun load(host:String):LocalPushRegistration?{
  val stored=runtime.store.encryptedRegistration(host) ?: return null
  val plain=runtime.secrets.pushOpen(host,stored.first,stored.second)
  return try{Protocol.strictJson.decodeFromString<LocalPushRegistration>(plain.toString(Charsets.UTF_8)).also{
   if(it.version!=1||it.createdAt<0||it.firstHostRegistrationAt?.let{at->at<0}==true||it.host!=host||it.generation!=stored.first||runtime.store.state.value.hosts.singleOrNull{row->row.hostId==host}?.deviceId!=it.device)throw ProtectedKeyUnavailable()
   it.generation.validateWire();it.probe.validateWire();Key32(it.key)
  }}finally{plain.fill(0)}
 }
 private suspend fun save(value:LocalPushRegistration){
  val plain=Protocol.strictJson.encodeToString(value).toByteArray(Charsets.UTF_8)
  try{runtime.store.encryptedRegistration(value.host,value.generation,runtime.secrets.pushSeal(value.host,value.generation,plain))}finally{plain.fill(0)}
 }
 suspend fun selectNtfy(host:String,repair:Boolean,active:()->Boolean){
  if(!active())throw StorePolicy(ErrorCode.NotReady)
  runtime.store.open();gate(host).withLock{
   val metadata=runtime.store.state.value.hosts.singleOrNull{it.hostId==host} ?: throw StorePolicy(ErrorCode.Unauthorized)
   val device=metadata.deviceId ?: throw StorePolicy(ErrorCode.NotReady)
   if(!ntfyInstalled())throw StorePolicy(ErrorCode.NotReady)
   if(repair){verifiedGenerations.remove(host);mark(host,"Repairing registration")}
   var registration=load(host)
   if(registration==null||registration.unregister||repair){
    verifiedGenerations.remove(host);mark(host,"Preparing registration")
    val alias=SecretStore.hostAlias(host,"push");val retained=runtime.store.ensurePushAlias(host,alias);runtime.secrets.material(alias,!retained).fill(0)
    val key=ByteArray(32).also(random::nextBytes)
    registration=try{LocalPushRegistration(1,host,device,Uuid.random(),Uuid.random(),PushHints.encode(key),System.currentTimeMillis())}finally{key.fill(0)}
    save(registration)
   }
   if(!active())throw StorePolicy(ErrorCode.NotReady)
   runtime.deliveryPreferences.setMode(host,DeliveryMode.Ntfy)
   // Explicit selection only; never consult default/current distributor convenience APIs.
   UnifiedPush.saveDistributor(app,NTFY);UnifiedPush.register(app,instance(host),"Goatr paired desktop")
   if(registration.acknowledged&&!registration.repair)confirmed(host,registration)else{verifiedGenerations.remove(host);mark(host,"Waiting for endpoint")}
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
 suspend fun forget(host:String){verifiedGenerations.remove(host);synced.remove(host);hydrated.remove(host);jobs.remove(host)?.cancel();try{UnifiedPush.unregister(app,instance(host))}catch(_:Exception){};gates.remove(host);mutable.update{it-host}}
 /** Receiver's bounded goAsync worker invokes these synchronously before SDK ACK. */
 private fun callback(instance:String,operation:suspend (String,LocalPushRegistration)->Unit){
  try{runBlocking(Dispatchers.IO){withTimeout(2000){runtime.store.open();val host=host(instance) ?: return@withTimeout
   gate(host).withLock{val record=load(host) ?: return@withLock
    if(runtime.deliveryPreferences.mode(host)!=DeliveryMode.Ntfy||record.unregister||UnifiedPush.getSavedDistributor(app)!=NTFY)return@withLock
    operation(host,record)
   };schedule(host)
  }}}catch(_:Exception){host(instance)?.let{mark(it,"Repair required")}}
 }
 fun callbackEndpoint(endpoint:PushEndpoint,instance:String)=callback(instance){host,record->
  if(endpoint.temporary||endpoint.url.length>2048||endpoint.url.any{Character.isISOControl(it)}){save(record.copy(repair=true));mark(host,"Repair required");return@callback}
  val uri=try{java.net.URI(endpoint.url)}catch(_:Exception){throw StorePolicy(ErrorCode.PushRejected)}
  if(uri.scheme!="https"||uri.host==null||uri.userInfo!=null||uri.fragment!=null)throw StorePolicy(ErrorCode.PushRejected)
  if(record.endpoint!=null&&record.endpoint!=endpoint.url){save(record.copy(repair=true));mark(host,"Endpoint changed: repair required");return@callback}
  save(record.copy(endpoint=endpoint.url));if(record.acknowledged&&!record.repair)confirmed(host,record)else mark(host,"Waiting for probe")
 }
 fun callbackMessage(message:PushMessage,instance:String)=callback(instance){host,record->
  val hint=try{PushHints.verify(message.content,record)}catch(_:Exception){return@callback}
  when(val verified=hint){
   is VerifiedPushHint.Probe->{save(record.copy(probeReceived=true));if(record.acknowledged&&!record.repair)confirmed(host,record)else mark(host,"Confirming probe")}
   is VerifiedPushHint.Event->{runtime.store.pushHint(host,verified.hint,{runtime.deliveryPreferences.hostEnabled(host)&&runtime.deliveryPreferences.canPostNotifications()},{runtime.deliveryPreferences.notifyCompleted(host)},{runtime.suppressesHint(host,verified.hint)})}
  }
 }
 fun callbackFailure(instance:String)=callback(instance){host,record->verifiedGenerations.remove(host);mark(host,"Repair required");save(record.copy(repair=true))}
 private fun schedule(host:String){
  if(!runtime.connection.state.value.hello.containsKey(host)||runtime.connection.state.value.inventoryReady[host]!=true)return
  synchronized(jobs){if(jobs[host]?.isActive==true)return
   jobs[host]=runtime.cleanupScope.launch{
    try{
     val original=runtime.connection.state.value.hello[host] ?: return@launch
     if(synced[host]!==original){runtime.connection.reconcilePush(host,original);runtime.notifications.reconcile(host);synced[host]=original}
     while(isActive&&runtime.connection.state.value.hello[host]===original){
      var record=gate(host).withLock{load(host)} ?: break
      if(record.unregister){
       runtime.connection.unregisterPush(host,record.generation)
       gate(host).withLock{if(load(host)?.let{it.generation==record.generation&&it.unregister}==true){runtime.store.retirePush(host,record.generation);mark(host,"Socket selected")}};break
      }
      if(runtime.deliveryPreferences.mode(host)!=DeliveryMode.Ntfy||record.repair||record.endpoint==null)break
      if(record.acknowledged){gate(host).withLock{load(host)?.takeIf{it.generation==record.generation&&it.acknowledged&&!it.repair}?.let{if(ntfyInstalled())confirmed(host,it)else{verifiedGenerations.remove(host);mark(host,"ntfy missing: repair required")}}};break}
      var retry=true
      try{
       // Never hold the registration gate across network I/O: a probe may arrive
       // before register's response and must be durably accepted immediately.
       record=gate(host).withLock{load(host)?.takeIf{it.generation==record.generation&&it.probe==record.probe&&it.endpoint==record.endpoint&&!it.unregister&&!it.repair}?.let{current->
        if(current.firstHostRegistrationAt==null)current.copy(firstHostRegistrationAt=System.currentTimeMillis()).also{save(it)}else current
       }} ?: break
       val registered=runtime.connection.registerPush(host,record)
       if(registered.endpointGeneration!=record.generation||registered.probe!=record.probe)throw StorePolicy(ErrorCode.PushRejected)
       val current=gate(host).withLock{load(host)?.takeIf{it.generation==record.generation&&it.probe==record.probe&&it.endpoint==record.endpoint&&!it.unregister&&!it.repair}}
       if(current==null||runtime.deliveryPreferences.mode(host)!=DeliveryMode.Ntfy)break
       if(current.probeReceived){
        runtime.connection.acknowledgePush(host,current)
        gate(host).withLock{load(host)?.takeIf{it.generation==current.generation&&it.probe==current.probe&&it.probeReceived&&!it.unregister&&!it.repair}?.let{val committed=it.copy(acknowledged=true);save(committed);confirmed(host,committed);retry=false}}
       }else if(System.currentTimeMillis()-(current.firstHostRegistrationAt ?: throw StorePolicy(ErrorCode.NotReady))>=45000){gate(host).withLock{load(host)?.takeIf{it.generation==current.generation&&!it.probeReceived}?.let{save(it.copy(repair=true));mark(host,"Probe timed out: repair required")}};retry=false}
       else mark(host,"Waiting for probe")
      }catch(cancel:CancellationException){throw cancel}catch(failure:Exception){
       if(failure is RemoteFailure&&failure.code in listOf(ErrorCode.StaleRegistration,ErrorCode.PushRejected)){
        gate(host).withLock{load(host)?.takeIf{it.generation==record.generation}?.let{save(it.copy(repair=true))}};mark(host,"Repair required");retry=false
       }else mark(host,"Verification pending: reconnect or repair")
      }
      if(!runtime.store.state.value.hosts.any{runtime.deliveryPreferences.requiresKeeper(it.hostId,verified(it.hostId))})DeliveryService.stopWhenUnused(app)
      if(!retry)break;delay(5000)
     }
    }catch(cancel:CancellationException){throw cancel}catch(_:Exception){mark(host,"Repair required")}
    finally{val original=currentCoroutineContext()[Job];synchronized(jobs){if(jobs[host]===original)jobs.remove(host)}}
   }
  }
 }
 companion object{const val NTFY="io.heckel.ntfy"}
}
