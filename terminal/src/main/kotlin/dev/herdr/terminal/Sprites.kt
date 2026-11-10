package dev.herdr.terminal

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sqrt

/**
 * Characters drawn as shapes on the cell's own pixels rather than from a font,
 * as ghostty draws them (its font/sprite/draw, MIT): a font's glyphs for these
 * leave gaps between cells or don't fill them, which splits borders, graphs
 * and pictures made of them. Each is drawn in [Paint]'s color.
 *
 * - Box drawing, U+2500–257F (lines light, heavy and double, dashes, arcs,
 *   diagonals)
 * - Block elements, U+2580–259F
 * - Geometric shapes: corner triangles, U+25E2–25E5, U+25F8–25FA, U+25FF
 * - Braille, U+2800–28FF
 * - Powerline, U+E0B0–E0BF, U+E0D2, U+E0D4
 * - Sextants, U+1FB00–1FB3B, and octants, U+1CD00–1CDE5
 */
internal class Sprites {
  private val path = Path()
  private val stroke = Paint()

  // The cell being drawn: its pixels, and the line thickness (ghostty's box_thickness).
  private var canvas: Canvas? = null
  private var paint = Paint()
  private var left = 0
  private var top = 0
  private var width = 0
  private var height = 0
  private var thickness = 1

  /**
   * Draws [codepoint] over the cell from ([left], [top]), [width] × [height]
   * pixels, with lines [thickness] pixels thick; false when it isn't one
   * drawn here.
   */
  fun draw(canvas: Canvas, codepoint: Int, left: Int, top: Int, width: Int, height: Int, thickness: Int, paint: Paint): Boolean {
    if (!covers(codepoint)) return false
    this.canvas = canvas
    this.paint = paint
    this.left = left
    this.top = top
    this.width = width
    this.height = height
    this.thickness = max(1, thickness)
    when (codepoint) {
      in 0x2500..0x257F -> box(codepoint)
      in 0x2580..0x259F -> block(codepoint)
      in 0x25E2..0x25E5, in 0x25F8..0x25FA, 0x25FF -> geometric(codepoint)
      in 0x2800..0x28FF -> braille(codepoint)
      in 0xE0B0..0xE0BF, 0xE0D2, 0xE0D4 -> powerline(codepoint)
      in 0x1FB00..0x1FB3B -> sextant(codepoint)
      in 0x1CD00..0x1CDE5 -> octant(codepoint)
    }
    this.canvas = null
    return true
  }

  // ── Primitives, in the cell's pixels ──

  /** A filled box from (x0, y0) to (x1, y1), exclusive. */
  private fun rect(x0: Int, y0: Int, x1: Int, y1: Int) {
    if (x1 <= x0 || y1 <= y0) return
    canvas!!.drawRect((left + x0).toFloat(), (top + y0).toFloat(), (left + x1).toFloat(), (top + y1).toFloat(), paint)
  }

  /** A part of the cell between fractions across and down, rounded as ghostty's `fill`. */
  private fun fill(x0: Double, x1: Double, y0: Double, y1: Double) =
    rect(minEdge(x0, width), minEdge(y0, height), maxEdge(x1, width), maxEdge(y1, height))

  // A box's start edge aligns with its complement taken from the end, so a
  // half and the other half split an odd size without a gap or overlap.
  private fun minEdge(fraction: Double, size: Int) = size - ((1 - fraction) * size).roundToInt()
  private fun maxEdge(fraction: Double, size: Int) = (fraction * size).roundToInt()

  /** [path] (in the cell's pixels) filled. */
  private fun fillPath() {
    path.offset(left.toFloat(), top.toFloat())
    canvas!!.drawPath(path, paint)
  }

  /** [path] stroked [lineWidth] wide, butt-capped. */
  private fun strokePath(lineWidth: Float) {
    path.offset(left.toFloat(), top.toFloat())
    canvas!!.drawPath(path, strokePaint(lineWidth))
  }

