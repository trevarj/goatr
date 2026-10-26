// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.os.IBinder
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.coroutines.coroutineContext

data class DeliverySnapshot(val running:Boolean=false,val reconnecting:Set<String> = emptySet())

/** User-started special-use FGS. It owns one application claim, never a second transport/store. */
class DeliveryService : Service() {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.IO)
    private lateinit var runtime:GoatrRuntime
    private lateinit var connectivity:ConnectivityManager
    private lateinit var claim:RuntimeClaim
    private val lifecycle=Mutex()
    private val retryMutex=Mutex()
    private val owned=mutableSetOf<Job>()
    private data class HostGate(var running:Boolean=false,var dirty:Boolean=false,var retryScheduled:Boolean=false)
    private val hostGates=mutableMapOf<String,HostGate>()
    private val retryIndex=mutableMapOf<String,Int>()
    private var retryGeneration=0L
    private var initialized=false
    private var destroyed=false
    private var networkCallback:ConnectivityManager.NetworkCallback?=null

    override fun onCreate() {
        super.onCreate()
        runtime=(application as GoatrApplication).runtime
        claim=runtime.newClaim(RuntimeClaimKind.Delivery)
        connectivity=getSystemService(ConnectivityManager::class.java)
    }
    private fun launchOwned(block:suspend ()->Unit):Job = scope.launch {
        try {block()} finally { synchronized(owned) {owned.remove(coroutineContext[Job])} }
    }.also { synchronized(owned) {owned+=it} }

    override fun onStartCommand(intent:Intent?,flags:Int,startId:Int):Int {
        if(intent?.action==ACTION_STOP) {runtime.deliveryPreferences.setServiceRequested(false);stopSelf();return START_NOT_STICKY}
        if(!runtime.deliveryPreferences.serviceRequested() || !runtime.deliveryPreferences.canPostNotifications()) {updateStatus(DeliverySnapshot());stopSelf();return START_NOT_STICKY}
        startForeground(FOREGROUND_ID,runtime.notifications.foreground())
        launchOwned { initialize() }
        return START_NOT_STICKY
    }
    private fun enabledHosts():List<String> = runtime.store.state.value.hosts.map {it.hostId}.filter{runtime.deliveryPreferences.requiresKeeper(it,runtime.push.verified(it))}

    private suspend fun initialize() {
        lifecycle.withLock {
            if(destroyed || initialized) return
            try { runtime.acquire(claim) }
            catch(cancel:CancellationException) { try {runtime.release(claim)} catch(_:Exception) {};throw cancel }
            catch(_:Exception) {
                // The selected policy remains durable; do not reset keys or create a replacement.
                updateStatus(DeliverySnapshot(running=false,reconnecting=enabledHosts().toSet()))
                try {runtime.release(claim)} catch(_:Exception) {}
                stopSelf()
                return
            }
            if(destroyed) {runtime.release(claim);return}
            if(!runtime.deliveryPreferences.canPostNotifications()) {enabledHosts().forEach {host->runtime.notifications.reconcile(host)};updateStatus(DeliverySnapshot());stopSelf();return}
            initialized=true
            updateStatus(DeliverySnapshot(running=true))
            observeNetwork()
            launchOwned { runtime.connection.notificationWakeups.collect { requestReconcile(it.host) } }
            launchOwned { runtime.deliveryPreferences.changes.collect {if(enabledHosts().isEmpty()){updateStatus(DeliverySnapshot());stopSelf()}else enabledHosts().forEach {host->requestReconcile(host)} } }
            // Authoritative connection loss is a wakeup for bounded recovery, healthy hosts are not redialed.
            launchOwned { runtime.connection.state.collect { view -> enabledHosts().filter { !view.hello.containsKey(it) }.forEach {host->requestReconcile(host)} } }
            enabledHosts().forEach {host->requestReconcile(host)}
        }
    }
    /** Every signal coalesces into one per-host owner. A signal arriving mid-sync sets dirty. */
    private fun requestReconcile(host:String) { launchOwned {
        val start=retryMutex.withLock {
            val gate=hostGates.getOrPut(host){HostGate()};gate.dirty=true
            if(gate.running) false else {gate.running=true;true}
        }
        if(start) runHostGate(host)
    } }
    private suspend fun runHostGate(host:String) {
        while(true) {
            retryMutex.withLock {hostGates[host]?.dirty=false}
            reconcileHostOnce(host)
            val again=retryMutex.withLock {
                val gate=hostGates[host] ?: return@withLock false
                if(gate.dirty) true else {gate.running=false;false}
            }
            if(!again) return
        }
    }
    private suspend fun reconcileHostOnce(host:String) {
        if(destroyed || !runtime.deliveryPreferences.serviceRequested() || !runtime.deliveryPreferences.canPostNotifications()) {
            if(!runtime.deliveryPreferences.canPostNotifications()) {runtime.notifications.reconcile(host);updateStatus(DeliverySnapshot());stopSelf()};return
        }
        if(!runtime.deliveryPreferences.requiresKeeper(host,runtime.push.verified(host))) {runtime.notifications.reconcile(host);return}
        markReconnecting(host,true)
        try {
            runtime.connection.ensureConnectedIfNeeded(host)
            runtime.connection.snapshot(host)
            val expected=runtime.connection.state.value.hello[host] ?: throw StorePolicy(ErrorCode.NotReady)
            var after:Uint?=null
            do { after=runtime.connection.notificationsSync(host,expected,after).nextSequence } while(after!=null)
            retryMutex.withLock {retryIndex.remove(host);hostGates[host]?.retryScheduled=false}
            markReconnecting(host,false)
            runtime.notifications.reconcile(host)
        } catch(cancel:CancellationException) {throw cancel}
        catch(_:Exception) {markReconnecting(host,true);schedule(host)}
    }
    private fun schedule(host:String) { launchOwned {
        val scheduled=retryMutex.withLock {
            val gate=hostGates.getOrPut(host){HostGate()}
            if(destroyed || gate.retryScheduled) null else {
                val seconds=RETRY_SECONDS[minOf(retryIndex[host] ?: 0,RETRY_SECONDS.lastIndex)]
                retryIndex[host]=(retryIndex[host] ?: 0)+1
                gate.retryScheduled=true
                seconds to retryGeneration
            }
        } ?: return@launchOwned
        delay(scheduled.first*1000L)
        val current=retryMutex.withLock {
            val matching=scheduled.second==retryGeneration
            if(matching) hostGates[host]?.retryScheduled=false
            matching
        }
        if(!current) return@launchOwned
        requestReconcile(host)
    } }
    private fun observeNetwork() {
        if(networkCallback!=null) return
        networkCallback=object:ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network:Network) { launchOwned { retryMutex.withLock {retryGeneration++;retryIndex.clear();hostGates.values.forEach {it.retryScheduled=false}};enabledHosts().forEach {host->requestReconcile(host)} } }
        }.also(connectivity::registerDefaultNetworkCallback)
    }
    private fun markReconnecting(host:String,reconnecting:Boolean) {
        val now=statusMutable.value;val next=now.reconnecting.toMutableSet()
        if(reconnecting) next+=host else next-=host
        updateStatus(now.copy(reconnecting=next))
    }
    override fun onDestroy() {
        destroyed=true
        networkCallback?.let { try {connectivity.unregisterNetworkCallback(it)} catch(_:Exception) {} };networkCallback=null
        updateStatus(DeliverySnapshot())
        runtime.cleanupScope.launch {
            val jobs=synchronized(owned) {owned.toList()};jobs.forEach {it.cancel()};jobs.forEach {try {it.join()} catch(_:CancellationException) {}}
            retryMutex.withLock {retryGeneration++;hostGates.clear();retryIndex.clear()}
            try {runtime.release(claim)} catch(_:Exception) {}
            scope.cancel()
        }
        super.onDestroy()
    }
    override fun onBind(intent:Intent?):IBinder?=null
    companion object {
        const val ACTION_START="io.github.trevarj.goatr.DELIVERY_START"
        const val ACTION_STOP="io.github.trevarj.goatr.DELIVERY_STOP"
        private const val FOREGROUND_ID=7124
        private val RETRY_SECONDS=intArrayOf(5,15,30,60,300)
        private val statusMutable=MutableStateFlow(DeliverySnapshot())
        val status:StateFlow<DeliverySnapshot> = statusMutable
        private fun updateStatus(value:DeliverySnapshot) {statusMutable.value=value}
        fun startFromUser(context:Context,admitted:()->Boolean):Boolean {
            if(!admitted())return false
            val app=context.applicationContext as GoatrApplication
            if(!app.runtime.deliveryPreferences.canPostNotifications()||!admitted())return false
            app.runtime.deliveryPreferences.setServiceRequested(true)
            if(!admitted())return false
            app.startForegroundService(Intent(app,DeliveryService::class.java).setAction(ACTION_START));return true
        }
        fun stopFromUser(context:Context) {val app=context.applicationContext as GoatrApplication;app.runtime.deliveryPreferences.setServiceRequested(false);app.stopService(Intent(app,DeliveryService::class.java))}
        fun stopWhenUnused(context:Context) {context.applicationContext.stopService(Intent(context.applicationContext,DeliveryService::class.java))}
        fun isReconnecting(host:String):Boolean=statusMutable.value.reconnecting.contains(host)
    }
}
