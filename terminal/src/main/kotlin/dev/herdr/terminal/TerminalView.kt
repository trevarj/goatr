package dev.herdr.terminal

import android.content.Context
import android.graphics.Typeface
import android.view.KeyEvent
import android.widget.LinearLayout
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/**
 * Terminal colors in ghostty config syntax (`#rrggbb`, `rrggbb`, X11 names).
 * Null fields keep ghostty's defaults.
 */
data class TerminalTheme(
    val background: String? = null,
    val foreground: String? = null,
    val cursorColor: String? = null,
    val selectionBackground: String? = null,
    val selectionForeground: String? = null,
    /** Overrides by index (0–255) on top of ghostty's default palette. */
    val palette: List<String?>? = null,
)

/** When the key bar shows. */
enum class KeyBarMode {
    /** Always, under the grid; above the soft keyboard while it's up. */
    ALWAYS,

    /** Only while the soft keyboard is up, riding on it. */
    TYPING,

    /** Never. */
    OFF,
}

/** The key bar's colors (ghostty color syntax); null fields keep the default. */
data class KeyColors(
    val background: String? = null,
    val key: String? = null,
    val foreground: String? = null,
    val activeKey: String? = null,
    val activeForeground: String? = null,
)

/**
 * A ghostty-backed terminal: the grid, plus a key bar (Termux's keys, paste
 * and enter) under it, which rides the soft keyboard while it's up. The host owns transport and
 * session lifecycle; this view renders bytes pushed via [write] and reports
 * user input and grid resizes through [onInput] and [onResize].
 *
 * Main thread only: every method, and every callback fires on it.
 */
class TerminalView(context: Context) : LinearLayout(context) {
    /** Bytes for the PTY: typed text, encoded keys, pastes, VT query replies. */
    var onInput: ((ByteArray) -> Unit)? = null

    /** The grid's size in cells; fires once laid out and on every change. */
    var onResize: ((cols: Int, rows: Int) -> Unit)? = null

    /**
     * A link the user tapped (with a mouse, Ctrl+clicked) or picked from the
     * long-press toolbar's "Open link": an OSC 8 hyperlink's target as the
     * program gave it, or an http(s) URL written in the terminal. Opening it
     * is the host's call: true when it did, else the tap is the terminal's.
     */
    var onLink: ((String) -> Boolean)? = null

    /**
     * Images for the program, as content URIs: the keyboard's (a screenshot
     * or copied image it offers) or the clipboard's, pasted. Sending them is
     * the host's (then [paste] where they landed).
     */
    var onImages: ((List<android.net.Uri>) -> Unit)? = null

    private val terminal = GhosttyTerminalView(context).also { view ->
        view.onInputBytes = { bytes -> onInput?.invoke(bytes) }
        view.onGridResize = { cols, rows -> onResize?.invoke(cols, rows) }
        view.onLink = { link -> onLink?.invoke(link) == true }
        view.onImages = { uris -> onImages?.invoke(uris) }
    }

    private val accessoryBar = TerminalAccessoryBar(context).also { bar ->
        bar.onKey = terminal::sendAccessoryKey
        bar.onText = terminal::sendText
        bar.onPaste = terminal::requestPaste
    }

    private var keyBarMode = KeyBarMode.ALWAYS

