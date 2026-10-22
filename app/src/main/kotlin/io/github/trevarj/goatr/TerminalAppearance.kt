// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/** Presentation-only appearance for the bounded, secure terminal snapshot. */
internal enum class TerminalAppearance { FollowApp, Light, Dark }

internal class TerminalAppearanceState internal constructor(
    initial: TerminalAppearance,
    private val preferences: TerminalAppearancePreferences,
) {
    var value by mutableStateOf(initial)
        private set

    fun select(next: TerminalAppearance) {
        if (value == next) return
        value = next
        preferences.save(next)
    }
}

/** Uses application storage only; terminal content and any native identity stay out of preferences. */
@Composable
internal fun rememberTerminalAppearanceState(): TerminalAppearanceState {
    val applicationContext = LocalContext.current.applicationContext
    val preferences = remember(applicationContext) { TerminalAppearancePreferences(applicationContext) }
    return remember(preferences) { TerminalAppearanceState(preferences.load(), preferences) }
}

@Composable
internal fun TerminalAppearancePicker(state: TerminalAppearanceState) {
    var expanded by remember {mutableStateOf(false)}
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(onClick={expanded=!expanded},modifier=Modifier.heightIn(min=48.dp).testTag("terminal_appearance_details")) {
            IconLabel(MaterialGlyph.ExpandMore,"Snapshot appearance · ${state.value.label}")
        }
        if(expanded) {
            Text("Applies only to the secure terminal snapshot.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TerminalAppearance.entries.forEach { appearance ->
                FilterChip(
                    selected = state.value == appearance,
                    onClick = { state.select(appearance) },
                    label = { Text(appearance.label) },
                    modifier = Modifier.heightIn(min = 48.dp).testTag(appearance.testTag),
                )
            }
        }
        }
    }
}

/** Styles the existing secure, bounded reader without changing its load or lifecycle behavior. */
@Composable
internal fun TerminalSnapshotText(text: String, appearance: TerminalAppearance) {
    val palette = terminalSnapshotPalette(appearance)
    Surface(
        modifier = Modifier.fillMaxWidth().testTag("shell_visible_reader"),
        color = palette.background,
        contentColor = palette.foreground,
        shape = MaterialTheme.shapes.small,
    ) {
        Text(
            text = text,
            fontFamily = FontFamily.Monospace,
            style = MaterialTheme.typography.bodyMedium,
            softWrap = true,
            modifier = Modifier.fillMaxWidth().padding(16.dp).testTag("shell_visible_text"),
        )
    }
}

private data class TerminalSnapshotPalette(val background: Color, val foreground: Color)

@Composable
private fun terminalSnapshotPalette(appearance: TerminalAppearance): TerminalSnapshotPalette = when (appearance) {
    TerminalAppearance.FollowApp -> TerminalSnapshotPalette(
        MaterialTheme.colorScheme.surfaceVariant,
        MaterialTheme.colorScheme.onSurfaceVariant,
    )
    TerminalAppearance.Light -> TerminalSnapshotPalette(Color(0xFFF7F7F2), Color(0xFF1B1C19))
    TerminalAppearance.Dark -> TerminalSnapshotPalette(Color(0xFF111513), Color(0xFFE3E5DE))
}

internal class TerminalAppearancePreferences(applicationContext: Context) {
    private val preferences = applicationContext.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    fun load(): TerminalAppearance {
        val stored = try { preferences.getString(KEY, null) } catch (_: ClassCastException) { null }
        return TerminalAppearance.entries.firstOrNull { it.name == stored } ?: TerminalAppearance.FollowApp
    }

    fun save(appearance: TerminalAppearance) {
        preferences.edit().putString(KEY, appearance.name).apply()
    }

    private companion object {
        const val PREFERENCES = "local_presentation"
        const val KEY = "terminal_appearance"
    }
}

private val TerminalAppearance.label: String
    get() = when (this) {
        TerminalAppearance.FollowApp -> "Follow app"
        TerminalAppearance.Light -> "Light"
        TerminalAppearance.Dark -> "Dark"
    }

private val TerminalAppearance.testTag: String
    get() = when (this) {
        TerminalAppearance.FollowApp -> "terminal_appearance_follow_app"
        TerminalAppearance.Light -> "terminal_appearance_light"
        TerminalAppearance.Dark -> "terminal_appearance_dark"
    }
