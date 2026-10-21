// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.LinkProperties
import computer.iroh.BiStream
import computer.iroh.Connection
import computer.iroh.Endpoint
import computer.iroh.EndpointBuilder
import computer.iroh.EndpointOptions
import computer.iroh.IrohAndroid
import computer.iroh.PathChangeCallback
import computer.iroh.PathEventCallback
import computer.iroh.PathEvent
import computer.iroh.PathSnapshot
import computer.iroh.Preset
import computer.iroh.RecvStream
import computer.iroh.OwnedReadObservation
import computer.iroh.OwnedReadPhase
import computer.iroh.readExactObserved
import computer.iroh.SendStream
import computer.iroh.WatchHandle
import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

class RemoteFailure(val code: ErrorCode) : Exception(code.name)
/** Process-local viewer lease; it carries no wire identity or replay authority. */
internal class ShellMutationLease {
    val active=AtomicBoolean(true)
    private val gate=Any()
    fun invalidate(){synchronized(gate){active.set(false)}}
    fun admit(block:()->Unit)=synchronized(gate) {
        if(!active.get()) throw StorePolicy(ErrorCode.StaleSession)
        block()
    }
}
internal class AgentAliasLease {
    private val gate=Any();private var active=true
    fun invalidate(){synchronized(gate){active=false}}
    fun admit(block:()->Unit)=synchronized(gate){if(!active)throw StorePolicy(ErrorCode.StaleSession);block()}
}
class AgentAliasMutationContext internal constructor(internal val original:Any,internal val lease:AgentAliasLease)
internal class WorktreeRemovalLease {
    private val gate=Any();private var active=true
    fun invalidate(){synchronized(gate){active=false}}
    fun admit(block:()->Unit)=synchronized(gate){if(!active)throw StorePolicy(ErrorCode.StaleSession);block()}
}
class WorktreeRemovalMutationContext internal constructor(internal val original:Any,internal val lease:WorktreeRemovalLease,val preview:WorktreesRemovePreviewResult,internal val ticket:Long,internal val expiresAt:Long)
data class ShellInputObservation(val token:Uuid,val unknown:Boolean)
private data class ShellInputStatusObservation(val order:Long,val observation:ShellInputObservation)
internal enum class ShellMutationPurpose {Create,Input}
class ShellMutationContext internal constructor(internal val original:Any,internal val lease:ShellMutationLease,internal val purpose:ShellMutationPurpose,internal val inputObservation:Pair<Uuid,Uuid>?=null)
enum class ConnectionFailurePhase { Dial, OpenStream, Hello, Snapshot, Reader, Recovery }
enum class ConnectionFailureCategory { Timeout, Known, Protocol, Transport, Other }
enum class HelloReceivePhase { NotStarted, Header, Payload, Decode, Apply, Completed }
data class ConnectionFailure(val phase: ConnectionFailurePhase,val category: ConnectionFailureCategory,val code: ErrorCode? = null,val helloTransmissionCompleted:Boolean? = null,val helloReceivePhase:HelloReceivePhase? = null,val helloNativeReadPhase:OwnedReadPhase? = null)
/** A bounded post-commit hint only. Delivery always re-reads SessionStore before presentation. */
data class NotificationWakeup(val host: String,val syncRequired:Boolean=false)
// Reducers mutate in place; each ordered publication must have a distinct observable identity.
class ConnectionView(
    val hello: Map<String,HelloResult> = emptyMap(),
    val inventory: Map<String,HostSnapshotResult> = emptyMap(),
    val sessions: Map<String,List<SessionReducer>> = emptyMap(),
    val selectedPath: Map<String,String> = emptyMap(),
    val inventoryReady: Map<String,Boolean> = emptyMap(),
    val sources: Map<String,Map<Target,Uuid>> = emptyMap(),
    val latestReceipt: Map<String,Receipt> = emptyMap(),
    // Current authenticated observations only; historical receipts remain unchanged.
    val shellInputStatus:Map<String,Map<Uuid,ShellInputObservation>> = emptyMap(),
    // Host receipt reconciliation is required by mutationContext before preSend can commit.
    val mutationReady: Map<String,Boolean> = emptyMap(),
    val connectionFailures: Map<String,ConnectionFailure> = emptyMap(),
    val routeAttempts: Map<String,RouteAttemptView> = emptyMap(),
    val activeRoutes: Map<String,RouteAttemptView> = emptyMap(),
    val selectedRoutes: Map<String,SelectedIrohPath> = emptyMap(),
)

internal enum class ActionAdmissionPhase(val closed:String) { OriginalScope("original_scope"), PrePreSend("pre_presend"), PostPreSend("post_presend"), Request("request"), Writer("writer"), Pending("pending"), ResponseApply("response_apply"), ExistingStatus("existing_status") }
internal enum class ActionAdmissionGuard(val closed:String) {
    OriginalAbsent("original_absent"), OriginalNotAlive("original_not_alive"), OriginalUnauthenticated("original_unauthenticated"), OriginalReplaced("original_replaced"), BootChanged("boot_changed"), HostNotReady("host_not_ready"), ReceiptsUnreconciled("receipts_unreconciled"), ReducerAbsent("reducer_absent"), SummaryAbsent("summary_absent"), TargetChanged("target_changed"), MutationsDisabled("mutations_disabled"), SourceNotCurrent("source_not_current"), NotAlive("not_alive"), Unauthenticated("unauthenticated"), InflightBusy("inflight_busy"), PendingConflict("pending_conflict"), IoBeforeFrame("io_before_frame"), IoAfterFrameStart("io_after_frame_start"), ConnectionClosed("connection_closed"), StorePolicy("store_policy"), Other("other"), Unknown("unknown"),
}
internal data class ActionAdmissionFailure(val phase:ActionAdmissionPhase,val guard:ActionAdmissionGuard,val code:ErrorCode,val sent:Boolean?,val frameStarted:Boolean?)
internal enum class NewActionContinuationBarrierPoint { BeforePreSend, AfterPreSend }
internal class NewActionContinuationBarrier internal constructor(val host:String,val action:Uuid,val point:NewActionContinuationBarrierPoint) {
    internal val reached=CompletableDeferred<Unit>()
    internal val release=CompletableDeferred<Unit>()
    internal val recoveryWaited=CompletableDeferred<Unit>()
    internal val frameStarted=CompletableDeferred<Unit>()
    internal val frameStarts=AtomicInteger(0)
}

/** Foreground-only owner. Constructor uses applicationContext; start installs JNI context BEFORE
 * binding. One installation endpoint, one coalesced platform callback, explicit native handles.
 * There is no automatic replay, hidden foreground service, or plaintext credential cache. */
