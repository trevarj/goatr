package dev.herdr.terminal

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Typeface
import android.util.SparseIntArray
import java.nio.ByteBuffer
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * Draws the grid: the cell metrics of the terminal's font, one snapshot row
 * record at a time ([drawRow]), and the cursor ([drawCursor]). It reads the
 * records jni.zig writes and knows nothing of views, so tests draw with it
 * onto a bitmap.
 */
internal class GridPainter(private val context: Context) {
  private val resources get() = context.resources

  // ── Cell metrics ──
  var fontSizeDp = FONT_SIZE_DP
    private set
  private var typeface: Typeface? = null
  var ligatures = false
    private set
  private val runText = StringBuilder()
  val textPaint = newTextPaint(Typeface.MONOSPACE)
  private val boldPaint = newTextPaint(Typeface.create(Typeface.MONOSPACE, Typeface.BOLD))
  private val italicPaint = newTextPaint(Typeface.create(Typeface.MONOSPACE, Typeface.ITALIC))
  private val boldItalicPaint =
    newTextPaint(Typeface.create(Typeface.MONOSPACE, Typeface.BOLD_ITALIC))
  private val stylePaints = listOf(textPaint, boldPaint, italicPaint, boldItalicPaint)

  // Nerd Font private-use glyphs (powerline, devicons, …) are absent from the
  // system fallback chain; route those cells to the bundled symbols-only font.
  private val symbolsPaint: Paint? = assetPaint(SYMBOLS_FONT_ASSET)

  // Symbols neither the terminal font nor Android's fallback chain draw (⏵,
  // many arrows and shapes): a subset of Noto Sans Symbols 2. Whether the
  // cell's own paint can, by code point, until the typeface changes.
  private val fallbackPaint: Paint? = assetPaint(FALLBACK_FONT_ASSET)
  // 0 not asked yet, else YES or NO: primitive, as it's asked for every cell
  // past ASCII on every row recorded.
  private val drawable = SparseIntArray()
  private val fillPaint = Paint()
  private val cursorPaint = Paint()
  private val sprites = Sprites()
  var cellWidth = 0f
    private set
  var cellHeight = 0
    private set
  private var baseline = 0f
  // Box drawing's line thickness, in whole pixels: the font's underline's, as ghostty's.
  private var boxThickness = 1
  val underlineThickness = max(1f, resources.displayMetrics.density)
  // Double, curly, dotted and dashed underlines, and colored ones.
  private val decorationPaint = Paint(Paint.ANTI_ALIAS_FLAG)
  private val curlyPath = Path()

  // ── One row's cells, as last read ──
  private var rowChars = CharArray(0)
  private var cellFg = IntArray(0)
  private var cellBg = IntArray(0)
  private var cellTextOff = IntArray(0)
  private var cellTextLen = IntArray(0)
  private var cellFlags = IntArray(0)
  private var cellUnderline = IntArray(0)
  private var cellUnderlineColor = IntArray(0)

  // Each row's selected columns, as its last record had them (-1: none).
  var rowSelStart = IntArray(0)
    private set
  var rowSelEnd = IntArray(0)
    private set
  var rowSelEndWide = BooleanArray(0)
    private set

  var defaultBg = 0xFF000000.toInt()
  var defaultFg = 0xFFFFFFFF.toInt()
  // Themed selection colors; when unset, selected cells keep the classic
  // fg/bg swap.
  var selectionBg: Int? = null
  var selectionFg: Int? = null

  init {
    applyFeatures()
    measureCell()
  }

  private fun assetPaint(name: String): Paint? = try {
    newTextPaint(Typeface.createFromAsset(context.assets, name))
  } catch (e: RuntimeException) {
    null
  }

  private fun newTextPaint(typeface: Typeface) = Paint(Paint.ANTI_ALIAS_FLAG).apply {
    this.typeface = typeface
    textSize = fontSizeDp * resources.displayMetrics.density
  }

