// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import computer.iroh.OwnedReadPhase
import android.Manifest
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.time.Instant
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.encodeToString

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GoatrApp(model: GoatrViewModel,privateWindow: (Boolean)->Unit) {
    val connection by model.connection.state.collectAsStateWithLifecycle()
    val store by model.store.state.collectAsStateWithLifecycle()
    model.artifacts?.let{ArtifactBrowser(it,model)}
    model.agentSessionCreation?.let {AgentSessionCreationSheet(it,connection,model)}
    model.agentTerminalCreation?.let {AgentTerminalCreationDialog(it,model)}
    if(model.attachmentVisible)model.attachmentTransfer?.let {AttachmentTransferSheet(it,model)}
    model.attachmentRecovery?.let {AttachmentRecoverySheet(it,model)}
    val selected=model.selection
    val context=LocalContext.current
    val notificationPermission=rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {model.notificationPermissionResult()}
    LaunchedEffect(model.notificationPermissionRequest) {
        if(model.notificationPermissionRequest>0) {
            if(Build.VERSION.SDK_INT>=33) notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            else model.notificationPermissionResult()
        }
    }
    // This is a one-frame presentation route only. It never survives recreation or owns creation state.
    var pendingCreationHost by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(model.screen,model.host) {
        if(pendingCreationHost!=null && (model.screen!=Screen.Sessions || model.host!=pendingCreationHost)) pendingCreationHost=null
    }
    val reducer=selected?.let {key->connection.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}}
    val hasSecret=reducer?.pending?.any {it.presentation is PresentationCodexUrl || (it.presentation as? PresentationCodexQuestions)?.questions?.any {q->q.isSecret}==true}==true
    // Inline catalogue names retain the picker’s screenshot/recents privacy after dismissal.
    val inlineSkillsPrivate=selected?.let {key->model.suggestedSkills(key,model.composers[key].orEmpty())?.catalogue!=null}==true
    SideEffect {privateWindow(model.attentionInbox!=null || model.artifacts!=null || model.agentTerminalCreation!=null || model.agentSessionCreation!=null || model.attachmentVisible || model.attachmentRecovery!=null || model.shell!=null || model.screen==Screen.Pairing && model.preview==null || model.screen==Screen.Conversation && (hasSecret || model.terminal!=null || model.worktrees!=null || model.changes!=null || inlineSkillsPrivate))}
    DisposableEffect(Unit) {onDispose {privateWindow(false)}}
    BackHandler(model.screen!=Screen.Sessions || model.host!=null) { model.sessions() }
    Scaffold(
        modifier=Modifier.fillMaxSize().semantics {testTagsAsResourceId=true},
        contentWindowInsets=WindowInsets.safeDrawing,
        // Conversation is a detail destination: its composer docks to the IME, not a navigation bar.
        bottomBar={ if(model.screen!=Screen.Conversation) TopLevelNav(model.screen, { model.sessions() }, model::connections, { model.settings() }) },
    ) {padding->
        BoxWithConstraints(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding).imePadding().padding(horizontal=if(model.screen==Screen.Conversation) 10.dp else 12.dp)) {
            val wide=maxWidth>=840.dp
            Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().heightIn(min=56.dp),horizontalArrangement=Arrangement.spacedBy(6.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                if(model.screen==Screen.Conversation) IconButton(onClick={model.sessions()},modifier=Modifier.size(48.dp).testTag("nav_sessions")) {GoatrIcon(MaterialGlyph.Back,"Back to sessions")}
                UiText(if(model.screen==Screen.Conversation) conversationTitle(reducer?.state?.summary) else when(model.screen) { Screen.Sessions -> "Sessions"; Screen.Connections -> "Hosts"; Screen.Pairing -> "Pair a desktop"; Screen.Settings -> "Settings"; Screen.Terminals -> "Terminals"; else -> "Goatr" },style=MaterialTheme.typography.titleLarge,maxLines=1,overflow=TextOverflow.Ellipsis,modifier=Modifier.weight(1f).semantics {heading()})
                var menu by remember {mutableStateOf(false)}
                Box {IconButton(onClick={menu=true},modifier=Modifier.size(48.dp).testTag("navigation_menu")) {GoatrIcon(MaterialGlyph.Menu,"Navigation menu")}
                    DropdownMenu(expanded=menu,onDismissRequest={menu=false}) {
                        DropdownMenuItem(text={UiText("Sessions")},onClick={menu=false;model.sessions()})
                        DropdownMenuItem(text={UiText("Hosts")},leadingIcon={GoatrIcon(MaterialGlyph.Desktop,null)},modifier=Modifier.testTag(if(model.screen==Screen.Conversation) "nav_hosts" else "navigation_hosts_menu"),onClick={menu=false;model.connections()})
                        DropdownMenuItem(text={UiText("Settings")},leadingIcon={GoatrIcon(MaterialGlyph.Settings,null)},modifier=Modifier.testTag(if(model.screen==Screen.Conversation) "nav_settings" else "navigation_settings_menu"),onClick={menu=false;model.settings()})
                        model.host?.let {host->DropdownMenuItem(text={UiText("Host settings")},modifier=Modifier.testTag("settings"),onClick={menu=false;model.settings(host)})}
                    }
                }
            }
            model.notice?.let {Surface(color=MaterialTheme.colorScheme.errorContainer,shape=MaterialTheme.shapes.small,modifier=Modifier.fillMaxWidth()) {
                UiText(it,color=MaterialTheme.colorScheme.onErrorContainer,style=MaterialTheme.typography.bodySmall,modifier=Modifier.heightIn(max=96.dp).verticalScroll(rememberScrollState()).padding(6.dp).semantics {liveRegion=LiveRegionMode.Assertive}.testTag("operation_notice"))
            }}
            // Keep transient progress from shifting the title, transcript and pinned controls.
            Box(Modifier.fillMaxWidth().height(4.dp)) {if(model.busy) LinearProgressIndicator(Modifier.fillMaxWidth().semantics {contentDescription="Operation in progress"})}
            Box(Modifier.weight(1f).fillMaxWidth()) {
            when(model.screen) {
                Screen.Connections -> HostsScreen(store,connection,model)
                Screen.Pairing -> PairingScreen(model)
                Screen.Sessions -> if(model.host==null) SessionsConsole(store,connection,model) {host->
                    pendingCreationHost=host
                    model.sessions(host)
                } else HostSessionsScreen(model.host!!,store,connection,model,pendingCreationHost==model.host) { pendingCreationHost=null }
                Screen.Terminals -> model.host?.let {ShellTerminalsScreen(it,store,connection,model)}
                Screen.Conversation -> selected?.let {key->
                    Row(Modifier.fillMaxSize(),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                        if(wide) Column(Modifier.width(280.dp).fillMaxHeight().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                            Heading("Conversations")
                            connection.inventory[key.host]?.let {SessionBrowser(it,connection.inventoryReady[key.host]==true,model.sessionListPreferences,store.hosts.firstOrNull {paired->paired.hostId==key.host}?.name) {target->model.open(key.host,target)}}
                        }
                        Box(Modifier.weight(1f)) {ConversationScreen(key,store,connection,model)}
                    }
                }
                Screen.Settings -> SettingsScreen(model.host,store,connection,model) {
                    context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE,context.packageName))
                }
            }
            }
            }
        }
    }
    model.attentionInbox?.takeIf {model.attentionInboxAdmitted(it)}?.let {AttentionInboxDialog(it,store,connection,model)}
    model.replacement?.let {prepared->
        AlertDialog(onDismissRequest=model::declineReplacement,title={UiText("Replace named recovery copies?")},text={Column(Modifier.verticalScroll(rememberScrollState())) {
            UiText("Only this Send action ${prepared.action.value} expires at ${deadline(prepared.warning.futureExpiry?.expiresAt)}. Separately approve deleting these retained copies:")
            prepared.warning.replace.forEach {id->UiText("Host ${store.hosts.firstOrNull {it.hostId==id.hostId}?.name ?: id.hostId}; action ${id.actionId.value}")}
        }},confirmButton={Action("Approve replacement and Send","recovery_replace",enabled=!model.busy,onClick=model::approveReplacement)},dismissButton={Action("Keep copies","recovery_keep",enabled=true,onClick=model::declineReplacement)})
    }
}

private fun deadline(at: Uint?): String = at?.let {Instant.ofEpochMilli(it.value).toString()} ?: "unavailable"
internal fun providerLabel(provider: Provider): String = when(provider) {
    Provider.Omp -> "OMP"
    Provider.Codex -> "Codex"
}
private fun conversationTitle(summary: SessionSummary?): String =
    summary?.title?.takeIf {it.isNotBlank()} ?: summary?.provider?.let {"${providerLabel(it)} conversation"} ?: "Conversation"
private fun sessionWorkspaceLabel(inventory: HostSnapshotResult,summary: SessionSummary): String? {
    val target=Target(summary.sessionId,summary.generation)
    val graph=inventory.hierarchy.firstOrNull {it.serverId==summary.serverId} ?: return null
    val pane=graph.panes.filter {it.target==target}.singleOrNull() ?: return null
    val tab=graph.tabs.filter {it.tabId==pane.tabId}.singleOrNull() ?: return null
    return graph.workspaces.filter {it.workspaceId==tab.workspaceId}.singleOrNull()?.label?.value
}
private fun sessionStatusLabel(status: SessionStatus): String = when(status) {
    SessionStatus.Attention -> "Needs your attention"
    SessionStatus.Working -> "Working"
    SessionStatus.Idle -> "Idle"
    SessionStatus.Unknown -> "Status unavailable"
}
private fun sessionGroupLabel(status: SessionStatus): String = when(status) {
    SessionStatus.Attention -> "Needs input"
    SessionStatus.Working -> "Working"
    SessionStatus.Idle -> "Idle"
    SessionStatus.Unknown -> "Unknown / unavailable"
}
private fun unboundStateLabel(state: UnboundState): String = when(state) {
    UnboundState.SetupRequired -> "Setup needed"
    UnboundState.Ambiguous -> "Needs desktop review"
    UnboundState.Unavailable -> "Unavailable"
}
private fun connectionLabel(host: String,view: ConnectionView,connecting: Boolean=false): String = when {
    connecting -> "Connecting"
    host !in view.hello -> "Offline"
    view.selectedPath[host]=="direct" -> "Connected directly"
    view.selectedPath[host]=="relay" -> "Connected through relay"
    else -> "Connected"
}
private fun inventoryRefreshing(host: String,view: ConnectionView): Boolean =
    host in view.hello && view.inventoryReady[host]!=true
private fun connectionFailureCopy(failure: ConnectionFailure): String = when(failure.category) {
    ConnectionFailureCategory.Timeout -> "Reconnect timed out. Check the desktop service, then reconnect."
    ConnectionFailureCategory.Known -> "Connection could not be completed. Check desktop setup, then reconnect."
    ConnectionFailureCategory.Protocol -> "Reconnect received an invalid desktop response. Check the desktop service, then reconnect."
    ConnectionFailureCategory.Transport -> "Reconnect lost its desktop connection. Check the desktop service, then reconnect."
    ConnectionFailureCategory.Other -> "Reconnect did not complete. Check the desktop service, then reconnect."
}
private fun connectionFailureStage(phase: ConnectionFailurePhase): String = when(phase) {
    ConnectionFailurePhase.Dial -> "connecting to the desktop"
    ConnectionFailurePhase.OpenStream -> "opening the desktop connection"
    ConnectionFailurePhase.Hello -> "confirming the desktop connection"
    ConnectionFailurePhase.Snapshot -> "loading desktop sessions"
    ConnectionFailurePhase.Reader -> "reading the desktop connection"
    ConnectionFailurePhase.Recovery -> "refreshing desktop sessions"
}
private fun helloReceiveStage(phase: HelloReceivePhase): String = when(phase) {
    HelloReceivePhase.NotStarted -> "not started"
    HelloReceivePhase.Header -> "waiting for a frame header"
    HelloReceivePhase.Payload -> "waiting for a frame payload"
    HelloReceivePhase.Decode -> "decoding a received frame"
    HelloReceivePhase.Apply -> "applying a received response"
    HelloReceivePhase.Completed -> "response completed"
}
private fun helloNativeReadStage(phase: OwnedReadPhase): String = when(phase) {
    OwnedReadPhase.NotStarted -> "not started"
    OwnedReadPhase.Entered -> "entered"
    OwnedReadPhase.PollRequested -> "native read poll requested"
    OwnedReadPhase.Wake -> "native poll woke"
    OwnedReadPhase.Ready -> "native poll ready"
    OwnedReadPhase.CompletionEntered -> "native completion entered"
    OwnedReadPhase.CompletionReturned -> "native completion returned"
}

