// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Application
import android.content.Context
import androidx.compose.runtime.*
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first

data class ConversationKey(val host: String, val target: Target)
internal data class AttentionInboxState(val host:String,val ticket:Long,val entries:List<StoredAttentionEvent> = emptyList(),val loading:Boolean=true,val error:Boolean=false)
internal data class AgentAliasScope(val key:ConversationKey,val source:Uuid,val lease:AgentAliasLease)
data class TerminalSnapshot(val key:ConversationKey,val source:Uuid,val ticket:Long,val grid:TerminalGrid?,val loading:Boolean,val error:Boolean)
data class WorktreesSnapshot(val key: ConversationKey,val source: Uuid,val ticket: Long,val worktrees: List<WorktreeEntry>?,val loading: Boolean,val error: Boolean)
data class SkillsSnapshot(val key:ConversationKey,val source:Uuid,val generation:Uuid,val ticket:Long,val catalogue:SkillsListResult?,val loading:Boolean,val error:Boolean)
data class SkillsDocumentState(val snapshot:SkillsSnapshot,val entry:SkillEntry,val ticket:Any,val loading:Boolean=true,val text:String?=null,val failed:Boolean=false)
data class ChangesSnapshot(val key: ConversationKey,val source: Uuid,val ticket: Long,val changes: ChangesReadResult?,val loading: Boolean,val error: Boolean)
data class ShellSelection(val host:String,val target:ShellTarget)
data class ShellSnapshot(val selection:ShellSelection,val source:Uuid,val ticket:Long,val summary:ShellSummary,val grid:TerminalGrid?,val loading:Boolean,val error:Boolean,val controlBegun:Boolean=false,val incarnation:Uuid?=null,val originalHello:HelloResult?=null,val history:ShellHistoryState=ShellHistoryState())
data class ShellInputDraft(val value:androidx.compose.ui.text.input.TextFieldValue=androidx.compose.ui.text.input.TextFieldValue(),val revision:Long=0,val direct:Boolean=false,val keyCharacter:String="",val modifiers:TerminalModifierState=TerminalModifierState(),val queuedRevision:Long?=null)
data class ShellInputGate(val scope:ShellControlScope,val loaded:Boolean,val prior:LocalAction?,val pending:Boolean,val observation:ShellInputObservation?=null)
data class ShellControlScope(val selection:ShellSelection,val source:Uuid,val ticket:Long)
private data class ShellSubmittedInput(val selection:ShellSelection,val revision:Long?,val text:String) {
    override fun toString()="ShellSubmittedInput(redacted)"
}
enum class Screen { Connections, Pairing, Sessions, Conversation, Terminals, Settings }
data class SendReservation(val key: ConversationKey, val action: Uuid, val text: String, val warning: RecoveryWarning,val options:ComposerOptions=ComposerOptions())
private data class SubmittedComposer(val key:ConversationKey,val text:String,val options:ComposerOptions,val action:Uuid)
private data class ForegroundGeneration(val claim: RuntimeClaim, val ready: CompletableDeferred<Unit>)
/** Resolve the displayed continuation before a read can advance its loaded-view revision. */
internal fun nativeQueueReadCursor(shown:SessionControlsState,currentRevision:Long,cursor:Id?):Id? {
    if(cursor==null||currentRevision==shown.queueRevision)return cursor
    shown.observeRevisions(shown.revision,currentRevision)
    shown.queueEntries=emptyList();shown.queueText=emptyMap();shown.next=null;shown.queueLoaded=false
    return null
}

/** Local-only list presentation; it never carries session, host, or receipt state. */
data class SessionListPreferences(
    val directoryView: SessionDirectoryView = SessionDirectoryView.Workspaces,
    val groupByStatus: Boolean = true,
    val showWorkspaceContext: Boolean = true,
    val showHostContext: Boolean = true,
)

/** Route-surviving local filters, never persisted native authority. */
internal class SessionListFilters {
    var host by mutableStateOf<String?>(null)
    var query by mutableStateOf("")
    var provider by mutableStateOf<Provider?>(null)
    var status by mutableStateOf<SessionStatus?>(null)
    fun clearFilters(){host=null;provider=null;status=null}
    fun forgetHost(removed:String){if(host==removed)host=null;query=""}
    fun retainHosts(paired:Set<String>){if(host!=null&&host !in paired){host=null;query=""}}
}
internal data class ConversationReaderIdentity(val server:Uuid,val incarnation:Uuid,val provider:Provider,val native:Id)
internal data class ConversationReaderAnchor(val row:String,val order:Long?,val offset:Int)
/** One original conversation's metadata-only reading intent, not transcript storage or admission. */
internal class ConversationReaderPosition(val identity:ConversationReaderIdentity?) {
    var following by mutableStateOf(true)
    var toolsExpanded by mutableStateOf(false)
    private var olderView=false
    fun observeHistoryView(older:Boolean){if(older!=olderView){olderView=older;toolsExpanded=older}}
    var anchor:ConversationReaderAnchor?=null
        private set
    var readTail by mutableStateOf<Id?>(null)
        private set
    var anchorMissing by mutableStateOf(false)
        private set
    private var ticket:Any?=null
    fun attach():Any=Any().also {ticket=it}
    fun retire(){ticket=null}
    fun current(expected:Any?)=expected!=null&&ticket===expected
    fun release(expected:Any?){if(ticket===expected)ticket=null}
    fun capture(expected:Any?,row:String,order:Long?,itemOffset:Int,atEnd:Boolean,scrolling:Boolean,older:Boolean,tail:Id?):Boolean {
        if(!current(expected)||anchorMissing&&!scrolling)return false
        if(scrolling){following=atEnd&&!older;anchorMissing=false}
        val boundedOffset=(-itemOffset.toLong()).coerceIn(0,Int.MAX_VALUE.toLong()).toInt()
        val previous=anchor
        if(previous==null||previous.row!=row||previous.order!=order||previous.offset!=boundedOffset)anchor=ConversationReaderAnchor(row,order,boundedOffset)
        if(following)readTail=tail
        return true
    }
    fun latest(tail:Id?){following=true;anchorMissing=false;readTail=tail}
    fun locate(history:List<HistoryRecord>,focused:Uuid?,receipts:List<Uuid>):Int? {
        val saved=anchor ?: return null
        val start=1+if(focused!=null)1 else 0
        val exact=when {
            saved.row=="conversation_header" -> 0
            saved.row=="conversation_activity_tail" -> start+history.size
            saved.row.startsWith("history_") -> {
                val id=saved.row.removePrefix("history_")
                history.indexOfFirst {it.id.value==id}.takeIf {it>=0}?.let {start+it}
            }
            saved.row.startsWith("receipt_") -> {
                val id=saved.row.removePrefix("receipt_")
                if(focused?.value==id)1 else receipts.indexOfFirst {it.value==id}.takeIf {it>=0}?.let {start+history.size+1+it}
            }
            else -> null
        }
        anchorMissing=exact==null
        if(exact!=null)return exact
        if(history.isEmpty())return 0
        val nearest=saved.order?.let {order->history.indexOfFirst {it.order.value>=order}.takeIf {it>=0}} ?: history.lastIndex
        return start+nearest
    }
}


/** Retained constructor wiring, not another protocol/state owner. Nothing here uses SavedState. */
class GoatrViewModel(application: Application) : AndroidViewModel(application) {
    private val runtime=(application as GoatrApplication).runtime
    val store = runtime.store
    val connection = runtime.connection
    val deliveryPreferences=runtime.deliveryPreferences
    var delivery by mutableStateOf(DeliverySnapshot())
        private set
    var notificationPermissionRequest by mutableStateOf(0L)
        private set
    internal var notificationPermissionLaunch by mutableStateOf<Long?>(null)
        private set
    private var deliveryPreferenceRevision by mutableStateOf(0L)
    private var permissionRequest:Pair<Long,()->Boolean>?=null
    @Volatile private var navigationTicket=Any()
    private var attentionRouteJob:Job?=null
    // Sessions is the landing console. A host filter is presentation state only.
    var screen by mutableStateOf(Screen.Sessions)
        private set
    var host by mutableStateOf<String?>(null)
        private set
    var selection by mutableStateOf<ConversationKey?>(null)
        private set
    var foreground by mutableStateOf(false)
        private set
    var busy by mutableStateOf(false)
        private set
    var notice by mutableStateOf<String?>(null)
        private set
    var preview by mutableStateOf<PairingPreview?>(null)
        private set
    var reservation by mutableStateOf<SendReservation?>(null)
        private set
    var replacement by mutableStateOf<SendReservation?>(null)
        private set
    private val shellState=mutableStateOf<ShellSnapshot?>(null)
    var shell by shellState
        private set
    val shellInputDrafts=mutableStateMapOf<ShellSelection,ShellInputDraft>()
    var shellInputGate by mutableStateOf<ShellInputGate?>(null)
        private set
    var separateShellInput by mutableStateOf<Pair<ShellControlScope,Uuid>?>(null)
        private set
    private val shellInputActions=mutableMapOf<ShellSelection,Uuid>()
    private val shellSubmittedInputs=mutableStateMapOf<Uuid,ShellSubmittedInput>()
    var clearShellDraftHost by mutableStateOf<String?>(null)
        private set
    private var shellInputObservedStatus:ShellInputObservation?=null
    private var shellInputSeparateStatus:ShellInputObservation?=null
    private var shellInputPermitStatus:ShellInputObservation?=null
    private var shellInputObserved:Pair<ShellSelection,Uuid>?=null
    private var shellInputPermit:Pair<ShellControlScope,Uuid>?=null
    private var shellInputGeneration=0L
    private var shellJob:Job?=null
    private var shellMutationLease:ShellMutationLease?=null
    private var shellTicket=0L
    private var terminalInputOwner by mutableStateOf<TerminalInputOwner?>(null)
    private var terminalInputScope:ShellControlScope?=null
    private var terminalInputToken:Any?=null
    private var terminalInputPreparation:Job?=null
    internal var terminalInputPause by mutableStateOf<TerminalInputPause?>(null)
        private set
    private val terminalState=mutableStateOf<TerminalSnapshot?>(null)
    var terminal by terminalState
        private set
    var worktrees by mutableStateOf<WorktreesSnapshot?>(null)
        private set
    var skills by mutableStateOf<SkillsSnapshot?>(null)
    var skillsDocument by mutableStateOf<SkillsDocumentState?>(null)
        private set
    private var skillsDocumentJob:Job?=null
        private set
    var skillsFocusSignal by mutableStateOf(0L)
        private set
    private var skillsCatalogue by mutableStateOf<SkillsSnapshot?>(null)
    private var skillsTicket=0L
    private var skillsJob:Job?=null
    private var skillsInvocationLease:SkillInvocationLease?=null
    var changes by mutableStateOf<ChangesSnapshot?>(null)
        private set
    var actions by mutableStateOf<Map<String,List<LocalAction>>>(emptyMap())
        private set
    var focusedAction by mutableStateOf<LocalAction?>(null)
        private set
    var nextActionPage by mutableStateOf<Uuid?>(null)
        private set
    private var actionAfter: Uuid? = null
    private var focusedId: Uuid? = null
        set(value) {
            field=value
            if(focusedAction?.metadata?.actionId!=value)focusedAction=null
        }
    private var observedHello: Map<String,HelloResult> = emptyMap()
    private var observedReceipts: Map<String,Receipt> = emptyMap()
    val composers = mutableStateMapOf<ConversationKey,String>()
    internal val composerUploads=mutableStateMapOf<ConversationKey,List<ComposerImageInput>>()
    private val composerDeliveries=mutableStateMapOf<ConversationKey,Pair<PromptDelivery,Id?>>()
    internal var sessionControls by mutableStateOf<SessionControlsState?>(null)
        private set
    private var controlsReadJob:Job?=null
    private var queueReadJob:Job?=null
    val forms = mutableMapOf<Triple<String,Target,Uuid>,FormMemory>()
    val creationForms = mutableMapOf<String,FormMemory>()
    val creationActions = mutableStateMapOf<String,Uuid>()
    // This is only a presentation confirmation for one exact retained action.
    var separateCreationAction by mutableStateOf<Uuid?>(null)
        private set
    private var separateCreationGeneration=0L
    val connectingHosts = mutableStateMapOf<String,Boolean>()
    // Presentation preferences only; they never persist drafts or change host/session state.
    private val presentationPreferences = application.getSharedPreferences(PRESENTATION_PREFERENCES, Context.MODE_PRIVATE)
    private var appearanceState by mutableStateOf(loadAppearance())
    private var sessionListPreferencesState by mutableStateOf(loadSessionListPreferences())
    internal val sessionFilters=SessionListFilters()
    private val readerPositions=LinkedHashMap<ConversationKey,ConversationReaderPosition>()
    var recentlyOpenedSessions by mutableStateOf<List<ConversationKey>>(emptyList())
        private set
    var appearance: Appearance
        get() = appearanceState
        set(value) {
            appearanceState = value
            // Apply updates in memory immediately and serialize without a UI-thread disk wait.
            presentationPreferences.edit().putString(APPEARANCE_KEY, value.name).apply()
        }
    var sessionListPreferences: SessionListPreferences
        get() = sessionListPreferencesState
        set(value) {
            sessionListPreferencesState = value
            presentationPreferences.edit()
                .putString(SESSION_DIRECTORY_VIEW_KEY, value.directoryView.name)
                .putBoolean(SESSION_LIST_GROUP_BY_STATUS_KEY, value.groupByStatus)
                .putBoolean(SESSION_LIST_SHOW_WORKSPACE_CONTEXT_KEY, value.showWorkspaceContext)
                .putBoolean(SESSION_LIST_SHOW_HOST_CONTEXT_KEY, value.showHostContext)
                .apply()
        }
    private val sentText = mutableStateMapOf<RecoveryIdentity,SubmittedComposer>()
    private var gate: PairingGate? = null
    private var pairingJob: Job? = null
    private var warningJob: Job? = null
    private val lifecycle = Channel<Boolean>(Channel.UNLIMITED)
    private var operation: Job? = null
    private var startup: Job? = null
    private var terminalJob: Job? = null
    private var terminalTicket=0L
    private val attachmentStaging=AttachmentStaging(application)
    internal var attachmentTransfer by mutableStateOf<AttachmentTransfer?>(null)
        private set
    internal var attachmentVisible by mutableStateOf(false)
        private set
    private var attachmentJob:Job?=null
    private var attachmentPreviewJob:Job?=null
    private var attachmentCleanup:AttachmentMutationContext?=null
    private var attachmentPickerConsent:AttachmentConsentScope?=null
    internal var artifacts by mutableStateOf<ArtifactBrowserState?>(null)
        private set
    private class ArtifactPickerHold(val ticket:ArtifactPickerTicket,val claim:RuntimeClaim,val expires:Long){var result:android.net.Uri?=null;var returnedForeground:Any?=null}
    private var artifactPickerHold:ArtifactPickerHold?=null
    private var artifactExportJob:Job?=null
    private class PickerHold(val ticket:AttachmentSourceTicket,val original:AttachmentConsentScope,val claim:RuntimeClaim,val expires:Long)
    private var attachmentPickerHold:PickerHold?=null
    internal var attachmentRecovery by mutableStateOf<AttachmentRecovery?>(null)
        private set
    private val composerRevisions=mutableMapOf<ConversationKey,Long>()
    private val composerCursors=mutableStateMapOf<ConversationKey,Pair<Int,Int>>()
    private var worktreesJob: Job? = null
    private var worktreesTicket=0L
    private var changesJob: Job? = null
    private var changesTicket=0L
    private var lifecycleActive = false
    /** Replaced atomically on every foreground entry; stale cleanup cannot release a new entry. */
    @Volatile private var foregroundGeneration: ForegroundGeneration?=null
    /** TEMPORARY main-owner scalar capture; never fetch action pages to improve dump knowledge. */
    internal fun debugEligibilityMain(request:DebugEligibilityArgs,publication:ConnectionView):DebugEligibilityMain {
        val generation=foregroundGeneration;val active=foreground;val operationBusy=busy
        val paired=store.state.value;val pageActions=actions;val focused=focusedAction
        val creation=creationActions[request.host];val currentHost=host;val currentScreen=screen
        val ongoing=creationOngoing(request.host,publication.inventory[request.host],this,focused?.takeIf {it.metadata.method==ActionMethod.SessionCreate})
        // A positive resident latch is known. Baseline has no freshness/completeness proof of absence.
        return DebugEligibilityMain(active,operationBusy,if(ongoing)true else null,paired.hosts.any {it.hostId==request.host}) {
            generation!=null && foreground==active && foregroundGeneration===generation && busy==operationBusy &&
                store.state.value===paired && actions===pageActions && focusedAction===focused &&
                creationActions[request.host]===creation && host==currentHost && screen==currentScreen
        }
    }
    internal fun debugEligibilityGate(request:DebugEligibilityArgs,publication:ConnectionView,main:DebugEligibilityMain,connection:DebugEligibilityConnection):Boolean? {
        if(!main.foreground || main.busy)return false
        if(!main.paired || !connection.scopeMatched)return null
        // Backend availability is diagnostic: the existing CreateSession conjunction uses canCreate.
        val allowed=creationReadinessReason(publication.hello[request.host],main.busy,connection.inventoryReady,connection.mutationReady,main.ongoing==true)==null
        return if(!allowed)false else if(main.ongoing==null)null else true
    }
    private data class PendingAttention(val host:String,val event:Uuid,val navigation:Any)
    @Volatile private var pendingAttention:PendingAttention?=null
    internal var attentionInbox by mutableStateOf<AttentionInboxState?>(null)
        private set
    private var attentionInboxForeground:ForegroundGeneration?=null
    private var attentionInboxTicket=0L
    private var attentionInboxJob:Job?=null


    internal var agentSessionCreation by mutableStateOf<AgentSessionCreation?>(null)
        private set
    internal var terminalCreation by mutableStateOf<TerminalCreationState?>(null)
        private set
    private var creationDirectoriesBrowser:CreationDirectoryBrowser?=null

