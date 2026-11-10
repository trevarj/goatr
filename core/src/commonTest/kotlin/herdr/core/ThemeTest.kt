package herdr.core

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class ThemeTest {
    @Test
    fun presetsAreDarkExceptPaper() {
        assertEquals(listOf("paper"), PRESETS.filterValues { isLight(it.background) }.keys.toList())
    }

    @Test
    fun graphiteResolvesLikeTheDesktop() {
        val t = resolve(PRESETS.getValue("graphite"))
        assertEquals("#17191d", t.hex("background"))
        assertEquals("#eeeeef", t.hex("foreground"))
        assertEquals("#96b2d7", t.hex("accent"))
        assertEquals("#f0705f", t.hex("blocked"))
        assertEquals("#ffffff18", t.hex("border"))
        assertEquals("#f0f0f2", t.hex("ansi15"))
        // The rest of what the desktop's generated CSS says for graphite.
        assertEquals("#131518", t.hex("canvas"))
        assertEquals("#101114", t.hex("chrome"))
        assertEquals("#25272b", t.hex("popover"))
        assertEquals("#2c2e31", t.hex("raised"))
        assertEquals("#98999b", t.hex("muted"))
        assertEquals("#696a6d", t.hex("dim"))
        assertEquals("#111215", t.hex("solid_foreground"))
        assertEquals("#aeaeb0", t.hex("ring"))
        assertEquals(0x18ffffffL, t["border"])
        assertEquals(0xff17191dL, t["background"])
    }

    @Test
    fun paperUsesTranslucentBlackForChromeStates() {
        val t = resolve(PRESETS.getValue("paper"))
        assertEquals("#ffffff", t.hex("popover"))
        assertEquals("#0000000c", t.hex("hover"))
        assertEquals("#ffffff", t.hex("solid_foreground"))
        assertEquals("#fafaf9", t.hex("canvas"))
        assertEquals("#f1f1ee", t.hex("chrome"))
        assertEquals("#f6f6f6", t.hex("raised"))
    }

    @Test
    fun themeFilesTakeStatusFromAnsiAndApplyOverrides() {
        val (palette, overrides) = parseThemeFile(
            ThemeFile(
                background = "#21252b",
                foreground = "#abb2bf",
                selection = "#323844",
                ansi = listOf(
                    "#454c59", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
                    "#767676", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
                ),
                colors = mapOf("accent" to "#123456", "hover" to "#11223344"),
            ),
        )
        val t = resolve(palette, overrides)
        assertEquals("#e5c07b", t.hex("working"))
        assertEquals("#123456", t.hex("accent"))
        assertEquals(0x44112233L, t["hover"])
        assertEquals("#11223344", t.hex("hover"))
    }

    @Test
    fun badThemeFilesAreRefused() {
        assertFailsWith<IllegalArgumentException> {
            parseThemeFile(ThemeFile(background = "red", foreground = "#000000", selection = "#000000", ansi = emptyList()))
        }
        assertFailsWith<IllegalArgumentException> {
            parseThemeFile(
                ThemeFile("#000000", "#000000", "#000000", ansi = List(16) { "#000000" }, colors = mapOf("x" to "nope")),
            )
        }
    }
}