  /** The font size in dp, clamped to 4–64; false when it didn't change. */
  fun setFontSize(dp: Float): Boolean {
    val clamped = dp.coerceIn(MIN_FONT_SIZE_DP, MAX_FONT_SIZE_DP)
    if (clamped == fontSizeDp) return false
    fontSizeDp = clamped
    val px = clamped * resources.displayMetrics.density
    for (paint in stylePaints) paint.textSize = px
    symbolsPaint?.textSize = px
    fallbackPaint?.textSize = px
    measureCell()
    return true
  }

  private fun measureCell() {
    val metrics = textPaint.fontMetrics
    cellWidth = textPaint.measureText("M")
    cellHeight = ceil(metrics.descent - metrics.ascent).toInt()
    baseline = -metrics.ascent
    boxThickness = max(1, textPaint.underlineThickness.roundToInt())
  }

  /**
   * The terminal font; null is the system monospace. Bold uses the font's
   * `wght` axis when it is a variable font, else Android's synthetic styles.
   * False when it didn't change.
   */
  fun setTypeface(face: Typeface?): Boolean {
    if (face == typeface) return false
    typeface = face
    drawable.clear()
    val base = face ?: Typeface.MONOSPACE
    styled(textPaint, base, bold = false, italic = false)
    styled(boldPaint, base, bold = true, italic = false)
    styled(italicPaint, base, bold = false, italic = true)
    styled(boldItalicPaint, base, bold = true, italic = true)
    measureCell()
    return true
  }

  private fun styled(paint: Paint, base: Typeface, bold: Boolean, italic: Boolean) {
    paint.typeface = base
    paint.textSkewX = 0f
    // Variable fonts (JetBrains Mono, Geist Mono) carry their weight as an axis.
    if (paint.setFontVariationSettings(if (bold) "'wght' 700" else "'wght' 400")) {
      if (italic) paint.textSkewX = -0.2f
      return
    }
    paint.setFontVariationSettings(null)
    val style = when {
      bold && italic -> Typeface.BOLD_ITALIC
      bold -> Typeface.BOLD
      italic -> Typeface.ITALIC
      else -> Typeface.NORMAL
    }
    paint.typeface = Typeface.create(base, style)
  }

  /**
   * Programming ligatures (`->`, `!=`, `===`): runs of cells are drawn as one
   * string either way, and with these on the font joins their ASCII. Off
   * disables liga/calt. False when it didn't change.
   */
  fun setLigatures(enabled: Boolean): Boolean {
    if (enabled == ligatures) return false
    ligatures = enabled
    applyFeatures()
    return true
  }

  private fun applyFeatures() {
    // No kerning either way: a run's characters stay on their cells.
    val features = if (ligatures) "'liga' 1, 'calt' 1, 'kern' 0" else "'liga' 0, 'calt' 0, 'kern' 0"
    for (paint in stylePaints) paint.fontFeatureSettings = features
  }

  /** Room for a grid of [cols] × [rows]: the per-cell arrays and row selections. */
  fun allocate(cols: Int, rows: Int) {
    rowChars = CharArray(cols * MAX_CELL_TEXT_UNITS)
    cellFg = IntArray(cols)
    cellBg = IntArray(cols)
    cellTextOff = IntArray(cols)
    cellTextLen = IntArray(cols)
    cellFlags = IntArray(cols)
    cellUnderline = IntArray(cols)
    cellUnderlineColor = IntArray(cols)
    rowSelStart = IntArray(rows) { -1 }
    rowSelEnd = IntArray(rows) { -1 }
    rowSelEndWide = BooleanArray(rows)
  }

  // The rows' selections as the frame began, for rows moved from them.
  private var keptSelEndWide = BooleanArray(0)

  /** Before a frame's records: the rows' selections as they are. */
  fun keepSelections() {
    if (keptSelEndWide.size != rowSelEndWide.size) keptSelEndWide = BooleanArray(rowSelEndWide.size)
    rowSelEndWide.copyInto(keptSelEndWide)
  }

