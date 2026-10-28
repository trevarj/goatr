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
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.Alignment
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.rememberTextMeasurer

private val HeaderStatusMeasureText=AnnotatedString("Ag\nAg")

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
    val statusStyle=MaterialTheme.typography.labelSmall
    val density=LocalDensity.current
    // The native paragraph engine owns nonlinear font scaling and first/last-line metrics.
    // Its one-entry cache follows the same current font resolver/density/layout direction.
    val measurer=rememberTextMeasurer(cacheSize=1)
    val statusHeight=measurer.measure(HeaderStatusMeasureText,style=statusStyle,maxLines=2,density=density).size.height
    val minimumHeight=with(density){statusHeight.toDp()}.coerceAtLeast(48.dp)
    Row(
        modifier.fillMaxWidth().heightIn(min=minimumHeight).padding(horizontal=8.dp,vertical=4.dp),
        verticalAlignment=Alignment.CenterVertically,
    ) {
        navigationIcon()
        Column(Modifier.weight(1f).padding(end=4.dp),verticalArrangement=Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment=Alignment.CenterVertically) {
                Icon(
                    painterResource(R.drawable.ic_goatr_title),
                    contentDescription=null,
                    modifier=Modifier.size(24.dp).testTag("titlebar_goat"),
                    tint=MaterialTheme.colorScheme.onSurface,
                )
                Spacer(Modifier.width(8.dp))
                UiText(
                    title.ifEmpty {"Goatr"},
                    style=if(title.isEmpty()) MaterialTheme.typography.titleSmall else MaterialTheme.typography.titleMedium,
                    maxLines=1,
                    overflow=TextOverflow.Ellipsis,
                    modifier=titleModifier.weight(1f).semantics {heading()},
                )
            }
            UiText(
                compactStatus,
                style=statusStyle,
                color=MaterialTheme.colorScheme.onSurfaceVariant,
                minLines=2,
                maxLines=2,
                overflow=TextOverflow.Ellipsis,
                modifier=Modifier.padding(start=32.dp).fillMaxWidth().semantics {contentDescription=status},
            )
        }
        // Decorative progress never repeats the full accessible status or moves the trailing actions.
        Box(Modifier.size(24.dp).clearAndSetSemantics {},contentAlignment=Alignment.Center) {
            if(loading) CircularProgressIndicator(modifier=Modifier.size(14.dp),strokeWidth=1.5.dp)
        }
        Row(Modifier.heightIn(min=48.dp),verticalAlignment=Alignment.CenterVertically,content=actions)
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
    Column(modifier.fillMaxWidth().padding(vertical=4.dp),verticalArrangement=Arrangement.spacedBy(2.dp)) {
        Row(Modifier.fillMaxWidth().heightIn(min=48.dp),verticalAlignment=Alignment.CenterVertically) {
            Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(2.dp)) {
                hostTitle?.let {UiText(it,style=MaterialTheme.typography.titleSmall,minLines=1,maxLines=1,overflow=TextOverflow.Ellipsis,modifier=Modifier.semantics {heading()})}
                UiText(transport,style=MaterialTheme.typography.bodySmall,color=if(online) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.tertiary,minLines=1,maxLines=1,overflow=TextOverflow.Ellipsis)
            }
            Row(horizontalArrangement=Arrangement.spacedBy(0.dp),content=actions)
        }
        Box(Modifier.fillMaxWidth()) {
            if(failureText.isNotEmpty()) UiText(failureText,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error,minLines=1,maxLines=1,overflow=TextOverflow.Ellipsis)
            else RefreshHint(refreshing)
        }
    }
}