  /** [path] stroked [lineWidth] wide inside its own outline only. */
  private fun innerStrokePath(lineWidth: Float) {
    path.offset(left.toFloat(), top.toFloat())
    val c = canvas!!
    c.save()
    c.clipPath(path)
    c.drawPath(path, strokePaint(lineWidth * 2))
    c.restore()
  }

  private fun strokePaint(lineWidth: Float) = stroke.apply {
    set(paint)
    style = Paint.Style.STROKE
    strokeWidth = lineWidth
    strokeCap = Paint.Cap.BUTT
    strokeJoin = Paint.Join.MITER
    isAntiAlias = true
  }

  /** Draws [draw] mirrored left to right within the cell. */
  private inline fun flipped(draw: () -> Unit) {
    val c = canvas!!
    c.save()
    c.scale(-1f, 1f, left + width / 2f, 0f)
    draw()
    c.restore()
  }

  private fun line(x0: Double, y0: Double, x1: Double, y1: Double, lineWidth: Int) {
    path.rewind()
    path.moveTo(x0.toFloat(), y0.toFloat())
    path.lineTo(x1.toFloat(), y1.toFloat())
    strokePath(lineWidth.toFloat())
  }

  // ── Box drawing ──

  private fun box(cp: Int) {
    val light = thickness
    val heavy = thickness * 2
    when (cp) {
      0x2504 -> dashHorizontal(3, light, max(4, light))
      0x2505 -> dashHorizontal(3, heavy, max(4, light))
      0x2506 -> dashVertical(3, light, max(4, light))
      0x2507 -> dashVertical(3, heavy, max(4, light))
      0x2508 -> dashHorizontal(4, light, max(4, light))
      0x2509 -> dashHorizontal(4, heavy, max(4, light))
      0x250A -> dashVertical(4, light, max(4, light))
      0x250B -> dashVertical(4, heavy, max(4, light))
      0x254C -> dashHorizontal(2, light, light)
      0x254D -> dashHorizontal(2, heavy, heavy)
      0x254E -> dashVertical(2, light, heavy)
      0x254F -> dashVertical(2, heavy, heavy)
      0x256D -> arc(Corner.BR)
      0x256E -> arc(Corner.BL)
      0x256F -> arc(Corner.TL)
      0x2570 -> arc(Corner.TR)
      0x2571 -> diagonalUp()
      0x2572 -> diagonalDown()
      0x2573 -> {
        diagonalUp()
        diagonalDown()
      }
      else -> lines(BOX_LINES[cp - 0x2500])
    }
  }