  /** Row [to] shows what row [from] did (it scrolled), selected [selStart]–[selEnd]. */
  fun moveSelection(from: Int, to: Int, selStart: Int, selEnd: Int) {
    if (to >= rowSelStart.size || from >= keptSelEndWide.size) return
    rowSelStart[to] = selStart
    rowSelEnd[to] = selEnd
    rowSelEndWide[to] = keptSelEndWide[from]
  }

  /**
   * Whether a cell's character can be in a run: one the terminal font draws
   * a cell wide (printable ASCII; Latin-1 and Latin Extended-A too without
   * ligatures, which only join ASCII).
   */
  private fun runs(col: Int): Boolean {
    val flags = cellFlags[col]
    if (cellTextLen[col] != 1 || flags and (FLAG_SPACER or FLAG_INVISIBLE or FLAG_WIDE) != 0) return false
    val c = rowChars[cellTextOff[col]].code
    return c in 0x21..0x7E || !ligatures && c in 0xA1..0x17F && canDraw(c)
  }

  /** A cell with nothing to draw but its background: a run goes on over it. */
  private fun blank(col: Int): Boolean {
    if (cellFlags[col] and (FLAG_SPACER or FLAG_WIDE) != 0) return false
    val len = cellTextLen[col]
    return len == 0 || len == 1 && rowChars[cellTextOff[col]] == ' '
  }

