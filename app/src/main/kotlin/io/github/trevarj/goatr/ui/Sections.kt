// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.Alignment
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow

/** Compact page chrome only; callers retain navigation, loading and original-host authority. */
@Composable
internal fun GoatrTitleBar(
    title:String,
    status:String,
    loading:Boolean,
    compactStatus:String,
    modifier:Modifier=Modifier,
    titleModifier:Modifier=Modifier,
    navigationIcon:@Composable ()->Unit={},
    actions:@Composable RowScope.()->Unit={},
) {
    Box(modifier.fillMaxWidth().height(48.dp)) {
        Row(Modifier.fillMaxSize().padding(horizontal=4.dp),horizontalArrangement=Arrangement.spacedBy(4.dp),verticalAlignment=Alignment.CenterVertically) {
            navigationIcon()
            Icon(painterResource(R.drawable.ic_notification_goat),"Goatr",modifier=Modifier.size(20.dp),tint=MaterialTheme.colorScheme.onSurface)
            UiText(title,style=MaterialTheme.typography.titleMedium,maxLines=1,overflow=TextOverflow.Ellipsis,modifier=titleModifier.weight(1f).semantics {heading()})
            Row(Modifier.width(100.dp),horizontalArrangement=Arrangement.spacedBy(4.dp),verticalAlignment=Alignment.CenterVertically) {
                UiText(compactStatus,style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant,maxLines=1,overflow=TextOverflow.Ellipsis,modifier=Modifier.weight(1f).semantics {contentDescription=status})
                Box(Modifier.size(16.dp)) {if(loading) CircularProgressIndicator(modifier=Modifier.fillMaxSize(),strokeWidth=2.dp)}
            }
            actions()
        }
    }
}

/** Shared presentation vocabulary for the Sessions, Hosts, and Settings flows. */
@Composable
internal fun GoatrSection(title: String, detail: String? = null, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().padding(vertical=4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        UiText(title, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.semantics { heading() })
        detail?.let { UiText(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}

@Composable
internal fun GroupedSurface(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surface,
        border = BorderStroke(1.dp,MaterialTheme.colorScheme.outline.copy(alpha=0.25f)),
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp), content = content)
    }
}

@Composable
internal fun EmptyState(title: String, body: String, modifier: Modifier = Modifier, action: (@Composable () -> Unit)? = null) {
    GroupedSurface(modifier) {
        UiText(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
        UiText(body, color = MaterialTheme.colorScheme.onSurfaceVariant)
        action?.invoke()
    }
}

@Composable
internal fun DetailDisclosure(expanded: Boolean, onToggle: () -> Unit, tag: String = "details") {
    TextButton(onClick = onToggle, modifier = Modifier.heightIn(min = 48.dp).testTag(tag)) {
        IconLabel(MaterialGlyph.ExpandMore, if (expanded) "Hide details" else "Details")
    }
}

/** Both Sessions routes keep transient feedback in their existing refresh-line footprint. */
@Composable
internal fun SessionsHostCard(
    hostTitle:String?,
    transport:String,
    online:Boolean,
    refreshing:Boolean,
    failureText:String,
    modifier:Modifier=Modifier,
    actions:@Composable RowScope.()->Unit,
) {
    GroupedSurface(modifier) {
        if(hostTitle!=null) Column(Modifier.fillMaxWidth().padding(vertical=4.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            UiText(hostTitle,style=MaterialTheme.typography.titleMedium,minLines=1,maxLines=1,overflow=TextOverflow.Ellipsis,modifier=Modifier.semantics {heading()})
            UiText(transport,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant,minLines=1,maxLines=1,overflow=TextOverflow.Ellipsis)
        } else StatusPill(transport,if(online)StatusEmphasis.Positive else StatusEmphasis.Attention)
        Box(Modifier.fillMaxWidth()) {
            if(failureText.isNotEmpty()) UiText(failureText,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error,minLines=1,maxLines=1,overflow=TextOverflow.Ellipsis)
            else RefreshHint(refreshing)
        }
        Row(horizontalArrangement=Arrangement.spacedBy(8.dp),content=actions)
    }
}
