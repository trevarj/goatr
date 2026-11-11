package dev.herdr.terminal

import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Rect
import android.graphics.RenderNode
import android.graphics.Typeface
import android.net.Uri
import android.os.SystemClock
import android.provider.Settings
import android.text.InputType
import android.view.ActionMode
import android.view.Choreographer
import android.view.GestureDetector
import android.view.HapticFeedbackConstants
import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.Menu
import android.view.MenuItem
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.ScaleGestureDetector
import android.view.View
import android.view.ViewConfiguration
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.Magnifier
import android.widget.OverScroller
import androidx.core.view.ContentInfoCompat
import androidx.core.view.OnReceiveContentListener
import androidx.core.view.ViewCompat
import androidx.core.view.inputmethod.EditorInfoCompat
import androidx.core.view.inputmethod.InputConnectionCompat
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.sign

/**
 * Canvas-backed ghostty terminal grid.
 *
 * Owns a libghostty-vt session via [GhosttyVt]. PTY output is fed in with
 * [write]; user input (IME text, key escape sequences, VT query responses)
 * comes back through [onInputBytes]. Rendering pulls flattened dirty-row
 * snapshots from JNI once per Choreographer frame and records each dirty row
 * into its own RenderNode; [onDraw] replays the rows (drawn by the GPU, clean
 * ones without re-recording) and overlays the cursor.
 */
internal class GhosttyTerminalView(context: Context) : View(context) {
  var onInputBytes: ((ByteArray) -> Unit)? = null
  var onGridResize: ((cols: Int, rows: Int) -> Unit)? = null

  /** A link tapped (OSC 8 or a written http(s) URL), for the host to open; true when it did. */
  var onLink: ((String) -> Boolean)? = null

  /**
   * Images for the program: the keyboard's (a screenshot or copied image it
   * offers, a sticker) or the clipboard's, pasted. Their content URIs, for
   * the host to send; null takes none, and the keyboard offers none.
   */
  var onImages: ((List<Uri>) -> Unit)? = null
    set(value) {
      field = value
      if (value == null) ViewCompat.setOnReceiveContentListener(this, null, null)
      else ViewCompat.setOnReceiveContentListener(this, arrayOf("image/*"), imageReceiver)
    }

  // Images go to onImages; anything else (text) is left for the caller.
  private val imageReceiver = OnReceiveContentListener { _, payload ->
    if (!payload.clip.description.hasMimeType("image/*")) return@OnReceiveContentListener payload
    val split = payload.partition { it.uri != null }
    split.first?.clip?.let { clip -> onImages?.invoke((0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }) }
    split.second
  }

  /** Draws only the background while true; see [TerminalView.held]. */
  var held = false
    set(value) {
      if (field == value) return
      field = value
      invalidate()
    }

  /** Sticky-modifier source for accessory-bar keys and IME text (optional). */
  var accessoryBar: TerminalAccessoryBar? = null

  private var handle: Long = GhosttyVt.create(INITIAL_COLS, INITIAL_ROWS, MAX_SCROLLBACK)
  private var finished = false

  // The font, its cell metrics, and drawing rows and the cursor.
  private val painter = GridPainter(context)
  private val cellWidth get() = painter.cellWidth
  private val cellHeight get() = painter.cellHeight
  private val underlineThickness get() = painter.underlineThickness

  // ── Grid state ──
  private var cols = INITIAL_COLS
  private var rows = INITIAL_ROWS
  // One display list per grid row, re-recorded only when the row changes, moved
  // when it only scrolled; none until the row is first drawn. Each spans the
  // whole grid and clips to it, so a glyph spilling past its row or the grid's
  // edge draws as it did onto the grid-sized bitmap these replaced.
  private val rowNodes = RowDrawings(
    create = { RenderNode("terminal row").apply { setPosition(0, 0, gridWidthPx, gridHeightPx) } },
    discard = RenderNode::discardDisplayList,
    place = { node, rowsMoved -> node.translationY = (rowsMoved * cellHeight).toFloat() },
  )
  // A frame's row records: where each row's is in the snapshot, where it moved from.
  private var recordAt = IntArray(0)
  private var movedFrom = IntArray(0)
  private var recorded = BooleanArray(0)
  private var gridWidthPx = 0
  private var gridHeightPx = 0
  private var snapshotBuf: ByteBuffer? = null

  // ── Selection state ──
  private var selectionMode = false
  private var actionMode: ActionMode? = null
  private var draggingHandle = HANDLE_NONE
  private var dragAnchorCol = 0
  private var dragAnchorRow = 0
  private var dragBiasRows = 0
  private var lastPressCol = 0
  private var lastPressRow = 0
  // The link under the long press, offered as "Open link" until the selection ends.
  private var pressedLink: String? = null
  private val handlePaint = Paint(Paint.ANTI_ALIAS_FLAG)
  private val handleRadius = HANDLE_RADIUS_DP * resources.displayMetrics.density
  private val magnifier = Magnifier.Builder(this).build()

  // The cursor's color rides the snapshot header (0 = unconfigured).
  private var cursorColor = 0
  private var cursorX = -1
  private var cursorY = -1
  private var cursorStyle = GridPainter.CURSOR_STYLE_BLOCK
  private var cursorVisible = false
  private var cursorBlinks = false
  // On a wide character: two cells wide.
  private var cursorWide = false
  // The character under it, which a block draws again over itself.
  private val cursorGlyph = CharArray(4)
  private var cursorGlyphLength = 0
  private var cursorGlyphFlags = 0
  // Text an input method is composing (CJK), shown at the cursor until it's committed.
  private var preedit = ""
  // The clipboard key last taken (ClipboardKeys), until it's let go.
  private var clipboardKey = KeyEvent.KEYCODE_UNKNOWN

  // The cursor is an onDraw overlay, so blinking only replays the recorded
  // rows; the grid is untouched.
  private var blinkPhaseOn = true
  private var blinkScheduled = false
  private val blinkRunnable = object : Runnable {
    override fun run() {
      if (!blinkScheduled) return
      blinkPhaseOn = !blinkPhaseOn
      invalidate()
      postDelayed(this, BLINK_INTERVAL_MS)
    }
  }

  private val frameCallback: Choreographer.FrameCallback = Choreographer.FrameCallback { if (pacer.fired()) renderFrame() }
  private val pacer: FramePacer = FramePacer(SystemClock::uptimeMillis) { delay ->
    Choreographer.getInstance().postFrameCallbackDelayed(frameCallback, delay)
  }

  private var scrollAccum = 0f
  private var lastTouchX = 0f
  private var lastTouchY = 0f

  // ── Scrollback state ──
  private val scroller = OverScroller(context)
  private var flingLastY = 0
  private var scrollbarTotal = 0L
  private var scrollbarOffset = 0L
  private var scrollbarLen = 0L
  private var lastScrollActivityAt = 0L
  private val scrollbarPaint = Paint(Paint.ANTI_ALIAS_FLAG)

