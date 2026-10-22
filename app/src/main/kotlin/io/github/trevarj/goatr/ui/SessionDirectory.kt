// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.Role as SemanticsRole
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.dp

/** Local presentation only: none of these keys grant native session authority. */
enum class SessionDirectoryView { Workspaces, RecentAgents }
internal data class WorkspaceDirectoryKey(val host: String, val server: Uuid, val incarnation: Uuid, val workspace: Id) {
    // Lazy saveable state accepts a String, and length prefixes keep every tuple distinct.
    val lazyKey: String get()=listOf(host,server.value,incarnation.value,workspace.value).joinToString("") {"${it.length}:$it"}
    val tag: String get()="workspace_${host}_${server.value}_${incarnation.value}_${workspace.value}"
}
internal data class WorkspaceDirectoryGroup(
    val key: WorkspaceDirectoryKey,
    val serverName: String,
    val workspace: NativeWorkspace,
    val agents: List<SessionSummary>,
)
internal data class SessionDirectoryProjection(
    val workspaces: List<WorkspaceDirectoryGroup>,
    val standalone: List<SessionSummary>,
)

/** Join only the current native graph's explicit Target chain, never summary route hints. */
internal fun projectSessionDirectory(
    host: String,
    inventory: HostSnapshotResult?,
    ready: Boolean,
    sessions: List<SessionSummary>,
    narrowed: Boolean = false,
): SessionDirectoryProjection {
    if(inventory==null || !ready) return SessionDirectoryProjection(emptyList(),sessions)
    val groups=inventory.hierarchy.filter {graph->
        graph.freshness==NativeFreshness.Current && inventory.hierarchy.count {it.serverId==graph.serverId}==1 &&
            inventory.servers.count {it.serverId==graph.serverId}==1
    }.flatMap {graph->
        val server=inventory.servers.single {it.serverId==graph.serverId}
        graph.workspaces.filter {workspace->graph.workspaces.count {it.workspaceId==workspace.workspaceId}==1}.map {workspace->
            val agents=sessions.filter {summary->
                if(summary.serverId!=graph.serverId) return@filter false
                val target=Target(summary.sessionId,summary.generation)
                val pane=graph.panes.filter {it.target==target}.singleOrNull() ?: return@filter false
                val tab=graph.tabs.filter {it.tabId==pane.tabId}.singleOrNull() ?: return@filter false
                pane.workspaceId==workspace.workspaceId && tab.workspaceId==workspace.workspaceId &&
                    summary.workspaceId==workspace.workspaceId && summary.paneId==pane.paneId && summary.terminalId==pane.terminalId
            }
            WorkspaceDirectoryGroup(WorkspaceDirectoryKey(host,graph.serverId,graph.serverIncarnation,workspace.workspaceId),server.name.value,workspace,agents)
        }
    }
    val mapped=groups.flatMap {it.agents}.map {Target(it.sessionId,it.generation)}.toSet()
    return SessionDirectoryProjection(groups.filter {!narrowed || it.agents.isNotEmpty()},sessions.filter {Target(it.sessionId,it.generation) !in mapped})
}

internal fun rememberExplicitSessionOpen(previous: List<ConversationKey>, key: ConversationKey): List<ConversationKey> =
    (listOf(key)+previous.filter {it!=key}).take(64)

/** Stable remainder order has no implied native-activity timestamp. */
internal fun orderRecentlyOpened(host: String, sessions: List<SessionSummary>, opened: List<ConversationKey>): List<SessionSummary> {
    val positions=opened.withIndex().associate {it.value to it.index}
    return sessions.sortedBy {positions[ConversationKey(host,Target(it.sessionId,it.generation))] ?: Int.MAX_VALUE}
}