  /**
   * Reads one row record at [buf]'s position (leaving it at the next) and
   * draws it on [canvas]; null only reads it (a row outside the grid). The
   * cursor at ([cursorX], [cursorY]) ends a ligature run.
   */
  fun drawRow(buf: ByteBuffer, canvas: Canvas?, snapCols: Int, cursorX: Int = -1, cursorY: Int = -1) {
    val rowIndex = buf.int
    val selStart = buf.int
    val selEnd = buf.int
    val textUnits = buf.int
    buf.int // moved from: -1 (a record with its cells)
    if (rowIndex < rowSelStart.size) {
      rowSelStart[rowIndex] = selStart
      rowSelEnd[rowIndex] = selEnd
      rowSelEndWide[rowIndex] = false
    }
    val cellsPos = buf.position()
    val textPos = cellsPos + snapCols * CELL_RECORD_BYTES

    buf.position(textPos)
    if (textUnits > rowChars.size) rowChars = CharArray(textUnits)
    for (i in 0 until textUnits) rowChars[i] = buf.char
    val nextRecord = textPos + textUnits * 2 + if (textUnits % 2 != 0) 2 else 0

    buf.position(cellsPos)
    val colCount = minOf(snapCols, cellFg.size)
    for (col in 0 until snapCols) {
      val fg = buf.int
      val bg = buf.int
      val textOff = buf.short.toInt() and 0xFFFF
      val textLen = buf.short.toInt() and 0xFFFF
      val flags = buf.short.toInt() and 0xFFFF
      val underline = buf.short.toInt() and 0xFFFF
      val underlineColor = buf.int
      if (col >= colCount) continue
      if (col == selEnd && flags and FLAG_WIDE != 0 && rowIndex < rowSelEndWide.size) {
        rowSelEndWide[rowIndex] = true
      }
      // A wide character's right half is selected with the character.
      val at = if (flags and FLAG_SPACER != 0 && col > 0) col - 1 else col
      val selected = selStart >= 0 && at >= selStart && at <= selEnd
      var effFg = if (flags and FLAG_FG_DEFAULT != 0) defaultFg else fg
      var effBg = if (flags and FLAG_BG_NONE != 0) defaultBg else bg
      if (flags and FLAG_INVERSE != 0) effFg = effBg.also { effBg = effFg }
      if (selected) {
        val themedBg = selectionBg
        if (themedBg != null) {
          // The text keeps its color (as herdr-app's), unless it would
          // vanish into the selection's.
          effBg = themedBg
          effFg = selectionFg ?: readableOn(themedBg, effFg)
        } else {
          // No themed selection colors: the default ones swapped, as ghostty's.
          effBg = defaultFg
          effFg = defaultBg
        }
      }
      cellFg[col] = effFg
      cellBg[col] = effBg
      cellTextOff[col] = textOff
      cellTextLen[col] = textLen
      cellFlags[col] = flags
      cellUnderline[col] = underline
      cellUnderlineColor[col] = underlineColor
    }
    buf.position(nextRecord)
    if (canvas == null) return

    val top = (rowIndex * cellHeight).toFloat()
    val bottom = top + cellHeight

    // Background pass: row clear, then backgrounds, a rect per run of one color.
    fillPaint.color = defaultBg
    canvas.drawRect(0f, top, canvas.width.toFloat(), bottom, fillPaint)
    // (A wide character's right half is in the character's.)
    fun bgAt(col: Int) =
      if (col > 0 && cellFlags[col] and FLAG_SPACER != 0 && cellFlags[col - 1] and FLAG_WIDE != 0) cellBg[col - 1] else cellBg[col]
    var from = 0
    while (from < colCount) {
      val bg = bgAt(from)
      var to = from + 1
      while (to < colCount && bgAt(to) == bg) to++
      if (bg != defaultBg) {
        fillPaint.color = bg
        canvas.drawRect(from * cellWidth, top, to * cellWidth, bottom, fillPaint)
      }
      from = to
    }

    // Text pass.
    var col = 0
    while (col < colCount) {
      val flags = cellFlags[col]
      val len = cellTextLen[col]
      // Nothing to draw (but a space's underline or strikethrough).
      if (len == 0 || flags and FLAG_SPACER != 0 || flags and FLAG_INVISIBLE != 0 ||
        blank(col) && flags and DECORATIONS == 0
      ) {
        col++
        continue
      }
      val codepoint = Character.codePointAt(rowChars, cellTextOff[col])
      val paint = paintFor(codepoint, flags)
      // A run: the cells after it that look the same (blanks among them),
      // drawn as one string, as herdr-app shapes them; monospace advances
      // keep each character on its cell. Not over a change of background (a
      // selection's edge) or the cursor, which draws its own cell's character.
      var end = col + 1
      if (runs(col)) {
        val cursor = if (rowIndex == cursorY) cursorX else -1
        while (end < colCount && (runs(end) || blank(end)) && cellFg[end] == cellFg[col] &&
          cellBg[end] == cellBg[col] && cellFlags[end] and STYLE_FLAGS == flags and STYLE_FLAGS &&
          end != cursor && col != cursor
        ) end++
        while (end > col + 1 && blank(end - 1)) end--
      }
      paint.color = cellFg[col]
      if (flags and FLAG_FAINT != 0) paint.alpha = 160
      val x = col * cellWidth
      if (end == col + 1) {
        if (!drawSprite(canvas, codepoint, col, top, paint)) {
          canvas.drawText(rowChars, cellTextOff[col], len, x, top + baseline, paint)
        }
      } else {
        runText.setLength(0)
        for (c in col until end) runText.append(if (cellTextLen[c] == 0) ' ' else rowChars[cellTextOff[c]])
        canvas.drawText(runText, 0, runText.length, x, top + baseline, paint)
      }
      for (c in col until end) {
        val cx = c * cellWidth
        val span = cellSpan(cellFlags[c])
        if (cellFlags[c] and FLAG_UNDERLINE != 0) {
          drawUnderline(canvas, cellUnderline[c], cellUnderlineColor[c], cx, span, bottom, paint)
        }
        if (cellFlags[c] and FLAG_OVERLINE != 0) {
          canvas.drawRect(cx, top, cx + span, top + underlineThickness, paint)
        }
        if (cellFlags[c] and FLAG_STRIKETHROUGH != 0) {
          val mid = top + cellHeight / 2f
          canvas.drawRect(cx, mid - underlineThickness / 2, cx + span, mid + underlineThickness / 2, paint)
        }
      }
      paint.alpha = 255
      col = end
    }
  }