class GoatrConnection(context: Context, val store: SessionStore, val secrets: SecretStore) {
    companion object {private val endpointOwned=AtomicBoolean(false);private val alpn="goatr/2".toByteArray(Charsets.UTF_8);internal fun endpointIsOwned():Boolean=endpointOwned.get()}
    private val app=context.applicationContext
    private val connectivity=app.getSystemService(ConnectivityManager::class.java)
    private val serial=Dispatchers.IO.limitedParallelism(1)
    private val scope=CoroutineScope(SupervisorJob()+serial)
    private val lifecycle=Mutex()
    private val hostOperations=Mutex()
    private var endpoint:Endpoint?=null
    private var callback:ConnectivityManager.NetworkCallback?=null
    private var networkJob:Job?=null
    private var changes:Channel<Unit>?=null
    private val sessions=ConcurrentHashMap<String,Live>()
    private val retiring=ConcurrentHashMap<Live,Unit>()
    private class ConnectionAttempt(val host:String,val routes:RouteAttemptView) {var hintsSupplied=false;var candidateToken:Any?=null;var failure:ConnectionFailure?=null}
    private val connectionFailureLock=Any()
    private val connectionAttempts=HashMap<String,ConnectionAttempt>()
    private class ActionAdmissionCapture(val host:String,val action:Uuid) {val failure=AtomicReference<ActionAdmissionFailure?>(null)}
    private val actionAdmissionCapture=AtomicReference<ActionAdmissionCapture?>(null)
    private val newActionContinuationBarrier=AtomicReference<NewActionContinuationBarrier?>(null)
    private val mutable=MutableStateFlow(ConnectionView())
    val state:StateFlow<ConnectionView> = mutable.asStateFlow()
    private val notificationWakeupMutable=MutableSharedFlow<NotificationWakeup>(extraBufferCapacity=32,onBufferOverflow=BufferOverflow.DROP_OLDEST)
    val notificationWakeups=notificationWakeupMutable
    private class Pending(val method:Method,val deferred:CompletableDeferred<ResultBody>,val apply:suspend (ResultBody)->Unit) {
        val admitted=AtomicBoolean(false)
        var sent=false
    }
    private class ShellReadContext(val host:String,val live:Live,val hostReducer:HostReducer,val subscription:Uuid,val shell:ShellSummary) {val active=AtomicBoolean(true)}
    private class TerminalReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid) {
        /** Caller cancellation invalidates a correlated late reply without closing the transport. */
        val active=AtomicBoolean(true)
    }
    private class WorktreesReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid) {
        /** A dialog ticket owns this transient read; canceled or replaced contexts cannot admit a late result. */
        val active=AtomicBoolean(true)
    }
    private class SkillsReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val generation:Uuid) {val active=AtomicBoolean(true);var catalogueRefresh=false}
    private class AttachmentContext(val host:String,val live:Live,val reducer:SessionReducer?,val target:Target,val source:Uuid?,val subscription:Uuid?)
    private class ArtifactOriginal(val host:String,val live:Live,val hostReducer:HostReducer,val subscription:Uuid,val source:ArtifactSource,val reducer:SessionReducer?,val epoch:Uuid?,val shell:ShellSummary?,val server:Uuid,val incarnation:Uuid)
    private class AgentSessionContext(val host:String,val live:Live,val boot:Uuid,val reducer:SessionReducer,val target:Target,val source:Uuid,val subscription:Uuid?,val server:Uuid,val provider:Provider,val workspace:Id,val pane:Id,val terminal:Id)
    private class AgentTerminalContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val subscription:Uuid?)
    private class AgentAliasContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid)
    private class ChangesReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid) {
        /** The sheet ticket owns this memory-only read; replacement UI cannot receive a late reply. */
        val active=AtomicBoolean(true)
    }
    private class Live(val host:HostMetadata,val attempt:ConnectionAttempt,val candidateToken:Any,val connection:Connection,val stream:BiStream,val send:SendStream,val recv:RecvStream) {
        val attachmentStatuses=ConcurrentHashMap<Uuid,AttachmentsStatusResult>()
        val alive=AtomicBoolean(true);val disposed=AtomicBoolean(false);val writer=Mutex();val pending=ConcurrentHashMap<Uuid,Pending>();val inflight=Semaphore(64)
        val teardownDone=CompletableDeferred<Unit>()
        val hostReducer=HostReducer();val reducers=ConcurrentHashMap<Uuid,SessionReducer>();val subscriptions=ConcurrentHashMap<Uuid,Uuid>()
        val operations=Mutex();val resnapshot=linkedSetOf<Uuid>();var recoveryRequested=false;var recovery:Job?=null;var receiptsRecovered=false
        val watches=mutableListOf<WatchHandle>();var reader:Job?=null;var authenticated=false;var hello:HelloResult?=null;var selectedPath:String?=null;var selectedRoute:SelectedIrohPath?=null
        val helloReceivePhase=AtomicReference(HelloReceivePhase.NotStarted)
        val helloReadObservation=AtomicReference<OwnedReadObservation?>(null);val helloHeaderPending=AtomicBoolean(false)
        val worktreeCreationScopes=mutableMapOf<Target,Pair<Uuid,WorktreeCreationScope>>()
        var statusObservationOrder=0L
        var statusObservationFloor=0L
        val shellInputStatuses=linkedMapOf<Uuid,ShellInputStatusObservation>()
        var latestReceipt:Receipt?=null
    }
    private fun beginConnectionAttempt(host:String): ConnectionAttempt {
        val stored=store.state.value
        val paired=stored.hosts.firstOrNull {it.hostId==host && it.deviceId!=null}
        val saved=if(paired!=null) stored.routes.firstOrNull {it.hostId==host} else null
        val attempt=ConnectionAttempt(host,RouteAttemptView(saved?.revision ?: 0,saved?.direct?.toList() ?: emptyList(),saved?.relay))
        synchronized(connectionFailureLock) {connectionAttempts[host]=attempt}
        publish()
        return attempt
    }
    private fun selectConnectionCandidate(attempt:ConnectionAttempt,candidate:Live) {
        synchronized(connectionFailureLock) {
            if(connectionAttempts[attempt.host]===attempt) {attempt.candidateToken=candidate.candidateToken;attempt.failure=null}
        }
    }
    private fun recordConnectionFailure(attempt:ConnectionAttempt,candidate:Live?=null,phase:ConnectionFailurePhase,failure:Exception,helloTransmissionCompleted:Boolean?=null,helloReplyCompleted:Boolean?=null) {
        val helloTransmission=helloTransmissionCompleted.takeIf {phase==ConnectionFailurePhase.Hello}
        val helloReceive=when {
            phase!=ConnectionFailurePhase.Hello -> null
            helloReplyCompleted==true -> HelloReceivePhase.Completed
            else -> candidate?.helloReceivePhase?.get()
        }
        if(phase==ConnectionFailurePhase.Hello) candidate?.helloHeaderPending?.compareAndSet(true,false)
        val helloNativeRead=candidate?.helloReadObservation?.get()?.phase()?.takeIf {phase==ConnectionFailurePhase.Hello}
        val observed=when(failure) {
            is TimeoutCancellationException -> ConnectionFailure(phase,ConnectionFailureCategory.Timeout,helloTransmissionCompleted=helloTransmission,helloReceivePhase=helloReceive,helloNativeReadPhase=helloNativeRead)
            is CancellationException -> return
            is RemoteFailure -> ConnectionFailure(phase,ConnectionFailureCategory.Known,failure.code,helloTransmission,helloReceive,helloNativeRead)
            is StorePolicy -> ConnectionFailure(phase,ConnectionFailureCategory.Known,failure.code,helloTransmission,helloReceive,helloNativeRead)
            is ProtocolFailure -> ConnectionFailure(phase,ConnectionFailureCategory.Protocol,helloTransmissionCompleted=helloTransmission,helloReceivePhase=helloReceive,helloNativeReadPhase=helloNativeRead)
            is computer.iroh.IrohException -> ConnectionFailure(phase,ConnectionFailureCategory.Transport,helloTransmissionCompleted=helloTransmission,helloReceivePhase=helloReceive,helloNativeReadPhase=helloNativeRead)
            else -> ConnectionFailure(phase,ConnectionFailureCategory.Other,helloTransmissionCompleted=helloTransmission,helloReceivePhase=helloReceive,helloNativeReadPhase=helloNativeRead)
        }
        val recorded=synchronized(connectionFailureLock) {
            if(connectionAttempts[attempt.host]!==attempt || candidate!=null &&
                (attempt.candidateToken!==candidate.candidateToken || !candidate.alive.get() || candidate.disposed.get()) || attempt.failure!=null) false
            else {attempt.failure=observed;true}
        }
        if(recorded) publish()
    }
    private fun connectionFailureSnapshot(): Map<String,ConnectionFailure> = synchronized(connectionFailureLock) {
        connectionAttempts.mapNotNull {(host,attempt)->attempt.failure?.let {host to it}}.toMap()
    }
    private fun clearConnectionAttempt(host:String) {
        synchronized(connectionFailureLock) {connectionAttempts.remove(host)}
    }
    private fun clearConnectionAttempts() {
        synchronized(connectionFailureLock) {connectionAttempts.clear()}
    }
    private fun startupHelloTimeoutCurrent(host:String,attempt:ConnectionAttempt):Boolean = synchronized(connectionFailureLock) {
        connectionAttempts[host]===attempt && attempt.failure?.let {
            it.phase==ConnectionFailurePhase.Hello && it.category==ConnectionFailureCategory.Timeout && it.code==null
        }==true
    }
    private class CanonicalAdmissionReceipt(val receipt:Receipt):Exception()
    private fun originalLive(host:String,action:Uuid):Live {
        val current=sessions[host] ?: run {
            captureActionAdmission(host,action,ActionAdmissionPhase.OriginalScope,ActionAdmissionGuard.OriginalAbsent,ErrorCode.NotReady)
            throw StorePolicy(ErrorCode.NotReady)
        }
        if(!current.alive.get()) {
            captureActionAdmission(host,action,ActionAdmissionPhase.OriginalScope,ActionAdmissionGuard.OriginalNotAlive,ErrorCode.NotReady)
            throw StorePolicy(ErrorCode.NotReady)
        }
        if(!current.authenticated) {
            captureActionAdmission(host,action,ActionAdmissionPhase.OriginalScope,ActionAdmissionGuard.OriginalUnauthenticated,ErrorCode.NotReady)
            throw StorePolicy(ErrorCode.NotReady)
        }
        return current
    }
    private inner class NewActionContinuation(val host:String,val live:Live,val request:Request,val caller:Job,val shell:ShellMutationContext?=null,val alias:AgentAliasMutationContext?=null,val removal:WorktreeRemovalMutationContext?=null,val attachment:AttachmentMutationContext?=null,val terminal:AgentTerminalMutationContext?=null,val agentCreation:AgentSessionMutationContext?=null) {
        private val body=request.body
        private val creationSource=(body as? RequestBody.SessionCreate)?.params?.sourceTarget?.let {live.reducers[it.sessionId]?.source}
        private fun original(current:Live,requested:RequestBody,id:Uuid,onFailure:((ActionAdmissionGuard,ErrorCode)->Unit)?=null) {
            caller.ensureActive()
            when {
                current!==live || sessions[host]!==live -> {onFailure?.invoke(ActionAdmissionGuard.OriginalReplaced,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}
                !live.alive.get() -> {onFailure?.invoke(ActionAdmissionGuard.OriginalNotAlive,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}
                !live.authenticated -> {onFailure?.invoke(ActionAdmissionGuard.OriginalUnauthenticated,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}
                id!=request.id || requested!==body -> throw StorePolicy(ErrorCode.InvalidRequest)
            }
            if(agentCreation!=null)validateAgentSession(agentCreation,current,requested as? RequestBody.SessionCreate ?: throw StorePolicy(ErrorCode.InvalidRequest))
            if(requested is RequestBody.TerminalCreate)validateAgentTerminal(terminal ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested.attachmentTarget()!=null)validateAttachment(attachment ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested is RequestBody.WorktreesRemove) validateWorktreeRemoval(removal ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested is RequestBody.AgentRename) validateAgentAlias(alias ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested is RequestBody.ShellCreate || requested is RequestBody.ShellInput) {
                try {validateShellMutation(shell ?: throw StorePolicy(ErrorCode.NotReady),current,requested)}
                catch(failure:StorePolicy){onFailure?.invoke(ActionAdmissionGuard.SourceNotCurrent,failure.code);throw failure}
            }
        }
        private fun validate(current:Live,requested:RequestBody,id:Uuid,onFailure:((ActionAdmissionGuard,ErrorCode)->Unit)?=null) {
            original(current,requested,id,onFailure)
            val target=(requested as? RequestBody.SessionCreate)?.params?.sourceTarget
            if(target!=null && (creationSource==null || current.reducers[target.sessionId]?.source!=creationSource)) {onFailure?.invoke(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.StaleSession);throw StorePolicy(ErrorCode.StaleSession)}
            mutationContext(current,requested,onFailure)
        }
        private fun activeRecovery(current:Live,requested:RequestBody,id:Uuid,failure:StorePolicy):Job? {
            if(failure.code!=ErrorCode.NotReady || current!==live || sessions[host]!==live || !live.alive.get() ||
                !live.authenticated || id!=request.id || requested!==body) return null
            return current.recovery?.takeIf {it.isActive}
        }
        private fun originalBeforeRequest(current:Live,requested:RequestBody,id:Uuid,onFailure:(ActionAdmissionGuard,ErrorCode)->Unit) =
            original(current,requested,id,onFailure)
        private suspend fun admit(current:Live,requested:RequestBody,id:Uuid,onFailure:(ActionAdmissionGuard,ErrorCode)->Unit) {
            while(true) {
                originalBeforeRequest(current,requested,id,onFailure)
                val canonical=store.localAction(host,request.id)
                originalBeforeRequest(current,requested,id,onFailure)
                try {
                    validate(current,requested,id)
                } catch(failure:StorePolicy) {
                    val recovery=activeRecovery(current,requested,id,failure)
                    if(recovery!=null) {
                        markNewActionContinuationRecovery(host,request.id)
                        if(recovery.isActive) recovery.join()
                        continue
                    }
                    try {
                        validate(current,requested,id,onFailure)
                    } catch(rejected:StorePolicy) {throw rejected}
                    continue
                }
                if(canonical?.authoritative==true && canonical.receipt!=null) throw CanonicalAdmissionReceipt(canonical.receipt)
                return
            }
        }
        private fun originalBeforePreSend() = original(live,body,request.id) {guard,code->
            captureActionAdmission(host,request.id,ActionAdmissionPhase.PrePreSend,guard,code)
        }
        suspend fun beforePreSend() {
            while(true) {
                originalBeforePreSend()
                try {
                    validate(live,body,request.id)
                    return
                } catch(failure:StorePolicy) {
                    val recovery=activeRecovery(live,body,request.id,failure)
                    if(recovery!=null) {
                        markNewActionContinuationRecovery(host,request.id)
                        if(recovery.isActive) recovery.join()
                        continue
                    }
                    validate(live,body,request.id) {guard,code->
                        captureActionAdmission(host,request.id,ActionAdmissionPhase.PrePreSend,guard,code)
                    }
                }
            }
        }
        suspend fun beforeRequest() = admit(live,body,request.id) {guard,code->
            captureActionAdmission(host,request.id,ActionAdmissionPhase.PostPreSend,guard,code)
        }
        fun recoveryFor(current:Live,requested:RequestBody,id:Uuid,failure:StorePolicy):Job? =
            activeRecovery(current,requested,id,failure)
        suspend fun beforeFrame(current:Live,requested:RequestBody,id:Uuid,recovery:Job,onFailure:(ActionAdmissionGuard,ErrorCode)->Unit) {
            if(recovery.isActive) {
                markNewActionContinuationRecovery(host,request.id)
                recovery.join()
            }
            admit(current,requested,id,onFailure)
        }
        fun validateFrame(current:Live,requested:RequestBody,id:Uuid,onFailure:(ActionAdmissionGuard,ErrorCode)->Unit) =
            validate(current,requested,id,onFailure)
        fun admitFrame(current:Live,requested:RequestBody,id:Uuid,admit:()->Unit) {
            if(agentCreation!=null){agentCreation.lease.admit {validate(current,requested,id);admit()}}else if(requested is RequestBody.TerminalCreate){val original=terminal ?: throw StorePolicy(ErrorCode.NotReady);original.lease.admit {validate(current,requested,id);admit()}}else if(requested is RequestBody.ShellCreate || requested is RequestBody.ShellInput) {
                val original=shell ?: throw StorePolicy(ErrorCode.NotReady)
                // No suspension or write under this gate. Revocation wins before
                // admission; once admitted, the existing frame owner finishes.
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested.attachmentTarget()!=null){
                val original=attachment ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested is RequestBody.WorktreesRemove){
                val original=removal ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested is RequestBody.AgentRename) {
                val original=alias ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else admit()
        }
    }
    /** Instrumentation installs one action-bound slot; ordinary operations leave it absent. */
    internal fun beginActionAdmissionCapture(host:String,action:Uuid) {check(actionAdmissionCapture.compareAndSet(null,ActionAdmissionCapture(host,action)))}
    internal fun actionAdmissionFailure(host:String,action:Uuid):ActionAdmissionFailure? = actionAdmissionCapture.get()?.takeIf {it.host==host && it.action==action}?.failure?.get()
    internal fun clearActionAdmissionCapture(host:String,action:Uuid) {actionAdmissionCapture.get()?.takeIf {it.host==host && it.action==action}?.let {actionAdmissionCapture.compareAndSet(it,null)}}
    internal fun pauseNewActionContinuation(host:String,action:Uuid,point:NewActionContinuationBarrierPoint=NewActionContinuationBarrierPoint.AfterPreSend):NewActionContinuationBarrier {
        val barrier=NewActionContinuationBarrier(host,action,point)
        check(newActionContinuationBarrier.compareAndSet(null,barrier));return barrier
    }
    private suspend fun awaitNewActionContinuationBarrier(host:String,action:Uuid,point:NewActionContinuationBarrierPoint) {
        val barrier=newActionContinuationBarrier.get()?.takeIf {it.host==host && it.action==action && it.point==point} ?: return
        barrier.reached.complete(Unit)
        barrier.release.await()
    }
    private fun markNewActionContinuationRecovery(host:String,action:Uuid) {
        newActionContinuationBarrier.get()?.takeIf {it.host==host && it.action==action}?.recoveryWaited?.complete(Unit)
    }
    private fun markNewActionContinuationFrame(host:String,action:Uuid) {
        val barrier=newActionContinuationBarrier.get()?.takeIf {it.host==host && it.action==action} ?: return
        if(barrier.frameStarts.incrementAndGet()==1) barrier.frameStarted.complete(Unit)
    }
    private fun clearNewActionContinuationBarrier(host:String,action:Uuid) {
        newActionContinuationBarrier.get()?.takeIf {it.host==host && it.action==action}?.let {newActionContinuationBarrier.compareAndSet(it,null)}
    }
    private fun captureActionAdmission(host:String,action:Uuid,phase:ActionAdmissionPhase,guard:ActionAdmissionGuard,code:ErrorCode,sent:Boolean?=null,frameStarted:Boolean?=null) {
        actionAdmissionCapture.get()?.takeIf {it.host==host && it.action==action}?.failure?.compareAndSet(null,ActionAdmissionFailure(phase,guard,code,sent,frameStarted))
    }
    suspend fun start():String = hostOperations.withLock {bindEndpoint()}
    private suspend fun bindEndpoint():String = lifecycle.withLock {
        endpoint?.let {return@withLock it.id().use {id->id.toString()}}
        secrets.completeDeletions(store)
        if (!endpointOwned.compareAndSet(false,true)) throw StorePolicy(ErrorCode.Busy)
        var bound:Endpoint?=null
        var bytes:ByteArray?=null
        try {
            IrohAndroid.installAndroidContext(app)
            val secret=secrets.endpoint(store);bytes=secret
            val options=EndpointOptions(preset=object:Preset {override fun apply(builder:EndpointBuilder) {builder.applyN0();builder.disablePortMapping()}},secretKey=secret,alpns=listOf(alpn))
            bound=try {withTimeout(15000) {Endpoint.bind(options)}} finally {options.destroy();secret.fill(0)}
            val owner=bound ?: throw StorePolicy(ErrorCode.NotReady)
            val channel=Channel<Unit>(Channel.CONFLATED)
            val watcher=object:ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network:Network) {channel.trySend(Unit)}
                override fun onLost(network:Network) {channel.trySend(Unit)}
                override fun onCapabilitiesChanged(network:Network,capabilities:NetworkCapabilities) {channel.trySend(Unit)}
                override fun onLinkPropertiesChanged(network:Network,properties:LinkProperties) {channel.trySend(Unit)}
            }
            connectivity.registerDefaultNetworkCallback(watcher)
            callback=watcher;changes=channel
            withTimeout(5000) {owner.networkChange()}
            networkJob=scope.launch {for (ignored in channel) {delay(100);while(channel.tryReceive().isSuccess) {}; try {withTimeout(5000) {owner.networkChange()}} catch (cancel:CancellationException) {if (!isActive) throw cancel} catch (_:Exception) { /* A future connectivity hint may recover the same retained endpoint. */ }}}
            endpoint=owner
            owner.id().use {it.toString()}
        } catch (failure:Exception) {
            bytes?.fill(0)
            withContext(NonCancellable) {
                callback?.let(connectivity::unregisterNetworkCallback);callback=null;changes?.close();changes=null
                networkJob?.cancel();withTimeoutOrNull(5000) {networkJob?.join()};networkJob=null
                bound?.let {try {withTimeoutOrNull(5000) {it.shutdown()}} finally {it.close()}}
                endpoint=null;endpointOwned.set(false)
            }
            if (failure is CancellationException || failure is ProtectedKeyUnavailable) throw failure
            if (failure is computer.iroh.IrohException) failure.close()
            throw StorePolicy(ErrorCode.NotReady)
        }
    }
    /** Store the pin/name and exact protected aliases BEFORE pair.confirm could commit remotely.
     * On response loss only known-peer hello is attempted; never retry the spent token. */
    suspend fun pair(invite:PairingInvitation, active:()->Boolean = {true}):HelloResult = hostOperations.withLock {
        currentCoroutineContext().ensureActive();check(active()) {"Pairing confirmation is inactive"}
        bindEndpoint()
        val alias=SecretStore.hostAlias(invite.host.value,"receipt")
        val previous=store.state.value.hosts.firstOrNull {it.hostId==invite.host.value}
        val retained=previous?.keyAliases?.contains(alias)==true
        val metadata=HostMetadata(invite.host.value,invite.name.value,keyAliases=((previous?.keyAliases ?: emptyList())+SecretStore.ENDPOINT_ALIAS+alias).distinct())
        store.host(metadata)
        val receipt=secrets.material(alias,!retained);receipt.fill(0)
        currentCoroutineContext().ensureActive();check(active()) {"Pairing confirmation is inactive"}
        val attempt=beginConnectionAttempt(metadata.hostId)
        var live=dial(metadata,attempt)
        try {
            val token=invite.token ?: throw StorePolicy(ErrorCode.InvalidRequest)
            currentCoroutineContext().ensureActive();check(active()) {"Pairing confirmation is inactive"}
            try {
                val paired=(request(live,RequestBody.PairConfirm(PairConfirmParams(token,deviceName()))) as ResultBody.PairConfirm).result
                requireWire(paired.hostId==invite.host)
            } catch (failure:Exception) {
                currentCoroutineContext().ensureActive()
                if (failure is RemoteFailure) throw failure
                teardown(live);check(active()) {"Pairing confirmation is inactive"};live=dial(metadata,attempt)
            } finally {invite.clear()}
            val hello=try {hello(live)} catch(failure:Exception) {recordConnectionFailure(attempt,live,ConnectionFailurePhase.Hello,failure);throw failure}
            install(live)
            try {snapshot(metadata.hostId)} catch(failure:Exception) {recordConnectionFailure(attempt,live,ConnectionFailurePhase.Snapshot,failure);throw failure}
            hello
        } catch (failure:Exception) {teardown(live);throw failure}
    }
    suspend fun reconnect(host:String):HelloResult = hostOperations.withLock {reconnectLocked(host)}
    /** Health check and connection replacement share the host-owner mutex. Callers that do not
     * explicitly request a manual reconnect reuse only the authenticated live incarnation. */
    suspend fun ensureConnectedIfNeeded(host:String):HelloResult = hostOperations.withLock {
        sessions[host]?.takeIf { it.authenticated && it.alive.get() && !it.disposed.get() }?.hello
            ?: reconnectLocked(host)
    }
    suspend fun reconnectAtStartup(host:String):HelloResult {
        var initial:ConnectionAttempt?=null
        try {
            return hostOperations.withLock {
                sessions[host]?.takeIf {it.authenticated && it.alive.get() && !it.disposed.get()}?.hello
                    ?: reconnectLocked(host) {initial=it}
            }
        } catch(timeout:TimeoutCancellationException) {
            currentCoroutineContext().ensureActive()
            val attempt=initial ?: throw timeout
            if(!startupHelloTimeoutCurrent(host,attempt)) throw timeout
            // A manual operation can supersede this startup-only attempt while it yields.
            delay(1000)
            currentCoroutineContext().ensureActive()
            return hostOperations.withLock {
                sessions[host]?.takeIf {it.authenticated&&it.alive.get()&&!it.disposed.get()}?.hello?.let {return@withLock it}
                if(!startupHelloTimeoutCurrent(host,attempt)) throw timeout
                reconnectLocked(host)
            }
        }
    }
    private suspend fun reconnectLocked(host:String,onAttempt:(ConnectionAttempt)->Unit = {}):HelloResult {
        store.open();val metadata=store.state.value.hosts.firstOrNull {it.hostId==host} ?: throw StorePolicy(ErrorCode.Unauthorized)
        val attempt=beginConnectionAttempt(host);onAttempt(attempt)
        try {bindEndpoint()} catch(failure:Exception) {recordConnectionFailure(attempt,phase=ConnectionFailurePhase.Dial,failure=failure);throw failure}
        val live=dial(metadata,attempt)
        return try {
            val hello=try {hello(live)} catch(failure:Exception) {recordConnectionFailure(attempt,live,ConnectionFailurePhase.Hello,failure);throw failure}
            install(live)
            try {snapshot(host)} catch(failure:Exception) {recordConnectionFailure(attempt,live,ConnectionFailurePhase.Snapshot,failure);throw failure}
            hello
        } catch (failure:Exception) {teardown(live);throw failure}
    }
    private fun deviceName():Label {val out=StringBuilder();val points=android.os.Build.MODEL.codePoints().iterator();var count=0;while(points.hasNext() && count<128) {val point=points.nextInt();if(point !in 0xd800..0xdfff && !Character.isISOControl(point)) {out.appendCodePoint(point);count++}};return Label(out.toString().ifEmpty {"Android"})}
    private suspend fun dial(metadata:HostMetadata,attempt:ConnectionAttempt,observeInitialHelloHeader:Boolean=false):Live {
        val owner=endpoint ?: run {
            val failure=StorePolicy(ErrorCode.NotReady)
            recordConnectionFailure(attempt,phase=ConnectionFailurePhase.Dial,failure=failure)
            throw failure
        }
        val id=computer.iroh.EndpointId.fromString(metadata.hostId)
        val address=computer.iroh.EndpointAddr(id,attempt.routes.relay,attempt.routes.direct)
        var connection:Connection?=null;var stream:BiStream?=null;var send:SendStream?=null;var recv:RecvStream?=null;var created:Live?=null
        try {
            val connected=try {withTimeout(15000) {
                synchronized(connectionFailureLock) {attempt.hintsSupplied=true};publish()
                owner.connect(address,alpn)
            }} catch(failure:Exception) {recordConnectionFailure(attempt,phase=ConnectionFailurePhase.Dial,failure=failure);throw failure};connection=connected
            try {requireWire(connected.alpn().contentEquals(alpn) && connected.remoteId().use {it.toString()}==metadata.hostId)} catch(failure:Exception) {recordConnectionFailure(attempt,phase=ConnectionFailurePhase.Dial,failure=failure);throw failure}
            val bi=try {withTimeout(10000) {connected.openBi()}} catch(failure:Exception) {recordConnectionFailure(attempt,phase=ConnectionFailurePhase.OpenStream,failure=failure);throw failure};stream=bi
            val sender=bi.send();send=sender;val receiver=bi.recv();recv=receiver
            val live=Live(metadata,attempt,Any(),connected,bi,sender,receiver);created=live
            selectConnectionCandidate(attempt,live)
            if(observeInitialHelloHeader) {
                live.helloReadObservation.set(OwnedReadObservation())
                live.helloHeaderPending.set(true)
            }
            live.watches+=connected.watchPaths(object:PathChangeCallback {
                override suspend fun onChange(paths:List<PathSnapshot>) {withContext(serial) {applySelectedPath(live,paths)}}
            })
            live.watches+=connected.watchPathEvents(object:PathEventCallback {
                override suspend fun onEvent(event:PathEvent) {withContext(serial) {
                    if(event is PathEvent.Lagged && isInstalledLive(live)) refreshSelectedPath(live)
                }}
            })
            live.reader=scope.launch {read(live)}
            return live
        } catch (failure:Exception) {
            recordConnectionFailure(attempt,created,ConnectionFailurePhase.OpenStream,failure)
            val partial=created
            if(partial!=null) teardown(partial) else {connection?.close(0,"closed".toByteArray());send?.close();recv?.close();stream?.close();connection?.close()}
            if(failure is computer.iroh.IrohException) {failure.close();throw StorePolicy(ErrorCode.NotReady)}
            throw failure
        }
        finally {address.close();id.close()}
    }
    private suspend fun hello(live:Live):HelloResult {
        val result=try {
            (request(live,RequestBody.Hello(HelloParams("0.1.0",deviceName()))) as ResultBody.Hello).result
        } finally {
            live.helloHeaderPending.compareAndSet(true,false)
        }
        requireWire(result.hostId.value==live.host.hostId && (live.host.deviceId==null || live.host.deviceId==result.deviceId))
        live.hello=result;live.authenticated=true
        store.host(live.host.copy(deviceId=result.deviceId));return result
    }
    private fun isInstalledLive(live:Live):Boolean = live.authenticated && live.alive.get() && !live.disposed.get() && sessions[live.host.hostId]===live
    private fun applySelectedPath(live:Live,paths:List<PathSnapshot>) {
        if(!isInstalledLive(live)) return
        val selected=paths.singleOrNull {it.isSelected}
        live.selectedRoute=selected?.takeIf {it.id.length in 1..128 && it.remoteAddr.length in 1..2048 && !it.id.any(Char::isISOControl) && !it.remoteAddr.any(Char::isISOControl)}?.let {SelectedIrohPath(it.id,it.remoteAddr,it.isIp,it.isRelay)}
        live.selectedPath=live.selectedRoute?.let {if(it.isRelay) "relay" else if(it.isIp) "direct" else "other"}
        publish()
    }
    private fun refreshSelectedPath(live:Live) {
        if(!isInstalledLive(live)) return
        try {applySelectedPath(live,live.connection.paths())}
        catch(failure:Exception) {
            if(failure is CancellationException) throw failure
            if(failure is computer.iroh.IrohException) failure.close()
            if(isInstalledLive(live)) {live.selectedRoute=null;live.selectedPath=null;publish()}
        }
    }
    private suspend fun install(live:Live) {
        val old=sessions.put(live.host.hostId,live)
        if(old!=null) teardown(old)
        // Seed only after Hello and installation; callbacks from a candidate grant no path claim.
        withContext(serial) {refreshSelectedPath(live);publish()}
    }
    private suspend fun request(live:Live,body:RequestBody,id:Uuid=Uuid(UUID.randomUUID().toString()),admissionAction:Uuid?=null,continuation:NewActionContinuation?=null,inheritedDeadline:Boolean=false,terminal:TerminalReadContext?=null,shell:ShellReadContext?=null,worktrees:WorktreesReadContext?=null,changes:ChangesReadContext?=null,skills:SkillsReadContext?=null,artifact:ArtifactReadContext?=null,apply:suspend (ResultBody)->Unit = {}):ResultBody = withContext(serial) {
        terminal?.let(::validateTerminalRead)
        shell?.let(::validateShellRead)
        worktrees?.let(::validateWorktreesRead)
        changes?.let(::validateChangesRead)
        skills?.let(::validateSkillsRead)
                                    artifact?.let(::validateArtifactRead)
        if (!live.alive.get()) {admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Request,ActionAdmissionGuard.NotAlive,ErrorCode.Unauthorized)};throw StorePolicy(ErrorCode.Unauthorized)}
        if (!live.authenticated && body !is RequestBody.Hello && body !is RequestBody.PairConfirm) {admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Request,ActionAdmissionGuard.Unauthenticated,ErrorCode.Unauthorized)};throw StorePolicy(ErrorCode.Unauthorized)}
        if (!live.inflight.tryAcquire()) {admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Request,ActionAdmissionGuard.InflightBusy,ErrorCode.Busy)};throw StorePolicy(ErrorCode.Busy)}
        val deferred=CompletableDeferred<ResultBody>();val pending=Pending(body.method,deferred,apply)
        if (live.pending.putIfAbsent(id,pending)!=null) {live.inflight.release();admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Request,ActionAdmissionGuard.PendingConflict,ErrorCode.ActionIdConflict)};throw StorePolicy(ErrorCode.ActionIdConflict)}
        var writing:Deferred<Unit>?=null
        try {
            val awaitResult:suspend ()->ResultBody = {
                val caller=currentCoroutineContext()[Job]!!
                // Only the bounded frame write belongs to the owner. Canceled waiters retain
                // method correlation, not another task retaining their request/secret body.
                writing=scope.async(start=CoroutineStart.UNDISPATCHED) {
                    var frameStarted=false
                    try {
                        val bytes=Protocol.encodePayload(Envelope.RequestFrame(Request(id,body)))
                        try {
                            while(true) {
                                val recovery=live.writer.withLock {
                                    caller.ensureActive()
                                    if(!live.alive.get()) {admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,ActionAdmissionGuard.NotAlive,ErrorCode.NotReady,pending.sent,frameStarted)};throw StorePolicy(ErrorCode.NotReady)}
                                    terminal?.let(::validateTerminalRead)
        shell?.let(::validateShellRead)
                                    worktrees?.let(::validateWorktreesRead)
                                    changes?.let(::validateChangesRead)
                                    skills?.let(::validateSkillsRead)
                                    artifact?.let(::validateArtifactRead)
                                    try {
                                        if(continuation==null) mutationContext(live,body) {guard,code->admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,guard,code,pending.sent,frameStarted)}}
                                        else continuation.validateFrame(live,body,id) {_,_->}
                                    } catch(failure:StorePolicy) {
                                        val existing=continuation?.recoveryFor(live,body,id,failure)
                                        if(existing!=null) return@withLock existing
                                        if(continuation==null) {
                                            admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,ActionAdmissionGuard.StorePolicy,failure.code,pending.sent,frameStarted)}
                                            throw failure
                                        }
                                        try {
                                            continuation.validateFrame(live,body,id) {guard,code->
                                                admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,guard,code,pending.sent,frameStarted)}
                                            }
                                        } catch(rejected:StorePolicy) {
                                            admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,ActionAdmissionGuard.StorePolicy,rejected.code,pending.sent,frameStarted)}
                                            throw rejected
                                        }
                                    }
                                    val admit={
                                        if(!pending.admitted.compareAndSet(false,true)) throw CancellationException()
                                        frameStarted=true
                                        markNewActionContinuationFrame(live.host.hostId,id)
                                    }
                                    if(continuation==null) admit() else continuation.admitFrame(live,body,id,admit)
                                    withTimeout(5000) {live.send.writeAll(ByteBuffer.allocate(4).putInt(bytes.size).array());live.send.writeAll(bytes)}
                                    pending.sent=true
                                    null
                                }
                                if(recovery==null) break
                                continuation!!.beforeFrame(live,body,id,recovery) {guard,code->admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,guard,code,pending.sent,frameStarted)}}
                            }
                        } finally {bytes.fill(0)}
                    } catch(failure:Exception) {
                        if(frameStarted) live.connection.close(0,"write failed".toByteArray())
                        if(failure is computer.iroh.IrohException) {failure.close();admissionAction?.let {captureActionAdmission(live.host.hostId,it,ActionAdmissionPhase.Writer,if(frameStarted) ActionAdmissionGuard.IoAfterFrameStart else ActionAdmissionGuard.IoBeforeFrame,ErrorCode.NotReady,pending.sent,frameStarted)};throw StorePolicy(ErrorCode.NotReady)}
                        throw failure
                    } finally {
                        if(!pending.sent && live.pending.remove(id,pending)) {deferred.completeExceptionally(StorePolicy(ErrorCode.NotReady));live.inflight.release()}
                    }
                }
                writing!!.await()
                deferred.await()
            }
            if(inheritedDeadline) awaitResult() else withTimeout(if(body is RequestBody.SessionCreate) 120000L else 15000L) {awaitResult()}
        } catch(failure:Exception) {
            if(body is RequestBody.Hello) recordConnectionFailure(live.attempt,live,ConnectionFailurePhase.Hello,failure,pending.sent,pending.sent && pending.deferred.isCompleted)
            throw failure
        } finally {
            if(pending.admitted.compareAndSet(false,true)) writing?.cancel()
            if(writing==null && live.pending.remove(id,pending)) {deferred.cancel();live.inflight.release()}
        }
    }
    private fun failPending(live:Live) {
        for((id,pending) in live.pending) if(live.pending.remove(id,pending)) {
            captureActionAdmission(live.host.hostId,id,ActionAdmissionPhase.Pending,ActionAdmissionGuard.ConnectionClosed,ErrorCode.NotReady,pending.sent)
            pending.deferred.completeExceptionally(StorePolicy(ErrorCode.NotReady));live.inflight.release()
        }
    }
    private suspend fun read(live:Live) {
        try {
            while (live.alive.get()) {
                live.helloReceivePhase.set(HelloReceivePhase.Header)
                val header=if(live.helloHeaderPending.compareAndSet(true,false)) live.helloReadObservation.get()?.let {live.recv.readExactObserved(4u,it)} ?: live.recv.readExact(4u) else live.recv.readExact(4u)
                val length=Protocol.frameLength(header);requireWire(live.authenticated || length<=4096)
                live.helloReceivePhase.set(HelloReceivePhase.Payload)
                val bytes=live.recv.readExact(length.toUInt())
                try {
                    live.helloReceivePhase.set(HelloReceivePhase.Decode)
                    val raw=Protocol.parseRawJson(bytes) as? JsonObject ?: invalidWire()
                    val responseId=(raw["id"] as? JsonPrimitive)?.takeIf {it.isString}?.content?.let(::Uuid)
                    val pending=responseId?.let(live.pending::get)
                    // Canceled/timed-out callers keep their bounded method correlation until
                    // a strictly validated reply or close; untracked replies are still fatal.
                    if(raw["type"]?.jsonPrimitive?.content=="response" && pending==null) throw StorePolicy(ErrorCode.InvalidRequest)
                    when(val frame=Protocol.decodePayload(bytes,pending?.method)) {
                        is Envelope.ResponseFrame->{
                            val response=frame.response
                            val expected=live.pending.remove(response.id) ?: throw StorePolicy(ErrorCode.InvalidRequest)
                            try {
                                if(response.error!=null) {expected.deferred.completeExceptionally(RemoteFailure(response.error.code));live.helloReceivePhase.set(HelloReceivePhase.Completed)}
                                else {val result=response.result ?: invalidWire();live.helloReceivePhase.set(HelloReceivePhase.Apply);expected.apply(result);expected.deferred.complete(result);live.helloReceivePhase.set(HelloReceivePhase.Completed)}
                            } catch(failure:StorePolicy) {
                                captureActionAdmission(live.host.hostId,response.id,ActionAdmissionPhase.ResponseApply,ActionAdmissionGuard.StorePolicy,failure.code,expected.sent)
                                when(failure.code) {
                                    ErrorCode.CursorExpired,ErrorCode.StaleSession,ErrorCode.ContentExpired,ErrorCode.NotReady->{expected.deferred.completeExceptionally(failure);live.helloReceivePhase.set(HelloReceivePhase.Completed)}
                                    else->{expected.deferred.completeExceptionally(StorePolicy(ErrorCode.NotReady));live.helloReceivePhase.set(HelloReceivePhase.Completed);throw failure}
                                }
                            } catch(failure:Exception) {captureActionAdmission(live.host.hostId,response.id,ActionAdmissionPhase.ResponseApply,ActionAdmissionGuard.Other,ErrorCode.NotReady,expected.sent);expected.deferred.completeExceptionally(StorePolicy(ErrorCode.NotReady));live.helloReceivePhase.set(HelloReceivePhase.Completed);throw failure}
                            finally {live.inflight.release()}
                        }
                        is Envelope.EventFrame->{if(!live.authenticated) throw StorePolicy(ErrorCode.Unauthorized);event(live,frame.event)}
                        is Envelope.RequestFrame->invalidWire()
                    }
                } finally {bytes.fill(0)}
            }
        } catch (failure:Exception) {
            recordConnectionFailure(live.attempt,live,ConnectionFailurePhase.Reader,failure)
            live.alive.set(false);live.authenticated=false;live.connection.close(0,"closed".toByteArray())
            live.hostReducer.disconnected();live.reducers.values.forEach {it.disconnected()}
            failPending(live);publish()
            if(failure is computer.iroh.IrohException) failure.close()
            teardown(live)
            if(failure is CancellationException) throw failure
        }
    }
    private suspend fun event(live:Live,event:Event) {
        val host=live.host.hostId
        if(event.subscription==live.hostReducer.inventory?.subscription) {
            if(event.body is EventBody.SyncRequired)notificationWakeupMutable.tryEmit(NotificationWakeup(host,true))
            val decision=live.hostReducer.event(event)
            if(decision==SessionReducer.Decision.Resnapshot) {recover(live);return}
            if(decision==SessionReducer.Decision.Duplicate) return
            when(val body=event.body) {
                is EventBody.ActionStatus->{if(store.localAction(host,body.data.receipt.actionId)!=null) committedReceipt(live,body.data.receipt)}
                is EventBody.NotificationUpsert->{store.liveNotification(host,body.data.event);notificationWakeupMutable.tryEmit(NotificationWakeup(host))}
                is EventBody.NotificationRead->{store.readConfirmed(host,body.data.read);notificationWakeupMutable.tryEmit(NotificationWakeup(host))}
                else->invalidWire()
            }
        } else {
            // Ordered unsubscribe ACKs drain the old source; untracked streams cannot alter the current view.
            val session=live.subscriptions[event.subscription] ?: return
            val reducer=live.reducers[session] ?: throw StorePolicy(ErrorCode.InvalidRequest)
            val decision=reducer.event(event)
            if(decision==SessionReducer.Decision.Resnapshot) {live.resnapshot.add(session);recover(live);return}
            if(decision==SessionReducer.Decision.Applied && (event.body is EventBody.HistoryUpsert || event.body is EventBody.SessionState)) cache(live,reducer)
        }
        publish()
    }
    /** Coalesce read-only recovery in the existing live owner. Not-ready installations wait for
     * authoritative inventory/readiness hints, never replaying or retargeting a mutation. */
    private fun recover(live:Live) {
        live.receiptsRecovered=false;live.recoveryRequested=true;publish()
        if(live.recovery!=null || !live.alive.get() || sessions[live.host.hostId]!==live) return
        live.recovery=scope.launch {
            try {
                while(live.recoveryRequested && live.alive.get() && sessions[live.host.hostId]===live) {
                    live.recoveryRequested=false
                    live.operations.withLock {
                        if(!live.alive.get() || sessions[live.host.hostId]!==live) return@withLock
                        snapshot(live)
                        for(session in live.resnapshot.toList()) {
                            if(!live.hostReducer.ready) break
                            val summary=live.hostReducer.inventory?.sessions?.firstOrNull {it.sessionId==session && it.connectivity==SessionConnectivity.Ready} ?: continue
                            try {open(live,Target(summary.sessionId,summary.generation))}
                            catch(failure:RemoteFailure) {if(failure.code!=ErrorCode.NotReady && failure.code!=ErrorCode.StaleSession) throw failure}
                        }
                        recoverReceipts(live)
                    }
                }
            } catch(timeout:TimeoutCancellationException) {
                recordConnectionFailure(live.attempt,live,ConnectionFailurePhase.Recovery,timeout)
                teardown(live);publish()
            } catch(cancel:CancellationException) {recordConnectionFailure(live.attempt,live,ConnectionFailurePhase.Recovery,cancel);throw cancel}
            catch(failure:Exception) {
                recordConnectionFailure(live.attempt,live,ConnectionFailurePhase.Recovery,failure)
                if(failure is computer.iroh.IrohException) failure.close()
                teardown(live);publish()
            } finally {live.recovery=null}
        }
    }
    private suspend fun cache(live:Live,reducer:SessionReducer,snapshot:Boolean=false) {
        val summary=reducer.state?.summary ?: return
        val source=reducer.source ?: return
        store.cache(live.host.hostId,summary,reducer.history.map(SafeDisplayHistory::redacted),reducer.partial,snapshot=snapshot,source=source)
    }
    private fun publish() {
        mutable.value=ConnectionView(
            hello=sessions.filterValues {it.authenticated&&it.alive.get()}.mapValues {it.value.hello!!},
            inventory=sessions.mapNotNull {(host,live)->live.hostReducer.inventory?.let {host to it}}.toMap(),
            sessions=sessions.mapValues {it.value.reducers.values.toList()},
            selectedPath=sessions.filterValues {isInstalledLive(it)}.mapNotNull {(host,live)->live.selectedPath?.let {host to it}}.toMap(),
            selectedRoutes=sessions.filterValues {isInstalledLive(it)}.mapNotNull {(host,live)->live.selectedRoute?.let {host to it}}.toMap(),
            // A failed replacement attempt must not obscure the installed connection's configuration.
            activeRoutes=sessions.filterValues {isInstalledLive(it)}.mapValues {(_,live)->live.attempt.routes.copy(supplied=true)},
            routeAttempts=synchronized(connectionFailureLock) {connectionAttempts.mapValues {it.value.routes.copy(supplied=it.value.hintsSupplied)}},
            inventoryReady=sessions.mapValues {it.value.authenticated&&it.value.alive.get()&&it.value.hostReducer.ready},
            sources=sessions.mapValues {(_,live)->if(!live.authenticated||!live.alive.get()||!live.hostReducer.ready) emptyMap() else live.reducers.values.mapNotNull {reducer->val s=reducer.state?.summary;val source=reducer.source;if(s==null||source==null||s.connectivity!=SessionConnectivity.Ready||!live.hostReducer.sourceCurrent(s.serverId)) null else Target(s.sessionId,s.generation) to source}.toMap()},
            shellInputStatus=sessions.filterValues {isInstalledLive(it)}.mapValues {(_,live)->live.shellInputStatuses.mapValues {(_,record)->record.observation}},
            latestReceipt=sessions.mapNotNull {(host,live)->live.latestReceipt?.let {host to it}}.toMap(),
            mutationReady=sessions.mapValues {it.value.authenticated&&it.value.alive.get()&&it.value.hostReducer.ready&&it.value.receiptsRecovered},
            connectionFailures=connectionFailureSnapshot(),
        )
    }
    private fun live(host:String):Live=sessions[host]?.takeIf {it.authenticated&&it.alive.get()} ?: throw StorePolicy(ErrorCode.NotReady)
    suspend fun snapshot(host:String):HostSnapshotResult = withContext(serial) {
        val live=live(host)
        val snapshot=try {live.operations.withLock {live.receiptsRecovered=false;publish();val result=snapshot(live);recoverReceipts(live);result}}
        catch(failure:Exception) {recordConnectionFailure(live.attempt,live,ConnectionFailurePhase.Snapshot,failure);throw failure}
        if(live.resnapshot.isNotEmpty()) recover(live)
        snapshot
    }
    private suspend fun snapshot(live:Live):HostSnapshotResult = (request(live,RequestBody.HostSnapshot(HostSnapshotParams())) {result->val snapshot=(result as ResultBody.HostSnapshot).result;live.hostReducer.snapshot(snapshot);store.cacheInventory(live.host.hostId,snapshot);publish()} as ResultBody.HostSnapshot).result
    suspend fun open(host:String,target:Target):SessionOpenResult = withContext(serial) {
        val live=live(host)
        try {live.operations.withLock {val result=open(live,target);recoverReceipts(live);result}}
        catch(failure:RemoteFailure) {if((failure.code==ErrorCode.NotReady || failure.code==ErrorCode.StaleSession) && target.sessionId in live.resnapshot) recover(live);throw failure}
    }
    private suspend fun open(live:Live,target:Target):SessionOpenResult {
        val caller=currentCoroutineContext()[Job]!!
        live.receiptsRecovered=false;publish()
        // Keep each old route/reducer until its ordered unsubscribe response has applied.
        for((subscription,session) in live.subscriptions) if(session==target.sessionId) unsubscribe(live,subscription)
        live.reducers[target.sessionId]?.disconnected();publish()
        try {
            return (request(live,RequestBody.SessionOpen(SessionOpenParams(target))) {result->
                val snapshot=(result as ResultBody.SessionOpen).result
                requireWire(Target(snapshot.state.summary.sessionId,snapshot.state.summary.generation)==target)
                if(!caller.isActive) {abandonSubscription(live,snapshot.subscription);return@request}
                val reducer=live.reducers.getOrPut(target.sessionId) {SessionReducer()}
                live.subscriptions[snapshot.subscription]=target.sessionId
                reducer.snapshot(snapshot);live.resnapshot.remove(target.sessionId)
                cache(live,reducer,true)
                if(caller.isActive) store.viewing(live.host.hostId,target.sessionId)
                publish()
                if(!caller.isActive) abandonSubscription(live,snapshot.subscription)
            } as ResultBody.SessionOpen).result
        } catch(failure:RemoteFailure) {
            if((failure.code==ErrorCode.NotReady || failure.code==ErrorCode.StaleSession) &&
                (live.reducers.containsKey(target.sessionId) || live.hostReducer.inventory?.sessions?.any {it.sessionId==target.sessionId}==true)) live.resnapshot.add(target.sessionId)
            throw failure
        }
    }
    /** A decoded abandoned open never enters the current reducer/cache. Cleanup waits outside
     * the reader, since another open may hold operations while awaiting that same reader. */
    private fun abandonSubscription(live:Live,subscription:Uuid) {
        scope.launch(start=CoroutineStart.UNDISPATCHED) {
            try {live.operations.withLock {
                if(live.alive.get() && sessions[live.host.hostId]===live) unsubscribe(live,subscription)
            }}
            catch(cancel:CancellationException) {throw cancel}
            catch(failure:Exception) {if(failure is computer.iroh.IrohException) failure.close();teardown(live);publish()}
        }
    }
    suspend fun history(host:String,target:Target,cursor:Id?=null,limit:Uint?=null):SessionHistoryResult = withContext(serial) {val live=live(host);val reducer=live.reducers[target.sessionId] ?: throw StorePolicy(ErrorCode.NotReady);val ticket=reducer.historyPage(target,cursor);(request(live,RequestBody.SessionHistory(SessionHistoryParams(target,cursor,limit,ticket.viewId))) {result->reducer.page(ticket,(result as ResultBody.SessionHistory).result);cache(live,reducer,snapshot=true);publish()} as ResultBody.SessionHistory).result}
    private suspend fun unsubscribe(live:Live,subscription:Uuid) {
        request(live,RequestBody.SessionUnsubscribe(SessionUnsubscribeParams(subscription))) {
            live.subscriptions.remove(subscription)?.let {session->live.reducers[session]?.takeIf {it.source==subscription}?.disconnected()}
            publish()
        }
    }
    /** Resolve the CURRENT route under the existing owner, including automatic reopen. */
    suspend fun leave(host:String,target:Target) = withContext(serial) {
        val live=sessions[host]
        if(live!=null) live.operations.withLock {
            for((subscription,session) in live.subscriptions.toMap()) if(session==target.sessionId && live.alive.get()) unsubscribe(live,subscription)
            live.resnapshot.remove(target.sessionId);live.reducers[target.sessionId]?.disconnected()
        }
        store.viewing(host,null);publish()
    }
    private fun contentSource(host:String,target:Target,source:Uuid):Pair<Live,SessionReducer> {
        val live=live(host);val reducer=live.reducers[target.sessionId] ?: throw StorePolicy(ErrorCode.NotReady)
        val summary=reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!live.hostReducer.ready || !live.hostReducer.sourceCurrent(summary.serverId) || summary.connectivity!=SessionConnectivity.Ready || reducer.source!=source || Target(summary.sessionId,summary.generation)!=target) throw StorePolicy(ErrorCode.ContentExpired)
        return live to reducer
    }
    /** Every chunk and completed admission rechecks this exact subscription, never a UI epoch. */
    suspend fun content(host:String,source:Uuid,params:ContentReadParams):ContentReadResult = withContext(serial) {
        val (live,_)=contentSource(host,params.target,source)
        val result=(request(live,RequestBody.ContentRead(params)) as ResultBody.ContentRead).result
        contentSource(host,params.target,source);result
    }
    /** Terminal text is transient: retain the original live/reducer/source triple. */
    private fun terminalContext(host:String,target:Target,source:Uuid):TerminalReadContext {
        val (live,reducer)=contentSource(host,target,source)
        if(reducer.state?.summary?.capabilities?.terminalRead!=true) throw StorePolicy(ErrorCode.NotReady)
        return TerminalReadContext(host,live,reducer,target,source)
    }
    private fun validateTerminalRead(context:TerminalReadContext) {
        if(!context.active.get() || sessions[context.host]!==context.live || !context.live.alive.get() || context.live.reducers[context.target.sessionId]!==context.reducer) throw StorePolicy(ErrorCode.StaleSession)
        val summary=context.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!context.live.hostReducer.ready || !context.live.hostReducer.sourceCurrent(summary.serverId) || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.terminalRead || context.reducer.source!=context.source || Target(summary.sessionId,summary.generation)!=context.target) throw StorePolicy(ErrorCode.StaleSession)
    }
    private fun worktreesContext(host:String,target:Target,source:Uuid):WorktreesReadContext {
        val (live,reducer)=contentSource(host,target,source)
        if(reducer.state?.summary?.capabilities?.worktreesRead!=true) throw StorePolicy(ErrorCode.NotReady)
        return WorktreesReadContext(host,live,reducer,target,source)
    }
    private fun validateWorktreesRead(context:WorktreesReadContext) {
        if(!context.active.get() || sessions[context.host]!==context.live || !context.live.alive.get() || context.live.reducers[context.target.sessionId]!==context.reducer) throw StorePolicy(ErrorCode.StaleSession)
        val summary=context.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!context.live.hostReducer.ready || !context.live.hostReducer.sourceCurrent(summary.serverId) || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.worktreesRead || context.reducer.source!=context.source || Target(summary.sessionId,summary.generation)!=context.target) throw StorePolicy(ErrorCode.StaleSession)
    }
    private fun validateSkillsRead(context:SkillsReadContext) {
        if(!context.active.get() || sessions[context.host]!==context.live || !context.live.alive.get() || !context.live.authenticated || context.live.disposed.get() || context.live.reducers[context.target.sessionId]!==context.reducer) throw StorePolicy(ErrorCode.StaleSession)
        val summary=context.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!context.live.hostReducer.ready || !context.live.hostReducer.sourceCurrent(summary.serverId) || summary.provider !in listOf(Provider.Codex,Provider.Omp) || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.skillsRead || (if(context.catalogueRefresh&&summary.provider==Provider.Omp)summary.skillsGeneration==null else summary.skillsGeneration!=context.generation) || context.reducer.source!=context.source || Target(summary.sessionId,summary.generation)!=context.target) throw StorePolicy(ErrorCode.StaleSession)
    }
    suspend fun skills(host:String,target:Target,source:Uuid,generation:Uuid):SkillsListResult = withContext(serial) {
        val (live,reducer)=contentSource(host,target,source)
        val context=SkillsReadContext(host,live,reducer,target,source,generation)
        validateSkillsRead(context)
        context.catalogueRefresh=true
        try {
            val result=(request(live,RequestBody.SkillsList(SkillsListParams(target)),skills=context) { response->
                validateSkillsRead(context)
                val result=(response as ResultBody.SkillsList).result
                if(result.target!=target || result.provider!=reducer.state?.summary?.provider || result.catalogueGeneration!=reducer.state?.summary?.skillsGeneration || result.provider==Provider.Codex&&result.catalogueGeneration!=generation) throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.SkillsList).result
            validateSkillsRead(context)
            if(result.target!=target || result.provider!=reducer.state?.summary?.provider || result.catalogueGeneration!=reducer.state?.summary?.skillsGeneration || result.provider==Provider.Codex&&result.catalogueGeneration!=generation) throw StorePolicy(ErrorCode.StaleSession)
            result
        } finally {context.active.set(false)}
    }
    /** One complete secure preview. No URI, file path or text is persisted. */
    internal suspend fun skillsDocument(host:String,target:Target,source:Uuid,generation:Uuid,document:Uuid):String {
        val caller=currentCoroutineContext()[Job] ?: throw StorePolicy(ErrorCode.NotReady)
        return withContext(serial){
            val(live,reducer)=contentSource(host,target,source);val context=SkillsReadContext(host,live,reducer,target,source,generation)
            validateSkillsRead(context);if(reducer.state?.summary?.provider!=Provider.Omp)throw StorePolicy(ErrorCode.NotReady)
            var observed:Uuid?=null;var abandoned=false;var owned=false;val cleanupStarted=AtomicBoolean(false)
            suspend fun close(read:Uuid){
                if(!live.alive.get()||!live.authenticated||live.disposed.get())return
                // Exact original memory ownership remains valid for cleanup only.
                request(live,RequestBody.SkillsPreviewClose(SkillsPreviewCloseParams(target,generation,read)))
            }
            fun cleanup(){val read=observed ?: return;if(cleanupStarted.compareAndSet(false,true))scope.launch{try{withTimeout(5000){close(read)}}catch(_:Exception){}}}
            try {
                val opened=(request(live,RequestBody.SkillsPreviewOpen(SkillsPreviewOpenParams(target,generation,document)),skills=context){reply->
                    val result=(reply as ResultBody.SkillsPreviewOpen).result
                    if(result.target!=target||result.catalogueGeneration!=generation||result.documentRef!=document)throw StorePolicy(ErrorCode.StaleSession)
                    // Pending callback owns the read ID before cancellation/viewer checks.
                    observed=result.readId
                    try{if(abandoned||!caller.isActive)throw StorePolicy(ErrorCode.StaleSession);validateSkillsRead(context);owned=true}finally{if(!owned)cleanup()}
                }as ResultBody.SkillsPreviewOpen).result
                validateSkillsRead(context);caller.ensureActive()
                val bytes=java.io.ByteArrayOutputStream(opened.size.value.toInt());var offset=0L
                while(true){
                    caller.ensureActive();validateSkillsRead(context)
                    val read=(request(live,RequestBody.SkillsPreviewRead(SkillsPreviewReadParams(target,generation,opened.readId,Uint(offset),Uint(65536))),skills=context){reply->
                        validateSkillsRead(context);val chunk=(reply as ResultBody.SkillsPreviewRead).result
                        if(chunk.target!=target||chunk.catalogueGeneration!=generation||chunk.readId!=opened.readId||chunk.offset.value!=offset)throw StorePolicy(ErrorCode.StaleSession)
                    }as ResultBody.SkillsPreviewRead).result
                    caller.ensureActive();validateSkillsRead(context)
                    val chunk=java.util.Base64.getDecoder().decode(read.data.value)
                    if(chunk.size>65536||offset+chunk.size>opened.size.value||chunk.isEmpty()&&!read.eof||read.eof!=(offset+chunk.size==opened.size.value))throw StorePolicy(ErrorCode.InvalidRequest)
                    bytes.write(chunk);offset+=chunk.size;if(read.eof)break
                }
                val complete=bytes.toByteArray();val digest=java.security.MessageDigest.getInstance("SHA-256").digest(complete).joinToString(""){"%02x".format(it)}
                if(offset!=opened.size.value||digest!=opened.sha256)throw StorePolicy(ErrorCode.InvalidRequest)
                val text=Charsets.UTF_8.newDecoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT).onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT).decode(java.nio.ByteBuffer.wrap(complete)).toString()
                if(text.any{Character.isISOControl(it)&&it!='\n'&&it!='\r'&&it!='\t'})throw StorePolicy(ErrorCode.InvalidRequest)
                caller.ensureActive();validateSkillsRead(context);text
            }finally{abandoned=true;context.active.set(false);cleanup()}
        }
    }
    private fun changesContext(host:String,target:Target,source:Uuid):ChangesReadContext {
        val (live,reducer)=contentSource(host,target,source)
        if(reducer.state?.summary?.capabilities?.changesRead!=true) throw StorePolicy(ErrorCode.NotReady)
        return ChangesReadContext(host,live,reducer,target,source)
    }
    private fun validateChangesRead(context:ChangesReadContext) {
        if(!context.active.get() || sessions[context.host]!==context.live || !context.live.alive.get() || context.live.reducers[context.target.sessionId]!==context.reducer) throw StorePolicy(ErrorCode.StaleSession)
        val summary=context.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!context.live.hostReducer.ready || !context.live.hostReducer.sourceCurrent(summary.serverId) || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.changesRead || context.reducer.source!=context.source || Target(summary.sessionId,summary.generation)!=context.target) throw StorePolicy(ErrorCode.StaleSession)
    }
    suspend fun terminal(host:String,target:Target,source:Uuid):TerminalReadResult = withContext(serial) {
        val context=terminalContext(host,target,source)
        var delivered=false
        try {
            val result=(request(context.live,RequestBody.TerminalRead(TerminalReadParams(target)),terminal=context) { response->
                validateTerminalRead(context)
                if((response as ResultBody.TerminalRead).result.target!=context.target) throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.TerminalRead).result
            if(result.target!=target) throw StorePolicy(ErrorCode.StaleSession)
            validateTerminalRead(context)
            delivered=true
            result
        } finally {
            if(!delivered) context.active.set(false)
        }
    }
    /** This inventory has no cache or reducer mutation. Every delayed response remains bound to
     * the live connection, reducer, original Target and source captured at dialog open. */
    suspend fun worktrees(host:String,target:Target,source:Uuid):WorktreesListResult = withContext(serial) {
        val context=worktreesContext(host,target,source)
        var delivered=false
        try {
            val result=(request(context.live,RequestBody.WorktreesList(WorktreesListParams(target)),worktrees=context) { response->
                validateWorktreesRead(context)
                if((response as ResultBody.WorktreesList).result.target!=context.target) throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.WorktreesList).result
            if(result.target!=target) throw StorePolicy(ErrorCode.StaleSession)
            validateWorktreesRead(context)
            if(context.live.worktreeCreationScopes.size>=512 && !context.live.worktreeCreationScopes.containsKey(target)) context.live.worktreeCreationScopes.clear()
            result.creationScope?.let {context.live.worktreeCreationScopes[target]=source to it} ?: context.live.worktreeCreationScopes.remove(target)
            delivered=true
            result
        } finally {
            if(!delivered) context.active.set(false)
        }
    }
    /** Changes are intentionally uncached: every dialog request retains its original live/reducer/source fence. */
    suspend fun changes(host:String,target:Target,source:Uuid):ChangesReadResult = withContext(serial) {
        val context=changesContext(host,target,source)
        var delivered=false
        try {
            val result=(request(context.live,RequestBody.ChangesRead(ChangesReadParams(target)),changes=context) { response->
                validateChangesRead(context)
                if((response as ResultBody.ChangesRead).result.target!=context.target) throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.ChangesRead).result
            if(result.target!=target) throw StorePolicy(ErrorCode.StaleSession)
            validateChangesRead(context); delivered=true; result
        } finally { if(!delivered) context.active.set(false) }
    }
    suspend fun cacheContent(host:String,target:Target,source:Uuid,reference:ContentRef,text:String) = withContext(serial) {
        val (_,reducer)=contentSource(host,target,source)
        if(reducer.history.none {r->r.complete && (r.text==reference || r.original==reference || r.tool?.input==reference || r.tool?.output==reference)}) throw StorePolicy(ErrorCode.InvalidRequest)
        store.cacheContent(host,target,source,reference,text)
        contentSource(host,target,source);Unit
    }
    suspend fun cachedContent(host:String,target:Target,source:Uuid,reference:ContentRef):String? = withContext(serial) {
        val (_,reducer)=contentSource(host,target,source)
        if(reducer.history.none {r->r.complete && (r.text==reference || r.original==reference || r.tool?.input==reference || r.tool?.output==reference)}) throw StorePolicy(ErrorCode.InvalidRequest)
        val text=store.content(host,target,source,reference.handle)
        contentSource(host,target,source);text
    }
    // A sync completion is caller-driven reconciliation, never a delivery wakeup. Emitting here
    // would make foreground read confirmation recursively request another sync page.
    internal suspend fun reconcilePush(host:String,expected:HelloResult)=withContext(serial){
        val original=live(host)
        fun admitted(){if(sessions[host]!==original||!original.alive.get()||!original.authenticated||original.hello!==expected||!original.hostReducer.ready)throw StorePolicy(ErrorCode.NotReady)}
        admitted();var after:Uint?=null;var pages=0
        do{
            admitted();if(++pages>100)throw StorePolicy(ErrorCode.ResourceLimit)
            val previous=after
            val result=(request(original,RequestBody.NotificationsSync(NotificationsSyncParams(after,Uint(100)))){reply->admitted();store.notifications(host,(reply as ResultBody.NotificationsSync).result);admitted()}as ResultBody.NotificationsSync).result
            admitted();after=result.nextSequence;if(after!=null&&after.value<=(previous?.value ?: 0L))throw StorePolicy(ErrorCode.InvalidRequest)
        }while(after!=null)
    }
    internal suspend fun registerPush(host:String,registration:LocalPushRegistration):PushRegisterResult=withContext(serial){
        val original=live(host);val boot=original.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady);val endpoint=registration.endpoint ?: throw StorePolicy(ErrorCode.NotReady)
        val result=(request(original,RequestBody.PushRegister(PushRegisterParams(boot,endpoint,registration.generation,Key32(registration.key),registration.probe))){reply->
            if(sessions[host]!==original||!original.alive.get()||!original.authenticated)throw StorePolicy(ErrorCode.NotReady)
            val result=(reply as ResultBody.PushRegister).result;if(result.endpointGeneration!=registration.generation||result.probe!=registration.probe)throw StorePolicy(ErrorCode.PushRejected)
        }as ResultBody.PushRegister).result;result
    }
    internal suspend fun acknowledgePush(host:String,registration:LocalPushRegistration):PushProbeAckResult=withContext(serial){
        val original=live(host);val boot=original.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        (request(original,RequestBody.PushProbeAck(PushProbeAckParams(boot,registration.generation,registration.probe))){
            if(sessions[host]!==original||!original.alive.get()||!original.authenticated)throw StorePolicy(ErrorCode.NotReady)
        }as ResultBody.PushProbeAck).result
    }
    internal suspend fun unregisterPush(host:String,generation:Uuid):PushUnregisterResult=withContext(serial){
        val original=live(host);val boot=original.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        (request(original,RequestBody.PushUnregister(PushUnregisterParams(boot,generation))){if(sessions[host]!==original||!original.alive.get())throw StorePolicy(ErrorCode.NotReady)}as ResultBody.PushUnregister).result
    }
    suspend fun notificationsSync(host:String,after:Uint?=null,limit:Uint?=null):NotificationsSyncResult = (request(live(host),RequestBody.NotificationsSync(NotificationsSyncParams(after,limit))) {result->store.notifications(host,(result as ResultBody.NotificationsSync).result)} as ResultBody.NotificationsSync).result
    suspend fun markRead(host:String,session:Uuid,through:Uint):ReadWatermark {val live=live(host);val boot=live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady);val params=store.readRequest(host,boot,session,through);return (request(live,RequestBody.NotificationsRead(params)) {result->store.readConfirmed(host,(result as ResultBody.NotificationsRead).result.read);notificationWakeupMutable.tryEmit(NotificationWakeup(host))} as ResultBody.NotificationsRead).result.read}
    /** A coalesced UI inspect hint, derived only from the durable canonical committed outcome.
     * An ignored old frame cannot regress it; it carries no payload and grants no readiness. */
    private fun nextShellStatusOrder(live:Live):Long {
        if(live.statusObservationOrder==Long.MAX_VALUE)throw StorePolicy(ErrorCode.NotReady)
        return ++live.statusObservationOrder
    }
    private fun applyShellStatus(live:Live,action:Uuid,order:Long,unknown:Boolean) {
        if(!isInstalledLive(live) || order<=live.statusObservationFloor || live.shellInputStatuses[action]?.let {it.order>=order}==true)return
        live.shellInputStatuses.remove(action)
        if(live.shellInputStatuses.size>=128) {
            val oldest=live.shellInputStatuses.minBy {it.value.order}
            live.statusObservationFloor=maxOf(live.statusObservationFloor,oldest.value.order)
            live.shellInputStatuses.remove(oldest.key)
            if(order<=live.statusObservationFloor)return
        }
        // Known tombstones retain order too. The eviction floor prevents an
        // older in-flight result from reviving an evicted observation.
        live.shellInputStatuses[action]=ShellInputStatusObservation(order,ShellInputObservation(Uuid.random(),unknown))
    }
    private suspend fun committedReceipt(live:Live,receipt:Receipt,statusOrder:Long?=null) {
        // Capture once before storage suspends; never restamp an older callback.
        val order=statusOrder ?: nextShellStatusOrder(live)
        val prior=store.localAction(live.host.hostId,receipt.actionId)
        store.receipt(live.host.hostId,receipt,true)
        val canonical=store.localAction(live.host.hostId,receipt.actionId)
        if(!isInstalledLive(live))return
        if(canonical?.authoritative==true) {
            live.latestReceipt=canonical.receipt
            if(canonical.metadata.method==ActionMethod.ShellInput && (statusOrder!=null || prior?.authoritative!=true || prior.receipt!=canonical.receipt))applyShellStatus(live,receipt.actionId,order,false)
        }
    }
    suspend fun status(host:String,action:Uuid):ActionStatusResult = withContext(serial) {status(live(host),action)}
    private suspend fun status(live:Live,action:Uuid):ActionStatusResult {
        val order=nextShellStatusOrder(live)
        return (request(live,RequestBody.ActionStatus(ActionStatusParams(action))) {result->
            val status=(result as ResultBody.ActionStatus).result
            val local=store.localAction(live.host.hostId,action)
            if(!isInstalledLive(live))throw StorePolicy(ErrorCode.StaleSession)
            when(status) {
                is ActionStatusResult.Known->{
                    if(status.value.receipt.actionId!=action)invalidWire()
                    if(local!=null)committedReceipt(live,status.value.receipt,order)
                }
                is ActionStatusResult.Unknown->if(local?.metadata?.method==ActionMethod.ShellInput)applyShellStatus(live,action,order,true)
            }
            // committedReceipt has storage awaits. Recheck the same original
            // Live after them before admitting any current observation to UI.
            if(!isInstalledLive(live))throw StorePolicy(ErrorCode.StaleSession)
            publish()
        } as ResultBody.ActionStatus).result
    }
    suspend fun recoverReceipts(host:String) = withContext(serial) {val live=live(host);live.operations.withLock {recoverReceipts(live)}}
    private suspend fun recoverReceipts(live:Live) {
        live.receiptsRecovered=false;publish();var after:Uuid?=null
        while(true) {val actions=store.localActions(live.host.hostId,after,100);for(action in actions) status(live,action.metadata.actionId);if(actions.size<100) break;after=actions.last().metadata.actionId}
        live.reducers.values.forEach {it.recoveryReconciled()};live.receiptsRecovered=true;publish()
    }
    private fun mutationContext(live:Live,body:RequestBody,onFailure:((ActionAdmissionGuard,ErrorCode)->Unit)?=null) {
        fun reject(guard:ActionAdmissionGuard,code:ErrorCode):Nothing {onFailure?.invoke(guard,code);throw StorePolicy(code)}
        val (boot,target)=when(body) {is RequestBody.TerminalCreate->body.params.bootId to body.params.target;is RequestBody.AttachmentsBegin->body.params.bootId to body.params.target;is RequestBody.AttachmentsChunk->body.params.bootId to body.params.target;is RequestBody.AttachmentsComplete->body.params.bootId to body.params.target;is RequestBody.AttachmentsDiscard->body.params.bootId to null;is RequestBody.WorktreesRemove->body.params.bootId to body.params.target;is RequestBody.AgentRename->body.params.bootId to body.params.target;is RequestBody.WorkspaceRename->body.params.bootId to body.params.target;is RequestBody.PromptSubmit->body.params.bootId to body.params.target;is RequestBody.TurnCancel->body.params.bootId to body.params.target;is RequestBody.RequestRespond->body.params.bootId to body.params.target;is RequestBody.SessionCreate->body.params.bootId to body.params.sourceTarget;is RequestBody.ShellCreate->body.params.bootId to null;is RequestBody.ShellInput->body.params.bootId to null;else->return}
        if(boot!=live.hello?.bootId) reject(ActionAdmissionGuard.BootChanged,ErrorCode.StaleHost)
        if(body !is RequestBody.AttachmentsDiscard && !live.hostReducer.ready) reject(ActionAdmissionGuard.HostNotReady,ErrorCode.NotReady)
        if(!live.receiptsRecovered) reject(ActionAdmissionGuard.ReceiptsUnreconciled,ErrorCode.NotReady)
        if(body is RequestBody.ShellCreate || body is RequestBody.ShellInput) {
            val shellTarget=when(body){is RequestBody.ShellCreate->body.params.target;is RequestBody.ShellInput->body.params.target;else->throw StorePolicy(ErrorCode.InvalidRequest)}
            val shell=live.hostReducer.inventory?.shells?.singleOrNull {it.target==shellTarget} ?: reject(ActionAdmissionGuard.TargetChanged,ErrorCode.StaleSession)
            if(shell.connectivity!=SessionConnectivity.Ready || !shell.capabilities.read || (if(body is RequestBody.ShellInput)!shell.capabilities.input else !shell.capabilities.create) || !live.hostReducer.sourceCurrent(shell.serverId)) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        }
        if(target!=null) {val reducer=live.reducers[target.sessionId] ?: reject(ActionAdmissionGuard.ReducerAbsent,ErrorCode.NotReady);val summary=reducer.state?.summary ?: reject(ActionAdmissionGuard.SummaryAbsent,ErrorCode.NotReady);if(Target(summary.sessionId,summary.generation)!=target) reject(ActionAdmissionGuard.TargetChanged,ErrorCode.StaleSession);if(!reducer.mutationsEnabled) reject(ActionAdmissionGuard.MutationsDisabled,ErrorCode.NotReady);if(!live.hostReducer.sourceCurrent(summary.serverId)) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)}
        if(body.attachmentTarget()!=null && body !is RequestBody.AttachmentsDiscard && live.reducers[body.attachmentTarget()!!.sessionId]?.state?.summary?.capabilities?.attachments!=true)reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.TerminalCreate&&live.reducers[body.params.target.sessionId]?.state?.summary?.capabilities?.terminalCreate!=true)reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.WorktreesRemove && live.reducers[body.params.target.sessionId]?.state?.summary?.capabilities?.worktreeRemove!=true)reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.AgentRename && live.reducers[body.params.target.sessionId]?.state?.summary?.capabilities?.agentRename!=true) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.SessionCreate && body.params.sourceTarget!=null) {
            val sourceTarget=body.params.sourceTarget
            val reducer=live.reducers[sourceTarget.sessionId] ?: reject(ActionAdmissionGuard.ReducerAbsent,ErrorCode.NotReady)
            val summary=reducer.state?.summary ?: reject(ActionAdmissionGuard.SummaryAbsent,ErrorCode.NotReady)
            val scope=live.worktreeCreationScopes[sourceTarget] ?: reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
            if(scope.first!=reducer.source || !scope.second.canCreate || scope.second.projectId!=body.params.projectId || summary.serverId!=body.params.serverId || !summary.capabilities.worktreeCreate) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
            if(live.hostReducer.inventory?.projects?.none {it.projectId==body.params.projectId && it.available}!=false) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        }

    }
    /** Capture actual immutable owner references before any suspension or system-picker launch. */
    internal fun captureAttachment(host:String,target:Target,source:Uuid,expected:SessionReducer,resume:AttachmentsStatusResult?=null):AttachmentMutationContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val original=AttachmentContext(host,live,expected,target,source,live.subscriptions[target.sessionId])
        val context=AttachmentMutationContext(original,AttachmentActionLease(),host,target,source,resumeStatus=resume)
        validateAttachment(context,live,null);return context
    }
    internal suspend fun captureAttachmentCleanup(record:LocalAttachmentUpload):AttachmentMutationContext=withContext(serial) {
        val live=originalLive(record.host,Uuid.random())
        if(store.attachmentUploads(record.host).none {it.uploadId==record.uploadId && it.target==record.target})throw StorePolicy(ErrorCode.NotReady)
        if(sessions[record.host]!==live)throw StorePolicy(ErrorCode.NotReady)
        AttachmentMutationContext(AttachmentContext(record.host,live,null,record.target,null,null),AttachmentActionLease(),record.host,record.target,null,record.uploadId)
    }
    private fun validateAttachment(context:AttachmentMutationContext,live:Live,body:RequestBody?) {
        context.lease.admit {
            val original=context.original as? AttachmentContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
            if(original.live!==live || sessions[original.host]!==live || !live.alive.get() || !live.authenticated || context.target!=original.target)throw StorePolicy(ErrorCode.StaleSession)
            context.resumeStatus?.let {expected->if(live.attachmentStatuses[expected.progress.uploadId]!==expected || expected.target!=context.target || expected.state!=AttachmentUploadState.Receiving || body!=null&&body.attachmentUpload()!=expected.progress.uploadId)throw StorePolicy(ErrorCode.NotReady)}
            if(body!=null && body.attachmentTarget()!=original.target)throw StorePolicy(ErrorCode.InvalidRequest)
            if(context.cleanupUpload!=null){if(body!=null && (body !is RequestBody.AttachmentsDiscard || body.params.uploadId!=context.cleanupUpload))throw StorePolicy(ErrorCode.InvalidRequest);return@admit}
            val reducer=original.reducer ?: throw StorePolicy(ErrorCode.NotReady)
            val summary=reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
            if(live.reducers[original.target.sessionId]!==reducer || original.source==null || reducer.source!=original.source || live.subscriptions[original.target.sessionId]!=original.subscription || Target(summary.sessionId,summary.generation)!=original.target || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.attachments || !reducer.mutationsEnabled || !live.hostReducer.ready || !live.hostReducer.sourceCurrent(summary.serverId))throw StorePolicy(ErrorCode.NotReady)
        }
    }
    internal fun attachmentBoot(context:AttachmentMutationContext):Uuid {val original=context.original as? AttachmentContext ?: throw StorePolicy(ErrorCode.InvalidRequest);validateAttachment(context,original.live,null);return original.live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady)}
    internal fun attachmentStatusCurrent(context:AttachmentMutationContext,status:AttachmentsStatusResult):Boolean {val original=context.original as? AttachmentContext ?: return false;return attachmentCurrent(context)&&original.live.attachmentStatuses[status.progress.uploadId]===status}
    internal fun attachmentCurrent(context:AttachmentMutationContext):Boolean {return try {val original=context.original as? AttachmentContext ?: return false;validateAttachment(context,original.live,null);true}catch(_:Exception){false}}
    internal suspend fun attachmentStatus(host:String,target:Target,upload:Uuid,context:AttachmentMutationContext?=null):AttachmentsStatusResult=withContext(serial) {
        val original=live(host)
        val result=(request(original,RequestBody.AttachmentsStatus(AttachmentsStatusParams(target,upload))) {body->
            if(sessions[host]!==original || !original.alive.get() || !original.authenticated)throw StorePolicy(ErrorCode.NotReady)
            val status=(body as ResultBody.AttachmentsStatus).result
            if(status.target!=target||status.progress.uploadId!=upload)throw StorePolicy(ErrorCode.InvalidRequest)
        } as ResultBody.AttachmentsStatus).result
        if(sessions[host]!==original)throw StorePolicy(ErrorCode.NotReady)
        val observed=if(context==null || !attachmentCurrent(context) || (context.original as? AttachmentContext)?.live!==original)result.copy(insertionReady=false)else result
        if(original.attachmentStatuses.size>=32 && !original.attachmentStatuses.containsKey(upload))throw StorePolicy(ErrorCode.ResourceLimit)
        original.attachmentStatuses[upload]=observed;observed
    }
    internal suspend fun previewWorktreeRemoval(host:String,target:Target,source:Uuid,candidate:Uuid,ticket:Long,lease:WorktreeRemovalLease):WorktreeRemovalMutationContext=withContext(serial) {
        val original=worktreesContext(host,target,source);var delivered=false
        try {
            lease.admit {validateWorktreesRead(original);if(original.reducer.state?.summary?.capabilities?.worktreeRemove!=true)throw StorePolicy(ErrorCode.NotReady)}
            val result=(request(original.live,RequestBody.WorktreesRemovePreview(WorktreesRemovePreviewParams(target,candidate)),worktrees=original){response->
                lease.admit {validateWorktreesRead(original)}
                val preview=(response as ResultBody.WorktreesRemovePreview).result
                if(preview.target!=target || preview.candidateId!=candidate)throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.WorktreesRemovePreview).result
            lease.admit {validateWorktreesRead(original)}
            if(result.target!=target || result.candidateId!=candidate)throw StorePolicy(ErrorCode.StaleSession)
            delivered=true
            WorktreeRemovalMutationContext(original,lease,result,ticket,android.os.SystemClock.elapsedRealtime()+25000)
        }finally{if(!delivered)original.active.set(false)}
    }
    private fun validateWorktreeRemoval(context:WorktreeRemovalMutationContext,live:Live,body:RequestBody.WorktreesRemove) {
        context.lease.admit {
            val original=context.original as? WorktreesReadContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
            if(original.live!==live || sessions[original.host]!==live || context.preview.target!=body.params.target || context.preview.candidateId!=body.params.candidateId || context.preview.previewId!=body.params.previewId || !context.preview.removable || android.os.SystemClock.elapsedRealtime()>=context.expiresAt)throw StorePolicy(ErrorCode.StaleSession)
            validateWorktreesRead(original);mutationContext(live,body)
        }
    }
    internal fun captureArtifacts(host:String,source:ArtifactSource,expected:SessionReducer?,subscription:Uuid,epoch:Uuid?):ArtifactReadContext{
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val shell=(source as? ArtifactSource.Shell)?.let{live.hostReducer.inventory?.shells?.singleOrNull{row->row.target==it.target}}
        if(!live.hostReducer.ready||live.hostReducer.inventory?.subscription!=subscription)throw StorePolicy(ErrorCode.NotReady)
        val server=shell?.serverId ?: expected?.state?.summary?.serverId ?: throw StorePolicy(ErrorCode.NotReady)
        val incarnation=live.hostReducer.inventory?.hierarchy?.singleOrNull{it.serverId==server}?.serverIncarnation ?: throw StorePolicy(ErrorCode.NotReady)
        return ArtifactReadContext(ArtifactOriginal(host,live,live.hostReducer,subscription,source,expected,epoch,shell,server,incarnation)).also(::validateArtifactRead)
    }
    internal fun artifactCurrent(context:ArtifactReadContext):Boolean=try{validateArtifactRead(context);true}catch(_:Exception){false}
    // Pending inventory is a pause, never a new source or permission. All other
    // failures permanently invalidate this browser's original authority.
    internal fun artifactIdentityCurrent(context:ArtifactReadContext):Boolean=try{validateArtifactRead(context);true}catch(failure:StorePolicy){failure.code==ErrorCode.NotReady}catch(_:Exception){false}
    private fun validateArtifactRead(context:ArtifactReadContext){
        val original=context.original as? ArtifactOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest);val live=original.live
        if(!context.active.get()||sessions[original.host]!==live||!live.alive.get()||!live.authenticated||live.disposed.get()||live.hostReducer!==original.hostReducer)throw StorePolicy(ErrorCode.StaleSession)
        when(val source=original.source){
            is ArtifactSource.Session->{val reducer=original.reducer ?: throw StorePolicy(ErrorCode.StaleSession);val summary=reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
                if(live.reducers[source.target.sessionId]!==reducer||Target(summary.sessionId,summary.generation)!=source.target||summary.serverId!=original.server||reducer.source!=null&&reducer.source!=original.epoch)throw StorePolicy(ErrorCode.StaleSession)
                if(reducer.source==null)throw StorePolicy(ErrorCode.NotReady)
            }
            is ArtifactSource.Shell->{val shell=live.hostReducer.inventory?.shells?.singleOrNull{it.target==source.target}
                if(shell==null){if(!live.hostReducer.ready)throw StorePolicy(ErrorCode.NotReady);throw StorePolicy(ErrorCode.StaleSession)}
                val pinned=original.shell ?: throw StorePolicy(ErrorCode.StaleSession)
                if(shell.serverId!=pinned.serverId||shell.workspaceId!=pinned.workspaceId||shell.tabId!=pinned.tabId||shell.paneId!=pinned.paneId||shell.terminalId!=pinned.terminalId)throw StorePolicy(ErrorCode.StaleSession)
            }
        }
        if(!live.hostReducer.ready||!live.hostReducer.sourceCurrent(original.server))throw StorePolicy(ErrorCode.NotReady)
        // Host inventory subscriptions may renew; session subscription and all
        // captured resource/process authority remain the originals.
        if(live.hostReducer.inventory?.hierarchy?.singleOrNull{it.serverId==original.server}?.serverIncarnation!=original.incarnation)throw StorePolicy(ErrorCode.StaleSession)
        when(val source=original.source){
            is ArtifactSource.Session->{val summary=original.reducer?.state?.summary ?: throw StorePolicy(ErrorCode.NotReady);if(summary.connectivity!=SessionConnectivity.Ready)throw StorePolicy(ErrorCode.NotReady);if(!summary.capabilities.artifacts)throw StorePolicy(ErrorCode.StaleSession)}
            is ArtifactSource.Shell->{val inventory=live.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady);val shell=inventory.shells.singleOrNull{it.target==source.target} ?: throw StorePolicy(if(live.hostReducer.ready)ErrorCode.StaleSession else ErrorCode.NotReady);if(shell.connectivity!=SessionConnectivity.Ready)throw StorePolicy(ErrorCode.NotReady);if(!shell.capabilities.artifacts)throw StorePolicy(ErrorCode.StaleSession)}
        }
    }
    internal suspend fun artifactList(context:ArtifactReadContext,parent:Uuid?=null,cursor:Uuid?=null):ArtifactsListResult=withContext(serial){
        validateArtifactRead(context);val original=context.original as ArtifactOriginal
        val result=(request(original.live,RequestBody.ArtifactsList(ArtifactsListParams(original.source,parent,cursor,Uint(100))),artifact=context){reply->validateArtifactRead(context);if((reply as ResultBody.ArtifactsList).result.source!=original.source)throw StorePolicy(ErrorCode.StaleSession)}as ResultBody.ArtifactsList).result;validateArtifactRead(context);result
    }
    internal suspend fun artifactOpen(context:ArtifactReadContext,file:Uuid,onOwned:(Uuid)->Unit):ArtifactsOpenResult {
        val caller=currentCoroutineContext()[Job] ?: throw StorePolicy(ErrorCode.NotReady)
        return withContext(serial){
            validateArtifactRead(context);val original=context.original as ArtifactOriginal
            var observed:Uuid?=null;var transferred=false;var abandoned=false
            val cleanupStarted=AtomicBoolean(false)
            fun cleanup(){val read=observed ?: return;if(!transferred&&cleanupStarted.compareAndSet(false,true))scope.launch {try{withTimeout(5000){artifactClose(context,read)}}catch(_:Exception){}}}
            try {
                val result=(request(original.live,RequestBody.ArtifactsOpen(ArtifactsOpenParams(original.source,file)),artifact=context){reply->
                    val opened=(reply as ResultBody.ArtifactsOpen).result
                    if(opened.source!=original.source)throw StorePolicy(ErrorCode.StaleSession)
                    // The bounded pending request owns the observed memory lease before
                    // viewer/caller validation. A late or rejected reply closes only it.
                    observed=opened.readId
                    try {
                        if(abandoned||!caller.isActive)throw StorePolicy(ErrorCode.StaleSession)
                        validateArtifactRead(context)
                        onOwned(opened.readId);transferred=true
                    } finally {if(!transferred)cleanup()}
                }as ResultBody.ArtifactsOpen).result
                validateArtifactRead(context);result
            } finally {abandoned=true;cleanup()}
        }
    }
    internal suspend fun artifactRead(context:ArtifactReadContext,read:Uuid,offset:Long):ArtifactsReadResult=withContext(serial){
        validateArtifactRead(context);val original=context.original as ArtifactOriginal
        val result=(request(original.live,RequestBody.ArtifactsRead(ArtifactsReadParams(original.source,read,Uint(offset),Uint(65536))),artifact=context){reply->validateArtifactRead(context);val readReply=(reply as ResultBody.ArtifactsRead).result;if(readReply.source!=original.source||readReply.readId!=read||readReply.offset.value!=offset)throw StorePolicy(ErrorCode.StaleSession)}as ResultBody.ArtifactsRead).result;validateArtifactRead(context);result
    }
    internal suspend fun artifactClose(context:ArtifactReadContext,read:Uuid)=withContext(serial){
        val original=context.original as? ArtifactOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(sessions[original.host]!==original.live||!original.live.alive.get()||!original.live.authenticated)throw StorePolicy(ErrorCode.StaleSession)
        // Closing only an already-owned memory lease needs no new Ready source.
        val result=(request(original.live,RequestBody.ArtifactsClose(ArtifactsCloseParams(original.source,read)))as ResultBody.ArtifactsClose).result;if(result.source!=original.source||result.readId!=read)throw StorePolicy(ErrorCode.StaleSession)
    }
    /** Presentation origin only: the configured project remains the sole new-session directory authority. */
    internal fun captureAgentSession(host:String,target:Target,source:Uuid,expected:SessionReducer):AgentSessionMutationContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val summary=expected.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        val subscription=live.subscriptions.entries.singleOrNull {it.value==target.sessionId}?.key ?: throw StorePolicy(ErrorCode.NotReady)
        val original=AgentSessionContext(host,live,live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady),expected,target,source,subscription,summary.serverId,summary.provider,summary.workspaceId,summary.paneId,summary.terminalId)
        val context=AgentSessionMutationContext(original,AgentSessionLease())
        validateAgentSessionOrigin(context,live);return context
    }
    internal fun agentSessionCurrent(context:AgentSessionMutationContext):Boolean=try {
        val original=context.original as? AgentSessionContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        validateAgentSessionOrigin(context,original.live);true
    }catch(_:Exception){false}
    private fun validateAgentSessionOrigin(context:AgentSessionMutationContext,live:Live)=context.lease.admit {
        val original=context.original as? AgentSessionContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(original.live!==live||sessions[original.host]!==live||!live.alive.get()||!live.authenticated||live.hello?.bootId!=original.boot||live.reducers[original.target.sessionId]!==original.reducer||original.reducer.source!=original.source||original.subscription==null||live.subscriptions[original.subscription]!=original.target.sessionId)throw StorePolicy(ErrorCode.StaleSession)
        val summary=original.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(Target(summary.sessionId,summary.generation)!=original.target||summary.serverId!=original.server||summary.provider!=original.provider||summary.workspaceId!=original.workspace||summary.paneId!=original.pane||summary.terminalId!=original.terminal)throw StorePolicy(ErrorCode.StaleSession)
        if(!original.reducer.mutationsEnabled||!live.hostReducer.ready||!live.hostReducer.sourceCurrent(original.server))throw StorePolicy(ErrorCode.NotReady)
    }
    private fun validateAgentSession(context:AgentSessionMutationContext,live:Live,body:RequestBody.SessionCreate)=context.lease.admit {
        validateAgentSessionOrigin(context,live)
        val original=context.original as AgentSessionContext
        if(body.params.serverId!=original.server||body.params.provider!=original.provider||body.params.sourceTarget!=null||body.params.checkout!=null)throw StorePolicy(ErrorCode.InvalidRequest)
        mutationContext(live,body)
    }
    internal fun captureAgentTerminal(host:String,target:Target,source:Uuid,expected:SessionReducer):AgentTerminalMutationContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val original=AgentTerminalContext(host,live,expected,target,source,live.subscriptions[target.sessionId])
        val context=AgentTerminalMutationContext(original,AgentTerminalLease())
        validateAgentTerminal(context,live,RequestBody.TerminalCreate(TerminalCreateParams(live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady),target)));return context
    }
    private fun validateAgentTerminal(context:AgentTerminalMutationContext,live:Live,body:RequestBody.TerminalCreate){context.lease.admit {
        val original=context.original as? AgentTerminalContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(original.live!==live||sessions[original.host]!==live||!live.alive.get()||!live.authenticated||body.params.target!=original.target||live.reducers[original.target.sessionId]!==original.reducer||original.reducer.source!=original.source||live.subscriptions[original.target.sessionId]!=original.subscription)throw StorePolicy(ErrorCode.StaleSession)
        mutationContext(live,body)
    }}
    internal suspend fun captureAgentAlias(host:String,target:Target,source:Uuid,lease:AgentAliasLease,name:AgentName):AgentAliasMutationContext=withContext(serial) {
        val live=live(host);val reducer=live.reducers[target.sessionId] ?: throw StorePolicy(ErrorCode.NotReady)
        val context=AgentAliasMutationContext(AgentAliasContext(host,live,reducer,target,source),lease)
        validateAgentAlias(context,live,RequestBody.AgentRename(AgentRenameParams(live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady),target,name)))
        context
    }
    private fun validateAgentAlias(context:AgentAliasMutationContext,live:Live,body:RequestBody.AgentRename) {
        context.lease.admit {
            val original=context.original as? AgentAliasContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
            if(original.live!==live || sessions[original.host]!==live || !live.alive.get() || !live.authenticated || original.target!=body.params.target || live.reducers[original.target.sessionId]!==original.reducer || original.reducer.source!=original.source) throw StorePolicy(ErrorCode.StaleSession)
            mutationContext(live,body)
        }
    }
    /** Ordinary recovery consent is action-bound; secret forms are memory-only. Call only for an
     * explicitly pressed Send/Stop/Answer after authoritative snapshots+receipt reconciliation. */
    suspend fun mutate(host:String,request:Request,ordinaryComposer:Boolean=false,approval:RecoveryApproval=RecoveryApproval(),shell:ShellMutationContext?=null,alias:AgentAliasMutationContext?=null,removal:WorktreeRemovalMutationContext?=null,attachment:AttachmentMutationContext?=null,terminal:AgentTerminalMutationContext?=null,agentCreation:AgentSessionMutationContext?=null):Receipt = withContext(serial) {
        val live=originalLive(host,request.id);val body=request.body
        if(body.attachmentTarget()==null && body !is RequestBody.TerminalCreate && body !is RequestBody.WorktreesRemove && body !is RequestBody.AgentRename && body !is RequestBody.WorkspaceRename && body !is RequestBody.PromptSubmit && body !is RequestBody.TurnCancel && body !is RequestBody.RequestRespond && body !is RequestBody.SessionCreate && body !is RequestBody.ShellCreate && body !is RequestBody.ShellInput) throw StorePolicy(ErrorCode.InvalidRequest)
        if(agentCreation!=null)validateAgentSession(agentCreation,live,body as? RequestBody.SessionCreate ?: throw StorePolicy(ErrorCode.InvalidRequest))
        if((body is RequestBody.TerminalCreate)!=(terminal!=null))throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.TerminalCreate)validateAgentTerminal(terminal!!,live,body)
        if((body is RequestBody.ShellCreate || body is RequestBody.ShellInput)!=(shell!=null)) throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.ShellCreate || body is RequestBody.ShellInput) validateShellMutation(shell!!,live,body)
        if((body is RequestBody.AgentRename)!=(alias!=null)) throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.AgentRename) validateAgentAlias(alias!!,live,body)
        if((body.attachmentTarget()!=null)!=(attachment!=null))throw StorePolicy(ErrorCode.InvalidRequest)
        if(attachment!=null)validateAttachment(attachment,live,body)
        if((body is RequestBody.WorktreesRemove)!=(removal!=null))throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.WorktreesRemove)validateWorktreeRemoval(removal!!,live,body)
        val continuation=NewActionContinuation(host,live,request,currentCoroutineContext()[Job]!!,shell,alias,removal,attachment,terminal,agentCreation)
        val requestDeadline=if(body is RequestBody.SessionCreate) 120000L else 15000L
        val receipt=try {
            withTimeout(requestDeadline) {
                val existing=store.localAction(host,request.id)
                // The initial local lookup suspends. Recheck this invocation's original live
                // context before a new pre-send action can commit.
                if(existing==null) {
                    awaitNewActionContinuationBarrier(host,request.id,NewActionContinuationBarrierPoint.BeforePreSend)
                    continuation.beforePreSend()
                }
                val receiptKey=secrets.receipt(host,store)
                val decision=try {
                    if(existing==null) continuation.beforePreSend()
                    store.preSend(host,request,receiptKey,ordinaryComposer,approval)
                } finally {receiptKey.fill(0)}
                if(decision is PreSendDecision.ExistingAction) return@withTimeout null
                awaitNewActionContinuationBarrier(host,request.id,NewActionContinuationBarrierPoint.AfterPreSend)
                continuation.beforeRequest()
                val result=this@GoatrConnection.request(live,body,request.id,admissionAction=request.id,continuation=continuation,inheritedDeadline=true)
                when(result) {is ResultBody.TerminalCreate->result.result.receipt;is ResultBody.AttachmentsBegin->result.result.receipt;is ResultBody.AttachmentsChunk->result.result.receipt;is ResultBody.AttachmentsComplete->result.result.receipt;is ResultBody.AttachmentsDiscard->result.result.receipt;is ResultBody.WorktreesRemove->result.result.receipt;is ResultBody.AgentRename->result.result.receipt;is ResultBody.WorkspaceRename->result.result.receipt;is ResultBody.PromptSubmit->result.result.receipt;is ResultBody.TurnCancel->result.result.receipt;is ResultBody.RequestRespond->result.result.receipt;is ResultBody.SessionCreate->result.result.receipt;is ResultBody.ShellCreate->result.result.receipt;is ResultBody.ShellInput->result.result.receipt;else->invalidWire()}
            }
        } catch(settled:CanonicalAdmissionReceipt) {settled.receipt}
        finally {clearNewActionContinuationBarrier(host,request.id)}
        if(receipt==null) {
            return@withContext when(val status=status(host,request.id)) {is ActionStatusResult.Known->status.value.receipt;is ActionStatusResult.Unknown->{captureActionAdmission(host,request.id,ActionAdmissionPhase.ExistingStatus,ActionAdmissionGuard.Unknown,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}}
        }
        committedReceipt(live,receipt);publish();receipt
    }
    suspend fun unpair(host:String) = hostOperations.withLock {val live=live(host);try {request(live,RequestBody.DeviceUnpair(DeviceUnpairParams(live.hello!!.bootId)))} finally {disconnectHost(host)};forgetLocal(host)}
    suspend fun forget(host:String) = hostOperations.withLock {forgetLocal(host)}
    /** Consent is provided by the explicit installation-Forget UI, never startup/key loss. */
    suspend fun forgetInstallation() = hostOperations.withLock {
        store.open()
        if(store.state.value.hosts.isNotEmpty()) throw StorePolicy(ErrorCode.Busy)
        shutdownEndpoint()
        val obligation=store.forgetInstallation()
        secrets.delete(obligation,store)
        publish()
    }
    private suspend fun forgetLocal(host:String) {disconnectHost(host);val obligation=store.forget(host);if(SecretStore.ENDPOINT_ALIAS in obligation.aliases) shutdownEndpoint();secrets.delete(obligation,store)}
    suspend fun disconnect(host:String) = hostOperations.withLock {disconnectHost(host)}
    private suspend fun disconnectHost(host:String) {clearConnectionAttempt(host);sessions.remove(host)?.let {teardown(it)};publish()}
    private suspend fun teardown(live:Live) {
        val caller=currentCoroutineContext()[Job]
        withContext(NonCancellable+serial) {
            if(!live.disposed.compareAndSet(false,true)) {if(live.reader!=caller) withTimeoutOrNull(5000) {live.teardownDone.await()};return@withContext}
            retiring[live]=Unit;live.alive.set(false);live.authenticated=false;live.shellInputStatuses.clear();live.selectedRoute=null;live.selectedPath=null;live.connection.close(0,"closed".toByteArray())
            live.receiptsRecovered=false;publish();live.recovery?.cancel()
            try {
                withTimeoutOrNull(5000) {live.writer.withLock {};if(live.reader!=caller) live.reader?.cancelAndJoin();for(watch in live.watches) watch.stop();live.send.reset(0uL);live.recv.stop(0uL)}
            } catch(closed:computer.iroh.IrohException) {
                closed.close();if(live.connection.closeReason()==null) throw StorePolicy(ErrorCode.NotReady)
            } finally {
                live.reader?.cancel();live.watches.forEach {it.close()};live.watches.clear();live.send.close();live.recv.close();live.stream.close();live.connection.close()
                failPending(live);live.hostReducer.disconnected();live.reducers.values.forEach {it.disconnected()}
                retiring.remove(live);live.teardownDone.complete(Unit)
            }
        }
    }
    suspend fun shutdown() = hostOperations.withLock {shutdownEndpoint()}
    private suspend fun shutdownEndpoint() = lifecycle.withLock {withContext(NonCancellable) {
        val owner=endpoint ?: return@withContext
        try {callback?.let(connectivity::unregisterNetworkCallback)} finally {
            callback=null;changes?.close();changes=null
            networkJob?.cancel();withTimeoutOrNull(5000) {networkJob?.join()};networkJob=null
            clearConnectionAttempts()
            for(live in (sessions.values.toList()+retiring.keys.toList()).distinct()) teardown(live);sessions.clear();publish()
            try {withTimeout(5000) {owner.shutdown()}} finally {owner.close();endpoint=null;endpointOwned.set(false)}
        }
    }}
    internal fun ownsNetworkCallback():Boolean=callback!=null
    /** Count-only observation of the existing serialized owner, used by native consumer tests. */
    internal suspend fun pendingReplies(host:String,sentOnly:Boolean=true):Int = withContext(serial) {sessions[host]?.pending?.values?.count {!sentOnly || it.sent} ?: 0}
    suspend fun recreate():String = hostOperations.withLock {shutdownEndpoint();bindEndpoint()}
    private fun shellContext(host:String,target:ShellTarget,source:Uuid):ShellReadContext {
        val live=live(host)
        val snapshot=live.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        val shell=snapshot.shells.singleOrNull {it.target==target} ?: throw StorePolicy(ErrorCode.StaleSession)
        return ShellReadContext(host,live,live.hostReducer,source,shell).also(::validateShellRead)
    }
    private fun validateShellRead(context:ShellReadContext) {
        if(!context.active.get() || sessions[context.host]!==context.live || !context.live.alive.get() || !context.live.authenticated || context.live.hostReducer!==context.hostReducer) throw StorePolicy(ErrorCode.StaleSession)
        val inventory=context.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        if(!context.hostReducer.ready || inventory.subscription!=context.subscription || !context.hostReducer.sourceCurrent(context.shell.serverId)) throw StorePolicy(ErrorCode.StaleSession)
        val current=inventory.shells.singleOrNull {it.target==context.shell.target} ?: throw StorePolicy(ErrorCode.StaleSession)
        if(current!=context.shell || current.connectivity!=SessionConnectivity.Ready || !current.capabilities.read) throw StorePolicy(ErrorCode.NotReady)
    }
    /** Admission only. Invalidating the viewer lease never cancels a committed frame. */
    internal suspend fun captureShellMutation(host:String,target:ShellTarget,source:Uuid,lease:ShellMutationLease,purpose:ShellMutationPurpose=ShellMutationPurpose.Create,inputObservation:Pair<Uuid,Uuid>?=null):ShellMutationContext=withContext(serial) {
        if(!lease.active.get()) throw StorePolicy(ErrorCode.StaleSession)
        val original=shellContext(host,target,source)
        if(if(purpose==ShellMutationPurpose.Create)!original.shell.capabilities.create else !original.shell.capabilities.input) throw StorePolicy(ErrorCode.NotReady)
        if(inputObservation!=null && (purpose!=ShellMutationPurpose.Input || original.live.shellInputStatuses[inputObservation.first]?.observation?.token!=inputObservation.second))throw StorePolicy(ErrorCode.NotReady)
        ShellMutationContext(original,lease,purpose,inputObservation)
    }
    private fun validateShellMutation(context:ShellMutationContext,live:Live,body:RequestBody) {
        val (target,purpose)=when(body){is RequestBody.ShellCreate->body.params.target to ShellMutationPurpose.Create;is RequestBody.ShellInput->body.params.target to ShellMutationPurpose.Input;else->throw StorePolicy(ErrorCode.InvalidRequest)}
        if(context.purpose!=purpose) throw StorePolicy(ErrorCode.InvalidRequest)
        context.inputObservation?.let {(action,token)->if(purpose!=ShellMutationPurpose.Input || live.shellInputStatuses[action]?.observation?.token!=token)throw StorePolicy(ErrorCode.NotReady)}
        if(!context.lease.active.get()) throw StorePolicy(ErrorCode.StaleSession)
        val original=context.original as? ShellReadContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(original.live!==live || original.host!=live.host.hostId || original.shell.target!=target) throw StorePolicy(ErrorCode.StaleSession)
        validateShellRead(original)
        if(if(purpose==ShellMutationPurpose.Create)!original.shell.capabilities.create else !original.shell.capabilities.input) throw StorePolicy(ErrorCode.NotReady)
    }
    /** Memory-only terminal read on the original authenticated host inventory. */
    suspend fun shell(host:String,target:ShellTarget,source:Uuid):ShellReadResult=withContext(serial) {
        val context=shellContext(host,target,source)
        try {
            val result=(request(context.live,RequestBody.ShellRead(ShellReadParams(target)),shell=context) {response->
                validateShellRead(context)
                if((response as ResultBody.ShellRead).result.target!=target) throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.ShellRead).result
            validateShellRead(context)
            if(result.target!=target) throw StorePolicy(ErrorCode.StaleSession)
            result
        } finally {context.active.set(false)}
    }

}