@Composable
private fun HostsScreen(store: StoreView,connection: ConnectionView,model: GoatrViewModel) {
    var confirmInstallation by remember {mutableStateOf(false)}
    LazyColumn(verticalArrangement=Arrangement.spacedBy(8.dp),modifier=Modifier.testTag("connections_list")) {
        if(store.hosts.isEmpty()) item {
            EmptyState("No paired desktops","Run goatr pair in an explicitly requested desktop terminal, then scan or paste its invitation.") {
                IconAction(MaterialGlyph.Scan,"Pair a desktop","pair_begin",enabled=!model.busy,onClick=model::beginPairing)
                TextButton(onClick={confirmInstallation=true},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("installation_forget")) {UiText("Forget unpaired installation identity")}
            }
        }
        items(store.hosts,key={it.hostId}) {host->
            val online=host.hostId in connection.hello
            GroupedSurface(modifier=Modifier.testTag("host_${host.hostId}")) {
                CompactRow(host.name,"Paired desktop",status={Column(horizontalAlignment=androidx.compose.ui.Alignment.End) {StatusPill(connectionLabel(host.hostId,connection,model.connectingHosts[host.hostId]==true),if(online) StatusEmphasis.Positive else StatusEmphasis.Attention);RefreshHint(inventoryRefreshing(host.hostId,connection))}})
                if(!online) connection.connectionFailures[host.hostId]?.let {failure->
                    UiText(connectionFailureCopy(failure),style=MaterialTheme.typography.bodySmall)
                    var details by remember(host.hostId,failure) {mutableStateOf(false)}
                    DetailDisclosure(details,{details=!details},"connection_failure_details")
                    if(details) {
                        UiText("Reconnect stopped while ${connectionFailureStage(failure.phase)}.",style=MaterialTheme.typography.bodySmall)
                        if(failure.phase==ConnectionFailurePhase.Hello) {
                            failure.helloTransmissionCompleted?.let {completed->UiText(if(completed) "Hello transmission completed." else "Hello transmission was not confirmed.",style=MaterialTheme.typography.bodySmall)}
                            failure.helloReceivePhase?.let {phase->UiText("Hello receive phase: ${helloReceiveStage(phase)}.",style=MaterialTheme.typography.bodySmall)}
                            failure.helloNativeReadPhase?.let {phase->UiText("Native Hello read: ${helloNativeReadStage(phase)}.",style=MaterialTheme.typography.bodySmall)}
                        }
                    }
                }
                if(!online) IconAction(MaterialGlyph.Refresh,"Reconnect","host_reconnect",enabled=!model.busy && model.foreground) { model.reconnect(host.hostId) }
                else IconAction(MaterialGlyph.Chat,"View sessions","host_sessions",enabled=!model.busy) { model.sessions(host.hostId) }
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    if(online) TextButton(onClick={model.reconnect(host.hostId)},enabled=!model.busy && model.foreground,modifier=Modifier.heightIn(min=48.dp).testTag("host_reconnect")) {IconLabel(MaterialGlyph.Refresh,"Refresh connection")}
                    IconAction(MaterialGlyph.Terminal,"Terminals","host_terminals_${host.hostId}",enabled=!model.busy) {model.terminals(host.hostId)}
                    TextButton(onClick={model.settings(host.hostId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("host_settings")) {IconLabel(MaterialGlyph.Settings,"Host details")}
                }
            }
        }
        if(store.hosts.isNotEmpty()) item {
            TextButton(onClick=model::beginPairing,enabled=!model.busy,modifier=Modifier.testTag("pair_begin")) {IconLabel(MaterialGlyph.Scan,"Add desktop")}
        }
        items(store.keyDeletion,key={it.hostId}) {obligation->
            val installation=obligation.hostId==SessionStore.INSTALLATION_DELETION_SCOPE
            UiText(if(installation) "Installation key deletion is not yet acknowledged. Identity is not replaced." else "Key deletion not yet acknowledged for ${obligation.hostId}. Restore protected-store access; identity is not replaced.")
            Action("Retry consented local deletion","forget_retry",enabled=!model.busy) { if(installation) model.forgetInstallation() else model.forget(obligation.hostId) }
        }
    }
    if(confirmInstallation) AlertDialog(
        onDismissRequest={confirmInstallation=false},
        title={UiText("Forget this unpaired installation identity?")},
        text={UiText("Only when no paired hosts remain: shut down the owned endpoint and delete its actual protected ciphertext, wrapping alias and initialization marker. This is an explicit local deletion, not a server revoke or automatic replacement identity.")},
        confirmButton={Action("Confirm installation Forget","installation_forget_confirm",enabled=!model.busy) { confirmInstallation=false;model.forgetInstallation() }},
        dismissButton={Action("Keep installation identity","installation_forget_cancel") {confirmInstallation=false}},
    )
}

@Composable
private fun PairingScreen(model: GoatrViewModel) {
    Column(Modifier.verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        val preview=model.preview
        GoatrSection("Pair a desktop","Pairing grants this phone access to supported same-user sessions on the selected host.")
        if(preview==null) GroupedSurface { PairScanner(model.foreground,model::acceptInvitation) }
        else {
            GroupedSurface {
                GoatrSection("Confirm ${preview.name.value}","Review the full trust pin before pairing this host.")
                SelectionContainer {UiText("Fingerprint ${preview.fingerprint}")}
                UiText(preview.authority,modifier=Modifier.testTag("pair_authority"))
                UiText(preview.relayDisclosure)
                UiText("Invitation display expiry ${deadline(preview.expiresAt)}; host monotonic expiry is authoritative.",style=MaterialTheme.typography.bodySmall)
                Action("Confirm host-wide pairing","pair_confirm",enabled=model.foreground && !model.busy,onClick=model::confirmPairing)
                TextButton(onClick=model::retryPairing,enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("pair_retry")) {UiText("Use another invitation")}
            }
        }
        TextButton(onClick=model::connections,enabled=true,modifier=Modifier.heightIn(min=48.dp).testTag("pair_cancel")) {IconLabel(MaterialGlyph.Back,"Back and clear invitation")}
    }
}

@Composable
fun InventoryRows(inventory: HostSnapshotResult,ready: Boolean,onOpen: (Target)->Unit) {
    if(!ready) UiText("Stale/synchronizing desktop inventory · display only; no native association or focus authority.",modifier=Modifier.testTag("inventory_stale"))
    if(inventory.servers.isEmpty()) UiText("No named Herdr servers returned.")
    inventory.servers.forEach {server->
        Heading("Server ${server.name.value} · ${server.connectivity}")
        server.reason?.let {UiText(it)}
        val graph=inventory.hierarchy.firstOrNull {it.serverId==server.serverId}
        if(graph==null) UiText("Native hierarchy unavailable; supported session aliases are listed separately.")
        else {
            UiText("Native source ${graph.freshness}${graph.reason?.let {" · $it"}.orEmpty()}")
            if(graph.freshness==NativeFreshness.Unavailable) UiText("Native display unavailable; empty arrays do not mean an empty desktop.")
            else if(graph.workspaces.isEmpty()) UiText("This returned native workspace inventory is empty.")
            graph.workspaces.forEach {workspace->
                Heading("Workspace ${workspace.number.value}: ${workspace.label?.value ?: "label unavailable (${workspace.displayUnavailable})"} · ${workspace.agentStatus}${if(workspace.focused) " · desktop focused" else ""}")
                graph.tabs.filter {it.workspaceId==workspace.workspaceId}.forEach {tab->
                    Heading("Tab ${tab.number.value}: ${tab.label?.value ?: "label unavailable (${tab.displayUnavailable})"} · ${tab.agentStatus}${if(tab.focused) " · desktop focused" else ""}")
                    graph.panes.filter {it.tabId==tab.tabId}.forEach {pane->
                        UiText("Pane ${pane.label?.value ?: pane.paneId.value} · native ${pane.agentStatus}${if(pane.focused) " · desktop focused" else ""}")
                        listOfNotNull(pane.title?.value,pane.terminalTitle?.value,pane.displayAgent?.value,pane.agent?.value,pane.agentName?.value).forEach {UiText(it)}
                        pane.restoreReason?.let {UiText("Restore: $it")};pane.displayUnavailable?.let {UiText("Display: $it")}
                        val target=pane.target
                        if(target!=null && ready && graph.freshness==NativeFreshness.Current) Action("Open associated conversation","pane_open") {onOpen(target)}
                        else UiText("Read-only pane · no current mutable phone Target.")
                        pane.unboundRowId?.let {id->inventory.unbound.firstOrNull {it.rowId==id}?.let {UiText("${it.state}: ${it.reason}")}}
                    }
                }
            }
        }
        Heading("Supported conversation aliases · independent phone selection")
        inventory.sessions.filter {it.serverId==server.serverId}.groupBy {it.workspaceId}.forEach {(workspace,sessions)->
            UiText("Historical/native route context ${workspace.value} · never joined across incarnations")
            sessions.forEach {summary->
                UiText("${summary.title} · ${summary.provider} · provider ${summary.status} · ${summary.connectivity}")
                summary.reason?.let {UiText(it)}
                Action("Read ${summary.title}","session_${summary.sessionId.value}") {onOpen(Target(summary.sessionId,summary.generation))}
            }
        }
        inventory.unbound.filter {it.serverId==server.serverId}.forEach {row->Heading("${row.title} · ${row.provider} · ${row.state}");UiText(row.reason);UiText("No supported session binding. Desktop setup/inspection required; no fabricated chat Target.")}
    }
    if(inventory.sessions.isEmpty() && inventory.unbound.isEmpty()) UiText("No supported conversations. Shell-only and unsupported panes above remain read-only.")
}

@Composable
fun SessionBrowser(
    inventory: HostSnapshotResult,
    ready: Boolean,
    preferences: SessionListPreferences,
    hostName: String?,
    onOpen: (Target) -> Unit,
) {
    var query by remember {mutableStateOf("")}
    OutlinedTextField(query,{query=it},label={UiText("Find a conversation")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("session_search"))
    Box(Modifier.fillMaxWidth().heightIn(min=24.dp)) {
        UiText(if(ready) "Live desktop sessions" else "Last observed sessions · reconnect to control",style=MaterialTheme.typography.bodySmall)
    }
    val rows=inventory.sessions.filter {it.title.contains(query,ignoreCase=true)}
    var previousStatus: SessionStatus? = null
    rows.forEach {summary->
        if(preferences.groupByStatus && previousStatus!=summary.status) GoatrSection(sessionGroupLabel(summary.status))
        SessionPresentationRow(
            summary=summary,
            inventory=inventory,
            hostName=hostName,
            preferences=preferences,
            staleDisplay=!ready,
            rowTag="session_${summary.sessionId.value}_${summary.generation.value}",
            onOpen=onOpen,
        )
        previousStatus=summary.status
    }
    if(inventory.unbound.isNotEmpty()) {
        Heading("Desktop setup required")
        UiText("${inventory.unbound.size} desktop ${if(inventory.unbound.size==1) "row needs" else "rows need"} setup before it can be opened from this phone.",style=MaterialTheme.typography.bodySmall)
        inventory.unbound.forEach { UnboundSessionRow(it) }
    }
    if(rows.isEmpty() && (query.isNotEmpty() || inventory.unbound.isEmpty())) UiText(
        when {
            query.isNotEmpty() -> "No matching conversations."
            !ready -> "Desktop inventory is synchronizing. Refresh after desktop report/control setup; no phone Target is available yet."
            query.isEmpty() -> "No supported conversations are currently bound. Complete desktop report/control setup, then refresh."
            else -> "No matching conversations."
        },
    )
}

@Composable
private fun SessionPresentationRow(
    summary: SessionSummary,
    inventory: HostSnapshotResult,
    hostName: String?,
    preferences: SessionListPreferences,
    staleDisplay: Boolean,
    rowTag: String,
    onOpen: (Target) -> Unit,
) {
    val target=Target(summary.sessionId,summary.generation)
    val workspace=sessionWorkspaceLabel(inventory,summary)
    val subtitle=listOfNotNull(
        providerLabel(summary.provider),
        hostName.takeIf {preferences.showHostContext},
        workspace?.takeIf {preferences.showWorkspaceContext},
        summary.connectivity.toString(),
        if(staleDisplay) "stale display" else null,
        summary.reason,
    ).joinToString(" · ")
    CompactRow(
        conversationTitle(summary),
        subtitle,
        status={StatusPill(sessionStatusLabel(summary.status),sessionStatusEmphasis(summary.status))},
        modifier=Modifier.testTag(rowTag),
        onClick={onOpen(target)},
    )
    var details by remember(summary.sessionId,summary.generation) {mutableStateOf(false)}
    DetailDisclosure(details,{details=!details},"${rowTag}_details")
    if(details) GroupedSurface {
        UiText("Provider: ${providerLabel(summary.provider)}",style=MaterialTheme.typography.bodySmall)
        hostName?.let {UiText("Desktop: $it",style=MaterialTheme.typography.bodySmall)}
        workspace?.let {UiText("Workspace: $it",style=MaterialTheme.typography.bodySmall)}
        UiText("Authoritative status: ${sessionStatusLabel(summary.status)}",style=MaterialTheme.typography.bodySmall)
        UiText("Session ID: ${target.sessionId.value}",style=MaterialTheme.typography.bodySmall)
        UiText("Generation: ${target.generation.value}",style=MaterialTheme.typography.bodySmall)
        Action("Open this conversation","${rowTag}_open") {onOpen(target)}
    }
}

private fun sessionStatusEmphasis(status: SessionStatus): StatusEmphasis = when(status) {
    SessionStatus.Attention -> StatusEmphasis.Attention
    SessionStatus.Working -> StatusEmphasis.Positive
    SessionStatus.Unknown -> StatusEmphasis.Muted
    SessionStatus.Idle -> StatusEmphasis.Neutral
}

/** A returned setup-only desktop row has no Target and is deliberately not openable. */
@Composable
private fun UnboundSessionRow(row: UnboundRow) {
    val provider=providerLabel(row.provider)
    val title=if(row.title.equals(provider,ignoreCase=true)) provider else row.title
    val subtitle=if(title==provider) unboundStateLabel(row.state) else "$provider · ${unboundStateLabel(row.state)}"
    CompactRow(title,subtitle,status={StatusPill("Setup",StatusEmphasis.Muted)},modifier=Modifier.testTag("unbound_${row.rowId.value}"))
    var details by remember(row.rowId) {mutableStateOf(false)}
    DetailDisclosure(details,{details=!details},"unbound_details_${row.rowId.value}")
    if(details) UiText(row.reason,style=MaterialTheme.typography.bodySmall)
}

/** The landing console groups exact native Targets by paired host without selecting one. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SessionsConsole(store: StoreView,connection: ConnectionView,model: GoatrViewModel,onNewSession: (String)->Unit) {
    var filter by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    var providerFilter by remember { mutableStateOf<Provider?>(null) }
    var statusFilter by remember { mutableStateOf<SessionStatus?>(null) }
    val narrowed=query.isNotEmpty() || providerFilter!=null || statusFilter!=null
    fun matches(summary:SessionSummary):Boolean=summary.title.contains(query,ignoreCase=true) && (providerFilter==null || summary.provider==providerFilter) && (statusFilter==null || summary.status==statusFilter)
    var choosingCreationHost by remember { mutableStateOf(false) }
    val hosts=store.hosts.filter { filter==null || it.hostId==filter }
    val creationHosts=hosts.filter {paired->(connection.inventory[paired.hostId] ?: store.inventory.firstOrNull {it.hostId==paired.hostId}?.inventory) != null}
    LaunchedEffect(choosingCreationHost,creationHosts.map {it.hostId}) {if(choosingCreationHost && creationHosts.isEmpty()) choosingCreationHost=false}
    // The persistent extended FAB needs a scroll-only exclusion zone. Scaffold already
    // consumes the navigation inset; this reserves the FAB, its margin, and a clear gap.
    val sessionsFabClearance=112.dp
    Box(Modifier.fillMaxSize()) {
    Column(Modifier.fillMaxSize(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        GoatrSection("Your sessions", "Pick up a conversation from any paired desktop.")
        if(store.hosts.isNotEmpty()) {
            FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                FilterChip(selected=filter==null,onClick={filter=null},label={UiText("All hosts")},modifier=Modifier.heightIn(min=48.dp).testTag("host_filter_all"))
                store.hosts.forEach { paired->FilterChip(selected=filter==paired.hostId,onClick={filter=paired.hostId},label={UiText(paired.name)},modifier=Modifier.heightIn(min=48.dp).testTag("host_filter_${paired.hostId}"))}
            }
            OutlinedTextField(query,{query=it},label={UiText("Find sessions")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("session_search"))
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                FilterChip(selected=providerFilter==null,onClick={providerFilter=null},label={UiText("All providers")},modifier=Modifier.heightIn(min=48.dp).testTag("session_provider_all"))
                Provider.entries.forEach {provider->FilterChip(selected=providerFilter==provider,onClick={providerFilter=provider},label={UiText(providerLabel(provider))},modifier=Modifier.heightIn(min=48.dp).testTag("session_provider_${provider.name}"))}
            }
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                FilterChip(selected=statusFilter==null,onClick={statusFilter=null},label={UiText("All statuses")},modifier=Modifier.heightIn(min=48.dp).testTag("session_status_all"))
                SessionStatus.entries.forEach {status->FilterChip(selected=statusFilter==status,onClick={statusFilter=status},label={UiText(sessionGroupLabel(status))},modifier=Modifier.heightIn(min=48.dp).testTag("session_status_${status.name}"))}
            }
            if(statusFilter!=null) UiText("Cached sessions use their last retained status, not a live status check.",style=MaterialTheme.typography.bodySmall)

        }
        LazyColumn(Modifier.weight(1f).fillMaxWidth().testTag("sessions_console"),verticalArrangement=Arrangement.spacedBy(6.dp),contentPadding=PaddingValues(bottom=if(creationHosts.isEmpty()) 16.dp else sessionsFabClearance)) {
            if(store.hosts.isEmpty()) item {
                EmptyState("Pair a desktop to begin","Goatr can show and control supported sessions only after you explicitly pair this phone with a same-user desktop.") {
                    IconAction(MaterialGlyph.Scan,"Scan or paste pairing invitation","pair_begin",enabled=!model.busy,onClick=model::beginPairing)
                }
            }
            hosts.forEach { paired ->
                val inventory=connection.inventory[paired.hostId] ?: store.inventory.firstOrNull {it.hostId==paired.hostId}?.inventory
                val online=paired.hostId in connection.hello
                val ready=connection.inventoryReady[paired.hostId]==true
                val liveTargets=inventory?.sessions?.map {Target(it.sessionId,it.generation)}.orEmpty().toSet()
                // Deduplicate by the original Target before applying the same presentation predicate.
                val retainedRows=store.cached.filter {it.hostId==paired.hostId && Target(it.summary.sessionId,it.summary.generation) !in liveTargets}
                val cachedRows=retainedRows.filter {matches(it.summary)}
                val rows=inventory?.sessions.orEmpty().filter(::matches)
                val hasRecordedSessions=inventory?.sessions?.isNotEmpty()==true || retainedRows.isNotEmpty()
                item(key="host_${paired.hostId}") {
                    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        GoatrSection(paired.name, connectionLabel(paired.hostId,connection,model.connectingHosts[paired.hostId]==true))
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                            IconAction(MaterialGlyph.Notification,"Unread activity","console_attention_inbox_${paired.hostId}",enabled=!model.busy&&model.foreground) {model.openConsoleAttentionInbox(paired.hostId)}
                            if(!online) IconAction(MaterialGlyph.Refresh,"Reconnect","host_reconnect_${paired.hostId}",enabled=!model.busy && model.foreground) {model.reconnect(paired.hostId)}
                            if(inventory!=null) {
                                if(online) IconAction(MaterialGlyph.Add,"New session","new_session_${paired.hostId}",enabled=!model.busy) {onNewSession(paired.hostId)}
                                else OutlinedButton(onClick={onNewSession(paired.hostId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("new_session_${paired.hostId}")) {IconLabel(MaterialGlyph.Add,"New session")}
                            }
                        }
                        if(!online) {
                            connection.connectionFailures[paired.hostId]?.let {failure->UiText(connectionFailureCopy(failure),style=MaterialTheme.typography.bodySmall)}
                            TextButton(onClick={model.settings(paired.hostId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("host_settings_${paired.hostId}")) {IconLabel(MaterialGlyph.Settings,"Host details")}
                        }
                    }
                }
                if(inventory==null) item(key="host_missing_${paired.hostId}") {
                    GroupedSurface {
                        UiText("No retained session inventory.",style=MaterialTheme.typography.titleSmall)
                        UiText("Reconnect this host to read supported desktop sessions.",style=MaterialTheme.typography.bodySmall)
                    }
                } else {
                    var previousStatus: SessionStatus? = null
                    rows.forEach {summary->
                        if(model.sessionListPreferences.groupByStatus && previousStatus!=summary.status) {
                            item(key="status_${paired.hostId}_${summary.status.name}_${summary.sessionId.value}_${summary.generation.value}") { GoatrSection(sessionGroupLabel(summary.status)) }
                        }
                        item(key="session_${paired.hostId}_${summary.sessionId.value}_${summary.generation.value}") {
                            SessionPresentationRow(
                                summary=summary,
                                inventory=inventory,
                                hostName=paired.name,
                                preferences=model.sessionListPreferences,
                                staleDisplay=!ready,
                                rowTag="session_${summary.sessionId.value}_${summary.generation.value}",
                                onOpen={target->model.open(paired.hostId,target)},
                            )
                        }
                        previousStatus=summary.status
                    }
                    inventory.unbound.forEach { row -> item(key="unbound_${paired.hostId}_${row.rowId.value}") { UnboundSessionRow(row) } }

                }
                if(rows.isEmpty() && cachedRows.isEmpty() && hasRecordedSessions && narrowed) item(key="filtered_empty_${paired.hostId}") {UiText("No sessions match these filters on this host.",style=MaterialTheme.typography.bodySmall)}
                else if(inventory!=null && !hasRecordedSessions && inventory.unbound.isEmpty()) item(key="empty_${paired.hostId}") {UiText("No supported phone conversations are bound on this host.",style=MaterialTheme.typography.bodySmall)}
                // Unbound setup rows stay separate; no provider or session status is inferred for them.
                cachedRows.forEach {cached->item(key="cached_${paired.hostId}_${cached.summary.sessionId.value}_${cached.summary.generation.value}") {
                    CompactRow(conversationTitle(cached.summary),"${providerLabel(cached.summary.provider)} · retained cache${if(cached.partial) " · partial" else ""}",status={StatusPill("Stale",StatusEmphasis.Muted)},modifier=Modifier.testTag("cached_session"),onClick={model.open(paired.hostId,Target(cached.summary.sessionId,cached.summary.generation))})
                }}
                item(key="host_actions_${paired.hostId}") { FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    IconAction(MaterialGlyph.Terminal,"Terminals","host_terminals_${paired.hostId}",enabled=!model.busy) {model.terminals(paired.hostId)}
                    IconAction(MaterialGlyph.Desktop,"View host","host_sessions_${paired.hostId}",enabled=!model.busy) {model.sessions(paired.hostId)}
                    if(online) TextButton(onClick={model.settings(paired.hostId)},modifier=Modifier.heightIn(min=48.dp).testTag("host_settings_${paired.hostId}")) {IconLabel(MaterialGlyph.Settings,"Host settings")}
                } }
            }
            if(store.hosts.isNotEmpty()) item("add_host") { TextButton(onClick=model::beginPairing,enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("pair_begin")) {IconLabel(MaterialGlyph.Scan,"Add desktop") } }
        }
    }
    if(creationHosts.isNotEmpty()) PrimaryActionFab(MaterialGlyph.Add,"New session","new_session",enabled=!model.busy,onClick={
        if(creationHosts.size==1) onNewSession(creationHosts.single().hostId) else choosingCreationHost=true
    },modifier=Modifier.align(androidx.compose.ui.Alignment.BottomEnd).padding(16.dp))
    if(choosingCreationHost) ModalBottomSheet(onDismissRequest={choosingCreationHost=false},containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().semantics {testTagsAsResourceId=true}.navigationBarsPadding().padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            GoatrSection("Choose a desktop","Start a new session only on the desktop you select.")
            val currentCreationHosts=creationHosts
            if(currentCreationHosts.isEmpty()) UiText("Session inventory is no longer available for this selection.",style=MaterialTheme.typography.bodySmall)
            currentCreationHosts.forEach {paired->
                IconAction(MaterialGlyph.Add,paired.name,"landing_new_session_host_${paired.hostId}",enabled=!model.busy) {
                    val stillEligible=!model.busy && store.hosts.any {it.hostId==paired.hostId} && ((connection.inventory[paired.hostId] ?: store.inventory.firstOrNull {it.hostId==paired.hostId}?.inventory) != null)
                    choosingCreationHost=false
                    if(stillEligible) onNewSession(paired.hostId)
                }
            }
            TextButton(onClick={choosingCreationHost=false},modifier=Modifier.heightIn(min=48.dp).align(androidx.compose.ui.Alignment.End).testTag("landing_new_session_close")) {IconLabel(MaterialGlyph.Close,"Close")}
        }
    }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HostSessionsScreen(host: String,store: StoreView,connection: ConnectionView,model: GoatrViewModel,openCreation: Boolean,onCreationRequestConsumed: ()->Unit) {
    val inventory=connection.inventory[host] ?: store.inventory.firstOrNull {it.hostId==host}?.inventory
    val focused=model.focusedAction?.takeIf {it.metadata.method==ActionMethod.SessionCreate}
    val currentCreation=model.creationActions[host]
    val olderFocused=currentCreation!=null && focused?.metadata?.actionId!=currentCreation
    var topology by remember(host) {mutableStateOf(false)}
    var createSheet by remember(host) {mutableStateOf(false)}
    LaunchedEffect(openCreation,inventory) {
        if(openCreation && inventory!=null) {
            createSheet=true
            onCreationRequestConsumed()
        }
    }
    val ongoing=creationOngoing(host,inventory,model,focused)
    Box(Modifier.fillMaxSize()) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom=80.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
            GoatrSection(store.hosts.firstOrNull {it.hostId==host}?.name ?: "Host sessions",modifier=Modifier.weight(1f))
            Column(horizontalAlignment=androidx.compose.ui.Alignment.End) {
                StatusPill(connectionLabel(host,connection,model.connectingHosts[host]==true),if(host in connection.hello) StatusEmphasis.Positive else StatusEmphasis.Attention)
                RefreshHint(inventoryRefreshing(host,connection))
            }
        }
        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            IconAction(MaterialGlyph.Refresh,"Refresh sessions","inventory_refresh",enabled=!model.busy && host in connection.hello,onClick=model::refresh)
            IconAction(MaterialGlyph.Terminal,"Terminals","host_terminals_$host",enabled=!model.busy) {model.terminals(host)}
            IconAction(MaterialGlyph.Notification,"Unread activity","attention_inbox_$host",enabled=!model.busy && model.foreground) {model.openAttentionInbox(host)}
        }
        if(inventory!=null && ongoing) UiText("A creation action is already reserved/running or awaiting inspection. Refresh its receipt below; never duplicate it after rotation or the 120-second response.")
        focused?.let { action->ReceiptPanel(host,action,model,onSeparateCreation=if(olderFocused) null else ({model.prepareSeparateCreation(host,action.metadata.actionId)})) }
        if(olderFocused) currentCreation?.let {current->
            UiText("A newer creation action is current. Inspect it before starting another session.",style=MaterialTheme.typography.bodySmall)
            Action("Inspect current creation","current_creation_inspect_${current.value}",enabled=!model.busy) {model.inspect(host,current)}
        }
        if(inventory==null) UiText("No cached inventory. Reconnect to read this host.") else {
            SessionBrowser(inventory,connection.inventoryReady[host]==true,model.sessionListPreferences,store.hosts.firstOrNull {paired->paired.hostId==host}?.name) {model.open(host,it)}
            TextButton(onClick={topology=!topology},modifier=Modifier.testTag("topology_details")) {UiText(if(topology) "Hide desktop details" else "Desktop details")}
            if(topology) {
                UiText("Desktop focus never selects a phone conversation.",style=MaterialTheme.typography.bodySmall)
                InventoryRows(inventory,connection.inventoryReady[host]==true) {model.open(host,it)}
            }
        }
        store.cached.filter {it.hostId==host}.forEach {cache->
            Heading("Offline cached conversation ${cache.summary.title}");UiText("Stale${if(cache.partial) " · partial" else ""}")
            Action("Read stale cache","cached_session") {model.open(host,Target(cache.summary.sessionId,cache.summary.generation))}
        }
        model.actions[host].orEmpty().filter {it.metadata.method==ActionMethod.SessionCreate && it.metadata.actionId!=focused?.metadata?.actionId}.forEach {ReceiptPanel(host,it,model)}
        ActionPaging(model,collapsed=true)
    }
    if(inventory!=null) PrimaryActionFab(MaterialGlyph.Add,"New session","new_session",enabled=!model.busy,onClick={createSheet=true},modifier=Modifier.align(androidx.compose.ui.Alignment.BottomEnd).padding(16.dp))
    }
    val separate=model.separateCreationAction
    if(separate!=null) {
        val action=focused?.takeIf {it.metadata.actionId==separate && !olderFocused}
        if(action==null) LaunchedEffect(separate) {model.dismissSeparateCreation(separate)}
        else SeparateCreationConfirmation(host,action,model) {createSheet=true}
    }
    if(createSheet) inventory?.let {HostCreationSheet(host,it,connection,model,onDismiss={createSheet=false;onCreationRequestConsumed()})}
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AttentionInboxDialog(shown:AttentionInboxState,store:StoreView,connection:ConnectionView,model:GoatrViewModel) {
    Dialog(onDismissRequest=model::closeAttentionInbox) {
        Surface(Modifier.fillMaxWidth().heightIn(max=600.dp).testTag("attention_inbox").semantics {testTagsAsResourceId=true},shape=MaterialTheme.shapes.extraLarge) {
            Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                    GoatrIcon(MaterialGlyph.Notification,null)
                    UiText("Unread activity",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).padding(start=8.dp).semantics {heading()})
                    IconAction(MaterialGlyph.Close,"Close unread activity","attention_inbox_close",onClick=model::closeAttentionInbox)
                }
                UiText(store.hosts.firstOrNull {it.hostId==shown.host}?.name ?: "Paired desktop",style=MaterialTheme.typography.labelLarge,maxLines=2,overflow=TextOverflow.Ellipsis)
                UiText("Up to 100 newest unread entries retained on this phone. Opening checks the original session; older sessions may show read-only history.",style=MaterialTheme.typography.bodySmall)
                TextButton(onClick={model.refreshAttentionInbox(shown)},enabled=!shown.loading,modifier=Modifier.heightIn(min=48.dp).testTag("attention_inbox_refresh")) {IconLabel(MaterialGlyph.Refresh,"Refresh retained list")}
                when {
                    shown.loading->Row(Modifier.heightIn(min=48.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {CircularProgressIndicator(Modifier.size(20.dp));UiText("Reading retained activity…",style=MaterialTheme.typography.bodyMedium)}
                    shown.error->UiText("Unread activity could not be read. Refresh to try again.",style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.error)
                    shown.entries.isEmpty()->UiText("No unread activity is retained on this phone.",style=MaterialTheme.typography.bodyMedium)
                    else->LazyColumn(Modifier.fillMaxWidth().heightIn(max=360.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        items(shown.entries,key={it.event.eventId.value}) {entry->
                            val event=entry.event
                            val summary=connection.sessions[shown.host]?.firstOrNull {it.state?.summary?.let {s->s.sessionId==event.sessionId && s.generation==event.generation}==true}?.state?.summary
                                ?: store.cached.firstOrNull {it.hostId==shown.host && it.summary.sessionId==event.sessionId && it.summary.generation==event.generation}?.summary
                            OutlinedCard(onClick={model.openAttentionInboxEntry(shown,entry)},enabled=!model.busy,modifier=Modifier.fillMaxWidth().heightIn(min=64.dp)) {
                                Column(Modifier.padding(12.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                                    UiText(if(event.category==AttentionCategory.Attention) "Needs attention" else "Work completed",style=MaterialTheme.typography.labelLarge)
                                    UiText(conversationTitle(summary),style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
                                    UiText(java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.SHORT,java.text.DateFormat.SHORT).format(java.util.Date(event.createdAt.value)),style=MaterialTheme.typography.labelSmall)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SeparateCreationConfirmation(host: String,action: LocalAction,model: GoatrViewModel,worktreeScope: WorktreeCreationRequestScope? = null,onConfirmed: ()->Unit) {
    val actionId=action.metadata.actionId.value
    val uncertain=action.receipt?.state==ReceiptState.Uncertain
    val resources=retainedCreationResources(action)
    val scroll=rememberScrollState()
    Dialog(onDismissRequest={model.dismissSeparateCreation(action.metadata.actionId)}) {
        Surface(Modifier.fillMaxWidth().heightIn(max=560.dp).testTag("separate_creation_$actionId").semantics {testTagsAsResourceId=true},shape=MaterialTheme.shapes.extraLarge,color=MaterialTheme.colorScheme.surface) {
            Column(Modifier.padding(24.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Column(Modifier.heightIn(max=320.dp).verticalScroll(scroll),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    UiText("Start a separate new session?",style=MaterialTheme.typography.titleLarge)
                    UiText("Prior action $actionId remains inspectable. This creates a new action; it does not retry or retarget the prior action.",style=MaterialTheme.typography.bodySmall)
                    if(uncertain) UiText("The prior action has an uncertain outcome and may already have effect. Known retained resources remain untouched.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error)
                    else UiText("Known retained resources remain untouched.",style=MaterialTheme.typography.bodySmall)
                    UiText("Known retained resources",style=MaterialTheme.typography.labelLarge)
                    if(resources.isEmpty()) UiText("No retained resource identifier was recorded.",style=MaterialTheme.typography.bodySmall)
                    else resources.forEach {UiText(it,style=MaterialTheme.typography.bodySmall)}
                }
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick={model.dismissSeparateCreation(action.metadata.actionId)},enabled=!model.busy) {IconLabel(MaterialGlyph.Close,"Cancel")}
                    IconAction(MaterialGlyph.Add,"Start separate new session","separate_creation_confirm_$actionId",enabled=!model.busy) {if(worktreeScope==null) model.confirmSeparateCreation(host,action.metadata.actionId,onConfirmed) else model.confirmSeparateWorktree(worktreeScope,action.metadata.actionId,onConfirmed)}
                }
            }
        }
    }
}

private fun retainedCreationResources(action: LocalAction): List<String> = action.receipt?.creation?.resources?.let {resources->
    listOfNotNull(
        resources.checkout?.let {"Branch ${it.branch}"},
        resources.checkout?.requestedBase?.let {"Base $it"},
        resources.checkout?.resolvedCommit?.let {"Commit $it"},
        resources.workspaceId?.let {"Workspace ${it.value}"},
        resources.paneId?.let {"Pane ${it.value}"},
        resources.terminalId?.let {"Terminal ${it.value}"},
        resources.nativeId?.let {"Native ${it.value}"},
        resources.bindingId?.let {"Binding ${it.value}"},
        resources.sessionId?.let {"Session ${it.value}"},
        resources.generation?.let {"Generation ${it.value}"},
    )
}.orEmpty()

private fun creationOngoing(host: String,inventory: HostSnapshotResult?,model: GoatrViewModel,focused: LocalAction?): Boolean = inventory!=null && (
    model.creationActions.containsKey(host) ||
        focused?.let {it.receipt==null || it.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching)}==true ||
        model.actions[host].orEmpty().any {it.metadata.method==ActionMethod.SessionCreate && (it.receipt==null || it.receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching))}
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WorktreeCreationSheet(key: ConversationKey, inventory: HostSnapshotResult, summary: SessionSummary, listing: WorktreesSnapshot, model: GoatrViewModel, onDismiss: () -> Unit) {
    val scope=listing.creationScope
    val requestScope=remember(key,listing.source,listing.ticket) {WorktreeCreationRequestScope(key,listing.source,listing.ticket)}
    LaunchedEffect(requestScope) {model.loadWorktreeCreation(requestScope)}
    val creationGate=model.worktreeCreationGate?.takeIf {it.scope==requestScope}
    val project=scope?.projectId
    var provider by remember(key,listing.source,listing.ticket) {mutableStateOf(summary.provider)}
    var name by remember(key,listing.source,listing.ticket) {mutableStateOf("")}
    var existing by remember(key,listing.source,listing.ticket) {mutableStateOf(false)}
    var branch by remember(key,listing.source,listing.ticket) {mutableStateOf("")}
    var base by remember(key,listing.source,listing.ticket) {mutableStateOf("")}
    val server=inventory.servers.singleOrNull {it.serverId==summary.serverId}
    val selectedProject=inventory.projects.singleOrNull {it.projectId==project && it.available}
    val connection=model.connection.state.value
    val sourceCurrent=model.foreground && model.selection==key && model.screen==Screen.Conversation && connection.sources[key.host]?.get(key.target)==listing.source && connection.mutationReady[key.host]==true && connection.inventoryReady[key.host]==true
    val validCheckout=runCatching {if(existing) CreationCheckout.Existing(branch.trim()).validate() else CreationCheckout.New(branch.trim().takeIf {it.isNotEmpty()},base.trim().takeIf {it.isNotEmpty()}).validate()}.isSuccess
    val providerReady=server?.let {if(provider==Provider.Omp) it.canCreate.omp else it.canCreate.codex}==true
    val available=scope?.canCreate==true && selectedProject!=null && summary.capabilities.worktreeCreate && sourceCurrent && providerReady && name.isNotBlank() && runCatching {Label(name)}.isSuccess && validCheckout && !model.busy && model.worktreeCreationAllowed(requestScope)
    ModalBottomSheet(onDismissRequest=onDismiss,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true),containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(16.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(10.dp)) {
            Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                UiText("Start in worktree",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).semantics {heading()})
                TextButton(onClick=onDismiss,modifier=Modifier.testTag("worktree_create_close")) {IconLabel(MaterialGlyph.Close,"Close")}
            }
            UiText(selectedProject?.label?.value ?: "No authorized project is available for this source.",style=MaterialTheme.typography.bodyMedium,modifier=Modifier.testTag("worktree_create_project"))
            if(creationGate?.loaded!=true) UiText("Checking retained creation actions…",style=MaterialTheme.typography.bodySmall)
            creationGate?.prior?.let {action->
                UiText("Inspect the retained creation before starting a separate action.",style=MaterialTheme.typography.bodySmall)
                ReceiptPanel(key.host,action,model,onSeparateCreation={model.prepareSeparateWorktree(requestScope,action.metadata.actionId)})
            }
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                listOf(Provider.Omp,Provider.Codex).forEachIndexed {index,option->
                    SegmentedButton(selected=provider==option,onClick={provider=option},enabled=server?.let {if(option==Provider.Omp) it.canCreate.omp else it.canCreate.codex}==true,shape=SegmentedButtonDefaults.itemShape(index,2),label={UiText(providerLabel(option))},modifier=Modifier.testTag("worktree_create_provider_"+option.name.lowercase()))
                }
            }
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                SegmentedButton(selected=!existing,onClick={existing=false},shape=SegmentedButtonDefaults.itemShape(0,2),label={UiText("New branch")},modifier=Modifier.testTag("worktree_create_new"))
                SegmentedButton(selected=existing,onClick={existing=true},shape=SegmentedButtonDefaults.itemShape(1,2),label={UiText("Existing branch")},modifier=Modifier.testTag("worktree_create_existing"))
            }
            OutlinedTextField(name,{name=it},label={UiText("Session name")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("worktree_create_name"))
            OutlinedTextField(branch,{branch=it},label={UiText(if(existing) "Existing branch" else "New branch (generated if blank)")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("worktree_create_branch"))
            if(!existing) OutlinedTextField(base,{base=it},label={UiText("Base (HEAD if blank)")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("worktree_create_base"))
            if(!sourceCurrent) UiText("Refresh this conversation before creating a checkout.",color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall)
            IconAction(MaterialGlyph.Add,"Create checkout and session","worktree_create_submit",enabled=available,modifier=Modifier.fillMaxWidth()) {
                val selected=project ?: return@IconAction
                model.createInWorktree(key,listing.source,listing.ticket,selected,provider,name,existing,branch,base)
            }
        }
    }
    if(model.separateWorktreeScope==requestScope) creationGate?.prior?.takeIf {it.metadata.actionId==model.separateCreationAction}?.let {action->
        SeparateCreationConfirmation(key.host,action,model,requestScope) {}
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HostCreationSheet(host: String,inventory: HostSnapshotResult,connection: ConnectionView,model: GoatrViewModel,onDismiss: ()->Unit) {
    val focused=model.focusedAction?.takeIf {it.metadata.method==ActionMethod.SessionCreate}
    val ongoing=creationOngoing(host,inventory,model,focused)
    var creationMemory by remember(host) {mutableStateOf<FormMemory?>(null)}
    LaunchedEffect(host) {creationMemory=model.creationForms.getOrPut(host) {FormMemory()}}
    ModalBottomSheet(onDismissRequest=onDismiss,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true),containerColor=MaterialTheme.colorScheme.surface) {
        BoxWithConstraints(Modifier.fillMaxWidth().heightIn(max=640.dp).semantics {testTagsAsResourceId=true}.navigationBarsPadding().imePadding()) {
            Column(Modifier.fillMaxSize().padding(16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                    UiText("New session",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).semantics {heading()})
                    TextButton(onClick=onDismiss,modifier=Modifier.testTag("new_session_close")) {IconLabel(MaterialGlyph.Close,"Close")}
                }
                creationMemory?.let {memory->Box(Modifier.weight(1f).fillMaxWidth()) {
                    val creationGate=creationReadinessReason(connection.hello[host],model.busy,connection.inventoryReady[host]==true,connection.mutationReady[host]==true,ongoing)
                    CreateSession(inventory,connection.hello[host],connection.inventoryReady[host]==true && connection.mutationReady[host]==true && !model.busy && !ongoing,memory,creationGate?.message,creationGate?.tag) {server,project,provider,name->
                        onDismiss()
                        model.create(server,project,provider,name)
                    }
                }}
            }
        }
    }
}

@Composable
fun CreateSession(inventory: HostSnapshotResult,hello: HelloResult?,enabled: Boolean,memory: FormMemory,gateMessage: String? = null,gateTag: String? = null,pinnedProvider:Provider?=null,requireProjectSelection:Boolean=false,onCreate: (Uuid,Uuid,Provider,String)->Unit) {
    var server by remember(inventory.servers) {mutableStateOf(memory.text["server"]?.let(::Uuid) ?: inventory.servers.firstOrNull()?.serverId)}
    var project by remember(inventory.projects) {mutableStateOf(memory.text["project"]?.let(::Uuid) ?: if(requireProjectSelection)null else inventory.projects.firstOrNull {it.available}?.projectId)}
    var provider by remember {mutableStateOf(pinnedProvider ?: memory.text["provider"]?.let(Provider::valueOf) ?: Provider.Omp)}
    val name=memory.text["name"].orEmpty()
    val submitted=memory.text["submitted"]=="yes"
    SideEffect {server?.let {memory.text["server"]=it.value};project?.let {memory.text["project"]=it.value};memory.text["provider"]=provider.name}
    val validName=runCatching {Label(name)}.isSuccess
    val selectedServer=inventory.servers.firstOrNull {it.serverId==server}
    val selectedProject=inventory.projects.firstOrNull {it.projectId==project}
    val selectedBackend=hello?.backends?.firstOrNull {it.provider==provider}
    val nativeCurrent=inventory.hierarchy.none {it.serverId==server && (it.freshness==NativeFreshness.Stale || it.reason==NativeUnavailableReason.SourceUnavailable)}
    // canCreate is authoritative guarded ATTEMPT eligibility, not an existing native binding.
    val available=enabled && selectedServer?.connectivity==ServerConnectivity.Online && nativeCurrent && (if(provider==Provider.Omp) selectedServer.canCreate.omp else selectedServer.canCreate.codex) && inventory.projects.any {it.projectId==project && it.available} && validName && !submitted
    val conditions=buildList {
            if(hello==null) add("Reconnect before creating a session.")
            if(!enabled) add(gateMessage ?: "Wait for the phone to finish synchronizing or its current operation.")
            when {
                selectedServer==null -> add("Choose a server.")
                selectedServer.connectivity!=ServerConnectivity.Online -> add("The selected server is not online.")
                !nativeCurrent -> add("Refresh the selected server's desktop source before creating a session.")
                !(if(provider==Provider.Omp) selectedServer.canCreate.omp else selectedServer.canCreate.codex) -> add("The selected server is not ready to create ${providerLabel(provider)} sessions.")
            }
            when {
                selectedProject==null -> add("Choose an available project.")
                !selectedProject.available -> add("Choose an available project.")
            }
            if(!validName) add(if(name.isEmpty()) "Name your session to continue." else "Enter a session name between 1 and 128 characters.")
    }
    val details=listOfNotNull(selectedServer?.reason,selectedBackend?.reason,selectedProject?.reason).distinct()
    Column(Modifier.fillMaxSize(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            UiText("Start a session without changing what is open on your desktop.",style=MaterialTheme.typography.bodySmall)
            val serverItems=inventory.servers.map {candidate->
                val sameName=inventory.servers.filter {it.name==candidate.name}
                val prefix=candidate.serverId.value.take(8)
                CreationPickerItem(
                    id=candidate.serverId,
                    label=candidate.name.value,
                    enabled=true,
                    secondary=if(sameName.size>1) "ID ${if(sameName.count {it.serverId.value.take(8)==prefix}>1) candidate.serverId.value else prefix}" else null,
                )
            }
            val projectItems=inventory.projects.map {candidate->CreationPickerItem(candidate.projectId,candidate.label.value,candidate.available)}
            CreationPicker("Server",server,"Choose a server","create_server",serverItems) {selectedId->server=selectedId}
            CreationPicker("Project",project,"Choose a project","create_project",projectItems) {selectedId->project=selectedId}
            if(inventory.projects.isEmpty()) UiText("No configured projects. New creation is unavailable; existing sessions remain host-wide accessible.")
            GroupedSurface {
                GoatrSection("Provider")
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {Provider.entries.forEach {p->
                    FilterChip(selected=provider==p,onClick={provider=p},enabled=pinnedProvider==null,label={UiText(providerLabel(p))},modifier=Modifier.heightIn(min=48.dp).testTag("create_provider"))
                }}
                OutlinedTextField(value=name,onValueChange={memory.text["name"]=it},label={UiText("Session name")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("create_name"))
            }
            if(!submitted && !available) {
                if(conditions.drop(1).isNotEmpty() || details.isNotEmpty()) {
            var showDetails by remember(server,project,provider) {mutableStateOf(false)}
            DetailDisclosure(showDetails,{showDetails=!showDetails},"create_setup_details")
            if(showDetails) (conditions.drop(1)+details).distinct().forEach {UiText(it,style=MaterialTheme.typography.bodySmall)}
                }
            }
        }
        Surface(color=MaterialTheme.colorScheme.surface,tonalElevation=3.dp,modifier=Modifier.fillMaxWidth()) {
            Column(Modifier.fillMaxWidth().padding(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                when {
                    submitted -> UiText("Creation submitted once. Inspect and refresh its receipt; do not create another session for this operation.",style=MaterialTheme.typography.bodySmall)
                    !available -> Box(if(gateTag==null) Modifier else Modifier.testTag(gateTag)) {UiText(conditions.firstOrNull() ?: "Creation is unavailable until the selected desktop setup is ready.",color=if(gateTag!=null || (name.isEmpty() && conditions.size==1 && conditions.first()=="Name your session to continue.") || conditions.firstOrNull()=="Wait for the phone to finish synchronizing or its current operation.") MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("create_blocking_reason"))}
                    else -> UiText("Ready to create your session.",style=MaterialTheme.typography.bodySmall)
                }
                IconAction(MaterialGlyph.Add,"Create session","session_create",enabled=available,modifier=Modifier.fillMaxWidth()) { memory.text["submitted"]="yes";onCreate(server!!,project!!,provider,name) }
            }
        }
    }
}

@Composable
private fun CreationPicker(label: String,selectedId: Uuid?,placeholder: String,tag: String,items: List<CreationPickerItem>,onSelect: (Uuid)->Unit) {
    var expanded by remember(label,items) {mutableStateOf(false)}
    val selection=items.firstOrNull {it.id==selectedId}
    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
        UiText(label,style=MaterialTheme.typography.labelLarge,modifier=Modifier.widthIn(min=64.dp))
        Box(Modifier.weight(1f)) {
            OutlinedButton(onClick={expanded=true},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag(tag)) {
                UiText(selection?.let {item->listOfNotNull(item.label,item.secondary).joinToString(" · ")} ?: placeholder,modifier=Modifier.weight(1f).testTag(selection?.let {"creation_${tag.removePrefix("create_")}_selected_id_${it.id.value}"} ?: "creation_${tag.removePrefix("create_")}_selected_empty"),maxLines=1,overflow=TextOverflow.Ellipsis)
                GoatrIcon(MaterialGlyph.ExpandMore,null)
            }
            DropdownMenu(expanded=expanded,onDismissRequest={expanded=false},modifier=Modifier.semantics {testTagsAsResourceId=true}) {
                items.forEachIndexed {index,item->key(item.id) {
                    DropdownMenuItem(
                        text={Column {UiText(item.label,modifier=Modifier.testTag("creation_${tag.removePrefix("create_")}_option_id_${item.id.value}"));item.secondary?.let {UiText(it,style=MaterialTheme.typography.bodySmall)} }},
                        enabled=item.enabled,
                        onClick={expanded=false;onSelect(item.id)},
                        modifier=Modifier.testTag("${tag}_$index"),
                    )
                }}
            }
        }
    }
}

/** Presentation-only choice: selection remains bound to the protocol identity. */
private data class CreationPickerItem(val id: Uuid,val label: String,val enabled: Boolean,val secondary: String? = null)

internal data class ReadinessReason(val message: String,val tag: String)

internal fun creationReadinessReason(hello: HelloResult?,busy: Boolean,inventoryCurrent: Boolean,receiptsRecovered: Boolean,ongoing: Boolean): ReadinessReason? = when {
    hello==null -> ReadinessReason("Reconnect before creating a session.","creation_gate_transport")
    !inventoryCurrent -> ReadinessReason("Refresh this desktop's session inventory before creating a session.","creation_gate_inventory")
    !receiptsRecovered -> ReadinessReason("Finish receipt recovery before creating a session.","creation_gate_receipt")
    busy -> ReadinessReason("Wait for the current phone operation to finish.","creation_gate_operation")
    ongoing -> ReadinessReason("Inspect the current creation receipt before starting another session.","creation_gate_receipt")
    else -> null
}

/** Presentation only: keep the underlying availability conjunction authoritative. */
private fun conversationReadinessReason(
    foreground: Boolean,
    authenticated: Boolean,
    busy: Boolean,
    inventoryCurrent: Boolean,
    receiptsRecovered: Boolean,
    sourcePresent: Boolean,
    nativeCurrent: Boolean,
    mutationsEnabled: Boolean,
): ReadinessReason? = when {
    !foreground -> ReadinessReason("Return to Goatr before sending.","conversation_gate_foreground")
    !authenticated -> ReadinessReason("Reconnect to this desktop before sending.","conversation_gate_transport")
    busy -> ReadinessReason("Wait for the current phone operation to finish.","conversation_gate_operation")
    !inventoryCurrent -> ReadinessReason("Refresh this desktop's session inventory before sending.","conversation_gate_inventory")
    !receiptsRecovered -> ReadinessReason("Finish receipt recovery before sending.","conversation_gate_receipt")
    !nativeCurrent -> ReadinessReason("Refresh this conversation's desktop source before sending.","conversation_gate_native")
    !sourcePresent -> ReadinessReason("Refresh this conversation before sending.","conversation_gate_source")
    !mutationsEnabled -> ReadinessReason("This conversation is display-only until native updates are ready.","conversation_gate_native")
    else -> null
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConversationScreen(key: ConversationKey,store: StoreView,connection: ConnectionView,model: GoatrViewModel) {
    val reducer=connection.sessions[key.host]?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==key.target}==true}
    val cache=store.cached.firstOrNull {it.hostId==key.host && Target(it.summary.sessionId,it.summary.generation)==key.target}
    val summary=reducer?.state?.summary ?: cache?.summary
    val source=connection.sources[key.host]?.get(key.target)
    val inventory=connection.inventory[key.host]
    val sourceCurrent=summary!=null && inventory?.hierarchy?.none {it.serverId==summary.serverId && (it.freshness==NativeFreshness.Stale || it.reason==NativeUnavailableReason.SourceUnavailable)}==true && inventory.servers.none {it.serverId==summary.serverId && it.connectivity!=ServerConnectivity.Online}
    val ready=model.foreground && source!=null && reducer?.mutationsEnabled==true && connection.inventoryReady[key.host]==true && connection.mutationReady[key.host]==true && sourceCurrent && !model.busy
    val readiness=conversationReadinessReason(
        foreground=model.foreground,
        authenticated=key.host in connection.hello,
        busy=model.busy,
        inventoryCurrent=connection.inventoryReady[key.host]==true,
        receiptsRecovered=connection.mutationReady[key.host]==true,
        sourcePresent=source!=null,
        nativeCurrent=sourceCurrent,
        mutationsEnabled=reducer?.mutationsEnabled==true,
    )
    val promptAllowed=ready && summary?.capabilities?.prompt==true && summary.capabilities.promptMode==PromptMode.SubmitOrSteer
    val stopAllowed=ready && summary?.capabilities?.cancel==true && (summary.provider==Provider.Omp && summary.capabilities.cancelMode==CancelMode.CurrentWork || summary.provider==Provider.Codex && summary.capabilities.cancelMode==CancelMode.NativeTurn && reducer?.state?.nativeTurnId!=null)
    var switcher by remember {mutableStateOf(false)}
    var snippets by remember(key) {mutableStateOf(false)}
    LaunchedEffect(key,source,model.foreground,reducer?.pending?.map {it.requestKey},summary?.status) {if(source!=null && model.foreground) try {model.viewed(key)} catch(cancel: CancellationException) {throw cancel} catch(_: Exception) {}}
    if(switcher) ModalBottomSheet(onDismissRequest={switcher=false},containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.8f).semantics {testTagsAsResourceId=true}.padding(16.dp).testTag("switcher_scope_${key.target.sessionId.value}_${key.target.generation.value}"),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                UiText("Switch conversation",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f))
                TextButton(onClick={switcher=false},modifier=Modifier.heightIn(min=48.dp).testTag("switcher_close")) {IconLabel(MaterialGlyph.Close,"Close")}
            }
            // The dismiss action stays reachable while long inventories scroll independently.
            Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                inventory?.let {SessionBrowser(it,connection.inventoryReady[key.host]==true,model.sessionListPreferences,store.hosts.firstOrNull {paired->paired.hostId==key.host}?.name) {target->switcher=false;model.open(key.host,target)}} ?: UiText("Inventory unavailable")
            }
        }
    }
    model.skills?.takeIf {it.key==key && it.source==source && (it.generation==summary?.skillsGeneration||it.loading&&summary?.provider==Provider.Omp) && summary?.capabilities?.skillsRead==true && ready && model.foreground}?.let { SkillsPicker(it,model) }
    model.skillsDocument?.takeIf{it.snapshot.key==key&&it.snapshot.source==source&&it.snapshot.generation==summary?.skillsGeneration&&summary?.capabilities?.skillsRead==true&&ready&&model.foreground}?.let{SkillsDocument(it,model)}
    if(snippets) ComposerSnippetPicker(key,store.snippets.filter {it.hostId==key.host},model,model.composers[key].orEmpty().isEmpty()) {snippets=false}
    val pending=reducer?.pending.orEmpty()
    val history=reducer?.history ?: cache?.history.orEmpty()
    val focusedAction=model.focusedAction?.takeIf {(it.metadata.target==key.target || it.metadata.sourceTarget==key.target)}
    val latestTailIndex=when {
        history.isNotEmpty() -> history.size + if(focusedAction!=null) 1 else 0
        focusedAction!=null -> 1
        else -> 0
    }
    val transcript=rememberLazyListState()
    val scope=rememberCoroutineScope()
    var following by remember(key,source) {mutableStateOf(true)}
    var readTail by remember(key,source) {mutableStateOf<Id?>(null)}
    LaunchedEffect(transcript,key,source) {
        snapshotFlow {transcript.isScrollInProgress to transcript.canScrollForward}.collect {(scrolling,canForward)->if(scrolling || !canForward) following=!canForward}
    }
    LaunchedEffect(history.lastOrNull()?.id,focusedAction?.metadata?.actionId,following,reducer?.browsingOlder) {
        if(following && reducer?.browsingOlder!=true) {
            readTail=history.lastOrNull()?.id
            // Follow the completed native transcript, falling back to the focused receipt
            // only until history arrives. Stable native keys retain a reader's anchor.
            if(history.isNotEmpty() || focusedAction!=null) transcript.scrollToItem(latestTailIndex)
        }
    }
    var selectedRequest by remember(key,source) {mutableStateOf<Uuid?>(null)}
    val request=pending.firstOrNull {it.requestKey==selectedRequest}
    LaunchedEffect(pending.map {it.requestKey}) {if(request==null) selectedRequest=null}
    request?.let {active->
        var memory by remember(key,source,active.requestKey) {mutableStateOf<FormMemory?>(null)}
        LaunchedEffect(key,source,active.requestKey) {memory=model.formMemory(key,source,active.requestKey)}
        DisposableEffect(key,source,active.requestKey) {onDispose {memory?.clearSecrets()}}
        Dialog(onDismissRequest={selectedRequest=null},properties=DialogProperties(usePlatformDefaultWidth=false)) {
            Surface(Modifier.fillMaxSize().safeDrawingPadding().imePadding(),color=MaterialTheme.colorScheme.surface) {
                Column(Modifier.padding(16.dp)) {
                    Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                        UiText("Needs your answer",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).semantics {heading()})
                        TextButton(onClick={selectedRequest=null},modifier=Modifier.heightIn(min=48.dp).testTag("request_close")) {IconLabel(MaterialGlyph.Close,"Close")}
                    }
                    Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).semantics {paneTitle="Native request answer"}.testTag("request_editor_${active.requestKey.value}")) {
                        UiText("Closing keeps the native request open.",style=MaterialTheme.typography.bodySmall)
                        memory?.let {form->ProviderForm(active,form,ready && summary?.capabilities?.respond==true,"Reconnect and refresh this request before answering.",{content,loaded->ContentView(content,key,source,model,onLoaded=loaded)},{url->model.forms.values.forEach(FormMemory::clearSecrets);openHttpUrl(model.getApplication(),url)}) {answer->model.respond(active,answer)}}
                    }
                    OutlinedButton(onClick=model::stop,enabled=stopAllowed,modifier=Modifier.fillMaxWidth().heightIn(min=56.dp).testTag("request_stop")) {IconLabel(MaterialGlyph.Stop,"Stop current work")}
                }
            }
        }
    }
    model.terminal?.takeIf {it.key==key && it.source==source}?.let {terminal->
        Dialog(onDismissRequest=model::closeTerminal,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
            Surface(Modifier.fillMaxSize().safeDrawingPadding().testTag("terminal_dialog"),color=MaterialTheme.colorScheme.surface) {
                Column(Modifier.padding(16.dp).testTag("terminal_target_${key.target.sessionId.value}_${key.target.generation.value}")) {
                    Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                        UiText("Terminal snapshot",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).semantics {heading()})
                        TextButton(onClick=model::closeTerminal,modifier=Modifier.heightIn(min=48.dp).testTag("terminal_close")) {IconLabel(MaterialGlyph.Close,"Close")}
                    }
                    when {
                        terminal.loading -> UiText("Loading terminal snapshot",modifier=Modifier.testTag("terminal_loading"))
                        terminal.error -> UiText("Terminal snapshot is unavailable",modifier=Modifier.testTag("terminal_error"))
                        terminal.text.isNullOrEmpty() -> UiText("Terminal is empty",modifier=Modifier.testTag("terminal_empty"))
                        else -> UiText(terminal.text,style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,modifier=Modifier.weight(1f).verticalScroll(rememberScrollState()).testTag("terminal_output"))
                    }
                    if(terminal.truncated) UiText("Snapshot truncated to 16 KiB",style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("terminal_truncated"))
                    TextButton(onClick=model::refreshTerminal,enabled=!terminal.loading,modifier=Modifier.heightIn(min=48.dp).testTag("terminal_refresh")) {IconLabel(MaterialGlyph.Refresh,"Refresh")}
                }
            }
        }
    }
    model.worktrees?.takeIf {it.key==key && it.source==source}?.let {worktrees->
        var createCheckout by remember(key,source,worktrees.ticket) { mutableStateOf(false) }
        Dialog(onDismissRequest=model::closeWorktrees,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
            Surface(Modifier.fillMaxSize().safeDrawingPadding().testTag("worktrees_dialog"),color=MaterialTheme.colorScheme.surface) {
                Column(Modifier.padding(16.dp).testTag("worktrees_target_${key.target.sessionId.value}_${key.target.generation.value}"),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                        UiText("Worktrees",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).semantics {heading()})
                        TextButton(onClick=model::closeWorktrees,modifier=Modifier.heightIn(min=48.dp).testTag("worktrees_close")) {IconLabel(MaterialGlyph.Close,"Close")}
                    }
                    when {
                        worktrees.loading -> UiText("Loading worktrees",modifier=Modifier.testTag("worktrees_loading"))
                        worktrees.error -> UiText("Worktrees are unavailable",modifier=Modifier.testTag("worktrees_error"))
                        worktrees.worktrees.isNullOrEmpty() -> UiText("No worktrees are available",modifier=Modifier.testTag("worktrees_empty"))
                        else -> LazyColumn(Modifier.weight(1f).testTag("worktrees_list"),verticalArrangement=Arrangement.spacedBy(10.dp)) {
                            items(worktrees.worktrees,key={it.path}) { entry ->
                                GroupedSurface(Modifier.testTag("worktrees_entry")) {
                                    UiText(entry.label,style=MaterialTheme.typography.titleMedium)
                                    UiText(entry.path,style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,softWrap=true)
                                    entry.branch?.let {UiText("Branch: $it",style=MaterialTheme.typography.bodySmall)}
                                    entry.head?.let {UiText("Head: $it",style=MaterialTheme.typography.bodySmall)}
                                    val flags=buildList {
                                        if(entry.isBare) add("bare")
                                        if(entry.isDetached) add("detached")
                                        if(entry.isLocked) add("locked")
                                        if(entry.isPrunable) add("prunable")
                                        add(if(entry.isLinkedWorktree==true) "linked worktree" else if(entry.isLinkedWorktree==false) "main worktree" else "link status unknown")
                                    }
                                    UiText(flags.joinToString(" · "),style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                                    if(entry.candidateId!=null && summary?.capabilities?.worktreeRemove==true)IconAction(MaterialGlyph.Delete,"Remove checkout","worktree_remove_open",enabled=ready && !model.busy && !worktrees.loading && !worktrees.error){model.previewWorktreeRemoval(worktrees,entry)}
                                }
                            }
                        }
                    }
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick=model::refreshWorktrees,enabled=!worktrees.loading,modifier=Modifier.heightIn(min=48.dp).testTag("worktrees_refresh")) {IconLabel(MaterialGlyph.Refresh,"Refresh")}
                        IconAction(MaterialGlyph.Add,"Start session in checkout","worktrees_create",enabled=ready && !worktrees.loading && !worktrees.error && worktrees.creationScope?.canCreate==true && summary?.capabilities?.worktreeCreate==true) { createCheckout=true }
                    }
                }
            }
        }
        model.worktreeRemoval?.takeIf {it.key==key && it.source==source && it.ticket==worktrees.ticket}?.let {WorktreeRemovalDialog(it,model)}
        if(createCheckout) {
            val inventory=connection.inventory[key.host]
            val summary=connection.sessions[key.host]?.firstOrNull {it.state?.summary?.let {state->Target(state.sessionId,state.generation)==key.target}==true}?.state?.summary
            if(inventory==null || summary==null) createCheckout=false else WorktreeCreationSheet(key,inventory,summary,worktrees,model) {createCheckout=false}
        }
    }
    model.changes?.takeIf {it.key==key && it.source==source}?.let {changes->
        var pendingChangesReference by remember(key,source,changes.ticket) { mutableStateOf<ChangesReferenceSelection?>(null) }
        var changesReferenceError by remember(key,source,changes.ticket) { mutableStateOf<String?>(null) }
        val closeChangesDialog = { pendingChangesReference=null;changesReferenceError=null;model.closeChanges() }
        Dialog(onDismissRequest=closeChangesDialog,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
            Surface(Modifier.fillMaxSize().safeDrawingPadding().testTag("changes_dialog"),color=MaterialTheme.colorScheme.surface) {
                Column(Modifier.padding(16.dp).testTag("changes_target_${key.target.sessionId.value}_${key.target.generation.value}"),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                        UiText("Changes",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).semantics {heading()})
                        TextButton(onClick=closeChangesDialog,modifier=Modifier.heightIn(min=48.dp).testTag("changes_close")) {IconLabel(MaterialGlyph.Close,"Close")}
                    }
                    when {
                        changes.loading -> UiText("Loading changes",modifier=Modifier.testTag("changes_loading"))
                        changes.error -> UiText("Changes are unavailable",modifier=Modifier.testTag("changes_error"))
                        changes.changes == null -> UiText("No changes are available",modifier=Modifier.testTag("changes_empty"))
                        else -> Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).testTag("changes_body"),verticalArrangement=Arrangement.spacedBy(10.dp)) {
                            val value=changes.changes.changes
                            val revision=buildList { if(value.detached) add("Detached HEAD"); value.branch?.let { add("Branch: $it") }; value.head?.let { add("Head: $it") } }
                            if(revision.isNotEmpty()) UiText(revision.joinToString(" · "),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                            GroupedSurface(Modifier.testTag("changes_repository_context")) {
                                val context=value.repositoryContext
                                if(context==null) UiText("Repository context unavailable; this desktop needs native context support.",style=MaterialTheme.typography.bodySmall)
                                else {
                                    if(context.unborn) UiText("Unborn branch · no commit yet",style=MaterialTheme.typography.titleSmall)
                                    else if(value.detached) UiText("Detached HEAD",style=MaterialTheme.typography.titleSmall)
                                    context.latestCommit?.let {commit->
                                        UiText("Latest commit: "+commit.oid,style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,softWrap=true)
                                        UiText(commit.subject.ifEmpty {"(empty commit subject)"},style=MaterialTheme.typography.bodyMedium,softWrap=true)
                                    }
                                    val upstream=context.upstream
                                    if(upstream==null) UiText(if(value.detached) "No upstream for detached HEAD" else "No upstream reported",style=MaterialTheme.typography.bodySmall)
                                    else {
                                        UiText("Upstream: "+upstream.name,style=MaterialTheme.typography.bodySmall,softWrap=true)
                                        if(upstream.ahead!=null && upstream.behind!=null) UiText("Ahead "+upstream.ahead.value+" · Behind "+upstream.behind.value,style=MaterialTheme.typography.bodySmall)
                                        else UiText("Ahead/behind comparison unavailable",style=MaterialTheme.typography.bodySmall)
                                        UiText("Local refs only · no fetch performed",style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                                    }
                                }
                            }
                            changesReferenceError?.let {UiText(it,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error,modifier=Modifier.testTag("changes_reference_error"))}
                            if(value.entries.isEmpty()) UiText("Working tree is clean",modifier=Modifier.testTag("changes_empty"))
                            value.entries.forEach { entry ->
                                GroupedSurface(Modifier.testTag("changes_entry")) {
                                    when(entry) {
                                        is ChangeEntry.Tracked -> {
                                            UiText(entry.path,style=MaterialTheme.typography.titleSmall,fontFamily=FontFamily.Monospace,softWrap=true)
                                            entry.originalPath?.let {UiText("From: $it",style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,softWrap=true)}
                                            UiText("Staged: ${changeStateLabel(entry.index)} · Unstaged: ${changeStateLabel(entry.worktree)}",style=MaterialTheme.typography.bodySmall)
                                            val flags=buildList { if(entry.stagedBinary) add("staged binary marker"); if(entry.unstagedBinary) add("unstaged binary marker"); entry.submodule?.let { if(it.commitChanged) add("submodule commit changed"); if(it.trackedDirty) add("submodule tracked dirty"); if(it.untrackedDirty) add("submodule untracked dirty") } }
                                            if(flags.isNotEmpty()) UiText(flags.joinToString(" · "),style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                                            TextButton(onClick={pendingChangesReference=ChangesReferenceSelection.File(entry.path);changesReferenceError=null},modifier=Modifier.heightIn(min=48.dp).testTag("changes_reference_file")) {IconLabel(MaterialGlyph.Add,"Add file reference")}
                                        }
                                        is ChangeEntry.Untracked -> {
                                            UiText(entry.path,style=MaterialTheme.typography.titleSmall,fontFamily=FontFamily.Monospace,softWrap=true)
                                            UiText("Untracked",style=MaterialTheme.typography.bodySmall)
                                            TextButton(onClick={pendingChangesReference=ChangesReferenceSelection.File(entry.path);changesReferenceError=null},modifier=Modifier.heightIn(min=48.dp).testTag("changes_reference_file")) {IconLabel(MaterialGlyph.Add,"Add file reference")}
                                        }
                                    }
                                }
                            }
                            ChangesPatchReferenceSections(value.stagedPatch,value.unstagedPatch,value.entries.mapTo(mutableSetOf()) {it.path}) { selection->pendingChangesReference=selection;changesReferenceError=null }
                        }
                    }
                    TextButton(onClick=model::refreshChanges,enabled=!changes.loading,modifier=Modifier.heightIn(min=48.dp).testTag("changes_refresh")) {IconLabel(MaterialGlyph.Refresh,"Refresh")}
                }
            }
        }
        pendingChangesReference?.let { selection ->
            val reference=changes.changes?.changes?.let {value->resolveChangesReference(selection,value.stagedPatch,value.unstagedPatch,value.entries.mapTo(mutableSetOf()) {it.path})}
            if(reference==null) {
                pendingChangesReference=null
                changesReferenceError="This Changes snapshot cannot provide that reference. Refresh before trying again."
            } else
            AlertDialog(
                onDismissRequest={pendingChangesReference=null},
                title={UiText("Add snapshot reference?")},
                text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    UiText("This adds plain text to the current conversation draft. It does not upload, open, share, or send anything.")
                    UiText("Draft text",style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                    UiText(reference.draftText,style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,softWrap=true)
                    UiText("Path: ${reference.path}",style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,softWrap=true)
                    UiText("Snapshot ${reference.sourceLabel} · ${reference.lineContext}",style=MaterialTheme.typography.bodySmall)
                    Box(Modifier.fillMaxWidth().heightIn(max=220.dp).verticalScroll(rememberScrollState())) {UiText(reference.snapshotText,style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,softWrap=true)}
                }},
                confirmButton={TextButton(onClick={
                    if(!model.insertChangesReference(changes,selection)) changesReferenceError=model.notice ?: "The reference was not added."
                    pendingChangesReference=null
                },modifier=Modifier.heightIn(min=48.dp).testTag("changes_reference_confirm")) {IconLabel(MaterialGlyph.Add,"Add to draft")}},
                dismissButton={TextButton(onClick={pendingChangesReference=null},modifier=Modifier.heightIn(min=48.dp).testTag("changes_reference_cancel")) {IconLabel(MaterialGlyph.Close,"Cancel")}},
                modifier=Modifier.semantics { testTagsAsResourceId = true }.testTag("changes_body"),
                properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
            )
        }
    }
    var aliasScope by remember(key,source) {mutableStateOf<AgentAliasScope?>(null)}
    var aliasName by remember(key,source) {mutableStateOf("")}
    val aliasAvailable=model.foreground && ready && source!=null && summary?.capabilities?.agentRename==true
    DisposableEffect(key,source) {onDispose {aliasScope?.let {if(model.agentAliasScope===it)model.closeAgentAlias()}}}
    LaunchedEffect(aliasAvailable) {if(!aliasAvailable){aliasScope?.let {if(model.agentAliasScope===it)model.closeAgentAlias()};aliasScope=null;aliasName=""}}
    aliasScope?.takeIf {aliasAvailable && model.agentAliasScope===it}?.let {scope ->
        AlertDialog(
            onDismissRequest={model.closeAgentAlias();aliasScope=null;aliasName=""},
            title={UiText("Agent alias")},
            text={Column(Modifier.imePadding(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                UiText("Rename this desktop agent’s alias. Its conversation and workspace stay the same.")
                OutlinedTextField(aliasName,{aliasName=it},label={UiText("Agent alias")},supportingText={UiText("Start with a lowercase letter; use up to 32 lowercase letters, numbers, _ or -.")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("agent_alias_name"))
            }},
            confirmButton={TextButton(enabled=!model.busy && runCatching {AgentName(aliasName)}.isSuccess,onClick={model.renameAgent(scope,aliasName);aliasScope=null;aliasName=""},modifier=Modifier.heightIn(min=48.dp).testTag("agent_alias_save")){IconLabel(MaterialGlyph.Edit,"Save")}},
            dismissButton={TextButton(onClick={model.closeAgentAlias();aliasScope=null;aliasName=""},modifier=Modifier.heightIn(min=48.dp).testTag("agent_alias_cancel")){IconLabel(MaterialGlyph.Close,"Cancel")}},
            properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
            modifier=Modifier.testTag("agent_alias_dialog"),
        )
    }
    var renameWorkspace by remember(key,source) { mutableStateOf(false) }
    var workspaceLabel by remember(key,source) { mutableStateOf("") }
    val currentWorkspaceLabel=summary?.let { current -> inventory?.let { sessionWorkspaceLabel(it,current) } }
    val workspaceRenameAvailable=ready && source != null && currentWorkspaceLabel != null && summary?.capabilities?.workspaceRename==true
    // Loss of foreground, authority, readiness, source or target closes the ephemeral editor.
    // The durable receipt remains in the action list and is never retried from this dialog.
    LaunchedEffect(model.foreground,source,key,workspaceRenameAvailable) {
        if(!workspaceRenameAvailable) { renameWorkspace=false; workspaceLabel="" }
    }
    val validWorkspaceLabel=runCatching { Label(workspaceLabel) }.isSuccess
    if(renameWorkspace && workspaceRenameAvailable) AlertDialog(
        onDismissRequest={ renameWorkspace=false; workspaceLabel="" },
        title={ UiText("Rename workspace") },
        text={ Column(Modifier.imePadding(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            UiText("This changes the current native workspace label, not the conversation title.")
            OutlinedTextField(workspaceLabel,{workspaceLabel=it},label={UiText("Workspace label")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("workspace_rename_label"))
        } },
        confirmButton={ TextButton(enabled=validWorkspaceLabel,onClick={model.renameWorkspace(key,workspaceLabel);renameWorkspace=false;workspaceLabel=""},modifier=Modifier.testTag("workspace_rename_save")) { IconLabel(MaterialGlyph.Edit,"Save") } },
        dismissButton={ TextButton(onClick={renameWorkspace=false;workspaceLabel=""},modifier=Modifier.testTag("workspace_rename_cancel")) { IconLabel(MaterialGlyph.Close,"Cancel") } },
        modifier=Modifier.testTag("workspace_rename_dialog"),
    )

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val compact=maxHeight<480.dp || LocalDensity.current.fontScale>=1.5f
        val expiry=model.reservation?.takeIf {it.key==key}?.let {"Recovery copy until ${deadline(it.warning.futureExpiry?.expiresAt)} (≤7 days)."}
        ConversationFrame(chrome={
            /* The scaffold already owns the conversation title; keep connection and native status together. */
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                UiText(connectionLabel(key.host,connection,model.connectingHosts[key.host]==true),style=MaterialTheme.typography.labelSmall,modifier=Modifier.weight(1f).testTag("conversation_connection"))
                StatusPill(summary?.status?.let(::sessionStatusLabel) ?: "Unavailable",when(summary?.status) {SessionStatus.Attention->StatusEmphasis.Attention;SessionStatus.Working->StatusEmphasis.Positive;SessionStatus.Unknown,null->StatusEmphasis.Muted;SessionStatus.Idle->StatusEmphasis.Neutral})
            }
            RequestDock(pending) {selectedRequest=it}
            if(summary?.provider==Provider.Omp) UiText("All branches; desktop-selected branch unavailable",style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("omp_branches"))
        },history={
            Box(Modifier.fillMaxSize().testTag("conversation_target_${key.target.sessionId.value}_${key.target.generation.value}")) {
            LazyColumn(Modifier.fillMaxSize().testTag("conversation_list"),state=transcript,verticalArrangement=Arrangement.spacedBy(6.dp),contentPadding=PaddingValues(vertical=8.dp)) {
                item(key="conversation_header") {
                    GoatrSection(summary?.provider?.let(::providerLabel) ?: "Provider unavailable",inventory?.let {current->summary?.let {sessionWorkspaceLabel(current,it)}})
                    var details by remember(key,source) {mutableStateOf(false)}
                    TextButton(onClick={details=!details},modifier=Modifier.testTag("conversation_details")) {UiText(if(details) "Hide details" else "Details")}
                    if(details) SelectionContainer {UiText("Session ${key.target.sessionId.value}\nGeneration ${key.target.generation.value}\nWorkspace ${summary?.workspaceId?.value ?: "unavailable"}\nSource ${source?.value ?: "unavailable"}",style=MaterialTheme.typography.bodySmall)}
                    if(source==null) UiText("Offline/stale${if(cache?.partial!=false) " · partial cache" else ""}. Writes disabled; retained source is not authority.")
                    val changed=inventory?.sessions?.firstOrNull {it.sessionId==key.target.sessionId && it.generation!=key.target.generation}
                    if(changed!=null) {UiText("Observed generation replacement. Selection was not retargeted.");Action("Explicitly open observed replacement","open_replacement") {model.open(key.host,Target(changed.sessionId,changed.generation))}}
                    if(summary?.historyCoverage?.sourceTruncated==true) UiText("Native history source truncated")
                    if(reducer?.partial==true || cache?.partial==true) UiText("Partial history; not a complete transcript")
                    if(reducer?.browsingOlder==true) UiText("Viewing older history. Newer messages may be outside this window.")
                    if (reducer?.state?.summary?.capabilities?.history==true) UiText("Native backscroll keeps a local window of up to 1,000 completed records.")
                    reducer?.state?.activity?.let {activity->Heading("Live · ${activity.state}");activity.text?.let {ContentView(it,key,source,model)}}
                }
                // Keep the current canonical receipt near the header. Completed native history
                // remains the follow target instead of being covered by this status card.
                focusedAction?.let {action->item(key="receipt_${action.metadata.actionId.value}") {ReceiptPanel(key.host,action,model)}}
                items(history,key={"history_${it.id.value}"},contentType={it.kind}) {record->Column(Modifier.testTag("history_record_${record.id.value}")) {HistoryCard(record,key,source,model)}}
                items(model.actions[key.host].orEmpty().filter {it.metadata.target==key.target && it.metadata.actionId!=focusedAction?.metadata?.actionId},key={"receipt_${it.metadata.actionId.value}"}) {ReceiptPanel(key.host,it,model)}
                item {ActionPaging(model,collapsed=true)}
            }
            }
        },editor={
            ComposerOverflow(
                browsingOlder = reducer?.browsingOlder == true,
                refreshEnabled = !model.busy && key.host in connection.hello,
                onSwitch = { switcher = true },
                onRefresh = model::refresh,
                onBrowseOlder = if (reducer != null && !reducer.browsingOlder) ({ model.older() }) else null,
                browseOlderEnabled = !model.busy && source != null && reducer?.state?.summary?.capabilities?.history == true,
                onLoadOlder = reducer?.nextCursor?.let { cursor -> { model.older(cursor) } },
                loadOlderEnabled = !model.busy && source != null,
                onSnippets = { snippets = true },
                skillsEnabled = ready && source != null && summary?.capabilities?.skillsRead==true && summary.skillsGeneration!=null,
                onSkills = { model.openSkills(key) },
                terminalEnabled = ready && source != null && summary?.capabilities?.terminalRead==true,
                onTerminal = { model.openTerminal(key) },
                worktreesEnabled = ready && source != null && summary?.capabilities?.worktreesRead==true,
                onWorktrees = { model.openWorktrees(key) },
                changesEnabled = ready && source != null && summary?.capabilities?.changesRead==true,
                onChanges = { model.openChanges(key) },
                agentAliasEnabled = aliasAvailable && !model.busy,
                onAgentAlias = {source?.let {aliasScope=model.beginAgentAlias(key,it);aliasName=""}},
                workspaceRenameEnabled = workspaceRenameAvailable,
                onWorkspaceRename = { workspaceLabel=currentWorkspaceLabel.orEmpty(); renameWorkspace=true },
                stopEnabled = stopAllowed && request == null,
                stopDisabledReason = when {
                    request!=null -> "Close the current answer form before stopping."
                    readiness!=null -> readiness.message
                    else -> "Stopping is unavailable for this current native state."
                },
                onStop = model::stop,
            )
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                AttachmentPicker(enabled=ready && summary?.capabilities?.attachments==true && request==null,captureSourceTicket={model.startAttachmentPicker(key)},returnSourceTicket=model::returnAttachmentPicker,cancelSourceTicket=model::finishAttachmentPicker,isSourceTicketCurrent=model::attachmentAdmitted,onUpload=model::beginAttachmentUpload)
                if(model.attachmentTransfer!=null)TextButton(onClick=model::showAttachments,modifier=Modifier.heightIn(min=48.dp)){AttachmentIconLabel(R.drawable.ic_material_attach_file,"Upload progress")}
                store.attachments.filter {it.host==key.host&&it.target==key.target}.forEach {record->TextButton(onClick={model.showAttachmentRecovery(record)},modifier=Modifier.heightIn(min=48.dp)){AttachmentIconLabel(R.drawable.ic_material_description,"Retained upload")};TextButton(onClick={model.openRetainedAttachment(record)},enabled=ready&&summary?.capabilities?.attachments==true,modifier=Modifier.heightIn(min=48.dp).testTag("attachment_inspect_original")){AttachmentIconLabel(R.drawable.ic_material_attach_file,"Inspect for this conversation")}}
            }
            FlowRow(Modifier.fillMaxWidth().heightIn(min=48.dp),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)){
                if(summary?.capabilities?.artifacts==true)TextButton(onClick={model.beginArtifacts(key.host,ArtifactSource.Session(key.target))},enabled=ready&&source!=null&&!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Folder,"Files")}
                TextButton(onClick={model.beginAgentSession(key)},enabled=ready&&source!=null&&!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("agent_new_session")){IconLabel(MaterialGlyph.Add,"New session")}
                TextButton(onClick={model.beginAgentTerminal(key)},enabled=ready&&source!=null&&summary?.capabilities?.terminalCreate==true&&!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("agent_terminal_new")){IconLabel(MaterialGlyph.Terminal,"New terminal in agent process directory")}
            }
            if(summary?.capabilities?.terminalCreate!=true)UiText("Agent process directory creation requires native support, the desktop shell grant and the original bound provider process.",style=MaterialTheme.typography.bodySmall)
            val inlineReadiness=readiness?.takeIf {it.tag in listOf("conversation_gate_inventory","conversation_gate_receipt")}
            if(inlineReadiness!=null) {
                Row(Modifier.fillMaxWidth().heightIn(min=48.dp),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                    Box(Modifier.weight(1f)) {ComposerGateNotice(inlineReadiness.message,inlineReadiness.tag)}
                    IconButton(onClick=model::refresh,enabled=model.foreground && !model.busy && key.host in connection.hello,modifier=Modifier.size(48.dp).testTag("conversation_inventory_refresh")) {GoatrIcon(MaterialGlyph.Refresh,if(inlineReadiness.tag=="conversation_gate_receipt") "Refresh receipt recovery" else "Refresh desktop inventory")}
                }
            }
            // Auxiliary notices scroll independently; neither the draft nor Send moves into that viewport.
            Column(Modifier.fillMaxWidth().heightIn(max=if(compact) 64.dp else 112.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(2.dp)) {
                if(!following && reducer?.browsingOlder!=true) {
                    val position=history.indexOfFirst {it.id==readTail}
                    val unread=if(position>=0) history.size-position-1 else 0
                    TextButton(onClick={following=true;scope.launch {if(history.isNotEmpty() || focusedAction!=null) transcript.scrollToItem(latestTailIndex)}},modifier=Modifier.testTag("jump_latest")) {UiText(if(unread>0) "$unread new messages · Latest" else "Jump to latest")}
                }
                val promptCapability=summary?.capabilities
                val promptCapabilityUnavailable=promptCapability?.let { !it.prompt || it.promptMode!=PromptMode.SubmitOrSteer } ?: true
                val promptGate=readiness ?: when {
                    promptCapabilityUnavailable -> ReadinessReason("Prompting is unavailable for this current native state.","conversation_gate_native")
                    else -> null
                }
                if(promptGate?.tag !in listOf("conversation_gate_inventory","conversation_gate_receipt")) ComposerGateNotice(promptGate?.message ?: if(summary?.status==SessionStatus.Working) "Send may steer the current native work." else null,promptGate?.tag)
                if(model.reservation==null && model.composers[key].orEmpty().isNotEmpty()) TextButton(onClick=model::prepareNewPrompt,enabled=promptAllowed,modifier=Modifier.heightIn(min=48.dp).testTag("prompt_new_action")) {IconLabel(MaterialGlyph.Add,"Prepare a new send action")}
            }
        },actions={
            val draft=model.composers[key].orEmpty()
            val suggestions=if(request==null) model.suggestedSkills(key,draft) else null
            val suggestionHeight=if(suggestions!=null) 100.dp.coerceAtMost((maxHeight-112.dp).coerceAtLeast(0.dp)) else 0.dp
            if(suggestions!=null && suggestionHeight>=80.dp) SkillsSuggestions(suggestions,model,Modifier.height(suggestionHeight))
            ConversationDraftControls(draft,model::composer,expiry,compact,(maxHeight-56.dp-if(suggestions!=null && suggestionHeight>=80.dp) suggestionHeight else 0.dp).coerceAtLeast(0.dp),promptAllowed && model.reservation?.key==key && request==null,model::send,model.skillsFocusSignal,model::composerCursor)
        })
    }
}

