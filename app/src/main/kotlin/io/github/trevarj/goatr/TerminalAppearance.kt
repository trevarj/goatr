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
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
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

internal const val TERMINAL_DISPLAY_SCALE_DEFAULT=1f
internal const val TERMINAL_DISPLAY_SCALE_MIN=.5f
internal const val TERMINAL_DISPLAY_SCALE_MAX=4f

/** A bounded local scale never changes a grid frame or native terminal geometry. */
internal fun terminalDisplayScale(value:Float?):Float =
    value?.takeIf {it.isFinite()}?.coerceIn(TERMINAL_DISPLAY_SCALE_MIN,TERMINAL_DISPLAY_SCALE_MAX)
        ?: TERMINAL_DISPLAY_SCALE_DEFAULT

internal fun terminalDisplayScaleFromStored(value:Any?):Float = terminalDisplayScale(value as? Float)

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

internal class TerminalDisplayScaleState internal constructor(
    initial:Float,
    private val save:(Float)->Unit,
) {
    var value by mutableFloatStateOf(terminalDisplayScale(initial))
        private set
    private var saved=value
    var resetEpoch by mutableIntStateOf(0)
        private set

    fun set(next:Float) {
        val normalized=terminalDisplayScale(next)
        if(value!=normalized)value=normalized
    }
    fun persist() {
        if(saved==value)return
        save(value)
        saved=value
    }
    fun reset() {
        value=TERMINAL_DISPLAY_SCALE_DEFAULT
        resetEpoch++
        persist()
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
internal fun rememberTerminalDisplayScaleState(): TerminalDisplayScaleState {
    val applicationContext=LocalContext.current.applicationContext
    val preferences=remember(applicationContext) {TerminalAppearancePreferences(applicationContext)}
    return remember(preferences) {TerminalDisplayScaleState(preferences.loadScale(),preferences::saveScale)}
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

/** Existing local pinch and accessibility scale, with an explicit local reset. */
@Composable
internal fun TerminalDisplayScaleControl(state:TerminalDisplayScaleState) {
    val percent=(state.value*100).toInt()
    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
        Text("Display scale · $percent%",style=MaterialTheme.typography.bodySmall,modifier=Modifier.padding(vertical=12.dp).testTag("terminal_display_scale"))
        TextButton(onClick=state::reset,modifier=Modifier.heightIn(min=48.dp).testTag("terminal_display_scale_reset")) {
            IconLabel(MaterialGlyph.Refresh,"Reset local scale")
        }
    }
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

    fun loadScale():Float = terminalDisplayScaleFromStored(preferences.all[SCALE_KEY])

    fun saveScale(scale:Float) {
        preferences.edit().putFloat(SCALE_KEY,terminalDisplayScale(scale)).apply()
    }

    private companion object {
        const val PREFERENCES = "local_presentation"
        const val KEY = "terminal_appearance"
        const val SCALE_KEY = "terminal_display_scale"
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
