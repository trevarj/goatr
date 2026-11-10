// A row's menu, as the desktop's right-click menu (herdr-app mods/menu): a
// long press or a right click opens it, its items each with their icon, Close
// last under a rule. Spaces also drag to a new place (spaces/main.js): a long
// press, then moving without lifting; a mouse just drags.

package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import kotlin.math.abs

/** One item of a row's menu; `close` ones go last, under a rule. */
class MenuItem(val label: String, val icon: @Composable () -> Painter, val close: Boolean = false, val onClick: () -> Unit)

/** A dragged row's moves: `to` is how far (px) below where it was picked up it is now. */
interface RowDrag {
    fun start()
    fun to(offset: Float)
    fun end()
    fun cancel()
}

/** Where a row is in the window, kept for its gestures. */
private class Placed {
    var at: LayoutCoordinates? = null

    /** A pointer's height in the window: unlike its place in the row, it doesn't move with a dragged row. */
    fun y(local: Offset) = at?.takeIf { it.isAttached }?.localToWindow(local)?.y ?: local.y
}

/**
 * Opens `onMenu` on a long press or a right click, and, with `drag`, moves the
 * row: a finger after a long press (lifting without moving opens the menu
 * instead), a mouse at once. Goes after the row's clickable, so a long press
 * never also counts as a tap.
 */
fun Modifier.rowMenu(label: String, onMenu: () -> Unit, drag: RowDrag? = null): Modifier = composed {
    val placed = remember { Placed() }
    onGloballyPositioned { placed.at = it }
        // What a screen reader offers for the long press.
        .semantics { onLongClick(label) { onMenu(); true } }
        .pointerInput(onMenu, drag) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                val slop = viewConfiguration.touchSlop
                val from = placed.y(down.position)
                if (down.type == PointerType.Mouse) {
                    if (currentEvent.buttons.isSecondaryPressed) {
                        down.consume()
                        onMenu()
                        return@awaitEachGesture
                    }
                    if (drag == null) return@awaitEachGesture
                    var dragging = false
                    while (true) {
                        val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                        val offset = placed.y(change.position) - from
                        if (!change.pressed) {
                            if (dragging) {
                                change.consume()
                                drag.end()
                            }
                            break
                        }
                        if (!dragging && abs(offset) > slop) {
                            dragging = true
                            drag.start()
                        }
                        if (dragging) {
                            drag.to(offset)
                            change.consume()
                        }
                    }
                    return@awaitEachGesture
                }
                // A finger: lifted or moved (a tap, a scroll) before the long press is someone else's.
                val early = withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                    do {
                        val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id }
                    } while (change != null && change.pressed && (change.position - down.position).getDistance() <= slop)
                    true
                }
                if (early != null) return@awaitEachGesture
                drag?.start()
                var moved = false
                while (true) {
                    val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                    change.consume()
                    if (!change.pressed) break
                    val offset = placed.y(change.position) - from
                    if (abs(offset) > slop) moved = true
                    drag?.to(offset)
                }
                if (moved && drag != null) drag.end() else {
                    drag?.cancel()
                    onMenu()
                }
            }
        }
}

/** The menu itself, anchored to the row it's in a Box with. */
@Composable
fun RowMenu(expanded: Boolean, onDismiss: () -> Unit, items: List<MenuItem>) {
    // On the popover color with a border, as the desktop's menus.
    DropdownMenu(
        expanded,
        onDismiss,
        shape = RoundedCornerShape(8.dp),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        val (rest, close) = items.partition { !it.close }
        for (item in rest + close) {
            if (item.close && rest.isNotEmpty() && item === close.first()) HorizontalDivider(Modifier.padding(vertical = 4.dp))
            DropdownMenuItem(
                text = { Text(item.label) },
                leadingIcon = {
                    Icon(item.icon(), null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                },
                onClick = {
                    onDismiss()
                    item.onClick()
                },
            )
        }
    }
}

/**
 * Asks for a new name (window.rs `ask_name`): it starts with the name someone
 * gave, empty for an automatic one, which `hint` says what empty means.
 */
@Composable
fun RenameDialog(title: String, current: String, hint: String, onDismiss: () -> Unit, onRename: (String) -> Unit) {
    var value by remember { mutableStateOf(TextFieldValue(current, TextRange(0, current.length))) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val done = {
        onDismiss()
        onRename(value.text)
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value,
                { value = it },
                Modifier.focusRequester(focus),
                placeholder = { Text(hint) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { done() }),
            )
        },
        confirmButton = { TextButton(done) { Text("Rename") } },
        dismissButton = { TextButton(onDismiss) { Text("Cancel") } },
    )
}
