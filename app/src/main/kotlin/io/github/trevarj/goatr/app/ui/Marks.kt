// The few pieces Material doesn't have: the desktop's attention marks and
// space avatars, the connection badge in the top bar, and the app's dialog
// (a Material AlertDialog drawn from a ViewModel's DialogState).

package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.trevarj.goatr.app.DialogState
import io.github.trevarj.goatr.app.session.Connection
import io.github.trevarj.goatr.app.session.Machine
import io.github.trevarj.goatr.app.session.MachineState
import herdr.core.Attention
import kotlinx.coroutines.delay

val SECTION = mapOf(
    Attention.Blocked to "Needs you",
    Attention.Done to "Done",
    Attention.Working to "Working",
    Attention.Idle to "Idle",
)

@Composable
fun tone(a: Attention): Color = when (a) {
    Attention.Blocked -> herdr.blocked
    Attention.Done -> herdr.done
    Attention.Working -> herdr.working
    Attention.Idle -> MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * How an agent's attention looks, as in the desktop's agents list: a spinner
 * while working, "Needs you" when blocked, a dot when done, a ring when idle.
 */
@Composable
fun StatusMark(attention: Attention?, compact: Boolean = false) {
    val label = Modifier.semantics { attention?.let { contentDescription = SECTION.getValue(it) } }
    when (attention) {
        Attention.Working -> Spinner(label.size(14.dp))
        Attention.Blocked -> Box(
            label.clip(RoundedCornerShape(if (compact) 4.dp else 6.dp))
                .background(herdr.blocked.copy(alpha = 0.15f))
                .then(if (compact) Modifier.size(16.dp) else Modifier.padding(horizontal = 6.dp, vertical = 2.dp)),
            contentAlignment = Alignment.Center,
        ) {
            Text(if (compact) "!" else "Needs you", color = herdr.blocked, fontSize = 11.sp, fontWeight = FontWeight.Bold)
        }
        Attention.Done -> Box(label.padding(horizontal = 3.dp).size(8.dp).clip(CircleShape).background(herdr.done))
        Attention.Idle -> Box(label.padding(horizontal = 3.dp).size(8.dp).border(1.5.dp, herdr.idle, CircleShape))
        null -> Box(Modifier.padding(horizontal = 3.dp).size(8.dp))
    }
}

/**
 * A quarter arc on its track, turning in steps. Material's spinner redraws
 * every frame, 60 a second, for a 14dp mark: a fifth of a core per screen.
 * This one draws its arc once and only rotates it, 15 times a second.
 */
@Composable
private fun Spinner(modifier: Modifier) {
    var turn by remember { mutableFloatStateOf(0f) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000L / 15)
            turn = (turn + 24f) % 360f
        }
    }
    val color = herdr.working
    Canvas(modifier.graphicsLayer { rotationZ = turn }) {
        val stroke = Stroke(2.dp.toPx(), cap = StrokeCap.Round)
        val inset = stroke.width / 2
        val arc = Size(size.width - stroke.width, size.height - stroke.width)
        drawArc(color.copy(alpha = 0.24f), 0f, 360f, false, Offset(inset, inset), arc, style = stroke)
        drawArc(color, -90f, 90f, false, Offset(inset, inset), arc, style = stroke)
    }
}

// The desktop's avatar colors in its order (spaces mod): one strong color per
// hue, light and dark alike. A space keeps its color while its position does.
private val AVATARS = listOf(
    Color(0xFF22A99A),
    Color(0xFFE3814F),
    Color(0xFF5B8DEF),
    Color(0xFF8B7CF6),
    Color(0xFF3FAE6A),
    Color(0xFFD9649A),
)

@Composable
fun SpaceAvatar(name: String, avatar: Int, size: Dp = 24.dp) {
    val fill = AVATARS[avatar % AVATARS.size]
    Box(
        Modifier.size(size).clip(RoundedCornerShape(size * 0.28f)).background(fill),
        contentAlignment = Alignment.Center,
    ) {
        val first = if (name.isEmpty()) "?" else String(Character.toChars(name.codePointAt(0))).uppercase()
        // Its line as tall as the mark, so the letter sits in the middle at any size
        // (the inherited 24sp line only fits the 24dp mark).
        Text(first, color = Color.White, fontSize = (size.value * 0.55f).sp, lineHeight = size.value.sp, fontWeight = FontWeight.Bold)
    }
}

/** Where the phone is connected: a dot in the connection's color and the host's name. */
@Composable
fun ConnectionBadge(host: String, connection: Connection, end: Dp = 16.dp) {
    val dot = when (connection) {
        Connection.Idle -> MaterialTheme.colorScheme.onSurfaceVariant
        Connection.Connected -> herdr.connected
        Connection.Connecting, Connection.Partial -> herdr.working
        Connection.Lost -> herdr.blocked
    }
    DotLabel(host, connection.label, dot, end)
}

/** The same badge for one machine: where a terminal runs, and whether it's reachable. */
@Composable
fun MachineBadge(machine: Machine) {
    val dot = when (machine.state) {
        MachineState.Connected -> herdr.connected
        MachineState.Connecting -> herdr.working
        MachineState.Asleep -> MaterialTheme.colorScheme.onSurfaceVariant
        MachineState.Lost -> herdr.blocked
    }
    DotLabel(machine.label, machine.state.label, dot)
}

@Composable
private fun DotLabel(label: String, state: String, dot: Color, end: Dp = 16.dp) {
    Row(
        Modifier.padding(end = end).semantics { contentDescription = "$state $label" },
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(6.dp).clip(CircleShape).background(dot))
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** One clock for every "4m" on screen, ticking once a minute. */
@Composable
fun rememberNow(): Long {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(60_000)
            now = System.currentTimeMillis()
        }
    }
    return now
}

@Composable
fun AppDialog(state: DialogState?, onDismiss: () -> Unit) {
    val d = state ?: return
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(d.title) },
        text = d.detail?.takeIf { it.isNotEmpty() }?.let { { Text(it) } },
        confirmButton = {
            if (d.confirm != null) {
                TextButton(
                    d.onConfirm,
                    colors = if (d.destructive) ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
                    else ButtonDefaults.textButtonColors(),
                ) { Text(d.confirm) }
            } else {
                TextButton(onDismiss) { Text("OK") }
            }
        },
        dismissButton = if (d.confirm != null) ({ TextButton(onDismiss) { Text("Cancel") } }) else null,
    )
}
