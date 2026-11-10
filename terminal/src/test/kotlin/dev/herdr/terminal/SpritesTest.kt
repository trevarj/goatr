package dev.herdr.terminal

import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SpritesTest {
  private fun Grid.lit(x: Int, y: Int) = distance(bitmap.getPixel(x, y), Grid.FG) < 60

  /** The x of a vertical line through [col]'s cells: its lit column nearest the middle. */
  private fun Grid.lineX(col: Int, row: Int): Int {
    val mid = ((col + 0.5) * painter.cellWidth).toInt()
    val y = (row + 0.5).times(painter.cellHeight).toInt()
    return (0..painter.cellWidth.toInt()).flatMap { listOf(mid - it, mid + it) }.first { lit(it, y) }
  }

  @Test
  fun aVerticalLineRunsUnbrokenDownRows() {
    val grid = Grid(rows = 3)
    for (row in 0..2) grid.draw(row, cells("│"))
    val x = grid.lineX(0, 0)
    val gaps = (0 until 3 * grid.painter.cellHeight).filter { !grid.lit(x, it) }
    assertEquals(emptyList(), gaps, "unlit pixels down the line")
  }

  @Test
  fun aHorizontalLineRunsUnbrokenAcrossCells() {
    val grid = Grid()
    grid.draw(0, cells("──────"))
    val y = (0 until grid.painter.cellHeight).first { grid.lit(grid.painter.cellWidth.toInt(), it) }
    val gaps = (0 until (6 * grid.painter.cellWidth).toInt()).filter { !grid.lit(it, y) }
    assertEquals(emptyList(), gaps, "unlit pixels along the line")
  }

  @Test
  fun aRoundedCornerMeetsItsLines() {
    // ╭─
    // │
    val grid = Grid(rows = 2)
    grid.draw(0, cells("╭─"))
    grid.draw(1, cells("│ "))
    val x = grid.lineX(0, 1)
    val h = grid.painter.cellHeight
    // The vertical line goes on up into the corner's cell...
    assertTrue(grid.lit(x, h - 1) && grid.lit(x, h), "the corner meets the line below")
    // ...and the horizontal one in from the next cell.
    val right = grid.painter.cellWidth.toInt()
    val y = (0 until h).first { grid.lit(right + 2, it) }
    assertTrue(grid.lit(right - 1, y), "the corner meets the line beside")
  }

  @Test
  fun brailleDotsSpanTheCell() {
    val grid = Grid()
    grid.draw(0, cells("⣿"))
    val h = grid.painter.cellHeight
    val litRows = (0 until h).filter { y -> (0 until grid.painter.cellWidth.toInt()).any { grid.lit(it, y) } }
    assertTrue(litRows.first() < h / 6, "the top dots near the top: ${litRows.first()} of $h")
    assertTrue(litRows.last() > h * 5 / 6, "the bottom dots near the bottom: ${litRows.last()} of $h")
    // Four rows of dots: four runs of lit rows.
    val runs = litRows.zipWithNext().count { (a, b) -> b != a + 1 } + 1
    assertEquals(4, runs)
  }

  @Test
  fun aPowerlineArrowFillsTheCellsHeight() {
    val grid = Grid()
    grid.draw(0, cells("\uE0B0"))
    // (Its tip's first and last rows are anti-aliased, as ghostty's.)
    val gaps = (1 until grid.painter.cellHeight - 1).filter { !grid.lit(0, it) }
    assertEquals(emptyList(), gaps, "unlit pixels down its left edge")
  }

  @Test
  fun aSextantFillsItsThirds() {
    // 🬋 (U+1FB0B): the middle row, both halves.
    val grid = Grid()
    grid.draw(0, listOf(Cell(String(Character.toChars(0x1FB0B)))))
    val h = grid.painter.cellHeight
    val x = (grid.painter.cellWidth / 2).toInt()
    assertTrue(!grid.lit(x, h / 6) && grid.lit(x, h / 2) && !grid.lit(x, h * 5 / 6))
  }
}