@Composable
internal fun SessionDirectoryMode(view: SessionDirectoryView, onChange: (SessionDirectoryView)->Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        SessionDirectoryView.entries.forEachIndexed {index,mode->
            SegmentedButton(selected=view==mode,onClick={onChange(mode)},shape=SegmentedButtonDefaults.itemShape(index,SessionDirectoryView.entries.size),label={UiText(if(mode==SessionDirectoryView.Workspaces) "Workspaces" else "Recent agents",maxLines=2,overflow=TextOverflow.Ellipsis)},modifier=Modifier.heightIn(min=48.dp).testTag("session_view_${mode.name}"))
        }
    }
    if(view==SessionDirectoryView.RecentAgents) UiText("Opened on this phone first · this app session",style=MaterialTheme.typography.bodySmall,fontStyle=FontStyle.Italic,color=MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
internal fun WorkspaceDirectoryCard(
    group: WorkspaceDirectoryGroup,
    hostName: String,
    preferences: SessionListPreferences,
    onOpen: (Target)->Unit,
) {
    var agentsExpanded by rememberSaveable(group.key.lazyKey) {mutableStateOf(false)}
    var metadataExpanded by rememberSaveable(group.key.lazyKey) {mutableStateOf(false)}
    val tag=group.key.tag
    val title=group.workspace.label?.value ?: "Workspace ${group.workspace.number.value} · label unavailable"
    OutlinedCard(Modifier.fillMaxWidth().testTag(tag)) {
        Row(Modifier.fillMaxWidth()) {
            Row(Modifier.weight(1f).testTag("${tag}_agents").combinedClickable(role=SemanticsRole.Button,onClickLabel=if(agentsExpanded) "Hide agents" else "Show agents",onClick={agentsExpanded=!agentsExpanded},onLongClickLabel="Workspace details",onLongClick={metadataExpanded=true}).semantics {stateDescription=if(agentsExpanded) "Agents expanded" else "Agents collapsed"}.heightIn(min=64.dp).padding(16.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(3.dp)) {
                    UiText(title,style=MaterialTheme.typography.titleMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
                    UiText("$hostName · ${group.serverName}",style=MaterialTheme.typography.bodySmall,maxLines=1,overflow=TextOverflow.Ellipsis)
                    UiText("${group.agents.size} ${if(group.agents.size==1) "agent" else "agents"}",style=MaterialTheme.typography.labelMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
                }
                GoatrIcon(MaterialGlyph.ExpandMore,null,Modifier.rotate(if(agentsExpanded) 180f else 0f))
            }
            IconButton(onClick={metadataExpanded=!metadataExpanded},modifier=Modifier.sizeIn(minWidth=48.dp,minHeight=48.dp).testTag("${tag}_details").semantics {stateDescription=if(metadataExpanded) "Workspace details expanded" else "Workspace details collapsed"}) {
                GoatrIcon(MaterialGlyph.Visibility,if(metadataExpanded) "Hide workspace details" else "Show workspace details")
            }
        }
        if(metadataExpanded) Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            UiText(title,style=MaterialTheme.typography.bodySmall)
            UiText("Desktop: $hostName\nServer: ${group.serverName}\nWorkspace number: ${group.workspace.number.value}\nNative status: ${group.workspace.agentStatus}\nDesktop focused: ${group.workspace.focused}",style=MaterialTheme.typography.bodySmall)
            UiText("Server ID: ${group.key.server.value}\nIncarnation: ${group.key.incarnation.value}\nWorkspace ID: ${group.key.workspace.value}",style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace)
        }
        if(agentsExpanded) Column(Modifier.padding(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            if(group.agents.isEmpty()) UiText("No supported bound agents in this workspace.",style=MaterialTheme.typography.bodySmall)
            group.agents.forEach {summary->
                key(summary.sessionId,summary.generation) {
                    DirectoryAgentRow(summary,hostName,title,preferences,false,"session_${group.key.host}_${summary.sessionId.value}_${summary.generation.value}",onOpen)
                }
            }
        }
    }
}

@Composable
internal fun DirectoryAgentRow(
    summary: SessionSummary,
    hostName: String?,
    workspace: String?,
    preferences: SessionListPreferences,
    stale: Boolean,
    rowTag: String,
    onOpen: (Target)->Unit,
    cachePartial: Boolean? = null,
) {
    val target=Target(summary.sessionId,summary.generation)
    var details by remember(summary.sessionId,summary.generation) {mutableStateOf(false)}
    val title=summary.title.takeIf {it.isNotBlank()} ?: "${providerLabel(summary.provider)} conversation"
    val subtitle=listOfNotNull(hostName.takeIf {preferences.showHostContext},workspace.takeIf {preferences.showWorkspaceContext},if(cachePartial!=null) "retained cache${if(cachePartial) " · partial" else ""}" else summary.connectivity.toString(),if(stale) "stale" else null,summary.reason).joinToString(" · ")
    val lastObserved=stale || cachePartial!=null || summary.connectivity!=SessionConnectivity.Ready
    val providerStatus=sessionStatusLabel(summary.status)
    val statusText=when {
        stale->"Stale · $providerStatus"
        cachePartial!=null->"Cached · $providerStatus"
        summary.connectivity!=SessionConnectivity.Ready->"${summary.connectivity} · $providerStatus"
        else->providerStatus
    }
    Surface(shape=MaterialTheme.shapes.medium,color=MaterialTheme.colorScheme.surface,border=BorderStroke(1.dp,MaterialTheme.colorScheme.outline.copy(alpha=0.25f)),modifier=Modifier.fillMaxWidth().testTag(rowTag).combinedClickable(onClickLabel="Open conversation",onClick={onOpen(target)},onLongClickLabel="Conversation details and actions",onLongClick={details=true})) {
        BoxWithConstraints(Modifier.fillMaxWidth().padding(16.dp)) {
            val stacked=maxWidth<320.dp || LocalDensity.current.fontScale>=1.3f
            Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                if(stacked) Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                    ProviderPill(summary.provider,Modifier.testTag("${rowTag}_provider"))
                    UiText(title,modifier=Modifier.weight(1f).testTag("${rowTag}_title"),style=MaterialTheme.typography.titleMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
                } else UiText(title,modifier=Modifier.testTag("${rowTag}_title"),style=MaterialTheme.typography.titleMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
                UiText(subtitle,modifier=Modifier.testTag("${rowTag}_subtitle"),style=MaterialTheme.typography.bodySmall,maxLines=1,overflow=TextOverflow.Ellipsis)
                Row(Modifier.fillMaxWidth(),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                    if(!stacked) ProviderPill(summary.provider,Modifier.testTag("${rowTag}_provider"))
                    Box(Modifier.weight(1f).testTag("${rowTag}_status")) {
                        StatusPill(statusText,if(lastObserved) StatusEmphasis.Muted else sessionStatusEmphasis(summary.status))
                    }
                    Box(Modifier.size(16.dp)) {
                        if(!stale && cachePartial==null && summary.connectivity==SessionConnectivity.Synchronizing) CircularProgressIndicator(
                            modifier=Modifier.fillMaxSize(),
                            color=MaterialTheme.colorScheme.onSurfaceVariant,
                            strokeWidth=1.5.dp,
                        )
                    }
                    IconButton(onClick={details=true},modifier=Modifier.sizeIn(minWidth=48.dp,minHeight=48.dp).testTag("${rowTag}_details")) {GoatrIcon(MaterialGlyph.ExpandMore,"Conversation details and actions")}
                }
            }
        }
    }
    if(details) AlertDialog(
        onDismissRequest={details=false},
        title={UiText(title)},
        text={Column(Modifier.verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(6.dp)) {
            ProviderPill(summary.provider)
            UiText(subtitle)
            UiText("${if(lastObserved) "Last observed" else "Current"} status: $providerStatus")
            hostName?.let {UiText("Desktop: $it")}
            workspace?.let {UiText("Workspace: $it")}
            UiText("Session ID: ${target.sessionId.value}\nGeneration: ${target.generation.value}",fontFamily=FontFamily.Monospace,style=MaterialTheme.typography.bodySmall)
            summary.reason?.let {UiText(it)}
        }},
        confirmButton={TextButton(onClick={details=false;onOpen(target)},modifier=Modifier.testTag("${rowTag}_open")) {UiText("Open conversation")}},
        dismissButton={TextButton(onClick={details=false}) {UiText("Close")}},
        modifier=Modifier.testTag("${rowTag}_dialog"),
    )
}
