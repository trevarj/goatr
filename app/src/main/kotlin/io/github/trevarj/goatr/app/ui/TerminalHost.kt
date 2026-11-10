// The live terminals, kept for as long as the activity: the one on screen and
// the last few shown, each a ghostty view with its attach stream, as herdr-app
// keeps its tabs. Switching back to one shows the same grid with nothing to
// replay, so it never flashes the screen at the size the program had before.
// The core's attach stream feeds each; bytes are credited back only once
// drawn, so a flood can't swamp the phone; a dropped connection reattaches
// (`resumed`) as soon as the client is back.

package io.github.trevarj.goatr.app.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.KeyEvent
import android.view.ViewGroup
import android.widget.Toast
import androidx.core.content.res.ResourcesCompat
import io.github.trevarj.goatr.app.R
import io.github.trevarj.goatr.app.session.Prefs
import io.github.trevarj.goatr.app.session.drawLigatures
import io.github.trevarj.goatr.app.session.hostOf
import io.github.trevarj.goatr.app.session.localOf
import io.github.trevarj.goatr.app.session.readFile
import io.github.trevarj.goatr.app.session.readImage
import io.github.trevarj.goatr.app.session.reason
import dev.herdr.terminal.KeyBarMode
import dev.herdr.terminal.KeyColors
import dev.herdr.terminal.TerminalTheme
import dev.herdr.terminal.TerminalView
import herdr.core.AttachOptions
import herdr.core.Attached
import herdr.core.Client
import herdr.core.Display
import herdr.core.End
import herdr.core.pasted
import herdr.core.put
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

sealed interface PaneState {
    data object Waiting : PaneState
    data object Attaching : PaneState
    data object Live : PaneState
    data class Ended(val why: String) : PaneState
    data class Failed(val why: String) : PaneState
}

private fun TerminalView.detach() {
    (parent as? ViewGroup)?.removeView(this)
}

/** How many terminals stay attached: the one on screen and the last ones shown. */
private const val KEPT = 4

/** A resized program's redraw is in once its output pauses this long (ms)... */
private const val SETTLED_MS = 32L

/** ...or after this long, if it doesn't redraw at all (ms). */
private const val SETTLE_MAX_MS = 100L

/** The most a main-thread turn spends parsing output before letting a frame or a touch in (ms). */
private const val PARSE_BUDGET_MS = 8L

/** One attached terminal: its view, its stream and how the attach is going. */
private class Live(val view: TerminalView, val state: MutableStateFlow<PaneState>) {
    var job: Job? = null
    var stream: Attached? = null
}

