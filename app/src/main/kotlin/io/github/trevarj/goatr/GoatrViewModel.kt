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

data class ConversationKey(val host: String, val target: Target)
internal data class AttentionInboxState(val host:String,val ticket:Long,val entries:List<StoredAttentionEvent> = emptyList(),val loading:Boolean=true,val error:Boolean=false)
internal data class AgentAliasScope(val key:ConversationKey,val source:Uuid,val lease:AgentAliasLease)
data class TerminalSnapshot(val key: ConversationKey,val source: Uuid,val ticket: Long,val text: String?,val truncated: Boolean,val loading: Boolean,val error: Boolean)
data class WorktreesSnapshot(val key: ConversationKey,val source: Uuid,val ticket: Long,val worktrees: List<WorktreeEntry>?,val loading: Boolean,val error: Boolean,val creationScope: WorktreeCreationScope? = null)
internal data class WorktreeRemovalState(val key:ConversationKey,val source:Uuid,val ticket:Long,val candidate:Uuid,val lease:WorktreeRemovalLease,val context:WorktreeRemovalMutationContext?=null,val loading:Boolean=true,val error:Boolean=false,val submittedAction:Uuid?=null)
data class WorktreeCreationRequestScope(val key: ConversationKey,val source: Uuid,val ticket: Long)
data class WorktreeCreationGate(val scope: WorktreeCreationRequestScope,val loaded: Boolean,val prior: LocalAction?,val pending: Boolean)
data class SkillsSnapshot(val key:ConversationKey,val source:Uuid,val generation:Uuid,val ticket:Long,val catalogue:SkillsListResult?,val loading:Boolean,val error:Boolean)
data class SkillsDocumentState(val snapshot:SkillsSnapshot,val entry:SkillEntry,val ticket:Any,val loading:Boolean=true,val text:String?=null,val failed:Boolean=false)
data class ChangesSnapshot(val key: ConversationKey,val source: Uuid,val ticket: Long,val changes: ChangesReadResult?,val loading: Boolean,val error: Boolean)
data class ShellSelection(val host:String,val target:ShellTarget)
data class ShellSnapshot(val selection:ShellSelection,val source:Uuid,val ticket:Long,val summary:ShellSummary,val text:String?,val truncated:Boolean,val loading:Boolean,val error:Boolean)
data class ShellInputDraft(val value:androidx.compose.ui.text.input.TextFieldValue=androidx.compose.ui.text.input.TextFieldValue(),val revision:Long=0,val keys:Boolean=false,val keyCharacter:String="",val modifiers:ShellKeyModifiers=ShellKeyModifiers(false,false,false,false))
data class ShellInputGate(val scope:ShellCreateScope,val loaded:Boolean,val prior:LocalAction?,val pending:Boolean,val observation:ShellInputObservation?=null)
data class ShellCreateScope(val selection:ShellSelection,val source:Uuid,val ticket:Long)
data class ShellCreationGate(val scope:ShellCreateScope,val loaded:Boolean,val prior:LocalAction?,val pending:Boolean)
enum class Screen { Connections, Pairing, Sessions, Conversation, Terminals, Settings }
data class SendReservation(val key: ConversationKey, val action: Uuid, val text: String, val warning: RecoveryWarning)
private data class ForegroundGeneration(val claim: RuntimeClaim, val ready: CompletableDeferred<Unit>)

