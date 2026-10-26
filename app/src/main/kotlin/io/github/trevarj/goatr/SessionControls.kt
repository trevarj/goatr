// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/** One foreground panel and its native catalogues, never a saved session preference. */
internal class SessionControlsState(val key:ConversationKey,val source:Uuid,val foreground:Any,val lease:NativeControlsLease=NativeControlsLease()) {
    var catalogue by mutableStateOf<SessionControlsReadResult?>(null)
    var loading by mutableStateOf(false)
    var pending by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var receipt by mutableStateOf<Receipt?>(null)
    var revision=0L
    var model by mutableStateOf<Uuid?>(null)
    var effort by mutableStateOf<Text?>(null)
    var mode by mutableStateOf<ControlMode?>(null)
    var queueOpen by mutableStateOf(false)
    var queueLoading by mutableStateOf(false)
    var queueRevision=0L
    var queueEntries by mutableStateOf<List<SessionQueueEntry>>(emptyList())
    var queueText by mutableStateOf<Map<Id,String>>(emptyMap())
    var next by mutableStateOf<Id?>(null)
    var queueLoaded by mutableStateOf(false)
    var queueError by mutableStateOf<String?>(null)
    var queueReceipt by mutableStateOf<Receipt?>(null)
    private var controlsChangedRevision:Long?=null
    private var queueChangedRevision:Long?=null
    fun observeRevisions(controls:Long,queue:Long) {
        if(!lease.active.get())return
        if(controls!=revision)controlsChangedRevision=controls
        if(queue!=queueRevision)queueChangedRevision=queue
    }
    val controlsRefreshNeeded:Boolean get()=lease.active.get()&&!pending&&!loading&&controlsChangedRevision!=null
    val queueRefreshNeeded:Boolean get()=lease.active.get()&&!pending&&!queueLoading&&queueOpen&&queueChangedRevision!=null
    fun beginControlsRead(current:Long):Boolean {
        if(!lease.active.get()||pending||loading)return false
        revision=current;controlsChangedRevision=null;loading=true;return true
    }
    fun controlsReadCurrent(read:Long):Boolean=lease.active.get()&&revision==read&&controlsChangedRevision==null
    fun settleControlsRead(){loading=false}
    fun beginQueueRead(current:Long):Boolean {
        if(!lease.active.get()||pending||queueLoading||!queueOpen)return false
        queueRevision=current;queueChangedRevision=null;queueLoading=true;return true
    }
    fun queueReadCurrent(read:Long):Boolean=lease.active.get()&&queueOpen&&queueRevision==read&&queueChangedRevision==null
    fun settleQueueRead(){queueLoading=false}
    fun retire(){lease.invalidate();controlsChangedRevision=null;queueChangedRevision=null;queueOpen=false}
    fun changes(boot:Uuid):SessionControlsApplyParams? {
        val original=catalogue ?: return null
        val changedModel=model?.takeUnless {it==original.current.modelRef}
        val changedEffort=effort?.takeUnless {it==original.current.effort&&changedModel==null}
        val changedMode=mode?.takeUnless {it==original.current.mode}
        if(changedModel==null&&changedEffort==null&&changedMode==null)return null
        return SessionControlsApplyParams(boot,key.target,original.catalogueGeneration,changedModel,changedEffort,changedMode)
    }
}

internal fun deliveryLabel(provider:Provider,delivery:PromptDelivery):String=when(delivery) {
    PromptDelivery.Native->if(provider==Provider.Omp)"Automatic" else "Native"
    PromptDelivery.Steer->if(provider==Provider.Omp)"Queue steer" else "Steer current turn"
    PromptDelivery.FollowUp->"Queue follow-up"
    PromptDelivery.Aside->"Aside"
    PromptDelivery.Queue->"Native queue"
}

