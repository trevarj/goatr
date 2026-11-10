package dev.herdr.terminal

import android.view.KeyEvent

/**
 * The clipboard's keys, as a Linux terminal's (and herdr-app's): Ctrl+Shift+V
 * and Shift+Insert paste, Ctrl+Shift+C and Ctrl+Insert copy. Ctrl+V and
 * Ctrl+C are the program's.
 */
internal object ClipboardKeys {
  const val NONE = 0
  const val COPY = 1
  const val PASTE = 2

  /** What a key press with [metaState] does with the clipboard. */
  fun actionFor(keyCode: Int, metaState: Int): Int {
    val ctrl = metaState and KeyEvent.META_CTRL_ON != 0
    val shift = metaState and KeyEvent.META_SHIFT_ON != 0
    val alt = metaState and (KeyEvent.META_ALT_ON or KeyEvent.META_META_ON) != 0
    if (alt) return NONE
    return when {
      keyCode == KeyEvent.KEYCODE_V && ctrl && shift -> PASTE
      keyCode == KeyEvent.KEYCODE_C && ctrl && shift -> COPY
      keyCode == KeyEvent.KEYCODE_INSERT && shift && !ctrl -> PASTE
      keyCode == KeyEvent.KEYCODE_INSERT && ctrl && !shift -> COPY
      else -> NONE
    }
  }
}
