package dev.herdr.terminal

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertSame
import kotlin.test.assertTrue

class RowDrawingsTest {
  private class Drawing(val id: Int) {
    var content = ""
    var moved = 0
  }

  private var made = 0
  private val discarded = mutableListOf<Drawing>()
  private val rows = RowDrawings({ Drawing(made++) }, { discarded += it }, { d, by -> d.moved = by })

  private fun frame(moves: Map<Int, Int> = emptyMap(), recorded: Map<Int, String> = emptyMap()) {
    val movedFrom = IntArray(rows.size) { moves[it] ?: -1 }
    val record = BooleanArray(rows.size) { it in recorded }
    rows.frame(movedFrom, record) { y, d -> d.content = recorded.getValue(y) }
  }

  private fun shown() = (0 until rows.size).map { rows[it]?.content }

  @Test
  fun scrolledRowsMoveTheirDrawingsWithoutRecordingThem() {
    rows.reset(3)
    frame(recorded = mapOf(0 to "a", 1 to "b", 2 to "c"))
    assertEquals(3, made)
    val b = rows[1]
    // The viewport up a row: a and b move down, a new row on top.
    frame(moves = mapOf(1 to 0, 2 to 1), recorded = mapOf(0 to "new"))
    assertEquals(listOf("new", "a", "b"), shown())
    assertSame(b, rows[2])
    assertEquals(1, b!!.moved)
    // c's drawing went to the new row: none made.
    assertEquals(3, made)
    assertTrue(discarded.isEmpty())
  }

  @Test
  fun keptRowsStayAsTheyAre() {
    rows.reset(2)
    frame(recorded = mapOf(0 to "a", 1 to "b"))
    val a = rows[0]
    frame(recorded = mapOf(1 to "B"))
    assertEquals(listOf("a", "B"), shown())
    assertSame(a, rows[0])
    assertEquals(2, made, "B re-recorded into its own drawing")
  }

  @Test
  fun aRowRecordedWhileItsDrawingMovesGetsAnother() {
    rows.reset(2)
    frame(recorded = mapOf(0 to "a", 1 to "b"))
    // Down a row: row 0's drawing moves to row 1 and row 0 is recorded anew.
    frame(moves = mapOf(1 to 0), recorded = mapOf(0 to "new"))
    assertEquals(listOf("new", "a"), shown())
    // b's drawing, no longer shown, was free for it: none made, none let go.
    assertEquals(2, made)
    assertTrue(discarded.isEmpty())
  }

  @Test
  fun aResetLetsGoOfEverything() {
    rows.reset(2)
    frame(recorded = mapOf(0 to "a", 1 to "b"))
    rows.reset(3)
    assertEquals(2, discarded.size)
    assertEquals(listOf(null, null, null), shown())
  }
}