private fun changeStateLabel(state: ChangeState): String = when(state) {
    ChangeState.Unchanged -> "unchanged"; ChangeState.Added -> "added"; ChangeState.Modified -> "modified"; ChangeState.Deleted -> "deleted";
    ChangeState.Renamed -> "renamed"; ChangeState.Copied -> "copied"; ChangeState.TypeChanged -> "type changed"; ChangeState.Unmerged -> "unmerged"
}

/** Local text only. This picker never reads a provider response, form, receipt, or secret. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ComposerSnippetPicker(key: ConversationKey,snippets: List<ComposerSnippet>,model: GoatrViewModel,draftEmpty: Boolean,onDismiss: ()->Unit) {
    var editing by remember(key) {mutableStateOf(false)}
    var editingId by remember(key) {mutableStateOf<Uuid?>(null)}
    var title by remember(key) {mutableStateOf("")}
    var body by remember(key) {mutableStateOf("")}
    var deleting by remember(key) {mutableStateOf<ComposerSnippet?>(null)}
    fun beginEdit(snippet: ComposerSnippet?) {
        editing=true
        // Allocate only an in-memory local identifier until the user explicitly saves text.
        editingId=snippet?.id ?: Uuid.random()
        title=snippet?.title.orEmpty()
        body=snippet?.body.orEmpty()
    }
    fun closeEditor() {
        editing=false
        editingId=null
        title=""
        body=""
    }
    val editorError = when {
        !validUnicode(title) || !validUnicode(body) -> "Use valid text before saving."
        title.codePointCount(0,title.length)>80 || title.toByteArray(Charsets.UTF_8).size>320 -> "Title is too long."
        body.isEmpty() -> "Add snippet text to save."
        body.toByteArray(Charsets.UTF_8).size>8192 -> "Snippet text is too long."
        else -> null
    }
    ModalBottomSheet(onDismissRequest=onDismiss,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true),containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.8f).semantics {testTagsAsResourceId=true}.imePadding().padding(16.dp).testTag("snippet_scope_${key.target.sessionId.value}_${key.target.generation.value}"),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                UiText("Local snippets",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f).testTag(if(draftEmpty) "snippet_draft_empty" else "snippet_draft_present"))
                TextButton(onClick=onDismiss,modifier=Modifier.heightIn(min=48.dp).testTag("snippet_close")) {IconLabel(MaterialGlyph.Close,"Close")}
            }
            if(editing) {
                editingId?.let {currentEditingId->
                Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).testTag("snippet_editor_${currentEditingId.value}"),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    UiText("Save stores only text you write on this phone. It remains local until deletion or Forget.",style=MaterialTheme.typography.bodySmall)
                    OutlinedTextField(value=title,onValueChange={title=it},label={UiText("Title")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("snippet_title"))
                    OutlinedTextField(value=body,onValueChange={body=it},label={UiText("Snippet text")},minLines=4,maxLines=10,modifier=Modifier.fillMaxWidth().testTag("snippet_body"))
                    editorError?.let {UiText(it,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error)}
                }
                Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick=::closeEditor,modifier=Modifier.weight(1f).heightIn(min=48.dp).testTag("snippet_editor_back")) {IconLabel(MaterialGlyph.Back,"Back")}
                    Button(onClick={
                        val savedTitle=title
                        val savedBody=body
                        model.saveSnippet(key.host,currentEditingId,savedTitle,savedBody) {
                            if(model.selection==key && editing && editingId==currentEditingId && title==savedTitle && body==savedBody) closeEditor()
                        }
                    },enabled=editorError==null,modifier=Modifier.weight(1f).heightIn(min=48.dp).testTag("snippet_save")) {IconLabel(MaterialGlyph.Save,"Save locally")}
                }
                }
            } else {
                Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                UiText("Save stores only text you write on this phone. It remains local until deletion or Forget.",style=MaterialTheme.typography.bodySmall)
                UiText("Provider Skills are unavailable.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                IconAction(MaterialGlyph.Add,"New local snippet","snippet_new",modifier=Modifier.fillMaxWidth()) {beginEdit(null)}
                if(snippets.isEmpty()) UiText("No local snippets saved for this desktop.",style=MaterialTheme.typography.bodyMedium)
                snippets.forEach {snippet->
                    GroupedSurface(Modifier.testTag("snippet_${snippet.id.value}")) {
                        UiText(snippet.title.ifEmpty {"Untitled snippet"},style=MaterialTheme.typography.titleSmall,maxLines=1,overflow=TextOverflow.Ellipsis)
                        UiText(snippet.body,style=MaterialTheme.typography.bodySmall,maxLines=2,overflow=TextOverflow.Ellipsis)
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(4.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                            TextButton(onClick={if(model.insertSnippet(key,snippet.id)) onDismiss()},modifier=Modifier.heightIn(min=48.dp).testTag("snippet_insert_${snippet.id.value}")) {IconLabel(MaterialGlyph.Send,"Insert")}
                            TextButton(onClick={beginEdit(snippet)},modifier=Modifier.heightIn(min=48.dp).testTag("snippet_edit_${snippet.id.value}")) {IconLabel(MaterialGlyph.Edit,"Edit")}
                            TextButton(onClick={deleting=snippet},modifier=Modifier.heightIn(min=48.dp).testTag("snippet_delete_${snippet.id.value}")) {IconLabel(MaterialGlyph.Delete,"Delete")}
                        }
                    }
                }
            }
            }
            }
        }
    deleting?.let {snippet->
        AlertDialog(onDismissRequest={deleting=null},modifier=Modifier.semantics {testTagsAsResourceId=true}.testTag("snippet_delete_scope_${snippet.id.value}"),title={UiText("Delete local snippet?")},text={UiText("This removes the saved local text from this phone.",modifier=Modifier.testTag("snippet_delete_target_${key.target.sessionId.value}_${key.target.generation.value}"))},confirmButton={TextButton(onClick={model.deleteSnippet(key.host,snippet.id);deleting=null},modifier=Modifier.testTag("snippet_delete_confirm")) {IconLabel(MaterialGlyph.Delete,"Delete")}},dismissButton={TextButton(onClick={deleting=null}) {UiText("Keep")}})
    }
}

@Composable
fun RequestDock(pending: List<PendingRequest>,onSelect: (Uuid)->Unit) {
    if(pending.isEmpty()) return
    Surface(color=MaterialTheme.colorScheme.primaryContainer,shape=MaterialTheme.shapes.medium,tonalElevation=2.dp,modifier=Modifier.fillMaxWidth().testTag("pending_cards")) {
        Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal=12.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            UiText("${pending.size} need your answer",modifier=Modifier.testTag("request_count").semantics {liveRegion=LiveRegionMode.Polite},style=MaterialTheme.typography.labelLarge)
            pending.forEachIndexed {index,request->
                TextButton(onClick={onSelect(request.requestKey)},modifier=Modifier.heightIn(min=48.dp).testTag("request_select_${request.requestKey.value}")) {UiText("Request ${index+1}")}
            }
        }
    }
}

/** Keep native notices, the transcript and the pinned composer in independent layout regions. */
@Composable
fun ConversationFrame(chrome: @Composable ()->Unit,history: @Composable ()->Unit,
    editor: @Composable ()->Unit,actions: @Composable ()->Unit) {
    BoxWithConstraints(Modifier.fillMaxSize().testTag("conversation_frame")) {
    val noticeHeight=if(maxHeight<420.dp) 64.dp else 96.dp
    Column(Modifier.fillMaxSize()) {
        Column(Modifier.weight(1f)) {
            // The control rail is measured before this weighted chrome/history region as well.
            Column(Modifier.weight(1f).fillMaxWidth()) {
                Column(Modifier.fillMaxWidth().heightIn(max=noticeHeight).verticalScroll(rememberScrollState()).testTag("conversation_notices")) {chrome()}
                Box(Modifier.weight(1f).fillMaxWidth()) {history()}
            }
            Surface(color=MaterialTheme.colorScheme.surface,tonalElevation=3.dp,modifier=Modifier.fillMaxWidth().testTag("composer_region")) {
                // Controls remain outside a clipped scroll viewport so they stay reachable above the IME.
                Column(Modifier.padding(top=2.dp),verticalArrangement=Arrangement.spacedBy(2.dp)) {editor()}
            }
        }
        // This nonweighted footer is measured before allocating the weighted remainder above it.
        actions()
    }
    }
}

