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
internal fun SkillsPicker(snapshot:SkillsSnapshot,model:GoatrViewModel) {
    var query by remember(snapshot.ticket) {mutableStateOf("")}
    Dialog(onDismissRequest=model::dismissSkills,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
        Surface(Modifier.fillMaxWidth().fillMaxHeight(.9f).padding(12.dp),shape=MaterialTheme.shapes.large) {
            Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                Row(Modifier.fillMaxWidth()) {
                    UiText("Skills",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f))
                    TextButton(onClick=model::dismissSkills,modifier=Modifier.testTag("skills_close")) {IconLabel(MaterialGlyph.Close,"Close")}
                }
                val omp=snapshot.catalogue?.provider==Provider.Omp
                UiText(if(omp)"Enabled skill commands in this OMP session" else "Available skills for this thread directory",style=MaterialTheme.typography.bodySmall)
                UiText(if(omp)"Insert adds command text to your draft. It does not expand or invoke the skill, or send a message." else "Insert adds an exact mention to your draft. It does not send a message.",style=MaterialTheme.typography.bodySmall)
                if(snapshot.catalogue!=null) UiText("Loaded here; reused only for this conversation's inline suggestions.",style=MaterialTheme.typography.bodySmall)
                OutlinedTextField(query,{query=it.take(256)},label={UiText("Search skills")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("skills_search"))
                LazyColumn(Modifier.weight(1f).fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    when {
                        snapshot.loading -> item {UiText("Loading catalogue…")}
                        snapshot.error -> item {UiText("The catalogue is unavailable. Refresh reads the current original session again.")}
                        else -> {
                            val all=snapshot.catalogue?.skills.orEmpty()
                            val visible=all.filter {query.isBlank() || it.name.contains(query,true) || it.description.contains(query,true) || it.displayName?.contains(query,true)==true}
                            if(visible.isEmpty()) item {UiText(if(all.isEmpty()) "No skills are available in this native catalogue." else "No matching skills.")}
                            itemsIndexed(visible,key={index,entry->"${index}:${entry.path}:${entry.name}"}) {_,entry->
                                OutlinedCard(Modifier.fillMaxWidth()) {
                                    Column(Modifier.padding(12.dp),verticalArrangement=Arrangement.spacedBy(6.dp)) {
                                        UiText(entry.displayName?.takeIf {it.isNotBlank()}?:entry.name,style=MaterialTheme.typography.titleSmall)
                                        UiText("${entry.name} · ${entry.scope}" ,style=MaterialTheme.typography.labelSmall)
                                        SelectionContainer {UiText(entry.description,style=MaterialTheme.typography.bodySmall)}
                                        entry.shortDescription?.takeIf {it!=entry.description}?.let {UiText(it,style=MaterialTheme.typography.bodySmall)}
                                        entry.path?.let {path->SelectionContainer {UiText(path,style=MaterialTheme.typography.labelSmall)}}
                                        val mention=entry.exactInsertText()
                                        if(!entry.enabled) UiText("Disabled by native settings",style=MaterialTheme.typography.bodySmall)
                                        else if(mention==null) UiText("This native name or path cannot be represented as an exact draft reference.",style=MaterialTheme.typography.bodySmall)
                                        if(omp){
                                            if(entry.documentRef!=null)TextButton(onClick={model.previewSkill(snapshot,entry)},modifier=Modifier.testTag("skills_view_content")){IconLabel(MaterialGlyph.Visibility,"View content")}
                                            else UiText("Document preview is unavailable for this native resource.",style=MaterialTheme.typography.bodySmall)
                                        }
                                        TextButton(onClick={model.insertSkill(snapshot,entry)},enabled=mention!=null,modifier=Modifier.testTag("skills_insert")) {IconLabel(MaterialGlyph.Add,if(omp)"Insert command text" else "Insert")}
                                    }
                                }
                            }
                            itemsIndexed(snapshot.catalogue?.errors.orEmpty()) {_,error->
                                OutlinedCard(Modifier.fillMaxWidth()) {Column(Modifier.padding(12.dp)) {UiText("Native loading error",style=MaterialTheme.typography.labelMedium);SelectionContainer {UiText("${error.path}\n${error.message}",style=MaterialTheme.typography.bodySmall)}}}
                            }
                        }
                    }
                    item {UiText(if(omp)"User-authorized read-only preview. Native skill invocation is unavailable; local snippets are separate." else "Permission-preserving Codex document preview is unavailable. Local snippets are separate.",style=MaterialTheme.typography.bodySmall)}
                }
                TextButton(onClick=model::refreshSkills,enabled=!snapshot.loading,modifier=Modifier.testTag("skills_refresh")) {IconLabel(MaterialGlyph.Refresh,"Refresh")}
            }
        }
    }
}
