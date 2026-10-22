// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.compose.ui.text.style.TextOverflow

private fun shellWorkspace(inventory:HostSnapshotResult?,shell:ShellSummary):String =
    inventory?.hierarchy?.singleOrNull {it.serverId==shell.serverId}?.workspaces?.singleOrNull {it.workspaceId==shell.workspaceId}?.label?.value ?: "Workspace ${shell.workspaceId.value}"

/** Positive shell rows are a separate resource kind, never fake provider sessions. */
@Composable
internal fun ShellTerminalsScreen(host:String,store:StoreView,view:ConnectionView,model:GoatrViewModel) {
    val inventory=view.inventory[host] ?: store.inventory.firstOrNull {it.hostId==host}?.inventory
    val ready=view.inventoryReady[host]==true && host in view.hello
    val hostName=store.hosts.firstOrNull {it.hostId==host}?.name ?: "Desktop"
    val terminalAppearance=rememberTerminalAppearanceState()
    LazyColumn(Modifier.fillMaxSize().testTag("shell_terminals"),verticalArrangement=Arrangement.spacedBy(16.dp),contentPadding=PaddingValues(bottom=24.dp)) {
        item {
            GroupedSurface {
                UiText(hostName,style=MaterialTheme.typography.titleMedium)
                UiText(if(ready) "Ordinary terminals" else "Last observed terminal list",style=MaterialTheme.typography.bodySmall)
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    IconButton(onClick=model::refreshTerminals,enabled=!model.busy && model.foreground && host in view.hello,modifier=Modifier.size(48.dp).testTag("shell_inventory_refresh")) {GoatrIcon(MaterialGlyph.Refresh,"Refresh terminals")}
                    IconButton(onClick={model.requestClearShellDrafts(host)},enabled=model.foreground && model.shellInputDrafts.keys.any {it.host==host},modifier=Modifier.size(48.dp).testTag("shell_clear_local_drafts")) {GoatrIcon(MaterialGlyph.Edit,"Clear local drafts")}
                    IconButton(onClick={model.sessions(host)},enabled=!model.busy,modifier=Modifier.size(48.dp).testTag("shell_back_sessions")) {GoatrIcon(MaterialGlyph.Chat,"Sessions")}
                }
                TerminalAppearancePicker(terminalAppearance)
            }
        }
        val rows=inventory?.shells.orEmpty()
        if(rows.isEmpty()) item {
            GroupedSurface {
                UiText(when {inventory==null->"Terminal list unavailable";!ready->"No rows in the last observed terminal list";else->"No terminal rows in this list"},style=MaterialTheme.typography.titleSmall)
                UiText(when {host in view.hello && inventory==null->"Connected. A terminal list has not been received yet.";model.connectingHosts[host]==true->"Connecting to this desktop.";!ready->"Reconnect or refresh to request its current terminal list.";else->"Refresh to request another snapshot. This list does not establish desktop shell grants."},style=MaterialTheme.typography.bodySmall)
            }
        }
        rows.groupBy {it.serverId to it.workspaceId}.forEach {(_,shells)->
            item {UiText(shellWorkspace(inventory,shells.first()),style=MaterialTheme.typography.titleSmall)}
            items(shells,key={"${it.target.shellId.value}_${it.target.generation.value}"}) {shell->
                val actionable=ready && model.foreground && shell.connectivity==SessionConnectivity.Ready && shell.capabilities.read
                GroupedSurface(Modifier.testTag("shell_row_${shell.target.shellId.value}")) {
                    UiText(shell.title.value,style=MaterialTheme.typography.titleSmall,maxLines=1,overflow=TextOverflow.Ellipsis)
                    var details by remember(host,shell.target) {mutableStateOf(false)}
                    if(!actionable) UiText(when { !ready->"Last observed terminal · current inventory is unavailable";shell.connectivity==SessionConnectivity.Synchronizing->"Synchronizing terminal";shell.connectivity!=SessionConnectivity.Ready->"${shell.connectivity} · terminal access unavailable";!shell.capabilities.read->"Terminal read capability is unavailable";else->"Terminal access is paused while this app is inactive"},style=MaterialTheme.typography.bodySmall)
                    DetailDisclosure(details,{details=!details},"shell_details_${shell.target.shellId.value}")
                    if(details) {
                        UiText(shell.title.value,style=MaterialTheme.typography.bodySmall)
                        UiText("Tab ${shell.tabId.value} · pane ${shell.paneId.value}",style=MaterialTheme.typography.bodySmall)
                    }
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                        if(shell.capabilities.artifacts)IconAction(MaterialGlyph.Folder,"Files","shell_files_${shell.target.shellId.value}",enabled=ready&&model.foreground&&shell.connectivity==SessionConnectivity.Ready&&!model.busy){model.beginArtifacts(host,ArtifactSource.Shell(shell.target))}
                        IconAction(MaterialGlyph.Terminal,"Open terminal","shell_open_${shell.target.shellId.value}",enabled=actionable && !model.busy) {model.openShell(host,shell.target)}
                    }
                }
            }
        }
        items(model.actions[host].orEmpty().filter {it.metadata.method==ActionMethod.ShellCreate},key={"shell_action_${it.metadata.actionId.value}"}) {action->
            ReceiptPanel(host,action,model)
            ShellCreatedAction(host,action,inventory,ready,host in view.hello,model)
        }
    }
    model.shell?.takeIf {it.selection.host==host}?.let {snapshot->ShellViewer(snapshot,hostName,inventory,host in view.hello,model,terminalAppearance.value,connectionLabel(snapshot.selection.host,view,model.connectingHosts[snapshot.selection.host]==true),compactConnectionLabel(snapshot.selection.host,view,model.connectingHosts[snapshot.selection.host]==true))}
    if(model.clearShellDraftHost==host) AlertDialog(
        onDismissRequest=model::cancelClearShellDrafts,title={UiText("Clear local terminal drafts?")},
        text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)){UiText(hostName);UiText("Remove all local terminal text, cursor positions, key characters and modes for this desktop, including offline and older terminal generations. Pending inputs and receipts are retained; no terminal operation is sent.")}},
        confirmButton={TextButton(onClick=model::confirmClearShellDrafts,modifier=Modifier.heightIn(min=48.dp).testTag("shell_clear_local_drafts_confirm")){UiText("Clear this desktop’s drafts")}},
        dismissButton={TextButton(onClick=model::cancelClearShellDrafts,modifier=Modifier.heightIn(min=48.dp)){UiText("Keep drafts")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
    )
}