/** Local-only list presentation; it never carries session, host, or receipt state. */
data class SessionListPreferences(
    val groupByStatus: Boolean = true,
    val showWorkspaceContext: Boolean = true,
    val showHostContext: Boolean = true,
)

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
    private var deliveryPreferenceRevision by mutableStateOf(0L)
    private var permissionRequestVisible=false
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
    var shell by mutableStateOf<ShellSnapshot?>(null)
        private set
    val shellInputDrafts=mutableStateMapOf<ShellSelection,ShellInputDraft>()
    var shellInputGate by mutableStateOf<ShellInputGate?>(null)
        private set
    var separateShellInput by mutableStateOf<Pair<ShellCreateScope,Uuid>?>(null)
        private set
    private val shellInputActions=mutableMapOf<ShellSelection,Uuid>()
    private val shellSubmittedDrafts=mutableMapOf<Uuid,Pair<ShellSelection,Long>>()
    var clearShellDraftHost by mutableStateOf<String?>(null)
        private set
    private var shellInputObservedStatus:ShellInputObservation?=null
    private var shellInputSeparateStatus:ShellInputObservation?=null
    private var shellInputPermitStatus:ShellInputObservation?=null
    private var shellInputObserved:Pair<ShellSelection,Uuid>?=null
    private var shellInputPermit:Pair<ShellCreateScope,Uuid>?=null
    private var shellInputGeneration=0L
    var shellCreationGate by mutableStateOf<ShellCreationGate?>(null)
        private set
    var separateShellCreation by mutableStateOf<Pair<ShellCreateScope,Uuid>?>(null)
        private set
    private var shellJob:Job?=null
    private var shellMutationLease:ShellMutationLease?=null
    private var shellTicket=0L
    private var shellGateGeneration=0L
    private val shellCreationActions=mutableMapOf<String,Uuid>()
    private var shellSeparatePermit:Pair<ShellCreateScope,Uuid>?=null
    var terminal by mutableStateOf<TerminalSnapshot?>(null)
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
    private var observedHello: Map<String,HelloResult> = emptyMap()
    private var observedReceipts: Map<String,Receipt> = emptyMap()
    val composers = mutableStateMapOf<ConversationKey,String>()
    val forms = mutableMapOf<Triple<String,Target,Uuid>,FormMemory>()
    val creationForms = mutableMapOf<String,FormMemory>()
    val creationActions = mutableStateMapOf<String,Uuid>()
    // This is only a presentation confirmation for one exact retained action.
    var separateCreationAction by mutableStateOf<Uuid?>(null)
        private set
    private var separateCreationGeneration=0L
    var separateWorktreeScope by mutableStateOf<WorktreeCreationRequestScope?>(null)
        private set
    var worktreeCreationGate by mutableStateOf<WorktreeCreationGate?>(null)
        private set
    private var worktreeGateGeneration=0L
    private var worktreeSeparatePermit: Pair<WorktreeCreationRequestScope,Uuid>? = null
    val connectingHosts = mutableStateMapOf<String,Boolean>()
    // Presentation preferences only; they never persist drafts or change host/session state.
    private val presentationPreferences = application.getSharedPreferences(PRESENTATION_PREFERENCES, Context.MODE_PRIVATE)
    private var appearanceState by mutableStateOf(loadAppearance())
    private var sessionListPreferencesState by mutableStateOf(loadSessionListPreferences())
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
                .putBoolean(SESSION_LIST_GROUP_BY_STATUS_KEY, value.groupByStatus)
                .putBoolean(SESSION_LIST_SHOW_WORKSPACE_CONTEXT_KEY, value.showWorkspaceContext)
                .putBoolean(SESSION_LIST_SHOW_HOST_CONTEXT_KEY, value.showHostContext)
                .apply()
        }
    private val sentText = mutableMapOf<RecoveryIdentity,Pair<ConversationKey,String>>()
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
    private val composerCursors=mutableMapOf<ConversationKey,Pair<Int,Int>>()
    private var worktreeRemovalJob:Job?=null
    internal var worktreeRemoval by mutableStateOf<WorktreeRemovalState?>(null)
        private set
    private var worktreesJob: Job? = null
    private var worktreesTicket=0L
    private var changesJob: Job? = null
    private var changesTicket=0L
    private var lifecycleActive = false
    /** Replaced atomically on every foreground entry; stale cleanup cannot release a new entry. */
    private var foregroundGeneration: ForegroundGeneration?=null
    private var pendingAttention:Pair<String,String>?=null
    internal var attentionInbox by mutableStateOf<AttentionInboxState?>(null)
        private set
    private var attentionInboxForeground:ForegroundGeneration?=null
    private var attentionInboxTicket=0L
    private var attentionInboxJob:Job?=null


    internal var agentSessionCreation by mutableStateOf<AgentSessionCreation?>(null)
        private set
    internal var agentTerminalCreation by mutableStateOf<AgentTerminalCreation?>(null)
        private set

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
                agentSessionCreation?.let {if(!agentSessionAdmitted(it))closeAgentSession()}
                agentTerminalCreation?.let {if(!agentTerminalAdmitted(it))closeAgentTerminal()}
                agentAliasScope?.let {shown->if(!foreground || selection!=shown.key || view.sources[shown.key.host]?.get(shown.key.target)!=shown.source)closeAgentAlias()}
                if(current!=null && (!foreground || screen!=Screen.Conversation || view.sources[current.host]?.containsKey(current.target)!=true)) runtime.setVisibleConversation(null)
                val retained = forms.keys.toList()
                for (key in retained) {
                    val request = view.sessions[key.first]?.firstOrNull { it.state?.summary?.sessionId == key.second.sessionId }
                        ?.takeIf { it.state?.summary?.generation == key.second.generation }?.pending
                        ?.firstOrNull { it.requestKey == key.third }
                    if (request == null || current != ConversationKey(key.first,key.second) || forms[key]?.source != view.sources[key.first]?.get(key.second)) forms.remove(key)?.clear()
                }
                shell?.let {shown->if(!shellAdmitted(shown)) closeShell() else shellInputGate?.takeIf {it.loaded && it.prior!=null && it.observation!=view.shellInputStatus[shown.selection.host]?.get(it.prior.metadata.actionId)}?.let {loadShellInput(it.scope)}}
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
                    shell?.takeIf {shellAdmitted(it)}?.let {loadShellInput(ShellCreateScope(it.selection,it.source,it.ticket))}
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
                var after:Uint?=null
                do{if(!foreground||foregroundGeneration!==attentionForeground)break;after=connection.notificationsSync(wakeup.host,after).nextSequence}while(after!=null)
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
        if(active && foregroundGeneration==null) foregroundGeneration=ForegroundGeneration(runtime.newClaim(RuntimeClaimKind.Ui),CompletableDeferred())
        foreground = active
        if (!active) {
            closeAttentionInbox()
            // Admission guards change immediately, before queued native cleanup finishes.
            foreground = false
            pauseArtifacts();closeAgentAlias();closeAgentTerminal();closeAgentSession();attachmentCleanup?.lease?.invalidate();attachmentCleanup=null;attachmentRecovery=null
            val stale=foregroundGeneration
            val staleStartup=startup
            foregroundGeneration=null
            staleStartup?.cancel();operation?.cancel();closePairing();closeTerminal();closeShell();closeWorktrees();closeChanges();closeSkills();forms.values.forEach(FormMemory::clearSecrets);invalidateSeparateCreation()
            runtime.setVisibleConversation(null)
            permissionRequestVisible=false
            if(stale!=null) runtime.cleanupScope.launch {
                try {staleStartup?.join()} catch(_:CancellationException) {}
                try {runtime.release(stale.claim)} catch(_:Exception) {}
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
        groupByStatus = presentationBoolean(SESSION_LIST_GROUP_BY_STATUS_KEY),
        showWorkspaceContext = presentationBoolean(SESSION_LIST_SHOW_WORKSPACE_CONTEXT_KEY),
        showHostContext = presentationBoolean(SESSION_LIST_SHOW_HOST_CONTEXT_KEY),
    )
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
        operation = viewModelScope.launch {
            try { block() }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { notice = failureNotice(error) }
            finally { busy = false; refreshActions() }
        }
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
            // Receipt refresh also refreshes the sheet's retained outcome, without clearing its latch.
            val worktreeScope=worktreeCreationGate?.scope
            if(worktreeScope!=null && worktreeScope.key.host==currentHost && runCatching {requireWorktreeCreationScope(worktreeScope)}.isSuccess) loadWorktreeCreation(worktreeScope)
            shell?.takeIf {shellAdmitted(it)}?.let {loadShellCreation(ShellCreateScope(it.selection,it.source,it.ticket))}
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
    private fun navigate(next: Screen, nextHost: String?) {
        closeAttentionInbox()
        closeArtifacts()
        closeAgentAlias()
        val old = selection
        skillsFocusSignal=0L
        closeAgentSession();closeAgentTerminal();closePairing(); closeTerminal(); closeShell(); closeWorktrees(); closeChanges();closeSkills(); forms.values.forEach(FormMemory::clear); forms.clear()
        selection = null; reservation = null; replacement = null; warningJob?.cancel(); invalidateSeparateCreation()
        runtime.setVisibleConversation(null)
        host = nextHost; screen = next
        actions=emptyMap();focusedAction=null;focusedId=null;actionAfter=null;nextActionPage=null
        if(nextHost!=null) viewModelScope.launch {try {refreshActions()} catch(error: Exception) {notice=safeFailure(error)}}
        if (old != null) viewModelScope.launch {
            try { connection.leave(old.host,old.target) }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { notice = safeFailure(error) }
        }
    }
    fun open(host: String,target: Target) {
        val selected=ConversationKey(host,target)
        val previous=operation
        navigate(Screen.Conversation,host); selection = selected
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
        pendingAttention=host to eventId
        if(foreground) openPendingAttention()
    }
    private fun openPendingAttention() {
        val entry=pendingAttention ?: return
        pendingAttention=null
        perform {
            val host=entry.first
            val event=Uuid(entry.second)
            EndpointId(host)
            val generation=foregroundGeneration ?: return@perform
            generation.ready.await()
            try {
                connection.ensureConnectedIfNeeded(host)
                val inventory=connection.snapshot(host)
                var after:Uint?=null
                do {after=connection.notificationsSync(host,after).nextSequence} while(after!=null)
                val retained=store.notificationEvent(host,event) ?: throw StorePolicy(ErrorCode.NotFound)
                val target=Target(retained.event.sessionId,retained.event.generation)
                val current=inventory.sessions.firstOrNull {Target(it.sessionId,it.generation)==target && it.connectivity==SessionConnectivity.Ready}
                if(current==null || connection.state.value.inventoryReady[host]!=true) {showStaleAttention(host,event);return@perform}
                connection.open(host,target)
                navigate(Screen.Conversation,host);selection=ConversationKey(host,target);runtime.setVisibleConversation(selection)
            } catch(cancel:CancellationException) {throw cancel}
            catch(_:Exception) {showStaleAttention(host,event)}
        }
    }
    private suspend fun showStaleAttention(host:String,event:Uuid) {
        val retained=try {store.notificationEvent(host,event)} catch(_:Exception) {null} ?: run {notice="This notification is no longer retained on this phone.";return}
        val target=Target(retained.event.sessionId,retained.event.generation)
        // Never select a replacement generation from an old OS alert. Cached history remains read-only.
        try {connection.leave(host,target)} catch(_:Exception) {}
        navigate(Screen.Conversation,host);selection=ConversationKey(host,target)
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
    fun openTerminal(key: ConversationKey) {
        val source=connection.state.value.sources[key.host]?.get(key.target) ?: return
        val summary=connection.state.value.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}?.state?.summary ?: return
        if(!foreground || selection!=key || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.terminalRead) return
        closeTerminal()
        val ticket=terminalTicket
        terminal=TerminalSnapshot(key,source,ticket,null,false,true,false)
        terminalJob=viewModelScope.launch {
            try {
                val result=connection.terminal(key.host,key.target,source)
                if(terminal?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) terminal=TerminalSnapshot(key,source,ticket,result.text,result.truncated,false,false)
            } catch(timeout: TimeoutCancellationException) {
                if(terminal?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) terminal=TerminalSnapshot(key,source,ticket,null,false,false,true)
            } catch(cancel: CancellationException) {throw cancel}
            catch(_: Exception) {if(terminal?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) terminal=TerminalSnapshot(key,source,ticket,null,false,false,true)}
        }
    }
    fun closeTerminal() { terminalTicket+=1;terminal=null;terminalJob?.cancel();terminalJob=null }
    fun refreshTerminal() { terminal?.key?.let(::openTerminal) }
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
                if(worktrees?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) worktrees=WorktreesSnapshot(key,source,ticket,result.worktrees,false,false,result.creationScope)
            } catch(timeout: TimeoutCancellationException) {
                if(worktrees?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) worktrees=WorktreesSnapshot(key,source,ticket,null,false,true)
            } catch(cancel: CancellationException) {throw cancel}
            catch(_: Exception) {if(worktrees?.ticket==ticket && foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[key.host]?.get(key.target)==source) worktrees=WorktreesSnapshot(key,source,ticket,null,false,true)}
        }
    }
    fun closeWorktrees() { closeAttachments(); closeWorktreeRemoval();worktreesTicket+=1;worktrees=null;worktreesJob?.cancel();worktreesJob=null }
    fun refreshWorktrees() { worktrees?.key?.let(::openWorktrees) }
    internal fun closeWorktreeRemoval(){worktreeRemoval?.lease?.invalidate();worktreeRemoval=null;worktreeRemovalJob?.cancel();worktreeRemovalJob=null}
    private fun requireWorktreeRemoval(scope:WorktreeRemovalState) {
        val snapshot=worktrees ?: throw StorePolicy(ErrorCode.StaleSession)
        val view=connection.state.value
        val summary=view.sessions[scope.key.host]?.firstOrNull {it.state?.summary?.sessionId==scope.key.target.sessionId}?.state?.summary
        if(worktreeRemoval!==scope || !foreground || selection!=scope.key || screen!=Screen.Conversation || snapshot.key!=scope.key || snapshot.source!=scope.source || snapshot.ticket!=scope.ticket || snapshot.loading || snapshot.error || view.sources[scope.key.host]?.get(scope.key.target)!=scope.source || view.inventoryReady[scope.key.host]!=true || summary?.generation!=scope.key.target.generation || summary.capabilities.worktreeRemove!=true || snapshot.worktrees?.none {it.candidateId==scope.candidate}!=false)throw StorePolicy(ErrorCode.StaleSession)
    }
    internal fun previewWorktreeRemoval(snapshot:WorktreesSnapshot,entry:WorktreeEntry) {
        if(busy || worktrees!==snapshot)return
        val candidate=entry.candidateId ?: return
        closeWorktreeRemoval()
        val scope=WorktreeRemovalState(snapshot.key,snapshot.source,snapshot.ticket,candidate,WorktreeRemovalLease())
        worktreeRemoval=scope
        if(runCatching {requireWorktreeRemoval(scope)}.isFailure){closeWorktreeRemoval();return}
        worktreeRemovalJob=viewModelScope.launch {
            try {
                val context=connection.previewWorktreeRemoval(scope.key.host,scope.key.target,scope.source,candidate,scope.ticket,scope.lease)
                requireWorktreeRemoval(scope)
                worktreeRemoval=scope.copy(context=context,loading=false)
                delay(25000)
                worktreeRemoval?.takeIf {it.lease===scope.lease && it.submittedAction==null}?.let {closeWorktreeRemoval();notice="Removal preview expired. Open a fresh preview."}
            }catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){if(worktreeRemoval===scope)worktreeRemoval=scope.copy(loading=false,error=true)}
        }
    }
    internal fun confirmWorktreeRemoval(scope:WorktreeRemovalState)=perform {
        requireWorktreeRemoval(scope)
        val context=scope.context ?: throw StorePolicy(ErrorCode.NotReady)
        if(scope.submittedAction!=null || !context.preview.removable || context.ticket!=scope.ticket)throw StorePolicy(ErrorCode.NotReady)
        val boot=connection.state.value.hello[scope.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();focusedId=action
        worktreeRemoval=scope.copy(submittedAction=action)
        connection.mutate(scope.key.host,Request(action,RequestBody.WorktreesRemove(WorktreesRemoveParams(boot,scope.key.target,scope.candidate,context.preview.previewId))),removal=context)
    }
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
    fun closeSkills() {closeSkillsDocument();skillsTicket+=1;skills=null;skillsCatalogue=null;skillsJob?.cancel();skillsJob=null}
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
        if(skills!==snapshot||snapshot.loading||snapshot.error||!skillsAdmitted(snapshot)||catalogue.provider!=Provider.Omp||entry !in catalogue.skills)return
        val document=entry.documentRef ?: return;val foreground=foregroundGeneration ?: return;closeSkillsDocument()
        val shown=SkillsDocumentState(snapshot,entry,Any());skillsDocument=shown
        skillsDocumentJob=viewModelScope.launch{
            try{val text=withTimeout(90000){connection.skillsDocument(snapshot.key.host,snapshot.key.target,snapshot.source,snapshot.generation,document)}
                if(skillsDocument?.ticket===shown.ticket&&skills===snapshot&&skillsAdmitted(snapshot)&&foregroundGeneration===foreground)skillsDocument=shown.copy(loading=false,text=text)
            }catch(_:TimeoutCancellationException){if(skillsDocument?.ticket===shown.ticket&&skills===snapshot&&skillsAdmitted(snapshot)&&foregroundGeneration===foreground)skillsDocument=shown.copy(loading=false,failed=true)}
            catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(skillsDocument?.ticket===shown.ticket&&skills===snapshot&&skillsAdmitted(snapshot)&&foregroundGeneration===foreground)skillsDocument=shown.copy(loading=false,failed=true)}
        }
    }

    fun insertSkill(snapshot:SkillsSnapshot,entry:SkillEntry):Boolean {
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
                val entries=if(append)(shown.entries+page.entries).distinctBy{it.reference}else page.entries
                if(entries.size>1000)throw StorePolicy(ErrorCode.ResourceLimit)
                shown.entries=entries;shown.cursor=page.nextCursor;shown.breadcrumbs=breadcrumbs
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
            if(opened.previewKind!=ArtifactPreviewKind.Text||opened.size.value>1024L*1024||opened.size!=entry.size||opened.name!=entry.name)throw StorePolicy(ErrorCode.InvalidRequest)
            val buffer=java.io.ByteArrayOutputStream(opened.size.value.toInt());consumeArtifact(shown,opened){bytes->buffer.write(bytes)}
            val decoded=Charsets.UTF_8.newDecoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT).onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT).decode(java.nio.ByteBuffer.wrap(buffer.toByteArray())).toString()
            if(decoded.any{Character.isISOControl(it)&&it!='\t'&&it!='\r'&&it!='\n'})throw StorePolicy(ErrorCode.InvalidRequest)
            if(artifactAdmitted(shown)&&shown.selected===entry)shown.preview=decoded
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
    private val agentTerminalActions=mutableMapOf<ConversationKey,Uuid>()
    internal fun closeAgentTerminal(){agentTerminalCreation?.context?.lease?.invalidate();agentTerminalCreation=null}
    private fun agentTerminalAdmitted(shown:AgentTerminalCreation):Boolean {
        val view=connection.state.value
        val summary=view.sessions[shown.key.host]?.firstOrNull {it.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==shown.key.target}==true}?.state?.summary
        return agentTerminalCreation===shown&&foreground&&foregroundGeneration===shown.foreground&&screen==Screen.Conversation&&selection==shown.key&&view.sources[shown.key.host]?.get(shown.key.target)==shown.source&&view.inventoryReady[shown.key.host]==true&&view.mutationReady[shown.key.host]==true&&summary?.capabilities?.terminalCreate==true
    }
    internal fun beginAgentTerminal(key:ConversationKey){
        closeAgentTerminal();val generation=foregroundGeneration ?: return
        val view=connection.state.value;val source=view.sources[key.host]?.get(key.target) ?: return
        val reducer=view.sessions[key.host]?.firstOrNull {it.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==key.target}==true} ?: return
        if(!foreground||screen!=Screen.Conversation||selection!=key||view.inventoryReady[key.host]!=true||view.mutationReady[key.host]!=true||reducer.state?.summary?.capabilities?.terminalCreate!=true)return
        try {val context=connection.captureAgentTerminal(key.host,key.target,source,reducer)
            val shown=AgentTerminalCreation(key,source,generation,context);agentTerminalCreation=shown;loadAgentTerminal(shown)
        }catch(_:Exception){notice="Agent process directory creation is unavailable for this original session."}
    }
    private suspend fun retainedAgentTerminals(key:ConversationKey):Pair<LocalAction?,Boolean>{
        var after:Uuid?=null;var latest:LocalAction?=null;var pending=false
        do {val page=store.localActions(key.host,after,100)
            for(action in page.filter {it.metadata.method==ActionMethod.TerminalCreate&&it.metadata.target==key.target}){
                if(latest==null||action.metadata.createdAt.value>latest.metadata.createdAt.value)latest=action
                if(action.receipt==null||action.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))pending=true
            };after=page.lastOrNull()?.metadata?.actionId
        }while(after!=null)
        return latest to pending
    }
    private fun loadAgentTerminal(shown:AgentTerminalCreation){val loading=Any();shown.loading=loading;shown.loaded=false;viewModelScope.launch {
        try {val (prior,pending)=retainedAgentTerminals(shown.key)
            if(!agentTerminalAdmitted(shown)||shown.loading!==loading)return@launch
            shown.prior=prior;shown.pending=pending;shown.loaded=true
            if(prior?.metadata?.actionId!=shown.separate)shown.separate=null
        }catch(cancel:CancellationException){throw cancel}catch(_:Exception){if(agentTerminalCreation===shown){shown.pending=true;notice="Retained terminal actions could not be inspected."}}
    }}
    internal fun inspectAgentTerminal(shown:AgentTerminalCreation){if(!agentTerminalAdmitted(shown))return;viewModelScope.launch {
        try {shown.prior?.metadata?.actionId?.let {action->if(connection.status(shown.key.host,action) is ActionStatusResult.Unknown)notice="The original terminal action is unknown. It is never retried; another terminal requires separate explicit consent."};if(agentTerminalAdmitted(shown))loadAgentTerminal(shown)}catch(cancel:CancellationException){throw cancel}catch(_:Exception){notice="Terminal receipt is unavailable. No creation was retried."}
    }}
    internal fun permitSeparateAgentTerminal(shown:AgentTerminalCreation,action:Uuid){if(!agentTerminalAdmitted(shown)||shown.pending)return
        val prior=shown.prior ?: return
        if(prior.metadata.actionId==action&&prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))shown.separate=action
    }
    internal fun agentTerminalAllowed(shown:AgentTerminalCreation):Boolean=agentTerminalAdmitted(shown)&&shown.loaded&&!shown.pending&&(shown.prior==null||shown.separate==shown.prior?.metadata?.actionId)&&(agentTerminalActions[shown.key]==null||agentTerminalActions[shown.key]==shown.prior?.metadata?.actionId)
    internal fun agentTerminalCreatedAvailable(shown:AgentTerminalCreation):Boolean {
        if(!agentTerminalAdmitted(shown))return false
        val receipt=shown.prior?.takeIf {it.authoritative}?.receipt?.takeIf {it.state==ReceiptState.Succeeded&&it.target==shown.key.target} ?: return false
        val resources=(receipt.result as? ActionResultShellCreation)?.resources ?: return false
        val target=resources.createdTarget ?: return false
        val inventory=connection.state.value.inventory[shown.key.host] ?: return false
        return connection.state.value.inventoryReady[shown.key.host]==true&&inventory.shells.any {it.target==target&&it.connectivity==SessionConnectivity.Ready&&it.serverId==resources.serverId&&it.workspaceId==resources.workspaceId&&it.tabId==resources.tabId&&it.paneId==resources.paneId&&it.terminalId==resources.terminalId}
    }
    internal fun openCreatedAgentTerminal(shown:AgentTerminalCreation){if(!agentTerminalCreatedAvailable(shown))return
        val target=(shown.prior?.receipt?.result as? ActionResultShellCreation)?.resources?.createdTarget ?: return
        val host=shown.key.host;closeAgentTerminal();openShell(host,target)
    }
    internal fun createAgentTerminal(shown:AgentTerminalCreation)=perform {
        if(!agentTerminalAllowed(shown))throw StorePolicy(ErrorCode.NotReady)
        val (prior,pending)=retainedAgentTerminals(shown.key)
        if(!agentTerminalAllowed(shown)||pending||prior!=shown.prior)throw StorePolicy(ErrorCode.NotReady)
        val boot=connection.state.value.hello[shown.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();val previous=agentTerminalActions.put(shown.key,action);val permit=shown.separate;shown.separate=null;shown.pending=true;focusedId=action
        try {connection.mutate(shown.key.host,Request(action,RequestBody.TerminalCreate(TerminalCreateParams(boot,shown.key.target))),terminal=shown.context)}
        catch(error:Exception){if(withContext(NonCancellable){store.localAction(shown.key.host,action)}==null){if(previous==null)agentTerminalActions.remove(shown.key)else agentTerminalActions[shown.key]=previous;shown.separate=permit};throw error}
        finally {if(agentTerminalAdmitted(shown))loadAgentTerminal(shown)}
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
    private fun pauseAttachments(){attachmentTransfer?.let {it.paused=true;it.original.context.lease.invalidate();it.status=it.status?.copy(insertionReady=false)};attachmentPickerConsent?.takeUnless {attachmentPickerHold?.original===it}?.context?.lease?.invalidate()}
    internal fun closeAttachments(){pauseAttachments();attachmentCleanup?.lease?.invalidate();attachmentCleanup=null;attachmentVisible=false}
    internal fun showAttachments(){attachmentTransfer?.let {attachmentVisible=true}}
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
    fun composer(text: String) { val key = selection ?: return;if(composers[key]!=text)composerRevisions[key]=(composerRevisions[key] ?: 0L).let {if(it==Long.MAX_VALUE)it else it+1}; composers[key] = text; reserve(text) }
    internal fun composerCursor(start:Int,end:Int){val key=selection ?: return;val length=composers[key].orEmpty().length;if(start in 0..length&&end in 0..length)composerCursors[key]=start to end}

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
        if (text.isEmpty()) return
        val action = Uuid.random()
        warningJob = viewModelScope.launch {
            delay(150)
            try {
                val warning = store.recoveryWarning(key.host,action,PromptText(text))
                if (selection == key && composers[key].orEmpty() == text) reservation = SendReservation(key,action,text,warning)
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
        val boot = connection.state.value.hello[prepared.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        // Consume this UUID once. An uncertain result remains status-only, never replayed.
        reservation = null
        sentText[RecoveryIdentity(prepared.key.host,prepared.action)] = prepared.key to prepared.text
        focusedId=prepared.action
        try {connection.mutate(prepared.key.host,Request(prepared.action,RequestBody.PromptSubmit(PromptSubmitParams(boot,prepared.key.target,PromptText(prepared.text)))),true,RecoveryApproval(prepared.warning.replace,prepared.warning.futureExpiry))}
        catch(error: RecoveryApprovalRequired) {
            // No pre-send commit/native request occurred. Prepare a NEW visible notice, not a replay.
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
    internal fun closeAgentSession(){agentSessionCreation?.context?.lease?.invalidate();agentSessionCreation?.memory?.clear();agentSessionCreation=null}
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
    internal fun createAgentSession(shown:AgentSessionCreation,project:Uuid,name:String)=perform {
        if(!agentSessionAllowed(shown))throw StorePolicy(ErrorCode.NotReady)
        val (prior,pending)=retainedAgentSession(shown)
        if(!agentSessionAllowed(shown)||pending||prior!=shown.prior)throw StorePolicy(ErrorCode.NotReady)
        val view=connection.state.value
        if(view.inventory[shown.key.host]?.projects?.none {it.projectId==project&&it.available}!=false)throw StorePolicy(ErrorCode.NotReady)
        val boot=view.hello[shown.key.host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();val previous=creationActions.put(shown.key.host,action);val permit=shown.separate
        shown.submitted=action;shown.separate=null;shown.pending=true;focusedId=action
        try {connection.mutate(shown.key.host,Request(action,RequestBody.SessionCreate(SessionCreateParams(boot,shown.server,project,shown.provider,Label(name)))),agentCreation=shown.context)}
        catch(error:Exception){if(withContext(NonCancellable){store.localAction(shown.key.host,action)}==null){if(previous==null)creationActions.remove(shown.key.host)else creationActions[shown.key.host]=previous;shown.submitted=null;shown.separate=permit;shown.memory.text.remove("submitted")};throw error}
        finally {if(agentSessionAdmitted(shown))loadAgentSession(shown)}
    }
    fun create(server: Uuid,project: Uuid,provider: Provider,name: String) = perform {
        val host = host ?: return@perform; check(foreground)
        val boot = connection.state.value.hello[host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();creationActions[host]=action;focusedId=action
        try {connection.mutate(host,Request(action,RequestBody.SessionCreate(SessionCreateParams(boot,server,project,provider,Label(name)))))}
        catch(error: Exception) {
            if(withContext(NonCancellable) {store.localAction(host,action)}==null) {creationActions.remove(host);creationForms[host]?.text?.remove("submitted")}
            throw error
        }
    }
    private fun requireWorktreeCreationScope(scope: WorktreeCreationRequestScope) {
        check(foreground && selection==scope.key && screen==Screen.Conversation)
        val listing=worktrees ?: throw StorePolicy(ErrorCode.NotReady)
        if(listing.key!=scope.key || listing.source!=scope.source || listing.ticket!=scope.ticket || listing.loading || listing.error
            || connection.state.value.sources[scope.key.host]?.get(scope.key.target)!=scope.source) throw StorePolicy(ErrorCode.StaleSession)
    }
    private suspend fun retainedWorktreeCreation(scope: WorktreeCreationRequestScope): Pair<LocalAction?,Boolean> {
        requireWorktreeCreationScope(scope)
        var after: Uuid?=null;var latest:LocalAction?=null;var pinned:LocalAction?=null;var pending=false
        val pinnedId=creationActions[scope.key.host]
        do {
            val page=store.localActions(scope.key.host,after,100)
            requireWorktreeCreationScope(scope)
            page.filter {it.metadata.method==ActionMethod.SessionCreate}.forEach {action->
                if(action.metadata.actionId==pinnedId) pinned=action
                if(latest==null || action.metadata.createdAt.value>latest!!.metadata.createdAt.value || (action.metadata.createdAt==latest!!.metadata.createdAt && action.metadata.actionId.value>latest!!.metadata.actionId.value)) latest=action
                if(action.receipt==null || action.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching)) pending=true
            }
            after=page.takeIf {it.size==100}?.lastOrNull()?.metadata?.actionId
        } while(after!=null)
        if(pinnedId!=null && pinned==null) throw StorePolicy(ErrorCode.NotReady)
        return (pinned ?: latest) to pending
    }
    fun loadWorktreeCreation(scope: WorktreeCreationRequestScope) {
        val generation=++worktreeGateGeneration
        worktreeCreationGate=WorktreeCreationGate(scope,false,null,true)
        viewModelScope.launch {
            try {
                val (prior,pending)=retainedWorktreeCreation(scope)
                requireWorktreeCreationScope(scope)
                if(worktreeGateGeneration==generation) {
                    if(prior!=null && worktreeSeparatePermit!=(scope to prior.metadata.actionId)) creationActions[scope.key.host]=prior.metadata.actionId
                    worktreeCreationGate=WorktreeCreationGate(scope,true,prior,pending)
                }
            } catch(cancel: CancellationException) {throw cancel}
            catch(error: Exception) {if(worktreeGateGeneration==generation) {worktreeCreationGate=null;notice=safeFailure(error)}}
        }
    }
    fun worktreeCreationAllowed(scope: WorktreeCreationRequestScope):Boolean {
        val gate=worktreeCreationGate?.takeIf {it.scope==scope && it.loaded && !it.pending} ?: return false
        return !creationActions.containsKey(scope.key.host) && (gate.prior==null || worktreeSeparatePermit==(scope to gate.prior.metadata.actionId))
    }
    /** One captured scope owns one durable action. Unknown outcomes never become another UUID. */
    fun createInWorktree(key: ConversationKey, source: Uuid, ticket: Long, project: Uuid, provider: Provider, name: String, existing: Boolean, branchInput: String, baseInput: String) = perform {
        val scope=WorktreeCreationRequestScope(key,source,ticket)
        requireWorktreeCreationScope(scope)
        val (prior,pending)=retainedWorktreeCreation(scope)
        requireWorktreeCreationScope(scope)
        if(pending || creationActions.containsKey(key.host) || (prior!=null && worktreeSeparatePermit!=(scope to prior.metadata.actionId))) {
            prior?.let {creationActions[key.host]=it.metadata.actionId;focusedId=it.metadata.actionId}
            worktreeCreationGate=WorktreeCreationGate(scope,true,prior,pending)
            throw StorePolicy(ErrorCode.NotReady)
        }
        val listing=worktrees ?: throw StorePolicy(ErrorCode.NotReady)
        if(listing.creationScope?.projectId!=project || listing.creationScope?.canCreate!=true) throw StorePolicy(ErrorCode.NotReady)
        val host=key.host;val view=connection.state.value
        val summary=view.sessions[host]?.firstOrNull {it.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==key.target}==true}?.state?.summary ?: throw StorePolicy(ErrorCode.StaleSession)
        if(summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.worktreeCreate || view.mutationReady[host]!=true || view.inventoryReady[host]!=true) throw StorePolicy(ErrorCode.NotReady)
        val server=view.inventory[host]?.servers?.singleOrNull {it.serverId==summary.serverId} ?: throw StorePolicy(ErrorCode.NotReady)
        if(!(if(provider==Provider.Omp) server.canCreate.omp else server.canCreate.codex)) throw StorePolicy(ErrorCode.NotReady)
        val boot=view.hello[host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val action=Uuid.random();val branch=branchInput.trim().ifEmpty {"worktree/${action.value}"}
        val checkout=if(existing) {if(branchInput.isBlank()) throw StorePolicy(ErrorCode.InvalidRequest);CreationCheckout.Existing(branch)} else CreationCheckout.New(branch,baseInput.trim().ifEmpty {"HEAD"})
        val permit=worktreeSeparatePermit;worktreeSeparatePermit=null
        creationActions[host]=action;focusedId=action
        worktreeCreationGate=WorktreeCreationGate(scope,false,null,true)
        try {connection.mutate(host,Request(action,RequestBody.SessionCreate(SessionCreateParams(boot,summary.serverId,project,provider,Label(name),key.target,checkout))))}
        catch(error:Exception) {
            if(withContext(NonCancellable) {store.localAction(host,action)}==null) {creationActions.remove(host);worktreeSeparatePermit=permit}
            throw error
        } finally {
            if(foreground && selection==key && screen==Screen.Conversation && connection.state.value.sources[host]?.get(key.target)==source) loadWorktreeCreation(scope)
        }
    }
    private fun validateSeparateWorktree(scope:WorktreeCreationRequestScope,action:Uuid,generation:Long,confirmation:Boolean,current:LocalAction?=null):LocalAction {
        requireWorktreeCreationScope(scope);check(generation==separateCreationGeneration)
        if(confirmation) check(separateCreationAction==action && separateWorktreeScope==scope)
        val displayed=worktreeCreationGate?.takeIf {it.scope==scope && it.loaded}?.prior?.takeIf {it.metadata.actionId==action} ?: throw StorePolicy(ErrorCode.NotReady)
        if(current!=null) check(displayed==current)
        check(displayed.authoritative && displayed.outcomeShown && displayed.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))
        creationActions[scope.key.host]?.let {check(it==action)}
        return displayed
    }
    fun prepareSeparateWorktree(scope:WorktreeCreationRequestScope,action:Uuid)=perform {
        invalidateSeparateCreation();val generation=separateCreationGeneration
        validateSeparateWorktree(scope,action,generation,false)
        check(!otherPendingCreation(scope.key.host,action) {requireWorktreeCreationScope(scope)})
        val current=store.localAction(scope.key.host,action) ?: throw StorePolicy(ErrorCode.NotReady)
        validateSeparateWorktree(scope,action,generation,false,current)
        separateCreationAction=action;separateWorktreeScope=scope
    }
    fun confirmSeparateWorktree(scope:WorktreeCreationRequestScope,action:Uuid,onConfirmed:()->Unit)=perform {
        val generation=separateCreationGeneration
        validateSeparateWorktree(scope,action,generation,true)
        check(!otherPendingCreation(scope.key.host,action) {requireWorktreeCreationScope(scope)})
        val current=store.localAction(scope.key.host,action) ?: throw StorePolicy(ErrorCode.NotReady)
        validateSeparateWorktree(scope,action,generation,true,current)
        creationActions.remove(scope.key.host)
        worktreeSeparatePermit=scope to action
        invalidateSeparateCreation(false);loadWorktreeCreation(scope);onConfirmed()
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
        shell?.takeIf {it.selection.host==host && shellAdmitted(it)}?.let {loadShellInput(ShellCreateScope(it.selection,it.source,it.ticket))}
    }
    fun displayed(host: String,receipt: Receipt) { viewModelScope.launch {
        try {
            store.outcomeShown(host,receipt.actionId,receipt)
            if(receipt.method==ActionMethod.ShellInput && receipt.state==ReceiptState.Succeeded) shellSubmittedDrafts.remove(receipt.actionId)?.let {(key,revision)->shellInputDrafts[key]?.takeIf {it.revision==revision}?.let {shellInputDrafts[key]=it.copy(value=androidx.compose.ui.text.input.TextFieldValue(),revision=it.revision+1)}}
            worktreeCreationGate?.takeIf {it.scope.key.host==host && it.prior?.metadata?.actionId==receipt.actionId}?.let {if(foreground && selection==it.scope.key && screen==Screen.Conversation) loadWorktreeCreation(it.scope)}
            if(receipt.state==ReceiptState.Dispatched || receipt.state==ReceiptState.Succeeded) {
                sentText.remove(RecoveryIdentity(host,receipt.actionId))?.let {(key,text)->if(composers[key]===text && reservation?.key!=key) composers.remove(key)}
            }
            refreshActions()
            agentSessionCreation?.takeIf {it.key.host==host&&it.prior?.metadata?.actionId==receipt.actionId&&agentSessionAdmitted(it)}?.let {loadAgentSession(it)}
            agentTerminalCreation?.takeIf {it.key.host==host&&it.prior?.metadata?.actionId==receipt.actionId&&agentTerminalAdmitted(it)}?.let {loadAgentTerminal(it)}
            shell?.takeIf {it.selection.host==host && shellAdmitted(it)}?.let {loadShellInput(ShellCreateScope(it.selection,it.source,it.ticket))}
            shell?.let {shown->if(host==shown.selection.host && shellAdmitted(shown)) loadShellCreation(ShellCreateScope(shown.selection,shown.source,shown.ticket))}
        } catch (error: Exception) { notice = safeFailure(error) }
    } }
    fun discard(host: String,action: Uuid) = perform { store.discardRecovery(host,action,true) }
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
        if(attentionInbox?.host==host) closeAttentionInbox()
        if(shell?.selection?.host==host) closeShell()
        shellInputDrafts.keys.filter {it.host==host}.forEach(shellInputDrafts::remove)
        shellInputActions.keys.filter {it.host==host}.forEach(shellInputActions::remove)
        shellSubmittedDrafts.keys.filter {shellSubmittedDrafts[it]?.first?.host==host}.forEach(shellSubmittedDrafts::remove)
        shellCreationActions.remove(host)
        if(terminal?.key?.host==host) closeTerminal()
        if(worktrees?.key?.host==host) closeWorktrees()
        if(skills?.key?.host==host || skillsCatalogue?.key?.host==host) closeSkills()
        if(changes?.key?.host==host) closeChanges()
        composers.keys.filter {it.host==host}.forEach(composers::remove)
        sentText.keys.filter {it.hostId==host}.forEach(sentText::remove)
        creationForms.remove(host)?.clear();creationActions.remove(host);invalidateSeparateCreation()
    }
    internal fun pushMode(host:String):DeliveryMode {deliveryPreferenceRevision;return deliveryPreferences.mode(host)}
    internal fun pushLabel(host:String):String {deliveryPreferenceRevision;return runtime.push.label(host)}
    internal fun ntfyAvailable():Boolean=runtime.push.ntfyInstalled()
    internal fun selectNtfy(host:String,repair:Boolean=false){
        val generation=foregroundGeneration ?: return
        fun active()=foreground&&foregroundGeneration===generation&&this.host==host&&screen==Screen.Settings&&store.state.value.hosts.any{it.hostId==host}
        if(!active())return
        viewModelScope.launch{try{
            runtime.push.selectNtfy(host,repair,::active)
            if(active()){
                runtime.notifications.setHostEnabled(host,true)
                if(!deliveryPreferences.requiresKeeper(host,runtime.push.verified(host)))return@launch
                if(deliveryPreferences.canPostNotifications())DeliveryService.startFromUser(getApplication())
                else{deliveryPreferences.setServiceRequested(true);permissionRequestVisible=true;notificationPermissionRequest++}
            }
        }catch(cancel:CancellationException){throw cancel}catch(_:ProtectedKeyUnavailable){notice="Protected push material is unavailable. Restore key access or explicitly Forget this desktop; repair never replaces retained keys."}catch(_:Exception){notice="ntfy registration needs repair. Pairing and the selected delivery mode were preserved."}}
    }
    internal fun selectSocketDelivery(host:String){
        if(!foreground||this.host!=host||screen!=Screen.Settings)return
        viewModelScope.launch{try{runtime.push.selectPersistent(host);notice="Socket delivery selected. Start it while this screen is visible."}catch(cancel:CancellationException){throw cancel}catch(_:Exception){notice="Socket selection could not complete; inspect delivery settings."}}
    }
    fun setPersistentDelivery(host:String,enabled:Boolean) {
        viewModelScope.launch {
            try { runtime.notifications.setHostEnabled(host,enabled);if(!enabled&&deliveryPreferences.mode(host)==DeliveryMode.Ntfy)runtime.push.disable(host)else if(enabled&&deliveryPreferences.mode(host)==DeliveryMode.Ntfy)selectNtfy(host) }
            catch(_:Exception) { notice="Delivery preference could not be applied." }
            if(!deliveryPreferences.anyEnabled()) DeliveryService.stopFromUser(getApplication())
        }
    }
    fun setCompletionDelivery(host:String,enabled:Boolean) {
        viewModelScope.launch {try {runtime.notifications.setCompleted(host,enabled)} catch(_:Exception) {notice="Delivery preference could not be applied."}}
    }
    fun startPersistentDelivery() {
        if(!deliveryPreferences.anyEnabled()) {notice="Choose at least one paired desktop before starting persistent delivery.";return}
        if(store.state.value.hosts.none{deliveryPreferences.requiresKeeper(it.hostId,runtime.push.verified(it.hostId))}){notice="Selected ntfy registrations are verified. No foreground socket is required.";return}
        deliveryPreferences.setServiceRequested(true)
        if(!deliveryPreferences.canPostNotifications()) {permissionRequestVisible=foreground;notificationPermissionRequest++;return}
        DeliveryService.startFromUser(getApplication())
    }
    fun stopPersistentDelivery() {DeliveryService.stopFromUser(getApplication())}
    fun notificationPermissionResult() {
        val admitted=permissionRequestVisible && foreground
        permissionRequestVisible=false
        if(admitted && deliveryPreferences.canPostNotifications() && deliveryPreferences.serviceRequested()) DeliveryService.startFromUser(getApplication())
    }
    fun deliveryEnabled(host:String):Boolean {deliveryPreferenceRevision;return deliveryPreferences.hostEnabled(host)}
    fun deliveryCompleted(host:String):Boolean {deliveryPreferenceRevision;return deliveryPreferences.notifyCompleted(host)}
    fun deliveryRequested():Boolean {deliveryPreferenceRevision;return deliveryPreferences.serviceRequested()}
    fun deliveryLabel(host:String):String {
        deliveryPreferenceRevision
        return when {
            !deliveryPreferences.canPostNotifications() -> "Permission required"
            DeliveryService.isReconnecting(host) -> "Reconnecting"
            delivery.running && deliveryPreferences.serviceRequested() && deliveryPreferences.hostEnabled(host) -> "Running"
            else -> "Stopped"
        }
    }
    private fun invalidateSeparateCreation(clearPermit:Boolean=true) { separateCreationAction=null;separateWorktreeScope=null;separateCreationGeneration++;if(clearPermit) worktreeSeparatePermit=null }
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
        store.viewing(key.host,key.target.sessionId)
        var after: Uint? = null
        do {
            val sync = connection.notificationsSync(key.host,after)
            if (!foreground || selection != key || screen != Screen.Conversation || connection.state.value.sources[key.host]?.get(key.target)!=source) return
            sync.events.lastOrNull {it.sessionId==key.target.sessionId}?.let {event->if(sync.read.none {it.sessionId==event.sessionId && it.throughSequence.value>=event.sequence.value}) connection.markRead(key.host,event.sessionId,event.sequence)}
            after = sync.nextSequence
        } while (after != null)
        // The persisted watermark is authoritative; this only drains already-admitted OS alerts.
        runtime.notifications.reconcile(key.host)
    }
    override fun onCleared() {
        closeAttentionInbox();closeAgentSession()
        closeArtifacts();finishAttachmentPicker(null)
        closePairing(); closeTerminal(); closeWorktrees(); closeChanges();closeSkills(); closeShell(); forms.values.forEach(FormMemory::clear); creationForms.values.forEach(FormMemory::clear); composers.clear(); sentText.clear(); lifecycle.close()
        // Runtime ownership outlives this presentation instance; it may still serve the user-selected service.
        val stale=foregroundGeneration; val staleStartup=startup; foregroundGeneration=null; staleStartup?.cancel()
        if(stale!=null) runtime.cleanupScope.launch {
            try {staleStartup?.join()} catch(_:CancellationException) {}
            try {runtime.release(stale.claim)} catch(_:Exception) {}
        }
    }
    fun terminals(host:String) {navigate(Screen.Terminals,host)}
    private fun shellAdmitted(snapshot:ShellSnapshot):Boolean {
        val view=connection.state.value
        val inventory=view.inventory[snapshot.selection.host] ?: return false
        return foreground && screen==Screen.Terminals && host==snapshot.selection.host &&
            store.state.value.hosts.any {it.hostId==snapshot.selection.host} && view.inventoryReady[snapshot.selection.host]==true &&
            inventory.subscription==snapshot.source && snapshot.selection.host in view.hello && inventory.shells.any {it==snapshot.summary && it.target==snapshot.selection.target && it.connectivity==SessionConnectivity.Ready && it.capabilities.read}
    }
    fun openShell(host:String,target:ShellTarget) {
        val view=connection.state.value;val inventory=view.inventory[host] ?: return
        val summary=inventory.shells.singleOrNull {it.target==target} ?: return
        if(!foreground || screen!=Screen.Terminals || this.host!=host || view.inventoryReady[host]!=true || summary.connectivity!=SessionConnectivity.Ready || !summary.capabilities.read) return
        closeShell()
        shellMutationLease=ShellMutationLease()
        val selected=ShellSelection(host,target);val source=inventory.subscription;val ticket=shellTicket
        shell=ShellSnapshot(selected,source,ticket,summary,null,false,true,false)
        loadShellCreation(ShellCreateScope(selected,source,ticket))
        loadShellInput(ShellCreateScope(selected,source,ticket))
        shellJob=viewModelScope.launch {
            try {
                val result=connection.shell(host,target,source)
                val current=shell?.takeIf {it.ticket==ticket && it.selection==selected && it.source==source} ?: return@launch
                if(shellAdmitted(current) && result.target==target) shell=current.copy(text=result.text,truncated=result.truncated,loading=false,error=false)
            } catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){shell?.takeIf {it.ticket==ticket && shellAdmitted(it)}?.let {shell=it.copy(text=null,loading=false,error=true)}}
        }
    }
    fun closeShell() {shell?.selection?.let {key->shellInputDrafts[key]?.let {shellInputDrafts[key]=it.copy(modifiers=ShellKeyModifiers(false,false,false,false))}};clearShellDraftHost=null;shellInputGate=null;shellInputObservedStatus=null;shellInputSeparateStatus=null;shellInputPermitStatus=null;shellInputObserved=null;separateShellInput=null;shellInputPermit=null;shellInputGeneration++;shellMutationLease?.invalidate();shellMutationLease=null;shellTicket++;shellGateGeneration++;shell=null;shellJob?.cancel();shellJob=null;shellCreationGate=null;separateShellCreation=null;shellSeparatePermit=null}
    fun refreshShell(){shell?.selection?.let {openShell(it.host,it.target)}}
    fun refreshTerminals() {
        val originalHost=host ?: return
        if(screen!=Screen.Terminals) return
        closeShell()
        perform(::connectionFailureNotice){if(foreground && screen==Screen.Terminals && host==originalHost) connection.snapshot(originalHost)}
    }
    private fun requireShellScope(scope:ShellCreateScope):ShellSnapshot {
        val shown=shell?.takeIf {it.selection==scope.selection && it.source==scope.source && it.ticket==scope.ticket} ?: throw StorePolicy(ErrorCode.NotReady)
        if(!shellAdmitted(shown) || !shown.summary.capabilities.create || connection.state.value.mutationReady[scope.selection.host]!=true) throw StorePolicy(ErrorCode.NotReady)
        return shown
    }

    private fun requireShellInputScope(scope:ShellCreateScope):ShellSnapshot {
        val shown=shell?.takeIf {it.selection==scope.selection && it.source==scope.source && it.ticket==scope.ticket} ?: throw StorePolicy(ErrorCode.NotReady)
        if(!shellAdmitted(shown) || !shown.summary.capabilities.input || connection.state.value.mutationReady[scope.selection.host]!=true) throw StorePolicy(ErrorCode.NotReady)
        return shown
    }
    fun shellDraft(selection:ShellSelection)=shellInputDrafts[selection] ?: ShellInputDraft()
    fun editShellDraft(scope:ShellCreateScope,value:androidx.compose.ui.text.input.TextFieldValue) {
        if(runCatching {requireShellInputScope(scope)}.isFailure)return
        val old=shellDraft(scope.selection)
        if(value.text.toByteArray(Charsets.UTF_8).size>16384){notice="Terminal draft is limited to 16 KiB.";return}
        if(scope.selection !in shellInputDrafts && shellInputDrafts.size>=16){notice="Keep or explicitly clear an existing terminal draft before opening another.";return}
        shellInputDrafts[scope.selection]=old.copy(value=value,revision=old.revision+1)
    }
    fun requestClearShellDrafts(selectedHost:String) {
        if(foreground && screen==Screen.Terminals && host==selectedHost && store.state.value.hosts.any {it.hostId==selectedHost})clearShellDraftHost=selectedHost
    }
    fun cancelClearShellDrafts(){clearShellDraftHost=null}
    fun confirmClearShellDrafts() {
        val selectedHost=clearShellDraftHost ?: return
        if(!foreground || screen!=Screen.Terminals || host!=selectedHost || store.state.value.hosts.none {it.hostId==selectedHost}){clearShellDraftHost=null;return}
        // Local text/mode only. Original receipt latches and admitted jobs survive.
        shellInputDrafts.keys.filter {it.host==selectedHost}.forEach(shellInputDrafts::remove)
        shellSubmittedDrafts.keys.filter {shellSubmittedDrafts[it]?.first?.host==selectedHost}.forEach(shellSubmittedDrafts::remove)
        clearShellDraftHost=null
    }
    fun clearShellDraft(scope:ShellCreateScope){if(runCatching {requireShellInputScope(scope)}.isFailure)return;shellSubmittedDrafts.keys.filter {shellSubmittedDrafts[it]?.first==scope.selection}.forEach(shellSubmittedDrafts::remove);shellInputDrafts.remove(scope.selection)}
    fun shellInputMode(scope:ShellCreateScope,keys:Boolean){if(runCatching {requireShellInputScope(scope)}.isFailure)return;if(scope.selection !in shellInputDrafts && shellInputDrafts.size>=16){notice="Terminal draft limit reached.";return};shellInputDrafts[scope.selection]=shellDraft(scope.selection).copy(keys=keys,modifiers=ShellKeyModifiers(false,false,false,false))}
    fun shellInputModifiers(scope:ShellCreateScope,modifiers:ShellKeyModifiers){if(runCatching {requireShellInputScope(scope)}.isFailure)return;if(scope.selection !in shellInputDrafts && shellInputDrafts.size>=16)return;shellInputDrafts[scope.selection]=shellDraft(scope.selection).copy(modifiers=modifiers)}
    private suspend fun retainedShellInput(scope:ShellCreateScope):LocalAction? {
        requireShellInputScope(scope);var after:Uuid?=null;var prior:LocalAction?=null;var latched:LocalAction?=null
        do {val page=store.localActions(scope.selection.host,after,100);requireShellInputScope(scope)
            for(action in page)if(action.metadata.method==ActionMethod.ShellInput && action.metadata.shellTarget==scope.selection.target){if(action.metadata.actionId==shellInputActions[scope.selection])latched=action;if(prior==null || action.metadata.createdAt.value>prior.metadata.createdAt.value || action.metadata.createdAt==prior.metadata.createdAt && action.metadata.actionId.value>prior.metadata.actionId.value)prior=action}
            after=if(page.size==100)page.last().metadata.actionId else null
        }while(after!=null)
        return latched ?: prior
    }
    fun loadShellInput(scope:ShellCreateScope) {
        val generation=++shellInputGeneration;shellInputGate=ShellInputGate(scope,false,null,true)
        viewModelScope.launch {try {val prior=retainedShellInput(scope);requireShellInputScope(scope)
            if(generation==shellInputGeneration){val observation=prior?.let {connection.state.value.shellInputStatus[scope.selection.host]?.get(it.metadata.actionId)};val pending=prior!=null && (observation==null || !observation.unknown && prior.receipt?.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching));shellInputGate=ShellInputGate(scope,true,prior,pending,observation)}
        }catch(_:Exception){if(generation==shellInputGeneration)shellInputGate=null}}
    }
    fun shellInputAllowed(scope:ShellCreateScope):Boolean {
        if(runCatching {requireShellInputScope(scope)}.isFailure)return false
        val gate=shellInputGate?.takeIf {it.scope==scope && it.loaded && !it.pending} ?: return false
        val prior=gate.prior ?: return shellInputActions[scope.selection]==null
        if(shellInputActions[scope.selection]!=null && shellInputActions[scope.selection]!=prior.metadata.actionId)return false
        // Historical success is inspection only without a current retained status.
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(prior.metadata.actionId) ?: return false
        if(gate.observation!=observation)return false
        return (!observation.unknown && prior.authoritative && prior.outcomeShown && prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed)) || (shellInputPermit==(scope to prior.metadata.actionId) && shellInputPermitStatus==observation && shellInputObserved==(scope.selection to prior.metadata.actionId) && shellInputObservedStatus==observation)
    }
    fun shellInputDisplayed(scope:ShellCreateScope,action:LocalAction) {
        if(runCatching {requireShellInputScope(scope)}.isFailure || shellInputGate?.prior!=action)return
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(action.metadata.actionId) ?: return
        if(shellInputGate?.observation!=observation)return
        shellInputObservedStatus=observation
        shellInputObserved=scope.selection to action.metadata.actionId
        if(!observation.unknown && action.authoritative && action.receipt!=null && !action.outcomeShown)displayed(scope.selection.host,action.receipt)
    }
    fun shellKeyCharacter(scope:ShellCreateScope,value:String){if(runCatching {requireShellInputScope(scope)}.isFailure)return;if(value.toByteArray().size>4 || scope.selection !in shellInputDrafts && shellInputDrafts.size>=16)return;shellInputDrafts[scope.selection]=shellDraft(scope.selection).copy(keyCharacter=value)}
    private fun clearShellModifiers(scope:ShellCreateScope){shellInputDrafts[scope.selection]?.let {shellInputDrafts[scope.selection]=it.copy(modifiers=ShellKeyModifiers(false,false,false,false))}}
    fun sendShellCharacter(scope:ShellCreateScope){val draft=shellDraft(scope.selection);clearShellModifiers(scope);sendShellInput(scope,ShellInput.Keys(listOf(ShellKeyEvent(ShellKey(ShellKeyName.Character,draft.keyCharacter),draft.modifiers))),null)}
    fun sendShellText(scope:ShellCreateScope){val draft=shellDraft(scope.selection);clearShellModifiers(scope);sendShellInput(scope,ShellInput.Text(draft.value.text),draft.revision)}
    fun sendShellKey(scope:ShellCreateScope,key:ShellKeyName){val draft=shellDraft(scope.selection);clearShellModifiers(scope);sendShellInput(scope,ShellInput.Keys(listOf(ShellKeyEvent(ShellKey(key),draft.modifiers))),null)}
    private fun sendShellInput(scope:ShellCreateScope,input:ShellInput,revision:Long?)=perform {
        requireShellInputScope(scope);input.validate()
        if(!shellInputAllowed(scope))throw StorePolicy(ErrorCode.NotReady)
        val prior=retainedShellInput(scope);requireShellInputScope(scope)
        if(prior!=shellInputGate?.prior || !shellInputAllowed(scope))throw StorePolicy(ErrorCode.NotReady)
        val lease=shellMutationLease ?: throw StorePolicy(ErrorCode.NotReady)
        val inputObservation=prior?.let {action->connection.state.value.shellInputStatus[scope.selection.host]?.get(action.metadata.actionId)?.let {action.metadata.actionId to it.token}}
        if(prior!=null && inputObservation==null)throw StorePolicy(ErrorCode.NotReady)
        val original=connection.captureShellMutation(scope.selection.host,scope.selection.target,scope.source,lease,ShellMutationPurpose.Input,inputObservation)
        requireShellInputScope(scope)
        if(shellMutationLease!==lease || !lease.active.get() || !shellInputAllowed(scope))throw StorePolicy(ErrorCode.StaleSession)
        val action=Uuid.random();val host=scope.selection.host;val boot=connection.state.value.hello[host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val oldAction=shellInputActions[scope.selection];val permit=shellInputPermit;val permitStatus=shellInputPermitStatus;shellInputPermit=null;shellInputPermitStatus=null;shellInputActions[scope.selection]=action
        if(revision!=null)shellSubmittedDrafts[action]=scope.selection to revision
        shellInputDrafts[scope.selection]?.let {shellInputDrafts[scope.selection]=it.copy(modifiers=ShellKeyModifiers(false,false,false,false))}
        shellInputGate=ShellInputGate(scope,false,prior,true)
        try {connection.mutate(host,Request(action,RequestBody.ShellInput(ShellInputParams(boot,scope.selection.target,input))),ordinaryComposer=false,shell=original)}
        catch(error:Exception){if(withContext(NonCancellable){store.localAction(host,action)}==null){if(oldAction==null)shellInputActions.remove(scope.selection)else shellInputActions[scope.selection]=oldAction;shellInputPermit=permit;shellInputPermitStatus=permitStatus;shellSubmittedDrafts.remove(action)};throw error}
        finally {shell?.takeIf {shellAdmitted(it)}?.let {loadShellInput(ShellCreateScope(it.selection,it.source,it.ticket))}}
    }
    fun requestSeparateShellInput(scope:ShellCreateScope,action:Uuid) {
        if(runCatching {requireShellInputScope(scope)}.isFailure)return
        val gate=shellInputGate?.takeIf {it.scope==scope && it.loaded} ?: return
        val prior=gate.prior ?: return
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(action) ?: return
        if(prior.metadata.actionId==action && gate.observation==observation && shellInputObservedStatus==observation && shellInputObserved==(scope.selection to action) && (observation.unknown || prior.receipt?.state==ReceiptState.Uncertain)) {shellInputSeparateStatus=observation;separateShellInput=scope to action}
    }
    fun cancelSeparateShellInput(){separateShellInput=null;shellInputSeparateStatus=null}
    fun confirmSeparateShellInput()=perform {
        val (scope,action)=separateShellInput ?: throw StorePolicy(ErrorCode.NotReady);requireShellInputScope(scope)
        val prior=retainedShellInput(scope) ?: throw StorePolicy(ErrorCode.NotReady)
        val observation=connection.state.value.shellInputStatus[scope.selection.host]?.get(action) ?: throw StorePolicy(ErrorCode.NotReady)
        if(separateShellInput!=(scope to action) || prior.metadata.actionId!=action || shellInputGate?.prior!=prior || shellInputGate?.observation!=observation || shellInputSeparateStatus!=observation || shellInputObservedStatus!=observation || shellInputObserved!=(scope.selection to action) || !observation.unknown && prior.receipt?.state!=ReceiptState.Uncertain)throw StorePolicy(ErrorCode.NotReady)
        shellInputPermitStatus=observation;shellInputPermit=scope to action;separateShellInput=null;shellInputSeparateStatus=null;shellInputActions[scope.selection]=action;loadShellInput(scope)
    }
    private suspend fun retainedShellCreation(scope:ShellCreateScope,exclude:Uuid?=null):Pair<LocalAction?,Boolean> {
        requireShellScope(scope)
        var after:Uuid?=null;var prior:LocalAction?=null;var pending=false
        do {
            val page=store.localActions(scope.selection.host,after,100);requireShellScope(scope)
            for(action in page) {
                if(action.metadata.method==ActionMethod.ShellCreate && (prior==null || action.metadata.createdAt.value>prior.metadata.createdAt.value)) prior=action
                if(action.metadata.actionId!=exclude && action.metadata.method in listOf(ActionMethod.ShellCreate,ActionMethod.SessionCreate) && (action.receipt==null || action.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))) pending=true
            }
            after=page.lastOrNull()?.metadata?.actionId
        }while(after!=null)
        return prior to pending
    }
    fun loadShellCreation(scope:ShellCreateScope) {
        val generation=++shellGateGeneration
        shellCreationGate=ShellCreationGate(scope,false,null,true)
        viewModelScope.launch {
            try {
                val (prior,pending)=retainedShellCreation(scope);requireShellScope(scope)
                if(shellGateGeneration==generation){
                    if(prior!=null && shellSeparatePermit!=(scope to prior.metadata.actionId)) shellCreationActions[scope.selection.host]=prior.metadata.actionId
                    shellCreationGate=ShellCreationGate(scope,true,prior,pending)
                }
            }catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){if(shellGateGeneration==generation) shellCreationGate=null}
        }
    }
    fun shellCreationAllowed(scope:ShellCreateScope):Boolean {
        if(runCatching {requireShellScope(scope)}.isFailure) return false
        val gate=shellCreationGate?.takeIf {it.scope==scope && it.loaded && !it.pending} ?: return false
        return !shellCreationActions.containsKey(scope.selection.host) && (gate.prior==null || shellSeparatePermit==(scope to gate.prior.metadata.actionId))
    }
    fun createShell(scope:ShellCreateScope)=perform {
        requireShellScope(scope)
        val (prior,pending)=retainedShellCreation(scope);requireShellScope(scope)
        if(pending || shellCreationActions.containsKey(scope.selection.host) || prior!=null && shellSeparatePermit!=(scope to prior.metadata.actionId)) {loadShellCreation(scope);throw StorePolicy(ErrorCode.NotReady)}
        val host=scope.selection.host;val boot=connection.state.value.hello[host]?.bootId ?: throw StorePolicy(ErrorCode.NotReady)
        val lease=shellMutationLease ?: throw StorePolicy(ErrorCode.NotReady)
        val original=connection.captureShellMutation(host,scope.selection.target,scope.source,lease)
        requireShellScope(scope)
        if(shellMutationLease!==lease || !lease.active.get()) throw StorePolicy(ErrorCode.StaleSession)
        val action=Uuid.random();val permit=shellSeparatePermit;shellSeparatePermit=null
        shellCreationActions[host]=action;focusedId=action;shellCreationGate=ShellCreationGate(scope,false,null,true)
        try {connection.mutate(host,Request(action,RequestBody.ShellCreate(ShellCreateParams(boot,scope.selection.target))),shell=original)}
        catch(error:Exception){if(withContext(NonCancellable){store.localAction(host,action)}==null){shellCreationActions.remove(host);shellSeparatePermit=permit};throw error}
        finally {shell?.takeIf {shellAdmitted(it)}?.let {loadShellCreation(ShellCreateScope(it.selection,it.source,it.ticket))}}
    }
    private fun displayedShellAction(scope:ShellCreateScope,action:Uuid):LocalAction {
        requireShellScope(scope)
        val prior=shellCreationGate?.takeIf {it.scope==scope && it.loaded}?.prior?.takeIf {it.metadata.actionId==action} ?: throw StorePolicy(ErrorCode.NotReady)
        if(!prior.authoritative || !prior.outcomeShown || prior.receipt?.state !in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain) || shellCreationActions[scope.selection.host]!=action) throw StorePolicy(ErrorCode.NotReady)
        return prior
    }
    fun prepareSeparateShell(scope:ShellCreateScope,action:Uuid)=perform {
        val original=displayedShellAction(scope,action)
        if(retainedShellCreation(scope,action).second) throw StorePolicy(ErrorCode.NotReady)
        val current=store.localAction(scope.selection.host,action) ?: throw StorePolicy(ErrorCode.NotReady)
        if(current!=original || displayedShellAction(scope,action)!=original) throw StorePolicy(ErrorCode.NotReady)
        separateShellCreation=scope to action
    }
    fun cancelSeparateShell(){separateShellCreation=null}
    fun confirmSeparateShell()=perform {
        val (scope,action)=separateShellCreation ?: throw StorePolicy(ErrorCode.NotReady)
        val original=displayedShellAction(scope,action)
        if(retainedShellCreation(scope,action).second) throw StorePolicy(ErrorCode.NotReady)
        val current=store.localAction(scope.selection.host,action) ?: throw StorePolicy(ErrorCode.NotReady)
        if(current!=original || displayedShellAction(scope,action)!=original || separateShellCreation!=(scope to action)) throw StorePolicy(ErrorCode.NotReady)
        shellCreationActions.remove(scope.selection.host);shellSeparatePermit=scope to action;separateShellCreation=null;loadShellCreation(scope)
    }

    private companion object {
        const val PRESENTATION_PREFERENCES = "local_presentation"
        const val APPEARANCE_KEY = "appearance"
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