/** Everything here runs on the main thread (the scope is the activity's composition). */
@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class TerminalHost(
    private val context: Context,
    private val scope: CoroutineScope,
    /** A host's client while it's connected. */
    private val client: (host: String) -> Flow<Client?>,
    /** A terminal's size as its core has it, before this viewer sizes it. */
    private val sizeOf: (address: String) -> Pair<Int, Int>?,
) {
    // Most recently shown last; the last one is on screen.
    private val kept = LinkedHashMap<String, Live>()
    private val shown = MutableStateFlow<Live?>(null)
    private val main = Handler(Looper.getMainLooper())

    /** How the terminal on screen is attaching. */
    val state: StateFlow<PaneState> = shown
        .flatMapLatest { it?.state ?: flowOf(PaneState.Waiting) }
        .stateIn(scope, SharingStarted.Eagerly, PaneState.Waiting)

    private val view: TerminalView? get() = shown.value?.view

    // This viewer's colors and cell size, for the core: it answers programs that
    // ask whether the terminal is dark or light (CSI ? 996 n) from them.
    private var display: Display? = null

    /**
     * The view for `address` (host-qualified): one kept from before, as it was
     * left, or a fresh one (ghostty's grid belongs to one stream).
     */
    fun show(address: String): TerminalView {
        shown.value?.let { if (it !== kept[address]) it.stream?.focus(false) }
        val live = kept.remove(address) ?: attach(address)
        kept[address] = live
        // The oldest beyond what's kept let go of their streams.
        while (kept.size > KEPT) {
            val oldest = kept.keys.first()
            kept.remove(oldest)?.let(::free)
        }
        shown.value = live
        live.stream?.focus(true)
        live.view.detach()
        return live.view
    }

    private fun attach(address: String): Live {
        val size = MutableStateFlow<Pair<Int, Int>?>(null)
        val made = TerminalView(context)
        val live = Live(made, MutableStateFlow(PaneState.Waiting))
        made.onInput = { bytes -> live.stream?.write(bytes) }
        made.onResize = { cols, rows ->
            size.value = cols to rows
            live.stream?.resize(cols, rows)
        }
        made.onLink = ::openLink
        made.onImages = { sendImages(address, it) }
        live.job = scope.launch {
            var seen = false
            // Attach once there's a client and a size; a later resize (the keyboard,
            // rotation) goes to the live stream, never a fresh attach.
            combine(client(hostOf(address)), size) { c, s -> c to (s != null) }.distinctUntilChanged().collectLatest { (c, sized) ->
                val s = size.value
                if (c == null || !sized || s == null) {
                    live.state.value = PaneState.Waiting
                    return@collectLatest
                }
                live.state.value = PaneState.Attaching
                // Attaching at another size resizes the program: the core's first paint
                // is its old frame cut to the new size, and the program redraws right
                // after. Show neither the old frame nor the jump: hold the grid until
                // its output pauses.
                val settling = sizeOf(address)?.let { it != s } ?: false
                val settled = Runnable {
                    made.held = false
                    if (live.stream != null) live.state.value = PaneState.Live
                }
                // Apart from the pause timer, which output keeps pushing back: a program
                // that never stops printing shows too.
                val cap = Runnable { settled.run() }
                val a = try {
                    c.attach(localOf(address), AttachOptions(s.first, s.second, history = 2000, resumed = seen, display = display))
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    live.state.value = PaneState.Failed(e.reason)
                    return@collectLatest
                }
                seen = true
                live.stream = a
                if (settling) {
                    made.held = true
                    main.postDelayed(cap, SETTLE_MAX_MS)
                }
                // Stream listeners run on the client's thread; the view is the main thread's.
                // What arrives while the main thread is busy is written together, so a
                // backlog is parsed before the next frame rather than one chunk per frame,
                // but in turns of at most PARSE_BUDGET_MS, so a flood doesn't hold frames
                // and touches until it's all in. Posted to the main thread rather than the
                // view: one off screen is detached, and its output must still be parsed and
                // credited. The end comes in order, after the output before it.
                val backlog = Backlog<End>(
                    now = SystemClock::uptimeMillis,
                    post = { main.post(it) },
                    budgetMs = PARSE_BUDGET_MS,
                    write = { chunk ->
                        if (live.stream === a) {
                            made.write(chunk)
                            // Parsed: credit the stream for them.
                            a.consumed(chunk.size)
                        }
                    },
                    ended = { end ->
                        if (live.stream === a) live.stream = null
                        when (end) {
                            is End.Lost -> live.state.value = PaneState.Waiting
                            is End.Closed -> {
                                made.finish()
                                live.state.value = PaneState.Ended(end.summary)
                            }
                        }
                    },
                    afterTurn = {
                        if (made.held) {
                            main.removeCallbacks(settled)
                            main.postDelayed(settled, SETTLED_MS)
                        }
                    },
                )
                a.onData(backlog::add)
                a.onEnd(backlog::end)
                a.focus(shown.value === live)
                if (!settling) live.state.value = PaneState.Live
                try {
                    awaitCancellation()
                } finally {
                    main.removeCallbacks(settled)
                    main.removeCallbacks(cap)
                    made.held = false
                    a.close()
                    if (live.stream === a) live.stream = null
                }
            }
        }
        return live
    }

    private fun free(live: Live) {
        live.job?.cancel()
        live.stream = null
        live.view.detach()
        live.view.destroy()
        if (shown.value === live) shown.value = null
    }

    /** Draws with the user's font and the Herdr palette in use (the terminal on screen's). */
    fun style(prefs: Prefs, colors: HerdrColors) {
        val v = view ?: return
        v.setTypeface(typeface(prefs.font))
        v.setFontSize(prefs.fontSize.toFloat())
        v.setLigatures(prefs.drawLigatures)
        v.setKeyBar(
            when {
                !prefs.keyBar -> KeyBarMode.OFF
                prefs.keyBarTyping -> KeyBarMode.TYPING
                else -> KeyBarMode.ALWAYS
            },
        )
        v.setTheme(
            TerminalTheme(
                background = colors.background.hex(),
                foreground = colors.terminalForeground.hex(),
                cursorColor = colors.cursor.hex(),
                selectionBackground = colors.selection.hex(),
                palette = colors.ansi.map { it.hex() },
            ),
        )
        v.setClipboardWrite(prefs.clipboardWrite)
        val (cellWidth, cellHeight) = v.cellPixels
        val next = Display(
            bg = colors.background.hex(),
            fg = colors.terminalForeground.hex(),
            cursor = colors.cursor.hex(),
            cell = listOf(cellWidth, cellHeight),
        )
        if (next != display) {
            display = next
            for (live in kept.values) live.stream?.facts(next)
        }
        v.setKeyColors(
            KeyColors(
                background = colors.chrome.hex(),
                key = colors.raised.hex(),
                foreground = colors.foreground.hex(),
                activeKey = colors.solid.hex(),
                activeForeground = colors.solidForeground.hex(),
            ),
        )
    }

    /**
     * A tapped link, in the browser; false when it isn't one to open. Only
     * http(s): an OSC 8 target is whatever the program wrote, and other
     * schemes (intent:, file:, app links) aren't for a terminal to open.
     */
    private fun openLink(link: String): Boolean {
        val uri = Uri.parse(link)
        if (uri.scheme?.lowercase() !in setOf("http", "https") || uri.host.isNullOrEmpty()) return false
        return try {
            context.startActivity(Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE))
            true
        } catch (_: ActivityNotFoundException) {
            false
        }
    }

    // Images and other files on their way to a terminal.
    private val inFlight = MutableStateFlow(0 to 0)

    /** What's on its way to a terminal, to show while it goes ("sending 2 files…"); null when nothing is. */
    val sending: StateFlow<String?> = inFlight.map { (images, files) ->
        val n = images + files
        when {
            n == 0 -> null
            files == 0 -> if (n == 1) "sending image…" else "sending $n images…"
            else -> if (n == 1) "sending file…" else "sending $n files…"
        }
    }.stateIn(scope, SharingStarted.Eagerly, null)

    /** Images for `address`'s program (picked, pasted, or the keyboard's), made ready for agents ([readImage]). */
    fun sendImages(address: String, uris: List<Uri>) = send(address, uris, files = false)

    /** Files for `address`'s program, as they are ([readFile]). */
    fun sendFiles(address: String, uris: List<Uri>) = send(address, uris, files = true)

    /**
     * Each copied beside `address`, on its machine, then where they landed
     * pasted into it, as herdr-app pastes an image into a terminal elsewhere.
     * One that can't go (too big, unreadable) doesn't stop the others.
     */
    private fun send(address: String, uris: List<Uri>, files: Boolean) {
        if (uris.isEmpty()) return
        fun count(by: Int) = inFlight.update { (i, f) -> if (files) i to f + by else i + by to f }
        scope.launch {
            count(uris.size)
            val failed = mutableListOf<String>()
            try {
                val c = client(hostOf(address)).first() ?: error("Not connected")
                val paths = uris.mapNotNull { uri ->
                    try {
                        val upload = withContext(Dispatchers.IO) {
                            if (files) readFile(context.contentResolver, uri) else readImage(context.contentResolver, uri)
                        }
                        put(c, localOf(address), upload.name, upload.bytes)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        failed += e.reason
                        null
                    }
                }
                if (paths.isNotEmpty()) kept[address]?.view?.paste(pasted(paths))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                failed += e.reason
            } finally {
                count(-uris.size)
            }
            if (failed.isNotEmpty()) {
                val what = if (files) "file" else "image"
                Toast.makeText(context, "Couldn't send the $what: ${failed.joinToString("; ")}", Toast.LENGTH_LONG).show()
            }
        }
    }

    private val typefaces = mutableMapOf<String?, Typeface?>()

    private fun typeface(font: String?): Typeface? = typefaces.getOrPut(font) {
        when (font) {
            "JetBrains Mono" -> ResourcesCompat.getFont(context, R.font.jetbrains_mono)
            "Geist Mono" -> ResourcesCompat.getFont(context, R.font.geist_mono)
            else -> null
        }
    }

    /** A hardware keyboard's key for the terminal on screen, if one is; false if it isn't taken. */
    fun takeHardwareKey(event: KeyEvent): Boolean = view?.takeIf { it.isShown }?.takeHardwareKey(event) ?: false

    /** Detaches and frees every terminal (none to show, or the activity is gone). */
    fun release() {
        for (live in kept.values) free(live)
        kept.clear()
        shown.value = null
    }
}