  /**
   * A line from each side to the center, each [NONE], [LIGHT], [HEAVY] or
   * [DOUBLE] ([spec]: up, right, down, left, two bits each), joined as ghostty
   * joins them.
   */
  private fun lines(spec: Int) {
    val up = spec shr 6 and 3
    val right = spec shr 4 and 3
    val down = spec shr 2 and 3
    val leftSide = spec and 3
    val lightPx = thickness
    val heavyPx = thickness * 2

    val hLightTop = sat(height - lightPx) / 2
    val hLightBottom = hLightTop + lightPx
    val hHeavyTop = sat(height - heavyPx) / 2
    val hHeavyBottom = hHeavyTop + heavyPx
    val hDoubleTop = sat(hLightTop - lightPx)
    val hDoubleBottom = hLightBottom + lightPx
    val vLightLeft = sat(width - lightPx) / 2
    val vLightRight = vLightLeft + lightPx
    val vHeavyLeft = sat(width - heavyPx) / 2
    val vHeavyRight = vHeavyLeft + heavyPx
    val vDoubleLeft = sat(vLightLeft - lightPx)
    val vDoubleRight = vLightRight + lightPx

    // Where each line stops short of (or runs past) the center, so they join.
    val upBottom = when {
      leftSide == HEAVY || right == HEAVY -> hHeavyBottom
      leftSide != right || down == up -> if (leftSide == DOUBLE || right == DOUBLE) hDoubleBottom else hLightBottom
      leftSide == NONE && right == NONE -> hLightBottom
      else -> hLightTop
    }
    val downTop = when {
      leftSide == HEAVY || right == HEAVY -> hHeavyTop
      leftSide != right || up == down -> if (leftSide == DOUBLE || right == DOUBLE) hDoubleTop else hLightTop
      leftSide == NONE && right == NONE -> hLightTop
      else -> hLightBottom
    }
    val leftRight = when {
      up == HEAVY || down == HEAVY -> vHeavyRight
      up != down || leftSide == right -> if (up == DOUBLE || down == DOUBLE) vDoubleRight else vLightRight
      up == NONE && down == NONE -> vLightRight
      else -> vLightLeft
    }
    val rightLeft = when {
      up == HEAVY || down == HEAVY -> vHeavyLeft
      up != down || right == leftSide -> if (up == DOUBLE || down == DOUBLE) vDoubleLeft else vLightLeft
      up == NONE && down == NONE -> vLightLeft
      else -> vLightRight
    }

    when (up) {
      LIGHT -> rect(vLightLeft, 0, vLightRight, upBottom)
      HEAVY -> rect(vHeavyLeft, 0, vHeavyRight, upBottom)
      DOUBLE -> {
        rect(vDoubleLeft, 0, vLightLeft, if (leftSide == DOUBLE) hLightTop else upBottom)
        rect(vLightRight, 0, vDoubleRight, if (right == DOUBLE) hLightTop else upBottom)
      }
    }
    when (right) {
      LIGHT -> rect(rightLeft, hLightTop, width, hLightBottom)
      HEAVY -> rect(rightLeft, hHeavyTop, width, hHeavyBottom)
      DOUBLE -> {
        rect(if (up == DOUBLE) vLightRight else rightLeft, hDoubleTop, width, hLightTop)
        rect(if (down == DOUBLE) vLightRight else rightLeft, hLightBottom, width, hDoubleBottom)
      }
    }
    when (down) {
      LIGHT -> rect(vLightLeft, downTop, vLightRight, height)
      HEAVY -> rect(vHeavyLeft, downTop, vHeavyRight, height)
      DOUBLE -> {
        rect(vDoubleLeft, if (leftSide == DOUBLE) hLightBottom else downTop, vLightLeft, height)
        rect(vLightRight, if (right == DOUBLE) hLightBottom else downTop, vDoubleRight, height)
      }
    }
    when (leftSide) {
      LIGHT -> rect(0, hLightTop, leftRight, hLightBottom)
      HEAVY -> rect(0, hHeavyTop, leftRight, hHeavyBottom)
      DOUBLE -> {
        rect(0, hDoubleTop, if (up == DOUBLE) vLightLeft else leftRight, hLightTop)
        rect(0, hLightBottom, if (down == DOUBLE) vLightLeft else leftRight, hDoubleBottom)
      }
    }
  }

  // Saturating subtraction, as ghostty's -| on sizes.
  private fun sat(value: Int) = max(0, value)

  /** [count] dashes across, centered, tiling into one even line cell to cell. */
  private fun dashHorizontal(count: Int, thick: Int, desiredGap: Int) {
    if (width < count * 2) {
      rect(0, sat(height - thickness) / 2, width, sat(height - thickness) / 2 + thickness)
      return
    }
    val gap = min(desiredGap, width / (2 * count))
    val dashes = width - count * gap
    val dash = dashes / count
    var extra = dashes % count
    val y = sat(height - thick) / 2
    var x = gap / 2
    repeat(count) {
      var x1 = x + dash
      if (extra > 0) {
        extra--
        x1++
      }
      rect(x, y, x1, y + thick)
      x = x1 + gap
    }
  }