  /**
   * [color] on [background] when it stands out from it; else whichever of the
   * default colors stands out more.
   */
  private fun readableOn(background: Int, color: Int): Int {
    if (contrast(color, background) >= MIN_SELECTED_CONTRAST) return color
    return if (contrast(defaultFg, background) >= contrast(defaultBg, background)) defaultFg else defaultBg
  }

  /** The paint a cell's character is drawn with: its font, by the character and the cell's style. */
  private fun paintFor(codepoint: Int, flags: Int): Paint = when {
    symbolsPaint != null &&
      (codepoint in PUA_BMP_FIRST..PUA_BMP_LAST || codepoint >= PUA_SUPPLEMENTARY_FIRST) ->
      symbolsPaint
    fallbackPaint != null && codepoint >= 0x80 && !canDraw(codepoint) -> fallbackPaint
    flags and FLAG_BOLD != 0 && flags and FLAG_ITALIC != 0 -> boldItalicPaint
    flags and FLAG_BOLD != 0 -> boldPaint
    flags and FLAG_ITALIC != 0 -> italicPaint
    else -> textPaint
  }

  /**
   * [codepoint] as a shape on cell [col]'s pixels ([Sprites]: box drawing,
   * blocks, braille, powerline, …); false for characters drawn from a font.
   */
  private fun drawSprite(canvas: Canvas, codepoint: Int, col: Int, top: Float, paint: Paint): Boolean {
    if (!Sprites.covers(codepoint)) return false
    val left = (col * cellWidth).roundToInt()
    val right = ((col + 1) * cellWidth).roundToInt()
    return sprites.draw(canvas, codepoint, left, top.toInt(), right - left, cellHeight, boxThickness, paint)
  }

  /**
   * One cell's underline in its SGR 4 style, in its SGR 58 color or else the
   * text's ([paint], faint included). A plain one is the text-colored bar.
   */
  private fun drawUnderline(canvas: Canvas, style: Int, color: Int, x: Float, span: Float, bottom: Float, paint: Paint) {
    val t = underlineThickness
    if (style <= UNDERLINE_SINGLE && color == 0) {
      canvas.drawRect(x, bottom - t, x + span, bottom, paint)
      return
    }
    val p = decorationPaint
    p.color = if (color != 0) color else paint.color
    p.alpha = paint.alpha
    p.style = Paint.Style.FILL
    when (style) {
      UNDERLINE_DOUBLE -> {
        canvas.drawRect(x, bottom - t, x + span, bottom, p)
        canvas.drawRect(x, bottom - 3 * t, x + span, bottom - 2 * t, p)
      }
      UNDERLINE_CURLY -> {
        // One wave per cell, peak to trough within the bottom of the cell.
        val amplitude = t * 1.5f
        val mid = bottom - amplitude - t / 2
        curlyPath.reset()
        curlyPath.moveTo(x, mid)
        var at = x
        val half = cellWidth / 2
        var up = true
        while (at < x + span - 0.01f) {
          val end = minOf(at + half, x + span)
          curlyPath.quadTo((at + end) / 2, if (up) mid - amplitude * 2 else mid + amplitude * 2, end, mid)
          at = end
          up = !up
        }
        p.style = Paint.Style.STROKE
        p.strokeWidth = t
        canvas.drawPath(curlyPath, p)
      }
      UNDERLINE_DOTTED -> {
        var at = x + t / 2
        while (at < x + span) {
          canvas.drawCircle(at, bottom - t / 2, t / 2, p)
          at += t * 2
        }
      }
      UNDERLINE_DASHED -> {
        val dash = span / 3
        canvas.drawRect(x, bottom - t, x + dash, bottom, p)
        canvas.drawRect(x + dash * 2, bottom - t, x + span, bottom, p)
      }
      else -> canvas.drawRect(x, bottom - t, x + span, bottom, p)
    }
  }

