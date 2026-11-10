package io.github.trevarj.goatr.app

import io.github.trevarj.goatr.app.ui.Backlog
import kotlin.test.Test
import kotlin.test.assertEquals

class BacklogTest {
  private var clock = 0L
  private val posted = ArrayDeque<Runnable>()
  private val log = mutableListOf<String>()

  // Parsing a chunk takes as many ms as it has bytes.
  private val backlog = Backlog<String>(
    now = { clock },
    post = { posted.addLast(it) },
    budgetMs = 8,
    write = { clock += it.size; log += "${it.size}" },
    ended = { log += it },
    afterTurn = { log += "|" },
  )

  private fun runPosted(): Int {
    var turns = 0
    while (posted.isNotEmpty()) {
      posted.removeFirst().run()
      turns++
    }
    return turns
  }

  @Test
  fun whatCameInMeanwhileIsOneTurn() {
    repeat(3) { backlog.add(ByteArray(1)) }
    assertEquals(1, posted.size)
    assertEquals(1, runPosted())
    assertEquals(listOf("1", "1", "1", "|"), log)
  }

  @Test
  fun aFloodIsParsedInTurnsOfTheBudget() {
    repeat(10) { backlog.add(ByteArray(4)) }
    // Two chunks a turn (8 ms), so frames and touches come in between.
    assertEquals(5, runPosted())
    assertEquals(10, log.count { it == "4" })
    assertEquals(5, log.count { it == "|" })
  }

  @Test
  fun theEndComesAfterTheOutputBeforeIt() {
    repeat(4) { backlog.add(ByteArray(5)) }
    backlog.end("closed")
    runPosted()
    assertEquals("closed", log.last { it != "|" })
    assertEquals(4, log.count { it == "5" })
  }

  @Test
  fun outputAfterAQuietSpellPostsAgain() {
    backlog.add(ByteArray(1))
    runPosted()
    backlog.add(ByteArray(1))
    assertEquals(1, posted.size)
  }
}
