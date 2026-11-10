package dev.herdr.terminal

/**
 * The grid's rows' drawings (a RenderNode each in the view) from frame to
 * frame. A frame re-records some rows, moves others (the viewport scrolled:
 * the drawing of the row it shows now, from where it was) and keeps the
 * rest as they are. A drawing is recorded at its row's place in the grid;
 * moved, it's shown [place]d by how many rows it moved since it was recorded.
 * Drawings no row needs any more are reused, then [discard]ed.
 */
internal class RowDrawings<T : Any>(
  private val create: () -> T,
  private val discard: (T) -> Unit,
  private val place: (T, rowsMoved: Int) -> Unit,
) {
  private var drawings: MutableList<T?> = mutableListOf()
  // The row each drawing was recorded at.
  private var recordedAt = IntArray(0)

  val size get() = drawings.size

  operator fun get(row: Int): T? = drawings[row]

  /** A grid of [rows]: every drawing let go. */
  fun reset(rows: Int) {
    for (d in drawings) d?.let(discard)
    drawings = MutableList(rows) { null }
    recordedAt = IntArray(rows) { it }
  }

  /**
   * One frame: row y shows the drawing that was at [movedFrom] y (-1: none),
   * or is re-recorded when [recorded] y ([record] draws it into the drawing
   * given), or else stays as it is.
   */
  fun frame(movedFrom: IntArray, recorded: BooleanArray, record: (row: Int, T) -> Unit) {
    val rows = drawings.size
    val old = drawings.toList()
    val oldAt = recordedAt.copyOf()
    // The drawings still shown (moved or kept); the others are free.
    val used = BooleanArray(rows)
    for (y in 0 until rows) {
      if (movedFrom[y] >= 0) used[movedFrom[y]] = true
      else if (!recorded[y]) used[y] = true
    }
    val free = ArrayDeque<T>()
    for (y in 0 until rows) if (!used[y]) old[y]?.let(free::add)
    for (y in 0 until rows) {
      val from = movedFrom[y]
      when {
        from >= 0 -> {
          val d = old[from] ?: continue
          drawings[y] = d
          recordedAt[y] = oldAt[from]
          place(d, y - oldAt[from])
        }
        recorded[y] -> {
          // Its own drawing when it's free (no row took it), else any free one.
          val own = old[y]
          val d = if (own != null && !used[y] && free.remove(own)) own else free.removeFirstOrNull() ?: create()
          drawings[y] = d
          recordedAt[y] = y
          place(d, 0)
          record(y, d)
        }
      }
    }
    for (d in free) discard(d)
  }

  /** Every drawing, to draw in row order. */
  fun forEach(action: (T) -> Unit) {
    for (d in drawings) if (d != null) action(d)
  }
}
