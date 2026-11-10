package dev.herdr.terminal

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Typeface
import org.robolectric.RuntimeEnvironment
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.ceil
import kotlin.math.floor

/** One cell of a row record, as jni.zig writes it. */
data class Cell(
  val text: String = "",
  val fg: Int = 0,
  val bg: Int = 0,
  val flags: Int = 0,
  val underline: Int = 0,
  val underlineColor: Int = 0,
)

/** A row record: `cells` padded with blanks to `cols`. */
fun rowRecord(row: Int, cols: Int, cells: List<Cell>, selStart: Int = -1, selEnd: Int = -1): ByteBuffer {
  val all = cells + List(maxOf(0, cols - cells.size)) { Cell() }
  val text = StringBuilder()
  val offsets = all.map { cell -> text.length.also { text.append(cell.text) } }
  val units = text.length
  val size = GridPainter.ROW_HEADER_BYTES + cols * GridPainter.CELL_RECORD_BYTES + units * 2 + 4
  val buf = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
  buf.putInt(row).putInt(selStart).putInt(selEnd).putInt(units).putInt(-1)
  for ((i, cell) in all.withIndex()) {
    var flags = cell.flags
    if (cell.fg == 0) flags = flags or GridPainter.FLAG_FG_DEFAULT
    if (cell.bg == 0) flags = flags or GridPainter.FLAG_BG_NONE
    buf.putInt(cell.fg).putInt(cell.bg)
    buf.putShort(offsets[i].toShort()).putShort(cell.text.length.toShort())
    buf.putShort(flags.toShort()).putShort(cell.underline.toShort()).putInt(cell.underlineColor)
  }
  for (c in text) buf.putChar(c)
  if (units % 2 != 0) buf.putShort(0)
  buf.flip()
  return buf
}

/** Cells for plain text, one character a cell. */
fun cells(text: String, fg: Int = 0, bg: Int = 0, flags: Int = 0): List<Cell> =
  text.map { Cell(it.toString(), fg, bg, flags) }

/** The app's JetBrains Mono (tests run in the terminal module's directory). */
val jetBrainsMono: Typeface = Typeface.createFromFile(File("../app/src/main/res/font/jetbrains_mono_regular.ttf"))

/**
 * A painter in the app's JetBrains Mono, with a grid to draw on. Large: the
 * tests' density is 1, where 14 dp would make cells 8 px wide.
 */
internal class Grid(val cols: Int = 20, val rows: Int = 4) {
  val painter = GridPainter(RuntimeEnvironment.getApplication()).apply {
    setTypeface(jetBrainsMono)
    setFontSize(32f)
    allocate(cols, rows)
    defaultBg = BG
    defaultFg = FG
  }
  val bitmap: Bitmap = Bitmap.createBitmap(
    ceil(cols * painter.cellWidth).toInt(), rows * painter.cellHeight, Bitmap.Config.ARGB_8888,
  )
  val canvas = Canvas(bitmap).apply { drawColor(BG) }

  fun draw(row: Int, cells: List<Cell>, selStart: Int = -1, selEnd: Int = -1) {
    painter.drawRow(rowRecord(row, cols, cells, selStart, selEnd), canvas, cols)
  }

  /** The pixels of a cell (wide: two), inset by [inset] px. */
  fun cellPixels(col: Int, row: Int, span: Int = 1, inset: Int = 0): List<Int> {
    val left = ceil(col * painter.cellWidth).toInt() + inset
    val right = floor((col + span) * painter.cellWidth).toInt() - inset
    val top = row * painter.cellHeight + inset
    val bottom = (row + 1) * painter.cellHeight - inset
    return buildList { for (y in top until bottom) for (x in left until right) add(bitmap.getPixel(x, y)) }
  }

  companion object {
    val BG = Color.rgb(0x10, 0x10, 0x10)
    val FG = Color.rgb(0xE0, 0xE0, 0xE0)
  }
}

/** How far apart two colors are (the largest channel difference). */
fun distance(a: Int, b: Int) = maxOf(
  kotlin.math.abs(Color.red(a) - Color.red(b)),
  kotlin.math.abs(Color.green(a) - Color.green(b)),
  kotlin.math.abs(Color.blue(a) - Color.blue(b)),
)

/** The share of [pixels] within [tolerance] of [color]. */
fun share(pixels: List<Int>, color: Int, tolerance: Int = 24) =
  pixels.count { distance(it, color) <= tolerance }.toDouble() / pixels.size

/** A bitmap's canvas that also keeps the text drawn on it, a string per call. */
class TextCanvas(bitmap: Bitmap) : Canvas(bitmap) {
  val texts = mutableListOf<String>()

  override fun drawText(text: CharArray, index: Int, count: Int, x: Float, y: Float, paint: android.graphics.Paint) {
    texts += String(text, index, count)
    super.drawText(text, index, count, x, y, paint)
  }

  override fun drawText(text: CharSequence, start: Int, end: Int, x: Float, y: Float, paint: android.graphics.Paint) {
    texts += text.subSequence(start, end).toString()
    super.drawText(text, start, end, x, y, paint)
  }

  override fun drawText(text: String, x: Float, y: Float, paint: android.graphics.Paint) {
    texts += text
    super.drawText(text, x, y, paint)
  }
}
