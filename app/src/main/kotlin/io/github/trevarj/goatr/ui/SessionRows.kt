// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow

/** Material3 setting row with a full-height label and a single local presentation toggle. */
@Composable
internal fun PresentationSwitchRow(
    title: String,
    detail: String,
    checked: Boolean,
    tag: String,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            UiText(title, style = MaterialTheme.typography.titleSmall)
            UiText(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

@Composable
internal fun CompactRow(
    title: String,
    subtitle: String,
    status: @Composable (() -> Unit)? = null,
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
) {
    val body: @Composable () -> Unit = {
        BoxWithConstraints(Modifier.fillMaxWidth().padding(12.dp)) {
            val stacked = maxWidth < 320.dp || LocalDensity.current.fontScale >= 1.3f
            val labels: @Composable () -> Unit = {
                UiText(title, style = MaterialTheme.typography.titleMedium, minLines = 1, maxLines = 1, overflow = TextOverflow.Ellipsis)
                UiText(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, minLines = 1, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (stacked) {
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    labels()
                    status?.invoke()
                }
            } else Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) { labels() }
                status?.invoke()
            }
        }
    }
    if (onClick == null) Surface(modifier = modifier.fillMaxWidth(), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surface) { body() }
    else OutlinedCard(onClick = onClick, modifier = modifier.fillMaxWidth()) { body() }
}

