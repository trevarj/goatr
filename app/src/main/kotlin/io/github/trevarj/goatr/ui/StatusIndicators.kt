// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.font.FontStyle

@Composable
internal fun StatusPill(label: String, emphasis: StatusEmphasis = StatusEmphasis.Neutral, modifier: Modifier = Modifier) {
    val statusColors=LocalStatusColors.current
    val colors = when (emphasis) {
        StatusEmphasis.Positive -> statusColors.positiveContainer to statusColors.positiveContent
        StatusEmphasis.Attention -> statusColors.attentionContainer to statusColors.attentionContent
        StatusEmphasis.Muted -> statusColors.mutedContainer to statusColors.mutedContent
        StatusEmphasis.Neutral -> statusColors.neutralContainer to statusColors.neutralContent
    }
    Surface(modifier = modifier.heightIn(min = 24.dp),color = colors.first,contentColor = colors.second,shape = MaterialTheme.shapes.small) {
        UiText(label,modifier = Modifier.padding(horizontal = 8.dp,vertical = 3.dp),style = MaterialTheme.typography.labelMedium,maxLines = 1,overflow = TextOverflow.Ellipsis)
    }
}

/** Provider identity is outlined; filled pills communicate state, never provider health. */
@Composable
internal fun ProviderPill(provider: Provider, modifier: Modifier = Modifier) {
    val ink=when(provider) {Provider.Omp->MaterialTheme.colorScheme.primary;Provider.Codex->MaterialTheme.colorScheme.tertiary}
    Surface(modifier=modifier.heightIn(min=24.dp),color=MaterialTheme.colorScheme.surface,contentColor=ink,border=BorderStroke(1.dp,ink.copy(alpha=0.5f)),shape=MaterialTheme.shapes.small) {
        UiText(providerLabel(provider),modifier=Modifier.padding(horizontal=8.dp,vertical=3.dp),style=MaterialTheme.typography.labelMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
    }
}

internal enum class StatusEmphasis { Positive, Attention, Neutral, Muted }

/** Keeps the refresh hint's footprint stable; progress exists only while refreshing. */
@Composable
internal fun RefreshHint(refreshing: Boolean) {
    val alpha by animateFloatAsState(if(refreshing) 1f else 0f,animationSpec=tween(durationMillis=180),label="refresh hint")
    Row(
        modifier=(if(refreshing) Modifier else Modifier.clearAndSetSemantics {}).alpha(alpha),
        verticalAlignment=Alignment.CenterVertically,
        horizontalArrangement=Arrangement.spacedBy(6.dp),
    ) {
        Box(Modifier.size(16.dp)) {
            if(refreshing) CircularProgressIndicator(
                modifier=Modifier.fillMaxSize(),
                color=MaterialTheme.colorScheme.onSurfaceVariant,
                strokeWidth=1.5.dp,
            )
        }
        UiText("Refreshing sessions",style=MaterialTheme.typography.bodySmall,fontStyle=FontStyle.Italic,color=MaterialTheme.colorScheme.onSurfaceVariant,maxLines=1,overflow=TextOverflow.Ellipsis)
    }
}

/** Keeps the composer gate slot stable while authoritative readiness changes. */
@Composable
internal fun ComposerGateNotice(message: String?, tag: String?) {
    Box(Modifier.fillMaxWidth().heightIn(min = 24.dp), contentAlignment = Alignment.CenterStart) {
        message?.let {
            UiText(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = if (tag == null) Modifier else Modifier.testTag(tag),
            )
        }
    }
}

