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

/** One foreground confirmation; invalidation stops pre-admission only, never an admitted action. */
internal class AgentTerminalLease {private val gate=Any();private var active=true
    fun invalidate(){synchronized(gate){active=false}}
    fun <T> admit(block:()->T):T=synchronized(gate){if(!active)throw StorePolicy(ErrorCode.NotReady);block()}
}
class AgentTerminalMutationContext internal constructor(internal val original:Any,internal val lease:AgentTerminalLease)
internal class AgentTerminalCreation(val key:ConversationKey,val source:Uuid,val foreground:Any,val context:AgentTerminalMutationContext) {
    var loaded by mutableStateOf(false);var pending by mutableStateOf(true)
    var prior by mutableStateOf<LocalAction?>(null);var separate by mutableStateOf<Uuid?>(null)
    var loading:Any?=null
}
@Composable internal fun AgentTerminalCreationDialog(shown:AgentTerminalCreation,model:GoatrViewModel,hostName:String,agentTitle:String,connectionStatus:String,compactConnectionStatus:String){
    AlertDialog(onDismissRequest=model::closeAgentTerminal,title={GoatrTitleBar("New terminal beside agent",connectionStatus,!shown.loaded,compactStatus=compactConnectionStatus)},text={Column(Modifier.fillMaxWidth().heightIn(max=460.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(16.dp)){
        UiText("Create an ordinary shell beside this exact agent on the selected desktop. Herdr uses the original agent process’s current OS directory, which may differ from its conversation directory.")
        UiText("The desktop stays unfocused. No command is sent and no existing terminal is closed.")
        UiText(hostName,style=MaterialTheme.typography.titleMedium)
        UiText(agentTitle,style=MaterialTheme.typography.bodyMedium)
        var details by remember(shown) {mutableStateOf(false)}
        DetailDisclosure(details,{details=!details},"agent_terminal_details")
        if(details) UiText("Host "+shown.key.host+" · original session "+shown.key.target.sessionId.value,style=MaterialTheme.typography.bodySmall,softWrap=true)
        shown.prior?.let {prior->ReceiptPanel(shown.key.host,prior,model)
            if(prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))TextButton(onClick={model.permitSeparateAgentTerminal(shown,prior.metadata.actionId)},enabled=!shown.pending,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Add,"Create a separate terminal")}
        }
        TextButton(onClick={model.terminals(shown.key.host)},modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Terminal,"View terminals")}
        if(model.agentTerminalCreatedAvailable(shown))TextButton(onClick={model.openCreatedAgentTerminal(shown)},modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Terminal,"Open created terminal")}
        if(shown.pending)UiText("Inspect the retained action before creating another terminal. It is never replayed.",style=MaterialTheme.typography.bodySmall)
        TextButton(onClick={model.inspectAgentTerminal(shown)},modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check retained receipt")}
    }},confirmButton={TextButton(onClick={model.createAgentTerminal(shown)},enabled=!model.busy&&model.agentTerminalAllowed(shown),modifier=Modifier.heightIn(min=48.dp).testTag("agent_terminal_create")){IconLabel(MaterialGlyph.Terminal,"Create terminal")}},dismissButton={TextButton(onClick=model::closeAgentTerminal,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Close,"Cancel")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),modifier=Modifier.testTag("agent_terminal_confirmation"))
}