  /** Whether the terminal font, or a font Android falls back to, has [codepoint]. */
  private fun canDraw(codepoint: Int): Boolean {
    val known = drawable.get(codepoint)
    if (known != 0) return known == YES
    val has = textPaint.hasGlyph(String(Character.toChars(codepoint)))
    drawable.put(codepoint, if (has) YES else NO)
    return has
  }

  /** A cell's width in px: two columns for a wide character. */
  private fun cellSpan(flags: Int) = if (flags and FLAG_WIDE != 0) cellWidth * 2 else cellWidth

  /**
   * The cursor at cell ([x], [y]) in [style], in [color] (0: the default
   * foreground); [wide] on a wide character, over both its cells. A block is
   * solid, the cell's character ([glyph], [glyphLength] units, its cell's
   * [glyphFlags]) drawn again over it in the background color, as ghostty
   * draws it.
   */
  fun drawCursor(
    canvas: Canvas,
    x: Int,
    y: Int,
    style: Int,
    color: Int,
    wide: Boolean = false,
    glyph: CharArray? = null,
    glyphLength: Int = 0,
    glyphFlags: Int = 0,
  ) {
    val left = x * cellWidth
    val span = if (wide) cellWidth * 2 else cellWidth
    val top = (y * cellHeight).toFloat()
    val bottom = top + cellHeight
    cursorPaint.color = if (color != 0) color else defaultFg
    when (style) {
      CURSOR_STYLE_BAR -> {
        cursorPaint.style = Paint.Style.FILL
        canvas.drawRect(left, top, left + underlineThickness * 2, bottom, cursorPaint)
      }
      CURSOR_STYLE_UNDERLINE -> {
        cursorPaint.style = Paint.Style.FILL
        canvas.drawRect(left, bottom - underlineThickness * 2, left + span, bottom, cursorPaint)
      }
      CURSOR_STYLE_BLOCK_HOLLOW -> {
        cursorPaint.style = Paint.Style.STROKE
        cursorPaint.strokeWidth = underlineThickness
        canvas.drawRect(left, top, left + span, bottom, cursorPaint)
      }
      else -> {
        cursorPaint.style = Paint.Style.FILL
        canvas.drawRect(left, top, left + span, bottom, cursorPaint)
        if (glyph == null || glyphLength == 0) return
        val codepoint = Character.codePointAt(glyph, 0)
        val paint = paintFor(codepoint, glyphFlags)
        paint.color = defaultBg
        if (!drawSprite(canvas, codepoint, x, top, paint)) {
          canvas.drawText(glyph, 0, glyphLength, left, top + baseline, paint)
        }
      }
    }
  }

  /**
   * Text being composed in an input method (CJK), at cell ([x], [y]) over
   * the grid: each character in a cell of its own (two when wide), on the
   * background, underlined, in the default foreground, as ghostty draws it.
   * Characters past the row's end ([cols]) aren't drawn.
   */
  fun drawPreedit(canvas: Canvas, text: CharSequence, x: Int, y: Int, cols: Int) {
    val top = (y * cellHeight).toFloat()
    val bottom = top + cellHeight
    var col = x
    var i = 0
    while (i < text.length && col < cols) {
      val codepoint = Character.codePointAt(text, i)
      val units = Character.charCount(codepoint)
      val paint = paintFor(codepoint, 0)
      paint.color = defaultFg
      val wide = paint.measureText(text, i, i + units) > cellWidth * 1.5f && col + 1 < cols
      val span = if (wide) cellWidth * 2 else cellWidth
      val left = col * cellWidth
      fillPaint.color = defaultBg
      canvas.drawRect(left, top, left + span, bottom, fillPaint)
      canvas.drawText(text, i, i + units, left, top + baseline, paint)
      canvas.drawRect(left, bottom - underlineThickness, left + span, bottom, paint)
      col += if (wide) 2 else 1
      i += units
    }
  }