@Composable
fun ComposerEditor(text: String,onText: (String)->Unit,expiry: String?,compact: Boolean,notice: String?,focusSignal:Long=0L,onCursor:((Int,Int)->Unit)?=null) {
    var editing by remember {mutableStateOf(TextFieldValue(text,TextRange(text.length)))}
    if(editing.text!=text)editing=TextFieldValue(text,TextRange(text.length))
    SideEffect {onCursor?.invoke(editing.selection.start,editing.selection.end)}
    val focus=remember {FocusRequester()}
    val keyboard=LocalSoftwareKeyboardController.current
    LaunchedEffect(focusSignal) {if(focusSignal>0L) {focus.requestFocus();keyboard?.show()}}
    notice?.let {UiText(it,style=MaterialTheme.typography.bodySmall)}
    OutlinedTextField(value=editing,onValueChange={value->val changed=value.text!=editing.text;editing=value;if(changed)onText(value.text);onCursor?.invoke(value.selection.start,value.selection.end)},label={UiText("Message")},
        minLines=1,maxLines=if(compact) 2 else 4,
        shape=MaterialTheme.shapes.large,modifier=Modifier.fillMaxWidth().heightIn(max=if(compact) 112.dp else 160.dp).focusRequester(focus).testTag("composer").semantics {contentDescription="Message to this exact session"})
    expiry?.let {UiText(it,style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("send_deadline"))}
}

