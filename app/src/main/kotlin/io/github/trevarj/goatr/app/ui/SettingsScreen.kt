// Small and phone-owned: hosts and this phone's key, the terminal's font, then
// the theme. Saved on the device (session/Store.kt).

package io.github.trevarj.goatr.app.ui

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.testTag
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.app.SettingsViewModel
import io.github.trevarj.goatr.app.BuildConfig
import io.github.trevarj.goatr.app.session.FONTS
import io.github.trevarj.goatr.app.session.FONT_SIZES
import io.github.trevarj.goatr.app.session.drawLigatures
import io.github.trevarj.goatr.app.session.fontChoice
import io.github.trevarj.goatr.app.session.LocalNetwork
import io.github.trevarj.goatr.app.session.LocalNetworkBlocked
import io.github.trevarj.goatr.app.session.Probe
import io.github.trevarj.goatr.app.session.parseTarget
import io.github.trevarj.goatr.app.session.reason
import dev.herdr.ssh.Refused
import dev.herdr.ssh.Seen
import herdr.core.Connection
import herdr.core.PRESET_NAMES
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

// Shows the ligatures a font has, and how its letters sit.
private const val SAMPLE = "fn x -> y != z === 0 <= 1"

@Composable
fun SettingsScreen(vm: SettingsViewModel) {
    val session by vm.session.collectAsStateWithLifecycle()
    TabScreen("Settings", Lucide.Settings, session) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
            val localNetwork by vm.localNetworkAllowed.collectAsStateWithLifecycle()
            if (!localNetwork) LocalNetworkBanner()
            PairedDesktops()
            HostsSection(vm)
            KeySection(vm)
            GeneralSection(vm)
            TerminalSection(vm)
            ThemeSection(vm)
            Text(
                "Goatr ${BuildConfig.VERSION_NAME}",
                Modifier.fillMaxWidth().padding(top = 24.dp),
                textAlign = TextAlign.Center,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** A section's head: a rule from the one above (none for the first), its icon and name. */
@Composable
internal fun SectionTitle(title: String, icon: Painter, first: Boolean = false, actions: @Composable RowScope.() -> Unit = {}) {
    if (!first) HorizontalDivider(Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp))
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = if (first) 16.dp else 20.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.size(28.dp).clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.secondaryContainer),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSecondaryContainer)
        }
        Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
        // The section's own actions, small, on the right of its title.
        Row(horizontalArrangement = Arrangement.spacedBy(2.dp), content = actions)
    }
}

/** A setting that's on or off: the whole row toggles it. */
@Composable
private fun SwitchItem(title: String, checked: Boolean, supporting: String? = null, enabled: Boolean = true, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = supporting?.let { { Text(it) } },
        trailingContent = { Switch(checked, null, enabled = enabled) },
        modifier = Modifier.selectable(checked, enabled = enabled, role = Role.Switch, onClick = onClick),
    )
}

/** One of a [selectableGroup]'s options: the whole row picks it. */
@Composable
private fun RadioItem(
    label: String,
    selected: Boolean,
    supporting: (@Composable () -> Unit)?,
    trailing: (@Composable () -> Unit)? = null,
    onClick: () -> Unit,
) {
    ListItem(
        headlineContent = { Text(label) },
        supportingContent = supporting,
        leadingContent = { RadioButton(selected, null) },
        trailingContent = trailing,
        modifier = Modifier.selectable(selected, role = Role.RadioButton, onClick = onClick),
    )
}

/** A host's next step, under its row. */
@Composable
private fun HostCard(content: @Composable ColumnScope.() -> Unit) {
    OutlinedCard(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp).fillMaxWidth()) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp), content = content)
    }
}

@Composable
private fun Hint(text: String, color: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color)
}

@Composable
private fun TryAgain(onClick: () -> Unit, modifier: Modifier = Modifier) {
    TextButton(onClick, modifier) { Text("Try again") }
}

/**
 * Android 17+ without the local network permission: hosts on the LAN can't
 * be reached until it's allowed. Allow asks, or once Android stops asking,
 * opens the app's page in its settings; the banner goes once it's granted
 * (the activity reads the answer again when the app comes back).
 */