    init {
        orientation = VERTICAL
        terminal.accessoryBar = accessoryBar
        addView(terminal, LayoutParams(LayoutParams.MATCH_PARENT, 0, 1f))
        addView(accessoryBar, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        // The bar rides the soft keyboard, padded by the overlap so it sits
        // right above it (and, per [KeyBarMode], shows only while it's up).
        // Insets aren't always dispatched to nested views, so size changes
        // re-check too.
        ViewCompat.setOnApplyWindowInsetsListener(this) { _, insets ->
            syncAccessoryBar(insets)
            insets
        }
    }

    private fun syncAccessoryBar(insets: WindowInsetsCompat) {
        val imeVisible = insets.isVisible(WindowInsetsCompat.Type.ime())
        val shown = when (keyBarMode) {
            KeyBarMode.ALWAYS -> true
            KeyBarMode.TYPING -> imeVisible
            KeyBarMode.OFF -> false
        }
        val visibility = if (shown) VISIBLE else GONE
        var changed = accessoryBar.visibility != visibility
        accessoryBar.visibility = visibility
        // Edge-to-edge windows don't resize for the keyboard; it covers us.
        // Pad by the covered amount so the bar sits above the IME and the
        // terminal shrinks. Hosts that already handle the keyboard leave no
        // overlap, so the padding stays zero.
        val imeBottom = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
        val location = IntArray(2)
        getLocationInWindow(location)
        val gapBelow = rootView.height - (location[1] + height)
        val overlap = if (imeVisible) (imeBottom - gapBelow).coerceAtLeast(0) else 0
        if (paddingBottom != overlap) {
            setPadding(0, 0, 0, overlap)
            changed = true
        }
        // From onSizeChanged this runs inside layout(), which drops a layout asked
        // for in it as it ends: ask again after, or the grid and the bar keep the
        // sizes they had with the old padding (a grid 0 rows high, coming back
        // to the app with the keyboard up).
        if (changed) post(::requestLayout)
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        resyncAccessoryBar()
    }

    private fun resyncAccessoryBar() {
        val rootInsets = rootWindowInsets ?: return
        syncAccessoryBar(WindowInsetsCompat.toWindowInsetsCompat(rootInsets, this))
    }

    /** Pastes `text` as the program takes pastes, without asking: the host's own (paths it put there). */
    fun paste(text: String) = terminal.pasteText(text)

    /** A cell's size in pixels (width, height), for the host's display facts. */
    val cellPixels: Pair<Int, Int> get() = terminal.cellPixels

    /** Whether programs may copy to the phone's clipboard (OSC 52); on by default. */
    fun setClipboardWrite(allowed: Boolean) = terminal.setClipboardWrite(allowed)

    /**
     * Hands the terminal a hardware keyboard's key that arrived while focus was
     * elsewhere on screen; it takes focus and the key. False for the keys it
     * leaves alone (system keys: back, volume, media).
     */
    fun takeHardwareKey(event: KeyEvent): Boolean = terminal.takeHardwareKey(event)

    /** When the key bar shows (default [KeyBarMode.ALWAYS]). */
    fun setKeyBar(mode: KeyBarMode) {
        if (mode == keyBarMode) return
        keyBarMode = mode
        if (rootWindowInsets != null) resyncAccessoryBar()
        else accessoryBar.visibility = if (mode == KeyBarMode.ALWAYS) VISIBLE else GONE
    }

    /** Base font size in dp (default 14, clamped 4–64); the grid reflows in place. */
    fun setFontSize(dp: Float) = terminal.setFontSize(dp)

    /** The terminal font; null is the system monospace. The grid reflows. */
    fun setTypeface(typeface: Typeface?) {
        terminal.setTypeface(typeface)
        accessoryBar.setLabelTypeface(terminal.labelTypeface())
    }

    /** Programming ligatures for fonts that have them (liga/calt). */
    fun setLigatures(enabled: Boolean) = terminal.setLigatures(enabled)

    /** Theme colors; null clears back to the defaults. */
    fun setTheme(theme: TerminalTheme?) = terminal.setTheme(theme)

    /** Key bar colors; null fields (or null) keep the defaults. */
    fun setKeyColors(colors: KeyColors?) {
        fun parse(c: String?, fallback: Int) = GhosttyVt.colorOrNull(c) ?: fallback
        val d = TerminalAccessoryBar.Colors()
        accessoryBar.setColors(
            TerminalAccessoryBar.Colors(
                background = parse(colors?.background, d.background),
                key = parse(colors?.key, d.key),
                foreground = parse(colors?.foreground, d.foreground),
                activeKey = parse(colors?.activeKey, d.activeKey),
                activeForeground = parse(colors?.activeForeground, d.activeForeground),
            ),
        )
    }

    /**
     * Feed program output into the grid. It is parsed before this returns
     * (the moment to grant the stream credit); drawing follows on the next frame.
     */
    fun write(data: ByteArray) = terminal.write(data)

    /**
     * While true the grid shows only its background: what's written is parsed
     * and kept, and shows once it's let go (a frame the program is about to replace).
     */
    var held: Boolean
        get() = terminal.held
        set(value) {
            terminal.held = value
        }

    /** The program exited: stop forwarding input. */
    fun finish() = terminal.finish()

    /** Free the native terminal. The view must not be used afterwards. */
    fun destroy() = terminal.destroy()
}