  // ── Mouse state ──
  // The buttons a program was sent a press for, so it gets their release too.
  private var mouseButtons = 0
  // A mouse drag selecting text (when the program doesn't take the mouse).
  private var mouseSelecting = false
  private var mouseMoved = false
  private var lastClickAt = 0L
  private var lastClickCell = -1
  // Where the mouse hovers (NaN: not over the grid), and the link under it
  // while Ctrl is held: viewport cells, row-major, first to last.
  private var hoverX = Float.NaN
  private var hoverY = Float.NaN
  private var hoverLink: IntRange? = null
  private val linkPaint = Paint()
  private val jumpChipPaint = Paint(Paint.ANTI_ALIAS_FLAG)
  private val jumpChipArrow = Path()
  private val flingRunnable = object : Runnable {
    override fun run() {
      if (handle == 0L || !scroller.computeScrollOffset()) return
      val y = scroller.currY
      scrollBy((y - flingLastY).toFloat())
      flingLastY = y
      postOnAnimation(this)
    }
  }

  private val gestureDetector = GestureDetector(
    context,
    object : GestureDetector.SimpleOnGestureListener() {
      override fun onDown(e: MotionEvent) = true

      override fun onSingleTapUp(e: MotionEvent): Boolean {
        if (scrolledBack) {
          val (cx, cy) = jumpChipCenter()
          val slop = JUMP_CHIP_RADIUS_DP * 1.5f * resources.displayMetrics.density
          if (hypot(e.x - cx, e.y - cy) <= slop) {
            jumpToBottom()
            return true
          }
        }
        if (selectionMode || actionMode != null) {
          clearSelection()
          return true
        }
        // A tap on a link opens it, even where the program takes the mouse: a finger
        // has no Ctrl to hold (a mouse does, and Ctrl+clicks, as on the desktop). A
        // long press still selects, and offers it as "Open link".
        if (openLinkAt(e.x, e.y)) return true
        // A program that takes the mouse (htop, lazygit, vim with mouse=a) gets the tap as a click.
        if (tracksMouse()) {
          sendMouse(MOUSE_PRESS, MOUSE_LEFT, e.x, e.y, e.metaState, anyPressed = true)
          sendMouse(MOUSE_RELEASE, MOUSE_LEFT, e.x, e.y, e.metaState, anyPressed = false)
        }
        requestFocus()
        context.getSystemService(InputMethodManager::class.java)
          ?.showSoftInput(this@GhosttyTerminalView, 0)
        return true
      }

      override fun onLongPress(e: MotionEvent) {
        if (handle == 0L || finished || scaleDetector.isInProgress) return
        lastPressCol = (e.x / cellWidth).toInt().coerceIn(0, cols - 1)
        lastPressRow = (e.y / cellHeight).toInt().coerceIn(0, rows - 1)
        pressedLink = linkAt(lastPressCol, lastPressRow)
        performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)
        selectionMode = GhosttyVt.selectWord(handle, lastPressCol, lastPressRow)
        scheduleFrame()
        if (selectionMode) {
          // Keep dragging without lifting: extend from the pressed cell.
          draggingHandle = HANDLE_END
          dragFrom(lastPressCol, lastPressRow, biasRows = 0)
        } else {
          // Even without a word under the finger, offer Paste / Select all.
          startTerminalActionMode()
        }
      }

      override fun onScroll(
        e1: MotionEvent?,
        e2: MotionEvent,
        distanceX: Float,
        distanceY: Float
      ): Boolean {
        if (handle == 0L || scaleDetector.isInProgress) return false
        lastTouchX = e2.x
        lastTouchY = e2.y
        scrollBy(distanceY)
        return true
      }

      override fun onFling(
        e1: MotionEvent?,
        e2: MotionEvent,
        velocityX: Float,
        velocityY: Float
      ): Boolean {
        if (handle == 0L || scaleDetector.isInProgress) return false
        // Finger up (negative velocity) scrolls content down: invert.
        flingLastY = 0
        scroller.fling(0, 0, 0, -velocityY.toInt(), 0, 0, Int.MIN_VALUE, Int.MAX_VALUE)
        postOnAnimation(flingRunnable)
        return true
      }
    }
  )

  // Mirrors the iOS pinch-zoom: every 0.1 of accumulated scale steps the
  // font size by ±1dp through the same path as the fontSize prop.
  private var pinchScale = 1f
  private var pinchLastScale = 1f
  private val scaleDetector = ScaleGestureDetector(
    context,
    object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
      override fun onScaleBegin(detector: ScaleGestureDetector): Boolean {
        pinchScale = 1f
        pinchLastScale = 1f
        return true
      }

      override fun onScale(detector: ScaleGestureDetector): Boolean {
        pinchScale *= detector.scaleFactor
        val steps = ((pinchScale - pinchLastScale) / PINCH_SCALE_STEP).toInt()
        if (steps != 0) {
          pinchLastScale += steps * PINCH_SCALE_STEP
          setFontSize(painter.fontSizeDp + steps)
        }
        return true
      }
    }
  )

  init {
    isFocusable = true
    isFocusableInTouchMode = true
    // A hardware key takes the view out of touch mode, where a focused view
    // gets Android's translucent focus highlight drawn over it: over the whole grid.
    defaultFocusHighlightEnabled = false
  }

  /**
   * Base font size in dp (default 14, clamped to 4–64 like iOS); pinch-to-zoom
   * steps from the current value. Content reflows in place — libghostty-vt
   * owns the grid state, so no output is lost.
   */
  fun setFontSize(dp: Float) {
    if (painter.setFontSize(dp)) remeasure()
  }

  private fun remeasure() {
    // Cell geometry moved under any active selection handles.
    if (selectionMode || actionMode != null) clearSelection()
    updateGridGeometry(force = true)
  }

  /**
   * The terminal font; null is the system monospace. Bold uses the font's
   * `wght` axis when it is a variable font, else Android's synthetic styles.
   */
  fun setTypeface(face: Typeface?) {
    if (painter.setTypeface(face)) remeasure()
  }

  /**
   * Programming ligatures (`->`, `!=`, `===`): runs of plain ASCII cells in one
   * style are drawn as one string so the font can join them; monospace
   * advances keep every glyph on its own cell. Off disables liga/calt.
   */
  fun setLigatures(enabled: Boolean) {
    if (painter.setLigatures(enabled)) updateGridGeometry(force = true)
  }

  /**
   * Apply theme colors. Foreground/background/cursor/palette go into the
   * terminal itself (so OSC 4/10/11/12 queries and resets stay truthful);
   * selection colors are a renderer concern and stay here. Null fields (or
   * a null theme) clear back to the defaults.
   */
  fun setTheme(theme: TerminalTheme?) {
    if (handle == 0L) return
    painter.selectionBg = GhosttyVt.colorOrNull(theme?.selectionBackground)
    painter.selectionFg = GhosttyVt.colorOrNull(theme?.selectionForeground)
    GhosttyVt.setTheme(
      handle, theme?.foreground, theme?.background, theme?.cursorColor, theme?.palette?.toTypedArray()
    )
    scheduleFrame()
  }

  // ── Session I/O ──

  /** Hands the link under ([x], [y]) to [onLink]; false when there's none, or the host didn't open it. */
  private fun openLinkAt(x: Float, y: Float): Boolean {
    if (cellWidth <= 0f || cellHeight <= 0) return false
    val col = (x / cellWidth).toInt()
    val row = (y / cellHeight).toInt()
    if (col !in 0 until cols || row !in 0 until rows) return false
    val link = linkAt(col, row) ?: return false
    return onLink?.invoke(link) == true
  }

  /** The link at a viewport cell, when the host takes links. */
  private fun linkAt(col: Int, row: Int): String? {
    if (onLink == null || handle == 0L) return null
    return GhosttyVt.linkAt(handle, col, row)?.let { String(it, Charsets.UTF_8) }
  }

  /** Feed PTY output into the grid. */
  fun write(data: ByteArray) {
    if (handle == 0L) return
    val response = GhosttyVt.write(handle, data)
    if (response != null && response.isNotEmpty() && !finished) {
      onInputBytes?.invoke(response)
    }
    GhosttyVt.takeClipboard(handle)?.let(::copyFromProgram)
    holdBlinkSolid()
    pacer.output()
  }

  /** Mark the underlying PTY as exited: stop forwarding input. */
  fun finish() {
    finished = true
  }

  fun destroy() {
    if (pacer.pending) Choreographer.getInstance().removeFrameCallback(frameCallback)
    pacer.detach()
    blinkScheduled = false
    removeCallbacks(blinkRunnable)
    scroller.abortAnimation()
    removeCallbacks(flingRunnable)
    actionMode?.finish()
    rowNodes.reset(0)
    if (handle != 0L) {
      GhosttyVt.free(handle)
      handle = 0
    }
  }

  private fun sendBytes(bytes: ByteArray) {
    if (!finished && bytes.isNotEmpty()) {
      if (selectionMode) clearSelection()
      // Typing while scrolled into history jumps back to the live view.
      if (handle != 0L && scrolledBack) jumpToBottom()
      holdBlinkSolid()
      pacer.input()
      onInputBytes?.invoke(bytes)
    }
  }

  /** Whether the viewport is above the live screen, in scrollback. */
  private val scrolledBack: Boolean
    get() = scrollbarTotal > 0 && scrollbarOffset + scrollbarLen < scrollbarTotal

  private fun jumpToBottom() {
    GhosttyVt.scrollToBottom(handle)
    scheduleFrame()
  }

  /** Accumulates a drag of [distance] px (positive: up) and scrolls by whole rows. */
  private fun scrollBy(distance: Float) {
    scrollAccum += distance
    val deltaRows = (scrollAccum / cellHeight).toInt()
    if (deltaRows != 0) {
      scrollAccum -= deltaRows * cellHeight
      scrollRows(deltaRows)
    }
  }

  /**
   * A swipe of [deltaRows] (negative: up, toward older lines) goes where the
   * program in front expects it: mouse-wheel events when it tracks the mouse
   * (opencode, htop, vim with mouse=a), arrow keys on the alternate screen
   * (less, man), else this view's scrollback.
   */
  private fun scrollRows(deltaRows: Int) {
    val notches = abs(deltaRows)
    when (GhosttyVt.scrollTarget(handle)) {
      1 -> GhosttyVt.encodeWheel(
        handle, deltaRows < 0, notches, lastTouchX, lastTouchY,
        width, height, cellWidth.roundToInt(), cellHeight
      )?.let(::sendBytes)
      2 -> {
        val key = if (deltaRows < 0) KeyEvent.KEYCODE_DPAD_UP else KeyEvent.KEYCODE_DPAD_DOWN
        repeat(notches) {
          GhosttyVt.encodeKey(handle, key, 1, 0, 0, 0, null)?.let(::sendBytes)
        }
      }
      else -> {
        GhosttyVt.scroll(handle, deltaRows)
        markScrollActivity()
        scheduleFrame()
      }
    }
  }

  /** The regular text face, for labels that should match the grid. */
  fun labelTypeface(): Typeface = painter.textPaint.typeface ?: Typeface.MONOSPACE

  private fun markScrollActivity() {
    lastScrollActivityAt = SystemClock.uptimeMillis()
  }

  /**
   * Typed text: IME commits and accessory-bar symbols. A lit sticky modifier
   * turns a single char into a key chord (Ctrl+C etc.) through ghostty's encoder.
   */
  fun sendText(text: CharSequence) {
    if (text.isEmpty()) return
    val meta = accessoryBar?.stickyMetaState() ?: 0
    if (meta != 0 && text.length == 1) {
      val keyCode = charToKeyCode(text[0])
      if (keyCode != KeyEvent.KEYCODE_UNKNOWN) {
        accessoryBar?.consumeSticky()
        GhosttyVt.encodeKey(handle, keyCode, 1, meta, 0, text[0].lowercaseChar().code, null)
          ?.let(::sendBytes)
        return
      }
    }
    sendBytes(text.toString().toByteArray(Charsets.UTF_8))
  }

  /** Send an accessory-bar key press with the current sticky modifiers. */
  fun sendAccessoryKey(keyCode: Int) {
    if (handle == 0L || finished) return
    val meta = accessoryBar?.takeStickyMetaState() ?: 0
    GhosttyVt.encodeKey(handle, keyCode, 1, meta, 0, 0, null)?.let(::sendBytes)
  }

  private fun charToKeyCode(char: Char): Int {
    val lower = char.lowercaseChar()
    return when (lower) {
      in 'a'..'z' -> KeyEvent.KEYCODE_A + (lower - 'a')
      in '0'..'9' -> KeyEvent.KEYCODE_0 + (lower - '0')
      ' ' -> KeyEvent.KEYCODE_SPACE
      '[' -> KeyEvent.KEYCODE_LEFT_BRACKET
      ']' -> KeyEvent.KEYCODE_RIGHT_BRACKET
      '-' -> KeyEvent.KEYCODE_MINUS
      '=' -> KeyEvent.KEYCODE_EQUALS
      '\\' -> KeyEvent.KEYCODE_BACKSLASH
      '/' -> KeyEvent.KEYCODE_SLASH
      '.' -> KeyEvent.KEYCODE_PERIOD
      ',' -> KeyEvent.KEYCODE_COMMA
      ';' -> KeyEvent.KEYCODE_SEMICOLON
      '\'' -> KeyEvent.KEYCODE_APOSTROPHE
      '`' -> KeyEvent.KEYCODE_GRAVE
      else -> KeyEvent.KEYCODE_UNKNOWN
    }
  }

  // ── Layout / resize ──

  override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
    super.onSizeChanged(w, h, oldw, oldh)
    updateGridGeometry(force = false)
  }

  private fun updateGridGeometry(force: Boolean) {
    if (width <= 0 || height <= 0 || handle == 0L) return
    val newCols = max(2, floor(width / cellWidth).toInt())
    val newRows = max(2, height / cellHeight)
    val changed = newCols != cols || newRows != rows
    cols = newCols
    rows = newRows
    if (changed || force || rowNodes.size != rows) {
      GhosttyVt.resize(handle, cols, rows, cellWidth.roundToInt(), cellHeight)
      allocateGridBuffers()
      onGridResize?.invoke(cols, rows)
    }
    scheduleFrame()
  }

  private fun allocateGridBuffers() {
    gridWidthPx = max(1, ceil(cols * cellWidth).toInt())
    gridHeightPx = max(1, rows * cellHeight)
    rowNodes.reset(rows)
    recordAt = IntArray(rows)
    movedFrom = IntArray(rows)
    recorded = BooleanArray(rows)
    val capacity = GridPainter.HEADER_BYTES + rows * (
      GridPainter.ROW_HEADER_BYTES + cols * GridPainter.CELL_RECORD_BYTES +
        cols * GridPainter.MAX_CELL_TEXT_UNITS * 2 + 4
      )
    snapshotBuf = ByteBuffer.allocateDirect(capacity).order(ByteOrder.LITTLE_ENDIAN)
    painter.allocate(cols, rows)
  }

  // ── Rendering ──

  private fun scheduleFrame() = pacer.request()

  private fun renderFrame() {
    val buf = snapshotBuf ?: return
    if (handle == 0L) return

    val rowCount = GhosttyVt.snapshot(handle, buf)
    if (rowCount < 0) return

    buf.position(0)
    buf.int // version
    buf.int // dirty kind: the rows sent say what changed
    val snapCols = buf.int
    buf.int // rows
    painter.defaultBg = buf.int
    painter.defaultFg = buf.int
    cursorX = buf.int
    cursorY = buf.int
    cursorStyle = buf.int
    cursorVisible = buf.int != 0
    buf.int // row count (== rowCount)
    cursorBlinks = buf.int != 0
    cursorColor = buf.int
    // Synchronized output: the rows are the frame the program left on screen
    // when its update began. Look again shortly; the hold ends with the
    // update, or on its own after a second.
    if (buf.int != 0) postDelayed(::scheduleFrame, HOLD_POLL_MS)
    cursorWide = buf.int != 0
    cursorGlyphFlags = buf.int
    cursorGlyphLength = buf.int
    for (i in 0 until 4) cursorGlyph[i] = buf.short.toInt().toChar()
    scrollbarTotal = buf.int.toLong()
    scrollbarOffset = buf.int.toLong()
    scrollbarLen = buf.int.toLong()
    syncBlinkTimer()

    recordRows(buf, rowCount, snapCols)
    syncSelectionUi()
    // The text under the pointer may have changed: the link with it.
    if (hoverLink != null) syncHoverLink(ctrl = true)
    invalidate()
  }

  // ── Selection ──

  /** Top-left / bottom-right cells of the rendered selection. */
  private data class SelectionBounds(val top: Int, val topCol: Int, val bottom: Int, val bottomCol: Int)

  private fun selectionBounds(): SelectionBounds? {
    var top = -1
    var bottom = -1
    for (row in painter.rowSelStart.indices) {
      if (painter.rowSelStart[row] >= 0) {
        if (top < 0) top = row
        bottom = row
      }
    }
    if (top < 0) return null
    return SelectionBounds(top, painter.rowSelStart[top], bottom, painter.rowSelEnd[bottom])
  }

  /** Where the start handle sits horizontally: the left edge of the first cell. */
  private val SelectionBounds.startX get() = topCol * cellWidth

  /** Where the end handle sits horizontally: the right edge of the last cell, wide or not. */
  private val SelectionBounds.endX: Float
    get() {
      val wide = bottom < painter.rowSelEndWide.size && painter.rowSelEndWide[bottom]
      return (bottomCol + 1 + if (wide) 1 else 0) * cellWidth
    }

  private fun syncSelectionUi() {
    if (!selectionMode) return
    if (selectionBounds() == null) {
      // Scrolled out of the viewport (or collapsed): drop the whole selection
      // so the UI and terminal state never disagree.
      clearSelection()
    } else {
      actionMode?.invalidateContentRect()
    }
  }

  private fun clearSelection() {
    if (handle != 0L) GhosttyVt.clearSelection(handle)
    selectionMode = false
    pressedLink = null
    draggingHandle = HANDLE_NONE
    actionMode?.finish()
    scheduleFrame()
  }

  private fun selectAllContent() {
    if (handle == 0L) return
    if (GhosttyVt.selectAll(handle)) {
      selectionMode = true
      scheduleFrame()
      actionMode?.invalidate()
    }
  }

  /** A cell's size in pixels, width rounded as the terminal is told it. */
  val cellPixels: Pair<Int, Int> get() = cellWidth.roundToInt() to cellHeight

  /** Whether programs may copy to the clipboard (OSC 52); on by default. */
  fun setClipboardWrite(allowed: Boolean) {
    if (handle != 0L) GhosttyVt.setClipboardWrite(handle, allowed)
  }

  /** Text the program copied (OSC 52), onto the phone's clipboard. */
  private fun copyFromProgram(bytes: ByteArray) {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText("terminal", String(bytes, Charsets.UTF_8)))
  }

  private fun copySelection() {
    if (handle == 0L) return
    val bytes = GhosttyVt.selectionText(handle) ?: return
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    clipboard.setPrimaryClip(
      ClipData.newPlainText("terminal", String(bytes, Charsets.UTF_8))
    )
    clearSelection()
  }

  /** The clipboard's text, or null when it has none. */
  private fun clipboardText(): String? {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return null
    val clip = clipboard.primaryClip?.takeIf { it.itemCount > 0 } ?: return null
    return clip.getItemAt(0).coerceToText(context)?.toString()?.takeIf { it.isNotEmpty() }
  }

  /** The clipboard, when it holds an image and the host takes images ([onImages]). */
  private fun clipboardImage(): ClipData? {
    if (onImages == null) return null
    val clip = context.getSystemService(ClipboardManager::class.java)?.primaryClip ?: return null
    return clip.takeIf { it.itemCount > 0 && it.description.hasMimeType("image/*") && it.getItemAt(0).uri != null }
  }

  /**
   * Paste the clipboard, confirming first when it could run commands. An
   * image goes to [onImages] (its text would only be its URI).
   */
  fun requestPaste() {
    if (handle == 0L || finished) return
    clipboardImage()?.let { clip ->
      if (selectionMode) clearSelection() else actionMode?.finish()
      ViewCompat.performReceiveContent(this, ContentInfoCompat.Builder(clip, ContentInfoCompat.SOURCE_CLIPBOARD).build())
      return
    }
    val text = clipboardText() ?: return
    val bytes = text.toByteArray(Charsets.UTF_8)
    if (GhosttyVt.isPasteSafe(bytes)) {
      sendPaste(bytes)
      return
    }
    AlertDialog.Builder(context)
      .setTitle("Paste multi-line text?")
      .setMessage("The clipboard contains newlines or control sequences that the shell may run as commands.")
      .setPositiveButton("Paste") { _, _ -> sendPaste(bytes) }
      .setNegativeButton(android.R.string.cancel, null)
      .show()
  }

  /** Pastes text the host vouches for (a path it put there), as the program takes pastes. */
  fun pasteText(text: String) {
    if (handle == 0L || finished) return
    sendPaste(text.toByteArray(Charsets.UTF_8))
  }

  private fun sendPaste(bytes: ByteArray) {
    val encoded = GhosttyVt.encodePaste(handle, bytes) ?: return
    if (selectionMode) clearSelection() else actionMode?.finish()
    sendBytes(encoded)
  }

  private fun startTerminalActionMode() {
    val current = actionMode
    if (current != null) {
      current.invalidate()
      current.invalidateContentRect()
      return
    }
    actionMode = startActionMode(
      object : ActionMode.Callback2() {
        override fun onCreateActionMode(mode: ActionMode, menu: Menu): Boolean {
          menu.add(Menu.NONE, MENU_OPEN_LINK, 0, "Open link")
          menu.add(Menu.NONE, MENU_COPY, 0, android.R.string.copy)
          menu.add(Menu.NONE, MENU_PASTE, 1, android.R.string.paste)
          menu.add(Menu.NONE, MENU_SELECT_ALL, 2, android.R.string.selectAll)
          return true
        }

        override fun onPrepareActionMode(mode: ActionMode, menu: Menu): Boolean {
          menu.findItem(MENU_OPEN_LINK)?.isVisible = pressedLink != null
          menu.findItem(MENU_COPY)?.isVisible = selectionMode
          return true
        }

        override fun onActionItemClicked(mode: ActionMode, item: MenuItem): Boolean {
          when (item.itemId) {
            MENU_OPEN_LINK -> {
              val link = pressedLink
              clearSelection()
              if (link != null) onLink?.invoke(link)
            }
            MENU_COPY -> copySelection()
            MENU_PASTE -> requestPaste()
            MENU_SELECT_ALL -> selectAllContent()
            else -> return false
          }
          return true
        }

        override fun onDestroyActionMode(mode: ActionMode) {
          actionMode = null
        }

        override fun onGetContentRect(mode: ActionMode, view: View, outRect: Rect) {
          outRect.set(selectionContentRect())
        }
      },
      ActionMode.TYPE_FLOATING
    )
  }

  private fun selectionContentRect(): Rect {
    val bounds = selectionBounds()
    return if (bounds != null) {
      val singleRow = bounds.top == bounds.bottom
      Rect(
        if (singleRow) bounds.startX.toInt() else 0,
        bounds.top * cellHeight,
        if (singleRow) bounds.endX.toInt() else width,
        (bounds.bottom + 1) * cellHeight
      )
    } else {
      Rect(
        (lastPressCol * cellWidth).toInt(),
        lastPressRow * cellHeight,
        ((lastPressCol + 1) * cellWidth).toInt(),
        (lastPressRow + 1) * cellHeight
      )
    }
  }

  /** Returns which handle (start/end) is under the touch, arming the drag anchor. */
  private fun hitTestHandle(x: Float, y: Float): Int {
    val bounds = selectionBounds() ?: return HANDLE_NONE
    val slop = HANDLE_HIT_SLOP_DP * resources.displayMetrics.density
    val distStart = hypot(x - bounds.startX, y - handleCenterY(bounds.top))
    val distEnd = hypot(x - bounds.endX, y - handleCenterY(bounds.bottom))
    // Dragging one handle keeps the other end where it is.
    return when {
      distEnd <= slop && distEnd <= distStart -> {
        dragFrom(bounds.topCol, bounds.top)
        HANDLE_END
      }
      distStart <= slop -> {
        dragFrom(bounds.bottomCol, bounds.bottom)
        HANDLE_START
      }
      else -> HANDLE_NONE
    }
  }

  /**
   * Arms a selection drag anchored at a cell. [biasRows]: how far above the
   * finger the dragged cell is (handles hang below their row).
   */
  private fun dragFrom(col: Int, row: Int, biasRows: Int = 1) {
    dragAnchorCol = col
    dragAnchorRow = row
    dragBiasRows = biasRows
  }

  /** The handle knob's center below a selection row. */
  private fun handleCenterY(row: Int) = (row + 1) * cellHeight + handleRadius

  private fun dragSelection(x: Float, y: Float) {
    if (handle == 0L) return
    val col = (x / cellWidth).toInt().coerceIn(0, cols - 1)
    val row = ((y / cellHeight).toInt() - dragBiasRows).coerceIn(0, rows - 1)
    GhosttyVt.setSelection(handle, dragAnchorCol, dragAnchorRow, col, row)
    magnifier.show(x, y)
    scheduleFrame()
  }

  private fun drawSelectionHandles(canvas: Canvas) {
    if (!selectionMode) return
    val bounds = selectionBounds() ?: return
    handlePaint.color = HANDLE_COLOR
    drawHandle(canvas, bounds.startX, bounds.top)
    drawHandle(canvas, bounds.endX, bounds.bottom)
  }

  /** A stem from inside the cell down to a knob below [row], at [x]. */
  private fun drawHandle(canvas: Canvas, x: Float, row: Int) {
    val stem = max(2f, resources.displayMetrics.density * 2)
    val rowBottom = ((row + 1) * cellHeight).toFloat()
    val knobY = handleCenterY(row)
    canvas.drawRect(x - stem / 2, rowBottom - cellHeight / 3f, x + stem / 2, knobY, handlePaint)
    canvas.drawCircle(x, knobY, handleRadius, handlePaint)
  }

  // ── Cursor blink ──

  private fun wantsBlink(): Boolean {
    return cursorBlinks && cursorVisible && isAttachedToWindow && hasWindowFocus() &&
      Settings.Global.getFloat(
        context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f
      ) != 0f
  }

  private fun syncBlinkTimer() {
    val wants = wantsBlink()
    if (wants && !blinkScheduled) {
      blinkScheduled = true
      postDelayed(blinkRunnable, BLINK_INTERVAL_MS)
    } else if (!wants && blinkScheduled) {
      blinkScheduled = false
      removeCallbacks(blinkRunnable)
      showCursor()
    }
  }

  private fun showCursor() {
    if (blinkPhaseOn) return
    blinkPhaseOn = true
    invalidate()
  }

  /** Show a solid cursor and restart the blink interval (any input/output). */
  private fun holdBlinkSolid() {
    showCursor()
    if (blinkScheduled) {
      removeCallbacks(blinkRunnable)
      postDelayed(blinkRunnable, BLINK_INTERVAL_MS)
    }
  }

  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    super.onWindowFocusChanged(hasWindowFocus)
    syncBlinkTimer()
    // Hollow without focus, solid with it.
    invalidate()
  }

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    // Shown again (a terminal kept while another was on screen): what it
    // missed meanwhile, or rows whose drawings went with the window, all
    // recorded again before its first frame.
    if (pacer.attach() && handle != 0L) {
      GhosttyVt.redraw(handle)
      renderFrame()
    } else {
      rerecordLostRows()
    }
    syncBlinkTimer()
  }

  /**
   * Records every row again from the grid when the renderer let go of some
   * rows' drawings: on leaving a window, or with the window's GPU resources
   * while the app is in the background. Rows the program doesn't rewrite
   * (a TUI's borders) would otherwise stay blank.
   */
  private fun rerecordLostRows() {
    if (handle == 0L || (0 until rowNodes.size).none { rowNodes[it]?.hasDisplayList() == false }) return
    GhosttyVt.redraw(handle)
    renderFrame()
  }

  override fun onDetachedFromWindow() {
    super.onDetachedFromWindow()
    pacer.detach()
    syncBlinkTimer()
  }

  /** Records one snapshot row record into its row's RenderNode. */
  /**
   * The snapshot's [count] row records, from [buf]'s position: rows that
   * changed recorded again, rows that only scrolled moved (their drawings
   * from where they were), the rest left as they are.
   */
  private fun recordRows(buf: ByteBuffer, count: Int, snapCols: Int) {
    movedFrom.fill(-1)
    recorded.fill(false)
    painter.keepSelections()
    repeat(count) {
      val at = buf.position()
      val row = buf.int
      val selStart = buf.int
      val selEnd = buf.int
      val textUnits = buf.int
      val from = buf.int
      if (from >= 0) {
        if (row in movedFrom.indices && from in movedFrom.indices) {
          movedFrom[row] = from
          painter.moveSelection(from, row, selStart, selEnd)
        }
        return@repeat
      }
      buf.position(at + GridPainter.ROW_HEADER_BYTES + snapCols * GridPainter.CELL_RECORD_BYTES + textUnits * 2 + if (textUnits % 2 != 0) 2 else 0)
      if (row in recorded.indices) {
        recorded[row] = true
        recordAt[row] = at
      }
    }
    val end = buf.position()
    rowNodes.frame(movedFrom, recorded) { row, node ->
      buf.position(recordAt[row])
      val canvas = node.beginRecording(gridWidthPx, gridHeightPx)
      try {
        painter.drawRow(buf, canvas, snapCols, cursorX, cursorY)
      } finally {
        node.endRecording()
      }
    }
    buf.position(end)
  }

  override fun onDraw(canvas: Canvas) {
    canvas.drawColor(painter.defaultBg)
    if (held) return
    // Rows replay only on a hardware canvas: a software one (a View.draw into
    // a bitmap) gets the background and overlays.
    if (canvas.isHardwareAccelerated) {
      // Back in front: the window's resources went in the background, the rows' drawings with them.
      rerecordLostRows()
      rowNodes.forEach { if (it.hasDisplayList()) canvas.drawRenderNode(it) }
    }
    drawHoverLink(canvas)
    // While composing, the text being composed stands where the cursor is.
    if (preedit.isNotEmpty() && cursorX >= 0 && cursorY >= 0) {
      painter.drawPreedit(canvas, preedit, cursorX, cursorY, cols)
    } else {
      drawCursor(canvas)
    }
    drawSelectionHandles(canvas)
    drawScrollIndicator(canvas)
    drawJumpToBottomChip(canvas)
  }

  private fun drawScrollIndicator(canvas: Canvas) {
    if (scrollbarTotal <= scrollbarLen) return
    val elapsed = SystemClock.uptimeMillis() - lastScrollActivityAt
    if (elapsed > SCROLLBAR_FADE_END_MS) return
    val alpha = if (elapsed <= SCROLLBAR_FADE_START_MS) {
      255
    } else {
      (255 * (SCROLLBAR_FADE_END_MS - elapsed) / (SCROLLBAR_FADE_END_MS - SCROLLBAR_FADE_START_MS)).toInt()
    }
    val density = resources.displayMetrics.density
    val barWidth = 3 * density
    val right = width - 2 * density
    val trackHeight = height.toFloat()
    val top = trackHeight * scrollbarOffset / scrollbarTotal
    val thumbHeight = max(16 * density, trackHeight * scrollbarLen / scrollbarTotal)
    scrollbarPaint.color = SCROLLBAR_COLOR
    scrollbarPaint.alpha = alpha
    canvas.drawRoundRect(
      right - barWidth, top, right, (top + thumbHeight).coerceAtMost(trackHeight),
      barWidth / 2, barWidth / 2, scrollbarPaint
    )
    if (elapsed > SCROLLBAR_FADE_START_MS) invalidate()
  }

  private fun jumpChipCenter(): Pair<Float, Float> {
    val density = resources.displayMetrics.density
    return Pair(width - 32 * density, height - 32 * density)
  }

  private fun drawJumpToBottomChip(canvas: Canvas) {
    if (!scrolledBack) return
    val density = resources.displayMetrics.density
    val (cx, cy) = jumpChipCenter()
    val radius = JUMP_CHIP_RADIUS_DP * density
    jumpChipPaint.style = Paint.Style.FILL
    jumpChipPaint.color = JUMP_CHIP_BACKGROUND
    canvas.drawCircle(cx, cy, radius, jumpChipPaint)
    // Downward chevron.
    val span = radius * 0.45f
    jumpChipArrow.reset()
    jumpChipArrow.moveTo(cx - span, cy - span * 0.5f)
    jumpChipArrow.lineTo(cx, cy + span * 0.5f)
    jumpChipArrow.lineTo(cx + span, cy - span * 0.5f)
    jumpChipPaint.style = Paint.Style.STROKE
    jumpChipPaint.strokeWidth = 2 * density
    jumpChipPaint.color = JUMP_CHIP_ARROW_COLOR
    canvas.drawPath(jumpChipArrow, jumpChipPaint)
  }

  private fun drawCursor(canvas: Canvas) {
    if (cursorX < 0 || cursorY < 0) return
    // Focus is the window's: a menu or dialog over the terminal takes it.
    val style = GridPainter.cursorShape(
      cursorStyle, cursorVisible, focused = hasWindowFocus(), blinkedOff = cursorBlinks && blinkScheduled && !blinkPhaseOn,
    ) ?: return
    painter.drawCursor(
      canvas, cursorX, cursorY, style, cursorColor, cursorWide, cursorGlyph, cursorGlyphLength, cursorGlyphFlags,
    )
  }

  // ── Mouse ──

  private val MotionEvent.isMouse
    get() = isFromSource(InputDevice.SOURCE_MOUSE) && getToolType(0) == MotionEvent.TOOL_TYPE_MOUSE

  /** The program in front tracks the mouse (clicks and motion are its, not ours). */
  private fun tracksMouse() = handle != 0L && !finished && GhosttyVt.scrollTarget(handle) == 1

  private fun sendMouse(action: Int, button: Int, x: Float, y: Float, meta: Int, anyPressed: Boolean) {
    GhosttyVt.encodeMouse(
      handle, action, button, meta, x, y, width, height, cellWidth.roundToInt(), cellHeight, anyPressed
    )?.let(::sendBytes)
  }

  /** The button a press or release is about, as the encoder numbers them. */
  private fun mouseButton(buttons: Int) = when {
    buttons and MotionEvent.BUTTON_PRIMARY != 0 -> MOUSE_LEFT
    buttons and MotionEvent.BUTTON_SECONDARY != 0 -> MOUSE_RIGHT
    buttons and MotionEvent.BUTTON_TERTIARY != 0 -> MOUSE_MIDDLE
    else -> MOUSE_NONE
  }

  /**
   * A mouse, as herdr-app takes one: Ctrl+click opens a link; a program that
   * tracks the mouse gets its presses, drags and releases (Shift keeps them
   * ours); otherwise a drag selects and copies, a double-click selects a word,
   * and a click focuses.
   */
  private fun onMouse(event: MotionEvent): Boolean {
    if (handle == 0L) return false
    val (x, y, meta) = Triple(event.x, event.y, event.metaState)
    hoverX = x
    hoverY = y
    when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> {
        requestFocus()
        if (meta and KeyEvent.META_CTRL_ON != 0 && openLinkAt(x, y)) return true
        val button = mouseButton(event.buttonState)
        if (tracksMouse() && meta and KeyEvent.META_SHIFT_ON == 0) {
          mouseButtons = mouseButtons or (1 shl button)
          sendMouse(MOUSE_PRESS, button, x, y, meta, anyPressed = true)
          return true
        }
        if (button != MOUSE_LEFT) return true
        if (selectionMode || actionMode != null) clearSelection() else GhosttyVt.clearSelection(handle)
        val col = (x / cellWidth).toInt().coerceIn(0, cols - 1)
        val row = (y / cellHeight).toInt().coerceIn(0, rows - 1)
        val cell = row * cols + col
        val now = SystemClock.uptimeMillis()
        val double = cell == lastClickCell && now - lastClickAt <= ViewConfiguration.getDoubleTapTimeout()
        lastClickAt = if (double) 0L else now
        lastClickCell = cell
        if (double) {
          if (GhosttyVt.selectWord(handle, col, row)) copyMouseSelection()
          scheduleFrame()
          return true
        }
        dragFrom(col, row, biasRows = 0)
        mouseSelecting = true
        mouseMoved = false
        scheduleFrame()
      }
      MotionEvent.ACTION_MOVE -> {
        if (mouseButtons != 0) {
          sendMouse(MOUSE_MOTION, mouseButton(event.buttonState), x, y, meta, anyPressed = true)
        } else if (mouseSelecting) {
          val col = (x / cellWidth).toInt().coerceIn(0, cols - 1)
          val row = (y / cellHeight).toInt().coerceIn(0, rows - 1)
          if (!mouseMoved && col == dragAnchorCol && row == dragAnchorRow) return true
          mouseMoved = true
          GhosttyVt.setSelection(handle, dragAnchorCol, dragAnchorRow, col, row)
          scheduleFrame()
        }
      }
      MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
        if (mouseButtons != 0) {
          // Every button the program saw go down, now up.
          for (button in MOUSE_LEFT..MOUSE_MIDDLE) {
            if (mouseButtons and (1 shl button) != 0) sendMouse(MOUSE_RELEASE, button, x, y, meta, anyPressed = false)
          }
          mouseButtons = 0
        } else if (mouseSelecting) {
          mouseSelecting = false
          // Copy on select, as herdr-app does; a plain click only focuses.
          if (mouseMoved) copyMouseSelection()
        }
      }
    }
    return true
  }

  /** The mouse's selection onto the clipboard; it stays on screen until the next click. */
  private fun copyMouseSelection() {
    val bytes = GhosttyVt.selectionText(handle) ?: return
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText("terminal", String(bytes, Charsets.UTF_8)))
  }

  override fun onGenericMotionEvent(event: MotionEvent): Boolean {
    if (event.isMouse && event.actionMasked == MotionEvent.ACTION_SCROLL && handle != 0L) {
      // A wheel notch moves three lines, as in Ghostty.
      val notches = event.getAxisValue(MotionEvent.AXIS_VSCROLL)
      if (notches != 0f) {
        lastTouchX = event.x
        lastTouchY = event.y
        scrollRows(-(notches * 3).roundToInt().let { if (it == 0) notches.sign.toInt() else it })
        return true
      }
    }
    return super.onGenericMotionEvent(event)
  }

  override fun onHoverEvent(event: MotionEvent): Boolean {
    if (!event.isMouse) return super.onHoverEvent(event)
    when (event.actionMasked) {
      MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
        hoverX = event.x
        hoverY = event.y
        // Any-motion tracking (mode 1003) reports a pointer with no button held.
        if (tracksMouse()) sendMouse(MOUSE_MOTION, MOUSE_NONE, event.x, event.y, event.metaState, anyPressed = false)
        syncHoverLink(event.metaState and KeyEvent.META_CTRL_ON != 0)
      }
      MotionEvent.ACTION_HOVER_EXIT -> {
        hoverX = Float.NaN
        hoverY = Float.NaN
        syncHoverLink(ctrl = false)
      }
    }
    return true
  }

  /** The link under the pointer while [ctrl] is held, underlined; none otherwise. */
  private fun syncHoverLink(ctrl: Boolean) {
    val span = if (ctrl && !hoverX.isNaN()) linkSpanAt(hoverX, hoverY) else null
    pointerIcon = if (span != null) PointerIcon.getSystemIcon(context, PointerIcon.TYPE_HAND) else null
    if (span == hoverLink) return
    hoverLink = span
    invalidate()
  }

  /** The cells of the link at a pixel position: its neighbors that are the same link. */
  private fun linkSpanAt(x: Float, y: Float): IntRange? {
    if (cellWidth <= 0f || cellHeight <= 0) return null
    val col = (x / cellWidth).toInt()
    val row = (y / cellHeight).toInt()
    if (col !in 0 until cols || row !in 0 until rows) return null
    if (onLink == null || handle == 0L) return null
    // One call, not one a neighboring cell: this runs on every move of the pointer.
    val span = GhosttyVt.linkSpan(handle, col, row) ?: return null
    return span[0]..span[1]
  }

  private fun drawHoverLink(canvas: Canvas) {
    val span = hoverLink ?: return
    linkPaint.color = painter.defaultFg
    val t = underlineThickness
    for (row in span.first / cols..span.last / cols) {
      val from = if (row == span.first / cols) span.first % cols else 0
      val to = if (row == span.last / cols) span.last % cols else cols - 1
      val bottom = ((row + 1) * cellHeight).toFloat()
      canvas.drawRect(from * cellWidth, bottom - t, (to + 1) * cellWidth, bottom, linkPaint)
    }
  }

  // ── Input ──

  override fun onTouchEvent(event: MotionEvent): Boolean {
    if (event.actionMasked == MotionEvent.ACTION_DOWN) scroller.abortAnimation()
    if (event.isMouse) return onMouse(event)
    scaleDetector.onTouchEvent(event)
    if (scaleDetector.isInProgress) {
      if (draggingHandle != HANDLE_NONE) {
        draggingHandle = HANDLE_NONE
        magnifier.dismiss()
      }
      // Keep the tap/long-press detector's stream consistent; its callbacks
      // are guarded on isInProgress.
      gestureDetector.onTouchEvent(event)
      return true
    }
    if (selectionMode) {
      when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
          draggingHandle = hitTestHandle(event.x, event.y)
          if (draggingHandle != HANDLE_NONE) {
            // Hide the toolbar while dragging; it comes back on release.
            actionMode?.finish()
            return true
          }
        }
        MotionEvent.ACTION_MOVE -> if (draggingHandle != HANDLE_NONE) {
          dragSelection(event.x, event.y)
          return true
        }
        MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL ->
          if (draggingHandle != HANDLE_NONE) {
            draggingHandle = HANDLE_NONE
            magnifier.dismiss()
            startTerminalActionMode()
            return true
          }
      }
    }
    return gestureDetector.onTouchEvent(event) || super.onTouchEvent(event)
  }

  override fun onCheckIsTextEditor(): Boolean = true

  override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
    outAttrs.inputType = InputType.TYPE_CLASS_TEXT
    outAttrs.imeOptions = EditorInfo.IME_ACTION_NONE or
      EditorInfo.IME_FLAG_NO_FULLSCREEN or
      EditorInfo.IME_FLAG_NO_EXTRACT_UI or
      EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
    val connection = TerminalInputConnection()
    // The keyboard may offer images (its clipboard's, screenshots, stickers) when the host takes them.
    val types = ViewCompat.getOnReceiveContentMimeTypes(this) ?: return connection
    EditorInfoCompat.setContentMimeTypes(outAttrs, types)
    return InputConnectionCompat.createWrapper(this, connection, outAttrs)
  }

  override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
    return handleKeyEvent(event) || super.onKeyDown(keyCode, event)
  }

  override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
    return handleKeyEvent(event) || super.onKeyUp(keyCode, event)
  }

  /**
   * A hardware keyboard's key that arrived while focus was elsewhere: the
   * terminal takes focus and the key. False for keys it leaves alone.
   */
  fun takeHardwareKey(event: KeyEvent): Boolean {
    if (event.isSystem) return false
    if (!hasFocus()) requestFocus()
    return handleKeyEvent(event)
  }

  /**
   * A clipboard key ([ClipboardKeys]) copies or pastes, its repeats and
   * release swallowed with it. With nothing selected to copy or nothing on
   * the clipboard to paste, it's the program's.
   */
  private fun takeClipboardKey(event: KeyEvent): Boolean {
    if (event.keyCode == clipboardKey) {
      if (event.action == KeyEvent.ACTION_UP) clipboardKey = KeyEvent.KEYCODE_UNKNOWN
      return true
    }
    if (event.action != KeyEvent.ACTION_DOWN || event.repeatCount > 0) return false
    when (ClipboardKeys.actionFor(event.keyCode, event.metaState)) {
      ClipboardKeys.COPY -> {
        if (GhosttyVt.selectionText(handle) == null) return false
        copySelection()
      }
      ClipboardKeys.PASTE -> {
        if (clipboardText() == null) return false
        requestPaste()
      }
      else -> return false
    }
    clipboardKey = event.keyCode
    return true
  }

  private fun handleKeyEvent(event: KeyEvent): Boolean {
    // Ctrl pressed or let go with the mouse over a link: underline it, or stop.
    if (event.keyCode == KeyEvent.KEYCODE_CTRL_LEFT || event.keyCode == KeyEvent.KEYCODE_CTRL_RIGHT) {
      syncHoverLink(event.metaState and KeyEvent.META_CTRL_ON != 0)
    }
    if (handle == 0L || finished) return false
    if (takeClipboardKey(event)) return true
    val action = when (event.action) {
      KeyEvent.ACTION_DOWN -> if (event.repeatCount > 0) 2 else 1
      KeyEvent.ACTION_UP -> 0
      else -> return false
    }
    // Sticky accessory-bar modifiers apply to hardware keys too.
    val stickyMeta = if (action == 1) accessoryBar?.takeStickyMetaState() ?: 0 else 0
    val metaState = event.metaState or stickyMeta
    val textMeta = metaState and (KeyEvent.META_CTRL_MASK or KeyEvent.META_META_MASK).inv()
    val unicode = event.getUnicodeChar(textMeta)
    val utf8 = if (unicode != 0 && unicode and KeyCharacterMap.COMBINING_ACCENT == 0) {
      String(Character.toChars(unicode)).toByteArray(Charsets.UTF_8)
    } else {
      null
    }
    // The modifiers the text used up (Shift for ':', AltGr for '@'). Under the
    // kitty keyboard protocol (Neovim's) the text goes as typed; otherwise
    // Shift+; would go as a modified ';' rather than ':'.
    var consumedMeta = 0
    if (utf8 != null) {
      for (mask in intArrayOf(KeyEvent.META_SHIFT_MASK, KeyEvent.META_ALT_MASK)) {
        if (textMeta and mask != 0 && event.getUnicodeChar(textMeta and mask.inv()) != unicode) {
          consumedMeta = consumedMeta or (textMeta and mask)
        }
      }
    }
    val unshifted = event.getUnicodeChar(0)
    val bytes = GhosttyVt.encodeKey(
      handle, event.keyCode, action, metaState, consumedMeta, unshifted, utf8
    ) ?: return false
    if (bytes.isEmpty()) return false
    sendBytes(bytes)
    return true
  }

  private fun setPreedit(text: String) {
    if (text == preedit) return
    preedit = text
    invalidate()
  }

  private inner class TerminalInputConnection :
    BaseInputConnection(this@GhosttyTerminalView, false) {
    override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
      setPreedit("")
      sendText(text)
      return super.commitText("", newCursorPosition)
    }

    // Composing (CJK): shown at the cursor, sent once committed. Nothing goes
    // to the program meanwhile, so it never sees half a word.
    override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
      setPreedit(text.toString())
      return true
    }

    override fun finishComposingText(): Boolean {
      val text = preedit
      setPreedit("")
      if (text.isNotEmpty()) sendText(text)
      return true
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
      repeat(beforeLength) { tap(KeyEvent.KEYCODE_DEL) }
      return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
      tap(KeyEvent.KEYCODE_ENTER)
      return true
    }

    private fun tap(keyCode: Int) {
      sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, keyCode))
      sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, keyCode))
    }

    override fun sendKeyEvent(event: KeyEvent): Boolean =
      handleKeyEvent(event) || super.sendKeyEvent(event)
  }

  private companion object {
    // GhosttyVt.encodeMouse's actions and buttons.
    const val MOUSE_PRESS = 0
    const val MOUSE_RELEASE = 1
    const val MOUSE_MOTION = 2
    const val MOUSE_NONE = 0
    const val MOUSE_LEFT = 1
    const val MOUSE_RIGHT = 2
    const val MOUSE_MIDDLE = 3
    // Same step as the iOS pinch-zoom (UITerminalView).
    const val PINCH_SCALE_STEP = 0.1f
    const val INITIAL_COLS = 80
    const val INITIAL_ROWS = 24
    const val MAX_SCROLLBACK = 10_000L
    const val BLINK_INTERVAL_MS = 600L

    const val HANDLE_NONE = 0
    const val HANDLE_START = 1
    const val HANDLE_END = 2
    const val HANDLE_RADIUS_DP = 8f
    const val HANDLE_HIT_SLOP_DP = 24f
    const val HANDLE_COLOR = 0xFF4C8DF5.toInt()

    const val MENU_OPEN_LINK = 4
    const val MENU_COPY = 1
    const val MENU_PASTE = 2
    const val MENU_SELECT_ALL = 3

    const val SCROLLBAR_FADE_START_MS = 800L
    const val SCROLLBAR_FADE_END_MS = 1400L
    const val SCROLLBAR_COLOR = 0xB3AAAAAA.toInt()
    const val JUMP_CHIP_RADIUS_DP = 18f
    const val JUMP_CHIP_BACKGROUND = 0xCC2A2A2E.toInt()
    const val JUMP_CHIP_ARROW_COLOR = 0xFFE0E0E0.toInt()

    const val HOLD_POLL_MS = 100L
  }
}