  /** [count] dashes down from the top, a gap under each, tiling cell to cell. */
  private fun dashVertical(count: Int, thick: Int, desiredGap: Int) {
    if (height < count * 2) {
      rect(sat(width - thickness) / 2, 0, sat(width - thickness) / 2 + thickness, height)
      return
    }
    val gap = min(desiredGap, height / (2 * count))
    val dashes = height - count * gap
    val dash = dashes / count
    var extra = dashes % count
    val x = sat(width - thick) / 2
    var y = 0
    repeat(count) {
      var y1 = y + dash
      if (extra > 0) {
        extra--
        y1++
      }
      rect(x, y, x + thick, y1)
      y = y1 + gap
    }
  }

  private enum class Corner { TL, TR, BL, BR }

  /** ╭ ╮ ╯ ╰: a light line from two sides, curving through the center. */
  private fun arc(corner: Corner) {
    val thick = thickness.toDouble()
    val w = width.toDouble()
    val h = height.toDouble()
    val cx = sat(width - thickness) / 2 + thick / 2
    val cy = sat(height - thickness) / 2 + thick / 2
    val r = min(w, h) / 2
    // How far from the center the middle control points sit, as a share of the radius.
    val s = 0.25
    val down = corner == Corner.BL || corner == Corner.BR
    val toRight = corner == Corner.TR || corner == Corner.BR
    val sy = if (down) 1 else -1
    val sx = if (toRight) 1 else -1
    path.rewind()
    path.moveTo(cx.toFloat(), (if (down) h else 0.0).toFloat())
    path.lineTo(cx.toFloat(), (cy + sy * r).toFloat())
    path.cubicTo(
      cx.toFloat(), (cy + sy * s * r).toFloat(),
      (cx + sx * s * r).toFloat(), cy.toFloat(),
      (cx + sx * r).toFloat(), cy.toFloat(),
    )
    path.lineTo((if (toRight) w else 0.0).toFloat(), cy.toFloat())
    strokePath(thickness.toFloat())
  }

  // The diagonals overshoot the corners a little, keeping their slope.
  private fun diagonalUp() {
    val w = width.toDouble()
    val h = height.toDouble()
    val sx = min(1.0, w / h)
    val sy = min(1.0, h / w)
    line(w + 0.5 * sx, -0.5 * sy, -0.5 * sx, h + 0.5 * sy, thickness)
  }

  private fun diagonalDown() {
    val w = width.toDouble()
    val h = height.toDouble()
    val sx = min(1.0, w / h)
    val sy = min(1.0, h / w)
    line(-0.5 * sx, -0.5 * sy, w + 0.5 * sx, h + 0.5 * sy, thickness)
  }

  // ── Block elements ──

  private fun block(cp: Int) {
    // A part of the cell, in eighths across and down.
    fun part(x0: Int, y0: Int, x1: Int, y1: Int) = rect(
      (width * x0 / 8.0).roundToInt(), (height * y0 / 8.0).roundToInt(),
      (width * x1 / 8.0).roundToInt(), (height * y1 / 8.0).roundToInt(),
    )
    when (cp) {
      0x2580 -> part(0, 0, 8, 4)
      in 0x2581..0x2588 -> part(0, 8 - (cp - 0x2580), 8, 8)
      in 0x2589..0x258F -> part(0, 0, 8 - (cp - 0x2588), 8)
      0x2590 -> part(4, 0, 8, 8)
      in 0x2591..0x2593 -> {
        // Shades: the whole cell, a quarter, half or three quarters as strong.
        val alpha = paint.alpha
        paint.alpha = alpha * (cp - 0x2590) / 4
        part(0, 0, 8, 8)
        paint.alpha = alpha
      }
      0x2594 -> part(0, 0, 8, 1)
      0x2595 -> part(7, 0, 8, 8)
      else -> {
        val quadrants = QUADRANTS[cp - 0x2596]
        if (quadrants and 1 != 0) part(0, 0, 4, 4)
        if (quadrants and 2 != 0) part(4, 0, 8, 4)
        if (quadrants and 4 != 0) part(0, 4, 4, 8)
        if (quadrants and 8 != 0) part(4, 4, 8, 8)
      }
    }
  }

