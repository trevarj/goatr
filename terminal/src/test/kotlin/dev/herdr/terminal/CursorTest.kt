package dev.herdr.terminal

import android.graphics.Color
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class CursorTest {
  private val cursor = Color.rgb(0x40, 0xC0, 0xFF)

  @Test
  fun aBlockOnAWideCharacterCoversBothItsCells() {
    val grid = Grid()
    grid.draw(0, listOf(Cell("中", flags = GridPainter.FLAG_WIDE), Cell(flags = GridPainter.FLAG_SPACER)))
    grid.painter.drawCursor(grid.canvas, 0, 0, GridPainter.CURSOR_STYLE_BLOCK, cursor, wide = true)
    for (col in 0..1) assertTrue(share(grid.cellPixels(col, 0), Grid.BG, tolerance = 8) < 0.1, "cell $col")
    assertTrue(share(grid.cellPixels(2, 0), Grid.BG, tolerance = 0) > 0.99)
  }

  @Test
  fun aHollowOneToo() {
    val grid = Grid()
    grid.painter.drawCursor(grid.canvas, 0, 0, GridPainter.CURSOR_STYLE_BLOCK_HOLLOW, cursor, wide = true)
    // Its right edge: where the second cell ends, halfway down.
    val edge = (2 * grid.painter.cellWidth).toInt()
    val y = grid.painter.cellHeight / 2
    assertTrue((edge - 2..edge + 1).any { distance(grid.bitmap.getPixel(it, y), Grid.BG) > 30 }, "the right edge")
  }

  @Test
  fun aBlockIsSolidWithItsCharacterOverItInTheBackground() {
    // A cursor the text's own color, as the default theme's: the character
    // must still show.
    val grid = Grid()
    grid.draw(0, cells("W"))
    grid.painter.drawCursor(grid.canvas, 0, 0, GridPainter.CURSOR_STYLE_BLOCK, Grid.FG, glyph = charArrayOf('W'), glyphLength = 1)
    val pixels = grid.cellPixels(0, 0)
    assertTrue(share(pixels, Grid.FG, tolerance = 8) > 0.4, "solid")
    assertTrue(share(pixels, Grid.BG, tolerance = 40) > 0.05, "the W, in the background color")
  }

  @Test
  fun unfocusedItsAHollowBoxEvenMidBlink() {
    val bar = GridPainter.CURSOR_STYLE_BAR
    val hollow = GridPainter.CURSOR_STYLE_BLOCK_HOLLOW
    assertEquals(bar, GridPainter.cursorShape(bar, visible = true, focused = true, blinkedOff = false))
    assertEquals(null, GridPainter.cursorShape(bar, visible = true, focused = true, blinkedOff = true))
    assertEquals(hollow, GridPainter.cursorShape(bar, visible = true, focused = false, blinkedOff = false))
    assertEquals(hollow, GridPainter.cursorShape(bar, visible = true, focused = false, blinkedOff = true))
    assertEquals(null, GridPainter.cursorShape(bar, visible = false, focused = false, blinkedOff = false))
  }
}