    init {
        viewModelScope.launch {store.open();while(isActive){cleanupLocalAttachments();delay(30000)}}
        viewModelScope.launch {
            refreshActions()
            for (active in lifecycle) {
                if (active == lifecycleActive) continue
                lifecycleActive = active
                if (active) {
                    val generation=foregroundGeneration ?: continue
                    val selected=selection
                    startup=launch {
                        fun originalForeground()=foreground && foregroundGeneration===generation && selection==selected
                        if(!originalForeground()) return@launch
                    try {
                        runtime.acquire(generation.claim)
                        generation.ready.complete(Unit)
                        val connected=mutableSetOf<String>()
                        for (paired in store.state.value.hosts) {
                            if (!originalForeground()) break
                            connectingHosts[paired.hostId]=true
                            try { connection.reconnectAtStartup(paired.hostId);connected.add(paired.hostId) }
                            catch (timeout: TimeoutCancellationException) { currentCoroutineContext().ensureActive();notice = connectionFailureNotice(timeout) }
                            catch (cancel: CancellationException) { throw cancel }
                            catch (error: Exception) { notice = connectionFailureNotice(error) }
                            finally {connectingHosts.remove(paired.hostId)}
                        }
                        selected?.takeIf {originalForeground() && it.host in connected && attachmentPickerHold==null&&artifactPickerHold==null}?.let { key ->
                            // Return reopens only the original exact Target. No write or retarget.
                            try { connection.open(key.host,key.target) }
                            catch (cancel: CancellationException) { throw cancel }
                            catch (error: Exception) { notice = connectionFailureNotice(error) }
                        }
                        refreshActions()
                    } catch (timeout: TimeoutCancellationException) { generation.ready.completeExceptionally(timeout);currentCoroutineContext().ensureActive();notice = connectionFailureNotice(timeout) }
                    catch (cancel: CancellationException) { generation.ready.completeExceptionally(cancel);throw cancel }
                    catch (error: Exception) { generation.ready.completeExceptionally(error);notice = connectionFailureNotice(error) }
                    }
                    startup?.join();startup=null
                } else {
                    creationDirectoriesBrowser?.let(::closeCreationDirectories)
                    operation?.cancelAndJoin(); operation = null; busy = false
                    closePairing(); closeWorktrees(); closeChanges();closeSkills(); forms.values.forEach(FormMemory::clearSecrets)
                    reservation = null; replacement = null; warningJob?.cancel()
                }
            }
        }
        viewModelScope.launch {
            connection.state.collect { view ->
                attachmentTransfer?.let {if(!attachmentAdmitted(it.ticket))pauseAttachments()}
                val current = selection
                sessionControls?.let {shown->
                    if(!controlsAdmitted(shown))closeSessionControls()
                    else {
                        val reducer=view.sessions[shown.key.host]?.firstOrNull {it.state?.summary?.sessionId==shown.key.target.sessionId}
                        if(reducer!=null)shown.observeRevisions(reducer.controlsRevision,reducer.queueRevision)
                        servicePendingNativeReads(shown)
                    }
                }
                agentSessionCreation?.let {if(!agentSessionAdmitted(it))closeAgentSession(it)}
                creationDirectoriesBrowser?.let {if(!it.current())closeCreationDirectories(it)}
                terminalCreation?.let {if(!terminalCreationAdmitted(it))closeTerminalCreation()}
                agentAliasScope?.let {shown->if(!foreground || selection!=shown.key || view.sources[shown.key.host]?.get(shown.key.target)!=shown.source)closeAgentAlias()}
                if(current!=null && (!foreground || screen!=Screen.Conversation || view.sources[current.host]?.containsKey(current.target)!=true)) runtime.setVisibleConversation(null)
                val retained = forms.keys.toList()
                for (key in retained) {
                    val request = view.sessions[key.first]?.firstOrNull { it.state?.summary?.sessionId == key.second.sessionId }
                        ?.takeIf { it.state?.summary?.generation == key.second.generation }?.pending
                        ?.firstOrNull { it.requestKey == key.third }
                    if (request == null || current != ConversationKey(key.first,key.second) || forms[key]?.source != view.sources[key.first]?.get(key.second)) forms.remove(key)?.clear()
                }
                shell?.let {shown->
                    if(!shellRetained(shown)) closeShell()
                    else {
                        val current=shellAdmitted(shown)
                        val latest=view.inventory[shown.selection.host]?.shells?.singleOrNull {it.target==shown.selection.target}
                        if(latest!=null&&latest!=shown.summary)shell=shown.copy(summary=latest,grid=if(current)shown.grid else null)
                        else if(!current&&shown.grid!=null)shell=shown.copy(grid=null,loading=shown.controlBegun,error=false)
                        val scope=ShellControlScope(shown.selection,shown.source,shown.ticket)
                        if(runCatching {requireShellInputScope(scope)}.isFailure)clearShellModifiers(scope)
                        if(runCatching {requireShellInputOwnerScope(scope)}.isFailure)terminalInputOwner?.unavailable()
                        if(current)shellInputGate?.takeIf {it.loaded && it.prior!=null && it.observation!=view.shellInputStatus[shown.selection.host]?.get(it.prior.metadata.actionId)}?.let {loadShellInput(it.scope)}
                    }
                }
                terminal?.let {shown->if(selection!=shown.key || !foreground || view.sources[shown.key.host]?.get(shown.key.target)!=shown.source) closeTerminal()}
                worktrees?.let {shown->if(selection!=shown.key || !foreground || view.sources[shown.key.host]?.get(shown.key.target)!=shown.source) closeWorktrees()}
                artifacts?.let{shown->
                    if(!connection.artifactIdentityCurrent(shown.context)||artifactPickerHold==null&&(!artifactOriginCurrent(shown)||!foreground||foregroundGeneration!==shown.foreground))closeArtifacts()
                    else if(!connection.artifactCurrent(shown.context)){if(!shown.paused){shown.paused=true;shown.preview=null;shown.selected=null;shown.exportConfirmation=null;shown.job?.cancel();shown.error="Files paused while the original desktop inventory refreshes."}}
                    else if(shown.paused){shown.paused=false;shown.error="Original source is ready. Refresh to continue."}
                }
                artifactPickerHold?.takeIf{it.result!=null}?.let(::continueArtifactExportPicker)
                skillsCatalogue?.let {shown->if(!skillsAdmitted(shown)) closeSkills()}
                skills?.let {shown->if(!skillsAdmitted(shown)) closeSkills()}
                changes?.let {shown->if(selection!=shown.key || !foreground || view.sources[shown.key.host]?.get(shown.key.target)!=shown.source) closeChanges()}
                if(view.hello!=observedHello || view.latestReceipt!=observedReceipts) {
                    observedHello=view.hello;observedReceipts=view.latestReceipt
                    refreshActions()
                    shell?.takeIf {shellAdmitted(it)}?.let {loadShellInput(ShellControlScope(it.selection,it.source,it.ticket))}
                }
            }
        }
        viewModelScope.launch {store.state.collect {
            attentionInbox?.takeIf {shown->!attentionInboxAdmitted(shown)}?.let {closeAttentionInbox()}
        }}
        viewModelScope.launch {DeliveryService.status.collect {delivery=it}}
        viewModelScope.launch {deliveryPreferences.changes.collect {deliveryPreferenceRevision=it}}
        viewModelScope.launch {connection.notificationWakeups.collect {wakeup->
            val attentionForeground=foregroundGeneration
            if(foreground&&wakeup.syncRequired)try{
                val expected=connection.state.value.hello[wakeup.host] ?: return@collect
                var after:Uint?=null
                do{if(!foreground||foregroundGeneration!==attentionForeground)break;after=connection.notificationsSync(wakeup.host,expected,after).nextSequence}while(after!=null)
            }catch(cancel:CancellationException){throw cancel}catch(_:Exception){}
            val current=selection
            if(foreground && screen==Screen.Conversation && current?.host==wakeup.host && connection.state.value.sources[current.host]?.containsKey(current.target)==true) {
                runtime.setVisibleConversation(current)
                try {viewed(current)} catch(cancel:CancellationException) {throw cancel} catch(_:Exception) {}
            }else if(foreground)try{runtime.notifications.reconcile(wakeup.host)}catch(cancel:CancellationException){throw cancel}catch(_:Exception){}
            attentionInbox?.takeIf {it.host==wakeup.host && attentionInboxAdmitted(it)}?.let(::refreshAttentionInbox)
        }}
    }
    fun foreground(active: Boolean) {
        val conversation=if(!active)selection?.let {connection.retireConversationSubscription(it.host,it.target)}else null
        if(active && foregroundGeneration==null) foregroundGeneration=ForegroundGeneration(runtime.newClaim(RuntimeClaimKind.Ui),CompletableDeferred())
        foreground = active
        if (!active) {
            closeAttentionInbox()
            // Admission guards change immediately, before queued native cleanup finishes.
            foreground = false
            closeSessionControls()
            pauseArtifacts();closeAgentAlias();closeTerminalCreation();closeAgentSession();attachmentCleanup?.lease?.invalidate();attachmentCleanup=null;attachmentRecovery=null
            val stale=foregroundGeneration
            if(stale!=null){pendingAttention=null;navigationTicket=Any();attentionRouteJob?.cancel()}
            val staleStartup=startup
            foregroundGeneration=null
            staleStartup?.cancel();operation?.cancel();closePairing();closeTerminal();closeShell();closeWorktrees();closeChanges();closeSkills();forms.values.forEach(FormMemory::clearSecrets);invalidateSeparateCreation()
            runtime.setVisibleConversation(null)
            permissionRequest=null;attentionRouteJob=null
            if(stale!=null || conversation!=null) runtime.cleanupScope.launch {
                try {staleStartup?.join()} catch(_:CancellationException) {}
                try {conversation?.let {connection.leave(it)}} catch(_:Exception) {}
                if(stale!=null)try {runtime.release(stale.claim)} catch(_:Exception) {}
            }
        }
        lifecycle.trySend(active)
        if(active) openPendingAttention()
    }
    private fun loadAppearance(): Appearance {
        val stored = try { presentationPreferences.getString(APPEARANCE_KEY, null) }
        catch (_: ClassCastException) { null }
        return Appearance.entries.firstOrNull { it.name == stored } ?: Appearance.System
    }
    private fun loadSessionListPreferences() = SessionListPreferences(
        directoryView = loadSessionDirectoryView(),
        groupByStatus = presentationBoolean(SESSION_LIST_GROUP_BY_STATUS_KEY),
        showWorkspaceContext = presentationBoolean(SESSION_LIST_SHOW_WORKSPACE_CONTEXT_KEY),
        showHostContext = presentationBoolean(SESSION_LIST_SHOW_HOST_CONTEXT_KEY),
    )
    private fun loadSessionDirectoryView(): SessionDirectoryView {
        val stored=try {presentationPreferences.getString(SESSION_DIRECTORY_VIEW_KEY,null)} catch(_:ClassCastException) {null}
        return SessionDirectoryView.entries.firstOrNull {it.name==stored} ?: SessionDirectoryView.Workspaces
    }
    private fun presentationBoolean(key: String): Boolean = try { presentationPreferences.getBoolean(key, true) }
    catch (_: ClassCastException) { true }
    fun formMemory(key: ConversationKey,source: Uuid?,request: Uuid): FormMemory? {
        val view=connection.state.value
        if(selection!=key || source==null || view.sources[key.host]?.get(key.target)!=source ||
            view.sessions[key.host].orEmpty().none {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true && it.pending.any {p->p.requestKey==request}}) return null
        val identity=Triple(key.host,key.target,request)
        if(forms[identity]?.source!=source) forms.remove(identity)?.clear()
        return forms.getOrPut(identity) {FormMemory(source)}
    }
    private fun perform(failureNotice: (Exception) -> String = ::safeFailure, block: suspend () -> Unit) {
        if (busy) return
        busy = true; notice = null
        val task=viewModelScope.launch(start=CoroutineStart.LAZY) {
            val original=currentCoroutineContext()[Job]
            try { block() }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) {if(operation===original)notice=failureNotice(error)}
            finally {if(operation===original){busy=false;try{refreshActions()}finally{openPendingAttention()}}}
        }
        operation=task
        task.start()
    }
    private suspend fun refreshActions() {
        val currentHost=host ?: return
        val currentScreen=screen
        if(store.state.value.hosts.none {it.hostId==currentHost}) return
        val after=actionAfter
        val focus=focusedId ?: (if(currentScreen==Screen.Sessions) creationActions[currentHost] else null) ?: connection.state.value.latestReceipt[currentHost]?.actionId
        val page=store.localActions(currentHost,after,100)
        val focused=focus?.let {store.localAction(currentHost,it)}
        val currentFocus=focusedId ?: (if(currentScreen==Screen.Sessions) creationActions[currentHost] else null) ?: connection.state.value.latestReceipt[currentHost]?.actionId
        if(host==currentHost && screen==currentScreen && actionAfter==after && currentFocus==focus) {
            actions=mapOf(currentHost to page);focusedAction=focused
            nextActionPage=page.takeIf {it.size==100}?.lastOrNull()?.metadata?.actionId
        }
    }
    fun actionPage(first: Boolean=false) {
        actionAfter=if(first) null else (nextActionPage ?: return)
        viewModelScope.launch {try {refreshActions()} catch(error: Exception) {notice=safeFailure(error)}}
    }
    fun connections() { navigate(Screen.Connections,null) }
    fun sessions() { navigate(Screen.Sessions,null) }
    fun sessions(host: String) { navigate(Screen.Sessions,host) }
    fun settings(host: String? = null) { navigate(Screen.Settings,host) }
    private fun navigate(next: Screen, nextHost: String?,old:ConversationSubscription?=selection?.let {connection.retireConversationSubscription(it.host,it.target)}) {
        navigationTicket=Any();pendingAttention=null;permissionRequest=null
        attentionRouteJob?.cancel();attentionRouteJob=null
        closeAttentionInbox()
        closeArtifacts()
        closeAgentAlias()
        skillsFocusSignal=0L
        closeAgentSession();closeTerminalCreation();closePairing(); closeTerminal(); closeShell(); closeWorktrees(); closeChanges();closeSkills(); forms.values.forEach(FormMemory::clear); forms.clear()
        closeSessionControls()
        selection = null; reservation = null; replacement = null; warningJob?.cancel(); invalidateSeparateCreation()
        runtime.setVisibleConversation(null)
        host = nextHost; screen = next
        actions=emptyMap();focusedAction=null;focusedId=null;actionAfter=null;nextActionPage=null
        if(nextHost!=null) viewModelScope.launch {try {refreshActions()} catch(error: Exception) {notice=safeFailure(error)}}
        if (old != null) runtime.cleanupScope.launch {
            try {connection.leave(old)}
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { notice = safeFailure(error) }
        }
    }
    fun open(host: String,target: Target) {
        val selected=ConversationKey(host,target)
        val previous=operation
        navigate(Screen.Conversation,host); selection = selected
        // This records an explicit local open, never native activity or routing authority.
        recentlyOpenedSessions=rememberExplicitSessionOpen(recentlyOpenedSessions,selected)
        viewModelScope.launch {
            previous?.cancelAndJoin()
            if(selection==selected && foreground && connection.state.value.hello.containsKey(host)) perform {
                connection.open(host,target)
                if(selection==selected && connection.state.value.sources[host]?.containsKey(target)==true) runtime.setVisibleConversation(selected)
            }
        }
        reserve(composers[selection].orEmpty())
    }
    fun reconnect(host: String) = perform(::connectionFailureNotice) {
        check(foreground)
        connectingHosts[host]=true
        try {connection.reconnect(host)} finally {connectingHosts.remove(host)}
        selection?.takeIf { it.host == host }?.let { connection.open(host,it.target) }
    }
    internal fun attentionInboxAdmitted(shown:AttentionInboxState):Boolean =
        attentionInbox?.let {it.host==shown.host && it.ticket==shown.ticket}==true &&
            attentionInboxForeground!=null && foregroundGeneration===attentionInboxForeground && foreground &&
            screen==Screen.Sessions && host==shown.host && selection==null &&
            store.state.value.hosts.any {it.hostId==shown.host}
    /** Synchronous navigation only: no pending host handoff may survive a foreground change. */
    internal fun openConsoleAttentionInbox(requestedHost:String) {
        val generation=foregroundGeneration ?: return
        val paired=store.state.value.hosts.singleOrNull {it.hostId==requestedHost} ?: return
        if(busy||!foreground||screen!=Screen.Sessions||host!=null||selection!=null)return
        navigate(Screen.Sessions,requestedHost)
        if(!foreground||foregroundGeneration!==generation||screen!=Screen.Sessions||host!=requestedHost||selection!=null||store.state.value.hosts.none {it==paired})return
        // Navigation invalidated the previous ticket; the ordinary inbox owns a fresh one.
        openAttentionInbox(requestedHost)
    }
    internal fun openAttentionInbox(requestedHost:String) {
        if(!foreground || foregroundGeneration==null || screen!=Screen.Sessions || host!=requestedHost || selection!=null || store.state.value.hosts.none {it.hostId==requestedHost}) return
        closeAttentionInbox()
        attentionInboxForeground=foregroundGeneration
        val shown=AttentionInboxState(requestedHost,attentionInboxTicket)
        attentionInbox=shown
        attentionInboxJob=viewModelScope.launch {
            try {
                // Retained authenticated events only. Listing never syncs or marks read.
                val entries=store.latestUnreadNotifications(requestedHost,100)
                if(attentionInboxAdmitted(shown)) attentionInbox=shown.copy(entries=entries,loading=false)
            } catch(cancel:CancellationException) {throw cancel}
            catch(_:Exception) {if(attentionInboxAdmitted(shown)) attentionInbox=shown.copy(loading=false,error=true)}
        }
    }
    internal fun refreshAttentionInbox(shown:AttentionInboxState) {
        if(attentionInboxAdmitted(shown)) openAttentionInbox(shown.host)
    }
    internal fun closeAttentionInbox() {
        attentionInboxTicket+=1;attentionInboxJob?.cancel();attentionInboxJob=null
        attentionInbox=null;attentionInboxForeground=null
    }
    internal fun openAttentionInboxEntry(shown:AttentionInboxState,entry:StoredAttentionEvent) {
        val current=attentionInbox ?: return
        if(busy || !attentionInboxAdmitted(shown) || current.loading || current.error || entry.read || entry !in current.entries) return
        // Same foreground/host/ticket immediately before the existing exact-event
        // entry route; that route owns connection/sync/stale read-only decisions.
        closeAttentionInbox()
        openNotification(shown.host,entry.event.eventId.value)
    }
    /** Notification extras are lookup keys only. The stored event and live inventory must agree. */
    fun openNotification(host:String,eventId:String) {
        val event=try {EndpointId(host);Uuid(eventId)}catch(_:Exception){return}
        navigationTicket=Any();permissionRequest=null
        attentionRouteJob?.cancel();attentionRouteJob=null
        pendingAttention=PendingAttention(host,event,navigationTicket)
        if(foreground)openPendingAttention()
    }
    private fun openPendingAttention() {
        val entry=pendingAttention ?: return
        val generation=foregroundGeneration ?: return
        if(busy||!foreground||entry.navigation!==navigationTicket)return
        fun admitted()=foreground&&foregroundGeneration===generation&&navigationTicket===entry.navigation
        perform {
            if(!admitted()||pendingAttention!==entry)return@perform
            // Busy keeps the lookup pending; an admitted attempt consumes only its own entry.
            pendingAttention=null
            var expected=connection.state.value.hello[entry.host]
            fun original()=admitted()&&connection.state.value.hello[entry.host]===expected
            try {
                generation.ready.await()
                if(!admitted())return@perform
                if(expected==null)expected=connection.state.value.hello[entry.host]
                if(!original())return@perform
                val connected=connection.ensureConnectedIfNeeded(entry.host)
                if(expected==null)expected=connected
                if(!original()||expected!==connected)return@perform
                val inventory=connection.snapshot(entry.host)
                if(!original())return@perform
                var after:Uint?=null
                do {after=connection.notificationsSync(entry.host,connected,after).nextSequence;if(!original())return@perform}while(after!=null)
                val retained=store.notificationEvent(entry.host,entry.event) ?: throw StorePolicy(ErrorCode.NotFound)
                if(!original())return@perform
                val target=Target(retained.event.sessionId,retained.event.generation)
                val current=inventory.sessions.firstOrNull {Target(it.sessionId,it.generation)==target&&it.connectivity==SessionConnectivity.Ready}
                if(current==null||connection.state.value.inventoryReady[entry.host]!=true){showStaleAttention(entry,::original);return@perform}
                val departing=selection?.let {connection.retireConversationSubscription(it.host,it.target)}
                connection.open(entry.host,target,::original)
                if(!original())return@perform
                attentionRouteJob=null
                navigate(Screen.Conversation,entry.host,departing);selection=ConversationKey(entry.host,target);runtime.setVisibleConversation(selection)
            }catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){if(original())showStaleAttention(entry,::original)}
        }
        attentionRouteJob=operation
    }
    private suspend fun showStaleAttention(entry:PendingAttention,admitted:()->Boolean) {
        val retained=try {store.notificationEvent(entry.host,entry.event)}catch(cancel:CancellationException){throw cancel}catch(_:Exception){null}
        if(!admitted())return
        attentionRouteJob=null
        if(retained==null){notice="This notification is no longer retained on this phone.";return}
        val target=Target(retained.event.sessionId,retained.event.generation)
        // The existing navigation cleanup retires only the actually selected original viewer.
        navigate(Screen.Conversation,entry.host);selection=ConversationKey(entry.host,target)
        notice="This notification refers to an older desktop session. Showing retained history only; reconnect or choose a current session before sending anything."
    }
    fun refresh() {
        val originalHost=host ?: return
        val originalSelection=selection
        perform(::connectionFailureNotice) {
            if(!foreground || host!=originalHost || selection!==originalSelection) return@perform
            closeTerminal();closeWorktrees();closeChanges();closeSkills()
            connection.snapshot(originalHost)
            // A read completion never selects or opens a different conversation.
            if(!foreground || host!=originalHost || selection!==originalSelection) return@perform
            originalSelection?.takeIf {it.host==originalHost}?.let {connection.open(originalHost,it.target)}
        }
    }
    fun readOnlyAgentTerminal(key:ConversationKey):Boolean {
        val view=connection.state.value
        val summary=view.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}?.state?.summary ?: return false
        val graph=view.inventory[key.host]?.hierarchy?.singleOrNull {it.serverId==summary.serverId}
        val pane=graph?.panes?.singleOrNull {it.target==key.target}
        return readOnlyAgentTerminal(view.hello[key.host]?.features,summary,pane,view.inventoryReady[key.host]==true && view.sources[key.host]?.containsKey(key.target)==true && graph?.freshness==NativeFreshness.Current)
    }
    fun openTerminal(key: ConversationKey) {
        val source=connection.state.value.sources[key.host]?.get(key.target) ?: return
        if(!foreground || selection!=key || !readOnlyAgentTerminal(key)) return
        closeShell();closeTerminal()
        val ticket=terminalTicket
        terminal=TerminalSnapshot(key,source,ticket,null,true,false)
        terminalJob=viewModelScope.launch {
            try {
                connection.terminal(key.host,key.target,source){grid,lease->
                    publishTerminalGrid(terminalState,key,source,ticket,foreground,selection,screen,
                        connection.state.value.sources[key.host]?.get(key.target)==source,lease,grid)
                }
            } catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){if(terminal?.ticket==ticket)terminal=TerminalSnapshot(key,source,ticket,null,false,true)}
        }
    }
    fun closeTerminal() { terminalTicket+=1;terminal=null;terminalJob?.cancel();terminalJob=null }
    fun openWorktrees(key: ConversationKey) {
        val source=connection.state.value.sources[key.host]?.get(key.target) ?: return
        val summary=connection.state.value.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}?.state?.summary ?: return
        if(!foreground || selection!=key || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.worktreesRead) return
        closeWorktrees()
        val ticket=worktreesTicket
        worktrees=WorktreesSnapshot(key,source,ticket,null,true,false)
        worktreesJob=viewModelScope.launch {
            try {
                val result=connection.worktrees(key.host,key.target,source)
                if(worktrees?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) worktrees=WorktreesSnapshot(key,source,ticket,result.worktrees,false,false)
            } catch(timeout: TimeoutCancellationException) {
                if(worktrees?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) worktrees=WorktreesSnapshot(key,source,ticket,null,false,true)
            } catch(cancel: CancellationException) {throw cancel}
            catch(_: Exception) {if(worktrees?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) worktrees=WorktreesSnapshot(key,source,ticket,null,false,true)}
        }
    }
    fun closeWorktrees() { closeAttachments();worktreesTicket+=1;worktrees=null;worktreesJob?.cancel();worktreesJob=null }
    fun refreshWorktrees() { worktrees?.key?.let(::openWorktrees) }
    private fun skillsAdmitted(snapshot:SkillsSnapshot):Boolean {
        val view=connection.state.value
        val summary=view.sessions[snapshot.key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==snapshot.key.target}==true}?.state?.summary
        return foreground && screen==Screen.Conversation && selection==snapshot.key && store.state.value.hosts.any {it.hostId==snapshot.key.host} && view.inventoryReady[snapshot.key.host]==true && view.sources[snapshot.key.host]?.get(snapshot.key.target)==snapshot.source && snapshot.key.host in view.hello && summary!=null && summary.provider in listOf(Provider.Codex,Provider.Omp) && summary.connectivity==SessionConnectivity.Ready && summary.capabilities.skillsRead && (summary.skillsGeneration==snapshot.generation||snapshot.loading&&summary.provider==Provider.Omp&&summary.skillsGeneration!=null)
    }
    fun openSkills(key:ConversationKey) {
        closeSkills()
        val view=connection.state.value
        val source=view.sources[key.host]?.get(key.target) ?: return
        val generation=view.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}?.state?.summary?.skillsGeneration ?: return
        val snapshot=SkillsSnapshot(key,source,generation,skillsTicket,null,true,false)
        if(!skillsAdmitted(snapshot)) return
        skills=snapshot
        skillsJob=viewModelScope.launch {
            try {
                val result=connection.skills(key.host,key.target,source,generation)
                val completed=snapshot.copy(generation=result.catalogueGeneration,catalogue=result,loading=false)
                if(skills?.ticket==snapshot.ticket && skillsAdmitted(completed)) {
                    val loaded=completed
                    skills=loaded;skillsCatalogue=loaded
                }
            } catch(timeout:TimeoutCancellationException) {if(skills?.ticket==snapshot.ticket && skillsAdmitted(snapshot)) skills=snapshot.copy(loading=false,error=true)}
            catch(cancel:CancellationException) {throw cancel}
            catch(_:Exception) {if(skills?.ticket==snapshot.ticket && skillsAdmitted(snapshot)) skills=snapshot.copy(loading=false,error=true)}
        }
    }
    fun closeSkills() {skillsInvocationLease?.invalidate();skillsInvocationLease=null;closeSkillsDocument();skillsTicket+=1;skills=null;skillsCatalogue=null;skillsJob?.cancel();skillsJob=null}
    internal fun invalidateSkillsInvocation(ticket:Long) {if(skills?.ticket==ticket)skillsInvocationLease?.invalidate()}
    /** Pure local projection. Typing never issues a native read or clears shared caches. */
    internal fun suggestedSkills(key:ConversationKey,draft:String):SkillsSuggestionsState? {
        if(skills!=null || composers[key].orEmpty()!=draft || !foreground || screen!=Screen.Conversation || selection!=key) return null
        val view=connection.state.value
        val source=view.sources[key.host]?.get(key.target) ?: return null
        val summary=view.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}?.state?.summary ?: return null
        val generation=summary.skillsGeneration ?: return null
        val trigger=skillSuggestionTrigger(summary.provider,draft) ?: return null
        if(summary.provider !in listOf(Provider.Codex,Provider.Omp) || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.skillsRead || view.inventoryReady[key.host]!=true || key.host !in view.hello || store.state.value.hosts.none {it.hostId==key.host}) return null
        val catalogue=skillsCatalogue?.takeIf {it.key==key && it.source==source && it.generation==generation && !it.loading && !it.error && it.catalogue?.provider==summary.provider && skillsAdmitted(it)}
        val prefix=trigger.removePrefix(when(summary.provider) {Provider.Codex->"$";Provider.Omp->"/skill:"})
        val entries=catalogue?.catalogue?.skills.orEmpty().asSequence().filter {it.name.startsWith(prefix,true)}.take(6).toList()
        return SkillsSuggestionsState(key,source,generation,summary.provider,skillsTicket,draft,trigger,catalogue,entries)
    }
    private fun suggestionAdmitted(value:SkillsSuggestionsState):Boolean {
        val current=suggestedSkills(value.key,value.draft) ?: return false
        return current.source==value.source && current.generation==value.generation && current.provider==value.provider && current.ticket==value.ticket && current.trigger==value.trigger && current.catalogue===value.catalogue
    }
    internal fun openSuggestedSkills(value:SkillsSuggestionsState) {
        if(suggestionAdmitted(value)) openSkills(value.key)
    }
    internal fun appendSuggestedSkill(value:SkillsSuggestionsState,entry:SkillEntry):Boolean {
        val catalogue=value.catalogue ?: return false
        if(!composerEditable(catalogue.key))return false
        if(!suggestionAdmitted(value) || skillsCatalogue!==catalogue || !skillsAdmitted(catalogue) || entry !in value.entries || catalogue.catalogue?.skills?.contains(entry)!=true) return false
        val mention=entry.exactInsertText() ?: return false
        // No cursor is inferred: append only, leaving the visible trigger exactly as typed.
        val appended="${value.draft}\n$mention"
        try {PromptText(appended)} catch(_:Exception) {notice="This skill reference does not fit in the current draft.";return false}
        if(!suggestionAdmitted(value) || !skillsAdmitted(catalogue)) return false
        composer(appended);skillsFocusSignal+=1
        return true
    }
    /** Closing the picker retains only a successfully loaded, still-admitted in-memory catalogue. */
    fun dismissSkills() {
        val retained=skillsCatalogue?.takeIf {skillsAdmitted(it)}
        closeSkills()
        skillsCatalogue=retained
    }
    fun refreshSkills() {skills?.key?.let(::openSkills)}
    fun closeSkillsDocument(){skillsDocumentJob?.cancel();skillsDocumentJob=null;skillsDocument=null}
    fun previewSkill(snapshot:SkillsSnapshot,entry:SkillEntry){
        val catalogue=snapshot.catalogue ?: return
        if(skills!==snapshot||snapshot.loading||snapshot.error||!skillsAdmitted(snapshot)||entry !in catalogue.skills)return
        val document=entry.documentRef ?: return;val foreground=foregroundGeneration ?: return;closeSkillsDocument()
        val shown=SkillsDocumentState(snapshot,entry,Any());skillsDocument=shown
        skillsDocumentJob=viewModelScope.launch{
            try{val text=withTimeout(90000){connection.skillsDocument(snapshot.key.host,snapshot.key.target,snapshot.source,snapshot.generation,document)}
                if(skillsDocument?.ticket===shown.ticket&&skills===snapshot&&skillsAdmitted(snapshot)&&foregroundGeneration===foreground)skillsDocument=shown.copy(loading=false,text=text)
            }catch(_:TimeoutCancellationException){if(skillsDocument?.ticket===shown.ticket&&skills===snapshot&&skillsAdmitted(snapshot)&&foregroundGeneration===foreground)skillsDocument=shown.copy(loading=false,failed=true)}
            catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(skillsDocument?.ticket===shown.ticket&&skills===snapshot&&skillsAdmitted(snapshot)&&foregroundGeneration===foreground)skillsDocument=shown.copy(loading=false,failed=true)}
        }
    }

    internal fun canInvokeSkill(snapshot:SkillsSnapshot,entry:SkillEntry):Boolean {
        val catalogue=snapshot.catalogue ?: return false
        val summary=connection.state.value.sessions[snapshot.key.host]?.firstOrNull {it.state?.summary?.let{s->Target(s.sessionId,s.generation)==snapshot.key.target}==true}?.state?.summary
        return skills===snapshot && skillsInvocationLease?.active?.get()!=false && !snapshot.loading && !snapshot.error && skillsAdmitted(snapshot) && catalogue.provider==Provider.Codex && summary?.capabilities?.prompt==true && entry.enabled && entry.invocationRef!=null && catalogue.skills.any {it===entry}
    }
    internal fun invokeSkill(snapshot:SkillsSnapshot,entry:SkillEntry) {
        if(!canInvokeSkill(snapshot,entry))return
        val catalogue=snapshot.catalogue ?: return
        val reference=entry.invocationRef ?: return
        val foreground=foregroundGeneration
        val lease=skillsInvocationLease ?: SkillInvocationLease().also {skillsInvocationLease=it}
        perform {
            val context=connection.skillInvocation(snapshot.key.host,catalogue,entry,lease){foregroundGeneration===foreground&&canInvokeSkill(snapshot,entry)}
            val boot=connection.state.value.hello[snapshot.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
            val action=Uuid.random();focusedId=action
            connection.mutate(snapshot.key.host,Request(action,RequestBody.PromptSubmit(PromptSubmitParams(boot,snapshot.key.target,PromptText(""),SkillSelector(catalogue.catalogueGeneration,reference)))),ordinaryComposer=false,skill=context)
        }
    }

    fun insertSkill(snapshot:SkillsSnapshot,entry:SkillEntry):Boolean {
        if(!composerEditable(snapshot.key))return false
        val current=skills ?: return false
        if(current !== snapshot || current.ticket!=snapshot.ticket || current.loading || current.error || current.catalogue!=snapshot.catalogue || !skillsAdmitted(current) || current.catalogue?.skills?.contains(entry)!=true) return false
        val mention=entry.exactInsertText() ?: return false
        val draft=composers[snapshot.key].orEmpty()
        val appended=if(draft.isEmpty()) mention else "$draft\n$mention"
        try {PromptText(appended)} catch(_:Exception) {notice="This skill mention does not fit in the current draft.";return false}
        if(!skillsAdmitted(current)) return false
        composer(appended);skillsFocusSignal+=1;dismissSkills();return true
    }
    fun openChanges(key: ConversationKey) {
        val source=connection.state.value.sources[key.host]?.get(key.target) ?: return
        val summary=connection.state.value.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}?.state?.summary ?: return
        if(!foreground || selection!=key || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.changesRead) return
        closeChanges()
        val ticket=changesTicket
        changes=ChangesSnapshot(key,source,ticket,null,true,false)
        changesJob=viewModelScope.launch {
            try {
                val result=connection.changes(key.host,key.target,source)
                if(changes?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) changes=ChangesSnapshot(key,source,ticket,result,false,false)
            } catch(timeout: TimeoutCancellationException) {
                if(changes?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) changes=ChangesSnapshot(key,source,ticket,null,false,true)
            } catch(cancel: CancellationException) { throw cancel }
            catch(_: Exception) { if(changes?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) changes=ChangesSnapshot(key,source,ticket,null,false,true) }
        }
    }
    fun closeChanges() { changesTicket+=1;changes=null;changesJob?.cancel();changesJob=null }
    fun refreshChanges() { changes?.key?.let(::openChanges) }
    /** Append one currently admitted Changes projection as plain draft text; this never uploads or sends anything. */
    internal fun insertChangesReference(snapshot: ChangesSnapshot, selection: ChangesReferenceSelection): Boolean {
        if(!composerEditable(snapshot.key))return false
        val reference = changesReferenceAdmitted(snapshot, selection)
        if (reference == null) {
            notice = "This Changes snapshot cannot provide that reference. Refresh it before adding it to the draft."
            return false
        }
        val current = composers[snapshot.key].orEmpty()
        val appended = if (current.isEmpty()) reference.draftText else "$current\n${reference.draftText}"
        try { PromptText(appended) }
        catch (_: Exception) {
            notice = "This Changes reference does not fit in the current message. Edit the draft before adding it."
            return false
        }
        if (changesReferenceAdmitted(snapshot, selection) == null) {
            notice = "This Changes snapshot changed before the reference could be added."
            return false
        }
        // This stays on the current main-thread action path: no suspension occurs between admission and append.
        composer(appended)
        closeChanges()
        notice = "Changes reference added to this conversation draft. It was not sent."
        return true
    }
    private fun changesReferenceAdmitted(snapshot: ChangesSnapshot, selection: ChangesReferenceSelection): ChangesReferencePreview? {
        val current = changes ?: return null
        val result = snapshot.changes ?: return null
        if (current !== snapshot || current.ticket != snapshot.ticket || current.loading || current.error || current.changes !== result) return null
        val view = connection.state.value
        val summary = view.sessions[snapshot.key.host]?.firstOrNull { entry ->
            entry.state?.summary?.let { Target(it.sessionId, it.generation) == snapshot.key.target } == true
        }?.state?.summary ?: return null
        if (!(foreground && screen == Screen.Conversation && this.selection == snapshot.key &&
            store.state.value.hosts.any { it.hostId == snapshot.key.host } &&
            view.hello.containsKey(snapshot.key.host) &&
            view.sources[snapshot.key.host]?.get(snapshot.key.target) == snapshot.source &&
            summary.connectivity == SessionConnectivity.Ready && summary.capabilities.changesRead &&
            result.target == snapshot.key.target)) return null
        return resolveChangesReference(
            selection,
            result.changes.stagedPatch,
            result.changes.unstagedPatch,
            result.changes.entries.mapTo(mutableSetOf()) { it.path },
        )
    }
    internal fun beginArtifacts(host:String,source:ArtifactSource){
        if(!foreground||artifactExportJob?.isCompleted==false)return
        val generation=foregroundGeneration ?: return;val view=connection.state.value;val inventory=view.inventory[host] ?: return
        val conversation=(source as? ArtifactSource.Session)?.let{ConversationKey(host,it.target)}
        if(conversation!=null&&(screen!=Screen.Conversation||selection!=conversation))return
        if(source is ArtifactSource.Shell&&(screen!=Screen.Terminals||this.host!=host))return
        val reducer=conversation?.let{key->view.sessions[host]?.singleOrNull{it.state?.summary?.let{summary->Target(summary.sessionId,summary.generation)==key.target}==true}}
        val epoch=conversation?.let{view.sources[host]?.get(it.target)}
        closeArtifacts()
        try{val context=connection.captureArtifacts(host,source,reducer,inventory.subscription,epoch);val shown=ArtifactBrowserState(host,source,context,screen,conversation,generation);artifacts=shown;refreshArtifacts(shown)}catch(_:Exception){notice="Files are unavailable for this original source or local read grant."}
    }
    private fun artifactOriginCurrent(shown:ArtifactBrowserState):Boolean=artifacts===shown&&screen==shown.origin&&host==shown.host&&(shown.conversation==null||selection==shown.conversation)&&store.state.value.hosts.any{it.hostId==shown.host}
    private fun artifactAdmitted(shown:ArtifactBrowserState):Boolean=artifactOriginCurrent(shown)&&foreground&&foregroundGeneration===shown.foreground&&connection.artifactCurrent(shown.context)
    private fun stopArtifactPicker(){val hold=artifactPickerHold ?: return;artifactPickerHold=null;runtime.cleanupScope.launch{runtime.release(hold.claim)}}
    internal fun closeArtifacts(){stopArtifactPicker();artifacts?.let{it.context.invalidate();it.preview=null;it.selected=null;it.exportConfirmation=null;it.ticket=Any();it.job?.cancel()};artifacts=null;artifactExportJob?.cancel()}
    private fun pauseArtifacts(){artifacts?.let{shown->shown.preview=null;shown.selected=null;shown.exportConfirmation=null;shown.job?.cancel();if(artifactPickerHold==null)closeArtifacts()}}
    private fun artifactPage(shown:ArtifactBrowserState,parent:Uuid?,cursor:Uuid?,breadcrumbs:List<Pair<Uuid,String>>,append:Boolean){
        if(!artifactAdmitted(shown)||shown.loading||shown.exporting)return
        val ticket=Any();shown.ticket=ticket;shown.loading=true;shown.error=null;shown.preview=null;shown.selected=null
        shown.job=viewModelScope.launch{
            try{val page=connection.artifactList(shown.context,parent,cursor);if(!artifactAdmitted(shown)||shown.ticket!==ticket)return@launch
                if(!shown.publishPage(ticket,page,cursor,breadcrumbs,append))return@launch
            }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(artifacts===shown)shown.error="Directory changed, expired, exceeded its bound or is unavailable. Refresh from this original source."}finally{if(artifacts===shown&&shown.ticket===ticket)shown.loading=false}
        }
    }
    internal fun refreshArtifacts(shown:ArtifactBrowserState)=artifactPage(shown,shown.breadcrumbs.lastOrNull()?.first,null,shown.breadcrumbs,false)
    internal fun moreArtifacts(shown:ArtifactBrowserState){val cursor=shown.cursor ?: return;artifactPage(shown,null,cursor,shown.breadcrumbs,true)}
    internal fun openArtifactDirectory(shown:ArtifactBrowserState,entry:ArtifactEntry){if(entry.kind!=ArtifactKind.Directory||entry !in shown.entries||shown.breadcrumbs.size>=16)return;artifactPage(shown,entry.reference,null,shown.breadcrumbs+(entry.reference to entry.name),false)}
    internal fun artifactUp(shown:ArtifactBrowserState){val path=shown.breadcrumbs.dropLast(1);artifactPage(shown,path.lastOrNull()?.first,null,path,false)}
    private suspend fun consumeArtifact(shown:ArtifactBrowserState,opened:ArtifactsOpenResult,consume:suspend(ByteArray)->Unit){
        val digest=java.security.MessageDigest.getInstance("SHA-256");var offset=0L
        while(true){currentCoroutineContext().ensureActive();if(!artifactAdmitted(shown))throw StorePolicy(ErrorCode.StaleSession)
            val chunk=connection.artifactRead(shown.context,opened.readId,offset);val bytes=android.util.Base64.decode(chunk.data.value,android.util.Base64.NO_WRAP)
            if(bytes.size>65536||offset+bytes.size>opened.size.value||bytes.isEmpty()&&!chunk.eof)throw StorePolicy(ErrorCode.InvalidRequest)
            currentCoroutineContext().ensureActive();if(!artifactAdmitted(shown))throw StorePolicy(ErrorCode.StaleSession)
            consume(bytes);digest.update(bytes);offset+=bytes.size;shown.progress=offset
            if(chunk.eof){if(offset!=opened.size.value)throw StorePolicy(ErrorCode.InvalidRequest);break}
        }
        val hash=digest.digest().joinToString(""){"%02x".format(it)};if(hash!=opened.sha256||!artifactAdmitted(shown))throw StorePolicy(ErrorCode.StaleSession)
    }
    private suspend fun releaseArtifact(shown:ArtifactBrowserState,read:Uuid?){if(read==null)return;withContext(NonCancellable){try{withTimeout(5000){connection.artifactClose(shown.context,read)}}catch(_:Exception){}}}
    internal fun previewArtifact(shown:ArtifactBrowserState,entry:ArtifactEntry){
        if(!artifactAdmitted(shown)||shown.loading||shown.exporting||entry !in shown.entries||(entry.size?.value ?: Long.MAX_VALUE)>1024L*1024)return
        shown.loading=true;shown.error=null;shown.preview=null;shown.selected=entry
        shown.job=viewModelScope.launch{var read:Uuid?=null;try{
            val opened=connection.artifactOpen(shown.context,entry.reference){read=it}
            if(opened.size.value>1024L*1024||opened.size!=entry.size||opened.name!=entry.name)throw StorePolicy(ErrorCode.InvalidRequest)
            when(opened.previewKind) {
                ArtifactPreviewKind.Binary->{if(artifactAdmitted(shown))shown.error="This is binary content, not a text preview. Explicit export remains available.";return@launch}
                ArtifactPreviewKind.Unavailable->{if(artifactAdmitted(shown))shown.error="Text preview is unavailable for this complete snapshot. Explicit export remains available.";return@launch}
                ArtifactPreviewKind.Text->Unit
            }
            val buffer=ByteArray(opened.size.value.toInt());var position=0
            try {
                consumeArtifact(shown,opened){bytes->bytes.copyInto(buffer,position);position+=bytes.size}
                val decoded=Charsets.UTF_8.newDecoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT).onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT).decode(java.nio.ByteBuffer.wrap(buffer)).toString()
                if(decoded.any{Character.isISOControl(it)&&it!='\t'&&it!='\r'&&it!='\n'})throw StorePolicy(ErrorCode.InvalidRequest)
                if(artifactAdmitted(shown)&&shown.selected===entry)shown.preview=decoded
            }finally{buffer.fill(0)}
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(artifacts===shown)shown.error="A complete verified text preview is unavailable. No partial text was displayed."}finally{releaseArtifact(shown,read);if(artifacts===shown)shown.loading=false}}
    }
    internal fun artifactExportName(name:String):String{val safe=StringBuilder();name.codePoints().forEachOrdered{code->if(code!=47&&code!=92&&!Character.isISOControl(code)){val text=String(Character.toChars(code));if(utf8Size(safe.toString())+utf8Size(text)<=128)safe.append(text)}};return safe.toString().takeIf{it.isNotBlank()&&it!="."&&it!=".."} ?: "artifact.bin"}
    internal suspend fun startArtifactExportPicker(shown:ArtifactBrowserState,entry:ArtifactEntry):ArtifactPickerTicket?{
        if(!artifactAdmitted(shown)||shown.loading||shown.exporting||artifactPickerHold!=null||attachmentPickerHold!=null||artifactExportJob?.isCompleted==false||entry !in shown.entries)return null
        val ticket=ArtifactPickerTicket(shown,entry,Any());val hold=ArtifactPickerHold(ticket,runtime.newClaim(RuntimeClaimKind.Ui),android.os.SystemClock.elapsedRealtime()+120000);artifactPickerHold=hold
        try{runtime.acquire(hold.claim);if(artifactPickerHold!==hold||!artifactAdmitted(shown))throw StorePolicy(ErrorCode.NotReady)}catch(cancel:CancellationException){stopArtifactPicker();throw cancel}catch(_:Exception){stopArtifactPicker();return null}
        viewModelScope.launch{delay((hold.expires-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0));if(artifactPickerHold===hold){stopArtifactPicker();closeArtifacts();notice="The export picker expired. No host file was opened."}};return ticket
    }
    internal fun cancelArtifactExportPicker(ticket:ArtifactPickerTicket){if(artifactPickerHold?.ticket===ticket){stopArtifactPicker();if(artifacts===ticket.browser)ticket.browser.error="Export picker canceled. No host file was opened."}}
    internal fun returnArtifactExportPicker(ticket:ArtifactPickerTicket,uri:android.net.Uri?){
        val hold=artifactPickerHold ?: run{if(uri!=null)notice="The original export picker is no longer active. No bytes were read or written; the provider may retain an empty document.";return}
        if(hold.ticket!==ticket||hold.result!=null)return
        if(uri==null){stopArtifactPicker();return}
        hold.result=uri;hold.returnedForeground=foregroundGeneration
        continueArtifactExportPicker(hold)
    }
    private fun continueArtifactExportPicker(hold:ArtifactPickerHold){
        if(artifactPickerHold!==hold)return
        val uri=hold.result ?: return;val shown=hold.ticket.browser;val generation=hold.returnedForeground
        val identity=foreground&&generation!=null&&foregroundGeneration===generation&&android.os.SystemClock.elapsedRealtime()<hold.expires&&artifactOriginCurrent(shown)&&connection.artifactIdentityCurrent(shown.context)
        if(!identity){stopArtifactPicker();closeArtifacts();notice="The original export source was lost or expired. No bytes were read or written; the selected provider may retain an empty document.";return}
        // Retain only this exact picker result until the original deadline.
        // Pending inventory never authorizes host reads or destination writes.
        if(!connection.artifactCurrent(shown.context)){shown.error="Export paused while the original desktop inventory refreshes.";return}
        stopArtifactPicker();shown.foreground=generation!!;exportArtifact(shown,hold.ticket.entry,uri)
    }
    private fun exportArtifact(shown:ArtifactBrowserState,entry:ArtifactEntry,uri:android.net.Uri){
        if(!artifactAdmitted(shown)||artifactExportJob?.isCompleted==false)return
        shown.exporting=true;shown.progress=0;shown.error=null
        val job=viewModelScope.launch{var read:Uuid?=null;var complete=false;var destinationOwned=false
            try{
                if(uri.scheme!="content"||uri.authority.isNullOrEmpty())throw StorePolicy(ErrorCode.InvalidRequest)
                destinationOwned=true
                // This is reached only after the retained original picker result.
                val opened=connection.artifactOpen(shown.context,entry.reference){read=it}
                if(opened.name!=entry.name||opened.size!=entry.size)throw StorePolicy(ErrorCode.StaleSession)
                withContext(Dispatchers.IO){
                    currentCoroutineContext().ensureActive();if(!artifactAdmitted(shown))throw StorePolicy(ErrorCode.StaleSession)
                    val descriptor=getApplication<Application>().contentResolver.openFileDescriptor(uri,"rwt") ?: throw StorePolicy(ErrorCode.NotReady)
                    android.os.ParcelFileDescriptor.AutoCloseOutputStream(descriptor).use{output->
                        consumeArtifact(shown,opened){bytes->currentCoroutineContext().ensureActive();if(!artifactAdmitted(shown))throw StorePolicy(ErrorCode.StaleSession);output.write(bytes);currentCoroutineContext().ensureActive()}
                        output.flush();try{android.system.Os.fsync(descriptor.fileDescriptor)}catch(error:android.system.ErrnoException){if(error.errno!=android.system.OsConstants.EINVAL&&error.errno!=android.system.OsConstants.ESPIPE)throw error}
                    }
                }
                if(!artifactAdmitted(shown))throw StorePolicy(ErrorCode.StaleSession);complete=true;shown.error="Export complete. Exact size and SHA-256 verified."
            }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(artifacts===shown)shown.error="Export did not complete. No automatic retry was started."}
            finally{
                releaseArtifact(shown,read)
                if(!complete&&destinationOwned)withContext(NonCancellable+Dispatchers.IO){val removed=try{android.provider.DocumentsContract.deleteDocument(getApplication<Application>().contentResolver,uri)}catch(_:Exception){false};if(!removed)withContext(Dispatchers.Main.immediate){notice="Export stopped. The selected document provider may retain a partial file; remove it there."}}
                if(artifacts===shown)shown.exporting=false
            }
        };artifactExportJob=job;shown.job=job
    }
    private val terminalCreationActions=mutableMapOf<String,Uuid>()
    internal fun closeTerminalCreation(){
        terminalCreation?.let {it.context.lease.invalidate();it.directories?.let(::closeCreationDirectories)}
        terminalCreation=null
    }
    private fun terminalCreationAdmitted(shown:TerminalCreationState):Boolean {
        if(terminalCreation!==shown||!foreground||foregroundGeneration!==shown.foreground||screen!=shown.originScreen||host!=shown.host)return false
        val selected=when(val source=shown.source) {
            is ArtifactSource.Session->screen==Screen.Conversation&&selection==ConversationKey(shown.host,source.target)
            is ArtifactSource.Shell->screen==Screen.Terminals&&shell?.selection==ShellSelection(shown.host,source.target)
            null->true
        }
        return selected&&connection.terminalCreationCurrent(shown.context)
    }
    internal fun beginAgentTerminal(key:ConversationKey) {
        val summary=connection.state.value.sessions[key.host]?.firstOrNull {it.state?.summary?.let {row->Target(row.sessionId,row.generation)==key.target}==true}?.state?.summary ?: return
        if(selection!=key||summary.capabilities.terminalCreate!=true)return
        beginTerminalCreation(key.host,summary.serverId,summary.workspaceId,ArtifactSource.Session(key.target))
    }
    internal fun beginTerminalCreation(host:String,server:Uuid,workspace:Id,source:ArtifactSource?=null) {
        closeTerminalCreation();val generation=foregroundGeneration ?: return
        if(!foreground||this.host!=host||screen !in listOf(Screen.Terminals,Screen.Conversation))return
        try {
            val context=connection.captureTerminalCreation(host,server,workspace,source)
            val shown=TerminalCreationState(host,server,workspace,source,generation,screen,context)
            terminalCreation=shown
            shown.directories=CreationDirectoryBrowser(connection,viewModelScope,host,null,context){terminalCreationAdmitted(shown)}.also {creationDirectoriesBrowser=it}
            loadTerminalCreation(shown)
        }catch(_:Exception){notice="A current original native workspace is required. Configure an approved project and the local terminal-control grant on this desktop."}
    }
    internal fun selectTerminalProject(shown:TerminalCreationState,project:Uuid) {
        if(!terminalCreationAdmitted(shown)||shown.pending||busy)return
        if(connection.state.value.inventory[shown.host]?.projects?.none {it.projectId==project&&it.available}!=false)return
        shown.project=project;shown.directories?.configure(shown.server,project)
    }
    private suspend fun retainedTerminals(host:String):Pair<LocalAction?,Boolean> {
        var after:Uuid?=null;var latest:LocalAction?=null;var pending=false
        do {val page=store.localActions(host,after,100)
            for(action in page.filter {it.metadata.method in listOf(ActionMethod.TerminalCreate,ActionMethod.ShellCreate)}) {
                if(latest==null||action.metadata.createdAt.value>latest.metadata.createdAt.value
                    ||action.metadata.createdAt==latest.metadata.createdAt&&action.metadata.actionId.value>latest.metadata.actionId.value)latest=action
                if(action.receipt==null||action.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))pending=true
            };after=if(page.size==100)page.last().metadata.actionId else null
        }while(after!=null)
        return latest to pending
    }
    private fun loadTerminalCreation(shown:TerminalCreationState) {
        val loading=Any();shown.loading=loading;shown.loaded=false
        viewModelScope.launch {try {
            val (prior,pending)=retainedTerminals(shown.host)
            if(!terminalCreationAdmitted(shown)||shown.loading!==loading)return@launch
            shown.prior=prior;shown.pending=pending;shown.loaded=true
            if(prior?.metadata?.actionId!=shown.separate)shown.separate=null
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(terminalCreation===shown){shown.pending=true;notice="Retained terminal actions could not be inspected."}}}
    }
    internal fun inspectTerminalCreation(shown:TerminalCreationState) {
        if(!terminalCreationAdmitted(shown))return
        viewModelScope.launch {try {
            shown.prior?.metadata?.actionId?.let {if(connection.status(shown.host,it) is ActionStatusResult.Unknown)notice="Original creation is unknown. Never replay it or adopt guessed resources."}
            if(terminalCreationAdmitted(shown))loadTerminalCreation(shown)
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){notice="Terminal receipt is unavailable; no creation was retried."}}
    }
    internal fun permitSeparateTerminal(shown:TerminalCreationState,action:Uuid) {
        if(!terminalCreationAdmitted(shown)||shown.pending)return
        val prior=shown.prior ?: return
        if(prior.metadata.actionId==action&&prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))shown.separate=action
    }
    internal fun terminalCreationAllowed(shown:TerminalCreationState):Boolean {
        if(!terminalCreationAdmitted(shown)||!shown.loaded||shown.pending||shown.project==null)return false
        if(connection.state.value.mutationReady[shown.host]!=true)return false
        if(shown.directories?.choice(shown.server,shown.project)==null)return false
        return (shown.prior==null||shown.separate==shown.prior?.metadata?.actionId)
            &&(terminalCreationActions[shown.host]==null||terminalCreationActions[shown.host]==shown.prior?.metadata?.actionId)
    }
    internal fun terminalCreatedAvailable(shown:TerminalCreationState):Boolean {
        if(!terminalCreationAdmitted(shown))return false
        val receipt=shown.prior?.takeIf {it.authoritative}?.receipt?.takeIf {it.state==ReceiptState.Succeeded} ?: return false
        val resources=(receipt.result as? ActionResultShellCreation)?.resources ?: return false
        val target=resources.createdTarget ?: return false
        val inventory=connection.state.value.inventory[shown.host] ?: return false
        return connection.state.value.inventoryReady[shown.host]==true&&inventory.shells.any {it.target==target&&it.connectivity==SessionConnectivity.Ready&&it.capabilities.read
            &&it.serverId==resources.serverId&&it.workspaceId==resources.workspaceId&&it.tabId==resources.tabId&&it.paneId==resources.paneId&&it.terminalId==resources.terminalId}
    }
    internal fun openCreatedTerminal(shown:TerminalCreationState) {
        if(!terminalCreatedAvailable(shown))return
        val target=(shown.prior?.receipt?.result as? ActionResultShellCreation)?.resources?.createdTarget ?: return
        val host=shown.host;closeTerminalCreation();terminals(host);openShell(host,target)
    }
    internal fun createTerminal(shown:TerminalCreationState)=perform {
        if(!terminalCreationAllowed(shown))throw StorePolicy(ErrorCode.NotReady)
        val project=shown.project ?: throw StorePolicy(ErrorCode.NotReady)
        val directory=shown.directories?.choice(shown.server,project) ?: throw StorePolicy(ErrorCode.NotReady)
        val (prior,pending)=retainedTerminals(shown.host)
        if(!terminalCreationAllowed(shown)||pending||prior!=shown.prior||shown.project!=project
            ||shown.directories?.choice(shown.server,project)!=directory)throw StorePolicy(ErrorCode.NotReady)
        val boot=connection.state.value.hello[shown.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();val previous=terminalCreationActions.put(shown.host,action);val permit=shown.separate
        shown.separate=null;shown.pending=true;focusedId=action
        try {connection.mutate(shown.host,Request(action,RequestBody.TerminalCreate(TerminalCreateParams(boot,shown.server,shown.workspace,project,directory.reference,shown.source))),
            terminal=shown.context,directories=directory.context)}
        catch(error:Exception){if(withContext(NonCancellable){store.localAction(shown.host,action)}==null){
            if(previous==null)terminalCreationActions.remove(shown.host)else terminalCreationActions[shown.host]=previous;shown.separate=permit};throw error}
        finally {if(terminalCreationAdmitted(shown))loadTerminalCreation(shown)}
    }
    internal var agentAliasScope:AgentAliasScope?=null
        private set
    internal fun closeAgentAlias(){agentAliasScope?.lease?.invalidate();agentAliasScope=null}
    internal fun beginAgentAlias(key:ConversationKey,source:Uuid):AgentAliasScope? {
        closeAgentAlias()
        val summary=connection.state.value.sessions[key.host]?.firstOrNull {it.state?.summary?.sessionId==key.target.sessionId}?.state?.summary
        if(!foreground || selection!=key || connection.state.value.sources[key.host]?.get(key.target)!=source || summary?.generation!=key.target.generation || summary.capabilities.agentRename!=true)return null
        return AgentAliasScope(key,source,AgentAliasLease()).also {agentAliasScope=it}
    }
    internal fun renameAgent(scope:AgentAliasScope,name:String)=perform {
        val alias=AgentName(name)
        val context=connection.captureAgentAlias(scope.key.host,scope.key.target,scope.source,scope.lease,alias)
        if(agentAliasScope!==scope || !foreground || selection!=scope.key || connection.state.value.sources[scope.key.host]?.get(scope.key.target)!=scope.source)throw StorePolicy(ErrorCode.StaleSession)
        val boot=connection.state.value.hello[scope.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();focusedId=action
        connection.mutate(scope.key.host,Request(action,RequestBody.AgentRename(AgentRenameParams(boot,scope.key.target,alias))),alias=context)
        if(agentAliasScope===scope)closeAgentAlias()
    }
    fun renameWorkspace(key: ConversationKey, label: String) = perform {
        val boot=connection.state.value.hello[key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        if(!foreground || selection!=key) throw StorePolicy(ErrorCode.StaleSession)
        val action=Uuid.random()
        focusedId=action
        connection.mutate(key.host,Request(action,RequestBody.WorkspaceRename(WorkspaceRenameParams(boot,key.target,Label(label)))))
    }
    fun beginPairing() { navigate(Screen.Pairing,null); gate = PairingGate() }
    fun acceptInvitation(raw: String): Boolean {
        if (screen != Screen.Pairing || !foreground || preview != null) return false
        val sanitized = gate?.submit(raw)
        if (sanitized == null) { notice = "Invalid or oversized invitation. Open a fresh pairing window on the desktop and retry."; return false }
        preview = sanitized; notice = null; return true
    }
    fun confirmPairing() {
        val candidate = gate ?: return
        if (!foreground || preview == null || busy) return
        busy = true
        pairingJob = viewModelScope.launch {
            try { val hello = candidate.confirm(connection); pairingJob=null; closePairing(); host = hello.hostId.value; screen = Screen.Sessions }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { notice = safeFailure(error); pairingJob=null; closePairing(); gate = PairingGate() }
            finally { busy = false; refreshActions() }
        }
    }
    fun retryPairing() { closePairing(); gate = PairingGate() }
    private fun closePairing() { gate?.close(); gate = null; preview = null; pairingJob?.cancel(); pairingJob = null }
    internal suspend fun startAttachmentPicker(key:ConversationKey):AttachmentSourceTicket? {
        if(attachmentPickerHold!=null||artifactPickerHold!=null)return null
        val ticket=captureAttachmentTicket(key) ?: return null
        val original=ticket.originalScope as AttachmentConsentScope
        val hold=PickerHold(ticket,original,runtime.newClaim(RuntimeClaimKind.Ui),android.os.SystemClock.elapsedRealtime()+120000)
        attachmentPickerHold=hold
        try {runtime.acquire(hold.claim);if(attachmentPickerHold!==hold||!attachmentAdmitted(ticket))throw StorePolicy(ErrorCode.NotReady)}
        catch(cancel:CancellationException){finishAttachmentPicker(ticket);throw cancel}
        catch(_:Exception){finishAttachmentPicker(ticket);return null}
        viewModelScope.launch {delay((hold.expires-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0));if(attachmentPickerHold===hold)finishAttachmentPicker(ticket)}
        return ticket
    }
    internal fun finishAttachmentPicker(ticket:AttachmentSourceTicket?){val hold=attachmentPickerHold ?: return;if(ticket!=null&&hold.ticket!==ticket)return
        attachmentPickerHold=null;hold.original.context.lease.invalidate()
        runtime.cleanupScope.launch {runtime.release(hold.claim)}
    }
    internal fun returnAttachmentPicker(ticket:AttachmentSourceTicket):AttachmentSourceTicket? {
        val hold=attachmentPickerHold ?: return null
        if(hold.ticket!==ticket)return null
        val generation=foregroundGeneration
        val valid=foreground&&generation!=null&&android.os.SystemClock.elapsedRealtime()<hold.expires&&selection==hold.original.key&&screen==Screen.Conversation&&connection.attachmentCurrent(hold.original.context)
        if(!valid){finishAttachmentPicker(ticket);notice="The original attachment source expired or disconnected. Choose again from its current conversation.";return null}
        attachmentPickerHold=null;runtime.cleanupScope.launch {runtime.release(hold.claim)}
        val original=AttachmentConsentScope(hold.original.key,hold.original.source,generation!!,hold.original.context,hold.original.draft,hold.original.revision,hold.original.cursor)
        attachmentPickerConsent=original
        return AttachmentSourceTicket(originalScope=original)
    }
    internal fun captureAttachmentTicket(key:ConversationKey,resume:AttachmentsStatusResult?=null):AttachmentSourceTicket? {
        val generation=foregroundGeneration ?: return null
        val view=connection.state.value;val source=view.sources[key.host]?.get(key.target) ?: return null
        val reducer=view.sessions[key.host]?.firstOrNull {it.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==key.target}==true} ?: return null
        if(!foreground||screen!=Screen.Conversation||selection!=key||view.inventoryReady[key.host]!=true||view.mutationReady[key.host]!=true||reducer.state?.summary?.capabilities?.attachments!=true)return null
        val cursor=composerCursors[key] ?: return null
        return try {val context=connection.captureAttachment(key.host,key.target,source,reducer,resume)
            attachmentPickerConsent?.takeIf {it!==attachmentTransfer?.original}?.context?.lease?.invalidate()
            val original=AttachmentConsentScope(key,source,generation,context,composers[key].orEmpty(),composerRevisions[key] ?: 0L,cursor)
            attachmentPickerConsent=original;AttachmentSourceTicket(originalScope=original)
        }catch(_:Exception){null}
    }
    internal fun attachmentAdmitted(ticket:AttachmentSourceTicket):Boolean {
        val original=ticket.originalScope as? AttachmentConsentScope ?: return false
        val view=connection.state.value
        return (attachmentPickerConsent===original || attachmentTransfer?.original===original)&&foreground&&foregroundGeneration===original.generation&&screen==Screen.Conversation&&selection==original.key&&store.state.value.hosts.any {it.hostId==original.key.host}&&view.sources[original.key.host]?.get(original.key.target)==original.source&&view.inventoryReady[original.key.host]==true&&view.mutationReady[original.key.host]==true&&connection.attachmentCurrent(original.context)
    }
    internal fun beginAttachmentUpload(ticket:AttachmentSourceTicket,prepared:PreparedAttachment):Boolean {
        val original=ticket.originalScope as? AttachmentConsentScope ?: return false
        if(!attachmentAdmitted(ticket)||attachmentTransfer?.working==true)return false
        val record=LocalAttachmentUpload(original.key.host,original.key.target,original.source,Uuid.random(),LocalAttachmentFile.from(prepared),prepared.size,prepared.sha256)
        val transfer=AttachmentTransfer(ticket,original,prepared,record);transfer.reviewedDraft=original.revision to original.cursor
        attachmentTransfer?.original?.context?.lease?.invalidate()
        attachmentTransfer=transfer;attachmentVisible=true
        attachmentJob=viewModelScope.launch {var retained=false
            try {store.saveAttachment(record);retained=true;runAttachment(transfer,true)}
            catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){transfer.paused=true;transfer.notice="Upload stopped. Inspect its receipt and desktop status; no automatic retry.";if(!retained)withContext(kotlinx.coroutines.NonCancellable){attachmentStaging.discard(prepared)}}
        };return true
    }
    private suspend fun attachmentStep(transfer:AttachmentTransfer,body:(Uuid)->RequestBody):Boolean {
        if(!attachmentAdmitted(transfer.ticket)||transfer.paused)return false
        val action=Uuid.random();val boot=connection.attachmentBoot(transfer.original.context)
        transfer.receipt=null;transfer.status=null;transfer.record=transfer.record.copy(actionId=action);store.saveAttachment(transfer.record)
        val receipt=connection.mutate(transfer.record.host,Request(action,body(boot)),ordinaryComposer=false,attachment=transfer.original.context)
        transfer.receipt=receipt
        if(transfer.record.actionId!=action)throw StorePolicy(ErrorCode.ActionIdConflict)
        val facts=receipt.result as? ActionResultAttachment
        if(facts!=null){if(facts.target!=transfer.record.target||facts.progress.uploadId!=transfer.record.uploadId)throw StorePolicy(ErrorCode.InvalidRequest);retainAttachment(transfer,facts.progress)}
        refreshActions()
        if(receipt.state!=ReceiptState.Succeeded){transfer.unknown=receipt.state!=ReceiptState.Failed;transfer.paused=true;transfer.notice="Inspect this action and the confirmed desktop offset before a separate continuation.";return false}
        return true
    }
    private suspend fun retainAttachment(transfer:AttachmentTransfer,progress:AttachmentProgress) {
        val record=transfer.record
        if(progress.uploadId!=record.uploadId||progress.committedBytes.value<record.committedBytes)throw StorePolicy(ErrorCode.InvalidRequest)
        transfer.record=record.copy(committedBytes=progress.committedBytes.value,providerPath=progress.providerPath ?: record.providerPath,published=progress.published==true||record.published,discarded=progress.discarded==true||record.discarded)
        store.saveAttachment(transfer.record)
    }
    private suspend fun runAttachment(transfer:AttachmentTransfer,begin:Boolean) {
        val prepared=transfer.prepared ?: throw StorePolicy(ErrorCode.NotReady)
        transfer.working=true;transfer.paused=false;transfer.unknown=false
        try {
            val target=transfer.record.target;val upload=transfer.record.uploadId
            if(begin&&!attachmentStep(transfer){boot->RequestBody.AttachmentsBegin(AttachmentsBeginParams(boot,target,upload,prepared.displayName,prepared.mediaType,Uint(prepared.size),prepared.sha256))})return
            while(transfer.record.committedBytes<prepared.size){
                if(!attachmentAdmitted(transfer.ticket)||transfer.paused)break
                val offset=transfer.record.committedBytes;val bytes=attachmentStaging.readChunk(prepared,offset)
                try {val digest=Protocol.sha256(bytes).joinToString(""){"%02x".format(it.toInt() and 255)};val encoded=java.util.Base64.getEncoder().encodeToString(bytes)
                    if(!attachmentStep(transfer){boot->RequestBody.AttachmentsChunk(AttachmentsChunkParams(boot,target,upload,Uint(offset),encoded,digest))})return
                }finally{bytes.fill(0)}
            }
            if(transfer.record.committedBytes==prepared.size&&!transfer.paused&&attachmentAdmitted(transfer.ticket)){
                if(attachmentStep(transfer){boot->RequestBody.AttachmentsComplete(AttachmentsCompleteParams(boot,target,upload))}){
                    transfer.status=connection.attachmentStatus(transfer.record.host,target,upload,transfer.original.context)
                    transfer.notice="Uploaded privately. Appending its path is a separate choice; no prompt was sent."
                }
            }
        }catch(cancel:CancellationException){throw cancel}
        catch(_:Exception){transfer.unknown=true;transfer.paused=true;transfer.notice="Upload stopped. Inspect the original action and desktop offset; no automatic retry."}
        finally {transfer.working=false}
    }
    private fun pauseAttachments(){attachmentPreviewJob?.cancel();attachmentPreviewJob=null;attachmentTransfer?.let {it.preview=null;it.previewLoading=false;it.paused=true;it.original.context.lease.invalidate();it.status=it.status?.copy(insertionReady=false)};attachmentPickerConsent?.takeUnless {attachmentPickerHold?.original===it}?.context?.lease?.invalidate()}
    internal fun closeAttachments(){pauseAttachments();attachmentCleanup?.lease?.invalidate();attachmentCleanup=null;attachmentVisible=false}
    internal fun showAttachments(){attachmentTransfer?.let {attachmentVisible=true}}
    internal fun previewAttachment(transfer:AttachmentTransfer) {
        val prepared=transfer.prepared ?: return
        if(!foreground||!attachmentVisible||attachmentTransfer!==transfer||transfer.previewLoading)return
        attachmentPreviewJob?.cancel();transfer.preview=null;transfer.previewLoading=true;transfer.previewUnavailable=false
        val generation=foregroundGeneration
        attachmentPreviewJob=viewModelScope.launch {
            try {
                val preview=attachmentStaging.preview(prepared)
                if(foreground&&foregroundGeneration===generation&&attachmentVisible&&attachmentTransfer===transfer){transfer.preview=preview;transfer.previewUnavailable=preview==null}
            }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(attachmentTransfer===transfer)transfer.previewUnavailable=true}
            finally {if(attachmentTransfer===transfer)transfer.previewLoading=false}
        }
    }
    internal fun inspectAttachment(transfer:AttachmentTransfer){if(transfer.working)return;val observation=Any();val action=transfer.record.actionId;transfer.inspection=observation;transfer.inspectedAction=null;viewModelScope.launch {
        try {
            if(action!=null){when(val outcome=connection.status(transfer.record.host,action)){is ActionStatusResult.Known->transfer.receipt=outcome.value.receipt;is ActionStatusResult.Unknown->{transfer.receipt=null;transfer.unknown=true}}}
            val status=connection.attachmentStatus(transfer.record.host,transfer.record.target,transfer.record.uploadId,transfer.original.context)
            if(attachmentTransfer!==transfer||transfer.inspection!==observation||transfer.record.actionId!=action)return@launch
            if(status.size.value!=transfer.record.size)throw StorePolicy(ErrorCode.InvalidRequest)
            retainAttachment(transfer,status.progress);transfer.status=status;transfer.inspectedAction=action;transfer.consentAction=null;refreshActions()
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){transfer.notice="Status unavailable. Retained facts do not authorize another frame.";transfer.status=null}
    }}
    internal fun prepareAttachmentResume(transfer:AttachmentTransfer){if(transfer.working||transfer.status?.state!=AttachmentUploadState.Receiving||transfer.inspectedAction==null||transfer.inspectedAction!=transfer.record.actionId)return;transfer.consentAction=transfer.record.actionId}
    internal fun confirmAttachmentResume(transfer:AttachmentTransfer,action:Uuid,status:AttachmentsStatusResult){
        if(transfer.working||transfer.consentAction!=action||transfer.inspectedAction!=action||transfer.record.actionId!=action||transfer.status!==status||status.state!=AttachmentUploadState.Receiving||status.progress.committedBytes.value!=transfer.record.committedBytes)return
        val key=ConversationKey(transfer.record.host,transfer.record.target);val ticket=captureAttachmentTicket(key,status) ?: return
        val original=ticket.originalScope as AttachmentConsentScope
        val resumed=AttachmentTransfer(ticket,original,transfer.prepared,transfer.record);resumed.reviewedDraft=original.revision to original.cursor
        transfer.original.context.lease.invalidate();attachmentTransfer=resumed;attachmentVisible=true
        attachmentJob=viewModelScope.launch {runAttachment(resumed,false)}
    }
    internal fun discardAttachment(transfer:AttachmentTransfer){if(transfer.working||!foreground||attachmentTransfer!==transfer)return;val generation=foregroundGeneration;viewModelScope.launch {
        transfer.working=true
        try {val context=connection.captureAttachmentCleanup(transfer.record);if(!foreground||foregroundGeneration!==generation||attachmentTransfer!==transfer){context.lease.invalidate();return@launch};attachmentCleanup=context;val action=Uuid.random();val boot=connection.state.value.hello[transfer.record.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
            transfer.record=transfer.record.copy(actionId=action);store.saveAttachment(transfer.record)
            val receipt=connection.mutate(transfer.record.host,Request(action,RequestBody.AttachmentsDiscard(AttachmentsDiscardParams(boot,transfer.record.target,transfer.record.uploadId))),attachment=context)
            transfer.receipt=receipt;(receipt.result as? ActionResultAttachment)?.let {retainAttachment(transfer,it.progress)}
            if(receipt.state==ReceiptState.Succeeded&&transfer.record.discarded){transfer.record.local?.let {if(attachmentStaging.discard(it.prepared())){transfer.record=transfer.record.copy(local=null);store.saveAttachment(transfer.record)}};transfer.notice="Desktop staging discarded. This does not remove any file copied elsewhere."}
            else transfer.notice="Discard outcome unknown. Inspect its receipt; no automatic retry."
            refreshActions()
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){transfer.unknown=true;transfer.notice="Discard outcome unavailable. Inspect its receipt."}finally{transfer.working=false}
    }}
    internal fun attachmentDraftCurrent(transfer:AttachmentTransfer):Boolean=transfer.reviewedDraft==((composerRevisions[transfer.original.key] ?: 0L) to composerCursors[transfer.original.key])
    internal fun reviewAttachmentDraft(transfer:AttachmentTransfer){if(attachmentAdmitted(transfer.ticket)){val cursor=composerCursors[transfer.original.key] ?: return;transfer.reviewedDraft=(composerRevisions[transfer.original.key] ?: 0L) to cursor;transfer.notice="Review the current draft. The next Append path adds a new line without sending."}}
    internal fun insertAttachment(transfer:AttachmentTransfer){
        if(!composerEditable(transfer.original.key))return
        val status=transfer.status ?: return;val path=status.progress.providerPath ?: return
        if(!status.insertionReady||status.state!=AttachmentUploadState.Complete||!attachmentAdmitted(transfer.ticket)||!attachmentDraftCurrent(transfer)||!connection.attachmentStatusCurrent(transfer.original.context,status))return
        val current=composers[transfer.original.key].orEmpty();val appended=if(current.isEmpty())path else current+"\n"+path
        try {PromptText(appended)}catch(_:Exception){transfer.notice="The path does not fit in this draft.";return}
        if(!attachmentAdmitted(transfer.ticket)||!attachmentDraftCurrent(transfer)||!connection.attachmentStatusCurrent(transfer.original.context,status))return
        composer(appended);skillsFocusSignal+=1;transfer.notice="Path appended to the original draft. It was not sent.";transfer.reviewedDraft=null
    }
    internal fun openRetainedAttachment(record:LocalAttachmentUpload){
        if(attachmentTransfer?.working==true)return
        val key=ConversationKey(record.host,record.target);val ticket=captureAttachmentTicket(key)
        val original=if(ticket!=null)ticket.originalScope as AttachmentConsentScope else null
        if(ticket==null||original==null){notice="Open the original conversation to inspect this upload. Its retained facts do not select another session.";return}
        val transfer=AttachmentTransfer(ticket,original,record.local?.prepared(),record);transfer.reviewedDraft=original.revision to original.cursor
        attachmentTransfer=transfer;attachmentVisible=true;inspectAttachment(transfer)
    }
    internal fun showAttachmentRecovery(record:LocalAttachmentUpload){if(!foreground)return;attachmentRecovery=AttachmentRecovery(Any(),record);inspectAttachmentRecovery(attachmentRecovery!!)}
    internal fun closeAttachmentRecovery(){attachmentCleanup?.lease?.invalidate();attachmentCleanup=null;attachmentRecovery=null}
    internal fun inspectAttachmentRecovery(recovery:AttachmentRecovery){if(recovery.loading)return;recovery.loading=true;viewModelScope.launch {
        try {val action=recovery.record.actionId
            if(action!=null){when(val result=connection.status(recovery.record.host,action)){is ActionStatusResult.Known->recovery.receipt=result.value.receipt;is ActionStatusResult.Unknown->{recovery.receipt=null;recovery.notice="The original action is currently unknown. It will not be replayed."}}}
            recovery.status=connection.attachmentStatus(recovery.record.host,recovery.record.target,recovery.record.uploadId,null)
            recovery.status?.let {status->val prior=recovery.record
                if(status.target!=prior.target||status.progress.uploadId!=prior.uploadId||status.size.value!=prior.size||status.progress.committedBytes.value<prior.committedBytes)throw StorePolicy(ErrorCode.InvalidRequest)
                recovery.record=prior.copy(committedBytes=status.progress.committedBytes.value,providerPath=status.progress.providerPath?:prior.providerPath,published=prior.published||status.progress.published==true,discarded=prior.discarded||status.progress.discarded==true);store.saveAttachment(recovery.record)
            }
            recovery.notice="Owner status only. Path insertion and further upload require the exact original conversation."
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){recovery.notice="Owner status unavailable. Retained facts do not prove completion."}finally{recovery.loading=false}
    }}
    internal fun openAttachmentOriginal(recovery:AttachmentRecovery){if(attachmentRecovery!==recovery||!foreground)return;closeAttachmentRecovery();open(recovery.record.host,recovery.record.target)}
    internal fun discardAttachmentRecovery(recovery:AttachmentRecovery){if(recovery.loading||attachmentRecovery!==recovery||!foreground)return;val generation=foregroundGeneration;recovery.loading=true;viewModelScope.launch {
        try {val context=connection.captureAttachmentCleanup(recovery.record);if(!foreground||foregroundGeneration!==generation||attachmentRecovery!==recovery){context.lease.invalidate();return@launch};attachmentCleanup=context;val action=Uuid.random();val boot=connection.attachmentBoot(context)
            recovery.record=recovery.record.copy(actionId=action);store.saveAttachment(recovery.record)
            val receipt=connection.mutate(recovery.record.host,Request(action,RequestBody.AttachmentsDiscard(AttachmentsDiscardParams(boot,recovery.record.target,recovery.record.uploadId))),attachment=context)
            recovery.receipt=receipt
            (receipt.result as? ActionResultAttachment)?.progress?.let {progress->val prior=recovery.record;if(progress.uploadId!=prior.uploadId||progress.committedBytes.value<prior.committedBytes)throw StorePolicy(ErrorCode.InvalidRequest);recovery.record=prior.copy(committedBytes=progress.committedBytes.value,providerPath=progress.providerPath?:prior.providerPath,published=prior.published||progress.published==true,discarded=prior.discarded||progress.discarded==true);store.saveAttachment(recovery.record)}
            if(receipt.state==ReceiptState.Succeeded&&recovery.record.discarded){recovery.record.local?.let {if(attachmentStaging.discard(it.prepared())){recovery.record=recovery.record.copy(local=null);store.saveAttachment(recovery.record)}};recovery.notice="Desktop staging discarded. No provider copy was removed."}else recovery.notice="Discard is not confirmed. Inspect this action; no automatic retry."
            refreshActions()
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){recovery.notice="Discard outcome unavailable. Inspect its retained action."}finally{recovery.loading=false}
    }}
    private suspend fun cleanupLocalAttachments(){
        val records=store.state.value.attachments
        attachmentStaging.sweepExpired(records.mapNotNull {it.local?.id}.toSet())
        for(record in records){val local=record.local
            if(attachmentTransfer?.let {it.record.uploadId==record.uploadId&&it.working}==true)continue
            if(local==null){if(record.forgetPending)store.removeAttachment(record)else store.retireAttachment(record);continue}
            if(record.forgetPending||record.discarded||System.currentTimeMillis()>=local.expiresAt){if(attachmentStaging.discard(local.prepared())){val cleared=record.copy(local=null);store.saveAttachment(cleared);if(record.forgetPending)store.removeAttachment(cleared)else store.retireAttachment(cleared)}}
        }
    }
    private fun composerEditable(key:ConversationKey):Boolean {
        if(!submittedComposer(key))return true
        notice="This submitted snapshot is read-only. Choose New message, or explicitly copy it with the duplicate-work warning."
        return false
    }
    fun composer(text: String) { val key = selection ?: return;if(!composerEditable(key))return;if(composers[key]!=text){composerRevisions[key]=(composerRevisions[key] ?: 0L).let {if(it==Long.MAX_VALUE)it else it+1};composerCursors[key]=text.length to text.length}; composers[key] = text; reserve(text) }
    internal fun composerCursor(start:Int,end:Int){val key=selection ?: return;val length=composers[key].orEmpty().length;if(start in 0..length&&end in 0..length)composerCursors[key]=start to end}
    internal fun composerSelection(key:ConversationKey):Pair<Int,Int> {val length=composers[key].orEmpty().length;val cursor=composerCursors[key] ?: return length to length;return if(cursor.first in 0..length&&cursor.second in 0..length)cursor else cursor.first.coerceIn(0,length) to cursor.second.coerceIn(0,length)}
    internal fun initializeComposerSelection(key:ConversationKey){if(selection!=key)return;val current=composerSelection(key);if(composerCursors[key]!=current)composerCursors[key]=current}
    internal fun clearComposerDraft(key:ConversationKey):Boolean {
        if(selection!=key||!foreground||busy||!composerEditable(key))return false
        composerUploads.remove(key);composerDeliveries.remove(key)
        composer("");composerCursors[key]=0 to 0
        return true
    }
    internal fun readerPosition(key:ConversationKey,identity:ConversationReaderIdentity?):ConversationReaderPosition {
        val previous=readerPositions[key]
        if(previous!=null&&(identity==null||previous.identity==identity))return previous
        previous?.retire()
        val position=ConversationReaderPosition(identity);readerPositions[key]=position
        while(readerPositions.size>64)readerPositions.remove(readerPositions.keys.first())?.retire()
        return position
    }
    internal fun composerOptions(key:ConversationKey):ComposerOptions {
        val delivery=composerDeliveries[key]
        return ComposerOptions(composerUploads[key]?.map {it.record.uploadId}?.takeIf {it.isNotEmpty()},delivery?.first,delivery?.second)
    }
    internal fun submittedComposer(key:ConversationKey):Boolean=sentText.values.any {it.key==key}
    internal fun newComposerMessage(key:ConversationKey,copy:Boolean=false,duplicateWarningAccepted:Boolean=false) {
        if(selection!=key||!foreground||busy||copy&&!duplicateWarningAccepted)return
        val submitted=sentText.values.lastOrNull {it.key==key} ?: return
        sentText.keys.filter {sentText[it]?.key==key}.forEach(sentText::remove)
        reservation=null;replacement=null;warningJob?.cancel()
        if(!copy){composers[key]="";composerUploads.remove(key);composerDeliveries.remove(key)}
        else {
            composers[key]=submitted.text
            composerDeliveries[key]=submitted.options.delivery?.let {it to submitted.options.nativeTurnId} ?: (PromptDelivery.Native to null)
            val original=composerUploads[key].orEmpty()
            composerUploads[key]=original.filter {it.record.uploadId in submitted.options.images.orEmpty()}
            if(composerUploads[key]?.size!=submitted.options.images.orEmpty().size)notice="Original image references are inspection-only. Explicitly select current completed uploads again."
        }
        composerRevisions[key]=(composerRevisions[key] ?: 0)+1
        val draftLength=composers[key].orEmpty().length;composerCursors[key]=draftLength to draftLength
        reserve(composers[key].orEmpty())
    }
    internal fun removeComposerImage(key:ConversationKey,upload:Uuid) {
        if(selection!=key||submittedComposer(key)||busy)return
        composerUploads[key]=composerUploads[key].orEmpty().filterNot {it.record.uploadId==upload}
        composerRevisions[key]=(composerRevisions[key] ?: 0)+1;reserve(composers[key].orEmpty())
    }
    internal fun attachNativeImage(transfer:AttachmentTransfer) {
        val key=transfer.original.key;val status=transfer.status ?: return
        if(submittedComposer(key)||transfer.working||!attachmentAdmitted(transfer.ticket)||!attachmentDraftCurrent(transfer)
            ||status.state!=AttachmentUploadState.Complete||!status.insertionReady||!connection.attachmentStatusCurrent(transfer.original.context,status))return
        val record=transfer.record
        if(record.host!=key.host||record.target!=key.target||status.target!=key.target
            ||status.progress.uploadId!=record.uploadId||status.size.value!=record.size)return
        val previous=composerUploads[key].orEmpty()
        val index=previous.indexOfFirst {it.record.uploadId==record.uploadId}
        if(index<0&&previous.size>=4||previous.sumOf {it.record.size}-(previous.getOrNull(index)?.record?.size ?: 0)+record.size>8L*1024*1024){transfer.notice="Image inputs are limited to four completed uploads and 8 MiB total.";return}
        val selected=ComposerImageInput(record,key,transfer.original.source)
        composerUploads[key]=previous.toMutableList().apply {if(index<0)add(selected)else set(index,selected)}
        composerRevisions[key]=(composerRevisions[key] ?: 0)+1;transfer.reviewedDraft=null
        transfer.notice="Image input attached to the original draft, not sent. Send checks the actual native model and file type."
        reserve(composers[key].orEmpty())
    }
    internal fun controlsAdmitted(shown:SessionControlsState):Boolean {
        if(sessionControls!==shown||!foreground||foregroundGeneration!==shown.foreground||selection!=shown.key||screen!=Screen.Conversation||!shown.lease.active.get())return false
        val view=connection.state.value
        val reducer=view.sessions[shown.key.host]?.singleOrNull {it.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==shown.key.target}==true} ?: return false
        return view.sources[shown.key.host]?.get(shown.key.target)==shown.source&&view.inventoryReady[shown.key.host]==true
            &&view.mutationReady[shown.key.host]==true&&reducer.mutationsEnabled&&reducer.state?.summary?.capabilities?.controls==true
    }
    internal fun openSessionControls(key:ConversationKey) {
        val view=connection.state.value;val source=view.sources[key.host]?.get(key.target) ?: return
        val generation=foregroundGeneration ?: return
        closeSessionControls()
        val shown=SessionControlsState(key,source,generation);sessionControls=shown
        if(!controlsAdmitted(shown)){closeSessionControls();notice="Native controls require this current original session and its local grant.";return}
        refreshSessionControls(shown)
    }
    internal fun closeSessionControls() {
        sessionControls?.retire();sessionControls=null
        controlsReadJob?.cancel();controlsReadJob=null;queueReadJob?.cancel();queueReadJob=null
    }
    private fun servicePendingNativeReads(shown:SessionControlsState) {
        if(!controlsAdmitted(shown))return
        val reducer=connection.state.value.sessions[shown.key.host]?.firstOrNull {it.state?.summary?.sessionId==shown.key.target.sessionId} ?: return
        shown.observeRevisions(reducer.controlsRevision,reducer.queueRevision)
        if(shown.controlsRefreshNeeded)refreshSessionControls(shown)
        if(shown.queueRefreshNeeded)openNativeQueue(shown)
    }
    internal fun refreshSessionControls(shown:SessionControlsState) {
        if(!controlsAdmitted(shown))return
        val revision=connection.state.value.sessions[shown.key.host]?.firstOrNull {it.state?.summary?.sessionId==shown.key.target.sessionId}?.controlsRevision ?: return
        if(!shown.beginControlsRead(revision))return
        controlsReadJob?.cancel();shown.catalogue=null;shown.error=null
        controlsReadJob=viewModelScope.launch {
            try {
                val catalogue=connection.nativeControls(shown.key.host,shown.key.target,shown.source,shown.lease){controlsAdmitted(shown)}
                if(!controlsAdmitted(shown)||!shown.controlsReadCurrent(revision))return@launch
                shown.catalogue=catalogue;shown.model=catalogue.current.modelRef;shown.effort=catalogue.current.effort;shown.mode=catalogue.current.mode
            }catch(cancel:CancellationException){throw cancel}catch(error:Exception){if(sessionControls===shown)shown.error="Native controls unavailable: ${safeFailure(error)}"}
            finally {if(sessionControls===shown){controlsReadJob=null;shown.settleControlsRead();servicePendingNativeReads(shown)}}
        }
    }
    internal fun sessionControlsApplyAllowed(shown:SessionControlsState):Boolean {
        val boot=connection.state.value.hello[shown.key.host]?.bootId ?: return false
        val params=shown.changes(boot) ?: return false
        val catalogue=shown.catalogue ?: return false
        val selected=catalogue.models.singleOrNull {it.reference==(params.modelRef?:catalogue.current.modelRef)}
        return controlsAdmitted(shown)&&!shown.pending&&!shown.loading
            &&(params.effort==null||selected?.efforts?.contains(params.effort)==true)
            &&(params.mode==null||params.mode in catalogue.modes&&selected!=null)
    }
    internal fun applySessionControls(shown:SessionControlsState) {
        if(!sessionControlsApplyAllowed(shown))return
        val boot=connection.state.value.hello[shown.key.host]?.bootId ?: return
        val params=shown.changes(boot) ?: return
        val catalogue=shown.catalogue ?: return
        shown.pending=true;shown.error=null;shown.receipt=null
        viewModelScope.launch {
            try {
                val context=connection.controlsMutation(shown.key.host,catalogue,shown.lease){controlsAdmitted(shown)}
                if(!controlsAdmitted(shown))return@launch
                val receipt=connection.mutate(shown.key.host,Request(Uuid.random(),RequestBody.SessionControlsApply(params)),controls=context)
                if(sessionControls===shown)shown.receipt=receipt
                refreshActions()
            }catch(cancel:CancellationException){throw cancel}catch(error:Exception){if(sessionControls===shown)shown.error="Apply outcome unavailable; inspect receipts. ${safeFailure(error)}"}
            finally {if(sessionControls===shown){shown.pending=false;if(controlsAdmitted(shown)){refreshSessionControls(shown);servicePendingNativeReads(shown)}}}
        }
    }
    internal fun selectPromptDelivery(shown:SessionControlsState,delivery:PromptDelivery) {
        val catalogue=shown.catalogue ?: return
        if(!controlsAdmitted(shown)||shown.pending||shown.loading||delivery !in catalogue.deliveries||submittedComposer(shown.key))return
        val summary=connection.state.value.sessions[shown.key.host]?.firstOrNull {it.state?.summary?.sessionId==shown.key.target.sessionId}?.state ?: return
        val expected=if(delivery==PromptDelivery.Steer&&summary.summary.provider==Provider.Codex)summary.nativeTurnId ?: return else null
        if(delivery==PromptDelivery.Native)composerDeliveries.remove(shown.key)else composerDeliveries[shown.key]=delivery to expected
        reserve(composers[shown.key].orEmpty())
    }
    internal fun openNativeQueue(shown:SessionControlsState) {
        if(!controlsAdmitted(shown)||shown.pending||shown.queueLoading||shown.catalogue?.queueAvailable!=true)return
        queueReadJob?.cancel();shown.queueOpen=true;shown.queueEntries=emptyList();shown.queueText=emptyMap();shown.next=null;shown.queueLoaded=false
        readNativeQueue(shown,null)
    }
    internal fun moreNativeQueue(shown:SessionControlsState) {
        val cursor=shown.next ?: return
        if(!controlsAdmitted(shown)||shown.queueLoading||shown.pending||!shown.queueLoaded)return
        readNativeQueue(shown,cursor)
    }
    private fun readNativeQueue(shown:SessionControlsState,cursor:Id?) {
        val revision=connection.state.value.sessions[shown.key.host]?.firstOrNull {it.state?.summary?.sessionId==shown.key.target.sessionId}?.queueRevision ?: return
        val readCursor=nativeQueueReadCursor(shown,revision,cursor)
        if(!shown.beginQueueRead(revision))return
        shown.queueError=null
        queueReadJob=viewModelScope.launch {
            try {
                val result=connection.nativeQueue(shown.key.host,shown.key.target,shown.source,readCursor,shown.lease){controlsAdmitted(shown)&&shown.queueOpen}
                val texts=LinkedHashMap(shown.queueText)
                for(entry in result.entries) {
                    val text=when(val content=entry.text) {
                        is ContentText->content.text.value
                        is ContentUnavailable->"[Native content unavailable: ${content.reason}]"
                        is ContentRef-> {
                            if(content.byteLength.value>256L*1024)throw StorePolicy(ErrorCode.ResourceLimit)
                            val output=java.io.ByteArrayOutputStream(content.byteLength.value.toInt());var offset=0L
                            while(offset<content.byteLength.value) {
                                if(!controlsAdmitted(shown)||!shown.queueReadCurrent(revision))throw StorePolicy(ErrorCode.StaleSession)
                                val chunk=connection.content(shown.key.host,shown.source,ContentReadParams(shown.key.target,content.handle,Uint(offset),Uint(minOf(65536L,content.byteLength.value-offset))))
                                val bytes=java.util.Base64.getDecoder().decode(chunk.bytes.value)
                                if(bytes.isEmpty()||chunk.nextOffsetBytes.value!=offset+bytes.size||chunk.nextOffsetBytes.value>content.byteLength.value||chunk.eof!=(chunk.nextOffsetBytes.value==content.byteLength.value))throw StorePolicy(ErrorCode.InvalidRequest)
                                output.write(bytes);bytes.fill(0);offset=chunk.nextOffsetBytes.value
                            }
                            val bytes=output.toByteArray()
                            try {Charsets.UTF_8.newDecoder().decode(java.nio.ByteBuffer.wrap(bytes)).toString()}finally {bytes.fill(0)}
                        }
                    }
                    texts[entry.id]=text
                }
                if(!controlsAdmitted(shown)||!shown.queueReadCurrent(revision))return@launch
                shown.queueEntries=shown.queueEntries+result.entries;shown.queueText=texts;shown.next=result.nextCursor;shown.queueLoaded=true
            }catch(cancel:CancellationException){throw cancel}catch(error:Exception){if(sessionControls===shown){shown.queueLoaded=false;shown.queueError="Native queue unavailable: ${safeFailure(error)}"}}
            finally {if(sessionControls===shown){queueReadJob=null;shown.settleQueueRead();servicePendingNativeReads(shown)}}
        }
    }
    private fun mutateQueue(shown:SessionControlsState,body:(Uuid)->RequestBody) {
        if(!controlsAdmitted(shown)||shown.pending||shown.queueLoading||!shown.queueLoaded)return
        val boot=connection.state.value.hello[shown.key.host]?.bootId ?: return
        shown.pending=true;shown.queueReceipt=null;shown.queueError=null
        viewModelScope.launch {
            try {
                val context=connection.queueMutation(shown.key.host,shown.key.target,shown.source,shown.lease){controlsAdmitted(shown)&&shown.queueOpen}
                if(!controlsAdmitted(shown))return@launch
                val receipt=connection.mutate(shown.key.host,Request(Uuid.random(),body(boot)),queue=context)
                if(sessionControls===shown)shown.queueReceipt=receipt
                refreshActions()
            }catch(cancel:CancellationException){throw cancel}catch(error:Exception){if(sessionControls===shown)shown.queueError="Queue outcome unavailable; inspect receipts. ${safeFailure(error)}"}
            finally {if(sessionControls===shown){shown.pending=false;if(controlsAdmitted(shown)){openNativeQueue(shown);servicePendingNativeReads(shown)}}}
        }
    }
    internal fun updateQueueEntry(shown:SessionControlsState,id:Id,text:String) {
        if(shown.queueEntries.none {it.id==id&&it.editable})return
        val prompt=try {PromptText(text).also {require(text.isNotEmpty())}}catch(_:Exception){return}
        mutateQueue(shown){RequestBody.SessionQueueUpdate(SessionQueueUpdateParams(it,shown.key.target,id,prompt))}
    }
    internal fun deleteQueueEntry(shown:SessionControlsState,id:Id) {
        if(shown.queueEntries.none {it.id==id})return
        mutateQueue(shown){RequestBody.SessionQueueDelete(SessionQueueDeleteParams(it,shown.key.target,id))}
    }
    internal fun startQueueEntry(shown:SessionControlsState,id:Id) {
        if(shown.queueEntries.none {it.id==id})return
        mutateQueue(shown){RequestBody.SessionQueueStart(SessionQueueStartParams(it,shown.key.target,id))}
    }
    internal fun moveQueueEntryUp(shown:SessionControlsState,id:Id) {
        if(!shown.queueLoaded||shown.next!=null)return
        val ids=shown.queueEntries.map {it.id}.toMutableList();val index=ids.indexOf(id);if(index<=0)return
        java.util.Collections.swap(ids,index,index-1)
        mutateQueue(shown){RequestBody.SessionQueueReorder(SessionQueueReorderParams(it,shown.key.target,ids))}
    }

    /** Saving hints is local-only and deliberately does not reconnect or replace a live attempt. */
    fun saveHostRoutes(host: String, revision: Long, direct: List<String>, relay: String?, onSaved: (SavedHostRoutes) -> Unit, onFinished: () -> Unit) {
        viewModelScope.launch {
            try {onSaved(store.saveHostRoutes(host,revision,direct,relay))}
            catch(cancel: CancellationException) {throw cancel}
            catch(error: Exception) {notice=if(error is RouteValidation) error.message else safeFailure(error)}
            finally {onFinished()}
        }
    }
    /** Local-only snippet persistence; values enter storage only through this explicit Save path. */
    fun saveSnippet(host: String, id: Uuid, title: String, body: String, onSaved: () -> Unit = {}) {
        viewModelScope.launch {
            try { store.saveSnippet(host, id, title, body); onSaved() }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { notice = safeFailure(error) }
        }
    }
    fun deleteSnippet(host: String, id: Uuid) {
        viewModelScope.launch {
            try { store.deleteSnippet(host, id) }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { notice = safeFailure(error) }
        }
    }
    /** Append only to the exact selection captured by the picker; this never sends or creates a receipt. */
    fun insertSnippet(key: ConversationKey, id: Uuid): Boolean {
        if(!composerEditable(key))return false
        if (selection != key || store.state.value.hosts.none { it.hostId == key.host }) {
            notice = "Snippet insertion needs the original paired conversation still selected."
            return false
        }
        val snippet = store.state.value.snippets.singleOrNull { it.id == id && it.hostId == key.host } ?: run {
            notice = "This local snippet is no longer available for the selected desktop."
            return false
        }
        val current = composers[key].orEmpty()
        val appended = if (current.isEmpty()) snippet.body else "$current\n${snippet.body}"
        try { PromptText(appended) }
        catch (_: Exception) {
            notice = "This snippet does not fit in the current message. Edit the draft or snippet before inserting."
            return false
        }
        if (selection != key || store.state.value.hosts.none { it.hostId == key.host }) {
            notice = "Snippet insertion needs the original paired conversation still selected."
            return false
        }
        composer(appended)
        return true
    }
    private fun reserve(text: String) {
        warningJob?.cancel(); reservation = null; replacement = null
        val key = selection ?: return
        if(submittedComposer(key))return
        val options=composerOptions(key)
        if (text.isEmpty()&&options.images==null) return
        val action = Uuid.random()
        warningJob = viewModelScope.launch {
            delay(150)
            try {
                val warning = store.recoveryWarning(key.host,action,PromptText(text),options)
                if (selection == key && !submittedComposer(key)&&composers[key].orEmpty() == text&&composerOptions(key)==options) reservation = SendReservation(key,action,text,warning,options)
            } catch (error: Exception) { notice = safeFailure(error) }
        }
    }
    fun send() {
        val prepared = reservation ?: return
        if (prepared.warning.replace.isNotEmpty()) { replacement = prepared; return }
        send(prepared)
    }
    fun approveReplacement() { replacement?.let(::send); replacement = null }
    fun declineReplacement() { replacement = null }
    fun prepareNewPrompt() {
        val key=selection ?: return
        if(!busy) reserve(composers[key].orEmpty())
    }
    private fun send(prepared: SendReservation) = perform {
        check(foreground && selection == prepared.key && reservation == prepared)
        check(composerOptions(prepared.key)==prepared.options&&!submittedComposer(prepared.key))
        val source=connection.state.value.sources[prepared.key.host]?.get(prepared.key.target)
        check(composerUploads[prepared.key].orEmpty().all {it.current(prepared.key,source)})
        val boot = connection.state.value.hello[prepared.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        // Consume this UUID once. An uncertain result remains status-only, never replayed.
        reservation = null
        sentText[RecoveryIdentity(prepared.key.host,prepared.action)] = SubmittedComposer(prepared.key,prepared.text,prepared.options,prepared.action)
        focusedId=prepared.action
        try {connection.mutate(prepared.key.host,Request(prepared.action,RequestBody.PromptSubmit(PromptSubmitParams(boot,prepared.key.target,PromptText(prepared.text),delivery=prepared.options.delivery,nativeTurnId=prepared.options.nativeTurnId,images=prepared.options.images))),true,RecoveryApproval(prepared.warning.replace,prepared.warning.futureExpiry))}
        catch(error: RecoveryApprovalRequired) {
            // No pre-send commit/native request occurred. Prepare a NEW visible notice, not a replay.
            sentText.remove(RecoveryIdentity(prepared.key.host,prepared.action))
            if(selection==prepared.key && composers[prepared.key]===prepared.text) reserve(prepared.text)
            throw error
        }
    }
    fun stop() = perform {
        val key = selection ?: return@perform; check(foreground)
        val view = connection.state.value
        val state = view.sessions[key.host]?.firstOrNull { it.state?.summary?.sessionId == key.target.sessionId }?.state ?: throw StorePolicy(ErrorCode.NotReady)
        val boot = view.hello[key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val params = TurnCancelParams(boot,key.target,if(state.summary.provider == Provider.Codex) state.nativeTurnId else null)
        validateTurnCancel(state.summary.provider,params)
        val action=Uuid.random();focusedId=action
        connection.mutate(key.host,Request(action,RequestBody.TurnCancel(params)))
    }
    fun respond(request: PendingRequest,answer: Answer) {
        val key = selection ?: return
        validateAnswer(request.presentation,answer)
        forms[Triple(key.host,key.target,request.requestKey)]?.clearSecrets()
        perform {
            check(foreground && selection == key)
            val boot = connection.state.value.hello[key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
            val action=Uuid.random();focusedId=action
            connection.mutate(key.host,Request(action,RequestBody.RequestRespond(RequestRespondParams(boot,key.target,request.requestKey,answer))))
        }
    }
    internal fun creationBrowser(host:String,agent:AgentSessionMutationContext?=null,viewCurrent:()->Boolean={true}):CreationDirectoryBrowser {
        val generation=foregroundGeneration;val originalScreen=screen;val originalSelection=selection;val originalHost=this.host
        creationDirectoriesBrowser?.let(::closeCreationDirectories)
        return CreationDirectoryBrowser(connection,viewModelScope,host,agent) {
            generation!=null&&foreground&&foregroundGeneration===generation&&screen==originalScreen
                &&selection==originalSelection&&this.host==originalHost&&viewCurrent()
        }.also {creationDirectoriesBrowser=it}
    }
    internal fun closeCreationDirectories(browser:CreationDirectoryBrowser) {
        browser.close()
        if(creationDirectoriesBrowser===browser)creationDirectoriesBrowser=null
    }
    internal fun closeAgentSession(shown:AgentSessionCreation?=null){
        val current=agentSessionCreation ?: return
        if(shown!=null&&shown!==current)return
        current.context.lease.invalidate();current.directories?.let(::closeCreationDirectories);current.memory.clear();agentSessionCreation=null
    }
    private fun agentSessionAdmitted(shown:AgentSessionCreation):Boolean {
        val view=connection.state.value
        return agentSessionCreation===shown&&foreground&&foregroundGeneration===shown.foreground&&screen==Screen.Conversation&&selection==shown.key&&store.state.value.hosts.any {it.hostId==shown.key.host}&&view.sources[shown.key.host]?.get(shown.key.target)==shown.source&&connection.agentSessionCurrent(shown.context)
    }
    internal fun beginAgentSession(key:ConversationKey){
        closeAgentSession();val generation=foregroundGeneration ?: return
        val view=connection.state.value;val source=view.sources[key.host]?.get(key.target) ?: return
        val reducer=view.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true} ?: return
        val summary=reducer.state?.summary ?: return
        if(!foreground||screen!=Screen.Conversation||selection!=key||view.inventoryReady[key.host]!=true||view.mutationReady[key.host]!=true)return
        try {
            val context=connection.captureAgentSession(key.host,key.target,source,reducer)
            val shown=AgentSessionCreation(key,source,generation,summary.serverId,summary.provider,context)
            agentSessionCreation=shown
            if(!agentSessionAdmitted(shown)){closeAgentSession();return}
            shown.directories=creationBrowser(key.host,context){agentSessionAdmitted(shown)}
            loadAgentSession(shown)
        }catch(_:Exception){notice="New session needs this original conversation and current desktop inventory."}
    }
    private suspend fun retainedAgentSession(shown:AgentSessionCreation):Pair<LocalAction?,Boolean> {
        var after:Uuid?=null;var latest:LocalAction?=null;var pending=false
        val latched=creationActions[shown.key.host]
        do {
            val page=store.localActions(shown.key.host,after,100)
            for(action in page.filter {it.metadata.method==ActionMethod.SessionCreate}){
                if(latest==null || latest.metadata.actionId!=latched && (action.metadata.actionId==latched || action.metadata.createdAt.value>latest.metadata.createdAt.value))latest=action
                if(action.receipt==null||action.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))pending=true
            }
            after=page.lastOrNull()?.metadata?.actionId
        }while(after!=null)
        return latest to pending
    }
    private fun loadAgentSession(shown:AgentSessionCreation){val loading=Any();shown.loading=loading;shown.loaded=false
        viewModelScope.launch {try {
            val (prior,pending)=retainedAgentSession(shown)
            if(agentSessionAdmitted(shown)&&shown.loading===loading){shown.prior=prior;shown.pending=pending;shown.loaded=true}
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(agentSessionCreation===shown){shown.pending=true;shown.loaded=true;notice="Retained creation actions could not be inspected."}}}
    }
    internal fun inspectAgentSession(shown:AgentSessionCreation){if(!agentSessionAdmitted(shown))return
        perform {shown.prior?.let {connection.status(shown.key.host,it.metadata.actionId)};if(agentSessionAdmitted(shown))loadAgentSession(shown)}
    }
    internal fun agentSessionAllowed(shown:AgentSessionCreation):Boolean=agentSessionAdmitted(shown)&&connection.state.value.inventoryReady[shown.key.host]==true&&connection.state.value.mutationReady[shown.key.host]==true&&shown.loaded&&!shown.pending&&shown.submitted==null&&(creationActions[shown.key.host]==null||creationActions[shown.key.host]==shown.prior?.metadata?.actionId)&&(shown.prior==null||shown.separate==shown.prior?.metadata?.actionId)
    internal fun prepareSeparateAgentSession(shown:AgentSessionCreation,action:Uuid){
        val prior=shown.prior ?: return
        if(agentSessionAdmitted(shown)&&!shown.pending&&prior.metadata.actionId==action&&prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))shown.confirming=action
    }
    internal fun confirmSeparateAgentSession(shown:AgentSessionCreation,action:Uuid)=perform {
        if(!agentSessionAdmitted(shown)||shown.confirming!=action||shown.pending)throw StorePolicy(ErrorCode.NotReady)
        val (prior,pending)=retainedAgentSession(shown)
        if(!agentSessionAdmitted(shown)||shown.confirming!=action||pending||prior!=shown.prior||prior?.metadata?.actionId!=action||!prior.authoritative||!prior.outcomeShown||prior.receipt?.state !in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))throw StorePolicy(ErrorCode.NotReady)
        creationActions[shown.key.host]?.let {if(it!=action)throw StorePolicy(ErrorCode.NotReady)}
        creationActions.remove(shown.key.host);shown.confirming=null;shown.separate=action;shown.submitted=null
        shown.memory.clear();shown.memory.text["server"]=shown.server.value;shown.memory.text["provider"]=shown.provider.name
    }
    internal fun createAgentSession(shown:AgentSessionCreation,choice:SessionCreationChoice)=perform {
        if(!agentSessionAllowed(shown))throw StorePolicy(ErrorCode.NotReady)
        val (prior,pending)=retainedAgentSession(shown)
        if(!agentSessionAllowed(shown)||pending||prior!=shown.prior)throw StorePolicy(ErrorCode.NotReady)
        val directory=choice.directory ?: throw StorePolicy(ErrorCode.NotReady)
        if(shown.directories?.context!==directory.context||!shown.directories!!.current())throw StorePolicy(ErrorCode.StaleSession)
        val view=connection.state.value
        if(view.inventory[shown.key.host]?.projects?.none {it.projectId==choice.project&&it.available}!=false)throw StorePolicy(ErrorCode.NotReady)
        val boot=view.hello[shown.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();val previous=creationActions.put(shown.key.host,action);val permit=shown.separate
        shown.submitted=action;shown.separate=null;shown.pending=true;focusedId=action
        try {connection.mutate(shown.key.host,Request(action,RequestBody.SessionCreate(SessionCreateParams(
            boot,choice.server,choice.project,choice.provider,Label(choice.name),directory.reference,choice.placement,
        ))),agentCreation=shown.context,directories=directory.context)}
        catch(error:Exception){if(withContext(NonCancellable){store.localAction(shown.key.host,action)}==null){if(previous==null)creationActions.remove(shown.key.host)else creationActions[shown.key.host]=previous;shown.submitted=null;shown.separate=permit;shown.memory.text.remove("submitted")};throw error}
        finally {if(agentSessionAdmitted(shown))loadAgentSession(shown)}
    }
    internal fun create(choice:SessionCreationChoice) = perform {
        val host = host ?: return@perform; check(foreground)
        val directory=choice.directory ?: throw StorePolicy(ErrorCode.NotReady)
        if(creationDirectoriesBrowser?.context!==directory.context||creationDirectoriesBrowser?.current()!=true||choice.placement!=null)throw StorePolicy(ErrorCode.StaleSession)
        val boot = connection.state.value.hello[host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();creationActions[host]=action;focusedId=action
        try {connection.mutate(host,Request(action,RequestBody.SessionCreate(SessionCreateParams(
            boot,choice.server,choice.project,choice.provider,Label(choice.name),directory.reference,null,
        ))),directories=directory.context)}
        catch(error: Exception) {
            if(withContext(NonCancellable) {store.localAction(host,action)}==null) {creationActions.remove(host);creationForms[host]?.text?.remove("submitted")}
            throw error
        }
    }
    fun prepareSeparateCreation(host: String, action: Uuid) = perform {
        invalidateSeparateCreation()
        val generation=separateCreationGeneration
        separateCreation(host,action,generation,false)
        separateCreationAction = action
    }
    fun dismissSeparateCreation(action: Uuid) {
        if(separateCreationAction==action) invalidateSeparateCreation()
    }
    fun confirmSeparateCreation(host: String, action: Uuid, onConfirmed: () -> Unit) = perform {
        check(separateCreationAction==action)
        val generation=separateCreationGeneration
        separateCreation(host,action,generation,true)
        creationActions.remove(host)
        creationForms.remove(host)?.clear()
        invalidateSeparateCreation()
        onConfirmed()
    }
    private suspend fun separateCreation(host: String, action: Uuid, generation: Long, requireConfirmation: Boolean): LocalAction {
        validateSeparateCreation(host,action,generation,requireConfirmation,null)
        check(!otherPendingCreation(host,action))
        val current=store.localAction(host,action) ?: throw StorePolicy(ErrorCode.NotReady)
        validateSeparateCreation(host,action,generation,requireConfirmation,current)
        return current
    }
    private fun validateSeparateCreation(host: String, action: Uuid, generation: Long, requireConfirmation: Boolean,current: LocalAction?) {
        check(foreground && this.host==host && screen==Screen.Sessions && separateCreationGeneration==generation)
        if(requireConfirmation) check(separateCreationAction==action)
        val displayed=focusedAction?.takeIf {it.metadata.actionId==action} ?: throw StorePolicy(ErrorCode.NotReady)
        if(current!=null) check(current==displayed)
        check(displayed.metadata.method==ActionMethod.SessionCreate && displayed.authoritative && displayed.outcomeShown)
        check(displayed.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))
        creationActions[host]?.let {check(it==action)}
    }
    private suspend fun otherPendingCreation(host: String, excluded: Uuid, validate: ()->Unit = {}): Boolean {
        var after: Uuid?=null
        do {
            val page=store.localActions(host,after,100)
            validate()
            if(page.any {it.metadata.actionId!=excluded && it.metadata.method==ActionMethod.SessionCreate && (it.receipt==null || it.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))}) return true
            after=page.lastOrNull()?.metadata?.actionId
        } while(after!=null)
        return false
    }
    fun inspect(host: String,action: Uuid) = perform {
        focusedId=action
        if(foreground && connection.state.value.hello.containsKey(host) && connection.status(host,action) is ActionStatusResult.Unknown) notice="Host outcome is unknown/expired. Retained historical metadata is inspection only; never replay or retarget this UUID."
        shell?.takeIf {it.selection.host==host && shellAdmitted(it)}?.let {loadShellInput(ShellControlScope(it.selection,it.source,it.ticket))}
        shell?.takeIf {it.selection.host==host&&it.history.action==action&&shellAdmitted(it)&&it.controlBegun}?.let {shown->
            store.localAction(host,action)?.takeIf {it.authoritative&&it.metadata.shellTarget==shown.selection.target}?.receipt?.let {shown.history.receipt(it);terminalOwner(ShellControlScope(shown.selection,shown.source,shown.ticket))?.blockForHistory(shown.history.mode!=ShellHistoryMode.Live)}
        }
    }
    fun displayed(host: String,receipt: Receipt) { viewModelScope.launch {
        try {
            store.outcomeShown(host,receipt.actionId,receipt)
            completeShellInputSnapshot(receipt)
            val identity=RecoveryIdentity(host,receipt.actionId)
            val submitted=sentText[identity]
            if(receipt.state==ReceiptState.Succeeded&&submitted?.options?.images==null) {
                sentText.remove(identity)?.let {snapshot->if(composers[snapshot.key]===snapshot.text&&reservation?.key!=snapshot.key){composers.remove(snapshot.key);composerUploads.remove(snapshot.key);composerDeliveries.remove(snapshot.key)}}
            }else if(receipt.state==ReceiptState.Failed) {
                sentText.remove(identity)
            }
            refreshActions()
            agentSessionCreation?.takeIf {it.key.host==host&&it.prior?.metadata?.actionId==receipt.actionId&&agentSessionAdmitted(it)}?.let {loadAgentSession(it)}
            terminalCreation?.takeIf {it.host==host&&it.prior?.metadata?.actionId==receipt.actionId&&terminalCreationAdmitted(it)}?.let {loadTerminalCreation(it)}
            shell?.takeIf {it.selection.host==host && shellAdmitted(it)}?.let {loadShellInput(ShellControlScope(it.selection,it.source,it.ticket))}
        } catch (error: Exception) { notice = safeFailure(error) }
    } }
    fun discard(host: String,action: Uuid) = perform {
        store.discardRecovery(host,action,true)
        sentText.remove(RecoveryIdentity(host,action))?.let {snapshot->
            if(composers[snapshot.key].orEmpty()==snapshot.text&&composerOptions(snapshot.key)==snapshot.options){composers.remove(snapshot.key);composerUploads.remove(snapshot.key);composerDeliveries.remove(snapshot.key)}
        }
    }
    fun forget(host: String) = perform {
        artifacts?.takeIf{it.host==host}?.let{closeArtifacts()}
        attachmentPickerHold?.takeIf {it.original.key.host==host}?.let {finishAttachmentPicker(it.ticket)}
        connection.forget(host);runtime.forgetDeliveryHost(host);clearHostMemory(host)
        navigate(Screen.Connections,null)
    }
    fun forgetInstallation() = perform {
        finishAttachmentPicker(null)
        connection.forgetInstallation()
        navigate(Screen.Connections,null)
        notice="Unpaired installation identity forgotten locally. No identity is replaced until subsequent explicit foreground initialization or pairing."
    }
    fun unpair(host: String) = perform {
        try { connection.unpair(host);runtime.forgetDeliveryHost(host);clearHostMemory(host);navigate(Screen.Connections,null) }
        catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) { notice = "Self-unpair was not confirmed. Server authority is unknown. Explicit local Forget remains available; use desktop goatr devices / goatr revoke DEVICE_ID to revoke." }
    }
    private fun clearHostMemory(host: String) {
        recentlyOpenedSessions=recentlyOpenedSessions.filter {it.host!=host}
        sessionFilters.forgetHost(host)
        readerPositions.keys.filter {it.host==host}.forEach {readerPositions.remove(it)?.retire()}
        composerCursors.keys.filter {it.host==host}.forEach(composerCursors::remove)
        composerRevisions.keys.filter {it.host==host}.forEach(composerRevisions::remove)
        if(attentionInbox?.host==host) closeAttentionInbox()
        if(shell?.selection?.host==host) closeShell()
        shellInputDrafts.keys.filter {it.host==host}.forEach(shellInputDrafts::remove)
        shellInputActions.keys.filter {it.host==host}.forEach(shellInputActions::remove)
        shellSubmittedInputs.keys.filter {shellSubmittedInputs[it]?.selection?.host==host}.forEach(shellSubmittedInputs::remove)
        terminalCreationActions.remove(host)
        if(terminal?.key?.host==host) closeTerminal()
        if(worktrees?.key?.host==host) closeWorktrees()
        if(skills?.key?.host==host || skillsCatalogue?.key?.host==host) closeSkills()
        if(changes?.key?.host==host) closeChanges()
        composers.keys.filter {it.host==host}.forEach(composers::remove)
        composerUploads.keys.filter {it.host==host}.forEach(composerUploads::remove)
        composerDeliveries.keys.filter {it.host==host}.forEach(composerDeliveries::remove)
        sentText.keys.filter {it.hostId==host}.forEach(sentText::remove)
        creationForms.remove(host)?.clear();creationActions.remove(host);invalidateSeparateCreation()
    }
    internal fun pushMode(host:String):DeliveryMode {deliveryPreferenceRevision;return deliveryPreferences.mode(host)}
    internal fun pushLabel(host:String):String {deliveryPreferenceRevision;return runtime.push.label(host)}
    internal fun ntfyAvailable():Boolean=runtime.push.ntfyInstalled()
    internal fun selectNtfy(host:String,repair:Boolean=false){
        val visible=deliveryVisibleGuard() ?: return
        val paired=store.state.value.hosts.singleOrNull {it.hostId==host} ?: return
        fun active()=visible()&&this.host==host&&store.state.value.hosts.any {it.hostId==host&&it.deviceId==paired.deviceId}
        if(!active())return
        viewModelScope.launch{try{
            runtime.push.selectNtfy(host,repair,::active)
            if(active()){
                runtime.notifications.setHostEnabled(host,true)
                if(!active()||!deliveryPreferences.requiresKeeper(host,runtime.push.verified(host)))return@launch
                requestDeliveryStart(::active)
            }
        }catch(cancel:CancellationException){throw cancel}catch(_:ProtectedKeyUnavailable){notice="Protected push material is unavailable. Restore key access or explicitly Forget this desktop; repair never replaces retained keys."}catch(_:Exception){notice="ntfy registration needs repair. Pairing and the selected delivery mode were preserved."}}
    }
    internal fun selectSocketDelivery(host:String){
        if(!foreground||this.host!=host||screen!=Screen.Settings)return
        viewModelScope.launch{try{runtime.push.selectPersistent(host);notice="Socket delivery selected. Start it while this screen is visible."}catch(cancel:CancellationException){throw cancel}catch(_:Exception){notice="Socket selection could not complete; inspect delivery settings."}}
    }
    fun setPersistentDelivery(host:String,enabled:Boolean) {
        val visible=deliveryVisibleGuard() ?: return
        val device=store.state.value.hosts.singleOrNull{it.hostId==host}?.deviceId ?: return
        viewModelScope.launch {
            try {runtime.notifications.setHostEnabled(host,enabled);if(!enabled&&deliveryPreferences.mode(host)==DeliveryMode.Ntfy)runtime.push.disable(host)else if(enabled&&visible()&&store.state.value.hosts.any{it.hostId==host&&it.deviceId==device}&&deliveryPreferences.mode(host)==DeliveryMode.Ntfy)selectNtfy(host)}
            catch(_:Exception) { notice="Delivery preference could not be applied." }
            if(!deliveryPreferences.anyEnabled()) DeliveryService.stopFromUser(getApplication())
        }
    }
    fun setCompletionDelivery(host:String,enabled:Boolean) {
        viewModelScope.launch {try {runtime.notifications.setCompleted(host,enabled)} catch(_:Exception) {notice="Delivery preference could not be applied."}}
    }
    private fun deliveryVisibleGuard():(()->Boolean)? {
        val generation=foregroundGeneration ?: return null
        if(!foreground||screen!=Screen.Settings)return null
        val navigation=navigationTicket;val selectedHost=host
        return {foreground&&foregroundGeneration===generation&&navigationTicket===navigation&&screen==Screen.Settings&&host==selectedHost}
    }
    private fun requestDeliveryStart(admitted:()->Boolean) {
        if(!admitted())return
        if(!deliveryPreferences.canPostNotifications()) {
            deliveryPreferences.setServiceRequested(true)
            if(permissionRequest?.second?.invoke()==true)return
            notificationPermissionRequest++
            permissionRequest=notificationPermissionRequest to admitted
            return
        }
        DeliveryService.startFromUser(getApplication(),admitted)
    }
    fun startPersistentDelivery() {
        val admitted=deliveryVisibleGuard() ?: return
        if(!deliveryPreferences.anyEnabled()) {notice="Choose at least one paired desktop before starting persistent delivery.";return}
        if(store.state.value.hosts.none{deliveryPreferences.requiresKeeper(it.hostId,runtime.push.verified(it.hostId))}){notice="Selected ntfy registrations need no foreground keeper. Their last/current registration verification is shown separately.";return}
        requestDeliveryStart(admitted)
    }
    fun stopPersistentDelivery() {permissionRequest=null;DeliveryService.stopFromUser(getApplication())}
    internal fun beginNotificationPermissionRequest(request:Long):Boolean {
        val original=permissionRequest ?: return false
        if(notificationPermissionLaunch!=null||original.first!=request||!original.second())return false
        notificationPermissionLaunch=request
        return true
    }
    fun notificationPermissionResult() {
        val launched=notificationPermissionLaunch ?: return
        notificationPermissionLaunch=null
        val original=permissionRequest ?: return
        if(original.first!=launched)return
        permissionRequest=null
        if(original.second()&&deliveryPreferences.canPostNotifications()&&deliveryPreferences.serviceRequested())DeliveryService.startFromUser(getApplication(),original.second)
    }
    fun deliveryEnabled(host:String):Boolean {deliveryPreferenceRevision;return deliveryPreferences.hostEnabled(host)}
    fun deliveryCompleted(host:String):Boolean {deliveryPreferenceRevision;return deliveryPreferences.notifyCompleted(host)}
    fun deliveryRequested():Boolean {deliveryPreferenceRevision;return deliveryPreferences.serviceRequested()}
    fun deliveryLabel(host:String):String {
        deliveryPreferenceRevision
        return when {
            !deliveryPreferences.hostEnabled(host) -> "Off"
            !deliveryPreferences.canPostNotifications() -> "Permission required"
            !deliveryPreferences.requiresKeeper(host,runtime.push.verified(host)) -> "No socket keeper required · ntfy registration"
            !deliveryPreferences.serviceRequested()||!delivery.running -> "Socket keeper stopped"
            DeliveryService.isReconnecting(host)||connection.state.value.hello[host]==null||connection.state.value.inventoryReady[host]!=true -> "Socket keeper reconnecting"
            else -> "Socket keeper connected"
        }
    }
    private fun invalidateSeparateCreation() { separateCreationAction=null;separateCreationGeneration++ }
    suspend fun loadContent(key: ConversationKey,source: Uuid?,reference: ContentRef,completed: Boolean): String {
        if (source == null) return store.offlineContent(key.host,key.target,reference.handle) ?: throw StorePolicy(ErrorCode.ContentExpired)
        if(completed) connection.cachedContent(key.host,key.target,source,reference)?.let {return it}
        val assembly = ContentAssembly(key.target,reference)
        try {
            do { val result = connection.content(key.host,source,assembly.request()); assembly.accept(result) } while (!result.eof)
            val text = assembly.text()
            if (completed) connection.cacheContent(key.host,key.target,source,reference,text)
            return text
        } finally { assembly.clear() }
    }
    fun older(cursor: Id? = null) = perform { selection?.let { connection.history(it.host,it.target,cursor) } }
    suspend fun viewed(key: ConversationKey) {
        if (!foreground || selection != key || screen != Screen.Conversation) return
        val source=connection.state.value.sources[key.host]?.get(key.target) ?: return
        val expected=connection.state.value.hello[key.host] ?: return
        store.viewing(key.host,key.target.sessionId)
        var after: Uint? = null
        do {
            val sync=connection.notificationsSync(key.host,expected,after)
            if (!foreground || selection != key || screen != Screen.Conversation || connection.state.value.sources[key.host]?.get(key.target)!=source) return
            sync.events.lastOrNull {it.sessionId==key.target.sessionId}?.let {event->if(sync.read.none {it.sessionId==event.sessionId&&it.throughSequence.value>=event.sequence.value})connection.markRead(key.host,expected,event.sessionId,event.sequence)}
            after = sync.nextSequence
        } while (after != null)
        // The persisted watermark is authoritative; this only drains already-admitted OS alerts.
        runtime.notifications.reconcile(key.host)
    }
    override fun onCleared() {
        closeAttentionInbox();closeAgentSession();closeTerminalCreation()
        closeSessionControls()
        closeArtifacts();finishAttachmentPicker(null)
        closePairing(); closeTerminal(); closeWorktrees(); closeChanges();closeSkills(); closeShell(); forms.values.forEach(FormMemory::clear); creationForms.values.forEach(FormMemory::clear); composers.clear(); sentText.clear(); lifecycle.close()
        composerCursors.clear();composerRevisions.clear();readerPositions.values.forEach(ConversationReaderPosition::retire);readerPositions.clear();sessionFilters.clearFilters();sessionFilters.query=""
        // Runtime ownership outlives this presentation instance; it may still serve the user-selected service.
        val stale=foregroundGeneration; val staleStartup=startup; foregroundGeneration=null; staleStartup?.cancel()
        if(stale!=null) runtime.cleanupScope.launch {
            try {staleStartup?.join()} catch(_:CancellationException) {}
            try {runtime.release(stale.claim)} catch(_:Exception) {}
        }
    }
    fun terminals(host:String) {navigate(Screen.Terminals,host)}
    private fun shellRetained(snapshot:ShellSnapshot):Boolean {
        val view=connection.state.value;val inventory=view.inventory[snapshot.selection.host] ?: return false
        val graph=inventory.hierarchy.singleOrNull {it.serverId==snapshot.summary.serverId} ?: return false
        return foreground&&screen==Screen.Terminals&&host==snapshot.selection.host&&snapshot.selection.host in view.hello&&
            store.state.value.hosts.any {it.hostId==snapshot.selection.host}&&snapshot.originalHello===view.hello[snapshot.selection.host]&&
            snapshot.incarnation!=null&&graph.serverIncarnation==snapshot.incarnation&&graph.freshness!=NativeFreshness.Unavailable&&
            inventory.servers.singleOrNull {it.serverId==snapshot.summary.serverId}?.connectivity==ServerConnectivity.Online&&
            inventory.shells.any {it.sameLogicalIdentity(snapshot.summary)}
    }
    private fun shellAdmitted(snapshot:ShellSnapshot):Boolean {
        if(!shellRetained(snapshot))return false
        val view=connection.state.value;val inventory=view.inventory[snapshot.selection.host] ?: return false
        return view.inventoryReady[snapshot.selection.host]==true&&
            inventory.hierarchy.singleOrNull {it.serverId==snapshot.summary.serverId}?.freshness==NativeFreshness.Current&&
            inventory.shells.any {it.sameLogicalIdentity(snapshot.summary)&&it.connectivity==SessionConnectivity.Ready&&it.capabilities.read}
    }
    fun openShell(host:String,target:ShellTarget) {
        val view=connection.state.value;val inventory=view.inventory[host] ?: return
        val summary=inventory.shells.singleOrNull {it.target==target} ?: return
        val graph=inventory.hierarchy.singleOrNull {it.serverId==summary.serverId&&it.freshness==NativeFreshness.Current} ?: return
        if(!foreground||screen!=Screen.Terminals||this.host!=host||view.inventoryReady[host]!=true||summary.connectivity!=SessionConnectivity.Ready||!summary.capabilities.read)return
        closeTerminal();closeShell();shellMutationLease=ShellMutationLease()
        terminalInputPause=null
        val selected=ShellSelection(host,target);val source=inventory.subscription;val ticket=shellTicket
        // Opening/dismissing is metadata-only. No native frontend exists before Begin confirmation.
        shell=ShellSnapshot(selected,source,ticket,summary,null,false,false,incarnation=graph.serverIncarnation,originalHello=view.hello[host])
        if(summary.capabilities.input)loadShellInput(ShellControlScope(selected,source,ticket))
    }
    fun shellControlAllowed(scope:ShellControlScope):Boolean =
        shell?.let {it.selection==scope.selection&&it.source==scope.source&&it.ticket==scope.ticket&&shellAdmitted(it)&&it.summary.capabilities.grid}==true
    fun beginShellControl(scope:ShellControlScope) {
        val shown=shell?.takeIf {shellControlAllowed(scope)&&!it.controlBegun} ?: return
        val selected=shown.selection;val source=shown.source;val ticket=shown.ticket
        val currentSource=connection.state.value.inventory[selected.host]?.subscription ?: return
        shell=shown.copy(controlBegun=true,loading=true,error=false)
        shellJob=viewModelScope.launch {
            try {connection.shell(selected.host,selected.target,currentSource){grid,lease->
                val admitted=shell?.let {shellAdmitted(it)&&it.controlBegun}==true
                publishShellGrid(shellState,selected,source,ticket,admitted,lease,grid)
                shell?.takeIf {it.selection==selected&&it.source==source&&it.ticket==ticket&&it.grid===grid&&lease.active()&&(grid==null||admitted)}?.let {
                    terminalOwner(ShellControlScope(selected,source,ticket))?.blockForHistory(it.history.mode!=ShellHistoryMode.Live)
                }
                if(!lease.active())terminalInputOwner?.unavailable()
            }}catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){shell?.takeIf {it.ticket==ticket&&shellRetained(it)}?.let {shell=it.copy(grid=null,loading=false,error=true)}}
        }
    }
    fun releaseShellControl(scope:ShellControlScope) {
        val shown=shell?.takeIf {it.selection==scope.selection&&it.source==scope.source&&it.ticket==scope.ticket} ?: return
        stopTerminalInput()
        shellJob?.cancel();shellJob=null;shellMutationLease?.invalidate();shellMutationLease=ShellMutationLease();shellTicket++
        shell=shown.copy(ticket=shellTicket,grid=null,controlBegun=false,loading=false,error=false,history=ShellHistoryState())
        shellInputDrafts[shown.selection]?.let {shellInputDrafts[shown.selection]=it.copy(modifiers=TerminalModifierState())}
    }
    fun resizeShell(scope:ShellControlScope,columns:Long,rows:Long)=perform {
        if(!shellControlAllowed(scope)||connection.state.value.mutationReady[scope.selection.host]!=true)throw StorePolicy(ErrorCode.NotReady)
        val shown=shell ?: throw StorePolicy(ErrorCode.NotReady)
        if(!shown.controlBegun||shown.grid==null||shown.grid.begin.popupBlocked)throw StorePolicy(ErrorCode.NotReady)
        val lease=shellMutationLease ?: throw StorePolicy(ErrorCode.NotReady)
        val currentSource=connection.state.value.inventory[scope.selection.host]?.subscription ?: throw StorePolicy(ErrorCode.NotReady)
        val original=connection.captureShellMutation(scope.selection.host,scope.selection.target,currentSource,lease,ShellMutationPurpose.Resize)
        if(!shellControlAllowed(scope)||shellMutationLease!==lease)throw StorePolicy(ErrorCode.StaleSession)
        val boot=connection.state.value.hello[scope.selection.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val params=ShellResizeParams(boot,scope.selection.target,original.gridSubscription ?: throw StorePolicy(ErrorCode.NotReady),Uint(columns),Uint(rows));params.validate()
        connection.mutate(scope.selection.host,Request(Uuid.random(),RequestBody.ShellResize(params)),shell=original)
        refreshActions()
    }
    fun shellHistoryAllowed(scope:ShellControlScope):Boolean {
        val shown=shell?.takeIf {it.selection==scope.selection&&it.source==scope.source&&it.ticket==scope.ticket} ?: return false
        val frame=shown.grid?.begin ?: return false
        return shellControlAllowed(scope)&&shown.controlBegun&&!shown.error&&shown.summary.capabilities.input&&shown.originalHello?.features?.shellHistory==true&&shown.history.active()&&shown.history.metrics!=null&&shown.history.mode!=ShellHistoryMode.Pending&&!frame.alternateScreen&&!frame.popupBlocked
    }
    fun scrollShell(scope:ShellControlScope,offset:Long)=perform {
        if(!shellHistoryAllowed(scope)||connection.state.value.mutationReady[scope.selection.host]!=true)throw StorePolicy(ErrorCode.NotReady)
        val shown=shell ?: throw StorePolicy(ErrorCode.NotReady);val history=shown.history
        val lease=shellMutationLease ?: throw StorePolicy(ErrorCode.NotReady);val action=Uuid.random()
        Uint(offset).validate();history.begin(action,offset)
        try {
            terminalOwner(scope)?.historyBarrier()
            if(shell?.history!==history||!shellControlAllowed(scope)||shellMutationLease!==lease)throw StorePolicy(ErrorCode.StaleSession)
            val source=connection.state.value.inventory[scope.selection.host]?.subscription ?: throw StorePolicy(ErrorCode.NotReady)
            val original=connection.captureShellMutation(scope.selection.host,scope.selection.target,source,lease,ShellMutationPurpose.Scroll)
            val boot=shown.originalHello?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
            val receipt=connection.mutate(scope.selection.host,Request(action,RequestBody.ShellScroll(ShellScrollParams(boot,scope.selection.target,original.gridSubscription ?: throw StorePolicy(ErrorCode.NotReady),Uint(offset)))),shell=original)
            if(shell?.history===history&&shellControlAllowed(scope)){history.receipt(receipt);terminalOwner(scope)?.blockForHistory(history.mode!=ShellHistoryMode.Live)}
        }catch(error:Exception){
            if(shell?.history===history){if(withContext(NonCancellable){store.localAction(scope.selection.host,action)}==null)history.refused()else history.uncertain();terminalOwner(scope)?.blockForHistory(history.mode!=ShellHistoryMode.Live)}
            throw error
        }finally{refreshActions()}
    }
    fun closeShell() {
        stopTerminalInput()
        shellInputDrafts.keys.toList().forEach {key->shellInputDrafts[key]?.let {shellInputDrafts[key]=it.copy(modifiers=TerminalModifierState())}}
        clearShellDraftHost=null;shellInputGate=null;shellInputObservedStatus=null;shellInputSeparateStatus=null;shellInputPermitStatus=null
        shellInputObserved=null;separateShellInput=null;shellInputPermit=null;shellInputGeneration++
        shellMutationLease?.invalidate();shellMutationLease=null;shellTicket++;shell=null;shellJob?.cancel();shellJob=null
    }
    fun refreshTerminals() {
        val originalHost=host ?: return
        if(screen!=Screen.Terminals) return
        closeShell()
        perform(::connectionFailureNotice){if(foreground && screen==Screen.Terminals && host==originalHost) connection.snapshot(originalHost)}
    }
    private fun requireShellDraftScope(scope:ShellControlScope):ShellSnapshot {
        val shown=shell?.takeIf {it.selection==scope.selection&&it.source==scope.source&&it.ticket==scope.ticket} ?: throw StorePolicy(ErrorCode.NotReady)
        if(!shellRetained(shown))throw StorePolicy(ErrorCode.NotReady)
        return shown
    }
    private fun requireShellInputOwnerScope(scope:ShellControlScope):ShellSnapshot {
        val shown=requireShellDraftScope(scope)
        if(!shown.controlBegun||shown.error||shellJob?.isActive!=true)throw StorePolicy(ErrorCode.NotReady)
        val inventory=connection.state.value.inventory[scope.selection.host] ?: throw StorePolicy(ErrorCode.StaleSession)
        val graph=inventory.hierarchy.singleOrNull {it.serverId==shown.summary.serverId} ?: throw StorePolicy(ErrorCode.StaleSession)
        val current=inventory.shells.singleOrNull {it.sameLogicalIdentity(shown.summary)} ?: throw StorePolicy(ErrorCode.StaleSession)
        if(graph.freshness==NativeFreshness.Current&&(current.connectivity!=SessionConnectivity.Ready
            ||!current.capabilities.read||!current.capabilities.grid||!current.capabilities.input))throw StorePolicy(ErrorCode.StaleSession)
        return shown
    }
    private fun requireShellInputScope(scope:ShellControlScope):ShellSnapshot {
        val shown=requireShellDraftScope(scope)
        if(!shellAdmitted(shown)||!shown.summary.capabilities.input||!shown.controlBegun||shown.error
            ||shown.grid==null||shown.grid.begin.popupBlocked||connection.state.value.mutationReady[scope.selection.host]!=true)throw StorePolicy(ErrorCode.NotReady)
        return shown
    }
    fun shellDraft(selection:ShellSelection)=shellInputDrafts[selection] ?: ShellInputDraft()
    fun editShellDraft(scope:ShellControlScope,value:androidx.compose.ui.text.input.TextFieldValue) {
        if(runCatching {requireShellDraftScope(scope)}.isFailure)return
        if(scope.selection !in shellInputDrafts&&shellInputDrafts.size>=16){notice="Keep or explicitly clear an existing terminal draft first.";return}
        val size=utf8Size(value.text);val pending=terminalOwner(scope)?.pendingTextBytes ?: 0
        if(size>65536||terminalOwner(scope)!=null&&size+pending>49152){notice="Draft/recovery limit reached. Active input reserves16KiB for one refused commit inside64KiB.";return}
        val old=shellDraft(scope.selection);shellInputDrafts[scope.selection]=old.copy(value=value,revision=old.revision+1)
    }
    fun requestClearShellDrafts(selectedHost:String) {
        if(foreground&&screen==Screen.Terminals&&host==selectedHost&&store.state.value.hosts.any {it.hostId==selectedHost})clearShellDraftHost=selectedHost
    }
    fun cancelClearShellDrafts(){clearShellDraftHost=null}
    fun confirmClearShellDrafts() {
        val selectedHost=clearShellDraftHost ?: return
        if(!foreground||screen!=Screen.Terminals||host!=selectedHost||store.state.value.hosts.none {it.hostId==selectedHost}){clearShellDraftHost=null;return}
        if(terminalInputScope?.selection?.host==selectedHost)stopTerminalInput(discard=true)
        shellInputDrafts.keys.filter {it.host==selectedHost}.forEach(shellInputDrafts::remove)
        shellSubmittedInputs.keys.filter {shellSubmittedInputs[it]?.selection?.host==selectedHost}.forEach(shellSubmittedInputs::remove)
        clearShellDraftHost=null
    }
    fun clearShellDraft(scope:ShellControlScope) {
        if(runCatching {requireShellDraftScope(scope)}.isFailure)return
        if(terminalInputScope==scope)stopTerminalInput(discard=true)
        shellSubmittedInputs.keys.filter {shellSubmittedInputs[it]?.selection==scope.selection}.forEach(shellSubmittedInputs::remove)
        shellInputDrafts.remove(scope.selection)
    }
    fun shellInputMode(scope:ShellControlScope,direct:Boolean) {
        if(runCatching {requireShellDraftScope(scope)}.isFailure)return
        if(scope.selection !in shellInputDrafts&&shellInputDrafts.size>=16){notice="Terminal draft limit reached.";return}
        shellInputDrafts[scope.selection]=shellDraft(scope.selection).copy(direct=direct,modifiers=TerminalModifierState())
    }
    fun shellInputModifier(scope:ShellControlScope,modifier:TerminalModifier,lock:Boolean) {
        if(!shellInputAllowed(scope))return
        val draft=shellDraft(scope.selection);shellInputDrafts[scope.selection]=draft.copy(modifiers=draft.modifiers.toggle(modifier,lock))
    }
    fun shellInputModifierTap(scope:ShellControlScope,modifier:TerminalModifier,at:Long,doubleTapWindow:Long) {
        if(!shellInputAllowed(scope))return
        val draft=shellDraft(scope.selection);shellInputDrafts[scope.selection]=draft.copy(modifiers=draft.modifiers.tap(modifier,at,doubleTapWindow))
    }
    private suspend fun retainedShellInput(scope:ShellControlScope):LocalAction? {
        requireShellDraftScope(scope)
        shellInputActions[scope.selection]?.let {return store.localAction(scope.selection.host,it) ?: throw StorePolicy(ErrorCode.NotReady)}
        var after:Uuid?=null;var prior:LocalAction?=null
        do {val page=store.localActions(scope.selection.host,after,100);requireShellDraftScope(scope)
            for(action in page)if(action.metadata.method==ActionMethod.ShellInput&&action.metadata.shellTarget==scope.selection.target
                &&(prior==null||action.metadata.createdAt.value>prior.metadata.createdAt.value
                    ||action.metadata.createdAt==prior.metadata.createdAt&&action.metadata.actionId.value>prior.metadata.actionId.value))prior=action
            after=if(page.size==100)page.last().metadata.actionId else null
        }while(after!=null)
        return prior
    }
    fun loadShellInput(scope:ShellControlScope) {
        val generation=++shellInputGeneration;shellInputGate=ShellInputGate(scope,false,null,true)
        viewModelScope.launch {try {
            val prior=retainedShellInput(scope);requireShellDraftScope(scope)
            if(generation==shellInputGeneration) {
                val observation=prior?.let {connection.state.value.shellInputStatus[scope.selection.host]?.get(it.metadata.actionId)}
                val pending=prior!=null&&(observation==null||!observation.unknown&&prior.receipt?.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))
                shellInputGate=ShellInputGate(scope,true,prior,pending,observation)
            }
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(generation==shellInputGeneration)shellInputGate=null}}
    }
    private fun inputConsentAllowed(scope:ShellControlScope):Boolean {
        if(runCatching {requireShellInputScope(scope)}.isFailure)return false
        val gate=shellInputGate?.takeIf {it.scope==scope&&it.loaded&&!it.pending} ?: return false
        val prior=gate.prior ?: return shellInputActions[scope.selection]==null
        if(shellInputActions[scope.selection]!=null&&shellInputActions[scope.selection]!=prior.metadata.actionId)return false
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(prior.metadata.actionId) ?: return false
        if(gate.observation!=observation)return false
        val settled=prior.receipt?.shellDispatched()==true||prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed)
        return !observation.unknown&&prior.authoritative&&settled
            ||shellInputPermit==(scope to prior.metadata.actionId)&&shellInputPermitStatus==observation
                &&shellInputObserved==(scope.selection to prior.metadata.actionId)&&shellInputObservedStatus==observation
    }
    internal fun terminalOwner(scope:ShellControlScope):TerminalInputOwner? =
        terminalInputOwner?.takeIf {terminalInputScope==scope&&runCatching {requireShellInputOwnerScope(scope)}.isSuccess}
    fun shellInputAllowed(scope:ShellControlScope)=terminalOwner(scope)?.accepting()==true
    internal fun shellDraftSendAllowed(scope:ShellControlScope):Boolean {
        val draft=shellDraft(scope.selection)
        return shellInputAllowed(scope)&&draft.queuedRevision!=draft.revision&&draft.value.text.isNotEmpty()&&utf8Size(draft.value.text)<=16384
    }
    internal fun prepareTerminalInput(scope:ShellControlScope) {
        if(terminalInputToken!=null||!inputConsentAllowed(scope))return
        val history=shell?.history ?: return
        if(history.mode!=ShellHistoryMode.Live)return
        if(scope.selection !in shellInputDrafts&&shellInputDrafts.size>=16){notice="Keep or explicitly clear an existing terminal draft first.";return}
        if(utf8Size(shellDraft(scope.selection).value.text)>49152){notice="Reduce the draft before direct input;16KiB recovery reserve stays inside64KiB.";return}
        if(scope.selection !in shellInputDrafts)shellInputDrafts[scope.selection]=ShellInputDraft()
        val lease=shellMutationLease ?: return
        val prior=shellInputGate?.prior
        val observation=prior?.let {action->connection.state.value.shellInputStatus[scope.selection.host]?.get(action.metadata.actionId)?.let {action.metadata.actionId to it.token}}
        if(prior!=null&&observation==null)return
        val token=Any();terminalInputToken=token;terminalInputPause=null
        terminalInputPreparation=viewModelScope.launch {try {
            requireShellInputScope(scope)
            val currentSource=connection.state.value.inventory[scope.selection.host]?.subscription ?: throw StorePolicy(ErrorCode.NotReady)
            val original=connection.captureShellMutation(scope.selection.host,scope.selection.target,currentSource,lease,ShellMutationPurpose.Input,observation)
            val captured=requireShellInputScope(scope)
            if(terminalInputToken!==token||shellMutationLease!==lease||captured.history!==history||!inputConsentAllowed(scope))throw StorePolicy(ErrorCode.StaleSession)
            var last:Receipt?=null
            fun retained()=terminalInputToken===token&&shellMutationLease===lease&&lease.active.get()&&runCatching {requireShellInputOwnerScope(scope)}.isSuccess
            fun ready()=retained()&&runCatching {requireShellInputScope(scope)}.isSuccess
            val owner=TerminalInputOwner(viewModelScope,
                current=::retained,ready=::ready,
                awaitReady={combine(connection.state,snapshotFlow {shell}){_,_->Unit}.first {!retained()||ready()}},
                draftBytes={utf8Size(shellDraft(scope.selection).value.text)},
                write={input,revision->sendQueuedShellInput(scope,original,last,input,revision).also {if(it.shellDispatched())last=it}},
                completed={_,receipt->completeShellInputSnapshot(receipt)},
                retain={items->retainUnsentTerminalText(scope.selection,items)},
                stopped={reason->if(terminalInputToken===token){
                    terminalInputToken=null;terminalInputOwner=null;terminalInputScope=null
                    clearShellModifiers(scope);terminalInputPause=reason
                    notice=when(reason){TerminalInputPause.Capacity->"Input paused at its reserved capacity. Known-unsent text is in this original terminal’s draft."
                        TerminalInputPause.Limit->"Input paused. Each committed text/send must fit16KiB; nothing was truncated or sent."
                        TerminalInputPause.Unsupported->"Input paused: this SDK key/composition is not representable. Use an explicit draft/IME gesture."
                        TerminalInputPause.Uncertain->"Input outcome uncertain. Inspect its receipt and retained attempted text; queued keys are not replayed."
                        TerminalInputPause.Refused->"Input was refused. Inspect its receipt before another explicit gesture."
                        TerminalInputPause.SourceLost->"Original input lease lost. Known-unsent text is retained; no reconnect replay."
                        TerminalInputPause.Closed->notice}
                    if(reason!=TerminalInputPause.Closed&&shell?.let {it.selection==scope.selection&&it.source==scope.source&&it.ticket==scope.ticket}==true)releaseShellControl(scope)
                }})
            val current=runCatching {requireShellInputScope(scope)}.getOrNull()
            if(terminalInputToken!==token||shellMutationLease!==lease||current?.history!==history||!inputConsentAllowed(scope)){owner.retire();return@launch}
            owner.blockForHistory(history.mode!=ShellHistoryMode.Live)
            terminalInputScope=scope;terminalInputOwner=owner
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(terminalInputToken===token){
            terminalInputToken=null;terminalInputOwner=null;terminalInputScope=null;terminalInputPause=TerminalInputPause.SourceLost;clearShellModifiers(scope)
        }}}
    }
    private fun stopTerminalInput(discard:Boolean=false) {
        val owner=terminalInputOwner;val scope=terminalInputScope
        val held=owner?.hardwareOutstanding()==true
        terminalInputToken=null;terminalInputOwner=null;terminalInputScope=null
        terminalInputPreparation?.cancel();terminalInputPreparation=null
        owner?.retire(discard)
        if(discard&&held&&scope!=null)releaseShellControl(scope)
    }
    private fun retainUnsentTerminalText(selection:ShellSelection,items:List<TerminalPendingText>) {
        if(items.isEmpty())return
        var draft=shellDraft(selection)
        // An in-flight snapshot is NOT unsent. Move only an unchanged staged snapshot
        // to its existing receipt owner before recovery; never strip a newer edit.
        if(shellSubmittedInputs.values.any {it.selection==selection&&it.revision==draft.revision&&it.text==draft.value.text}) {
            draft=draft.copy(value=androidx.compose.ui.text.input.TextFieldValue(),revision=draft.revision+1)
        }
        val text=StringBuilder(draft.value.text)
        for(item in items)if(item.draftRevision!=draft.revision||item.text!=draft.value.text)text.append(item.text)
        val recovered=text.toString();check(utf8Size(recovered)<=65536) {"terminal recovery reservation exceeded"}
        shellInputDrafts[selection]=draft.copy(value=androidx.compose.ui.text.input.TextFieldValue(recovered),revision=draft.revision+1,direct=false,queuedRevision=null,modifiers=TerminalModifierState())
    }
    private fun completeShellInputSnapshot(receipt:Receipt) {
        if(!receipt.shellDispatched())return
        val submitted=shellSubmittedInputs.remove(receipt.actionId) ?: return
        shellInputDrafts[submitted.selection]?.takeIf {submitted.revision!=null&&it.revision==submitted.revision&&it.value.text==submitted.text}?.let {
            shellInputDrafts[submitted.selection]=it.copy(value=androidx.compose.ui.text.input.TextFieldValue(),revision=it.revision+1,queuedRevision=null)
        }
    }
    internal fun attemptedShellText(selection:ShellSelection):Pair<Uuid,String>? =
        shellSubmittedInputs.entries.lastOrNull {it.value.selection==selection}?.let {it.key to it.value.text}
    fun forgetAttemptedShellText(scope:ShellControlScope,action:Uuid) {
        if(runCatching {requireShellDraftScope(scope)}.isFailure)return
        if(shellSubmittedInputs[action]?.selection==scope.selection)shellSubmittedInputs.remove(action)
    }
    private suspend fun sendQueuedShellInput(scope:ShellControlScope,initial:ShellMutationContext,last:Receipt?,input:ShellInput,revision:Long?):Receipt {
        input.validate()
        val action=Uuid.random();val host=scope.selection.host
        input.terminalTextSnapshot()?.let {shellSubmittedInputs[action]=ShellSubmittedInput(scope.selection,revision,it)}
        val oldAction=shellInputActions[scope.selection];val permit=shellInputPermit;val permitStatus=shellInputPermitStatus
        try {
            val shown=requireShellInputScope(scope)
            val original=if(last==null)initial else {
                if(!last.shellDispatched())throw StorePolicy(ErrorCode.NotReady)
                val observation=connection.state.value.shellInputStatus[host]?.get(last.actionId)?.takeUnless {it.unknown} ?: throw StorePolicy(ErrorCode.NotReady)
                val currentSource=connection.state.value.inventory[host]?.subscription ?: throw StorePolicy(ErrorCode.NotReady)
                connection.captureShellMutation(host,scope.selection.target,currentSource,initial.lease,ShellMutationPurpose.Input,last.actionId to observation.token)
            }
            requireShellInputScope(scope)
            if(original.grid!==initial.grid||original.gridSubscription!=initial.gridSubscription||shellMutationLease!==initial.lease||!initial.lease.active.get())throw StorePolicy(ErrorCode.StaleSession)
            val boot=shown.originalHello?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
            shellInputPermit=null;shellInputPermitStatus=null;shellInputActions[scope.selection]=action
            shellInputGate=ShellInputGate(scope,false,shellInputGate?.prior,true)
            val receipt=connection.mutate(host,Request(action,RequestBody.ShellInput(ShellInputParams(boot,scope.selection.target,
                original.gridSubscription ?: throw StorePolicy(ErrorCode.NotReady),input))),ordinaryComposer=false,shell=original)
            val canonical=store.localAction(host,action)
            if(runCatching {requireShellDraftScope(scope)}.isSuccess){
                val observed=connection.state.value.shellInputStatus[host]?.get(action)
                shellInputGate=ShellInputGate(scope,true,canonical,false,observed)
            }
            return receipt
        }catch(error:Exception){
            if(withContext(NonCancellable){store.localAction(host,action)}==null) {
                if(oldAction==null)shellInputActions.remove(scope.selection)else shellInputActions[scope.selection]=oldAction
                shellInputPermit=permit;shellInputPermitStatus=permitStatus
            }
            throw error
        }
    }
    private fun enqueueShellInput(scope:ShellControlScope,input:ShellInput,revision:Long?=null) {
        val owner=terminalOwner(scope) ?: return
        try {if(owner.offer(input,revision))shellInputDrafts[scope.selection]?.let {
            shellInputDrafts[scope.selection]=it.copy(queuedRevision=if(revision!=null&&it.revision==revision)revision else it.queuedRevision,
                modifiers=if(input is ShellInput.Keys)it.modifiers.afterKey()else TerminalModifierState())
        }}catch(_:ProtocolFailure){owner.limit()}
    }
    fun sendShellCharacter(scope:ShellControlScope){val draft=shellDraft(scope.selection);enqueueShellInput(scope,ShellInput.Keys(listOf(ShellKeyEvent(ShellKey(ShellKeyName.Character,draft.keyCharacter),draft.modifiers.wire(),ShellKeyPhase.Press))))}
    fun sendShellText(scope:ShellControlScope){if(!shellDraftSendAllowed(scope))return;val draft=shellDraft(scope.selection);enqueueShellInput(scope,ShellInput.Text(draft.value.text),draft.revision)}
    fun sendShellKey(scope:ShellControlScope,key:ShellKeyName){val draft=shellDraft(scope.selection);enqueueShellInput(scope,ShellInput.Keys(listOf(ShellKeyEvent(ShellKey(key),draft.modifiers.wire(),ShellKeyPhase.Press))))}
    fun shellKeyCharacter(scope:ShellControlScope,value:String) {
        if(runCatching {requireShellDraftScope(scope)}.isFailure||value.length>2||utf8Size(value)>4)return
        shellInputDrafts[scope.selection]=shellDraft(scope.selection).copy(keyCharacter=value)
    }
    private fun clearShellModifiers(scope:ShellControlScope) {
        val shown=shell ?: return
        if(shown.selection!=scope.selection||shown.source!=scope.source||shown.ticket!=scope.ticket)return
        shellInputDrafts[scope.selection]?.let {shellInputDrafts[scope.selection]=it.copy(modifiers=TerminalModifierState())}
    }
    fun shellInputDisplayed(scope:ShellControlScope,action:LocalAction) {
        if(runCatching {requireShellDraftScope(scope)}.isFailure||shellInputGate?.prior!=action)return
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(action.metadata.actionId) ?: return
        if(shellInputGate?.observation!=observation)return
        shellInputObservedStatus=observation;shellInputObserved=scope.selection to action.metadata.actionId
        if(!observation.unknown&&action.authoritative&&action.receipt!=null&&!action.outcomeShown)displayed(scope.selection.host,action.receipt)
    }
    fun requestSeparateShellInput(scope:ShellControlScope,action:Uuid) {
        if(runCatching {requireShellInputScope(scope)}.isFailure)return
        val gate=shellInputGate?.takeIf {it.scope==scope&&it.loaded} ?: return
        val prior=gate.prior ?: return
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(action) ?: return
        if(prior.metadata.actionId==action&&gate.observation==observation&&shellInputObservedStatus==observation
            &&shellInputObserved==(scope.selection to action)&&(observation.unknown||prior.receipt?.state==ReceiptState.Uncertain)){
            shellInputSeparateStatus=observation;separateShellInput=scope to action
        }
    }
    fun cancelSeparateShellInput(){separateShellInput=null;shellInputSeparateStatus=null}
    fun confirmSeparateShellInput()=perform {
        val (scope,action)=separateShellInput ?: throw StorePolicy(ErrorCode.NotReady);requireShellInputScope(scope)
        val prior=retainedShellInput(scope) ?: throw StorePolicy(ErrorCode.NotReady)
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(action) ?: throw StorePolicy(ErrorCode.NotReady)
        if(separateShellInput!=(scope to action)||prior.metadata.actionId!=action||shellInputGate?.prior!=prior||shellInputGate?.observation!=observation
            ||shellInputSeparateStatus!=observation||shellInputObservedStatus!=observation||shellInputObserved!=(scope.selection to action)
            ||!observation.unknown&&prior.receipt?.state!=ReceiptState.Uncertain)throw StorePolicy(ErrorCode.NotReady)
        shellInputPermitStatus=observation;shellInputPermit=scope to action;separateShellInput=null;shellInputSeparateStatus=null
        shellInputActions[scope.selection]=action;loadShellInput(scope)
    }

    private companion object {
        const val PRESENTATION_PREFERENCES = "local_presentation"
        const val APPEARANCE_KEY = "appearance"
        const val SESSION_DIRECTORY_VIEW_KEY = "session_directory_view"
        const val SESSION_LIST_GROUP_BY_STATUS_KEY = "session_list_group_by_status"
        const val SESSION_LIST_SHOW_WORKSPACE_CONTEXT_KEY = "session_list_show_workspace_context"
        const val SESSION_LIST_SHOW_HOST_CONTEXT_KEY = "session_list_show_host_context"
    }
}

internal fun safeFailure(error: Exception): String = when(error) {
    is ProtectedKeyUnavailable -> "Protected identity is inaccessible. No replacement identity was created. Restore access or explicitly Forget."
    is RemoteFailure -> "Host operation: ${error.code}. No automatic retry or replay."
    is StorePolicy -> "Operation unavailable: ${error.code}. Inspect receipts; no automatic replay."
    is RecoveryApprovalRequired -> "Recovery capacity changed. Review a new action-specific replacement warning before sending."
    else -> "Operation did not complete. Delivery may be unknown; inspect receipts. No automatic replay."
}

/** Only connection/read reconciliation callers use this; native action uncertainty stays in safeFailure. */
internal fun connectionFailureNotice(error: Exception): String = when(error) {
    is ProtectedKeyUnavailable -> safeFailure(error)
    is RemoteFailure -> "Desktop connection or synchronization was refused: ${error.code}. Reconnect from Hosts to try again."
    is StorePolicy -> "Desktop connection or synchronization is unavailable: ${error.code}. Reconnect from Hosts to try again."
    else -> "Desktop connection or synchronization did not complete. Reconnect from Hosts to try again."
}
