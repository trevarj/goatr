// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.serialization.Serializable

/** One original user lease; invalidation prevents new frames and never cancels an admitted frame. */
internal class AttachmentActionLease {
    private val gate=Any();private var active=true
    fun invalidate(){synchronized(gate){active=false}}
    fun <T> admit(block:()->T):T=synchronized(gate){if(!active)throw StorePolicy(ErrorCode.NotReady);block()}
}
class AttachmentMutationContext internal constructor(internal val original:Any,internal val lease:AttachmentActionLease,val host:String,val target:Target,val source:Uuid?,internal val cleanupUpload:Uuid?=null,internal val resumeStatus:AttachmentsStatusResult?=null)
@Serializable
data class LocalAttachmentFile(val id:String,val size:Long,val sha256:String,val expiresAt:Long,val device:Long,val inode:Long,val ciphertextSize:Long) {
    internal fun prepared():PreparedAttachment=PreparedAttachment(java.util.UUID.fromString(id),"Prepared attachment","application/octet-stream",size,sha256,expiresAt,device,inode,ciphertextSize)
    companion object {internal fun from(file:PreparedAttachment)=LocalAttachmentFile(file.id.toString(),file.size,file.sha256,file.expiresAt,file.device,file.inode,file.ciphertextSize)}
}
/** Durable recovery facts only. A URI, display filename, MIME, payload or action lease is never stored. */
@Serializable
data class LocalAttachmentUpload(val host:String,val target:Target,val source:Uuid,val uploadId:Uuid,val local:LocalAttachmentFile?,val size:Long,val sha256:String,val actionId:Uuid?=null,val committedBytes:Long=0,val providerPath:String?=null,val published:Boolean=false,val discarded:Boolean=false,val forgetPending:Boolean=false) {
    internal fun validateLocal(){EndpointId(host);target.validate();source.validate();uploadId.validate();actionId?.validate();requireWire(size in 1..8L*1024*1024&&committedBytes in 0..size&&sha256.matches(Regex("[0-9a-f]{64}")));providerPath?.let {requireWire(validUnicode(it)&&it.startsWith('/')&&it.toByteArray(Charsets.UTF_8).size<=4096&&it.none(Char::isISOControl)&&published)};local?.let {file->requireWire(java.util.UUID.fromString(file.id).toString()==file.id&&file.size==size&&file.sha256==sha256&&file.expiresAt>0&&file.inode!=0L&&file.ciphertextSize==size+37)}}
    override fun toString()="LocalAttachmentUpload(redacted)"
}
internal class AttachmentConsentScope(val key:ConversationKey,val source:Uuid,val generation:Any,val context:AttachmentMutationContext,val draft:String,val revision:Long,val cursor:Pair<Int,Int>)
internal class AttachmentTransfer(val ticket:AttachmentSourceTicket,val original:AttachmentConsentScope,val prepared:PreparedAttachment?,record:LocalAttachmentUpload) {
    var record by mutableStateOf(record)
    var working by mutableStateOf(false)
    var paused by mutableStateOf(false)
    var unknown by mutableStateOf(false)
    var notice by mutableStateOf<String?>(null)
    var receipt by mutableStateOf<Receipt?>(null)
    var status by mutableStateOf<AttachmentsStatusResult?>(null)
    var consentAction by mutableStateOf<Uuid?>(null)
    var inspectedAction:Uuid?=null
    var inspection:Any?=null
    var reviewedDraft by mutableStateOf<Pair<Long,Pair<Int,Int>>?>(null)
}

internal class AttachmentRecovery(val generation:Any,record:LocalAttachmentUpload) {
    var record by mutableStateOf(record)
    var loading by mutableStateOf(false)
    var receipt by mutableStateOf<Receipt?>(null)
    var status by mutableStateOf<AttachmentsStatusResult?>(null)
    var notice by mutableStateOf<String?>(null)
    var cleanup:AttachmentMutationContext?=null
}

