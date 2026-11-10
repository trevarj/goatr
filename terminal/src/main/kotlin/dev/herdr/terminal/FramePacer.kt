package dev.herdr.terminal

/**
 * When the view builds a frame (a snapshot, the rows that changed): [post]
 * schedules one (on the vsync after a delay in ms, 0 the next) and [fired]
 * runs it. As herdr-app paces its terminals:
 *
 * - Something asked for ([request]: a scroll, a selection, a resize) is on
 *   the next vsync.
 * - Output ([output]) is drawn at most every [OUTPUT_FRAME_MS], so a flood
 *   isn't parsed and drawn again 60 to 120 times a second; but right away
 *   when nothing was drawn for that long (an echo, a prompt), and for
 *   [TYPING_MS] after you type ([input]), so typing never waits.
 * - Never while the view is off screen: a kept terminal still parses its
 *   output but builds no frames until it's shown again ([attach] says
 *   whether it missed any).
 */
internal class FramePacer(
  private val now: () -> Long,
  private val post: (delayMs: Long) -> Unit,
) {
  private var posted = false
  // When the posted frame is due (it may be moved up, not back).
  private var due = 0L
  private var attached = false
  private var missed = false
  private var lastFrame = Long.MIN_VALUE / 2
  private var lastInput = Long.MIN_VALUE / 2

  /** Something changed: a frame on the next vsync, or once on screen again. */
  fun request() = schedule(0)

  /** Output came in: a frame when the output's pace allows. */
  fun output() {
    val at = now()
    val wait = if (at - lastInput < TYPING_MS) 0 else maxOf(0, lastFrame + OUTPUT_FRAME_MS - at)
    schedule(wait)
  }

  /** You typed: output that answers it is drawn right away. */
  fun input() {
    lastInput = now()
  }

  private fun schedule(delayMs: Long) {
    if (!attached) {
      missed = true
      return
    }
    val at = now() + delayMs
    if (posted && due <= at) return
    posted = true
    due = at
    post(delayMs)
  }

  /** A posted frame's turn: whether to build one (a frame moved up leaves a later turn with none). */
  fun fired(): Boolean {
    if (!posted || !attached) return false
    val at = now()
    // A vsync a little before it's due: the next one, not none (it's still posted).
    if (at < due) {
      post(0)
      return false
    }
    posted = false
    lastFrame = at
    return true
  }

  /** On screen: true when frames were asked for meanwhile (build one now, every row). */
  fun attach(): Boolean {
    attached = true
    return missed.also { missed = false }
  }

  fun detach() {
    attached = false
    posted = false
  }

  /** Whether a frame is posted (to cancel it). */
  val pending get() = posted

  companion object {
    /** Output's frames at most this often: about 30 a second, as herdr-app's. */
    const val OUTPUT_FRAME_MS = 33L

    /** For this long after a key, output is drawn on the next vsync. */
    const val TYPING_MS = 200L
  }
}
