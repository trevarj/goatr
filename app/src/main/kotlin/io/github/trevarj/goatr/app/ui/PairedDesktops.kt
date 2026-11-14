// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.PairingGate
import io.github.trevarj.goatr.PairingPreview
import io.github.trevarj.goatr.PairScanner
import io.github.trevarj.goatr.app.container
import io.github.trevarj.goatr.app.session.IROH_CONNECT_TIMEOUT_SECONDS
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch

/** The transport desktop is selected here; downstream Herdr machines keep their own scoped ids. */
@Composable
fun PairedDesktops() {
    val app = LocalContext.current.container
    val hosts by app.store.hosts.collectAsStateWithLifecycle()
    val prefs by app.store.prefs.collectAsStateWithLifecycle()
    val states by app.live.hostStates.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    var pairing by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<String?>(null) }
    var regions by remember { mutableStateOf<Map<String, String>>(emptyMap()) }
    var relay by remember { mutableStateOf(app.iroh.savedRelay()) }
    var draftRelay by remember { mutableStateOf(relay) }
    var changing by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        try { regions = app.iroh.relayRegions() }
        catch (e: CancellationException) { throw e }
        catch (_: Exception) { message = "Relay catalog unavailable. No custom relay will be substituted." }
    }
    SectionTitle("Paired desktops (iroh)", Lucide.Server, first = true)
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Choose a desktop first. Agents, spaces and terminals remain scoped to the Herdr machine shown; switching desktops never retargets an action.", style = MaterialTheme.typography.bodySmall)
        for (host in hosts.filter { it.peer != null }) {
            OutlinedCard(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(12.dp)) {
                    Text(host.host)
                    Text(host.fingerprint ?: host.peer.orEmpty(), style = MaterialTheme.typography.bodySmall)
                    Text(if (!host.paired) "Pairing response unconfirmed; confirm a fresh invitation to retry."
                        else states[host.id]?.connection?.toString() ?: "Not selected")
                    Row {
                        TextButton(enabled = host.paired, onClick = { scope.launch {
                            app.store.setPrefs { it.copy(desktop = host.peer, terminal = null) }
                        } }) { Text(if (prefs.desktop == host.peer) "Selected desktop" else "Select desktop") }
                        TextButton(onClick = { scope.launch { app.store.removeHost(host.id)
                            if (prefs.desktop == host.peer) app.store.setPrefs { it.copy(desktop = null, terminal = null) }
                        } }) { Text("Remove from phone") }
                    }
                    if (host.paired && prefs.desktop == host.peer) TextButton(onClick = { app.live.reconnect(host.id) }) { Text("Reconnect") }
                }
            }
        }
        Text("Removing here does not revoke the phone on the desktop. Use goatr devices / goatr revoke there.", style = MaterialTheme.typography.bodySmall)
        Button(onClick = { pairing = true }) { Text("Pair a desktop") }
        Text("Iroh connection timeout", style = MaterialTheme.typography.titleSmall)
        Text("5–300 seconds, in 5-second steps. Saved for the next iroh desktop connection attempt, including pairing. Does not change an attempt in progress or reconnect automatically; SSH and invitation expiry are unchanged.", style = MaterialTheme.typography.bodySmall)
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            FilledTonalIconButton(
                onClick = { scope.launch {
                    app.store.setPrefs { it.copy(irohConnectTimeoutSeconds = (it.irohConnectTimeoutSeconds - 5).coerceIn(IROH_CONNECT_TIMEOUT_SECONDS)) }
                } },
                enabled = prefs.irohConnectTimeoutSeconds > IROH_CONNECT_TIMEOUT_SECONDS.first,
            ) { Icon(Lucide.Minus, "Decrease iroh connection timeout by 5 seconds") }
            Text("${prefs.irohConnectTimeoutSeconds} seconds", Modifier.weight(1f), textAlign = TextAlign.Center)
            FilledTonalIconButton(
                onClick = { scope.launch {
                    app.store.setPrefs { it.copy(irohConnectTimeoutSeconds = (it.irohConnectTimeoutSeconds + 5).coerceIn(IROH_CONNECT_TIMEOUT_SECONDS)) }
                } },
                enabled = prefs.irohConnectTimeoutSeconds < IROH_CONNECT_TIMEOUT_SECONDS.last,
            ) { Icon(Lucide.Plus, "Increase iroh connection timeout by 5 seconds") }
        }
        Text("Home relay region", style = MaterialTheme.typography.titleSmall)
        Text("Saved for this phone; changing it disconnects and reconnects every iroh link. SSH is unchanged.", style = MaterialTheme.typography.bodySmall)
        for ((url, label) in listOf(null to "Automatic (recommended)") + regions.map { it.key to it.value }) {
            TextButton(enabled = !changing, onClick = { draftRelay = url }) {
                Text((if (draftRelay == url) "Selected: " else "") + label)
            }
        }
        Button(enabled = !changing && draftRelay != relay, onClick = {
            val selected = draftRelay
            changing = true
            scope.launch {
                try { app.iroh.saveRelay(selected); relay = selected; app.live.foreground() }
                catch (e: CancellationException) { throw e }
                catch (_: Exception) { message = "Relay change failed; choose an available built-in region." }
                finally { changing = false }
            }
        }) { Text(if (changing) "Reconnecting…" else "Save and reconnect") }
        if (relay != null && relay !in regions) Text("Saved relay unavailable. Select Automatic or an available region.")
        message?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }
    if (pairing) PairDesktopDialog(onDismiss = { pairing = false })
}

@Composable
fun PairDesktopDialog(onDismiss: () -> Unit, invitation: String? = null) {
    val app = LocalContext.current.container
    val gate = remember { PairingGate() }
    var preview by remember { mutableStateOf<PairingPreview?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    DisposableEffect(gate) { onDispose { gate.close() } }
    LaunchedEffect(invitation) { if (invitation != null) {
        preview = gate.submit(invitation)
        if (preview == null) error = "Invalid invitation. Nothing was connected."
    } }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
        title = { Text("Pair desktop — full control") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val shown = preview
            if (shown == null) PairScanner(active = !busy) { raw ->
                preview = gate.submit(raw)
                error = if (preview == null) "Invalid invitation. Nothing was connected." else null
                preview != null
            } else {
                Text(shown.name.value)
                Text(shown.fingerprint, style = MaterialTheme.typography.bodySmall)
                Text("Expires: ${java.util.Date(shown.expiresAt.value)}")
                Text(shown.authority)
                Text(shown.relayDisclosure, style = MaterialTheme.typography.bodySmall)
            }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        } },
        confirmButton = { TextButton(enabled = preview != null && !busy, onClick = {
            busy = true
            scope.launch {
                try { gate.confirm(app.iroh); onDismiss() }
                catch (_: Exception) {
                    // Operation deadlines are failures; route/disposal cancellation must still propagate.
                    currentCoroutineContext().ensureActive()
                    error = "Pairing failed or expired. Preview a fresh desktop invitation."
                    preview = null
                }
                finally { busy = false }
            }
        }) { Text(if (busy) "Pairing…" else "Confirm full control") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