@Composable
fun ConversationActions(sendEnabled: Boolean,stopEnabled: Boolean,onSend: ()->Unit,onStop: ()->Unit) {
    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(6.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
        IconAction(MaterialGlyph.Stop,"Stop","turn_stop",enabled=stopEnabled,onClick=onStop)
        Spacer(Modifier.weight(1f))
        PrimaryActionFab(MaterialGlyph.Send,"Send","prompt_send",sendEnabled,onSend)
    }
}

/** Fixed-size Send sits beside the independently scrolling draft and directly above the IME. */
@Composable
private fun ConversationPrimaryAction(sendEnabled: Boolean,onSend: ()->Unit) {
    ComposerSendFab(sendEnabled,onSend)
}

/** One viewport budget bounds the disclosure and draft; neither region uses weighted height. */
@Composable
private fun ConversationDraftControls(text: String,onText: (String)->Unit,expiry: String?,compact: Boolean,maximumHeight: Dp,sendEnabled: Boolean,onSend: ()->Unit,focusSignal:Long=0L,onCursor:((Int,Int)->Unit)?=null) {
    val availableHeight=(maximumHeight-6.dp).coerceAtLeast(0.dp)
    val minimumDraftHeight=56.dp.coerceAtMost(availableHeight)
    val disclosureHeight=if(expiry!=null) 48.dp.coerceAtMost((availableHeight-minimumDraftHeight).coerceAtLeast(0.dp)) else 0.dp
    val draftHeight=(availableHeight-disclosureHeight).coerceAtLeast(0.dp)
    Column(Modifier.fillMaxWidth().heightIn(max=maximumHeight).padding(bottom=6.dp)) {
        expiry?.let {
            Box(Modifier.fillMaxWidth().heightIn(max=disclosureHeight).verticalScroll(rememberScrollState())) {
                UiText(it,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant,modifier=Modifier.testTag("send_deadline"))
            }
        }
        Row(Modifier.fillMaxWidth().heightIn(min=minimumDraftHeight,max=draftHeight),horizontalArrangement=Arrangement.spacedBy(6.dp),verticalAlignment=androidx.compose.ui.Alignment.Bottom) {
            Box(Modifier.weight(1f)) {ComposerEditor(text,onText,null,compact,null,focusSignal,onCursor)}
            ConversationPrimaryAction(sendEnabled,onSend)
        }
    }
}

