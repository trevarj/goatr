package dev.herdr.terminal

import java.nio.ByteBuffer

/**
 * JNI bindings for libghostty-vt (see native/src/jni.zig, which registers
 * these by name when the library loads).
 *
 * All functions must be called from the main thread; the native side keeps
 * per-handle state without locking.
 */
internal object GhosttyVt {
  init {
    System.loadLibrary("herdrterminal")
  }

  /** Create a terminal session. Returns 0 on failure. */
  external fun create(cols: Int, rows: Int, maxScrollback: Long): Long

  external fun free(handle: Long)

  /**
   * Feed PTY output bytes through the VT parser. Returns query-response bytes
   * the terminal wants written back to the PTY (DSR etc.), or null.
   */
  external fun write(handle: Long, data: ByteArray): ByteArray?

  external fun resize(handle: Long, cols: Int, rows: Int, cellWidthPx: Int, cellHeightPx: Int)

  /**
   * Set the terminal's default colors. Colors use ghostty config syntax
   * (hex with or without '#', X11 names); null clears back to the built-in
   * default, invalid strings are logged and skipped. [palette] overrides
   * entries by index on top of ghostty's default 256-color palette.
   */
  external fun setTheme(
    handle: Long,
    foreground: String?,
    background: String?,
    cursor: String?,
    palette: Array<String?>?
  )

  /** Parse a ghostty-syntax color to ARGB, or 0 when invalid. */
  external fun parseColor(color: String): Int

  /** [parseColor], with null for a null or invalid color. */
  fun colorOrNull(color: String?): Int? = color?.let(::parseColor)?.takeIf { it != 0 }

  /** Scroll the viewport by [deltaRows]; negative is up (into scrollback). */
  external fun scroll(handle: Long, deltaRows: Int)

  external fun scrollToBottom(handle: Long)

  /** Where a vertical swipe goes: 0 scrollback, 1 mouse wheel, 2 arrow keys. */
  external fun scrollTarget(handle: Long): Int

  /** Marks every row for the next [snapshot]. */
  external fun redraw(handle: Long)

  /** [count] wheel notches at a pixel position, for the active mouse mode. */
  external fun encodeWheel(
    handle: Long,
    up: Boolean,
    count: Int,
    x: Float,
    y: Float,
    screenWidth: Int,
    screenHeight: Int,
    cellWidth: Int,
    cellHeight: Int
  ): ByteArray?

  /**
   * A pointer's press (0), release (1) or motion (2) of [button] (0 none,
   * 1 left, 2 right, 3 middle) at a pixel position, for a program that tracks
   * the mouse; null when it encodes to nothing (motion within a cell).
   */
  external fun encodeMouse(
    handle: Long,
    action: Int,
    button: Int,
    metaState: Int,
    x: Float,
    y: Float,
    screenWidth: Int,
    screenHeight: Int,
    cellWidth: Int,
    cellHeight: Int,
    anyPressed: Boolean
  ): ByteArray?

  /**
   * Update the render state and flatten dirty rows into [buffer] (direct,
   * little-endian; layout documented in native/src/session.zig). Returns the number
   * of row records written, or -1 on error/too-small buffer.
   */
  external fun snapshot(handle: Long, buffer: ByteBuffer): Int

  /**
   * Encode a key event ([action]: 0 release / 1 press / 2 repeat) into the
   * escape-sequence bytes to send to the PTY, or null when the key encodes
   * to nothing under the terminal's active modes. [consumedMetaState] is the
   * part of [metaState] that went into [utf8] (Shift for ':').
   */
  external fun encodeKey(
    handle: Long,
    keyCode: Int,
    action: Int,
    metaState: Int,
    consumedMetaState: Int,
    unshiftedCodepoint: Int,
    utf8: ByteArray?
  ): ByteArray?

  /** Select the word at a viewport cell; false when there is nothing there. */
  external fun selectWord(handle: Long, col: Int, row: Int): Boolean

  external fun selectAll(handle: Long): Boolean

  /** Install a linear selection between two viewport cells (inclusive). */
  external fun setSelection(
    handle: Long,
    anchorCol: Int,
    anchorRow: Int,
    col: Int,
    row: Int
  ): Boolean

  external fun clearSelection(handle: Long)

  /** Active selection as UTF-8 (plain, unwrapped, trimmed), or null if none. */
  external fun selectionText(handle: Long): ByteArray?

  /**
   * The link at a viewport cell as UTF-8: its OSC 8 hyperlink, or else an
   * http(s) URL written across its line (soft wraps included); null if none.
   */
  external fun linkAt(handle: Long, col: Int, row: Int): ByteArray?

  /** The cells the link at a viewport cell covers: [first, last], row-major (y * cols + x); null when none. */
  external fun linkSpan(handle: Long, col: Int, row: Int): IntArray?

  /** Text a program copied (OSC 52) since the last call, as UTF-8, or null. */
  external fun takeClipboard(handle: Long): ByteArray?

  /** Whether programs may copy to the clipboard (default true). */
  external fun setClipboardWrite(handle: Long, allowed: Boolean)

  /** Conservative check: newlines / bracketed-paste escapes are unsafe. */
  external fun isPasteSafe(data: ByteArray): Boolean

  /** Encode clipboard bytes for the PTY (control-byte strip + mode-2004 wrap). */
  external fun encodePaste(handle: Long, data: ByteArray): ByteArray?
}
