// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

/** One original foreground configured-workspace claim; invalidation never replays an action. */
internal class TerminalCreationLease {private val gate=Any();private var active=true
    fun invalidate(){synchronized(gate){active=false}}
    fun <T> admit(block:()->T):T=synchronized(gate){if(!active)throw StorePolicy(ErrorCode.NotReady);block()}
}
class TerminalCreationMutationContext internal constructor(internal val original:Any,internal val lease:TerminalCreationLease)
internal class TerminalCreationState(val host:String,val server:Uuid,val workspace:Id,val source:ArtifactSource?,
    val foreground:Any,val originScreen:Screen,val context:TerminalCreationMutationContext) {
    var project by mutableStateOf<Uuid?>(null)
    var loaded by mutableStateOf(false);var pending by mutableStateOf(true)
    var prior by mutableStateOf<LocalAction?>(null);var separate by mutableStateOf<Uuid?>(null)
    var loading:Any?=null
    var directories:CreationDirectoryBrowser?=null
}
@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun TerminalCreationDialog(shown:TerminalCreationState,view:ConnectionView,model:GoatrViewModel,hostName:String) {
    val inventory=view.inventory[shown.host]
    val graph=inventory?.hierarchy?.singleOrNull {it.serverId==shown.server}
    val workspace=graph?.workspaces?.singleOrNull {it.workspaceId==shown.workspace}
    AlertDialog(onDismissRequest=model::closeTerminalCreation,
        title={GoatrTitleBar("New terminal",connectionLabel(shown.host,view,model.connectingHosts[shown.host]==true),!shown.loaded,
            compactStatus=compactConnectionLabel(shown.host,view,model.connectingHosts[shown.host]==true),titleModifier=Modifier.testTag("terminal_creation_scope_${shown.host}_${shown.server.value}_${shown.workspace.value}"))},
        text={Column(Modifier.fillMaxWidth().heightIn(max=460.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            UiText(hostName,style=MaterialTheme.typography.titleMedium)
            UiText(workspace?.label?.value ?: "Original workspace",style=MaterialTheme.typography.bodyMedium,modifier=Modifier.testTag(if(shown.source==null)"terminal_creation_source_workspace"else"terminal_creation_source_bound"))
            UiText("Create one unfocused native tab in this original workspace. Choose an approved project/directory; no cwd is inferred from a conversation or terminal.")
            UiText("Requires the desktop’s explicit terminal-control grant. Goatr verifies the actual new shell directory before enabling input. A directory replacement or lost outcome retains uncertain resources without cd, retry or cleanup.")
            val projects=inventory?.projects.orEmpty().filter {it.available}
            if(projects.isEmpty())UiText("Configure an approved creation project on this desktop first.")
            else {
                GoatrSection("Configured project")
                FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    for(project in projects)FilterChip(selected=shown.project==project.projectId,
                        onClick={model.selectTerminalProject(shown,project.projectId)},enabled=!shown.pending,
                        label={UiText(project.label.value)},modifier=Modifier.heightIn(min=48.dp).testTag("terminal_project_${project.projectId.value}"))
                }
                shown.directories?.takeIf {shown.project!=null}?.let {browser->
                    CreationDirectoriesChooser(browser,projects.singleOrNull {it.projectId==shown.project}?.label?.value ?: "Selected project")
                }
            }
            shown.prior?.let {prior->
                ReceiptPanel(shown.host,prior,model)
                if(prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))
                    TextButton(onClick={model.permitSeparateTerminal(shown,prior.metadata.actionId)},enabled=!shown.pending,modifier=Modifier.heightIn(min=48.dp)){
                        UiText("Allow a fresh, separate terminal")
                    }
            }
            if(shown.pending)UiText("Inspect the retained creation first. It is never retried, adopted or cleaned up.")
            TextButton(onClick={model.inspectTerminalCreation(shown)},modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check retained receipt")}
            if(model.terminalCreatedAvailable(shown))TextButton(onClick={model.openCreatedTerminal(shown)},modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Terminal,"Open created terminal")}
        }},
        confirmButton={TextButton(onClick={model.createTerminal(shown)},enabled=!model.busy&&model.terminalCreationAllowed(shown),modifier=Modifier.heightIn(min=48.dp).testTag("terminal_create_confirm")){IconLabel(MaterialGlyph.Add,"Create terminal")}},
        dismissButton={TextButton(onClick=model::closeTerminalCreation,modifier=Modifier.heightIn(min=48.dp)){UiText("Cancel")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),modifier=Modifier.testTag("terminal_creation_confirmation"))
}