@Composable
private fun ShellCreatedAction(host:String,action:LocalAction,inventory:HostSnapshotResult?,ready:Boolean,connected:Boolean,model:GoatrViewModel) {
    val receipt=action.receipt ?: return
    val resources=(receipt.result as? ActionResultShellCreation)?.resources ?: return
    if(receipt.state!=ReceiptState.Succeeded) return
    val current=resources.createdTarget?.let {target->inventory?.shells?.singleOrNull {shell->shell.target==target && shell.serverId==resources.serverId && shell.workspaceId==resources.workspaceId && shell.tabId==resources.tabId && shell.paneId==resources.paneId && shell.terminalId==resources.terminalId && shell.connectivity==SessionConnectivity.Ready && shell.capabilities.read}}
    if(current!=null && ready) IconAction(MaterialGlyph.Terminal,"Open created terminal","shell_open_created",enabled=model.foreground && !model.busy) {model.openShell(host,current.target)}
    else if(connected) IconAction(MaterialGlyph.Refresh,"Refresh terminals to find it","shell_refresh_created",enabled=model.foreground && !model.busy,onClick=model::refreshTerminals)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShellViewer(snapshot:ShellSnapshot,hostName:String,inventory:HostSnapshotResult?,connected:Boolean,model:GoatrViewModel,terminalAppearance:TerminalAppearance,connectionStatus:String,compactConnectionStatus:String) {
    val keyboard=LocalSoftwareKeyboardController.current
    LaunchedEffect(snapshot.selection,snapshot.ticket) {keyboard?.hide()}
    val scope=ShellCreateScope(snapshot.selection,snapshot.source,snapshot.ticket)
    var confirm by remember(scope) {mutableStateOf(false)}
    val gate=model.shellCreationGate?.takeIf {it.scope==scope}
    Dialog(onDismissRequest=model::closeShell,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
        Surface(Modifier.fillMaxSize().safeDrawingPadding()) {
            Scaffold(
                contentWindowInsets=WindowInsets(0,0,0,0),
                topBar={GoatrTitleBar(snapshot.summary.title.value,connectionStatus,snapshot.loading,compactStatus=compactConnectionStatus,navigationIcon={
                    IconButton(onClick=model::closeShell,modifier=Modifier.size(48.dp).testTag("shell_viewer_back")){GoatrIcon(MaterialGlyph.Back,"Back to terminals")}
                },actions={
                    IconButton(onClick=model::refreshShell,enabled=!model.busy && !snapshot.loading,modifier=Modifier.size(48.dp).testTag("shell_viewer_refresh")){GoatrIcon(MaterialGlyph.Refresh,"Refresh terminal")}
                })},
                bottomBar={if(snapshot.summary.capabilities.input) ShellInputPanel(snapshot,model)},
                floatingActionButton={if(snapshot.summary.capabilities.create) PrimaryActionFab(MaterialGlyph.Add,"New terminal here","shell_create_here",enabled=model.shellCreationAllowed(scope),onClick={if(model.shellCreationAllowed(scope))confirm=true})},
            ) {padding->
                LazyColumn(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding).padding(horizontal=16.dp),verticalArrangement=Arrangement.spacedBy(8.dp),contentPadding=PaddingValues(bottom=112.dp)) {
                    item {Column {UiText(hostName,style=MaterialTheme.typography.bodySmall);UiText(shellWorkspace(inventory,snapshot.summary),style=MaterialTheme.typography.bodySmall)}}
                    item {
                        UiText("Text snapshot · refresh to update",style=MaterialTheme.typography.labelMedium)
                        Box(Modifier.fillMaxWidth().heightIn(min=160.dp)) {
                            when {
                                snapshot.loading->UiText("Reading this terminal snapshot…",style=MaterialTheme.typography.bodySmall)
                                snapshot.error->UiText("Terminal could not be read. Refresh when this original terminal is available.",color=MaterialTheme.colorScheme.error)
                                snapshot.text.isNullOrEmpty()->UiText("No snapshot text returned.",style=MaterialTheme.typography.bodySmall)
                                else->TerminalSnapshotText(snapshot.text.orEmpty(),terminalAppearance)
                            }
                        }
                        if(snapshot.truncated) UiText("Showing the bounded visible terminal snapshot.",style=MaterialTheme.typography.bodySmall)
                    }
                    gate?.prior?.let {prior->item {
                        ReceiptPanel(snapshot.selection.host,prior,model,onSeparateCreation={model.prepareSeparateShell(scope,prior.metadata.actionId)})
                        ShellCreatedAction(snapshot.selection.host,prior,inventory,true,connected,model)
                    }}
                    if(snapshot.summary.capabilities.create && !model.shellCreationAllowed(scope)) item {
                        UiText(if(gate?.pending==true || gate?.loaded!=true) "Creation readiness or receipt reconciliation pending." else "Inspect the prior receipt before explicitly starting a separate terminal.",style=MaterialTheme.typography.bodySmall)
                    }
                }
            }
        }
    }
    if(confirm) AlertDialog(
        onDismissRequest={confirm=false},title={UiText("New terminal here?")},
        text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
            UiText(hostName);UiText(shellWorkspace(inventory,snapshot.summary));UiText(snapshot.summary.title.value)
            UiText("Create one unfocused terminal in this original terminal’s native current directory. No command or directory override is sent.")
        }},
        confirmButton={TextButton(onClick={confirm=false;model.createShell(scope)},enabled=!model.busy && model.shellCreationAllowed(scope),modifier=Modifier.heightIn(min=48.dp).testTag("shell_create_confirm")){IconLabel(MaterialGlyph.Add,"Create terminal")}},
        dismissButton={TextButton(onClick={confirm=false},modifier=Modifier.heightIn(min=48.dp)){UiText("Cancel")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
    )
    model.separateShellCreation?.takeIf {it.first==scope}?.let {(_,action)->AlertDialog(
        onDismissRequest=model::cancelSeparateShell,title={UiText("Start a separate terminal?")},
        text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
            UiText(hostName);UiText(snapshot.summary.title.value);UiText("Prior action ${action.value}")
            UiText("The prior terminal may already exist. Keep its resources untouched. This permits a fresh, separate action only; it never retries the prior action.")
        }},
        confirmButton={TextButton(onClick=model::confirmSeparateShell,enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("shell_separate_confirm")){UiText("Allow separate creation")}},
        dismissButton={TextButton(onClick=model::cancelSeparateShell,modifier=Modifier.heightIn(min=48.dp)){UiText("Keep prior action only")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
    )}
}

