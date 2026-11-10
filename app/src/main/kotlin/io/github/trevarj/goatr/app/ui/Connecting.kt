// The hosts as they connect, all at once, each on its own: the screen the app
// opens on, and the terminal's while nothing it could show has arrived.

package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.trevarj.goatr.app.session.Host
import io.github.trevarj.goatr.app.session.HostState
import io.github.trevarj.goatr.app.session.shortName
import herdr.core.Connection
import kotlinx.coroutines.delay

/** How long the checks stay up once the first host is in, before the app shows. */
private const val SETTLE = 500L

private fun trying(c: Connection?) = c == null || c == Connection.Idle || c == Connection.Connecting

/**
 * What the app shows first, in its content (the tabs and sidebar stay): every
 * host connecting, a check as each gets in. Half a second after the first one does, [onReady]: the others
 * keep connecting behind the app. Every host failed: retry, or go on anyway.
 */
@Composable
fun LoadingScreen(hosts: List<Host>?, states: Map<String, HostState>, onRetry: (String) -> Unit, onReady: () -> Unit) {
    val on = hosts.orEmpty().filter { !it.off }
    val none = hosts != null && on.isEmpty()
    val up = on.any { states[it.id]?.connection == Connection.Connected }
    val failed = on.isNotEmpty() && on.none { trying(states[it.id]?.connection) } && !up
    // Keyed on whether any is in, not which: more getting in doesn't wait longer.
    LaunchedEffect(up, none) {
        if (none) onReady()
        else if (up) {
            delay(SETTLE)
            onReady()
        }
    }
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(Modifier.widthIn(max = 320.dp).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(
                Lucide.Plug,
                null,
                Modifier.size(32.dp),
                tint = if (failed) herdr.blocked else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                if (failed) "Can't connect" else "Connecting",
                Modifier.padding(top = 16.dp, bottom = 20.dp),
                style = MaterialTheme.typography.titleMedium,
            )
            HostRows(on, states, onRetry)
            if (failed) TextButton(onReady, Modifier.padding(top = 16.dp)) { Text("Continue") }
        }
    }
}

/** One row a host: a spinner while it tries, a check once in, Retry when it failed. */
@Composable
fun HostRows(hosts: List<Host>, states: Map<String, HostState>, onRetry: (String) -> Unit) {
    val dim = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        for (h in hosts) {
            val c = states[h.id]?.connection ?: Connection.Idle
            // A host still trying after a while says so.
            var slow by remember(h.id) { mutableStateOf(false) }
            LaunchedEffect(h.id, trying(c)) {
                slow = false
                if (trying(c)) {
                    delay(8_000)
                    slow = true
                }
            }
            Row(
                Modifier.fillMaxWidth().height(40.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(Modifier.size(18.dp), contentAlignment = Alignment.Center) {
                    when (c) {
                        Connection.Connected -> Icon(Lucide.Check, "connected", Modifier.size(18.dp), tint = herdr.connected)
                        Connection.Lost -> Icon(Lucide.X, "can't reach", Modifier.size(16.dp), tint = herdr.blocked)
                        else -> CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    }
                }
                Text(shortName(h), Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                when {
                    c == Connection.Lost -> TextButton({ onRetry(h.id) }) { Text("Retry") }
                    c == Connection.Connected -> Text("connected", style = MaterialTheme.typography.bodySmall, color = dim)
                    else -> Text(if (slow) "taking a while" else "connecting…", style = MaterialTheme.typography.bodySmall, color = dim)
                }
            }
        }
    }
}
