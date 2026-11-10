package dev.herdr.terminal

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.SystemClock
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.KeyEvent
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Two fixed rows of extra keys above the soft keyboard, Termux's default set
 * plus paste and enter:
 *
 *   ESC  /    -    HOME  ↑  END  PGUP  PASTE
 *   TAB  CTRL ALT  ←     ↓  →    PGDN  ENTER
 *
 * Every key fits on screen (no scrolling). Keys go through ghostty's encoder,
 * so arrows and HOME/END follow the program's keyboard modes. CTRL/ALT are
 * sticky: a tap arms them for the next key, a quick double tap locks them
 * (bottom indicator) until tapped again. Colors come from the host app
 * ([setColors]).
 */
internal class TerminalAccessoryBar(context: Context) : LinearLayout(context) {
  var onKey: ((keyCode: Int) -> Unit)? = null
  var onText: ((text: String) -> Unit)? = null
  var onPaste: (() -> Unit)? = null

  private enum class Activation { INACTIVE, ARMED, LOCKED }

  private val keys = mutableListOf<KeyButton>()
  private var colors = Colors()

  /** A sticky modifier key and the KeyEvent meta bits it adds while lit. */
  private inner class Sticky(label: String, val meta: Int) {
    var state = Activation.INACTIVE
    private var lastTap = 0L
    val button = text(label) { tap() }

    private fun tap() {
      val now = SystemClock.uptimeMillis()
      state = when (state) {
        Activation.INACTIVE -> Activation.ARMED
        Activation.ARMED -> if (now - lastTap < DOUBLE_TAP_MS) Activation.LOCKED else Activation.INACTIVE
        Activation.LOCKED -> Activation.INACTIVE
      }
      lastTap = now
      syncModifierVisuals()
    }
  }

  private val ctrl = Sticky("CTRL", KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON)
  private val alt = Sticky("ALT", KeyEvent.META_ALT_ON or KeyEvent.META_ALT_LEFT_ON)
  private val stickies = listOf(ctrl, alt)

  /** The bar's colors as ARGB ints. */
  data class Colors(
    val background: Int = 0xFF101114.toInt(),
    val key: Int = 0xFF2C2E31.toInt(),
    val foreground: Int = 0xFFEEEEEF.toInt(),
    val activeKey: Int = 0xFFEEEEEF.toInt(),
    val activeForeground: Int = 0xFF101114.toInt(),
  )

  init {
    orientation = VERTICAL
    val pad = dp(PADDING_DP)
    setPadding(pad, pad, pad, pad)
    row(
      text("ESC") { onKey?.invoke(KeyEvent.KEYCODE_ESCAPE) },
      text("/") { onText?.invoke("/") },
      text("-") { onText?.invoke("-") },
      text("HOME") { onKey?.invoke(KeyEvent.KEYCODE_MOVE_HOME) },
      text("↑") { onKey?.invoke(KeyEvent.KEYCODE_DPAD_UP) },
      text("END") { onKey?.invoke(KeyEvent.KEYCODE_MOVE_END) },
      text("PGUP") { onKey?.invoke(KeyEvent.KEYCODE_PAGE_UP) },
      text("PASTE") { onPaste?.invoke() },
    )
    row(
      text("TAB") { onKey?.invoke(KeyEvent.KEYCODE_TAB) },
      ctrl.button,
      alt.button,
      text("←") { onKey?.invoke(KeyEvent.KEYCODE_DPAD_LEFT) },
      text("↓") { onKey?.invoke(KeyEvent.KEYCODE_DPAD_DOWN) },
      text("→") { onKey?.invoke(KeyEvent.KEYCODE_DPAD_RIGHT) },
      text("PGDN") { onKey?.invoke(KeyEvent.KEYCODE_PAGE_DOWN) },
      text("ENTER") { onKey?.invoke(KeyEvent.KEYCODE_ENTER) },
    )
    applyColors()
  }

  fun setColors(next: Colors) {
    colors = next
    applyColors()
  }

  /** The terminal's font, so the labels match the grid. */
  fun setLabelTypeface(typeface: Typeface) {
    for (key in keys) key.typeface = typeface
  }

  /** KeyEvent meta bits for the currently lit sticky modifiers. */
  fun stickyMetaState(): Int =
    stickies.filter { it.state != Activation.INACTIVE }.fold(0) { meta, sticky -> meta or sticky.meta }

  /** A key consumed the modifiers: armed ones unlatch, locked ones persist. */
  fun consumeSticky() {
    for (sticky in stickies) if (sticky.state == Activation.ARMED) sticky.state = Activation.INACTIVE
    syncModifierVisuals()
  }

  /** [stickyMetaState], consumed by the key it's returned for. */
  fun takeStickyMetaState(): Int = stickyMetaState().also { if (it != 0) consumeSticky() }

  private fun applyColors() {
    setBackgroundColor(colors.background)
    syncModifierVisuals()
  }

  private fun syncModifierVisuals() {
    for (key in keys) {
      val state = stickies.find { it.button === key }?.state ?: Activation.INACTIVE
      key.paint(colors, active = state != Activation.INACTIVE, locked = state == Activation.LOCKED)
    }
  }

  private fun text(label: String, onTap: () -> Unit): KeyButton {
    val key = KeyButton(context)
    key.text = label
    key.setOnClickListener {
      it.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
      onTap()
    }
    keys += key
    return key
  }

  private fun row(vararg buttons: KeyButton) {
    val row = LinearLayout(context).apply { orientation = HORIZONTAL }
    val gap = dp(GAP_DP)
    for ((i, key) in buttons.withIndex()) {
      val params = LayoutParams(0, dp(KEY_HEIGHT_DP), 1f)
      if (i > 0) params.marginStart = gap
      row.addView(key, params)
    }
    val params = LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT)
    if (childCount > 0) params.topMargin = gap
    addView(row, params)
  }

  private fun dp(value: Float): Int = (value * resources.displayMetrics.density).toInt()

  /** Rounded key; locked modifiers draw a bar under the label. */
  private class KeyButton(context: Context) : TextView(context) {
    private var locked = false
    private val lockPaint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val lockRect = RectF()
    private val shape = GradientDrawable().apply {
      cornerRadius = KEY_RADIUS_DP * context.resources.displayMetrics.density
    }

    init {
      typeface = Typeface.MONOSPACE
      textSize = KEY_TEXT_SIZE_SP
      gravity = Gravity.CENTER
      includeFontPadding = false
      maxLines = 1
      background = shape
    }

    fun paint(colors: Colors, active: Boolean, locked: Boolean) {
      this.locked = locked
      shape.setColor(if (active) colors.activeKey else colors.key)
      setTextColor(if (active) colors.activeForeground else colors.foreground)
      lockPaint.color = currentTextColor
      invalidate()
    }

    override fun onDraw(canvas: Canvas) {
      super.onDraw(canvas)
      if (!locked) return
      val density = resources.displayMetrics.density
      val barWidth = 12f * density
      val barHeight = 2f * density
      lockRect.set(
        (width - barWidth) / 2f,
        height - barHeight - 3f * density,
        (width + barWidth) / 2f,
        height - 3f * density
      )
      canvas.drawRoundRect(lockRect, barHeight / 2f, barHeight / 2f, lockPaint)
    }
  }

  private companion object {
    const val KEY_HEIGHT_DP = 36f
    const val KEY_RADIUS_DP = 6f
    const val GAP_DP = 4f
    const val PADDING_DP = 4f
    const val DOUBLE_TAP_MS = 300L
    const val KEY_TEXT_SIZE_SP = 12f
  }
}
