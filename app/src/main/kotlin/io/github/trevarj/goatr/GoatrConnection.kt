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
data class ShellInputObservation(val token:Uuid,val unknown:Boolean)
private data class ShellInputStatusObservation(val order:Long,val observation:ShellInputObservation)
internal enum class ShellMutationPurpose {Input,Resize}
class ShellMutationContext internal constructor(internal val original:Any,internal val lease:ShellMutationLease,internal val purpose:ShellMutationPurpose,internal val inputObservation:Pair<Uuid,Uuid>?=null,internal val grid:Any?=null,internal val gridSubscription:Uuid?=null)
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

/** Original viewer ownership, shared by its automatic read-only reopen routes. */
class ConversationSubscription internal constructor(internal val original:Any,internal val host:String,internal val target:Target) {
    internal val active=AtomicBoolean(true)
}
/** One sheet's entry gate; invalidation never cancels an already-admitted frame. */
internal class SkillInvocationLease {
    val active=AtomicBoolean(true)
    private val gate=Any()
    fun invalidate(){synchronized(gate){active.set(false)}}
    fun admit(block:()->Unit)=synchronized(gate){if(!active.get())throw StorePolicy(ErrorCode.StaleSession);block()}
}
/** A fresh explicit invocation, separate from the completed catalogue read ticket. */
class SkillInvocationContext internal constructor(internal val publication:Any?,internal val catalogue:SkillsListResult,internal val reference:Uuid?,internal val lease:SkillInvocationLease,internal val admitted:()->Boolean)
internal class NativeControlsLease {
    val active=AtomicBoolean(true)
    private val gate=Any()
    fun invalidate(){synchronized(gate){active.set(false)}}
    fun admit(block:()->Unit)=synchronized(gate){if(!active.get())throw StorePolicy(ErrorCode.StaleSession);block()}
}
class ControlsMutationContext internal constructor(internal val publication:Any?,internal val catalogue:SessionControlsReadResult,internal val lease:NativeControlsLease,internal val admitted:()->Boolean)
class QueueMutationContext internal constructor(internal val publication:Any?,internal val lease:NativeControlsLease,internal val admitted:()->Boolean)

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
    private class ShellReadContext(val host:String,val live:Live,val hostReducer:HostReducer,val shell:ShellSummary) {
        val active=AtomicBoolean(true)
        val incarnation=hostReducer.inventory?.hierarchy?.singleOrNull {it.serverId==shell.serverId}?.serverIncarnation ?: throw StorePolicy(ErrorCode.NotReady)
    }
    private class TerminalReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid) {
        /** Caller cancellation invalidates a correlated late reply without closing the transport. */
        val active=AtomicBoolean(true)
    }
    private class GridReadContext(val live:Live,val terminal:TerminalReadContext?,val shell:ShellReadContext?,val lease:TerminalGridLease,val display:(TerminalGrid?,TerminalGridLease)->Unit) {
        val reducer=TerminalGridReducer()
        val server=terminal?.reducer?.state?.summary?.serverId ?: shell?.shell?.serverId
        val incarnation=server?.let {id->live.hostReducer.inventory?.hierarchy?.singleOrNull {it.serverId==id}?.serverIncarnation}
        var subscription:Uuid?=null
        val active get()=lease.active()
        var deadline=0L
        var timeout:Job?=null
        var paused=false
    }
    private class WorktreesReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid) {
        /** A dialog ticket owns this transient read; canceled or replaced contexts cannot admit a late result. */
        val active=AtomicBoolean(true)
    }
    private class SkillsReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val generation:Uuid) {val active=AtomicBoolean(true);var catalogueRefresh=false}
    private class SkillsInvocationPublication(val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val catalogue:SkillsListResult)
    private class ControlsReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val lease:NativeControlsLease,val admitted:()->Boolean,val queue:Boolean=false) {
        val active=AtomicBoolean(true);val hello=live.hello;val revision=if(queue)reducer.queueRevision else reducer.controlsRevision
    }
    private class ControlsPublication(val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val catalogue:SessionControlsReadResult,val expires:Long,val revision:Long) {val hello=live.hello}
    private class QueuePublication(val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid,val expires:Long,val revision:Long,
        val entries:LinkedHashMap<Id,Boolean>,val next:Id?,val bytes:Long) {val complete get()=next==null;val hello=live.hello}
    private class ControlsClaim(val publication:ControlsPublication,val scope:ControlsReadContext)
    private class QueueClaim(val publication:QueuePublication,val scope:ControlsReadContext)
    private class AttachmentContext(val host:String,val live:Live,val reducer:SessionReducer?,val target:Target,val source:Uuid?,val subscription:Uuid?)
    private class ArtifactOriginal(val host:String,val live:Live,val hostReducer:HostReducer,val subscription:Uuid,val source:ArtifactSource,val reducer:SessionReducer?,val epoch:Uuid?,val shell:ShellSummary?,val server:Uuid,val incarnation:Uuid)
    private class AgentSessionContext(val host:String,val live:Live,val boot:Uuid,val reducer:SessionReducer,val target:Target,val source:Uuid,val subscription:Uuid?,val server:Uuid,val provider:Provider,val workspace:Id,val pane:Id,val terminal:Id)
    private class CreationDirectoryOriginal(
        val host:String,val live:Live,val boot:Uuid,val incarnation:Uuid,
        val source:ArtifactSource?,val agent:AgentSessionMutationContext?,
        val terminal:TerminalCreationMutationContext?=null,
    )
    private class TerminalCreationOriginal(val host:String,val live:Live,val boot:Uuid,val server:Uuid,val workspace:Id,val incarnation:Uuid,
        val source:ArtifactSource?,val session:SessionSummary?,val shell:ShellSummary?)
    private class AgentAliasContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid)
    private class ChangesReadContext(val host:String,val live:Live,val reducer:SessionReducer,val target:Target,val source:Uuid) {
        /** The sheet ticket owns this memory-only read; replacement UI cannot receive a late reply. */
        val active=AtomicBoolean(true)
    }
    private class ConversationRoute(val target:Target,val owner:ConversationSubscription)
    private class Live(val host:HostMetadata,val attempt:ConnectionAttempt,val candidateToken:Any,val connection:Connection,val stream:BiStream,val send:SendStream,val recv:RecvStream) {
        var grid:GridReadContext?=null
        var skillsPublication:SkillsInvocationPublication?=null
        var controlsPublication:ControlsPublication?=null
        var queuePublication:QueuePublication?=null
        var payloadBytes=0
        val attachmentStatuses=ConcurrentHashMap<Uuid,AttachmentsStatusResult>()
        val alive=AtomicBoolean(true);val disposed=AtomicBoolean(false);val writer=Mutex();val pending=ConcurrentHashMap<Uuid,Pending>();val inflight=Semaphore(64)
        val teardownDone=CompletableDeferred<Unit>()
        val hostReducer=HostReducer();val reducers=ConcurrentHashMap<Uuid,SessionReducer>();val subscriptions=ConcurrentHashMap<Uuid,ConversationRoute>()
        val conversation=AtomicReference<ConversationSubscription?>(null)
        val operations=Mutex();val resnapshot=linkedSetOf<Uuid>();var recoveryRequested=false;var recovery:Job?=null;var receiptsRecovered=false
        val watches=mutableListOf<WatchHandle>();var reader:Job?=null;var authenticated=false;var hello:HelloResult?=null;var selectedPath:String?=null;var selectedRoute:SelectedIrohPath?=null
        val helloReceivePhase=AtomicReference(HelloReceivePhase.NotStarted)
        val helloReadObservation=AtomicReference<OwnedReadObservation?>(null);val helloHeaderPending=AtomicBoolean(false)
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
    private inner class NewActionContinuation(val host:String,val live:Live,val request:Request,val caller:Job,val shell:ShellMutationContext?=null,val alias:AgentAliasMutationContext?=null,val attachment:AttachmentMutationContext?=null,val terminal:TerminalCreationMutationContext?=null,val agentCreation:AgentSessionMutationContext?=null,val skill:SkillInvocationContext?=null,val directories:CreationDirectoriesContext?=null,val controls:ControlsMutationContext?=null,val queue:QueueMutationContext?=null) {
        private val body=request.body
        private val creationSource=(body as? RequestBody.SessionCreate)?.params?.placement?.sourceTarget?.let {live.reducers[it.sessionId]?.source}
        private val promptSource=(body as? RequestBody.PromptSubmit)?.takeIf {it.params.images!=null||it.params.delivery?.let {delivery->delivery!=PromptDelivery.Native}==true}?.params?.target?.let {live.reducers[it.sessionId]?.source}
        private fun original(current:Live,requested:RequestBody,id:Uuid,onFailure:((ActionAdmissionGuard,ErrorCode)->Unit)?=null) {
            caller.ensureActive()
            when {
                current!==live || sessions[host]!==live -> {onFailure?.invoke(ActionAdmissionGuard.OriginalReplaced,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}
                !live.alive.get() -> {onFailure?.invoke(ActionAdmissionGuard.OriginalNotAlive,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}
                !live.authenticated -> {onFailure?.invoke(ActionAdmissionGuard.OriginalUnauthenticated,ErrorCode.NotReady);throw StorePolicy(ErrorCode.NotReady)}
                id!=request.id || requested!==body -> throw StorePolicy(ErrorCode.InvalidRequest)
            }
            if(agentCreation!=null)validateAgentSession(agentCreation,current,requested as? RequestBody.SessionCreate ?: throw StorePolicy(ErrorCode.InvalidRequest))
            if(directories!=null)validateCreationDirectoryMutation(directories,current,requested)
            if(requested is RequestBody.TerminalCreate)validateTerminalCreation(terminal ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested.attachmentTarget()!=null)validateAttachment(attachment ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested is RequestBody.AgentRename) validateAgentAlias(alias ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            if(requested is RequestBody.ShellInput || requested is RequestBody.ShellResize) {
                try {validateShellMutation(shell ?: throw StorePolicy(ErrorCode.NotReady),current,requested)}
                catch(failure:StorePolicy){onFailure?.invoke(ActionAdmissionGuard.SourceNotCurrent,failure.code);throw failure}
            }
        }
        private fun validate(current:Live,requested:RequestBody,id:Uuid,onFailure:((ActionAdmissionGuard,ErrorCode)->Unit)?=null) {
            original(current,requested,id,onFailure)
            if(requested is RequestBody.PromptSubmit && requested.params.skill!=null)validateSkillInvocation(skill ?: throw StorePolicy(ErrorCode.NotReady),current,requested.params)
            else if(skill!=null)throw StorePolicy(ErrorCode.InvalidRequest)
            if(requested is RequestBody.SessionControlsApply)validateControlsMutation(controls ?: throw StorePolicy(ErrorCode.NotReady),current,requested.params)
            else if(controls!=null)throw StorePolicy(ErrorCode.InvalidRequest)
            if(requested.queueTarget()!=null)validateQueueMutation(queue ?: throw StorePolicy(ErrorCode.NotReady),current,requested)
            else if(queue!=null)throw StorePolicy(ErrorCode.InvalidRequest)
            if(requested is RequestBody.PromptSubmit&&(requested.params.images!=null||requested.params.delivery?.let {it!=PromptDelivery.Native}==true))validatePromptOptions(current,requested.params,promptSource)
            val target=(requested as? RequestBody.SessionCreate)?.params?.placement?.sourceTarget
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
                if(canonical?.authoritative==true && canonical.receipt!=null) throw CanonicalAdmissionReceipt(canonical.receipt)
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
                return
            }
        }
        private fun originalBeforePreSend() = original(live,body,request.id) {guard,code->
            captureActionAdmission(host,request.id,ActionAdmissionPhase.PrePreSend,guard,code)
        }
        suspend fun beforePreSend() {
            while(true) {
                originalBeforePreSend()
                if(body is RequestBody.PromptSubmit&&(body.params.skill!=null||body.params.images!=null||body.params.delivery!=null)||body is RequestBody.SessionControlsApply||body.queueTarget()!=null) {
                    val existing=store.localAction(host,request.id)
                    originalBeforePreSend()
                    // preSend still verifies the keyed body fingerprint before status-only recovery.
                    if(existing!=null)return
                }
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
            if(directories!=null){directories.lease.admit {
                if(agentCreation!=null)agentCreation.lease.admit {validate(current,requested,id);admit()}
                else if(terminal!=null)terminal.lease.admit {validate(current,requested,id);admit()}
                else {validate(current,requested,id);admit()}
            }}else if(agentCreation!=null){agentCreation.lease.admit {validate(current,requested,id);admit()}}else if(requested is RequestBody.TerminalCreate){val original=terminal ?: throw StorePolicy(ErrorCode.NotReady);original.lease.admit {validate(current,requested,id);admit()}}else if(requested is RequestBody.ShellInput || requested is RequestBody.ShellResize) {
                val original=shell ?: throw StorePolicy(ErrorCode.NotReady)
                // No suspension or write under this gate. Revocation wins before
                // admission; once admitted, the existing frame owner finishes.
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested.attachmentTarget()!=null){
                val original=attachment ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested is RequestBody.AgentRename) {
                val original=alias ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested is RequestBody.PromptSubmit && requested.params.skill!=null){
                val original=skill ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested is RequestBody.SessionControlsApply) {
                val original=controls ?: throw StorePolicy(ErrorCode.NotReady)
                original.lease.admit {validate(current,requested,id);admit()}
            }else if(requested.queueTarget()!=null) {
                val original=queue ?: throw StorePolicy(ErrorCode.NotReady)
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
    private suspend fun request(live:Live,body:RequestBody,id:Uuid=Uuid(UUID.randomUUID().toString()),admissionAction:Uuid?=null,continuation:NewActionContinuation?=null,inheritedDeadline:Boolean=false,terminal:TerminalReadContext?=null,shell:ShellReadContext?=null,worktrees:WorktreesReadContext?=null,changes:ChangesReadContext?=null,skills:SkillsReadContext?=null,controls:ControlsReadContext?=null,artifact:ArtifactReadContext?=null,apply:suspend (ResultBody)->Unit = {}):ResultBody = withContext(serial) {
        terminal?.let(::validateTerminalRead)
        shell?.let(::validateShellRead)
        worktrees?.let(::validateWorktreesRead)
        changes?.let(::validateChangesRead)
        skills?.let(::validateSkillsRead)
        controls?.let(::validateControlsRead)
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
                                    controls?.let(::validateControlsRead)
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
                live.payloadBytes=bytes.size
                try {
                    live.helloReceivePhase.set(HelloReceivePhase.Decode)
                    val raw=Protocol.parseRawJson(bytes) as? JsonObject ?: invalidWire()
                    val responseId=(raw["id"] as? JsonPrimitive)?.takeIf {it.isString}?.content?.let(::Uuid)
                    val pending=responseId?.let(live.pending::get)
                    // Canceled/timed-out callers keep their bounded method correlation until
                    // a strictly validated reply or close; untracked replies are still fatal.
                    if(raw["type"]?.jsonPrimitive?.content=="response" && pending==null) throw StorePolicy(ErrorCode.InvalidRequest)
                    when(val frame=Protocol.decodeParsed(raw,pending?.method)) {
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
        live.grid?.takeIf {it.subscription==event.subscription}?.let {grid->
            try {
                val current=validateGrid(grid)
                if(grid.reducer.event(event,live.payloadBytes)&&current){
                    grid.paused=false
                    grid.lease.stage(grid.reducer.committed ?: invalidWire())
                    grid.lease.deliver(grid.display)
                }
                gridTimeout(grid)
            } catch(_:Exception){endGrid(grid)}
            return
        }
        val host=live.host.hostId
        if(event.subscription==live.hostReducer.inventory?.subscription) {
            if(event.body is EventBody.SyncRequired)notificationWakeupMutable.tryEmit(NotificationWakeup(host,true))
            val decision=live.hostReducer.event(event)
            invalidateGrid(live)
            if(decision==SessionReducer.Decision.Resnapshot) {recover(live);return}
            if(decision==SessionReducer.Decision.Duplicate) return
            when(val body=event.body) {
                is EventBody.ActionStatus->{if(store.localAction(host,body.data.receipt.actionId)!=null) committedReceipt(live,body.data.receipt)}
                is EventBody.NotificationUpsert->{val expected=live.hello ?: throw StorePolicy(ErrorCode.NotReady);store.liveNotification(host,body.data.event){notificationOwnerCurrent(live,expected)};if(notificationOwnerCurrent(live,expected))notificationWakeupMutable.tryEmit(NotificationWakeup(host))}
                is EventBody.NotificationRead->{val expected=live.hello ?: throw StorePolicy(ErrorCode.NotReady);store.readConfirmed(host,body.data.read){notificationOwnerCurrent(live,expected)};if(notificationOwnerCurrent(live,expected))notificationWakeupMutable.tryEmit(NotificationWakeup(host))}
                else->invalidWire()
            }
        } else {
            // Ordered unsubscribe ACKs drain the old source; untracked streams cannot alter the current view.
            val route=live.subscriptions[event.subscription] ?: return
            val reducer=live.reducers[route.target.sessionId] ?: throw StorePolicy(ErrorCode.InvalidRequest)
            val decision=reducer.event(event)
            invalidateGrid(live)
            if(decision==SessionReducer.Decision.Resnapshot) {live.resnapshot.add(route.target.sessionId);recover(live);return}
            if(decision==SessionReducer.Decision.Applied) when(val body=event.body) {
                is EventBody.ControlsChanged->{live.controlsPublication?.takeIf {it.target==body.data.target}?.let {live.controlsPublication=null}}
                is EventBody.QueueChanged->{live.queuePublication?.takeIf {it.target==body.data.target}?.let {live.queuePublication=null}}
                is EventBody.HistoryUpsert -> {
                    // Persist the retained post-merge records, never raw event placeholders
                    // or unrelated history. Older browsing may reject a new event record.
                    val changed=body.data.records.map {it.id}.toSet()
                    cache(live,reducer,records=reducer.history.filter {it.id in changed})
                }
                is EventBody.SessionState -> cache(live,reducer,records=emptyList())
                else -> Unit
            }
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
                            val owner=live.conversation.get()
                            if(owner==null || owner.original!==live || owner.host!=live.host.hostId || owner.target.sessionId!=session || !owner.active.get()){live.resnapshot.remove(session);continue}
                            val summary=live.hostReducer.inventory?.sessions?.singleOrNull {it.sessionId==session} ?: continue
                            if(Target(summary.sessionId,summary.generation)!=owner.target){owner.active.set(false);live.resnapshot.remove(session);continue}
                            if(summary.connectivity!=SessionConnectivity.Ready)continue
                            try {open(live,owner.target,owner)}
                            catch(failure:RemoteFailure) {if(failure.code!=ErrorCode.NotReady && failure.code!=ErrorCode.StaleSession) throw failure}
                            catch(failure:StorePolicy) {if(failure.code!=ErrorCode.NotReady || owner.active.get())throw failure}
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
    private suspend fun cache(live:Live,reducer:SessionReducer,snapshot:Boolean=false,records:List<HistoryRecord> = reducer.history) {
        val summary=reducer.state?.summary ?: return
        val source=reducer.source ?: return
        store.cache(live.host.hostId,summary,records.map(SafeDisplayHistory::redacted),reducer.partial,snapshot=snapshot,source=source)
    }
    private fun publish() {
        for(live in sessions.values)invalidateGrid(live)
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
            sources=sessions.mapValues {(_,live)->if(!live.authenticated||!live.alive.get()||!live.hostReducer.ready) emptyMap() else live.reducers.values.mapNotNull {reducer->val s=reducer.state?.summary;val source=reducer.source;val route=source?.let {live.subscriptions[it]};if(s==null||source==null||route==null||!route.owner.active.get()||route.target!=Target(s.sessionId,s.generation)||s.connectivity!=SessionConnectivity.Ready||!live.hostReducer.sourceCurrent(s.serverId)) null else route.target to source}.toMap()},
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
    private suspend fun snapshot(live:Live):HostSnapshotResult = (request(live,RequestBody.HostSnapshot(HostSnapshotParams())) {result->val snapshot=(result as ResultBody.HostSnapshot).result;live.hostReducer.snapshot(snapshot);invalidateGrid(live);store.cacheInventory(live.host.hostId,snapshot);publish()} as ResultBody.HostSnapshot).result
    suspend fun open(host:String,target:Target,admitted:()->Boolean={true}):SessionOpenResult = withContext(serial) {
        val live=live(host);if(!admitted())throw StorePolicy(ErrorCode.NotReady)
        val owner=ConversationSubscription(live,host,target)
        try {live.operations.withLock {
            if(!admitted())throw StorePolicy(ErrorCode.NotReady)
            val previous=live.conversation.getAndSet(owner)
            if(previous!=null) {
                previous.active.set(false)
                // Register exact-owner cleanup independently before this open can suspend/cancel.
                scope.launch(start=CoroutineStart.UNDISPATCHED) {
                    try {leave(previous)}
                    catch(cancel:CancellationException) {throw cancel}
                    catch(failure:Exception) {
                        if(failure is computer.iroh.IrohException)failure.close()
                        teardown(live);publish()
                    }
                }
            }
            val result=open(live,target,owner,admitted)
            if(!admitted()){if(owner.active.getAndSet(false))abandonSubscription(live,result.subscription);throw StorePolicy(ErrorCode.NotReady)}
            recoverReceipts(live)
            if(!admitted()){if(owner.active.getAndSet(false))abandonSubscription(live,result.subscription);throw StorePolicy(ErrorCode.NotReady)}
            result
        }}
        catch(cancel:CancellationException) {owner.active.set(false);throw cancel}
        catch(failure:RemoteFailure) {if((failure.code==ErrorCode.NotReady || failure.code==ErrorCode.StaleSession) && target.sessionId in live.resnapshot) recover(live);throw failure}
    }
    private suspend fun open(live:Live,target:Target,owner:ConversationSubscription,admitted:()->Boolean={true}):SessionOpenResult {
        if(!owner.active.get()||!admitted())throw StorePolicy(ErrorCode.NotReady)
        val caller=currentCoroutineContext()[Job]!!
        live.receiptsRecovered=false;publish()
        // Keep each old route/reducer until its ordered unsubscribe response has applied.
        for((subscription,route) in live.subscriptions) if(route.target.sessionId==target.sessionId) unsubscribe(live,subscription)
        live.reducers[target.sessionId]?.disconnected();publish()
        try {
            return (request(live,RequestBody.SessionOpen(SessionOpenParams(target))) {result->
                val snapshot=(result as ResultBody.SessionOpen).result
                requireWire(Target(snapshot.state.summary.sessionId,snapshot.state.summary.generation)==target)
                if(!caller.isActive||!owner.active.get()||!admitted()){owner.active.set(false);abandonSubscription(live,snapshot.subscription);return@request}
                val reducer=live.reducers.getOrPut(target.sessionId) {SessionReducer()}
                live.subscriptions[snapshot.subscription]=ConversationRoute(target,owner)
                reducer.snapshot(snapshot);live.resnapshot.remove(target.sessionId)
                invalidateGrid(live)
                cache(live,reducer,true)
                if(caller.isActive&&owner.active.get()&&admitted())store.viewing(live.host.hostId,target.sessionId)
                publish()
                if(!caller.isActive||!owner.active.get()||!admitted()){owner.active.set(false);abandonSubscription(live,snapshot.subscription)}
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
            live.subscriptions.remove(subscription)?.let {route->live.reducers[route.target.sessionId]?.takeIf {it.source==subscription}?.disconnected()}
            publish()
        }
    }
    /** Retire synchronously; never inspect mutable reducer state from the main owner. */
    internal fun retireConversationSubscription(host:String,target:Target):ConversationSubscription? {
        val live=sessions[host] ?: return null
        val original=live.conversation.get()?.takeIf {it.target==target}
            ?: live.subscriptions.values.firstOrNull {it.owner.target==target}?.owner ?: return null
        if(original.active.getAndSet(false))scope.launch {publish()}
        return original
    }
    /** Delayed cleanup can remove only routes of the captured original viewer, not a new foreground. */
    suspend fun leave(original:ConversationSubscription) = withContext(serial) {
        val live=original.original as? Live ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(sessions[original.host]!==live || !live.alive.get())return@withContext
        live.operations.withLock {
            for((subscription,route) in live.subscriptions.toMap()) {
                if(route.owner===original && live.alive.get() && live.subscriptions[subscription]===route)unsubscribe(live,subscription)
            }
            if(live.conversation.get()===original)live.resnapshot.remove(original.target.sessionId)
            invalidateGrid(live)
            if(live.subscriptions.isEmpty())store.viewing(original.host,null)
        }
        publish()
    }
    private fun contentSource(host:String,target:Target,source:Uuid):Pair<Live,SessionReducer> {
        val live=live(host);val reducer=live.reducers[target.sessionId] ?: throw StorePolicy(ErrorCode.NotReady)
        val summary=reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!live.hostReducer.ready || !live.hostReducer.sourceCurrent(summary.serverId) || summary.connectivity!=SessionConnectivity.Ready || reducer.source!=source || Target(summary.sessionId,summary.generation)!=target) throw StorePolicy(ErrorCode.ContentExpired)
        if(live.subscriptions[source]?.owner?.active?.get()!=true)throw StorePolicy(ErrorCode.ContentExpired)
        return live to reducer
    }
    /** Every chunk and completed admission rechecks this exact subscription, never a UI epoch. */
    suspend fun content(host:String,source:Uuid,params:ContentReadParams):ContentReadResult = withContext(serial) {
        val (live,_)=contentSource(host,params.target,source)
        val result=(request(live,RequestBody.ContentRead(params)) as ResultBody.ContentRead).result
        contentSource(host,params.target,source);result
    }
    /** The selected terminal retains its original live/reducer/source triple. */
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
    private fun validateControlsRead(context:ControlsReadContext) {
        val live=context.live;val reducer=context.reducer
        if(!context.active.get()||!context.lease.active.get()||!context.admitted()||sessions[context.host]!==live||!live.alive.get()||live.disposed.get()||!live.authenticated||live.hello!==context.hello||context.hello==null||live.reducers[context.target.sessionId]!==reducer)throw StorePolicy(ErrorCode.StaleSession)
        val summary=reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(!live.hostReducer.ready||!live.hostReducer.sourceCurrent(summary.serverId)||summary.connectivity!=SessionConnectivity.Ready||!summary.capabilities.controls
            ||reducer.source!=context.source||summary.sessionId!=context.target.sessionId||summary.generation!=context.target.generation
            ||live.subscriptions[context.source]?.owner?.active?.get()!=true
            ||(if(context.queue)reducer.queueRevision else reducer.controlsRevision)!=context.revision||context.queue&&summary.provider!=Provider.Codex)throw StorePolicy(ErrorCode.StaleSession)
    }
    internal suspend fun nativeControls(host:String,target:Target,source:Uuid,lease:NativeControlsLease,admitted:()->Boolean):SessionControlsReadResult=withContext(serial) {
        val(live,reducer)=contentSource(host,target,source);live.controlsPublication=null
        val context=ControlsReadContext(host,live,reducer,target,source,lease,admitted);validateControlsRead(context)
        val expires=android.os.SystemClock.elapsedRealtime()+300000
        try {
            val result=(request(live,RequestBody.SessionControlsRead(SessionControlsReadParams(target)),controls=context) {response->
                validateControlsRead(context);if((response as ResultBody.SessionControlsRead).result.target!=target)throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.SessionControlsRead).result
            validateControlsRead(context)
            if(result.target!=target||android.os.SystemClock.elapsedRealtime()>=expires)throw StorePolicy(ErrorCode.StaleSession)
            live.controlsPublication=ControlsPublication(live,reducer,target,source,result,expires,context.revision);result
        }finally{context.active.set(false)}
    }
    internal suspend fun controlsMutation(host:String,catalogue:SessionControlsReadResult,lease:NativeControlsLease,admitted:()->Boolean):ControlsMutationContext=withContext(serial) {
        val publication=sessions[host]?.controlsPublication?.takeIf {it.catalogue===catalogue}
        val claim=publication?.let {ControlsClaim(it,ControlsReadContext(host,it.live,it.reducer,it.target,it.source,lease,admitted))}
        ControlsMutationContext(claim,catalogue,lease,admitted)
    }
    private fun validateControlsMutation(context:ControlsMutationContext,live:Live,params:SessionControlsApplyParams) {
        val claim=context.publication as? ControlsClaim ?: throw StorePolicy(ErrorCode.StaleSession)
        val publication=claim.publication;validateControlsRead(claim.scope)
        if(publication.live!==live||live.controlsPublication!==publication||publication.catalogue!==context.catalogue||live.hello!==publication.hello
            ||publication.revision!=publication.reducer.controlsRevision||android.os.SystemClock.elapsedRealtime()>=publication.expires
            ||params.target!=publication.target||params.catalogueGeneration!=publication.catalogue.catalogueGeneration)throw StorePolicy(ErrorCode.StaleSession)
        val catalogue=publication.catalogue
        val selected=(params.modelRef ?: catalogue.current.modelRef)?.let {reference->catalogue.models.singleOrNull {it.reference==reference} ?: throw StorePolicy(ErrorCode.StaleSession)}
        params.effort?.let {if(selected==null||it !in selected.efforts)throw StorePolicy(ErrorCode.InvalidRequest)}
        params.mode?.let {if(it !in catalogue.modes||params.modelRef==null&&catalogue.current.modelRef==null)throw StorePolicy(ErrorCode.NotReady)}
    }
    internal suspend fun nativeQueue(host:String,target:Target,source:Uuid,cursor:Id?,lease:NativeControlsLease,admitted:()->Boolean):SessionQueueListResult=withContext(serial) {
        val(live,reducer)=contentSource(host,target,source)
        val previous=if(cursor!=null)live.queuePublication?.takeIf {it.live===live&&it.reducer===reducer&&it.hello===live.hello&&it.revision==reducer.queueRevision&&it.target==target&&it.source==source&&it.next==cursor} ?: throw StorePolicy(ErrorCode.StaleSession)
            else null.also {live.queuePublication=null}
        val context=ControlsReadContext(host,live,reducer,target,source,lease,admitted,true);validateControlsRead(context)
        val expires=previous?.expires ?: (android.os.SystemClock.elapsedRealtime()+300000)
        if(android.os.SystemClock.elapsedRealtime()>=expires)throw StorePolicy(ErrorCode.StaleSession)
        try {
            val result=(request(live,RequestBody.SessionQueueList(SessionQueueListParams(target,cursor)),controls=context) {response->
                validateControlsRead(context);if((response as ResultBody.SessionQueueList).result.target!=target)throw StorePolicy(ErrorCode.StaleSession)
            } as ResultBody.SessionQueueList).result
            validateControlsRead(context)
            if(result.target!=target||live.queuePublication!==previous||android.os.SystemClock.elapsedRealtime()>=expires)throw StorePolicy(ErrorCode.StaleSession)
            val entries=LinkedHashMap<Id,Boolean>(previous?.entries ?: emptyMap())
            if(entries.size+result.entries.size>128)throw StorePolicy(ErrorCode.ResourceLimit)
            var bytes=previous?.bytes ?: 0
            for(entry in result.entries) {
                if(entries.putIfAbsent(entry.id,entry.editable)!=null)throw StorePolicy(ErrorCode.StaleSession)
                bytes+=utf8Size(entry.id.value)+(when(val text=entry.text) {is ContentText->utf8Size(text.text.value).toLong();is ContentRef->text.byteLength.value;is ContentUnavailable->0})
                if(bytes>256L*1024)throw StorePolicy(ErrorCode.ResourceLimit)
            }
            live.queuePublication=QueuePublication(live,reducer,target,source,expires,context.revision,entries,result.nextCursor,bytes);result
        }finally{context.active.set(false)}
    }
    internal suspend fun queueMutation(host:String,target:Target,source:Uuid,lease:NativeControlsLease,admitted:()->Boolean):QueueMutationContext=withContext(serial) {
        val publication=sessions[host]?.queuePublication?.takeIf {it.target==target&&it.source==source}
        val claim=publication?.let {QueueClaim(it,ControlsReadContext(host,it.live,it.reducer,it.target,it.source,lease,admitted,true))}
        QueueMutationContext(claim,lease,admitted)
    }
    private fun validateQueueMutation(context:QueueMutationContext,live:Live,body:RequestBody) {
        val claim=context.publication as? QueueClaim ?: throw StorePolicy(ErrorCode.StaleSession)
        val publication=claim.publication;validateControlsRead(claim.scope)
        if(publication.live!==live||live.queuePublication!==publication||live.hello!==publication.hello
            ||publication.revision!=publication.reducer.queueRevision||android.os.SystemClock.elapsedRealtime()>=publication.expires)throw StorePolicy(ErrorCode.StaleSession)
        val target:Target;val entry:Id?
        when(body) {
            is RequestBody.SessionQueueUpdate->{target=body.params.target;entry=body.params.entryId;if(publication.entries[entry]!=true)throw StorePolicy(ErrorCode.InvalidRequest)}
            is RequestBody.SessionQueueDelete->{target=body.params.target;entry=body.params.entryId}
            is RequestBody.SessionQueueStart->{
                target=body.params.target;entry=body.params.entryId
                val state=publication.reducer.state ?: throw StorePolicy(ErrorCode.NotReady)
                if(state.nativeTurnId!=null||state.summary.status!=SessionStatus.Idle)throw StorePolicy(ErrorCode.Busy)
            }
            is RequestBody.SessionQueueReorder->{
                target=body.params.target;entry=null
                if(!publication.complete||body.params.entryIds.size!=publication.entries.size||body.params.entryIds.any {it !in publication.entries})throw StorePolicy(ErrorCode.StaleSession)
            }
            else->throw StorePolicy(ErrorCode.InvalidRequest)
        }
        if(target!=publication.target||entry!=null&&entry !in publication.entries)throw StorePolicy(ErrorCode.StaleSession)
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
    internal suspend fun skillInvocation(host:String,catalogue:SkillsListResult,entry:SkillEntry,lease:SkillInvocationLease,admitted:()->Boolean):SkillInvocationContext = withContext(serial) {
        val publication=sessions[host]?.skillsPublication?.takeIf {it.catalogue===catalogue&&catalogue.skills.any {row->row===entry}}
        SkillInvocationContext(publication,catalogue,entry.invocationRef,lease,admitted)
    }
    private fun validateSkillInvocation(context:SkillInvocationContext,live:Live,params:PromptSubmitParams) {
        if(!context.lease.active.get())throw StorePolicy(ErrorCode.StaleSession)
        val publication=context.publication as? SkillsInvocationPublication ?: throw StorePolicy(ErrorCode.NotReady)
        val selector=params.skill ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(publication.live!==live || live.skillsPublication!==publication || publication.catalogue!==context.catalogue || selector.reference!=context.reference || selector.catalogueGeneration!=context.catalogue.catalogueGeneration || params.target!=publication.target || params.text.value.isNotEmpty())throw StorePolicy(ErrorCode.StaleSession)
        if(!context.admitted() || !live.alive.get() || live.disposed.get() || !live.authenticated || live.reducers[publication.target.sessionId]!==publication.reducer)throw StorePolicy(ErrorCode.StaleSession)
        val summary=publication.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(sessions[live.host.hostId]!==live || !live.hostReducer.ready || !live.hostReducer.sourceCurrent(summary.serverId) || publication.reducer.source!=publication.source || Target(summary.sessionId,summary.generation)!=publication.target || summary.provider!=Provider.Codex || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.prompt || !summary.capabilities.skillsRead || summary.skillsGeneration!=selector.catalogueGeneration || !context.catalogue.skills.any {it.enabled&&it.invocationRef==selector.reference})throw StorePolicy(ErrorCode.StaleSession)
    }
    suspend fun skills(host:String,target:Target,source:Uuid,generation:Uuid):SkillsListResult = withContext(serial) {
        val (live,reducer)=contentSource(host,target,source)
        live.skillsPublication=null
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
            live.skillsPublication=SkillsInvocationPublication(live,reducer,target,source,result)
            result
        } finally {context.active.set(false)}
    }
    /** One complete secure preview. No URI, file path or text is persisted. */
    internal suspend fun skillsDocument(host:String,target:Target,source:Uuid,generation:Uuid,document:Uuid):String {
        val caller=currentCoroutineContext()[Job] ?: throw StorePolicy(ErrorCode.NotReady)
        return withContext(serial){
            val(live,reducer)=contentSource(host,target,source);val context=SkillsReadContext(host,live,reducer,target,source,generation)
            validateSkillsRead(context)
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
    internal suspend fun terminal(host:String,target:Target,source:Uuid,display:(TerminalGrid?,TerminalGridLease)->Unit)=withContext(serial) {
        val context=terminalContext(host,target,source)
        gridStream(context.live,RequestBody.TerminalGridSubscribe(TerminalGridSubscribeParams(target)),context,null,display)
    }
    private fun invalidateGrid(live:Live) {
        live.grid?.let {grid->try {
            if(!validateGrid(grid)){grid.paused=true;grid.lease.pause();scope.launch{grid.lease.clearPaused(grid.display)}}
            else if(grid.paused){grid.paused=false;grid.reducer.committed?.let {grid.lease.stage(it);scope.launch{grid.lease.deliver(grid.display)}}}
        }catch(_:Exception){endGrid(grid)}}
    }
    private fun validateGrid(grid:GridReadContext):Boolean {
        if(!grid.active || grid.live.grid!==grid || grid.reducer.expired())throw StorePolicy(ErrorCode.StaleSession)
        var current=true
        grid.terminal?.let {
            validateTerminalRead(it)
            if(grid.live.hostReducer.inventory?.hierarchy?.singleOrNull {graph->graph.serverId==grid.server}?.serverIncarnation!=grid.incarnation)
                throw StorePolicy(ErrorCode.StaleSession)
        }
        grid.shell?.let {context->
            current=shellReadCurrent(context)
            if(context.live.hostReducer.inventory?.hierarchy?.singleOrNull {it.serverId==context.shell.serverId}?.serverIncarnation!=grid.incarnation)throw StorePolicy(ErrorCode.StaleSession)
            if(current&&context.live.hostReducer.inventory?.shells?.singleOrNull {it.target==context.shell.target}?.capabilities?.grid!=true)throw StorePolicy(ErrorCode.NotReady)
        }
        return current
    }
    private fun gridTimeout(grid:GridReadContext) {
        val deadline=grid.reducer.transactionDeadline
        if(deadline==grid.deadline)return
        grid.timeout?.cancel();grid.timeout=null;grid.deadline=deadline
        if(deadline!=0L)grid.timeout=scope.launch(serial) {
            delay(((deadline-System.nanoTime()).coerceAtLeast(0)+999_999)/1_000_000)
            if(grid.active && grid.live.grid===grid && grid.reducer.transactionDeadline==deadline && grid.reducer.expired())endGrid(grid)
        }
    }
    private fun endGrid(grid:GridReadContext) {
        if(!grid.lease.close())return
        grid.terminal?.active?.set(false);grid.shell?.active?.set(false)
        grid.timeout?.cancel();grid.timeout=null;grid.deadline=0
        grid.reducer.clear()
        scope.launch {grid.lease.clear(grid.display)}
        if(grid.live.grid===grid)grid.live.grid=null
        grid.subscription?.let {abandonSubscription(grid.live,it)}
    }
    private suspend fun gridStream(live:Live,body:RequestBody,terminal:TerminalReadContext?,shell:ShellReadContext?,display:(TerminalGrid?,TerminalGridLease)->Unit) {
        if(live.grid!=null)throw StorePolicy(ErrorCode.Busy)
        val caller=currentCoroutineContext()[Job]!!
        val grid=GridReadContext(live,terminal,shell,TerminalGridLease(caller),display);live.grid=grid
        try {
            request(live,body,terminal=terminal,shell=shell){response->
                val (subscription,frame)=when(response){
                    is ResultBody.TerminalGridSubscribe->{requireWire(response.result.target==terminal?.target);response.result.subscription to response.result.frame}
                    is ResultBody.ShellGridSubscribe->{requireWire(response.result.target==shell?.shell?.target);response.result.subscription to response.result.frame}
                    else->invalidWire()
                }
                grid.subscription=subscription
                if(!caller.isActive || !grid.active){abandonSubscription(live,subscription);return@request}
                validateGrid(grid);grid.reducer.open(frame,live.payloadBytes);gridTimeout(grid)
            }
            grid.lease.stopped.await()
            throw StorePolicy(ErrorCode.NotReady)
        } finally {endGrid(grid)}
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
    private fun notificationOwnerCurrent(original:Live,expected:HelloResult):Boolean =
        sessions[original.host.hostId]===original&&original.alive.get()&&original.authenticated&&original.hello===expected&&mutable.value.hello[original.host.hostId]===expected
    // A sync completion is caller-driven reconciliation, never a delivery wakeup. Emitting here
    // would make foreground read confirmation recursively request another sync page.
    internal suspend fun reconcilePush(host:String,expected:HelloResult)=withContext(serial){
        val original=live(host)
        fun admitted(){if(!notificationOwnerCurrent(original,expected)||!original.hostReducer.ready)throw StorePolicy(ErrorCode.NotReady)}
        admitted();var after:Uint?=null;var pages=0
        do{
            admitted();if(++pages>100)throw StorePolicy(ErrorCode.ResourceLimit)
            val previous=after
            val result=(request(original,RequestBody.NotificationsSync(NotificationsSyncParams(after,Uint(100)))){reply->admitted();store.notifications(host,(reply as ResultBody.NotificationsSync).result){notificationOwnerCurrent(original,expected)&&original.hostReducer.ready};admitted()}as ResultBody.NotificationsSync).result
            admitted();after=result.nextSequence;if(after!=null&&after.value<=(previous?.value ?: 0L))throw StorePolicy(ErrorCode.InvalidRequest)
        }while(after!=null)
    }
    internal suspend fun registerPush(host:String,expected:HelloResult,registration:LocalPushRegistration):PushRegisterResult=withContext(serial){
        val original=live(host);if(!notificationOwnerCurrent(original,expected)||registration.device!=expected.deviceId)throw StorePolicy(ErrorCode.NotReady)
        val endpoint=registration.endpoint ?: throw StorePolicy(ErrorCode.NotReady)
        val result=(request(original,RequestBody.PushRegister(PushRegisterParams(expected.bootId,endpoint,registration.generation,Key32(registration.key),registration.probe))){reply->
            if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
            val result=(reply as ResultBody.PushRegister).result;if(result.endpointGeneration!=registration.generation||result.probe!=registration.probe)throw StorePolicy(ErrorCode.PushRejected)
        }as ResultBody.PushRegister).result
        if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        result
    }
    internal suspend fun acknowledgePush(host:String,expected:HelloResult,registration:LocalPushRegistration):PushProbeAckResult=withContext(serial){
        val original=live(host);if(!notificationOwnerCurrent(original,expected)||registration.device!=expected.deviceId)throw StorePolicy(ErrorCode.NotReady)
        val result=(request(original,RequestBody.PushProbeAck(PushProbeAckParams(expected.bootId,registration.generation,registration.probe))){
            if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        }as ResultBody.PushProbeAck).result
        if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        result
    }
    internal suspend fun unregisterPush(host:String,expected:HelloResult,generation:Uuid):PushUnregisterResult=withContext(serial){
        val original=live(host);if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        val result=(request(original,RequestBody.PushUnregister(PushUnregisterParams(expected.bootId,generation))){if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)}as ResultBody.PushUnregister).result
        if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        result
    }
    suspend fun notificationsSync(host:String,expected:HelloResult,after:Uint?=null,limit:Uint?=null):NotificationsSyncResult=withContext(serial){
        val original=live(host);if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        val result=(request(original,RequestBody.NotificationsSync(NotificationsSyncParams(after,limit))){reply->
            if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
            store.notifications(host,(reply as ResultBody.NotificationsSync).result){notificationOwnerCurrent(original,expected)}
        }as ResultBody.NotificationsSync).result
        if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        result
    }
    suspend fun markRead(host:String,expected:HelloResult,session:Uuid,through:Uint):ReadWatermark=withContext(serial){
        val original=live(host);if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        val params=store.readRequest(host,expected.bootId,session,through)
        if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        val result=(request(original,RequestBody.NotificationsRead(params)){reply->
            if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
            store.readConfirmed(host,(reply as ResultBody.NotificationsRead).result.read){notificationOwnerCurrent(original,expected)}
            if(notificationOwnerCurrent(original,expected))notificationWakeupMutable.tryEmit(NotificationWakeup(host))
        }as ResultBody.NotificationsRead).result.read
        if(!notificationOwnerCurrent(original,expected))throw StorePolicy(ErrorCode.NotReady)
        result
    }
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
        val (boot,target)=when(body) {
            is RequestBody.SessionControlsApply->body.params.bootId to body.params.target
            is RequestBody.SessionQueueUpdate->body.params.bootId to body.params.target
            is RequestBody.SessionQueueDelete->body.params.bootId to body.params.target
            is RequestBody.SessionQueueReorder->body.params.bootId to body.params.target
            is RequestBody.SessionQueueStart->body.params.bootId to body.params.target
            is RequestBody.TerminalCreate->body.params.bootId to null
            is RequestBody.AttachmentsBegin->body.params.bootId to body.params.target
            is RequestBody.AttachmentsChunk->body.params.bootId to body.params.target
            is RequestBody.AttachmentsComplete->body.params.bootId to body.params.target
            is RequestBody.AttachmentsDiscard->body.params.bootId to null
            is RequestBody.AgentRename->body.params.bootId to body.params.target
            is RequestBody.WorkspaceRename->body.params.bootId to body.params.target
            is RequestBody.PromptSubmit->body.params.bootId to body.params.target
            is RequestBody.TurnCancel->body.params.bootId to body.params.target
            is RequestBody.RequestRespond->body.params.bootId to body.params.target
            is RequestBody.SessionCreate->body.params.bootId to body.params.placement?.sourceTarget
            is RequestBody.ShellInput->body.params.bootId to null
            is RequestBody.ShellResize->body.params.bootId to null
            else->return
        }
        if(boot!=live.hello?.bootId) reject(ActionAdmissionGuard.BootChanged,ErrorCode.StaleHost)
        if(body !is RequestBody.AttachmentsDiscard && !live.hostReducer.ready) reject(ActionAdmissionGuard.HostNotReady,ErrorCode.NotReady)
        if(!live.receiptsRecovered) reject(ActionAdmissionGuard.ReceiptsUnreconciled,ErrorCode.NotReady)
        if(body is RequestBody.ShellInput || body is RequestBody.ShellResize) {
            val shellTarget=when(body){is RequestBody.ShellInput->body.params.target;is RequestBody.ShellResize->body.params.target;else->throw StorePolicy(ErrorCode.InvalidRequest)}
            val shell=live.hostReducer.inventory?.shells?.singleOrNull {it.target==shellTarget} ?: reject(ActionAdmissionGuard.TargetChanged,ErrorCode.StaleSession)
            val allowed=when(body){is RequestBody.ShellInput->shell.capabilities.input;is RequestBody.ShellResize->shell.capabilities.grid;else->shell.capabilities.create}
            if(shell.connectivity!=SessionConnectivity.Ready || !shell.capabilities.read || !allowed || !live.hostReducer.sourceCurrent(shell.serverId)) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        }
        if(target!=null) {val reducer=live.reducers[target.sessionId] ?: reject(ActionAdmissionGuard.ReducerAbsent,ErrorCode.NotReady);val summary=reducer.state?.summary ?: reject(ActionAdmissionGuard.SummaryAbsent,ErrorCode.NotReady);if(Target(summary.sessionId,summary.generation)!=target) reject(ActionAdmissionGuard.TargetChanged,ErrorCode.StaleSession);if(!reducer.mutationsEnabled) reject(ActionAdmissionGuard.MutationsDisabled,ErrorCode.NotReady);val route=reducer.source?.let {live.subscriptions[it]};if(route==null || route.target!=target || !route.owner.active.get())reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady);if(!live.hostReducer.sourceCurrent(summary.serverId)) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)}
        if(body.attachmentTarget()!=null && body !is RequestBody.AttachmentsDiscard && live.reducers[body.attachmentTarget()!!.sessionId]?.state?.summary?.capabilities?.attachments!=true)reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.AgentRename && live.reducers[body.params.target.sessionId]?.state?.summary?.capabilities?.agentRename!=true) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.SessionControlsApply&&live.reducers[body.params.target.sessionId]?.state?.summary?.capabilities?.controls!=true)reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        if(body is RequestBody.SessionCreate && body.params.placement!=null) {
            val sourceTarget=body.params.placement.sourceTarget
            val reducer=live.reducers[sourceTarget.sessionId] ?: reject(ActionAdmissionGuard.ReducerAbsent,ErrorCode.NotReady)
            val summary=reducer.state?.summary ?: reject(ActionAdmissionGuard.SummaryAbsent,ErrorCode.NotReady)
            if(summary.serverId!=body.params.serverId)reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.StaleSession)
            if(live.hostReducer.inventory?.projects?.none {it.projectId==body.params.projectId && it.available}!=false) reject(ActionAdmissionGuard.SourceNotCurrent,ErrorCode.NotReady)
        }

    }
    private fun validatePromptOptions(live:Live,params:PromptSubmitParams,source:Uuid?) {
        val reducer=live.reducers[params.target.sessionId] ?: throw StorePolicy(ErrorCode.NotReady)
        val state=reducer.state ?: throw StorePolicy(ErrorCode.NotReady)
        if(source==null||reducer.source!=source||state.summary.sessionId!=params.target.sessionId||state.summary.generation!=params.target.generation
            ||live.subscriptions[source]?.owner?.active?.get()!=true)throw StorePolicy(ErrorCode.StaleSession)
        val delivery=params.delivery ?: PromptDelivery.Native
        when(state.summary.provider) {
            Provider.Omp->if(delivery==PromptDelivery.Queue||params.nativeTurnId!=null||delivery!=PromptDelivery.Native&&!state.summary.capabilities.controls)throw StorePolicy(ErrorCode.NotReady)
            Provider.Codex->if(delivery==PromptDelivery.FollowUp||delivery==PromptDelivery.Aside||delivery==PromptDelivery.Steer&&(params.nativeTurnId==null||params.nativeTurnId!=state.nativeTurnId))throw StorePolicy(ErrorCode.StaleSession)
        }
        params.images?.let {images->
            if(!state.summary.capabilities.attachments)throw StorePolicy(ErrorCode.NotReady)
            var bytes=0L
            for(image in images) {
                val status=live.attachmentStatuses[image] ?: throw StorePolicy(ErrorCode.NotReady)
                if(status.target!=params.target||status.state!=AttachmentUploadState.Complete||!status.insertionReady||status.progress.published!=true||status.progress.discarded==true)throw StorePolicy(ErrorCode.StaleSession)
                bytes+=status.size.value;if(bytes>8L*1024*1024)throw StorePolicy(ErrorCode.ResourceLimit)
            }
        }
    }
    /** Capture actual immutable owner references before any suspension or system-picker launch. */
    internal fun captureAttachment(host:String,target:Target,source:Uuid,expected:SessionReducer,resume:AttachmentsStatusResult?=null):AttachmentMutationContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val original=AttachmentContext(host,live,expected,target,source,live.subscriptions.entries.singleOrNull {it.value.target==target}?.key)
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
            if(live.reducers[original.target.sessionId]!==reducer || original.source==null || reducer.source!=original.source || original.subscription==null || live.subscriptions[original.subscription]?.target!=original.target || Target(summary.sessionId,summary.generation)!=original.target || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.attachments || !reducer.mutationsEnabled || !live.hostReducer.ready || !live.hostReducer.sourceCurrent(summary.serverId))throw StorePolicy(ErrorCode.NotReady)
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
    internal fun captureCreationDirectories(host:String,server:Uuid,project:Uuid,agent:AgentSessionMutationContext?,terminal:TerminalCreationMutationContext?=null,viewCurrent:()->Boolean):CreationDirectoriesContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        if(agent!=null&&terminal!=null)throw StorePolicy(ErrorCode.InvalidRequest)
        agent?.let {validateAgentSessionOrigin(it,live,false)}
        terminal?.let {validateTerminalCreationOrigin(it,live,false)}
        val originalAgent=agent?.original as? AgentSessionContext
        val originalTerminal=terminal?.original as? TerminalCreationOriginal
        if(originalTerminal!=null&&originalTerminal.server!=server)throw StorePolicy(ErrorCode.StaleSession)
        val source=originalTerminal?.source ?: originalAgent?.takeIf {it.server==server}?.let {ArtifactSource.Session(it.target)}
        val hierarchy=live.hostReducer.inventory?.hierarchy?.singleOrNull {it.serverId==server&&it.freshness==NativeFreshness.Current}
            ?: throw StorePolicy(ErrorCode.NotReady)
        val original=CreationDirectoryOriginal(host,live,live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady),hierarchy.serverIncarnation,source,agent,terminal)
        val context=CreationDirectoriesContext(original,AgentSessionLease(),server,project,android.os.SystemClock.elapsedRealtime()+300000,viewCurrent)
        validateCreationDirectories(context,live,true);return context
    }
    internal fun creationDirectoriesCurrent(context:CreationDirectoriesContext):Boolean=try {
        val original=context.original as? CreationDirectoryOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
        validateCreationDirectories(context,original.live,false);true
    }catch(_:Exception){false}
    private fun validateCreationDirectories(context:CreationDirectoriesContext,live:Live,ready:Boolean)=context.lease.admit {
        val original=context.original as? CreationDirectoryOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(!context.viewCurrent()||original.live!==live||sessions[original.host]!==live||!live.alive.get()||!live.authenticated||live.hello?.bootId!=original.boot)throw StorePolicy(ErrorCode.StaleSession)
        original.agent?.let {validateAgentSessionOrigin(it,live,false)}
        original.terminal?.let {validateTerminalCreationOrigin(it,live,false)}
        val inventory=live.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        val hierarchy=inventory.hierarchy.singleOrNull {it.serverId==context.server&&it.serverIncarnation==original.incarnation}
            ?: throw StorePolicy(ErrorCode.StaleSession)
        if(hierarchy.freshness==NativeFreshness.Unavailable||inventory.projects.none {it.projectId==context.project&&it.available})throw StorePolicy(ErrorCode.NotReady)
        if(ready&&(!live.hostReducer.ready||!live.hostReducer.sourceCurrent(context.server)||hierarchy.freshness!=NativeFreshness.Current))throw StorePolicy(ErrorCode.NotReady)
    }
    internal suspend fun creationDirectories(context:CreationDirectoriesContext,parent:Uuid?,cursor:Uuid?):CreationDirectoriesResult=withContext(serial) {
        val original=context.original as? CreationDirectoryOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
        fun validate()=context.lease.admit {
            validateCreationDirectories(context,original.live,true)
            if(android.os.SystemClock.elapsedRealtime()>=context.expiresAt)throw StorePolicy(ErrorCode.CursorExpired)
            if(parent!=null&&parent !in context.references)throw StorePolicy(ErrorCode.CursorExpired)
            if(cursor!=null&&(!context.cursors.containsKey(cursor)||context.cursors[cursor]!=parent))throw StorePolicy(ErrorCode.CursorExpired)
        }
        validate()
        val params=CreationDirectoriesParams(context.server,context.project,original.source,parent,cursor,Uint(100))
        val result=(request(original.live,RequestBody.CreationDirectories(params)){response->
            context.lease.admit {
                validate()
                val page=(response as? ResultBody.CreationDirectories)?.result ?: throw ProtocolFailure(FailureKind.Incompatible)
                if(page.serverId!=context.server||page.projectId!=context.project||page.source!=original.source)throw StorePolicy(ErrorCode.StaleSession)
                val incoming=page.entries.count {it.reference !in context.references}
                val next=page.nextCursor
                if(context.references.size+incoming>4096||next!=null&&next !in context.cursors&&context.cursors.size>=4096)throw StorePolicy(ErrorCode.ResourceLimit)
                if(next!=null&&context.cursors.containsKey(next)&&context.cursors[next]!=parent)throw StorePolicy(ErrorCode.CursorExpired)
                page.entries.forEach {context.references.add(it.reference)}
                if(next!=null)context.cursors[next]=parent
            }
        } as ResultBody.CreationDirectories).result
        validate();result
    }
    private fun validateCreationDirectoryMutation(context:CreationDirectoriesContext,live:Live,body:RequestBody)=context.lease.admit {
        validateCreationDirectories(context,live,true)
        val (server,project,reference)=when(body) {
            is RequestBody.SessionCreate->Triple(body.params.serverId,body.params.projectId,body.params.directoryRef)
            is RequestBody.TerminalCreate->{
                val original=context.original as? CreationDirectoryOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
                if(body.params.source!=original.source)throw StorePolicy(ErrorCode.StaleSession)
                Triple(body.params.serverId,body.params.projectId,body.params.directoryRef)
            }
            else->throw StorePolicy(ErrorCode.InvalidRequest)
        }
        if(server!=context.server||project!=context.project)throw StorePolicy(ErrorCode.StaleSession)
        reference?.let {if(android.os.SystemClock.elapsedRealtime()>=context.expiresAt||it !in context.references)throw StorePolicy(ErrorCode.CursorExpired)}
    }
    /** Original-view claim; configured roots and scoped references remain independent directory authority. */
    internal fun captureAgentSession(host:String,target:Target,source:Uuid,expected:SessionReducer):AgentSessionMutationContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val summary=expected.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        val subscription=live.subscriptions.entries.singleOrNull {it.value.target==target}?.key ?: throw StorePolicy(ErrorCode.NotReady)
        val original=AgentSessionContext(host,live,live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady),expected,target,source,subscription,summary.serverId,summary.provider,summary.workspaceId,summary.paneId,summary.terminalId)
        val context=AgentSessionMutationContext(original,AgentSessionLease())
        validateAgentSessionOrigin(context,live);return context
    }
    internal fun agentSessionCurrent(context:AgentSessionMutationContext):Boolean=try {
        val original=context.original as? AgentSessionContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        validateAgentSessionOrigin(context,original.live,false);true
    }catch(_:Exception){false}
    private fun validateAgentSessionOrigin(context:AgentSessionMutationContext,live:Live,ready:Boolean=true)=context.lease.admit {
        val original=context.original as? AgentSessionContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(original.live!==live||sessions[original.host]!==live||!live.alive.get()||!live.authenticated||live.hello?.bootId!=original.boot||live.reducers[original.target.sessionId]!==original.reducer||original.reducer.source!=original.source||original.subscription==null||live.subscriptions[original.subscription]?.target!=original.target)throw StorePolicy(ErrorCode.StaleSession)
        val summary=original.reducer.state?.summary ?: throw StorePolicy(ErrorCode.NotReady)
        if(Target(summary.sessionId,summary.generation)!=original.target||summary.serverId!=original.server||summary.provider!=original.provider||summary.workspaceId!=original.workspace||summary.paneId!=original.pane||summary.terminalId!=original.terminal)throw StorePolicy(ErrorCode.StaleSession)
        if(summary.connectivity!=SessionConnectivity.Ready&&summary.connectivity!=SessionConnectivity.Synchronizing)throw StorePolicy(ErrorCode.NotReady)
        if(ready&&(!original.reducer.mutationsEnabled||!live.hostReducer.ready||!live.hostReducer.sourceCurrent(original.server)))throw StorePolicy(ErrorCode.NotReady)
    }
    private fun validateAgentSession(context:AgentSessionMutationContext,live:Live,body:RequestBody.SessionCreate)=context.lease.admit {
        validateAgentSessionOrigin(context,live)
        val original=context.original as AgentSessionContext
        body.params.placement?.let {if(it.sourceTarget!=original.target||body.params.serverId!=original.server)throw StorePolicy(ErrorCode.InvalidRequest)}
        mutationContext(live,body)
    }
    internal fun captureTerminalCreation(host:String,server:Uuid,workspace:Id,source:ArtifactSource?):TerminalCreationMutationContext {
        val live=sessions[host] ?: throw StorePolicy(ErrorCode.NotReady)
        val inventory=live.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        val graph=inventory.hierarchy.singleOrNull {it.serverId==server&&it.freshness==NativeFreshness.Current} ?: throw StorePolicy(ErrorCode.NotReady)
        val target=(source as? ArtifactSource.Session)?.target
        val session=target?.let {value->inventory.sessions.singleOrNull {Target(it.sessionId,it.generation)==value} ?: throw StorePolicy(ErrorCode.NotReady)}
        val shell=(source as? ArtifactSource.Shell)?.target?.let {value->inventory.shells.singleOrNull {it.target==value} ?: throw StorePolicy(ErrorCode.NotReady)}
        val original=TerminalCreationOriginal(host,live,live.hello?.bootId ?: throw StorePolicy(ErrorCode.NotReady),server,workspace,graph.serverIncarnation,source,session,shell)
        return TerminalCreationMutationContext(original,TerminalCreationLease()).also {validateTerminalCreationOrigin(it,live)}
    }
    internal fun terminalCreationCurrent(context:TerminalCreationMutationContext):Boolean=try {
        val original=context.original as? TerminalCreationOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
        validateTerminalCreationOrigin(context,original.live,false);true
    }catch(_:Exception){false}
    private fun validateTerminalCreationOrigin(context:TerminalCreationMutationContext,live:Live,ready:Boolean=true)=context.lease.admit {
        val original=context.original as? TerminalCreationOriginal ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(original.live!==live||sessions[original.host]!==live||!live.alive.get()||!live.authenticated||live.hello?.bootId!=original.boot)throw StorePolicy(ErrorCode.StaleSession)
        val inventory=live.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        val graph=inventory.hierarchy.singleOrNull {it.serverId==original.server&&it.serverIncarnation==original.incarnation} ?: throw StorePolicy(ErrorCode.StaleSession)
        if(graph.freshness==NativeFreshness.Unavailable||graph.workspaces.none {it.workspaceId==original.workspace}
            ||inventory.servers.singleOrNull {it.serverId==original.server}?.connectivity!=ServerConnectivity.Online)throw StorePolicy(ErrorCode.StaleSession)
        when(val source=original.source) {
            is ArtifactSource.Session->{
                val captured=original.session ?: throw StorePolicy(ErrorCode.StaleSession)
                val current=inventory.sessions.singleOrNull {Target(it.sessionId,it.generation)==source.target} ?: throw StorePolicy(ErrorCode.StaleSession)
                if(current.serverId!=original.server||current.workspaceId!=original.workspace||current.terminalId!=captured.terminalId
                    ||current.paneId!=captured.paneId||current.provider!=captured.provider||current.nativeId!=captured.nativeId)throw StorePolicy(ErrorCode.StaleSession)
            }
            is ArtifactSource.Shell->{
                val shell=inventory.shells.singleOrNull {it.target==source.target} ?: throw StorePolicy(ErrorCode.StaleSession)
                if(original.shell==null||!shell.sameLogicalIdentity(original.shell)||shell.serverId!=original.server||shell.workspaceId!=original.workspace)throw StorePolicy(ErrorCode.StaleSession)
                if(ready&&shell.connectivity!=SessionConnectivity.Ready)throw StorePolicy(ErrorCode.NotReady)
            }
            null->{}
        }
        if(ready&&(!live.hostReducer.ready||!live.hostReducer.sourceCurrent(original.server)||graph.freshness!=NativeFreshness.Current))throw StorePolicy(ErrorCode.NotReady)
    }
    private fun validateTerminalCreation(context:TerminalCreationMutationContext,live:Live,body:RequestBody.TerminalCreate)=context.lease.admit {
        validateTerminalCreationOrigin(context,live)
        val original=context.original as TerminalCreationOriginal
        if(body.params.serverId!=original.server||body.params.workspaceId!=original.workspace||body.params.source!=original.source)throw StorePolicy(ErrorCode.StaleSession)
        if(live.hostReducer.inventory?.projects?.none {it.projectId==body.params.projectId&&it.available}!=false)throw StorePolicy(ErrorCode.NotReady)
        mutationContext(live,body)
    }
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
    suspend fun mutate(host:String,request:Request,ordinaryComposer:Boolean=false,approval:RecoveryApproval=RecoveryApproval(),shell:ShellMutationContext?=null,alias:AgentAliasMutationContext?=null,attachment:AttachmentMutationContext?=null,terminal:TerminalCreationMutationContext?=null,agentCreation:AgentSessionMutationContext?=null,skill:SkillInvocationContext?=null,directories:CreationDirectoriesContext?=null,controls:ControlsMutationContext?=null,queue:QueueMutationContext?=null):Receipt = withContext(serial) {
        val live=originalLive(host,request.id);val body=request.body
        if((body is RequestBody.TerminalCreate||body is RequestBody.ShellInput||body is RequestBody.ShellResize||body is RequestBody.SessionControlsApply||body.queueTarget()!=null)&&store.localAction(host,request.id)!=null) {
            val key=secrets.receipt(host,store)
            val matched=try {store.matchingAction(host,request,key)}finally {key.fill(0)}
            if(!matched)throw StorePolicy(ErrorCode.NotReady)
            return@withContext when(val status=status(host,request.id)) {
                is ActionStatusResult.Known->status.value.receipt
                is ActionStatusResult.Unknown->throw StorePolicy(ErrorCode.NotReady)
            }
        }
        if(body.attachmentTarget()==null&&body !is RequestBody.TerminalCreate&&body !is RequestBody.AgentRename&&body !is RequestBody.WorkspaceRename&&body !is RequestBody.PromptSubmit&&body !is RequestBody.TurnCancel&&body !is RequestBody.RequestRespond&&body !is RequestBody.SessionCreate&&body !is RequestBody.ShellInput&&body !is RequestBody.ShellResize&&body !is RequestBody.SessionControlsApply&&body.queueTarget()==null)throw StorePolicy(ErrorCode.InvalidRequest)
        if(agentCreation!=null)validateAgentSession(agentCreation,live,body as? RequestBody.SessionCreate ?: throw StorePolicy(ErrorCode.InvalidRequest))
        if((body is RequestBody.TerminalCreate)!=(terminal!=null))throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.TerminalCreate)validateTerminalCreation(terminal!!,live,body)
        if((body is RequestBody.ShellInput || body is RequestBody.ShellResize)!=(shell!=null)) throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.ShellInput || body is RequestBody.ShellResize) validateShellMutation(shell!!,live,body)
        if((body is RequestBody.AgentRename)!=(alias!=null)) throw StorePolicy(ErrorCode.InvalidRequest)
        if(body is RequestBody.AgentRename) validateAgentAlias(alias!!,live,body)
        if((body.attachmentTarget()!=null)!=(attachment!=null))throw StorePolicy(ErrorCode.InvalidRequest)
        if(attachment!=null)validateAttachment(attachment,live,body)
        if(directories!=null)validateCreationDirectoryMutation(directories,live,body)
        else if(body is RequestBody.SessionCreate&&body.params.directoryRef!=null||body is RequestBody.TerminalCreate&&body.params.directoryRef!=null)throw StorePolicy(ErrorCode.NotReady)
        if((body is RequestBody.SessionControlsApply)!=(controls!=null)||(body.queueTarget()!=null)!=(queue!=null))throw StorePolicy(ErrorCode.NotReady)
        val continuation=NewActionContinuation(host,live,request,currentCoroutineContext()[Job]!!,shell,alias,attachment,terminal,agentCreation,skill,directories,controls,queue)
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
                when(result) {
                    is ResultBody.SessionControlsApply->result.result.receipt
                    is ResultBody.SessionQueueUpdate->result.result.receipt
                    is ResultBody.SessionQueueDelete->result.result.receipt
                    is ResultBody.SessionQueueReorder->result.result.receipt
                    is ResultBody.SessionQueueStart->result.result.receipt
                    is ResultBody.TerminalCreate->result.result.receipt
                    is ResultBody.AttachmentsBegin->result.result.receipt
                    is ResultBody.AttachmentsChunk->result.result.receipt
                    is ResultBody.AttachmentsComplete->result.result.receipt
                    is ResultBody.AttachmentsDiscard->result.result.receipt
                    is ResultBody.AgentRename->result.result.receipt
                    is ResultBody.WorkspaceRename->result.result.receipt
                    is ResultBody.PromptSubmit->result.result.receipt
                    is ResultBody.TurnCancel->result.result.receipt
                    is ResultBody.RequestRespond->result.result.receipt
                    is ResultBody.SessionCreate->result.result.receipt
                    is ResultBody.ShellInput->result.result.receipt
                    is ResultBody.ShellResize->result.result.receipt
                    else->invalidWire()
                }
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
            live.grid?.let(::endGrid)
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
    /** TEMPORARY ephemeral guard. Hello is assigned once before this Live is installed. */
    internal fun debugEligibilityOriginalGuard(host:String,expectedHello:HelloResult):(() -> Boolean)? {
        val original=sessions[host] ?: return null
        if(original.host.hostId!=host || expectedHello.hostId.value!=host || original.hello!==expectedHello)return null
        return {sessions[host]===original && original.hello===expectedHello && original.alive.get() && !original.disposed.get()}
    }
    suspend fun recreate():String = hostOperations.withLock {shutdownEndpoint();bindEndpoint()}
    private fun shellContext(host:String,target:ShellTarget,source:Uuid):ShellReadContext {
        val live=live(host)
        val snapshot=live.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        if(snapshot.subscription!=source)throw StorePolicy(ErrorCode.StaleSession)
        val shell=snapshot.shells.singleOrNull {it.target==target} ?: throw StorePolicy(ErrorCode.StaleSession)
        return ShellReadContext(host,live,live.hostReducer,shell).also(::validateShellRead)
    }
    private fun shellReadCurrent(context:ShellReadContext):Boolean {
        if(!context.active.get() || sessions[context.host]!==context.live || !context.live.alive.get() || !context.live.authenticated || context.live.hostReducer!==context.hostReducer) throw StorePolicy(ErrorCode.StaleSession)
        val inventory=context.hostReducer.inventory ?: throw StorePolicy(ErrorCode.NotReady)
        val graph=inventory.hierarchy.singleOrNull {it.serverId==context.shell.serverId} ?: throw StorePolicy(ErrorCode.StaleSession)
        if(graph.serverIncarnation!=context.incarnation || inventory.servers.singleOrNull {it.serverId==context.shell.serverId}?.connectivity!=ServerConnectivity.Online)throw StorePolicy(ErrorCode.StaleSession)
        val current=inventory.shells.singleOrNull {it.target==context.shell.target} ?: throw StorePolicy(ErrorCode.StaleSession)
        if(!current.sameLogicalIdentity(context.shell))throw StorePolicy(ErrorCode.StaleSession)
        if(graph.freshness==NativeFreshness.Unavailable)throw StorePolicy(ErrorCode.StaleSession)
        if(graph.freshness==NativeFreshness.Stale||!context.hostReducer.ready)return false
        if(graph.freshness!=NativeFreshness.Current || !context.hostReducer.ready || !context.hostReducer.sourceCurrent(current.serverId) || current.connectivity!=SessionConnectivity.Ready || !current.capabilities.read)throw StorePolicy(ErrorCode.NotReady)
        return true
    }
    private fun validateShellRead(context:ShellReadContext){if(!shellReadCurrent(context))throw StorePolicy(ErrorCode.NotReady)}
    /** Admission only. Invalidating the viewer lease never cancels a committed frame. */
    internal suspend fun captureShellMutation(host:String,target:ShellTarget,source:Uuid,lease:ShellMutationLease,purpose:ShellMutationPurpose,inputObservation:Pair<Uuid,Uuid>?=null):ShellMutationContext=withContext(serial) {
        if(!lease.active.get()) throw StorePolicy(ErrorCode.StaleSession)
        val original=shellContext(host,target,source)
        val allowed=when(purpose){ShellMutationPurpose.Input->original.shell.capabilities.input;ShellMutationPurpose.Resize->original.shell.capabilities.grid}
        if(!allowed)throw StorePolicy(ErrorCode.NotReady)
        if(inputObservation!=null && (purpose!=ShellMutationPurpose.Input || original.live.shellInputStatuses[inputObservation.first]?.observation?.token!=inputObservation.second))throw StorePolicy(ErrorCode.NotReady)
        val grid=original.live.grid?.takeIf {it.shell?.shell?.target==target&&validateGrid(it)&&it.reducer.committed!=null} ?: throw StorePolicy(ErrorCode.NotReady)
        if(grid.reducer.committed?.begin?.popupBlocked==true)throw StorePolicy(ErrorCode.NotReady)
        ShellMutationContext(original,lease,purpose,inputObservation,grid,grid.subscription)
    }
    private fun validateShellMutation(context:ShellMutationContext,live:Live,body:RequestBody) {
        val (target,purpose)=when(body){is RequestBody.ShellInput->body.params.target to ShellMutationPurpose.Input;is RequestBody.ShellResize->body.params.target to ShellMutationPurpose.Resize;else->throw StorePolicy(ErrorCode.InvalidRequest)}
        if(context.purpose!=purpose) throw StorePolicy(ErrorCode.InvalidRequest)
        context.inputObservation?.let {(action,token)->if(purpose!=ShellMutationPurpose.Input || live.shellInputStatuses[action]?.observation?.token!=token)throw StorePolicy(ErrorCode.NotReady)}
        if(!context.lease.active.get()) throw StorePolicy(ErrorCode.StaleSession)
        val original=context.original as? ShellReadContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(original.live!==live || original.host!=live.host.hostId || original.shell.target!=target) throw StorePolicy(ErrorCode.StaleSession)
        validateShellRead(original)
        val allowed=when(purpose){ShellMutationPurpose.Input->original.shell.capabilities.input;ShellMutationPurpose.Resize->original.shell.capabilities.grid}
        if(!allowed)throw StorePolicy(ErrorCode.NotReady)
        val subscription=when(body){is RequestBody.ShellInput->body.params.subscription;is RequestBody.ShellResize->body.params.subscription;else->throw StorePolicy(ErrorCode.InvalidRequest)}
        val grid=context.grid as? GridReadContext ?: throw StorePolicy(ErrorCode.InvalidRequest)
        if(live.grid!==grid || grid.subscription!=subscription || context.gridSubscription!=grid.subscription || !validateGrid(grid) || grid.reducer.committed==null || grid.reducer.committed?.begin?.popupBlocked==true)throw StorePolicy(ErrorCode.StaleSession)
    }
    /** Both input and resize bind this exact active original control lease. */
    internal suspend fun shell(host:String,target:ShellTarget,source:Uuid,display:(TerminalGrid?,TerminalGridLease)->Unit)=withContext(serial) {
        val context=shellContext(host,target,source)
        if(!context.shell.capabilities.grid)throw StorePolicy(ErrorCode.NotReady)
        gridStream(context.live,RequestBody.ShellGridSubscribe(ShellGridSubscribeParams(target)),null,context,display)
    }

}