  // ── Geometric shapes ──

  private fun geometric(cp: Int) {
    val w = width.toFloat()
    val h = height.toFloat()
    // The triangle in a corner: its three points.
    fun triangle(corner: Corner) {
      path.rewind()
      when (corner) {
        Corner.TL -> { path.moveTo(0f, 0f); path.lineTo(0f, h); path.lineTo(w, 0f) }
        Corner.TR -> { path.moveTo(0f, 0f); path.lineTo(w, h); path.lineTo(w, 0f) }
        Corner.BL -> { path.moveTo(0f, 0f); path.lineTo(0f, h); path.lineTo(w, h) }
        Corner.BR -> { path.moveTo(0f, h); path.lineTo(w, h); path.lineTo(w, 0f) }
      }
      path.close()
    }
    when (cp) {
      0x25E2 -> { triangle(Corner.BR); fillPath() }
      0x25E3 -> { triangle(Corner.BL); fillPath() }
      0x25E4 -> { triangle(Corner.TL); fillPath() }
      0x25E5 -> { triangle(Corner.TR); fillPath() }
      0x25F8 -> { triangle(Corner.TL); innerStrokePath(thickness.toFloat()) }
      0x25F9 -> { triangle(Corner.TR); innerStrokePath(thickness.toFloat()) }
      0x25FA -> { triangle(Corner.BL); innerStrokePath(thickness.toFloat()) }
      0x25FF -> { triangle(Corner.BR); innerStrokePath(thickness.toFloat()) }
    }
  }

  // ── Braille ──

  /** Eight dots, sized and spaced to the cell as ghostty fits them. */
  private fun braille(cp: Int) {
    var w = min(width / 4, height / 8)
    var xSpacing = width / 4
    var ySpacing = height / 8
    var xMargin = xSpacing / 2
    var yMargin = ySpacing / 2
    var xLeft = width - 2 * xMargin - xSpacing - 2 * w
    var yLeft = height - 2 * yMargin - 3 * ySpacing - 4 * w
    if (xLeft >= 2 && yLeft >= 4 && w == 0) {
      w++
      xLeft -= 2
      yLeft -= 4
    }
    if (xLeft >= 2 && xMargin == 0) {
      xMargin = 1
      xLeft -= 2
    }
    if (yLeft >= 2 && yMargin == 0) {
      yMargin = 1
      yLeft -= 2
    }
    if (xLeft >= 1) {
      xSpacing++
      xLeft--
    }
    if (yLeft >= 3) {
      ySpacing++
      yLeft -= 3
    }
    if (xLeft >= 2) {
      xMargin++
      xLeft -= 2
    }
    if (yLeft >= 2) {
      yMargin++
      yLeft -= 2
    }
    if (xLeft >= 2 && yLeft >= 4) w++
    val x = intArrayOf(xMargin, xMargin + w + xSpacing)
    val y = IntArray(4) { yMargin + it * (w + ySpacing) }
    // Dots 1-3 and 7 down the left, 4-6 and 8 down the right (the code point's bits).
    val bits = cp - 0x2800
    val dots = arrayOf(0 to 0, 0 to 1, 0 to 2, 1 to 0, 1 to 1, 1 to 2, 0 to 3, 1 to 3)
    for ((bit, dot) in dots.withIndex()) {
      if (bits shr bit and 1 == 0) continue
      val (col, row) = dot
      rect(x[col], y[row], x[col] + w, y[row] + w)
    }
  }

  // ── Powerline ──

