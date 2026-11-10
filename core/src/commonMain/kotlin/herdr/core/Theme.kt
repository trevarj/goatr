// Herdr's themes: a palette plus optional color overrides, resolved into every
// named color the UI uses. A port of herdr-app/src/settings.rs (Palette,
// presets, resolve) so the phone draws exactly the desktop's colors.

package herdr.core

import kotlinx.serialization.Serializable
import kotlin.math.roundToInt

/** A terminal palette, as herdr-app ships them. Colors are 0xrrggbb. */
data class Palette(
    val background: Int,
    val foreground: Int,
    val selection: Int,
    val cursor: Int? = null,
    val ansi: List<Int>,
    /**
     * working, blocked, done: the presets pick their own; theme files take
     * them from ansi (yellow, red, blue).
     */
    val status: List<Int>? = null,
)

/**
 * Every color the app uses, resolved, by name (`background`, `chrome`,
 * `ansi0`…). Values are 0xAARRGGBB, ready for Compose's `Color(Long)`.
 */
data class Theme(val colors: Map<String, Long>) {
    operator fun get(name: String): Long = colors[name] ?: error("no color $name")

    /** `#rrggbb`, or `#rrggbbaa` when translucent: the desktop's spelling. */
    fun hex(name: String): String = hex(get(name))
}

/** `themes/<name>.json`: a palette, plus `colors` overrides. */
@Serializable
data class ThemeFile(
    val background: String,
    val foreground: String,
    val selection: String,
    val cursor: String? = null,
    val ansi: List<String>,
    val colors: Map<String, String> = emptyMap(),
)

private val DARK_STATUS = listOf(0xe5c07b, 0xf0705f, 0x76a8ff)

val PRESETS: Map<String, Palette> = linkedMapOf(
    "graphite" to Palette(
        background = 0x17191d,
        foreground = 0xd2d4d9,
        selection = 0x394454,
        status = DARK_STATUS,
        ansi = listOf(
            0x45484f, 0xdc8c95, 0xa6bd91, 0xd8be87, 0x96b2d7, 0xb5a2ce, 0x8cbdc3, 0xd2d4d9, 0x818793,
            0xf0a5ad, 0xbad5a2, 0xead2a1, 0xb0cbef, 0xcbb7e6, 0xa4d5db, 0xf0f0f2,
        ),
    ),
    "ink" to Palette(
        background = 0x111720,
        foreground = 0xcdd6e4,
        selection = 0x344861,
        status = DARK_STATUS,
        ansi = listOf(
            0x414b5a, 0xe58c9a, 0x89c5aa, 0xd9be82, 0x91b6ee, 0xb9a0e2, 0x85c4d0, 0xcdd6e4, 0x7f91a7,
            0xf5a5b2, 0xa5dec3, 0xedd39c, 0xaccbfa, 0xd0b7f4, 0xa0dce5, 0xf0f5fc,
        ),
    ),
    "moss" to Palette(
        background = 0x151c19,
        foreground = 0xcdd7ce,
        selection = 0x354a40,
        status = DARK_STATUS,
        ansi = listOf(
            0x435248, 0xdf978d, 0xa0c4a2, 0xd4bc8b, 0x99b9cd, 0xbba4c8, 0x8bbdb5, 0xcdd7ce, 0x82968a,
            0xf1afa3, 0xb8dab9, 0xe9d2a5, 0xb1d0e2, 0xd1bddc, 0xa4d6ca, 0xeff5ee,
        ),
    ),
    // Light: white paper, dark ink. Every ANSI color stays readable on white.
    "paper" to Palette(
        background = 0xffffff,
        foreground = 0x25272c,
        selection = 0xcfdcf5,
        cursor = 0x25272c,
        status = listOf(0x9a6700, 0xc93c2c, 0x2f6fe4),
        ansi = listOf(
            0x25272c, 0xc62a2a, 0x1a7f37, 0x8f5d00, 0x1f62d0, 0x7a45c8, 0x0f6f74, 0x5f6068, 0x7b7e87,
            0xd1242f, 0x1f883d, 0x9a6700, 0x0969da, 0x8250df, 0x1b7c83, 0x111214,
        ),
    ),
)

val PRESET_NAMES: List<String> = PRESETS.keys.toList()

/** The theme shown when none is chosen, by appearance. */
const val DEFAULT_DARK = "graphite"
const val DEFAULT_LIGHT = "paper"

fun isLight(c: Int): Boolean {
    fun ch(s: Int) = ((c shr s) and 255) / 255.0
    return 0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0) > 0.5
}

private fun mix(a: Int, b: Int, t: Double): Int = listOf(16, 8, 0).fold(0) { acc, s ->
    val x = (a shr s) and 255
    val y = (b shr s) and 255
    acc or ((x * (1 - t) + y * t).roundToInt() shl s)
}

private fun opaque(rgb: Int): Long = 0xff000000L or (rgb.toLong() and 0xffffff)