@Composable
private fun LocalNetworkBanner() {
    val context = LocalContext.current
    var refused by rememberSaveable { mutableStateOf(false) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> refused = !granted }
    Surface(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp),
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
    ) {
        Row(Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                "If you're connecting to devices on your local network, you need to allow access to them.",
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodyMedium,
            )
            TextButton({
                if (!refused) ask.launch(LocalNetwork.PERMISSION)
                else context.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
                )
            }) { Text("Allow") }
        }
    }
}

@Composable
private fun HostsSection(vm: SettingsViewModel) {
    val hosts by vm.hosts.collectAsStateWithLifecycle()
    val states by vm.hostStates.collectAsStateWithLifecycle()
    var target by rememberSaveable { mutableStateOf("") }
    fun add() {
        if (vm.addHost(target)) target = ""
    }
    SectionTitle("SSH hosts", Lucide.Server)
    for (h in hosts.filter { it.peer == null }) {
        val st = states[h.id]
        val error = st?.problem?.takeIf { !h.off }
        val state = when {
            h.off -> "off"
            h.hostKey == null -> "new"
            error != null -> "failed"
            st?.connection == Connection.Connected -> "connected"
            st?.connection == Connection.Lost -> "lost"
            else -> "connecting"
        }
        ListItem(
            headlineContent = { Text(h.id.removeSuffix(":22")) },
            supportingContent = {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        state,
                        color = when (state) {
                            "connected" -> herdr.connected
                            "failed", "lost" -> MaterialTheme.colorScheme.error
                            else -> MaterialTheme.colorScheme.onSurfaceVariant
                        },
                    )
                    if (error != null) Text(error.reason, color = MaterialTheme.colorScheme.error)
                    st?.version?.let { Text("herdr-core $it", style = MaterialTheme.typography.labelSmall) }
                    h.fingerprint?.let { Text(it, maxLines = 1, style = MaterialTheme.typography.labelSmall.copy(fontFamily = GeistMono)) }
                }
            },
            trailingContent = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Switch(!h.off, { vm.setOn(h.id, it) }, Modifier.semantics { contentDescription = "Connect to ${h.id}" })
                    IconButton({ vm.removeHost(h.id) }) { Icon(Lucide.Trash2, "Remove ${h.id}") }
                }
            },
        )
        when {
            h.off -> {}
            error is LocalNetworkBlocked -> {}
            h.hostKey == null -> NewHost(vm, h.id, st?.probe)
            error == null -> {}
            error is Refused.Key -> InstallKey(vm, h.id, st.password)
            else -> TryAgain({ vm.reconnect(h.id) }, Modifier.padding(start = 8.dp))
        }
    }
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            target,
            { target = it },
            Modifier.weight(1f).testTag("host-input"),
            label = { Text("Add a host") },
            placeholder = { Text("user@host or user@host:port") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { add() }),
        )
        Button(::add, enabled = parseTarget(target) != null) { Text("Add") }
    }
}

/**
 * A host seen for the first time: its key, fetched without logging in, to
 * check against the host's own before trusting it. Trusting connects with the
 * phone's key; if the host doesn't have it yet, InstallKey takes over.
 */
