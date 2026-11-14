// What the phone keeps, in a Preferences DataStore: the hosts it connects to
// (each with its pinned host key) and its own settings. The SSH key isn't
// here: the :ssh module keeps it (PhoneKey), out of the app's reach.

package io.github.trevarj.goatr.app.session

import android.content.Context
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import herdr.core.PRESET_NAMES
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable
data class Host(
    val id: String,
    val user: String,
    val host: String,
    val port: Int = 22,
    /** Pinned once you've checked it (base64), with its fingerprint for display; null for a new host. */
    val hostKey: String? = null,
    val fingerprint: String? = null,
    /** Switched off: not connected, kept for later. */
    val off: Boolean = false,
    /** Authenticated iroh desktop identity; null means this is an SSH host. */
    val peer: String? = null,
    /** A pre-confirm pin is retained on response loss; it is never silently trusted. */
    val paired: Boolean = true,
)

/** `user@host[:port]`, or null when it isn't one. */
fun parseTarget(text: String): Host? {
    val m = Regex("""^\s*([^@\s]+)@([^:\s]+)(?::(\d+))?\s*$""").find(text) ?: return null
    val (user, host, port) = m.destructured
    val p = port.toIntOrNull() ?: 22
    return Host(id = "$user@$host:$p", user = user, host = host, port = p)
}

@Serializable
data class Prefs(
    /** A preset name, "system" (graphite / paper, following the system) or "material". */
    val theme: String = "graphite",
    /** A family from [FONTS], or null for the system monospace. */
    val font: String? = "JetBrains Mono",
    val fontSize: Int = 13,
    val ligatures: Boolean = true,
    /** Alerts when an agent needs you or finishes, and the connection kept while out of sight. Opt in. */
    val notifications: Boolean = false,
    /** The desktop's sound with each alert. */
    val sounds: Boolean = true,
    /** Ask before closing a tab or space. */
    val confirmClose: Boolean = true,
    /** The terminal last opened (app-wide address), to open again on the next start. */
    val terminal: String? = null,
    /** The tablet's sidebar at full width; folded to a rail when false. */
    val sidebar: Boolean = true,
    /** The app over the status and navigation bars; a swipe from the edge shows them for a moment. */
    val fullscreen: Boolean = false,
    /** The terminal's key bar (Esc, Tab, Ctrl, Alt, arrows, paste). */
    val keyBar: Boolean = true,
    /** The key bar only while the keyboard is up; otherwise it's always there. */
    val keyBarTyping: Boolean = false,
    /** Programs may copy to the phone's clipboard (OSC 52), as on the desktop. */
    val clipboardWrite: Boolean = true,
    /** Selected paired desktop, distinct from the downstream Herdr machine/terminal address. */
    val desktop: String? = null,
    val irohConnectTimeoutSeconds: Int = 45,
)

data class FontChoice(val family: String?, val label: String, val ligatures: Boolean)

/** The font chosen, JetBrains Mono when it's gone. */
val Prefs.fontChoice get() = FONTS.find { it.family == font } ?: FONTS[0]

/** Ligatures on and a font that draws them. */
val Prefs.drawLigatures get() = ligatures && fontChoice.ligatures

/**
 * Terminal fonts: JetBrains Mono (the desktop's) and
 * Geist Mono, both built into the app; and Android's own.
 * Geist Mono is drawn without its ligatures: they turn two or
 * three characters into one glyph a single cell wide, so the line beyond them
 * slips out of the grid.
 */
val FONTS = listOf(
    FontChoice("JetBrains Mono", "JetBrains Mono", ligatures = true),
    FontChoice("Geist Mono", "Geist Mono", ligatures = false),
    FontChoice(null, "System monospace", ligatures = false),
)

val FONT_SIZES = 9..22
val IROH_CONNECT_TIMEOUT_SECONDS = 5..300

/**
 * The line for the host's ~/.ssh/authorized_keys. `restrict` + a forced
 * command: over SSH this key only runs `hc connect` (no PTY, no forwarding).
 * herdr itself can run terminals and commands, so it still acts as the user.
 */
fun authorizedLine(publicKey: String) = """restrict,command="hc connect" $publicKey"""

/** What to paste into a terminal on the host, once. */
fun installCommand(publicKey: String) =
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '${authorizedLine(publicKey)}' >> ~/.ssh/authorized_keys"

private val json = Json { ignoreUnknownKeys = true }

private val Context.dataStore by preferencesDataStore("herdr")

private val HOSTS = stringPreferencesKey("hosts")
private val PREFS = stringPreferencesKey("prefs")

private fun Preferences.hosts(): List<Host> =
    this[HOSTS]?.let { json.decodeFromString<List<Host>>(it) } ?: emptyList()

// A font or theme this version no longer has falls back to the default.
internal fun Preferences.prefs(): Prefs {
    val default = Prefs()
    val p = this[PREFS]?.let { runCatching { json.decodeFromString<Prefs>(it) }.getOrNull() } ?: default
    return p.copy(
        font = if (FONTS.any { it.family == p.font }) p.font else default.font,
        theme = if (p.theme == "system" || p.theme == "material" || p.theme in PRESET_NAMES) p.theme else default.theme,
        fontSize = p.fontSize.coerceIn(FONT_SIZES),
        irohConnectTimeoutSeconds = p.irohConnectTimeoutSeconds.coerceIn(IROH_CONNECT_TIMEOUT_SECONDS),
    )
}

class Store(context: Context, scope: CoroutineScope) {
    private val data = context.dataStore

    /** False until the saved values are read. */
    val loaded: StateFlow<Boolean> = data.data.map { true }.stateIn(scope, SharingStarted.Eagerly, false)
    val hosts: StateFlow<List<Host>> = data.data.map { it.hosts() }.stateIn(scope, SharingStarted.Eagerly, emptyList())

    /** The hosts, null until read: "none yet" told apart from "none". */
    val savedHosts: StateFlow<List<Host>?> = data.data.map<Preferences, List<Host>?> { it.hosts() }.stateIn(scope, SharingStarted.Eagerly, null)
    val prefs: StateFlow<Prefs> = data.data.map { it.prefs() }.stateIn(scope, SharingStarted.Eagerly, Prefs())

    suspend fun currentPrefs(): Prefs = data.data.first().prefs()

    private suspend fun editHosts(change: (List<Host>) -> List<Host>) {
        data.edit { it[HOSTS] = json.encodeToString(change(it.hosts())) }
    }

    suspend fun addHost(target: Host) {
        val host = if (target.peer == null) target.copy(id = "${target.user}@${target.host}:${target.port}") else target
        editHosts { hosts -> hosts.filter { it.id != host.id } + host }
    }

    suspend fun updateHost(id: String, patch: (Host) -> Host) = editHosts { hosts -> hosts.map { if (it.id == id) patch(it) else it } }

    suspend fun removeHost(id: String) = editHosts { hosts -> hosts.filter { it.id != id } }

    suspend fun setPrefs(patch: (Prefs) -> Prefs) {
        data.edit { it[PREFS] = json.encodeToString(patch(it.prefs())) }
    }
}