/** Explicit gestures only. Neither key controls nor mode changes submit the text draft. */
@Composable
private fun ShellInputPanel(snapshot:ShellSnapshot,model:GoatrViewModel) {
 val scope=ShellCreateScope(snapshot.selection,snapshot.source,snapshot.ticket)
 val draft=model.shellDraft(snapshot.selection)
 val gate=model.shellInputGate?.takeIf {it.scope==scope}
 val enabled=!model.busy && model.shellInputAllowed(scope)
 Surface(tonalElevation=3.dp) {
  Column(Modifier.fillMaxWidth().imePadding().padding(horizontal=16.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
   gate?.prior?.let {action->
    val receipt=action.receipt
    val label=when {
     gate?.observation==null->"Current status needs checking · last receipt retained"
     gate?.observation?.unknown==true->"Current host status: unknown · last receipt retained"
     action.authoritative && receipt?.state==ReceiptState.Succeeded->"Input accepted"
     action.authoritative && receipt?.state==ReceiptState.Failed->"Input refused before effect"
     receipt?.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching)->"Input pending"
     else->"Input outcome unknown · do not repeat"
    }
    UiText(label,style=MaterialTheme.typography.labelMedium,modifier=Modifier.testTag("shell_input_outcome"))
    LaunchedEffect(scope,action,gate?.observation) {withFrameNanos {};model.shellInputDisplayed(scope,action)}
    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
     TextButton(onClick={model.inspect(snapshot.selection.host,action.metadata.actionId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check receipt")}
     if(gate?.observation?.let {it.unknown || receipt?.state==ReceiptState.Uncertain}==true) TextButton(onClick={model.requestSeparateShellInput(scope,action.metadata.actionId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Separate action…")}
    }
    Column(Modifier.fillMaxWidth().heightIn(max=112.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(4.dp)) {
     if((gate?.observation==null || gate?.observation?.unknown==true) && receipt!=null) UiText("Last known receipt: "+receipt.state.name,style=MaterialTheme.typography.labelSmall)
    (receipt?.result as? ActionResultShellInput)?.progress?.let {progress->UiText("Known bytes: "+(progress.writtenBytes?.toString()?:"unknown")+" / "+(progress.encodedBytes?.toString()?:"unknown"),style=MaterialTheme.typography.labelSmall)}
    }
   }
   FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
    FilterChip(selected=!draft.keys,onClick={model.shellInputMode(scope,false)},label={UiText("Text")},leadingIcon={GoatrIcon(MaterialGlyph.Edit,null)})
    FilterChip(selected=draft.keys,onClick={model.shellInputMode(scope,true)},label={UiText("Keys")},leadingIcon={GoatrIcon(MaterialGlyph.Terminal,null)})
    TextButton(onClick={model.clearShellDraft(scope)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Clear draft")}
   }
   UiText(if(draft.keys)"Tap modifiers once; double-tap to lock" else "Exact text · no added Enter",style=MaterialTheme.typography.labelSmall)
   if(!draft.keys) Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.Bottom,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
    OutlinedTextField(value=draft.value,onValueChange={model.editShellDraft(scope,it)},modifier=Modifier.weight(1f).heightIn(min=56.dp,max=144.dp).testTag("shell_input_text"),placeholder={UiText("Terminal input")},maxLines=5)
    IconButton(onClick={model.sendShellText(scope)},enabled=enabled && draft.value.text.isNotEmpty(),modifier=Modifier.size(48.dp).testTag("shell_input_send")){GoatrIcon(MaterialGlyph.Send,"Send exact terminal text")}
   } else {
    TerminalKeyBar(draft.modifiers,enabled,{modifier,lock->model.shellInputModifier(scope,modifier,lock)},{modifier,at,window->model.shellInputModifierTap(scope,modifier,at,window)},{key->model.sendShellKey(scope,key)})
    Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
     OutlinedTextField(value=draft.keyCharacter,onValueChange={model.shellKeyCharacter(scope,it)},label={UiText("One character + modifiers")},singleLine=true,modifier=Modifier.weight(1f))
     IconButton(onClick={model.sendShellCharacter(scope)},enabled=enabled && draft.keyCharacter.isNotEmpty(),modifier=Modifier.size(48.dp)){GoatrIcon(MaterialGlyph.Send,"Send character key")}
    }
   }
   if(gate?.loaded!=true)UiText("Checking original terminal and retained receipts…",style=MaterialTheme.typography.labelSmall)
  }
 }
 model.separateShellInput?.takeIf {it.first==scope}?.let {(_,action)->AlertDialog(
  onDismissRequest=model::cancelSeparateShellInput,title={UiText("Allow a separate input?")},
  text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)){UiText(snapshot.summary.title.value);UiText("Prior action "+action.value);UiText("The prior input may already have taken effect. This permits a fresh gesture only. It never repeats that input.")}},
  confirmButton={TextButton(onClick=model::confirmSeparateShellInput,enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Allow separate action")}},
  dismissButton={TextButton(onClick=model::cancelSeparateShellInput,modifier=Modifier.heightIn(min=48.dp)){UiText("Keep prior action")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
 )}
}