@Composable
private fun NewHost(vm: SettingsViewModel, id: String, probe: Probe?) {
    val scope = rememberCoroutineScope()
    HostCard {
        when (probe) {
            null, Probe.Fetching -> Hint("Getting the host's key…")
            is Probe.Failed -> {
                Hint(probe.message, MaterialTheme.colorScheme.error)
                TryAgain({ vm.reconnect(id) })
            }
            is Probe.Got -> {
                val seen = probe.seen
                Hint("New host. Check its key there with:")
                SelectionContainer {
                    Text(
                        "ssh-keygen -lf /etc/ssh/ssh_host_${keyFile(seen)}_key.pub",
                        style = MaterialTheme.typography.bodySmall.copy(fontFamily = GeistMono),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                SelectionContainer {
                    Text(seen.fingerprint, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = GeistMono))
                }
                Button({ scope.launch { vm.trust(id, seen) } }) { Text("It matches: trust and connect") }
            }
        }
    }
}

/** The host key file `ssh-keygen -l` should read for this key type. */
private fun keyFile(seen: Seen) = when {
    seen.hostKeyType == "ssh-ed25519" -> "ed25519"
    seen.hostKeyType.startsWith("ecdsa") -> "ecdsa"
    seen.hostKeyType == "ssh-rsa" -> "rsa"
    else -> "*"
}

/**
 * The host refused the phone's key. If it takes passwords, one login adds the
 * restricted key line to its authorized_keys (ssh-copy-id); if not, it has to
 * be added there by hand, with the command under "This phone's key".
 */
@Composable
private fun InstallKey(vm: SettingsViewModel, id: String, takesPassword: Boolean?) {
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    fun run() {
        if (password.isEmpty() || busy) return
        busy = true
        error = null
        scope.launch {
            try {
                vm.installKey(id, password)
                password = ""
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.reason
            }
            busy = false
        }
    }
    HostCard {
        when (takesPassword) {
            null -> Hint("The host doesn't have this phone's key yet. Checking how it signs in…")
            false -> {
                Hint("The host doesn't have this phone's key, and doesn't take passwords. Copy the install command below, run it on the host, then try again.")
                TryAgain({ vm.reconnect(id) })
            }
            true -> {
                Hint("The host doesn't have this phone's key yet. Enter your password there to install it. It isn't saved.")
                OutlinedTextField(
                    password,
                    { password = it },
                    Modifier.fillMaxWidth(),
                    label = { Text("Password") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Go),
                    keyboardActions = KeyboardActions(onGo = { run() }),
                    isError = error != null,
                    supportingText = error?.let { { Text(it) } },
                )
                Button(::run, enabled = password.isNotEmpty() && !busy) { Text(if (busy) "Installing…" else "Install key") }
            }
        }
    }
}

/**
 * Agent alerts (opt in: the desktop's sounds, and the hosts kept connected
 * while the app is out of sight) and whether closing asks first.
 */
@Composable
private fun GeneralSection(vm: SettingsViewModel) {
    val prefs by vm.prefs.collectAsStateWithLifecycle()
    val context = LocalContext.current
    // Android's answer can change in its settings while we're away: ask again on return.
    var allowed by remember { mutableStateOf(vm.notificationsAllowed()) }
    LifecycleResumeEffect(Unit) {
        allowed = vm.notificationsAllowed()
        onPauseOrDispose {}
    }
    // Refused once more than Android will ask: only its settings can allow it now.
    var refused by rememberSaveable { mutableStateOf(false) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        allowed = granted
        if (granted) vm.setPrefs { it.copy(notifications = true) } else refused = true
    }
    fun openAndroidSettings() = context.startActivity(
        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
    )
    val on = prefs.notifications && allowed
    SectionTitle("General", Lucide.Sliders)
    SwitchItem(
        "Agent alerts",
        on,
        if ((prefs.notifications || refused) && !allowed) "Blocked in Android settings. Tap to allow."
        else "When an agent needs you or finishes. Keeps hosts connected in the background.",
    ) {
        when {
            on -> vm.setPrefs { it.copy(notifications = false) }
            allowed -> vm.setPrefs { it.copy(notifications = true) }
            Build.VERSION.SDK_INT >= 33 && !refused -> ask.launch(Manifest.permission.POST_NOTIFICATIONS)
            else -> {
                // Back from Android's settings, the resume check picks it up.
                vm.setPrefs { it.copy(notifications = true) }
                openAndroidSettings()
            }
        }
    }
    SwitchItem("Alert sound", prefs.sounds, enabled = on) { vm.setPrefs { it.copy(sounds = !it.sounds) } }
    SwitchItem("Confirm before closing", prefs.confirmClose, "Tabs and spaces") { vm.setPrefs { it.copy(confirmClose = !it.confirmClose) } }
    SwitchItem("Full screen", prefs.fullscreen, "Hides the status and navigation bars. Swipe from an edge to see them.") {
        vm.setPrefs { it.copy(fullscreen = !it.fullscreen) }
    }
}

@Composable
private fun TerminalSection(vm: SettingsViewModel) {
    val prefs by vm.prefs.collectAsStateWithLifecycle()
    val font = prefs.fontChoice
    fun resize(by: Int) = vm.setPrefs { it.copy(fontSize = (it.fontSize + by).coerceIn(FONT_SIZES)) }
    SectionTitle("Terminal", Lucide.SquareTerminal)
    Column(Modifier.selectableGroup()) {
        for (f in FONTS) {
            val family = when (f.family) {
                "JetBrains Mono" -> JetBrainsMono
                "Geist Mono" -> GeistMono
                else -> FontFamily.Monospace
            }
            RadioItem(
                f.label,
                prefs.font == f.family,
                supporting = {
                    Text(
                        SAMPLE,
                        maxLines = 1,
                        style = TextStyle(fontFamily = family, fontFeatureSettings = if (prefs.ligatures && f.ligatures) null else "liga 0, calt 0"),
                    )
                },
            ) { vm.setPrefs { it.copy(font = f.family) } }
        }
    }
    ListItem(
        headlineContent = { Text("Font size") },
        supportingContent = { Text("Pinch to zoom for a moment.") },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                FilledTonalIconButton({ resize(-1) }, enabled = prefs.fontSize > FONT_SIZES.first) { Icon(Lucide.Minus, "Smaller") }
                Text("${prefs.fontSize}", Modifier.width(32.dp), textAlign = TextAlign.Center, style = tabular)
                FilledTonalIconButton({ resize(1) }, enabled = prefs.fontSize < FONT_SIZES.last) { Icon(Lucide.Plus, "Larger") }
            }
        },
    )
    SwitchItem("Ligatures", prefs.drawLigatures, if (!font.ligatures) "None in ${font.label}" else null, enabled = font.ligatures) {
        vm.setPrefs { it.copy(ligatures = !it.ligatures) }
    }
    SwitchItem("Key bar", prefs.keyBar, "Esc, Tab, Ctrl, Alt, arrows and paste, under the terminal") {
        vm.setPrefs { it.copy(keyBar = !it.keyBar) }
    }
    SwitchItem("Only while typing", prefs.keyBarTyping, "Shows the key bar only while the keyboard is up", enabled = prefs.keyBar) {
        vm.setPrefs { it.copy(keyBarTyping = !it.keyBarTyping) }
    }
    SwitchItem("Programs can copy", prefs.clipboardWrite, "Lets vim, tmux and agents put text on this phone's clipboard") {
        vm.setPrefs { it.copy(clipboardWrite = !it.clipboardWrite) }
    }
}

