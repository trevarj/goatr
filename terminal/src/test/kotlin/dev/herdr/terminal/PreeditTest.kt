package dev.herdr.terminal

import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PreeditTest {
  @Test
  fun composingTextShowsAtTheCursorUnderlined() {
    val grid = Grid()
    grid.draw(0, cells("\$ xxxxxx"))
    grid.painter.drawPreedit(grid.canvas, "日本", 2, 0, grid.cols)
    // Each wide character on two cells, over what was there.
    for (col in 2..5) {
      val pixels = grid.cellPixels(col, 0)
      assertTrue(share(pixels, Grid.FG, tolerance = 60) > 0.05, "cell $col: the text")
      val bottom = pixels.takeLast(grid.painter.cellWidth.toInt() - 2)
      assertTrue(share(bottom, Grid.FG, tolerance = 30) > 0.9, "cell $col: underlined")
    }
    // Past it, the row as it was.
    assertTrue(share(grid.cellPixels(6, 0), Grid.FG, tolerance = 60) > 0.05, "the x after it")
  }
}