@androidx.compose.runtime.Composable
internal fun AttachmentTransferSheet(transfer:AttachmentTransfer,model:GoatrViewModel,connectionStatus:String,compactConnectionStatus:String){
    var discard by remember(transfer){mutableStateOf(false)}
    var details by remember(transfer){mutableStateOf(false)}
    val record=transfer.record
    androidx.compose.material3.AlertDialog(
        onDismissRequest=model::closeAttachments,
        title={GoatrTitleBar("Attachment",connectionStatus,transfer.working,compactStatus=compactConnectionStatus)},
        text={androidx.compose.foundation.layout.Column(androidx.compose.ui.Modifier.fillMaxWidth().heightIn(max=420.dp).verticalScroll(androidx.compose.foundation.rememberScrollState()),verticalArrangement=androidx.compose.foundation.layout.Arrangement.spacedBy(16.dp)){
            androidx.compose.material3.Text(transfer.prepared?.displayName ?: "Retained desktop attachment",softWrap=true)
            androidx.compose.material3.Text("${record.committedBytes} / ${record.size} bytes committed on the desktop")
            androidx.compose.material3.LinearProgressIndicator(progress={record.committedBytes.toFloat()/record.size.toFloat()},modifier=androidx.compose.ui.Modifier.fillMaxWidth())
            if(transfer.working)androidx.compose.material3.Text("Processing this action; the bar shows desktop-confirmed bytes only.",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)
            transfer.notice?.let {androidx.compose.material3.Text(it)}
            transfer.receipt?.let {receipt->androidx.compose.material3.Text("Action status: ${receipt.state.name}",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)}
            DetailDisclosure(details,{details=!details},"attachment_upload_details")
            if(details) {
                androidx.compose.material3.Text("Upload ${record.uploadId.value}",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)
                transfer.receipt?.let {androidx.compose.material3.Text("Action ${it.actionId.value}",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)}
            }
            if(transfer.unknown)androidx.compose.material3.Text("Outcome unknown. Inspect the original action and desktop offset. No bytes are retried automatically.")
            if(record.published && !record.discarded)androidx.compose.material3.Text("Uploaded files expire after seven days. A path does not bypass the provider’s normal file permissions.")
            transfer.status?.progress?.providerPath?.let {path->androidx.compose.foundation.text.selection.SelectionContainer {androidx.compose.material3.Text(path,softWrap=true,style=androidx.compose.material3.MaterialTheme.typography.bodySmall)}}
            if(transfer.status?.insertionReady==true){
                if(!model.attachmentDraftCurrent(transfer))androidx.compose.material3.TextButton(onClick={model.reviewAttachmentDraft(transfer)},enabled=!transfer.working,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Edit,"Review current draft")}
                androidx.compose.material3.TextButton(onClick={model.insertAttachment(transfer)},enabled=!transfer.working&&model.attachmentDraftCurrent(transfer)&&model.attachmentAdmitted(transfer.ticket),modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp).testTag("attachment_insert")){AttachmentIconLabel(R.drawable.ic_material_content_paste,"Append path to draft")}
            }
            if(transfer.status?.state==AttachmentUploadState.Receiving&&transfer.prepared!=null&&!transfer.working)androidx.compose.material3.TextButton(onClick={model.prepareAttachmentResume(transfer)},modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp).testTag("attachment_prepare_resume")){IconLabel(MaterialGlyph.Play,"Review separate continuation")}
            androidx.compose.material3.TextButton(onClick={discard=true},enabled=!transfer.working&&!record.discarded,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp).testTag("attachment_discard_remote")){IconLabel(MaterialGlyph.Delete,"Discard desktop staging")}
            androidx.compose.material3.Text("Closing pauses further frames. An already admitted action and its retained receipt continue.",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)
        }},
        confirmButton={androidx.compose.material3.TextButton(onClick={model.inspectAttachment(transfer)},enabled=!transfer.working,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp).testTag("attachment_inspect")){IconLabel(MaterialGlyph.Refresh,"Check receipt and status")}},
        dismissButton={androidx.compose.material3.TextButton(onClick=model::closeAttachments,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Close,"Close")}},
        properties=androidx.compose.ui.window.DialogProperties(securePolicy=androidx.compose.ui.window.SecureFlagPolicy.SecureOn),
    )
    if(discard)androidx.compose.material3.AlertDialog(onDismissRequest={discard=false},title={androidx.compose.material3.Text("Discard private desktop file?")},text={androidx.compose.material3.Text("This removes only the companion’s retained staging for this upload. It does not remove the original selected file, provider copies or conversation text.")},confirmButton={androidx.compose.material3.TextButton(onClick={discard=false;model.discardAttachment(transfer)}){IconLabel(MaterialGlyph.Delete,"Confirm discard")}},dismissButton={androidx.compose.material3.TextButton(onClick={discard=false}){androidx.compose.material3.Text("Keep file")}},properties=androidx.compose.ui.window.DialogProperties(securePolicy=androidx.compose.ui.window.SecureFlagPolicy.SecureOn))
    transfer.consentAction?.let {action->val status=transfer.status
        if(status!=null)androidx.compose.material3.AlertDialog(onDismissRequest={transfer.consentAction=null},title={androidx.compose.material3.Text("Start a separate continuation?")},text={androidx.compose.material3.Text("Inspected action ${action.value}. The desktop confirms ${status.progress.committedBytes.value} bytes committed. Continue only from that confirmed offset with new action IDs. The prior action is not replayed.")},confirmButton={androidx.compose.material3.TextButton(onClick={model.confirmAttachmentResume(transfer,action,status)}){IconLabel(MaterialGlyph.Play,"Continue from confirmed offset")}},dismissButton={androidx.compose.material3.TextButton(onClick={transfer.consentAction=null}){androidx.compose.material3.Text("Keep paused")}},properties=androidx.compose.ui.window.DialogProperties(securePolicy=androidx.compose.ui.window.SecureFlagPolicy.SecureOn))
    }
}

