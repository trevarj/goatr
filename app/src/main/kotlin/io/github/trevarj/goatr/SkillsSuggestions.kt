// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

internal data class SkillsSuggestionsState(val key:ConversationKey,val source:Uuid,val generation:Uuid,val provider:Provider,val ticket:Long,val draft:String,val trigger:String,val catalogue:SkillsSnapshot?,val entries:List<SkillEntry>)

/** Standalone trailing provider token only; no cursor or replacement range is inferred. */
internal fun skillSuggestionTrigger(provider:Provider,draft:String):String? {
    val prefix=when(provider) {Provider.Codex->"$";Provider.Omp->"/skill:"}
    val maximum=when(provider) {Provider.Codex->65;Provider.Omp->135}
    val tail=draft.takeLast(maximum+1)
    val boundary=tail.indexOfLast {it.isWhitespace()}
    if(boundary<0 && tail.length<draft.length) return null
    val token=tail.substring(boundary+1)
    if(token.length !in prefix.length..maximum || !token.startsWith(prefix)) return null
    val punctuation=when(provider) {Provider.Codex->"_:-";Provider.Omp->"._-"}
    if(token.drop(prefix.length).any {it !in 'a'..'z' && it !in 'A'..'Z' && it !in '0'..'9' && it !in punctuation}) return null
    return token
}

/** Metadata-only suggestions from an explicitly loaded, still-admitted original catalogue. */
@Composable
internal fun SkillsSuggestions(state:SkillsSuggestionsState,model:GoatrViewModel,modifier:Modifier=Modifier) {
    Surface(modifier.fillMaxWidth().testTag("skills_suggestions"),shape=MaterialTheme.shapes.medium,color=MaterialTheme.colorScheme.surfaceVariant) {
        Column(Modifier.padding(horizontal=12.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(6.dp),verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                GoatrIcon(MaterialGlyph.Menu,null)
                UiText(if(state.catalogue==null) "Skills · load once" else "Skills · draft only",style=MaterialTheme.typography.labelMedium,maxLines=1,overflow=TextOverflow.Ellipsis,modifier=Modifier.weight(1f).semantics {contentDescription=if(state.catalogue==null) "Load the Skills catalogue once; no reads while typing." else if(state.provider==Provider.Omp) "Append command text; does not invoke a skill." else "Append mention; does not send a message."})
            }
            LazyRow(Modifier.fillMaxWidth().height(48.dp),horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                if(state.catalogue==null) {
                    item {TextButton(onClick={model.openSuggestedSkills(state)},modifier=Modifier.height(48.dp).testTag("skills_suggestions_load")) {IconLabel(MaterialGlyph.Refresh,"Load Skills")}}
                } else {
                    itemsIndexed(state.entries,key={index,entry->"${index}:${entry.path}:${entry.name}"}) {index,entry->
                        val mention=entry.exactInsertText()
                        AssistChip(onClick={model.appendSuggestedSkill(state,entry)},enabled=mention!=null,leadingIcon={GoatrIcon(MaterialGlyph.Add,null)},label={
                            UiText((if(state.provider==Provider.Omp) entry.commandName?.let {"/$it"} ?: entry.name else entry.name)+if(!entry.enabled) " · disabled" else if(mention==null) " · unavailable" else "",maxLines=1,overflow=TextOverflow.Ellipsis,modifier=Modifier.widthIn(max=196.dp))
                        },modifier=Modifier.height(48.dp).testTag("skills_suggestion_$index"))
                    }
                    if(state.entries.isEmpty()) item {UiText(if(state.provider==Provider.Omp) "No matching enabled commands" else "No matching skills",style=MaterialTheme.typography.bodySmall,modifier=Modifier.padding(vertical=12.dp))}
                    item {TextButton(onClick={model.openSuggestedSkills(state)},modifier=Modifier.height(48.dp).testTag("skills_suggestions_catalogue")) {IconLabel(MaterialGlyph.Menu,if(state.provider==Provider.Omp) "Catalogue" else "Catalogue metadata")}}
                }
            }
        }
    }
}