/**
 * The panel colors around a palette. `chrome` (sidebars, bars) and `canvas`
 * (behind cards) step away from the terminal's background: darker on a dark
 * theme, toward warm grey on a light one. Overrides are `#rrggbb[aa]`.
 */
fun resolve(p: Palette, overrides: Map<String, String> = emptyMap()): Theme {
    val bg = p.background
    val light = isLight(bg)
    val fg = if (light) 0x1b1b1e else 0xeeeeef
    val (working, blocked, done) = p.status ?: listOf(p.ansi[3], p.ansi[1], p.ansi[4])
    val warm = 0xe8e8e2
    val t = linkedMapOf<String, Long>()
    t["background"] = opaque(bg)
    t["terminal_foreground"] = opaque(p.foreground)
    t["foreground"] = opaque(fg)
    t["surface"] = opaque(bg)
    if (light) {
        t["canvas"] = opaque(mix(bg, warm, 0.2))
        t["chrome"] = opaque(mix(bg, warm, 0.6))
        t["popover"] = opaque(bg)
        t["raised"] = opaque(mix(bg, 0, 0.035))
        t["hover"] = 0x0c000000L
        t["selected"] = 0x12000000L
        t["border"] = 0x17000000L
        t["input"] = 0x26000000L
    } else {
        t["canvas"] = opaque(mix(bg, 0, 0.16))
        t["chrome"] = opaque(mix(bg, 0, 0.32))
        t["popover"] = opaque(mix(bg, 0xffffff, 0.06))
        t["raised"] = opaque(mix(bg, 0xffffff, 0.09))
        t["hover"] = 0x0effffffL
        t["selected"] = 0x17ffffffL
        t["border"] = 0x18ffffffL
        t["input"] = 0x24ffffffL
    }
    t["muted"] = opaque(mix(bg, fg, if (light) 0.66 else 0.6))
    t["dim"] = opaque(mix(bg, fg, if (light) 0.46 else 0.38))
    t["accent"] = opaque(p.ansi[4])
    t["accent_foreground"] = opaque(bg)
    t["solid"] = opaque(fg)
    t["solid_foreground"] = opaque(if (light) 0xffffff else mix(bg, 0, 0.28))
    t["ring"] = opaque(mix(bg, fg, 0.7))
    t["selection"] = opaque(p.selection)
    t["cursor"] = opaque(p.cursor ?: p.foreground)
    t["working"] = opaque(working)
    t["blocked"] = opaque(blocked)
    t["done"] = opaque(done)
    t["idle"] = opaque(mix(bg, fg, 0.38))
    t["unknown"] = opaque(mix(bg, fg, 0.38))
    p.ansi.forEachIndexed { i, c -> t["ansi$i"] = opaque(c) }
    for ((k, v) in overrides) t[k] = requireNotNull(color(v)) { "$k \"$v\" is not a color" }
    return Theme(t)
}

private val RRGGBB = Regex("^#?[0-9a-fA-F]{6}$")
private val RRGGBBAA = Regex("^#?([0-9a-fA-F]{6}|[0-9a-fA-F]{8})$")

/** `#rrggbb` or `#rrggbbaa` (CSS order) as 0xAARRGGBB, or null. */
fun color(v: String): Long? {
    val h = v.trim()
    if (!RRGGBBAA.matches(h)) return null
    val digits = h.removePrefix("#")
    val n = digits.toLong(16)
    return if (digits.length == 6) 0xff000000L or n else ((n and 0xff) shl 24) or (n ushr 8)
}

/** 0xAARRGGBB as `#rrggbb`, or `#rrggbbaa` when not opaque. */
fun hex(argb: Long): String {
    val rgb = (argb and 0xffffff).toString(16).padStart(6, '0')
    val a = (argb ushr 24) and 0xff
    return if (a == 0xffL) "#$rgb" else "#$rgb${a.toString(16).padStart(2, '0')}"
}

private fun rgb(k: String, v: String): Int {
    val h = v.trim()
    require(RRGGBB.matches(h)) { "$k \"$v\" is not #rrggbb" }
    return h.removePrefix("#").toInt(16)
}

/** A theme file's palette and overrides, or why it can't be shown (thrown). */
fun parseThemeFile(f: ThemeFile): Pair<Palette, Map<String, String>> {
    require(f.ansi.size == 16) { "ansi has ${f.ansi.size} colors, not 16" }
    for ((k, v) in f.colors) {
        require(color(v) != null) { "colors.$k: \"$v\" is not #rrggbb or #rrggbbaa" }
    }
    val palette = Palette(
        background = rgb("background", f.background),
        foreground = rgb("foreground", f.foreground),
        selection = rgb("selection", f.selection),
        cursor = f.cursor?.let { rgb("cursor", it) },
        ansi = f.ansi.mapIndexed { i, v -> rgb("ansi[$i]", v) },
    )
    return palette to f.colors
}