@Composable
private fun ThemeSection(vm: SettingsViewModel) {
    val prefs by vm.prefs.collectAsStateWithLifecycle()
    SectionTitle("Theme", Lucide.Palette)
    Column(Modifier.selectableGroup()) {
        val options = buildList {
            add(Triple("system", "Follow the system", "graphite / paper"))
            if (hasDynamicColor) add(Triple(MATERIAL, "Material You", "your wallpaper's colors; the terminal follows the system"))
            PRESET_NAMES.forEach { add(Triple(it, it.replaceFirstChar(Char::uppercase), null)) }
        }
        for ((name, label, detail) in options) {
            RadioItem(
                label,
                prefs.theme == name,
                supporting = detail?.let { { Text(it) } },
                trailing = if (name in PRESET_NAMES) ({ Swatch(name) }) else null,
            ) { vm.setPrefs { it.copy(theme = name) } }
        }
    }
}

/** A preset at a glance: its chrome with working, blocked, done and accent on it. */
@Composable
private fun Swatch(name: String) {
    val t = remember(name) { HerdrColors(name) }
    Row(
        Modifier.height(28.dp).clip(RoundedCornerShape(4.dp)).background(t.chrome).padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        for (c in listOf(t.working, t.blocked, t.done, t.accent)) {
            Box(Modifier.size(width = 14.dp, height = 20.dp).clip(RoundedCornerShape(2.dp)).background(c))
        }
    }
}