  companion object {
    /**
     * How the cursor shows, as ghostty decides it ([style] the program's), or
     * null when it doesn't: unfocused it's a hollow box, shown even mid-blink.
     */
    fun cursorShape(style: Int, visible: Boolean, focused: Boolean, blinkedOff: Boolean): Int? = when {
      !visible -> null
      !focused -> CURSOR_STYLE_BLOCK_HOLLOW
      blinkedOff -> null
      else -> style
    }

    /** WCAG's contrast ratio of two colors (1–21). */
    fun contrast(a: Int, b: Int): Double {
      val (light, dark) = listOf(luminance(a), luminance(b)).sortedDescending()
      return (light + 0.05) / (dark + 0.05)
    }

    private fun luminance(color: Int): Double {
      fun channel(c: Int): Double {
        val v = c / 255.0
        return if (v <= 0.03928) v / 12.92 else Math.pow((v + 0.055) / 1.055, 2.4)
      }
      return 0.2126 * channel(color shr 16 and 0xFF) + 0.7152 * channel(color shr 8 and 0xFF) +
        0.0722 * channel(color and 0xFF)
    }

    private const val YES = 1
    private const val NO = 2

    // Below this, selected text takes a default color.
    private const val MIN_SELECTED_CONTRAST = 1.5

    const val FONT_SIZE_DP = 14f
    // Same bounds and step as the iOS pinch-zoom (UITerminalView).
    const val MIN_FONT_SIZE_DP = 4f
    const val MAX_FONT_SIZE_DP = 64f
    const val SYMBOLS_FONT_ASSET = "SymbolsNerdFontMono-Regular.ttf"
    const val FALLBACK_FONT_ASSET = "NotoSansSymbols2-Regular.ttf"

    // Nerd Font glyph ranges: BMP private-use area plus the supplementary
    // private-use planes (material icons live at U+F0000+ since v3).
    const val PUA_BMP_FIRST = 0xE000
    const val PUA_BMP_LAST = 0xF8FF
    const val PUA_SUPPLEMENTARY_FIRST = 0xF0000

    // The snapshot's layout (jni.zig).
    const val HEADER_BYTES = 88
    const val ROW_HEADER_BYTES = 20
    const val CELL_RECORD_BYTES = 20
    const val MAX_CELL_TEXT_UNITS = 32
    const val UNDERLINE_SINGLE = 1
    const val UNDERLINE_DOUBLE = 2
    const val UNDERLINE_CURLY = 3
    const val UNDERLINE_DOTTED = 4
    const val UNDERLINE_DASHED = 5

    const val CURSOR_STYLE_BAR = 0
    const val CURSOR_STYLE_BLOCK = 1
    const val CURSOR_STYLE_UNDERLINE = 2
    const val CURSOR_STYLE_BLOCK_HOLLOW = 3

    const val FLAG_BOLD = 1 shl 0
    const val FLAG_ITALIC = 1 shl 1
    const val FLAG_FAINT = 1 shl 2
    const val FLAG_UNDERLINE = 1 shl 3
    const val FLAG_STRIKETHROUGH = 1 shl 4
    const val FLAG_INVERSE = 1 shl 5
    const val FLAG_INVISIBLE = 1 shl 6
    const val FLAG_WIDE = 1 shl 7
    const val FLAG_SPACER = 1 shl 8
    const val FLAG_FG_DEFAULT = 1 shl 9
    const val FLAG_BG_NONE = 1 shl 10
    const val FLAG_OVERLINE = 1 shl 11
    const val STYLE_FLAGS = FLAG_BOLD or FLAG_ITALIC or FLAG_FAINT
    private const val DECORATIONS = FLAG_UNDERLINE or FLAG_STRIKETHROUGH or FLAG_OVERLINE
  }
}
