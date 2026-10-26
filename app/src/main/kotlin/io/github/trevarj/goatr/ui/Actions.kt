// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics

@Composable
internal fun IconAction(glyph: MaterialGlyph, label: String, tag: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    Button(onClick = onClick, enabled = enabled, modifier = modifier.heightIn(min = 48.dp).testTag(tag)) {
        IconLabel(glyph, label)
    }
}

/** A stable, labeled control rail keeps essential controls above the IME. */
@Composable
internal fun ComposerControlAction(
    glyph: MaterialGlyph,
    label: String,
    tag: String,
    enabled: Boolean = true,
    disabledReason: String? = null,
    onClick: () -> Unit,
) {
    TextButton(
        onClick = onClick,
        enabled = enabled,
        contentPadding = PaddingValues(horizontal = 8.dp),
        colors = ButtonDefaults.textButtonColors(contentColor = if (glyph == MaterialGlyph.Stop) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant),
        modifier = Modifier.heightIn(min = 48.dp).widthIn(min = 48.dp).testTag(tag).semantics {
            contentDescription = if (enabled || disabledReason == null) label else "$label. $disabledReason"
        },
    ) {
        GoatrIcon(glyph, null)
        Spacer(Modifier.width(4.dp))
        UiText(label, style = MaterialTheme.typography.labelMedium, maxLines = 1)
    }
}

@Composable
internal fun PrimaryActionFab(
    glyph: MaterialGlyph,
    label: String,
    tag: String,
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    if (enabled) {
        ExtendedFloatingActionButton(
            onClick = onClick,
            modifier = modifier.sizeIn(minWidth = 112.dp, minHeight = 56.dp).testTag(tag),
            icon = { GoatrIcon(glyph, null) },
            text = { UiText(label) },
        )
    } else {
        Surface(
            modifier = modifier.sizeIn(minWidth = 112.dp, minHeight = 56.dp).testTag(tag).semantics(mergeDescendants = true) {
                role = Role.Button
                disabled()
                contentDescription = "$label unavailable"
            },
            shape = MaterialTheme.shapes.large,
            color = MaterialTheme.colorScheme.surfaceVariant,
            contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
        ) {
            Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GoatrIcon(glyph, "$label unavailable")
                UiText(label)
            }
        }
    }
}

/** The chat send action has a fixed footprint while the adjacent draft grows. */
@Composable
internal fun ComposerSendFab(enabled: Boolean, onClick: () -> Unit) {
    if (enabled) {
        FloatingActionButton(
            onClick = onClick,
            modifier = Modifier.size(56.dp).testTag("prompt_send"),
            containerColor = MaterialTheme.colorScheme.primary,
            contentColor = MaterialTheme.colorScheme.onPrimary,
        ) {
            GoatrIcon(MaterialGlyph.Send, "Send message")
        }
    } else {
        Surface(
            modifier = Modifier.size(56.dp).testTag("prompt_send").semantics(mergeDescendants = true) {
                role = Role.Button
                disabled()
                contentDescription = "Send message unavailable"
            },
            shape = MaterialTheme.shapes.large,
            color = MaterialTheme.colorScheme.surfaceVariant,
            contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
        ) {
            Box(contentAlignment = Alignment.Center) { GoatrIcon(MaterialGlyph.Send, null) }
        }
    }
}

/** Keeps existing conversation controls visible above the composer without changing their gates. */
@Composable
internal fun ComposerOverflow(
    browsingOlder: Boolean,
    refreshEnabled: Boolean,
    onSwitch: () -> Unit,
    onRefresh: () -> Unit,
    onBrowseOlder: (() -> Unit)?,
    browseOlderEnabled: Boolean,
    onLoadOlder: (() -> Unit)?,
    loadOlderEnabled: Boolean,
    onSnippets: () -> Unit,
    skillsEnabled: Boolean,
    onSkills: () -> Unit,
    terminalEnabled: Boolean,
    onTerminal: () -> Unit,
    worktreesEnabled: Boolean,
    onWorktrees: () -> Unit,
    changesEnabled: Boolean,
    onChanges: () -> Unit,
    agentAliasEnabled: Boolean,
    onAgentAlias: () -> Unit,
    workspaceRenameEnabled: Boolean,
    onWorkspaceRename: () -> Unit,
    stopEnabled: Boolean,
    stopDisabledReason: String,
    onStop: () -> Unit,
) {
    Surface(Modifier.fillMaxWidth().testTag("composer_overflow"), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .45f)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Row(
                Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(0.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ComposerControlAction(MaterialGlyph.Chat, "Switch", "switcher", onClick = onSwitch)
                ComposerControlAction(MaterialGlyph.Add, "Snippets", "composer_snippets", onClick = onSnippets)
                ComposerControlAction(MaterialGlyph.Menu, "Skills", "composer_skills", skillsEnabled, "Skills are not available for this session.", onSkills)
                ComposerControlAction(MaterialGlyph.Terminal, "Terminal", "terminal_open", terminalEnabled, "Terminal text is unavailable for this current native session.", onTerminal)
                ComposerControlAction(MaterialGlyph.Desktop, "Worktrees", "worktrees_open", worktreesEnabled, PROJECT_GIT_READ_REQUIREMENT, onWorktrees)
                ComposerControlAction(MaterialGlyph.Terminal, "Changes", "changes_open", changesEnabled, PROJECT_GIT_READ_REQUIREMENT, onChanges)
                ComposerControlAction(MaterialGlyph.Edit, "Agent alias", "agent_alias_open", agentAliasEnabled, "Agent alias is unavailable for this original native session.", onAgentAlias)
                ComposerControlAction(MaterialGlyph.Edit, "Rename", "workspace_rename_open", workspaceRenameEnabled, "Workspace rename is unavailable for this current native session.", onWorkspaceRename)
                ComposerControlAction(
                    MaterialGlyph.Refresh,
                    if (browsingOlder) "Latest" else "Refresh",
                    "conversation_refresh",
                    enabled = refreshEnabled,
                    disabledReason = "Finish connecting or the current operation before refreshing",
                    onClick = onRefresh,
                )
                onBrowseOlder?.let { browse ->
                    ComposerControlAction(MaterialGlyph.Back, "History", "history_browse", browseOlderEnabled, "History is unavailable", browse)
                }
                onLoadOlder?.let { load ->
                    ComposerControlAction(MaterialGlyph.Back, "Older", "history_older", loadOlderEnabled, "Older history is unavailable", load)
                }
            }
            VerticalDivider(Modifier.height(24.dp))
            ComposerControlAction(MaterialGlyph.Stop, "Stop", "turn_stop", stopEnabled, stopDisabledReason, onStop)
        }
    }
}

