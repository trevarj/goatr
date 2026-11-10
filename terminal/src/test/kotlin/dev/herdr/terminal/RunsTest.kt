package dev.herdr.terminal

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class RunsTest {
  private fun drawn(cells: List<Cell>): List<String> {
    val grid = Grid(cols = 30)
    val canvas = TextCanvas(grid.bitmap)
    grid.painter.drawRow(rowRecord(0, grid.cols, cells), canvas, grid.cols)
    return canvas.texts
  }

  @Test
  fun aLineInOneLookIsOneDraw() {
    assertEquals(listOf("ls -la /home"), drawn(cells("ls -la /home")))
  }

  @Test
  fun aRunEndsWhereTheLookChanges() {
    assertEquals(listOf("drwxr", "home"), drawn(cells("drwxr ", fg = Color.BLUE) + cells("home")))
  }

  @Test
  fun aRunGoesOverBlankCells() {
    assertEquals(listOf("a  b"), drawn(cells("a") + listOf(Cell(), Cell()) + cells("b")))
  }

  @Test
  fun runsLookAsCellByCell() {
    // Each character where it would be drawn on its own.
    val text = "Hello, wörld! {[(<>)]} 0123456789 ~"
    val grid = Grid(cols = text.length)
    grid.draw(0, cells(text))
    val reference = Bitmap.createBitmap(grid.bitmap.width, grid.bitmap.height, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(reference).apply { drawColor(Grid.BG) }
    val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
      typeface = jetBrainsMono
      textSize = grid.painter.textPaint.textSize
      fontFeatureSettings = grid.painter.textPaint.fontFeatureSettings
      fontVariationSettings = "'wght' 400"
      color = Grid.FG
    }
    val baseline = -paint.fontMetrics.ascent
    for ((i, c) in text.withIndex()) canvas.drawText(c.toString(), i * grid.painter.cellWidth, baseline, paint)
    var differ = 0
    for (y in 0 until grid.painter.cellHeight) for (x in 0 until grid.bitmap.width) {
      if (distance(grid.bitmap.getPixel(x, y), reference.getPixel(x, y)) > 48) differ++
    }
    assertTrue(differ < 20, "$differ pixels differ")
  }

  @Test
  fun backgroundsAreARectPerRunOfOneColor() {
    val grid = Grid(cols = 10)
    val rects = mutableListOf<Int>()
    val canvas = object : Canvas(grid.bitmap) {
      override fun drawRect(left: Float, top: Float, right: Float, bottom: Float, paint: Paint) {
        rects += paint.color
        super.drawRect(left, top, right, bottom, paint)
      }
    }
    grid.painter.drawRow(rowRecord(0, grid.cols, cells("    ", bg = Color.BLUE) + cells("  ", bg = Color.RED)), canvas, grid.cols)
    // The row's clear, then one for the blue run and one for the red.
    assertEquals(listOf(Grid.BG, Color.BLUE, Color.RED), rects)
  }

  @Test
  fun aWideCharactersRightHalfHasItsBackground() {
    val grid = Grid()
    grid.draw(0, listOf(Cell("中", bg = Color.BLUE, flags = GridPainter.FLAG_WIDE), Cell(flags = GridPainter.FLAG_SPACER)))
    val bottom = grid.cellPixels(1, 0).takeLast(grid.painter.cellWidth.toInt())
    assertTrue(share(bottom, Color.BLUE, tolerance = 4) > 0.9)
  }
}