@OptIn(ExperimentalMaterial3Api::class,ExperimentalLayoutApi::class)
@Composable
internal fun SessionControls(shown:SessionControlsState,model:GoatrViewModel,provider:Provider) {
    val current=model.controlsAdmitted(shown)
    val enabled=current&&!shown.loading&&!shown.pending
    var edit by remember(shown,shown.queueRevision){mutableStateOf<SessionQueueEntry?>(null)}
    var text by remember(shown,shown.queueRevision){mutableStateOf("")}
    ModalBottomSheet(onDismissRequest=model::closeSessionControls,containerColor=MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.85f).imePadding().padding(16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            GoatrTitleBar("Shared native controls","Changes affect this original desktop session",shown.loading||shown.pending,compactStatus=if(current)"Shared native" else "Unavailable",actions={TextButton(onClick=model::closeSessionControls,modifier=Modifier.heightIn(min=48.dp)){UiText("Close")}})
            Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                UiText("Opening or dismissing does not change native settings. Only Apply changes future native work.",style=MaterialTheme.typography.bodySmall)
                if(!current)UiText("The original foreground session is unavailable. Reopen controls from its current conversation.")
                shown.error?.let {UiText(it)}
                if(shown.pending)UiText("Waiting for the original native effective state. A queued update is not confirmation.")
                shown.receipt?.let {receipt->
                    UiText("Apply: ${receipt.state}")
                    if(receipt.state==ReceiptState.Uncertain)UiText("Some settings may have changed. Inspect the actual native state; no rollback or retry.")
                    TextButton(onClick={model.inspect(shown.key.host,receipt.actionId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Inspect this Apply receipt")}
                }
                shown.catalogue?.let {catalogue->
                    UiText("Model",style=MaterialTheme.typography.titleSmall)
                    if(catalogue.current.modelRef==null)UiText("Current native model unknown",style=MaterialTheme.typography.bodySmall)
                    if(catalogue.models.isEmpty())UiText("No controllable native model surface.")
                    catalogue.models.forEach {entry->FilterChip(selected=shown.model==entry.reference,onClick={shown.model=entry.reference;shown.effort=null},enabled=enabled,label={UiText(entry.label.value)},modifier=Modifier.heightIn(min=48.dp).testTag("control_model_${entry.reference.value}"))}
                    val selected=catalogue.models.singleOrNull {it.reference==(shown.model?:catalogue.current.modelRef)}
                    if(selected!=null&&selected.efforts.isNotEmpty()) {
                        UiText("Effort / thinking",style=MaterialTheme.typography.titleSmall)
                        if(catalogue.current.effort==null)UiText("Current native effort unknown",style=MaterialTheme.typography.bodySmall)
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)){selected.efforts.forEach {effort->FilterChip(selected=shown.effort==effort,onClick={shown.effort=effort},enabled=enabled,label={UiText(effort)},modifier=Modifier.heightIn(min=48.dp))}}
                    }
                    if(catalogue.modes.isNotEmpty()) {
                        UiText("Native mode",style=MaterialTheme.typography.titleSmall)
                        if(catalogue.current.mode==null)UiText("Current mode unknown; it is not assumed Default.",style=MaterialTheme.typography.bodySmall)
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)){catalogue.modes.forEach {mode->FilterChip(selected=shown.mode==mode,onClick={shown.mode=mode},enabled=enabled&&(shown.model!=null||catalogue.current.modelRef!=null),label={UiText(mode.name)},modifier=Modifier.heightIn(min=48.dp))}}
                    }else UiText("Native planning controls unavailable for this provider.",style=MaterialTheme.typography.bodySmall)
                    Button(onClick={model.applySessionControls(shown)},enabled=enabled&&model.sessionControlsApplyAllowed(shown),modifier=Modifier.heightIn(min=48.dp)) {UiText("Apply")}
                    UiText("Message delivery",style=MaterialTheme.typography.titleSmall)
                    UiText("This selects how the next explicit Send is delivered. It never sends the draft.",style=MaterialTheme.typography.bodySmall)
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)){catalogue.deliveries.forEach {delivery->FilterChip(selected=model.composerOptions(shown.key).delivery?.let {it==delivery}?: (delivery==PromptDelivery.Native),onClick={model.selectPromptDelivery(shown,delivery)},enabled=enabled,label={UiText(deliveryLabel(provider,delivery))},modifier=Modifier.heightIn(min=48.dp))}}
                    if(provider==Provider.Omp)UiText("Queue steer / follow-up stay queued even while idle. Aside starts idle or injects at a native step boundary. Native Skill invocation and generic Plan entry/exit are unavailable.",style=MaterialTheme.typography.bodySmall)
                    if(catalogue.queueAvailable)TextButton(onClick={model.openNativeQueue(shown)},enabled=enabled,modifier=Modifier.heightIn(min=48.dp)){UiText("Inspect native queue")}
                    else UiText("Native queue inspection and editing unavailable.",style=MaterialTheme.typography.bodySmall)
                }
                TextButton(onClick={model.refreshSessionControls(shown)},enabled=current&&!shown.pending&&!shown.loading,modifier=Modifier.heightIn(min=48.dp)){UiText("Read native settings again")}
                if(shown.queueOpen) {
                    UiText("Native queue",style=MaterialTheme.typography.titleMedium)
                    UiText("Native delivery may start queued entries automatically. Start below always selects the displayed entry.",style=MaterialTheme.typography.bodySmall)
                    shown.queueError?.let {UiText(it)}
                    shown.queueReceipt?.let {receipt->
                        UiText("Queue action: ${receipt.state}")
                        (receipt.result as? ActionResultQueue)?.deleted?.let {deleted->UiText(if(deleted)"Native entry deleted" else "Native entry was not deleted")}
                        if(receipt.state==ReceiptState.Uncertain)UiText("This queue action may already have taken effect. Inspect it; do not repeat an uncertain action.")
                        TextButton(onClick={model.inspect(shown.key.host,receipt.actionId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Inspect this queue receipt")}
                    }
                    if(shown.queueLoading)UiText("Reading the original native queue…")
                    if(shown.queueLoaded&&shown.queueEntries.isEmpty())UiText("Native queue is empty")
                    shown.queueEntries.forEachIndexed {index,entry->
                        HorizontalDivider()
                        SelectionContainer {UiText(shown.queueText[entry.id]?:"Content unavailable")}
                        if(!entry.editable)UiText("Contains native non-text input; text editing is unavailable.",style=MaterialTheme.typography.bodySmall)
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            TextButton(onClick={model.startQueueEntry(shown,entry.id)},enabled=enabled&&!shown.queueLoading&&shown.queueLoaded,modifier=Modifier.heightIn(min=48.dp)){UiText("Start this entry")}
                            TextButton(onClick={model.deleteQueueEntry(shown,entry.id)},enabled=enabled&&!shown.queueLoading&&shown.queueLoaded,modifier=Modifier.heightIn(min=48.dp)){UiText("Delete")}
                            if(entry.editable)TextButton(onClick={edit=entry;text=shown.queueText[entry.id].orEmpty()},enabled=enabled&&!shown.queueLoading&&shown.queueLoaded&&entry.id in shown.queueText,modifier=Modifier.heightIn(min=48.dp)){UiText("Edit")}
                            if(index>0)TextButton(onClick={model.moveQueueEntryUp(shown,entry.id)},enabled=enabled&&!shown.queueLoading&&shown.queueLoaded&&shown.next==null,modifier=Modifier.heightIn(min=48.dp)){UiText("Move up")}
                        }
                    }
                    if(shown.next!=null)TextButton(onClick={model.moreNativeQueue(shown)},enabled=enabled&&!shown.queueLoading,modifier=Modifier.heightIn(min=48.dp)){UiText("Load next native page")}
                    TextButton(onClick={model.openNativeQueue(shown)},enabled=enabled&&!shown.queueLoading,modifier=Modifier.heightIn(min=48.dp)){UiText("Refresh queue")}
                }
            }
        }
    }
    edit?.let {entry->AlertDialog(onDismissRequest={edit=null},title={UiText("Edit native queued text")},text={OutlinedTextField(text,{text=it},modifier=Modifier.heightIn(max=280.dp).verticalScroll(rememberScrollState()))},confirmButton={TextButton(onClick={edit=null;model.updateQueueEntry(shown,entry.id,text)},enabled=enabled&&text.isNotEmpty()&&utf8Size(text)<=262144,modifier=Modifier.heightIn(min=48.dp)){UiText("Apply edit")}},dismissButton={TextButton(onClick={edit=null},modifier=Modifier.heightIn(min=48.dp)){UiText("Keep entry")}})}
}
