package dev.herdr.terminal

import android.graphics.Color
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SelectionTest {
  private val selection = Color.rgb(0x32, 0x38, 0x44)

  @Test
  fun selectedTextKeepsItsColorOnTheSelection() {
    val grid = Grid().apply { painter.selectionBg = selection }
    grid.draw(0, cells("WW", fg = Color.RED), selStart = 0, selEnd = 1)
    val pixels = grid.cellPixels(0, 0)
    assertTrue(share(pixels, selection, tolerance = 4) > 0.4, "the selection behind")
    assertTrue(share(pixels, Color.RED, tolerance = 60) > 0.05, "the text, red")
  }

  @Test
  fun selectedTextTheSelectionsColorStillShows() {
    // A program's text in the selection's own color (or near it) would vanish.
    val grid = Grid().apply { painter.selectionBg = selection }
    grid.draw(0, cells("WW", fg = Color.rgb(0x30, 0x36, 0x42)), selStart = 0, selEnd = 1)
    assertTrue(share(grid.cellPixels(0, 0), Grid.FG, tolerance = 60) > 0.05, "the text, readable")
  }

  @Test
  fun anInverseCellSelectedWithoutThemeLooksSelected() {
    // Unthemed, a selection is the default colors swapped: an inverse cell
    // swapped back would look unselected.
    val grid = Grid()
    grid.draw(0, cells("  ", flags = GridPainter.FLAG_INVERSE), selStart = 0, selEnd = 1)
    grid.draw(1, cells("  "), selStart = 0, selEnd = 1)
    for (row in 0..1) assertTrue(share(grid.cellPixels(0, row), Grid.FG, tolerance = 4) > 0.95, "row $row")
  }

  @Test
  fun aWideCharactersRightHalfIsSelectedWithIt() {
    val grid = Grid().apply { painter.selectionBg = selection }
    grid.draw(0, listOf(Cell("中", bg = Color.BLUE, flags = GridPainter.FLAG_WIDE), Cell(bg = Color.BLUE, flags = GridPainter.FLAG_SPACER)), selStart = 0, selEnd = 0)
    // Under the glyph's strokes the cells' background shows; at the bottom edge only it.
    val bottom = grid.cellPixels(1, 0).takeLast(grid.painter.cellWidth.toInt())
    assertTrue(share(bottom, selection, tolerance = 4) > 0.9, "the right half's background")
  }
}
