// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.coroutines.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

/** Invalidates only a not-yet-admitted original-view action; no wire authority or replay. */
internal class AgentSessionLease {
    private val gate=Any();private var active=true
    fun invalidate(){synchronized(gate){active=false}}
    fun <T> admit(block:()->T):T=synchronized(gate){if(!active)throw StorePolicy(ErrorCode.NotReady);block()}
}
class AgentSessionMutationContext internal constructor(internal val original:Any,internal val lease:AgentSessionLease)
internal class AgentSessionCreation(val key:ConversationKey,val source:Uuid,val foreground:Any,val server:Uuid,val provider:Provider,val context:AgentSessionMutationContext) {
    val memory=FormMemory().apply {text["server"]=server.value;text["provider"]=provider.name}
    var loaded by mutableStateOf(false);var pending by mutableStateOf(true)
    var prior by mutableStateOf<LocalAction?>(null);var separate by mutableStateOf<Uuid?>(null)
    var confirming by mutableStateOf<Uuid?>(null);var submitted by mutableStateOf<Uuid?>(null)
    var loading:Any?=null
    var directories:CreationDirectoryBrowser?=null
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun AgentSessionCreationSheet(shown:AgentSessionCreation,connection:ConnectionView,model:GoatrViewModel,hostName:String) {
    val inventory=connection.inventory[shown.key.host]
    val server=inventory?.servers?.singleOrNull {it.serverId==shown.server}
    ModalBottomSheet(onDismissRequest={model.closeAgentSession(shown)},sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true),containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.9f).navigationBarsPadding().imePadding().padding(16.dp).testTag("agent_session_creation"),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            GoatrTitleBar("New session",connectionLabel(shown.key.host,connection,model.connectingHosts[shown.key.host]==true),!shown.loaded,compactStatus=compactConnectionLabel(shown.key.host,connection,model.connectingHosts[shown.key.host]==true),actions={
                IconButton(onClick={model.closeAgentSession(shown)},modifier=Modifier.size(48.dp)){GoatrIcon(MaterialGlyph.Close,"Close new session")}
            })
            val prior=shown.prior
            if(prior!=null && shown.separate!=prior.metadata.actionId) {
                Column(Modifier.fillMaxWidth().weight(1f).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    UiText("Create a separate native session in an explicitly selected project/directory, optionally beside this original agent. Retained actions below are inspection only.",style=MaterialTheme.typography.bodySmall)
                    UiText(hostName+" · "+(server?.name?.value ?: "Original desktop server")+" · "+providerLabel(shown.provider),style=MaterialTheme.typography.labelLarge)
                    ReceiptPanel(shown.key.host,prior,model)
                    UiText("This desktop’s retained creation is never retried. A separate session needs a new name and explicit confirmation.",style=MaterialTheme.typography.bodySmall)
                    if(prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))TextButton(onClick={model.prepareSeparateAgentSession(shown,prior.metadata.actionId)},enabled=!shown.pending&&!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Add,"Start a separate session")}
                }
            } else if(inventory!=null && server!=null) {
                val gate=creationReadinessReason(connection.hello[shown.key.host],model.busy,connection.inventoryReady[shown.key.host]==true,connection.mutationReady[shown.key.host]==true,!model.agentSessionAllowed(shown))
                Box(Modifier.fillMaxWidth().weight(1f)) {
                    CreateSession(inventory.copy(servers=listOf(server)),connection.hello[shown.key.host],model.agentSessionAllowed(shown)&&!model.busy,shown.memory,{
                        UiText("Choose a configured project and directory. Placement beside this agent is independent; nothing is inferred from its cwd.",style=MaterialTheme.typography.bodySmall)
                        UiText(hostName+" · "+server.name.value+" · "+providerLabel(shown.provider),style=MaterialTheme.typography.labelLarge)
                    },gate?.message,gate?.tag,requireProjectSelection=true,directories=shown.directories,beside=shown.server to shown.key.target) {choice->model.createAgentSession(shown,choice)}
                }
            } else UiText("Refresh this original desktop’s session inventory before creating a session.")
            TextButton(onClick={model.inspectAgentSession(shown)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check retained creation")}
            if(shown.pending && shown.loaded)UiText("A creation is still pending or unknown. Inspect its receipt; no action is replayed.",style=MaterialTheme.typography.bodySmall)
        }
    }
    shown.confirming?.let {action->AlertDialog(onDismissRequest={shown.confirming=null},title={UiText("Start a separate session?")},text={UiText("The prior creation may already have resources. Leave them untouched. This creates a new session in an explicitly selected configured project; it never retries or cleans up the prior action.")},confirmButton={TextButton(onClick={model.confirmSeparateAgentSession(shown,action)},enabled=!model.busy){UiText("Continue with a new session")}},dismissButton={TextButton(onClick={shown.confirming=null}){UiText("Cancel")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))}
}

