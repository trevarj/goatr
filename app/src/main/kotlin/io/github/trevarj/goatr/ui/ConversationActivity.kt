// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.painterResource
import androidx.annotation.DrawableRes
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp

/** Only native-success tool-only records are routine; no inferred turn or semantic grouping.
 * OMP duplicates its exact tool output in text; distinct native text remains independently visible. */
internal fun isRoutineToolRecord(record: HistoryRecord, activeToolRecordId: Id?): Boolean =
    record.id!=activeToolRecordId && record.complete && !record.sourceTruncated &&
        record.role==Role.Tool && record.kind==RecordKind.Tool &&
        record.tool?.let {it.status==ToolStatus.Succeeded && (record.text==null || record.text==it.output) && it.input !is ContentUnavailable && it.output !is ContentUnavailable}==true &&
        record.original !is ContentUnavailable

@Composable
internal fun ToolActivitySummary(count: Int, expanded: Boolean, onToggle: ()->Unit) {
    if(count==0) return
    Surface(color=MaterialTheme.colorScheme.background,shape=MaterialTheme.shapes.medium,modifier=Modifier.fillMaxWidth()) {
        TextButton(onClick=onToggle,modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag("tool_activity_summary").semantics {stateDescription=if(expanded) "Routine activity expanded" else "Routine activity collapsed"},colors=ButtonDefaults.textButtonColors(contentColor=MaterialTheme.colorScheme.onSurfaceVariant)) {
            GoatrIcon(MaterialGlyph.ExpandMore,null,Modifier.rotate(if(expanded) 180f else 0f))
            Spacer(Modifier.width(8.dp))
            UiText("$count completed ${if(count==1) "tool" else "tools"} · ${if(expanded) "Hide routine activity" else "Show routine activity"}",style=MaterialTheme.typography.bodySmall,modifier=Modifier.weight(1f))
        }
    }
}

/** A single scrollable control rail reserves Stop outside its horizontal viewport. */
@Composable
internal fun ConversationControlRail(stopEnabled: Boolean, stopDisabledReason: String, onStop: ()->Unit, onMore: ()->Unit, controls: @Composable RowScope.()->Unit) {
    val composerColors=LocalComposerColors.current
    Surface(color=composerColors.surface,modifier=Modifier.fillMaxWidth().testTag("composer_overflow")) {
        Row(Modifier.fillMaxWidth().padding(horizontal=4.dp),verticalAlignment=Alignment.CenterVertically) {
            Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()),verticalAlignment=Alignment.CenterVertically,content=controls)
            TextButton(onClick=onMore,modifier=Modifier.heightIn(min=48.dp).testTag("conversation_more"),contentPadding=PaddingValues(horizontal=8.dp),colors=ButtonDefaults.textButtonColors(contentColor=composerColors.content)) {
                GoatrIcon(MaterialGlyph.Menu,null)
                Spacer(Modifier.width(4.dp))
                UiText("More",style=MaterialTheme.typography.labelMedium)
            }
            VerticalDivider(Modifier.height(24.dp))
            ConversationControl(MaterialGlyph.Stop,"Stop","turn_stop",stopEnabled,stopDisabledReason,onStop)
        }
    }
}

@Composable
internal fun ConversationControl(glyph: MaterialGlyph,label: String,tag: String,enabled: Boolean=true,disabledReason: String?=null,onClick: ()->Unit) {
    ConversationControl(glyph.drawable,label,tag,enabled,disabledReason,onClick)
}

@Composable
internal fun ConversationControl(@DrawableRes icon: Int,label: String,tag: String,enabled: Boolean=true,disabledReason: String?=null,onClick: ()->Unit) {
    IconButton(onClick=onClick,enabled=enabled,colors=IconButtonDefaults.iconButtonColors(contentColor=if(icon==MaterialGlyph.Stop.drawable) MaterialTheme.colorScheme.error else LocalComposerColors.current.content),modifier=Modifier.size(48.dp).testTag(tag).semantics {contentDescription=if(enabled || disabledReason==null) label else "$label. $disabledReason"}) {
        Icon(painterResource(icon),null,Modifier.size(20.dp))
    }
}
