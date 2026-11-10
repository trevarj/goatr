package io.github.trevarj.goatr.app.ui

/**
 * A terminal's output on its way to the main thread: chunks from the
 * stream's thread ([add]), then how it ended ([end]), handed over in order
 * ([write], then [ended]) on the main thread, a turn at a time. A turn takes
 * what came in since the last, but stops after [budgetMs] and leaves the rest
 * for the next turn, so a flood is parsed between frames and touches rather
 * than blocking them until it's all in. (Ghostty parses on a thread of its
 * own; here the terminal is the main thread's.)
 */
internal class Backlog<E : Any>(
  private val now: () -> Long,
  /** Runs a turn on the main thread, after what's queued there. */
  private val post: (Runnable) -> Unit,
  private val budgetMs: Long,
  private val write: (ByteArray) -> Unit,
  private val ended: (E) -> Unit,
  /** After a turn's chunks. */
  private val afterTurn: () -> Unit = {},
) {
  private val queue = ArrayDeque<Any>()
  private var posted = false

  /** Output, from any thread. */
  fun add(bytes: ByteArray) = queue(bytes)

  /** The end, from any thread: after the output before it. */
  fun end(end: E) = queue(end)

  private fun queue(item: Any) {
    val first = synchronized(queue) {
      queue.addLast(item)
      !posted.also { posted = true }
    }
    if (first) post(turn)
  }

  private val turn = object : Runnable {
    override fun run() {
      val start = now()
      var wrote = false
      while (true) {
        val item = synchronized(queue) {
          queue.removeFirstOrNull() ?: run {
            posted = false
            null
          }
        } ?: break
        @Suppress("UNCHECKED_CAST")
        if (item is ByteArray) {
          write(item)
          wrote = true
        } else {
          ended(item as E)
        }
        if (now() - start >= budgetMs) {
          val more = synchronized(queue) { queue.isNotEmpty().also { if (!it) posted = false } }
          if (wrote) afterTurn()
          if (more) post(this)
          return
        }
      }
      if (wrote) afterTurn()
    }
  }
}
