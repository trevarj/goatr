// Material 3, in Herdr's colors: a palette resolved by herdr.core (graphite,
// ink, moss, paper) fills every role of the color scheme, so stock components
// look like the desktop. "material" uses the wallpaper's colors instead
// (Android 12+). Herdr's own roles (attention, ansi) sit beside the scheme in
// [HerdrColors], and the terminal always draws with a Herdr palette.

package io.github.trevarj.goatr.app.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import io.github.trevarj.goatr.app.R
import herdr.core.DEFAULT_DARK
import herdr.core.DEFAULT_LIGHT
import herdr.core.PRESETS
import herdr.core.hex
import herdr.core.isLight
import herdr.core.resolve

/** The theme pref that follows the wallpaper (Material You). */
const val MATERIAL = "material"

/** Whether this phone can follow the wallpaper. */
val hasDynamicColor = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

/** A resolved Herdr palette: the desktop's colors that the app draws with. */
class HerdrColors(val name: String) {
    private val theme = resolve(PRESETS.getValue(name))
    private fun color(key: String) = Color(theme[key])

    val light = isLight(PRESETS.getValue(name).background)
    val background = color("background")
    val terminalForeground = color("terminal_foreground")
    val canvas = color("canvas")
    val chrome = color("chrome")
    val popover = color("popover")
    val raised = color("raised")
    val selected = color("selected")
    val border = color("border")
    val foreground = color("foreground")
    val muted = color("muted")
    val accent = color("accent")
    val accentForeground = color("accent_foreground")
    val solid = color("solid")
    val solidForeground = color("solid_foreground")
    val selection = color("selection")
    val cursor = color("cursor")
    val working = color("working")
    val blocked = color("blocked")
    val done = color("done")
    val idle = color("idle")
    val ansi = List(16) { color("ansi$it") }
    val connected = ansi[2]
}

/** `#rrggbb[aa]`, the spelling the terminal view takes. */
fun Color.hex(): String = hex(toArgb().toLong() and 0xFFFFFFFFL)

/**
 * Herdr's colors as Material roles. Primary is the accent, as the desktop maps
 * it for its own controls (herdr-app `mods.rs` `apply_theme`): switches, radio
 * buttons, buttons and text fields take it.
 */
fun HerdrColors.scheme(): ColorScheme {
    val base = if (light) lightColorScheme() else darkColorScheme()
    return base.copy(
        primary = accent,
        onPrimary = accentForeground,
        primaryContainer = selected,
        onPrimaryContainer = foreground,
        secondary = accent,
        onSecondary = accentForeground,
        secondaryContainer = selected,
        onSecondaryContainer = foreground,
        tertiary = done,
        background = chrome,
        onBackground = foreground,
        surface = chrome,
        onSurface = foreground,
        surfaceVariant = raised,
        onSurfaceVariant = muted,
        surfaceTint = Color.Transparent,
        surfaceDim = canvas,
        surfaceBright = raised,
        surfaceContainerLowest = background,
        surfaceContainerLow = chrome,
        surfaceContainer = chrome,
        surfaceContainerHigh = popover,
        surfaceContainerHighest = raised,
        inverseSurface = foreground,
        inverseOnSurface = background,
        outline = border,
        outlineVariant = border,
        error = blocked,
        onError = background,
        scrim = Color.Black,
    )
}

private val geist = FontFamily(
    listOf(400, 500, 600, 700).map {
        Font(R.font.geist, FontWeight(it), variationSettings = FontVariation.Settings(FontVariation.weight(it)))
    },
)

val GeistMono = FontFamily(
    listOf(400, 500, 700).map {
        Font(R.font.geist_mono, FontWeight(it), variationSettings = FontVariation.Settings(FontVariation.weight(it)))
    },
)

val JetBrainsMono = FontFamily(Font(R.font.jetbrains_mono_regular), Font(R.font.jetbrains_mono_bold, FontWeight.Bold))

private val typography = Typography().run {
    copy(
        displayLarge = displayLarge.copy(fontFamily = geist),
        displayMedium = displayMedium.copy(fontFamily = geist),
        displaySmall = displaySmall.copy(fontFamily = geist),
        headlineLarge = headlineLarge.copy(fontFamily = geist),
        headlineMedium = headlineMedium.copy(fontFamily = geist),
        headlineSmall = headlineSmall.copy(fontFamily = geist),
        titleLarge = titleLarge.copy(fontFamily = geist),
        titleMedium = titleMedium.copy(fontFamily = geist),
        titleSmall = titleSmall.copy(fontFamily = geist),
        bodyLarge = bodyLarge.copy(fontFamily = geist),
        bodyMedium = bodyMedium.copy(fontFamily = geist),
        bodySmall = bodySmall.copy(fontFamily = geist),
        labelLarge = labelLarge.copy(fontFamily = geist),
        labelMedium = labelMedium.copy(fontFamily = geist),
        labelSmall = labelSmall.copy(fontFamily = geist),
    )
}

private val LocalHerdr = staticCompositionLocalOf { HerdrColors(DEFAULT_DARK) }

/** Herdr's palette in use (the terminal's colors, attention colors). */
val herdr: HerdrColors
    @Composable @ReadOnlyComposable get() = LocalHerdr.current

/** The Herdr palette for a theme pref: a preset, or graphite / paper following the system. */
private fun paletteName(theme: String, dark: Boolean): String =
    if (theme in PRESETS) theme else if (dark) DEFAULT_DARK else DEFAULT_LIGHT

@Composable
fun HerdrTheme(theme: String, content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val name = paletteName(theme, dark)
    val colors = remember(name) { HerdrColors(name) }
    val context = LocalContext.current
    val scheme = if (theme == MATERIAL && hasDynamicColor) {
        if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
    } else colors.scheme()
    CompositionLocalProvider(LocalHerdr provides colors) {
        MaterialTheme(colorScheme = scheme, typography = typography, content = content)
    }
}