/** Standalone gallery/editor surface uses the same production editor and action row. */
@Composable
fun ConversationComposer(text: String,onText: (String)->Unit,sendEnabled: Boolean,stopEnabled: Boolean,
    expiry: String?,compact: Boolean,onSend: ()->Unit,onStop: ()->Unit,notice: String?) {
    val scroll=rememberScrollState()
    LaunchedEffect(expiry) {if(expiry!=null) {withFrameNanos {};scroll.scrollTo(scroll.maxValue)}}
    Column(Modifier.fillMaxWidth().heightIn(max=if(compact) 256.dp else 360.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
        Column(Modifier.weight(1f,fill=false).verticalScroll(scroll),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            ComposerEditor(text,onText,expiry,compact,notice)
        }
        ConversationActions(sendEnabled,stopEnabled,onSend,onStop)
    }
}

@Composable
fun ReceiptPanel(host: String,action: LocalAction,model: GoatrViewModel? = null,foreground: Boolean = model?.foreground==true,onDisplayed: (Receipt)->Unit = {receipt->model?.displayed(host,receipt)},onSeparateCreation: (() -> Unit)? = null) {
    val receipt=action.receipt
    val backing=remember(receipt) {receipt?.let {Protocol.strictJson.encodeToString(it)}}
    var visible by remember(receipt) {mutableStateOf(false)}
    var expanded by remember(action.metadata.actionId) {mutableStateOf(false)}
    val window=LocalResources.current.displayMetrics
    GroupedSurface(Modifier.testTag("receipt_${action.metadata.actionId.value}")) {
        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
            UiText(when(action.metadata.method) {ActionMethod.PromptSubmit->"Message status";ActionMethod.TurnCancel->"Stop status";ActionMethod.RequestRespond->"Answer status";ActionMethod.SessionCreate->"New conversation status";ActionMethod.TerminalCreate,ActionMethod.ShellCreate->"New terminal status";ActionMethod.ShellInput->"Terminal input status";ActionMethod.WorktreesRemove->"Worktree removal status";ActionMethod.AgentRename->"Agent alias status";ActionMethod.WorkspaceRename->"Workspace rename status";ActionMethod.AttachmentsBegin,ActionMethod.AttachmentsChunk,ActionMethod.AttachmentsComplete,ActionMethod.AttachmentsDiscard->"Attachment status"},style=MaterialTheme.typography.titleSmall,modifier=Modifier.weight(1f).padding(end=8.dp))
            StatusPill(when(receipt?.state) {ReceiptState.Succeeded,ReceiptState.Dispatched->"Known";ReceiptState.Failed,ReceiptState.Uncertain,null->"Inspect";ReceiptState.Reserved,ReceiptState.Dispatching->"Pending"},when(receipt?.state) {ReceiptState.Succeeded,ReceiptState.Dispatched->StatusEmphasis.Positive;ReceiptState.Reserved,ReceiptState.Dispatching->StatusEmphasis.Attention;else->StatusEmphasis.Muted})
        }
        if(receipt==null) UiText("Outcome unknown · local reservation is not Sent. Inspect only; never replay this UUID.")
        else {
            val label=when(receipt.state) {ReceiptState.Reserved->"Preparing";ReceiptState.Dispatching->"Sending";ReceiptState.Dispatched->"Sent to desktop";ReceiptState.Succeeded->"Action accepted";ReceiptState.Failed->"Action failed · inspect details";ReceiptState.Uncertain->"Outcome unknown · check status"}
            UiText(if(action.authoritative) label else "Last observed: $label",modifier=Modifier.testTag("receipt_state"))
            receipt.creation?.resources?.checkout?.let {checkout->
                UiText("Branch ${checkout.branch}",style=MaterialTheme.typography.bodySmall)
                checkout.resolvedCommit?.let {UiText("Commit ${it.take(12)}",style=MaterialTheme.typography.bodySmall)}
            }
            if(receipt.method==ActionMethod.SessionCreate && receipt.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching)) UiText("Creation ongoing (including after 120-second response). Refresh receipt; do not submit another Create.")
            if(receipt.state in listOf(ReceiptState.Failed,ReceiptState.Uncertain)) UiText("Known partial resources are retained in Details. Inspect on desktop; no automatic retry or cleanup.")
            if(receipt.result is ActionResultResponse) UiText("This receipt does not prove this phone won first-response resolution.")
            if(receipt.state==ReceiptState.Succeeded || receipt.state==ReceiptState.Dispatched) UiText("Operation admission/acknowledgement, not entire-turn completion.")
        }
        if(model!=null) Action("Inspect / refresh status (no replay)","receipt_inspect",enabled=!model.busy) { model.inspect(host,action.metadata.actionId) }
        if(onSeparateCreation!=null && action.metadata.method in listOf(ActionMethod.SessionCreate,ActionMethod.ShellCreate) && action.authoritative && action.outcomeShown && receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain)) {
            IconAction(MaterialGlyph.Add,if(action.metadata.method==ActionMethod.ShellCreate) "Start a separate terminal" else "Start a separate new session","separate_creation_prepare_${action.metadata.actionId.value}",enabled=model?.busy!=true,onClick=onSeparateCreation)
        }
        // Keep recovery controls reachable when the exact receipt is long.
        TextButton(onClick={expanded=!expanded;visible=false},modifier=Modifier.heightIn(min=48.dp).testTag("receipt_details_${action.metadata.actionId.value}")) {UiText(if(expanded) "Hide action details" else "Action details")}
        if(receipt!=null && expanded) key(action.metadata.actionId,receipt) {
            val detailsScroll=rememberScrollState()
            Box(Modifier.fillMaxWidth().heightIn(max=220.dp).verticalScroll(detailsScroll)) {
                SelectionContainer {UiText(backing!!,style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("receipt_exact").onGloballyPositioned {coordinates->val bounds=coordinates.boundsInWindow();visible=bounds.overlaps(Rect(0f,0f,window.widthPixels.toFloat(),window.heightPixels.toFloat()))})}
            }
        }
    }
    LaunchedEffect(receipt,expanded,visible,foreground) {
        if(receipt!=null && expanded && visible && foreground && action.authoritative && !action.outcomeShown) {withFrameNanos {};onDisplayed(receipt)}
    }
}

