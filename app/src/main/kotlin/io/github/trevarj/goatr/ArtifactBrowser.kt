// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean

/** Opaque original Live ownership; contains neither paths nor persistence. */
internal class ArtifactReadContext internal constructor(internal val original:Any){internal val active=AtomicBoolean(true);internal fun invalidate(){active.set(false)}}
internal class ArtifactBrowserState(val host:String,val source:ArtifactSource,val context:ArtifactReadContext,val origin:Screen,val conversation:ConversationKey?,var foreground:Any){
 var paused by mutableStateOf(false)
 var entries by mutableStateOf<List<ArtifactEntry>>(emptyList());var cursor by mutableStateOf<Uuid?>(null)
 var breadcrumbs by mutableStateOf<List<Pair<Uuid,String>>>(emptyList());var loading by mutableStateOf(false)
 var error by mutableStateOf<String?>(null);var preview by mutableStateOf<String?>(null)
 var selected by mutableStateOf<ArtifactEntry?>(null);var exporting by mutableStateOf(false)
 var progress by mutableStateOf(0L);var exportConfirmation by mutableStateOf<ArtifactEntry?>(null)
 var ticket:Any=Any();var job:kotlinx.coroutines.Job?=null
 override fun toString()="ArtifactBrowserState(redacted)"
}
internal class ArtifactPickerTicket(val browser:ArtifactBrowserState,val entry:ArtifactEntry,val token:Any)

@Composable
internal fun ArtifactBrowser(shown:ArtifactBrowserState,model:GoatrViewModel){
 val scope=rememberCoroutineScope();var pending by remember(shown){mutableStateOf<ArtifactPickerTicket?>(null)}
 DisposableEffect(shown){onDispose{pending?.let(model::cancelArtifactExportPicker)}}
 val picker=rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")){uri:Uri?->
  val ticket=pending;pending=null;if(ticket!=null)model.returnArtifactExportPicker(ticket,uri)
 }
 Dialog(onDismissRequest=model::closeArtifacts,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)){
  Surface(Modifier.fillMaxSize(),color=MaterialTheme.colorScheme.background){
   Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding().padding(horizontal=12.dp)){
    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp)){
     IconButton(onClick=model::closeArtifacts,modifier=Modifier.size(48.dp)){GoatrIcon(MaterialGlyph.Back,"Close files")}
     Column(Modifier.weight(1f).padding(top=8.dp)){UiText("Files",style=MaterialTheme.typography.titleLarge);UiText("Read-only · original process directory",style=MaterialTheme.typography.bodySmall)}
     IconButton(onClick={model.refreshArtifacts(shown)},enabled=!shown.paused&&!shown.loading&&!shown.exporting,modifier=Modifier.size(48.dp)){GoatrIcon(MaterialGlyph.Refresh,"Refresh directory")}
    }
    if(shown.breadcrumbs.isNotEmpty())TextButton(onClick={model.artifactUp(shown)},enabled=!shown.paused&&!shown.loading&&!shown.exporting){IconLabel(MaterialGlyph.Back,"Up one directory")}
    if(shown.loading||shown.exporting)LinearProgressIndicator(Modifier.fillMaxWidth())
    shown.error?.let{UiText(it,color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodyMedium)}
    if(shown.exporting)UiText("Exported ${shown.progress} bytes",style=MaterialTheme.typography.bodySmall)
    if(shown.preview!=null){
     TextButton(onClick={shown.preview=null;shown.selected=null}){IconLabel(MaterialGlyph.Back,"Back to files")}
     LazyColumn(Modifier.weight(1f).fillMaxWidth(),contentPadding=PaddingValues(bottom=24.dp)){item{UiText(shown.preview.orEmpty(),fontFamily=FontFamily.Monospace,style=MaterialTheme.typography.bodyMedium)}}
    }else LazyColumn(Modifier.weight(1f).fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp),contentPadding=PaddingValues(bottom=24.dp)){
     if(shown.entries.isEmpty()&&!shown.loading)item{GroupedSurface{UiText("No readable files or directories.")}}
     items(shown.entries,key={it.reference.value}){entry->GroupedSurface{
      UiText(entry.name,style=MaterialTheme.typography.titleMedium)
      UiText(if(entry.kind==ArtifactKind.Directory)"Directory" else "${entry.size?.value ?: 0} bytes",style=MaterialTheme.typography.bodySmall)
      FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)){
       if(entry.kind==ArtifactKind.Directory)TextButton(onClick={model.openArtifactDirectory(shown,entry)},enabled=!shown.paused&&!shown.loading&&!shown.exporting){IconLabel(MaterialGlyph.Folder,"Open")}
       else{
        if((entry.size?.value ?: Long.MAX_VALUE)<=1024L*1024)TextButton(onClick={model.previewArtifact(shown,entry)},enabled=!shown.paused&&!shown.loading&&!shown.exporting){IconLabel(MaterialGlyph.Visibility,"Preview text")}
        TextButton(onClick={shown.exportConfirmation=entry},enabled=!shown.paused&&!shown.loading&&!shown.exporting&&(entry.size?.value ?: Long.MAX_VALUE)<=32L*1024*1024){IconLabel(MaterialGlyph.Save,"Export…")}
       }
      }
     }}
     if(shown.cursor!=null)item{TextButton(onClick={model.moreArtifacts(shown)},enabled=!shown.paused&&!shown.loading&&!shown.exporting){IconLabel(MaterialGlyph.ExpandMore,"More files")}}
    }
   }
  }
 }
 shown.exportConfirmation?.let{entry->AlertDialog(onDismissRequest={shown.exportConfirmation=null},title={UiText("Export this file?")},text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)){UiText(entry.name);UiText("${entry.size?.value ?: 0} bytes");UiText("The name and contents will be disclosed to the document provider you choose. No host file is opened until you return from that picker.")}},confirmButton={TextButton(onClick={shown.exportConfirmation=null;scope.launch{model.startArtifactExportPicker(shown,entry)?.let{ticket->pending=ticket;try{picker.launch(model.artifactExportName(entry.name))}catch(_:Exception){pending=null;model.cancelArtifactExportPicker(ticket)}}}}){IconLabel(MaterialGlyph.Save,"Choose destination")}},dismissButton={TextButton(onClick={shown.exportConfirmation=null}){UiText("Cancel")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))}
}