  private fun powerline(cp: Int) {
    val w = width.toFloat()
    val h = height.toFloat()
    fun triangle(x0: Float, y0: Float, x1: Float, y1: Float, x2: Float, y2: Float) {
      path.rewind()
      path.moveTo(x0, y0)
      path.lineTo(x1, y1)
      path.lineTo(x2, y2)
      path.close()
      fillPath()
    }
    // The thin arrow: two strokes meeting at the right edge's middle.
    fun arrow() {
      path.rewind()
      path.moveTo(0f, 0f)
      path.lineTo(w, h / 2)
      path.lineTo(0f, h)
      strokePath(thickness.toFloat())
    }
    // The half disc on the left edge.
    fun halfDisc(outline: Boolean) {
      val c = ((sqrt(2.0) - 1) * 4 / 3).toFloat()
      val r = min(w, h / 2)
      path.rewind()
      path.moveTo(0f, 0f)
      if (outline) path.lineTo(1f, 0f)
      path.cubicTo(r * c, 0f, r, r - r * c, r, r)
      path.lineTo(r, h - r)
      path.cubicTo(r, h - r + r * c, r * c, h, if (outline) 1f else 0f, h)
      if (outline) {
        path.lineTo(0f, h)
        innerStrokePath(thickness.toFloat())
      } else {
        path.close()
        fillPath()
      }
    }
    // Two wedges, top and bottom, meeting short of the middle.
    fun flames() {
      val t = thickness.toFloat()
      path.rewind()
      path.moveTo(0f, 0f); path.lineTo(w, 0f); path.lineTo(w / 2, h / 2 - t / 2); path.lineTo(0f, h / 2 - t / 2)
      path.close()
      fillPath()
      path.rewind()
      path.moveTo(0f, h); path.lineTo(w, h); path.lineTo(w / 2, h / 2 + t / 2); path.lineTo(0f, h / 2 + t / 2)
      path.close()
      fillPath()
    }
    when (cp) {
      0xE0B0 -> triangle(0f, 0f, w, h / 2, 0f, h)
      0xE0B1 -> arrow()
      0xE0B2 -> triangle(w, 0f, 0f, h / 2, w, h)
      0xE0B3 -> flipped { arrow() }
      0xE0B4 -> halfDisc(outline = false)
      0xE0B5 -> halfDisc(outline = true)
      0xE0B6 -> flipped { halfDisc(outline = false) }
      0xE0B7 -> flipped { halfDisc(outline = true) }
      0xE0B8 -> triangle(0f, 0f, w, h, 0f, h)
      0xE0B9, 0xE0BF -> diagonalDown()
      0xE0BA -> triangle(w, 0f, w, h, 0f, h)
      0xE0BB, 0xE0BD -> diagonalUp()
      0xE0BC -> triangle(0f, 0f, w, 0f, 0f, h)
      0xE0BE -> triangle(0f, 0f, w, 0f, w, h)
      0xE0D2 -> flames()
      0xE0D4 -> flipped { flames() }
    }
  }

  // ── Legacy computing ──

  /** 🬀–🬻: a cell in two columns and three rows, some filled (those of █ ▌ ▐ left out). */
  private fun sextant(cp: Int) {
    val i = cp - 0x1FB00
    val cells = i + i / 0x14 + 1
    val thirds = doubleArrayOf(0.0, 1.0 / 3, 2.0 / 3, 1.0)
    for (part in 0 until 6) {
      if (cells shr part and 1 == 0) continue
      val col = part % 2
      val row = part / 2
      fill(col * 0.5, (col + 1) * 0.5, thirds[row], thirds[row + 1])
    }
  }

  /** Two columns and four rows, some filled. */
  private fun octant(cp: Int) {
    val cells = OCTANTS[cp - 0x1CD00]
    for (part in 0 until 8) {
      if (cells shr part and 1 == 0) continue
      val col = part % 2
      val row = part / 2
      fill(col * 0.5, (col + 1) * 0.5, row * 0.25, (row + 1) * 0.25)
    }
  }

