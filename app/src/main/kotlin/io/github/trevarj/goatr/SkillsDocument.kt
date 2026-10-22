// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

/** Full verified text only, memory-only and secure; no WebView or clipboard. */
@Composable
internal fun SkillsDocument(shown:SkillsDocumentState,model:GoatrViewModel,connectionStatus:String,compactConnectionStatus:String){
    Dialog(onDismissRequest=model::closeSkillsDocument,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)){
        Surface(Modifier.fillMaxWidth().fillMaxHeight(.94f).padding(16.dp),shape=MaterialTheme.shapes.large){
            Column(Modifier.fillMaxSize().padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)){
                GoatrTitleBar(shown.entry.displayName ?: shown.entry.name,connectionStatus,shown.loading,compactStatus=compactConnectionStatus,actions={
                    IconButton(onClick=model::closeSkillsDocument,modifier=Modifier.size(48.dp).testTag("skills_document_close")){GoatrIcon(MaterialGlyph.Close,"Close document")}
                })
                UiText("Original OMP skill document · read only",style=MaterialTheme.typography.labelMedium)
                Box(Modifier.weight(1f).fillMaxWidth()) {
                when{
                    shown.loading->UiText("Reading and verifying the complete document…",style=MaterialTheme.typography.bodySmall)
                    shown.failed->UiText("The complete document is unavailable, changed, expired or exceeded 1MiB. Close and refresh this original catalogue to inspect it again.",style=MaterialTheme.typography.bodyMedium)
                    shown.text!=null->{
                        val chunks=remember(shown.ticket,shown.text){
                            val text=shown.text;val parts=ArrayList<String>();var start=0
                            while(start<text.length){val newline=text.indexOf('\n',minOf(text.length,start+2048));val end=if(newline<0)text.length else newline+1;parts.add(text.substring(start,end));start=end};parts
                        }
                        LazyColumn(Modifier.fillMaxSize().testTag("skills_document_text"),verticalArrangement=Arrangement.spacedBy(0.dp)){
                            itemsIndexed(chunks){index,text->
                                val display=if(index<chunks.lastIndex) text.removeSuffix("\n").removeSuffix("\r") else text
                                UiText(display,style=MaterialTheme.typography.bodyMedium,fontFamily=FontFamily.Monospace,modifier=Modifier.fillMaxWidth())
                            }
                            if(chunks.isEmpty())item{UiText("Empty document",style=MaterialTheme.typography.bodySmall)}
                        }
                    }
                }
                }
            }
        }
    }
}
