package dev.herdr.terminal

import android.graphics.Color
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class GridPainterTest {
  @Test
  fun aFullBlockFillsItsCell() {
    val grid = Grid()
    grid.draw(0, cells("█", fg = Color.RED))
    assertTrue(share(grid.cellPixels(0, 0), Color.RED, tolerance = 0) > 0.99)
  }

  @Test
  fun textDrawsInItsCellsOnly() {
    val grid = Grid()
    grid.draw(1, cells("WW"))
    assertTrue(share(grid.cellPixels(0, 1), Grid.FG, tolerance = 40) > 0.05)
    assertTrue(share(grid.cellPixels(1, 1), Grid.FG, tolerance = 40) > 0.05)
    assertTrue(share(grid.cellPixels(2, 1), Grid.BG, tolerance = 0) > 0.99)
    assertTrue(share(grid.cellPixels(0, 0), Grid.BG, tolerance = 0) > 0.99)
  }

  @Test
  fun anOverlineRunsAlongTheCellsTop() {
    val grid = Grid()
    grid.draw(0, cells("ab", flags = GridPainter.FLAG_OVERLINE))
    val top = (0 until (2 * grid.painter.cellWidth).toInt()).map { grid.bitmap.getPixel(it, 0) }
    assertTrue(share(top, Grid.FG, tolerance = 30) > 0.95)
  }
}
