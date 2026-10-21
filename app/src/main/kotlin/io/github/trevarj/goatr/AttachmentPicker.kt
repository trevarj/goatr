// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Deliberately local UI state, not a protocol capability, upload receipt or saved-state model. */
private class AttachmentPreparation(val ticket:AttachmentSourceTicket) {
    var prepared by mutableStateOf<PreparedAttachment?>(null)
    var bytes by mutableLongStateOf(0)
    var preparing by mutableStateOf(false)
    var cancelling by mutableStateOf(false)
    var failure by mutableStateOf<AttachmentPreparationFailure?>(null)
    var uploadHandedOff by mutableStateOf(false)
    var current by mutableStateOf(true)
}

/** System selection plus an explicit upload handoff. Owners supply exact source/foreground checks;
 * selecting/preparing bytes never sends, mutates a draft, reconnects or discovers a replacement. */
@Composable
internal fun AttachmentPicker(
    enabled:Boolean,
    captureSourceTicket:suspend ()->AttachmentSourceTicket?,
    returnSourceTicket:(AttachmentSourceTicket)->AttachmentSourceTicket?,
    cancelSourceTicket:(AttachmentSourceTicket?)->Unit,
    isSourceTicketCurrent:(AttachmentSourceTicket)->Boolean,
    onUpload:(AttachmentSourceTicket,PreparedAttachment)->Boolean,
    modifier:Modifier=Modifier,
) {
    val app=LocalContext.current.applicationContext
    val staging=remember(app){AttachmentStaging(app)}
    val scope=rememberCoroutineScope()
    val owner=LocalLifecycleOwner.current
    val capture by rememberUpdatedState(captureSourceTicket)
    val check by rememberUpdatedState(isSourceTicketCurrent)
    val returned by rememberUpdatedState(returnSourceTicket)
    val cancelled by rememberUpdatedState(cancelSourceTicket)
    val upload by rememberUpdatedState(onUpload)
    var foreground by remember(owner){mutableStateOf(owner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED))}
    var state by remember {mutableStateOf<AttachmentPreparation?>(null)}
    var pending by remember {mutableStateOf<Pair<AttachmentSourceTicket,Uri>?>(null)}
    var pickerTicket by remember {mutableStateOf<AttachmentSourceTicket?>(null)}
    var preparingJob by remember {mutableStateOf<Job?>(null)}
    var pickerOpen by remember {mutableStateOf(false)}
    var menu by remember {mutableStateOf(false)}
    val receive:(Uri?)->Unit={uri->
        val original=pickerTicket
        pickerTicket=null;pickerOpen=false
        if(uri!=null && uri.scheme=="content" && !uri.authority.isNullOrEmpty() && original!=null)pending=original to uri else original?.let {cancelled(it)}
    }
    val photo=rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia(),receive)
    val document=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument(),receive)
    DisposableEffect(owner){
        val observer=LifecycleEventObserver {_,event->
            if(event==Lifecycle.Event.ON_RESUME)foreground=true
            else if(event==Lifecycle.Event.ON_PAUSE){foreground=false
                // Picker's own lifecycle transition is expected. Other departures revoke local work.
                if(!pickerOpen){pending=null;state?.current=false;preparingJob?.cancel()}
            }
        }
        owner.lifecycle.addObserver(observer)
        onDispose {owner.lifecycle.removeObserver(observer);preparingJob?.cancel();(pickerTicket?:pending?.first)?.let {cancelled(it)}
            val original=state;val prepared=original?.prepared
            if(prepared!=null && original?.uploadHandedOff==false)scope.launch(NonCancellable){staging.discard(prepared)}
        }
    }
    LaunchedEffect(pending,foreground){
        val selection=pending ?: return@LaunchedEffect
        if(!foreground)return@LaunchedEffect
        pending=null
        val (picked,uri)=selection
        val ticket=returned(picked) ?: return@LaunchedEffect
        if(!check(ticket))return@LaunchedEffect
        val original=AttachmentPreparation(ticket);state=original
        preparingJob=scope.launch {
            original.preparing=true
            val measured=AtomicLong(0)
            val progressJob=launch {while(true){delay(100);original.bytes=measured.get()}}
            try {
                val prepared=staging.prepare(uri){measured.set(it)}
                original.bytes=prepared.size
                if(!foreground || state!==original || !check(ticket)){original.current=false;withContext(NonCancellable){staging.discard(prepared)}}
                else original.prepared=prepared
            }catch(cancelled:CancellationException){throw cancelled}
            catch(error:AttachmentPreparationException){original.failure=error.reason}
            catch(_:Exception){original.failure=AttachmentPreparationFailure.Unavailable}
            finally {progressJob.cancel();original.preparing=false;original.cancelling=false}
        }
    }
    val current=state
    LaunchedEffect(current,foreground){
        while(current!=null && state===current){
            if(!current.uploadHandedOff && (!foreground || !check(current.ticket)))current.current=false
            val prepared=current.prepared
            if(prepared!=null && !current.uploadHandedOff && System.currentTimeMillis()>=prepared.expiresAt){current.current=false;current.failure=AttachmentPreparationFailure.Expired;if(staging.discard(prepared))current.prepared=null}
            delay(250)
        }
    }
    fun closeLocal(){
        val original=state ?: return
        if(original.preparing){original.cancelling=true;original.current=false;preparingJob?.cancel();return}
        if(original.uploadHandedOff){state=null;return}
        scope.launch {val prepared=original.prepared
            if(prepared==null || staging.discard(prepared)){if(state===original)state=null}
            else {original.current=false;original.failure=AttachmentPreparationFailure.Changed}
        }
    }
    Box(modifier){
        TextButton(onClick={menu=true},enabled=enabled && foreground && !pickerOpen && current==null,modifier=Modifier.heightIn(min=48.dp).testTag("attachment_choose")) {AttachmentIconLabel(R.drawable.ic_material_attach_file,"Attach file")}
        DropdownMenu(expanded=menu,onDismissRequest={menu=false}) {
            DropdownMenuItem(text={UiText("Choose photo")},leadingIcon={androidx.compose.material3.Icon(androidx.compose.ui.res.painterResource(R.drawable.ic_material_photo_library),null)},onClick={
                menu=false
                if(foreground)scope.launch {capture()?.let {ticket->pickerTicket=ticket;pickerOpen=true;try {photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))}catch(_:Exception){pickerOpen=false;pickerTicket=null;cancelled(ticket)}}}
            })
            DropdownMenuItem(text={UiText("Choose document")},leadingIcon={androidx.compose.material3.Icon(androidx.compose.ui.res.painterResource(R.drawable.ic_material_description),null)},onClick={
                menu=false
                if(foreground)scope.launch {capture()?.let {ticket->pickerTicket=ticket;pickerOpen=true;try {document.launch(arrayOf("*/*"))}catch(_:Exception){pickerOpen=false;pickerTicket=null;cancelled(ticket)}}}
            })
        }
    }
    if(current!=null)AlertDialog(
        onDismissRequest=::closeLocal,
        title={UiText("Prepare attachment")},
        text={Column(Modifier.fillMaxWidth().heightIn(max=420.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)){
            UiText("Up to 8 MiB. Original bytes are encrypted on this phone for up to one hour. No metadata is stripped.")
            when {
                current.cancelling->UiText("Stopping local preparation…")
                current.preparing->{LinearProgressIndicator(modifier=Modifier.fillMaxWidth());UiText("Prepared ${current.bytes / 1024} KiB locally")}
                current.uploadHandedOff->UiText("Upload handed to this conversation. Follow its progress and receipts; this does not send a prompt.")
                current.failure!=null->UiText(when(current.failure){
                    AttachmentPreparationFailure.Empty->"The selected file is empty."
                    AttachmentPreparationFailure.TooLarge->"The actual file exceeds 8 MiB."
                    AttachmentPreparationFailure.Capacity->"Private attachment staging is full. Discard retained local preparations first."
                    AttachmentPreparationFailure.Changed->"The prepared file changed or could not be safely discarded."
                    AttachmentPreparationFailure.Expired->"This local preparation has expired. Discard it and choose again."
                    else->"The selected file could not be prepared. No upload was started."
                },color=MaterialTheme.colorScheme.error)
                current.prepared!=null->{val prepared=current.prepared!!
                    UiText(prepared.displayName,softWrap=true)
                    UiText("${prepared.size} bytes · ${prepared.mediaType}",style=MaterialTheme.typography.bodySmall,softWrap=true)
                    UiText("Upload stores a private desktop file. Inserting its path is a separate choice; your draft is kept.")
                }
            }
            if(!current.current && !current.uploadHandedOff)UiText("The original foreground conversation is no longer available. This preparation cannot be uploaded.",color=MaterialTheme.colorScheme.error)
        }},
        confirmButton={if(!current.uploadHandedOff)TextButton(onClick={
            val prepared=current.prepared
            // No suspension between final original-ticket admission and this explicit handoff.
            if(prepared!=null && !current.preparing && current.current && foreground && check(current.ticket) && System.currentTimeMillis()<prepared.expiresAt){
                if(upload(current.ticket,prepared)){current.uploadHandedOff=true;state=null}else current.current=false
            }else current.current=false
        },enabled=enabled && foreground && current.current && !current.preparing && current.failure==null && current.prepared!=null,modifier=Modifier.heightIn(min=48.dp).testTag("attachment_upload")){AttachmentIconLabel(R.drawable.ic_material_file_upload,"Upload")}},
        dismissButton={TextButton(onClick=::closeLocal,enabled=!current.cancelling,modifier=Modifier.heightIn(min=48.dp).testTag("attachment_discard")){IconLabel(if(current.uploadHandedOff)MaterialGlyph.Close else MaterialGlyph.Delete,if(current.uploadHandedOff)"Close" else if(current.preparing)"Cancel preparation" else "Discard local file")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
        modifier=Modifier.testTag("attachment_preparation"),
    )
}

@Composable internal fun AttachmentIconLabel(@androidx.annotation.DrawableRes icon:Int,label:String){Row(horizontalArrangement=Arrangement.spacedBy(6.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically){Icon(androidx.compose.ui.res.painterResource(icon),null,modifier=Modifier.size(18.dp));UiText(label,maxLines=1,softWrap=false)}}
