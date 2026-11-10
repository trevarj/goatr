package dev.herdr.terminal

import android.view.KeyEvent.KEYCODE_C
import android.view.KeyEvent.KEYCODE_INSERT
import android.view.KeyEvent.KEYCODE_V
import android.view.KeyEvent.META_ALT_ON
import android.view.KeyEvent.META_CTRL_ON
import android.view.KeyEvent.META_SHIFT_ON
import dev.herdr.terminal.ClipboardKeys.COPY
import dev.herdr.terminal.ClipboardKeys.NONE
import dev.herdr.terminal.ClipboardKeys.PASTE
import dev.herdr.terminal.ClipboardKeys.actionFor
import org.junit.Test
import kotlin.test.assertEquals

class ClipboardKeysTest {
  @Test
  fun pasteKeys() {
    assertEquals(PASTE, actionFor(KEYCODE_V, META_CTRL_ON or META_SHIFT_ON))
    assertEquals(PASTE, actionFor(KEYCODE_INSERT, META_SHIFT_ON))
  }

  @Test
  fun copyKeys() {
    assertEquals(COPY, actionFor(KEYCODE_C, META_CTRL_ON or META_SHIFT_ON))
    assertEquals(COPY, actionFor(KEYCODE_INSERT, META_CTRL_ON))
  }

  @Test
  fun theProgramKeepsTheRest() {
    // Ctrl+C interrupts, Ctrl+V is ^V; V alone and Alt chords are typing.
    assertEquals(NONE, actionFor(KEYCODE_C, META_CTRL_ON))
    assertEquals(NONE, actionFor(KEYCODE_V, META_CTRL_ON))
    assertEquals(NONE, actionFor(KEYCODE_V, 0))
    assertEquals(NONE, actionFor(KEYCODE_V, META_CTRL_ON or META_SHIFT_ON or META_ALT_ON))
  }
}
