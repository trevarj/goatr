// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

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
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun AgentSessionCreationSheet(shown:AgentSessionCreation,connection:ConnectionView,model:GoatrViewModel) {
    val inventory=connection.inventory[shown.key.host]
    val server=inventory?.servers?.singleOrNull {it.serverId==shown.server}
    ModalBottomSheet(onDismissRequest=model::closeAgentSession,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true),containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().heightIn(max=680.dp).navigationBarsPadding().imePadding().padding(16.dp).testTag("agent_session_creation"),verticalArrangement=Arrangement.spacedBy(10.dp)) {
            Row(Modifier.fillMaxWidth()) {
                UiText("New session",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f))
                TextButton(onClick=model::closeAgentSession,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Close,"Close")}
            }
            UiText("Start a separate workspace on this agent’s desktop server. Choose a configured project; this does not use the agent’s directory or place anything in its workspace.",style=MaterialTheme.typography.bodySmall)
            UiText((server?.name?.value ?: "Original desktop server")+" · "+providerLabel(shown.provider),style=MaterialTheme.typography.labelLarge)
            if(!shown.loaded)LinearProgressIndicator(Modifier.fillMaxWidth())
            val prior=shown.prior
            if(prior!=null && shown.separate!=prior.metadata.actionId) {
                Column(Modifier.fillMaxWidth().heightIn(max=360.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    ReceiptPanel(shown.key.host,prior,model)
                    UiText("This desktop’s retained creation is never retried. A separate session needs a new name and explicit confirmation.",style=MaterialTheme.typography.bodySmall)
                    if(prior.authoritative&&prior.outcomeShown&&prior.receipt?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain))TextButton(onClick={model.prepareSeparateAgentSession(shown,prior.metadata.actionId)},enabled=!shown.pending&&!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Add,"Start a separate session")}
                }
            } else if(inventory!=null && server!=null) {
                val gate=creationReadinessReason(connection.hello[shown.key.host],model.busy,connection.inventoryReady[shown.key.host]==true,connection.mutationReady[shown.key.host]==true,!model.agentSessionAllowed(shown))
                Box(Modifier.fillMaxWidth().heightIn(min=280.dp,max=460.dp)) {
                    CreateSession(inventory.copy(servers=listOf(server)),connection.hello[shown.key.host],model.agentSessionAllowed(shown)&&!model.busy,shown.memory,gate?.message,gate?.tag,pinnedProvider=shown.provider,requireProjectSelection=true) {_,project,_,name->model.createAgentSession(shown,project,name)}
                }
            } else UiText("Refresh this original desktop’s session inventory before creating a session.")
            TextButton(onClick={model.inspectAgentSession(shown)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check retained creation")}
            if(shown.pending && shown.loaded)UiText("A creation is still pending or unknown. Inspect its receipt; no action is replayed.",style=MaterialTheme.typography.bodySmall)
        }
    }
    shown.confirming?.let {action->AlertDialog(onDismissRequest={shown.confirming=null},title={UiText("Start a separate session?")},text={UiText("The prior creation may already have resources. Leave them untouched. This creates a new session in an explicitly selected configured project; it never retries or cleans up the prior action.")},confirmButton={TextButton(onClick={model.confirmSeparateAgentSession(shown,action)},enabled=!model.busy){UiText("Continue with a new session")}},dismissButton={TextButton(onClick={shown.confirming=null}){UiText("Cancel")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))}
}
