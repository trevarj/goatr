package dev.herdr.terminal

import android.graphics.Color
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertEquals

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class LigatureTest {
  private fun drawn(cells: List<Cell>, cursorX: Int = -1): List<String> {
    val grid = Grid().apply { painter.setLigatures(true) }
    val canvas = TextCanvas(grid.bitmap)
    grid.painter.drawRow(rowRecord(0, grid.cols, cells), canvas, grid.cols, cursorX, cursorY = 0)
    return canvas.texts
  }

  @Test
  fun aRunJoinsCellsThatLookAlike() {
    assertEquals(listOf("a=>b"), drawn(cells("a=>b")))
  }

  @Test
  fun aRunEndsWhereTheBackgroundChanges() {
    // A selection's edge between = and >.
    assertEquals(listOf("a=", ">b"), drawn(cells("a=") + cells(">b", bg = Color.BLUE)))
  }

  @Test
  fun aRunEndsAroundTheCursor() {
    assertEquals(listOf("a", "=", ">b"), drawn(cells("a=>b"), cursorX = 1))
  }
}