  companion object {
    /** Whether [codepoint] is drawn here. */
    fun covers(codepoint: Int) = when (codepoint) {
      in 0x2500..0x259F, in 0x25E2..0x25E5, in 0x25F8..0x25FA, 0x25FF, in 0x2800..0x28FF,
      in 0xE0B0..0xE0BF, 0xE0D2, 0xE0D4, in 0x1FB00..0x1FB3B, in 0x1CD00..0x1CDE5 -> true
      else -> false
    }

    private const val NONE = 0
    private const val LIGHT = 1
    private const val HEAVY = 2
    private const val DOUBLE = 3

    /** A line spec: up, right, down, left, two bits each. */
    private fun l(up: Int = NONE, right: Int = NONE, down: Int = NONE, left: Int = NONE) =
      up shl 6 or (right shl 4) or (down shl 2) or left

    private const val L = LIGHT
    private const val H = HEAVY
    private const val D = DOUBLE

    // U+2500–257F as lines (the dashes, arcs and diagonals among them are drawn otherwise).
    private val BOX_LINES = intArrayOf(
      l(left = L, right = L), l(left = H, right = H), l(up = L, down = L), l(up = H, down = H), // ─━│┃
      0, 0, 0, 0, 0, 0, 0, 0, // ┄┅┆┇┈┉┊┋
      l(down = L, right = L), l(down = L, right = H), l(down = H, right = L), l(down = H, right = H), // ┌┍┎┏
      l(down = L, left = L), l(down = L, left = H), l(down = H, left = L), l(down = H, left = H), // ┐┑┒┓
      l(up = L, right = L), l(up = L, right = H), l(up = H, right = L), l(up = H, right = H), // └┕┖┗
      l(up = L, left = L), l(up = L, left = H), l(up = H, left = L), l(up = H, left = H), // ┘┙┚┛
      l(up = L, down = L, right = L), l(up = L, down = L, right = H), l(up = H, right = L, down = L), l(down = H, right = L, up = L), // ├┝┞┟
      l(up = H, down = H, right = L), l(down = L, right = H, up = H), l(up = L, right = H, down = H), l(up = H, down = H, right = H), // ┠┡┢┣
      l(up = L, down = L, left = L), l(up = L, down = L, left = H), l(up = H, left = L, down = L), l(down = H, left = L, up = L), // ┤┥┦┧
      l(up = H, down = H, left = L), l(down = L, left = H, up = H), l(up = L, left = H, down = H), l(up = H, down = H, left = H), // ┨┩┪┫
      l(down = L, left = L, right = L), l(left = H, right = L, down = L), l(right = H, left = L, down = L), l(down = L, left = H, right = H), // ┬┭┮┯
      l(down = H, left = L, right = L), l(right = L, left = H, down = H), l(left = L, right = H, down = H), l(down = H, left = H, right = H), // ┰┱┲┳
      l(up = L, left = L, right = L), l(left = H, right = L, up = L), l(right = H, left = L, up = L), l(up = L, left = H, right = H), // ┴┵┶┷
      l(up = H, left = L, right = L), l(right = L, left = H, up = H), l(left = L, right = H, up = H), l(up = H, left = H, right = H), // ┸┹┺┻
      l(up = L, down = L, left = L, right = L), l(left = H, right = L, up = L, down = L), // ┼┽
      l(right = H, left = L, up = L, down = L), l(up = L, down = L, left = H, right = H), // ┾┿
      l(up = H, down = L, left = L, right = L), l(down = H, up = L, left = L, right = L), // ╀╁
      l(up = H, down = H, left = L, right = L), l(left = H, up = H, right = L, down = L), // ╂╃
      l(right = H, up = H, left = L, down = L), l(left = H, down = H, right = L, up = L), // ╄╅
      l(right = H, down = H, left = L, up = L), l(down = L, up = H, left = H, right = H), // ╆╇
      l(up = L, down = H, left = H, right = H), l(right = L, left = H, up = H, down = H), // ╈╉
      l(left = L, right = H, up = H, down = H), l(up = H, down = H, left = H, right = H), // ╊╋
      0, 0, 0, 0, // ╌╍╎╏
      l(left = D, right = D), l(up = D, down = D), l(down = L, right = D), l(down = D, right = L), // ═║╒╓
      l(down = D, right = D), l(down = L, left = D), l(down = D, left = L), l(down = D, left = D), // ╔╕╖╗
      l(up = L, right = D), l(up = D, right = L), l(up = D, right = D), l(up = L, left = D), // ╘╙╚╛
      l(up = D, left = L), l(up = D, left = D), l(up = L, down = L, right = D), l(up = D, down = D, right = L), // ╜╝╞╟
      l(up = D, down = D, right = D), l(up = L, down = L, left = D), l(up = D, down = D, left = L), l(up = D, down = D, left = D), // ╠╡╢╣
      l(down = L, left = D, right = D), l(down = D, left = L, right = L), l(down = D, left = D, right = D), l(up = L, left = D, right = D), // ╤╥╦╧
      l(up = D, left = L, right = L), l(up = D, left = D, right = D), // ╨╩
      l(up = L, down = L, left = D, right = D), l(up = D, down = D, left = L, right = L), l(up = D, down = D, left = D, right = D), // ╪╫╬
      0, 0, 0, 0, 0, 0, 0, // ╭╮╯╰╱╲╳
      l(left = L), l(up = L), l(right = L), l(down = L), l(left = H), l(up = H), l(right = H), l(down = H), // ╴╵╶╷╸╹╺╻
      l(left = L, right = H), l(up = L, down = H), l(left = H, right = L), l(up = H, down = L), // ╼╽╾╿
    )

    // ▖ ▗ ▘ ▙ ▚ ▛ ▜ ▝ ▞ ▟ (U+2596–259F): which quarters each fills
    // (1 upper left, 2 upper right, 4 lower left, 8 lower right).
    private val QUADRANTS = intArrayOf(4, 8, 1, 1 or 4 or 8, 1 or 8, 1 or 2 or 4, 1 or 2 or 8, 2, 2 or 4, 2 or 4 or 8)

    // U+1CD00–1CDE5: which eighths each fills, bit n for Unicode's octant n + 1
    // (1-2 the top row, left and right, down to 7-8), from ghostty's octants.txt.
    private val OCTANTS = intArrayOf(
      0x04, 0x06, 0x07, 0x08, 0x09, 0x0B, 0x0C, 0x0D, 0x0E, 0x10, 0x11, 0x12, 0x13, 0x15, 0x16, 0x17,
      0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
      0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38,
      0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A,
      0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x51, 0x52, 0x53, 0x54, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C, 0x5D,
      0x5E, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E,
      0x6F, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E,
      0x7F, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F,
      0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9D, 0x9E, 0x9F,
      0xA1, 0xA2, 0xA3, 0xA4, 0xA6, 0xA7, 0xA8, 0xA9, 0xAB, 0xAC, 0xAD, 0xAE, 0xB0, 0xB1, 0xB2, 0xB3,
      0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC1, 0xC2, 0xC3, 0xC4,
      0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xCB, 0xCC, 0xCD, 0xCE, 0xCF, 0xD0, 0xD1, 0xD2, 0xD3, 0xD4,
      0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDE, 0xDF, 0xE0, 0xE1, 0xE2, 0xE3, 0xE4,
      0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xEB, 0xEC, 0xED, 0xEE, 0xEF, 0xF1, 0xF2, 0xF3, 0xF4, 0xF6,
      0xF7, 0xF8, 0xF9, 0xFB, 0xFD, 0xFE,
    )
  }
}
