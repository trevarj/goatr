// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

/** This memory-only catalogue never opens native paths, remote icons or executable metadata. */
@Composable
internal fun SkillsPicker(snapshot:SkillsSnapshot,model:GoatrViewModel,connectionStatus:String,compactConnectionStatus:String) {
    var query by remember(snapshot.ticket) {mutableStateOf("")}
    DisposableEffect(snapshot.ticket) {onDispose {model.invalidateSkillsInvocation(snapshot.ticket)}}
    Dialog(onDismissRequest=model::dismissSkills,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
        Surface(Modifier.fillMaxWidth().fillMaxHeight(.9f).padding(16.dp),shape=MaterialTheme.shapes.large) {
            Column(Modifier.fillMaxSize().imePadding().padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                GoatrTitleBar("Skills",connectionStatus,snapshot.loading,compactStatus=compactConnectionStatus,actions={
                    IconButton(onClick=model::dismissSkills,modifier=Modifier.size(48.dp).testTag("skills_close")) {GoatrIcon(MaterialGlyph.Close,"Close Skills")}
                })
                val omp=snapshot.catalogue?.provider==Provider.Omp
                OutlinedTextField(query,{query=it.take(256)},label={UiText("Search skills")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("skills_search"))
                LazyColumn(Modifier.weight(1f).fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    item {Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        UiText(if(omp)"Enabled skill commands in this OMP session" else "Available skills for this thread directory",style=MaterialTheme.typography.bodySmall)
                        UiText("Insert adds draft text only. It never sends a message or invokes a skill.",style=MaterialTheme.typography.bodySmall)
                        if(!omp)UiText("Invoke requests this native skill without sending your draft. Acknowledgement is admission, not execution.",style=MaterialTheme.typography.bodySmall)
                    }}
                    when {
                        snapshot.loading -> item {UiText("Loading catalogue…")}
                        snapshot.error -> item {UiText("The catalogue is unavailable. Refresh reads the current original session again.")}
                        else -> {
                            val all=snapshot.catalogue?.skills.orEmpty()
                            val visible=all.filter {query.isBlank() || it.name.contains(query,true) || it.description.contains(query,true) || it.displayName?.contains(query,true)==true}
                            if(visible.isEmpty()) item {UiText(if(all.isEmpty()) "No skills are available in this native catalogue." else "No matching skills.")}
                            itemsIndexed(visible,key={index,entry->"${index}:${entry.path}:${entry.name}"}) {_,entry->
                                OutlinedCard(Modifier.fillMaxWidth()) {
                                    Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                                        UiText(entry.displayName?.takeIf {it.isNotBlank()}?:entry.name,style=MaterialTheme.typography.titleMedium)
                                        UiText(entry.shortDescription?.takeIf {it.isNotBlank()} ?: entry.description,style=MaterialTheme.typography.bodySmall,maxLines=2,overflow=androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                                        var details by remember(entry) {mutableStateOf(false)}
                                        DetailDisclosure(details,{details=!details},"skills_metadata_details")
                                        if(details) {
                                            UiText("${entry.name} · ${entry.scope}",style=MaterialTheme.typography.labelMedium)
                                            SelectionContainer {UiText(entry.description,style=MaterialTheme.typography.bodySmall)}
                                            entry.shortDescription?.takeIf {it!=entry.description}?.let {UiText(it,style=MaterialTheme.typography.bodySmall)}
                                            entry.path?.let {path->SelectionContainer {UiText(path,style=MaterialTheme.typography.bodySmall,fontFamily=androidx.compose.ui.text.font.FontFamily.Monospace)}}
                                        }
                                        val mention=entry.exactInsertText()
                                        if(!entry.enabled) UiText("Disabled by native settings",style=MaterialTheme.typography.bodySmall)
                                        else if(mention==null) UiText("This native name or path cannot be represented as an exact draft reference.",style=MaterialTheme.typography.bodySmall)
                                        if(entry.enabled&&entry.documentRef!=null)TextButton(onClick={model.previewSkill(snapshot,entry)},modifier=Modifier.heightIn(min=48.dp).testTag("skills_view_content")){IconLabel(MaterialGlyph.Visibility,"View complete content")}
                                        else UiText("Preview requires an exact enabled document under a locally granted Skill document root.",style=MaterialTheme.typography.bodySmall)
                                        TextButton(onClick={model.insertSkill(snapshot,entry)},enabled=mention!=null,modifier=Modifier.heightIn(min=48.dp).testTag("skills_insert")) {IconLabel(MaterialGlyph.Add,if(omp)"Insert command text" else "Insert")}
                                        if(!omp)TextButton(onClick={model.invokeSkill(snapshot,entry)},enabled=!model.busy&&model.canInvokeSkill(snapshot,entry),modifier=Modifier.heightIn(min=48.dp).testTag("skills_invoke")) {UiText("Invoke")}
                                    }
                                }
                            }
                            itemsIndexed(snapshot.catalogue?.errors.orEmpty()) {_,error->
                                OutlinedCard(Modifier.fillMaxWidth()) {Column(Modifier.padding(12.dp)) {UiText("Native loading error",style=MaterialTheme.typography.labelMedium);SelectionContainer {UiText("${error.path}\n${error.message}",style=MaterialTheme.typography.bodySmall)}}}
                            }
                        }
                    }
                    item {UiText("Document reading is independently granted on this desktop, not by model tool approval. Native OMP Skill invocation is unavailable; insertion never sends. Local snippets are separate.",style=MaterialTheme.typography.bodySmall)}
                }
                TextButton(onClick=model::refreshSkills,enabled=!snapshot.loading,modifier=Modifier.heightIn(min=48.dp).testTag("skills_refresh")) {IconLabel(MaterialGlyph.Refresh,"Refresh")}
            }
        }
    }
}