/** Memory-only names/navigation scope. Nothing here is a phone pathname or persisted action data. */
class CreationDirectoriesContext internal constructor(
    internal val original:Any,
    internal val lease:AgentSessionLease,
    val server:Uuid,
    val project:Uuid,
    internal val expiresAt:Long,
    internal val viewCurrent:()->Boolean,
) {
    internal val references=mutableSetOf<Uuid>()
    internal val cursors=mutableMapOf<Uuid,Uuid?>()
    internal fun <T> admitView(block:()->T):T=lease.admit {
        if(!viewCurrent())throw StorePolicy(ErrorCode.StaleSession)
        block()
    }
    internal fun forgetReferences()=lease.admit {references.clear();cursors.clear()}
}
internal data class CreationDirectoryChoice(val context:CreationDirectoriesContext,val reference:Uuid?)
internal data class SessionCreationChoice(
    val server:Uuid,val project:Uuid,val provider:Provider,val name:String,
    val directory:CreationDirectoryChoice?,val placement:SessionPlacement?,
)
internal data class CreationBreadcrumb(val reference:Uuid,val name:Label)

/** One foreground dialog's bounded metadata owner; cancellation never submits or retries creation. */
internal class CreationDirectoryBrowser(
    private val connection:GoatrConnection,
    private val scope:CoroutineScope,
    private val host:String,
    private val agent:AgentSessionMutationContext?,
    private val terminal:TerminalCreationMutationContext?=null,
    private val viewCurrent:()->Boolean,
) {
    var server by mutableStateOf<Uuid?>(null);private set
    var project by mutableStateOf<Uuid?>(null);private set
    var context by mutableStateOf<CreationDirectoriesContext?>(null);private set
    var entries by mutableStateOf<List<CreationDirectoryEntry>>(emptyList());private set
    var breadcrumbs by mutableStateOf<List<CreationBreadcrumb>>(emptyList());private set
    var next by mutableStateOf<Uuid?>(null);private set
    var loading by mutableStateOf(false);private set
    var error by mutableStateOf<ErrorCode?>(null);private set
    var expired by mutableStateOf(false);private set
    var selectionExpired by mutableStateOf(false);private set
    var closed by mutableStateOf(false);private set
    private var job:Job?=null
    private var expiry:Job?=null
    private var ticket=Any()
    private val displayed=HashSet<Uuid>(134)
    private val reference get()=breadcrumbs.lastOrNull()?.reference
    fun current():Boolean=!closed&&viewCurrent()&&(context?.let {connection.creationDirectoriesCurrent(it)}!=false)
    fun close() {
        if(closed)return
        closed=true;context?.lease?.invalidate();context=null;ticket=Any()
        job?.cancel();expiry?.cancel();job=null;expiry=null
        entries=emptyList();displayed.clear();breadcrumbs=emptyList();next=null;loading=false
    }
    fun configure(server:Uuid?,project:Uuid?) {
        if(closed||this.server==server&&this.project==project&&context!=null)return
        context?.lease?.invalidate();context=null;ticket=Any();job?.cancel();expiry?.cancel()
        this.server=server;this.project=project;entries=emptyList();displayed.clear();breadcrumbs=emptyList();next=null
        expired=false;selectionExpired=false;error=null;loading=false
        if(server==null||project==null||!viewCurrent())return
        val captured=try {connection.captureCreationDirectories(host,server,project,agent,terminal) {!closed&&viewCurrent()}}
        catch(failure:StorePolicy){error=failure.code;return}
        context=captured
        expiry=scope.launch {
            delay((captured.expiresAt-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0))
            if(!closed&&context===captured) {
                job?.cancel();ticket=Any()
                runCatching {captured.forgetReferences()}
                selectionExpired=reference!=null
                expired=true;entries=emptyList();displayed.clear();breadcrumbs=emptyList();next=null;loading=false;error=ErrorCode.CursorExpired
            }
        }
        load(false)
    }
    fun refresh() {
        if(closed||!viewCurrent())return
        val server=server;val project=project
        context?.lease?.invalidate();context=null
        configure(server,project)
    }
    fun enter(entry:CreationDirectoryEntry) {
        if(!current()||loading||expired||entry !in entries||breadcrumbs.size>=16)return
        breadcrumbs=breadcrumbs+CreationBreadcrumb(entry.reference,entry.name)
        load(false)
    }
    fun parent(depth:Int) {
        if(!current()||loading||expired||depth !in 0..breadcrumbs.size)return
        breadcrumbs=breadcrumbs.take(depth);load(false)
    }
    fun more(){if(next!=null&&!loading&&!expired)load(true)}
    fun choice(server:Uuid?,project:Uuid?):CreationDirectoryChoice? {
        val captured=context ?: return null
        if(server!=this.server||project!=this.project||loading||!current())return null
        if(selectionExpired)return null
        if(reference!=null&&(expired||android.os.SystemClock.elapsedRealtime()>=captured.expiresAt))return null
        return CreationDirectoryChoice(captured,reference)
    }
    private fun load(append:Boolean) {
        val captured=context ?: return
        if(!current()||expired)return
        val parent=reference;val cursor=if(append)next else null
        if(append&&cursor==null)return
        val mine=Any();ticket=mine;job?.cancel();loading=true;error=null
        if(!append){entries=emptyList();displayed.clear();next=null}
        job=scope.launch {
            try {
                val page=connection.creationDirectories(captured,parent,cursor)
                captured.admitView {
                    if(ticket===mine&&context===captured&&current()&&reference==parent) {
                        if(append&&(entries.size+page.entries.size>1000||page.entries.any {it.reference in displayed}))throw StorePolicy(ErrorCode.ResourceLimit)
                        page.entries.forEach {displayed.add(it.reference)}
                        entries=if(append)entries+page.entries else page.entries
                        next=page.nextCursor;loading=false
                    }
                }
            }catch(cancel:CancellationException){throw cancel}
            catch(failure:StorePolicy){if(ticket===mine&&context===captured&&current()){error=failure.code;loading=false}}
            catch(_:Exception){if(ticket===mine&&context===captured&&current()){error=ErrorCode.NotReady;loading=false}}
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun CreationDirectoriesChooser(browser:CreationDirectoryBrowser,rootLabel:String) {
    val entries=browser.entries
    GroupedSurface {
        GoatrSection("Creation directory")
        UiText("Configured project: $rootLabel",style=MaterialTheme.typography.bodySmall)
        UiText(if(browser.selectionExpired)"Directory selection expired · choose again" else browser.breadcrumbs.lastOrNull()?.name?.value ?: "Configured project root",style=MaterialTheme.typography.labelLarge)
        UiText("Directory names/navigation only. Choosing never launches a session or sends input.",style=MaterialTheme.typography.bodySmall)
        UiText("Goatr verifies the actual new shell directory before provider input. Concurrent host directory replacement can leave uncertain retained resources; no atomic native check-and-chdir is promised.",style=MaterialTheme.typography.bodySmall)
        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            TextButton(onClick={browser.parent(0)},enabled=!browser.loading&&!browser.expired,modifier=Modifier.heightIn(min=48.dp).testTag("creation_directory_root")) {UiText("Project root")}
            browser.breadcrumbs.forEachIndexed {index,crumb->
                TextButton(onClick={browser.parent(index+1)},enabled=!browser.loading&&!browser.expired,modifier=Modifier.heightIn(min=48.dp)) {UiText(crumb.name.value)}
            }
        }
        if(browser.loading)UiText("Reading directories",style=MaterialTheme.typography.bodySmall)
        if(browser.error!=null)UiText(
            if(browser.expired&&browser.selectionExpired)"The selected directory expired. Refresh from project root and explicitly choose again."
            else if(browser.expired)"Directory names/references expired; your configured-root selection is unchanged."
            else "Subdirectory navigation is unavailable (${browser.error?.name}). The currently selected directory is not submitted automatically.",
            style=MaterialTheme.typography.bodySmall,
        )
        if(entries.isNotEmpty()) {
            androidx.compose.foundation.lazy.LazyColumn(Modifier.fillMaxWidth().heightIn(max=240.dp).testTag("creation_directories")) {
                items(entries.size,key={entries[it].reference.value}) {index->
                    val entry=entries[index]
                    TextButton(onClick={browser.enter(entry)},enabled=!browser.loading&&!browser.expired&&browser.breadcrumbs.size<16,modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)) {UiText(entry.name.value)}
                }
            }
        }else if(browser.context!=null&&!browser.loading&&browser.error==null)UiText("No selectable subdirectories. Empty directories are supported.",style=MaterialTheme.typography.bodySmall)
        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            if(browser.next!=null)TextButton(onClick=browser::more,enabled=!browser.loading&&!browser.expired,modifier=Modifier.heightIn(min=48.dp)) {UiText("More directories")}
            TextButton(onClick=browser::refresh,enabled=!browser.loading&&!browser.closed,modifier=Modifier.heightIn(min=48.dp).testTag("creation_directories_refresh")) {UiText("Refresh from project root")}
        }
    }
}
