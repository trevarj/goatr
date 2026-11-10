package dev.herdr.terminal

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class FramePacerTest {
  private var clock = 1_000L
  private val posts = mutableListOf<Long>()
  private val pacer = FramePacer({ clock }, { posts += it })

  /** Runs the frames the pacer posted, as the vsyncs after their delays would. */
  private fun vsync(): Int {
    val delays = posts.toList()
    posts.clear()
    var built = 0
    for (delay in delays.sorted()) {
      clock += delay
      if (pacer.fired()) built++
    }
    return built
  }

  @Test
  fun onScreenAFramePerVsyncHoweverManyAsk() {
    pacer.attach()
    repeat(5) { pacer.request() }
    assertEquals(listOf(0L), posts)
    assertEquals(1, vsync())
  }

  @Test
  fun offScreenNoFramesUntilShownAgain() {
    pacer.attach()
    pacer.detach()
    repeat(100) { pacer.request(); pacer.output() }
    assertEquals(emptyList(), posts)
    assertTrue(pacer.attach(), "missed some: one now, of every row")
    assertFalse(pacer.attach(), "only once")
  }

  @Test
  fun aFramePostedBeforeGoingOffScreenIsSkipped() {
    pacer.attach()
    pacer.request()
    pacer.detach()
    assertFalse(pacer.fired())
  }

  @Test
  fun aFloodIsDrawnAboutThirtyTimesASecond() {
    pacer.attach()
    var frames = 0
    // A second of output arriving every 8 ms (a 120 Hz display's vsyncs).
    val end = clock + 1000
    while (clock < end) {
      pacer.output()
      if (posts.isNotEmpty() && posts.first() == 0L) frames += vsync()
      clock += 8
      if (posts.isNotEmpty()) {
        // A delayed frame comes due.
        val due = posts.first()
        if (due <= 8) frames += vsync()
        else posts[0] = due - 8
      }
    }
    assertTrue(frames in 25..35, "$frames frames")
  }

  @Test
  fun outputAfterAQuietSpellIsDrawnRightAway() {
    pacer.attach()
    pacer.output()
    assertEquals(listOf(0L), posts)
  }

  @Test
  fun outputJustAfterAFrameWaitsItsTurn() {
    pacer.attach()
    pacer.output()
    vsync()
    clock += 5
    pacer.output()
    assertEquals(listOf(FramePacer.OUTPUT_FRAME_MS - 5), posts)
  }

  @Test
  fun typingIsDrawnRightAwayEvenInAFlood() {
    pacer.attach()
    pacer.output()
    vsync()
    clock += 5
    pacer.input()
    pacer.output()
    assertEquals(listOf(0L), posts)
  }

  @Test
  fun aRequestMovesAWaitingFrameUp() {
    pacer.attach()
    pacer.output()
    vsync()
    clock += 5
    pacer.output()
    pacer.request()
    assertEquals(listOf(FramePacer.OUTPUT_FRAME_MS - 5, 0L), posts)
    // One frame now; the later turn finds nothing to build.
    assertEquals(1, vsync())
  }

  @Test
  fun aTurnBeforeItsFrameIsDueLeavesItForTheNext() {
    pacer.attach()
    pacer.output()
    vsync()
    clock += 5
    pacer.output()
    posts.clear()
    // The delayed frame's vsync comes a millisecond early: no frame yet, but
    // one on the next vsync, and output meanwhile isn't lost.
    clock += FramePacer.OUTPUT_FRAME_MS - 5 - 1
    assertFalse(pacer.fired())
    assertEquals(listOf(0L), posts)
    clock += 1
    assertTrue(pacer.fired())
  }
}