@androidx.compose.runtime.Composable
internal fun AttachmentRecoverySheet(recovery:AttachmentRecovery,model:GoatrViewModel,connectionStatus:String,compactConnectionStatus:String){
    var discard by remember(recovery){mutableStateOf(false)}
    var details by remember(recovery){mutableStateOf(false)}
    androidx.compose.material3.AlertDialog(onDismissRequest=model::closeAttachmentRecovery,title={GoatrTitleBar("Retained attachment",connectionStatus,recovery.loading,compactStatus=compactConnectionStatus)},text={androidx.compose.foundation.layout.Column(androidx.compose.ui.Modifier.fillMaxWidth().heightIn(max=420.dp).verticalScroll(androidx.compose.foundation.rememberScrollState()),verticalArrangement=androidx.compose.foundation.layout.Arrangement.spacedBy(12.dp)){
        androidx.compose.material3.Text("${recovery.record.committedBytes} / ${recovery.record.size} last known committed bytes")
        androidx.compose.material3.LinearProgressIndicator(progress={recovery.record.committedBytes.toFloat()/recovery.record.size.toFloat()},modifier=androidx.compose.ui.Modifier.fillMaxWidth())
        if(recovery.loading)androidx.compose.material3.Text("Checking retained desktop status…",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)
        recovery.status?.let {androidx.compose.material3.Text("Desktop state: ${it.state.name}")}
        recovery.receipt?.let {androidx.compose.material3.Text("Action status: ${it.state.name}",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)}
        DetailDisclosure(details,{details=!details},"attachment_recovery_details")
        if(details) {
            androidx.compose.material3.Text("Upload ${recovery.record.uploadId.value}",style=androidx.compose.material3.MaterialTheme.typography.bodySmall,softWrap=true)
            recovery.receipt?.let {androidx.compose.material3.Text("Action ${it.actionId.value}",style=androidx.compose.material3.MaterialTheme.typography.bodySmall)}
        }
        recovery.notice?.let {androidx.compose.material3.Text(it)}
        androidx.compose.material3.Text("These facts do not authorize upload or path insertion. Return to the exact original conversation for a fresh admitted view.")
        androidx.compose.material3.TextButton(onClick={model.openAttachmentOriginal(recovery)},enabled=!recovery.loading,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Chat,"Open original conversation")}
        androidx.compose.material3.TextButton(onClick={discard=true},enabled=!recovery.loading&&!recovery.record.discarded,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Delete,"Discard desktop staging")}
    }},confirmButton={androidx.compose.material3.TextButton(onClick={model.inspectAttachmentRecovery(recovery)},enabled=!recovery.loading,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check receipt and status")}},dismissButton={androidx.compose.material3.TextButton(onClick=model::closeAttachmentRecovery,modifier=androidx.compose.ui.Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Close,"Close")}},properties=androidx.compose.ui.window.DialogProperties(securePolicy=androidx.compose.ui.window.SecureFlagPolicy.SecureOn))
    if(discard)androidx.compose.material3.AlertDialog(onDismissRequest={discard=false},title={androidx.compose.material3.Text("Discard original upload staging?")},text={androidx.compose.material3.Text("This applies only to the retained upload on this paired desktop. It never selects a replacement session or removes provider copies.")},confirmButton={androidx.compose.material3.TextButton(onClick={discard=false;model.discardAttachmentRecovery(recovery)}){IconLabel(MaterialGlyph.Delete,"Confirm discard")}},dismissButton={androidx.compose.material3.TextButton(onClick={discard=false}){androidx.compose.material3.Text("Keep file")}},properties=androidx.compose.ui.window.DialogProperties(securePolicy=androidx.compose.ui.window.SecureFlagPolicy.SecureOn))
}