@Composable
private fun ActionPaging(model: GoatrViewModel,collapsed: Boolean=false) {
    if(!collapsed) {
        UiText("Bounded 100-action metadata page · UUID order. Current inspected action is shown separately.")
        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            Action("First action metadata page","actions_first",enabled=!model.busy) { model.actionPage(true) }
            Action("Load next action metadata page","actions_next",enabled=!model.busy && model.nextActionPage!=null) { model.actionPage() }
        }
        return
    }
    var expanded by remember {mutableStateOf(false)}
    TextButton(onClick={expanded=!expanded},modifier=Modifier.testTag("action_paging_details")) {UiText(if(expanded) "Hide activity details" else "Activity and action details")}
    if(expanded) {
        UiText("Up to 100 actions per page. The current inspected action is shown separately.")
        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            Action("First action metadata page","actions_first",enabled=!model.busy) { model.actionPage(true) }
            Action("Load next action metadata page","actions_next",enabled=!model.busy && model.nextActionPage!=null) { model.actionPage() }
        }
    }
}

@Composable
private fun SettingsScreen(host: String?,store: StoreView,connection: ConnectionView,model: GoatrViewModel,onRepairNotifications: () -> Unit) {
    if(host!=null) {
        Column(Modifier.fillMaxSize(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            TextButton(onClick={model.settings()},modifier=Modifier.heightIn(min=48.dp).testTag("settings_overview")) {IconLabel(MaterialGlyph.Back,"All settings")}
            HostSettingsScreen(host,store,connection,model,showAppearance=false,onRepairNotifications=onRepairNotifications,modifier=Modifier.weight(1f))
        }
        return
    }
    LazyColumn(Modifier.fillMaxSize().testTag("settings_overview_list"),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        item { GoatrSection("Settings","Appearance and session presentation are local to this phone. Desktop trust remains scoped to a paired desktop.") }
        item {
            GroupedSurface {
                GoatrSection("Appearance")
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {Appearance.entries.forEach {appearance->
                    FilterChip(selected=model.appearance==appearance,onClick={model.appearance=appearance},label={UiText(appearance.name)},modifier=Modifier.heightIn(min=48.dp).testTag("appearance_${appearance.name}"))
                }}
            }
        }
        item {
            val preferences=model.sessionListPreferences
            GroupedSurface {
                GoatrSection("Session list","These options change only how this phone presents existing session rows.")
                PresentationSwitchRow(
                    title="Group by status",
                    detail="Show status headers without changing session order or selection.",
                    checked=preferences.groupByStatus,
                    tag="settings_session_list_group_by_status",
                    onCheckedChange={model.sessionListPreferences=model.sessionListPreferences.copy(groupByStatus=it)},
                )
                PresentationSwitchRow(
                    title="Show workspace context",
                    detail="Show workspace context directly in each bound session row.",
                    checked=preferences.showWorkspaceContext,
                    tag="settings_session_list_workspace_context",
                    onCheckedChange={model.sessionListPreferences=model.sessionListPreferences.copy(showWorkspaceContext=it)},
                )
                PresentationSwitchRow(
                    title="Show desktop context",
                    detail="Show paired desktop context directly in each bound session row.",
                    checked=preferences.showHostContext,
                    tag="settings_session_list_host_context",
                    onCheckedChange={model.sessionListPreferences=model.sessionListPreferences.copy(showHostContext=it)},
                )
            }
        }
        item {
            GroupedSurface {
                GoatrSection("Paired desktops","Choose a desktop for its existing trust, recovery, and delivery settings.")
                if(store.hosts.isEmpty()) {
                    UiText("No paired desktop. Add one through the normal secure pairing flow.",style=MaterialTheme.typography.bodySmall)
                } else store.hosts.forEach {paired->
                    val online=paired.hostId in connection.hello
                    CompactRow(
                        title=paired.name,
                        subtitle="Paired desktop",
                        status={StatusPill(connectionLabel(paired.hostId,connection,model.connectingHosts[paired.hostId]==true),if(online) StatusEmphasis.Positive else StatusEmphasis.Attention)},
                        modifier=Modifier.testTag("settings_paired_desktop_${paired.hostId}"),
                        onClick={model.settings(paired.hostId)},
                    )
                }
                IconAction(MaterialGlyph.Scan,"Add desktop","settings_add_desktop",enabled=!model.busy,onClick=model::beginPairing)
            }
        }
    }
}

@Composable
private fun HostSettingsScreen(host: String,store: StoreView,connection: ConnectionView,model: GoatrViewModel,showAppearance: Boolean,onRepairNotifications: () -> Unit,modifier: Modifier = Modifier) {
    var confirmForget by remember {mutableStateOf(false)}
    var confirmUnpair by remember {mutableStateOf(false)}
    var discard by remember {mutableStateOf<RecoveryDraft?>(null)}
    LazyColumn(modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        item {
            GroupedSurface {
            GoatrSection(store.hosts.firstOrNull {it.hostId==host}?.name ?: "Host", "Trust, recovery, and delivery are local to this paired desktop.")
            if(showAppearance) FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {Appearance.entries.forEach {appearance->
                FilterChip(selected=model.appearance==appearance,onClick={model.appearance=appearance},label={UiText(appearance.name)},modifier=Modifier.heightIn(min=48.dp).testTag("appearance_${appearance.name}"))
            }}
            UiText("Trusted pairing authorizes ALL supported same-user host sessions, including outside configured creation projects.")
            UiText(connectionLabel(host,connection,model.connectingHosts[host]==true))
            HostRoutesEditor(host,store,connection,model)
            store.attachments.filter {it.host==host}.forEach {record->TextButton(onClick={model.showAttachmentRecovery(record)},modifier=Modifier.heightIn(min=48.dp)){AttachmentIconLabel(R.drawable.ic_material_attach_file,"Retained attachment · ${record.committedBytes} / ${record.size} bytes")}}
            GroupedSurface {
                GoatrSection("Background delivery","Choose persistent socket delivery or ntfy. A received probe and authenticated acknowledgement are required before ntfy can replace the verification socket.")
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)){
                    FilterChip(selected=model.pushMode(host)==DeliveryMode.Persistent,onClick={model.selectSocketDelivery(host)},label={IconLabel(MaterialGlyph.Notification,"Persistent socket")},modifier=Modifier.heightIn(min=48.dp))
                    FilterChip(selected=model.pushMode(host)==DeliveryMode.Ntfy,onClick={model.selectNtfy(host)},enabled=model.ntfyAvailable(),label={IconLabel(MaterialGlyph.Notification,"ntfy")},modifier=Modifier.heightIn(min=48.dp))
                }
                if(!model.ntfyAvailable())UiText("ntfy distributor is not installed. No distributor is chosen automatically.",style=MaterialTheme.typography.bodySmall)
                if(model.pushMode(host)==DeliveryMode.Ntfy){
                    UiText("ntfy · ${model.pushLabel(host)}",style=MaterialTheme.typography.bodyMedium)
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)){
                        TextButton(onClick={model.selectNtfy(host,true)},enabled=model.ntfyAvailable()){IconLabel(MaterialGlyph.Refresh,"Repair ntfy registration")}
                        TextButton(onClick={model.selectSocketDelivery(host)}){IconLabel(MaterialGlyph.Notification,"Use socket instead")}
                    }
                }
                PresentationSwitchRow(
                    title="Deliver attention from this desktop",
                    detail="Use durable host attention events only. No prompt, terminal, path, tool, or approval content is shown.",
                    checked=model.deliveryEnabled(host),
                    tag="delivery_enabled_$host",
                    onCheckedChange={model.setPersistentDelivery(host,it)},
                )
                PresentationSwitchRow(
                    title="Notify when work finishes",
                    detail="Completion events remain durable whether or not OS alerts are enabled.",
                    checked=model.deliveryCompleted(host),
                    tag="delivery_completed_$host",
                    onCheckedChange={model.setCompletionDelivery(host,it)},
                )
                StatusPill(model.deliveryLabel(host),when(model.deliveryLabel(host)) {"Running"->StatusEmphasis.Positive;"Reconnecting","Permission required"->StatusEmphasis.Attention;else->StatusEmphasis.Muted})
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                    IconAction(MaterialGlyph.Play,"Start","delivery_start_$host",enabled=!model.busy && model.deliveryEnabled(host),onClick=model::startPersistentDelivery)
                    IconAction(MaterialGlyph.Stop,"Stop","delivery_stop_$host",enabled=!model.busy && model.deliveryRequested(),onClick=model::stopPersistentDelivery)
                    IconAction(MaterialGlyph.Notification,"Repair notifications","delivery_repair_$host",enabled=!model.busy,onClick=onRepairNotifications)
                }
                UiText("ntfy hints contain no conversation text. Open a notification to authenticate and synchronize. Doze, notification permission, force-stop, desktop sleep and relay outages can delay delivery.",style=MaterialTheme.typography.bodySmall)
            }
            SelectionContainer {UiText("Pinned fingerprint $host",style=MaterialTheme.typography.bodySmall)}
            connection.hello[host]?.backends?.forEach {UiText("${it.provider} ${it.version}: ${it.availability}${it.reason?.let {r->" · $r"}.orEmpty()}")}
            if(host !in connection.hello) UiText("Disconnected: server authority and any unconfirmed self-unpair outcome are unknown. Explicit local Forget below does not confirm server revocation.")
            connection.connectionFailures[host]?.let {failure->
                var diagnostics by remember(host,failure) {mutableStateOf(false)}
                DetailDisclosure(diagnostics,{diagnostics=!diagnostics},"host_diagnostics")
                if(diagnostics) UiText("Closed diagnostic: ${connectionFailureCopy(failure)} Stopped while ${connectionFailureStage(failure.phase)}.",style=MaterialTheme.typography.bodySmall)
            }
            UiText("Arbitrary device administration is desktop-local: goatr devices / goatr revoke DEVICE_ID.")
            UiText("Local Forget deletes this host's local keys, cache, recovery, receipts and read state; it does not confirm server revocation. Backgrounding/Forget never kills desktop work.")
            TextButton(onClick={confirmForget=true},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("host_forget")) {UiText("Forget locally (available offline)")}
            OutlinedButton(onClick={confirmUnpair=true},enabled=!model.busy && host in connection.hello,modifier=Modifier.heightIn(min=48.dp).testTag("host_unpair")) {UiText("Self-unpair online")}
            model.focusedAction?.let {ReceiptPanel(host,it,model)}
            }
        }
        items(store.recovery.filter {it.hostId==host},key={it.action.actionId.value}) {draft->
            GroupedSurface {
                GoatrSection("Retained ordinary recovery",draft.action.actionId.value)
                UiText("Original Target ${draft.action.target} · deletion deadline ${deadline(draft.expiresAt)}")
                SelectionContainer {UiText(draft.text,modifier=Modifier.testTag("recovery_text"))}
                UiText("Inspection only. Explicitly compose a new action if needed; never automatically resend or retarget this copy.",style=MaterialTheme.typography.bodySmall)
                TextButton(onClick={discard=draft},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("recovery_discard")) {UiText("Discard this inspected copy")}
            }
        }
        items(model.actions[host].orEmpty().filter {it.metadata.actionId!=model.focusedAction?.metadata?.actionId},key={it.metadata.actionId.value}) {ReceiptPanel(host,it,model)}
        item {ActionPaging(model)}
    }
    if(confirmForget) AlertDialog(onDismissRequest={confirmForget=false},title={UiText("Forget this host locally?")},text={UiText("Delete actual local host keys and all private host state. Server authority remains until confirmed online self-unpair or desktop revoke. Desktop sessions keep running.")},confirmButton={Action("Confirm local Forget","forget_confirm") {confirmForget=false;model.forget(host)}},dismissButton={Action("Keep pairing","forget_cancel") {confirmForget=false}})
    if(confirmUnpair) AlertDialog(onDismissRequest={confirmUnpair=false},title={UiText("Self-unpair this phone?")},text={UiText("Requires authoritative online confirmation. If the response fails, server authority is unknown; local Forget remains explicit, never automatic re-pair.")},confirmButton={Action("Confirm self-unpair","unpair_confirm") {confirmUnpair=false;model.unpair(host)}},dismissButton={Action("Keep pairing","unpair_cancel") {confirmUnpair=false}})
    discard?.let {draft->AlertDialog(onDismissRequest={discard=null},title={UiText("Discard inspected recovery copy?")},text={UiText("Action ${draft.action.actionId.value}. This neither cancels desktop work nor sends any action.")},confirmButton={Action("Confirm discard","discard_confirm") {discard=null;model.discard(host,draft.action.actionId)}},dismissButton={Action("Keep copy","discard_cancel") {discard=null}})}
}
