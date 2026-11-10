// Herdr's attention alerts on the phone: a notification with the desktop's
// sound when an agent needs you or finished while you weren't looking
// (herdr-app's `Model::alert`), and the ongoing one the service shows while
// agents work. One Android channel for the alerts, silent: the app plays the
// sound itself, so each kind keeps its own and Settings can turn it off.

package io.github.trevarj.goatr.app.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import io.github.trevarj.goatr.app.BuildConfig
import io.github.trevarj.goatr.app.MainActivity
import io.github.trevarj.goatr.app.R
import io.github.trevarj.goatr.app.session.Live
import io.github.trevarj.goatr.app.session.Store
import io.github.trevarj.goatr.app.session.hostOf
import io.github.trevarj.goatr.app.session.localOf
import io.github.trevarj.goatr.app.session.ref
import herdr.core.AgentState
import herdr.core.Attention
import herdr.core.Connection
import herdr.core.SessionSnapshot
import herdr.core.Terminal
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

enum class Alert { Blocked, Done }

/**
 * The desktop's rule: entering blocked (or error) needs you; a busy agent
 * going quiet finished, which is news only if you weren't looking.
 */
fun alertFor(prev: AgentState?, new: AgentState?, looking: Boolean): Alert? {
    fun busy(s: AgentState?) = s == AgentState.Working || s?.blocked == true
    return when {
        new?.blocked == true && prev?.blocked != true -> Alert.Blocked
        busy(prev) && new != null && !busy(new) && !looking -> Alert.Done
        else -> null
    }
}

// Box drawing and blocks: a TUI's frames and bars, nothing to read.
private val CHROME = Regex("""[\s\u2500-\u259F]*""")

/**
 * The end of what a terminal shows (`terminal.read`), for an alert: its last
 * lines with words in them, frames and their side borders left out.
 */
fun previewOf(screen: String, lines: Int = 6): String? =
    screen.lines()
        .map { it.trim().trim('│', '┃', '|').trim() }
        .filter { it.isNotEmpty() && !CHROME.matches(it) }
        .takeLast(lines)
        .joinToString("\n")
        .ifEmpty { null }

/** One working agent, for the ongoing notification. */
data class Working(val address: String, val title: String)

/** An alert to give: the terminal (app-wide address) and whether you're looking at it. */
data class Ring(val address: String, val terminal: Terminal, val alert: Alert, val looking: Boolean)

/** What one look at the hosts says: alerts to give, agents working, alerts no longer news. */
data class Tended(val rings: List<Ring>, val working: List<Working>, val settled: List<String>)

/** What the agent's alert and the ongoing notification call it. */
fun titleOf(t: Terminal): String =
    t.detail ?: t.agent ?: t.name ?: t.program

/**
 * Follows every host's terminals from snapshot to snapshot and says what
 * rings. What a host had when first listed isn't news, and words are kept
 * across a reconnect, so a lost link doesn't ring everything again.
 */
class Watch {
    /** Each terminal's last word, by app-wide address. */
    private val words = HashMap<String, AgentState?>()

    /** Hosts listed at least once. */
    private val listed = HashSet<String>()

    fun tend(snaps: List<Pair<String, SessionSnapshot>>, looking: (String) -> Boolean): Tended {
        val rings = mutableListOf<Ring>()
        val working = mutableListOf<Working>()
        val settled = mutableListOf<String>()
        for ((host, s) in snaps) {
            if (s.connection != Connection.Connected) continue
            val first = host !in listed && s.terminals.isNotEmpty()
            for (t in s.terminals) {
                val address = ref(host, t.address)
                val prev = words[address]
                words[address] = t.status
                // Only ours ring: a sub-agent reports to its parent, and one in
                // no space isn't on anyone's screen.
                if (t.parent != null || t.space == null) continue
                if (t.status == AgentState.Working) working += Working(address, titleOf(t))
                if (t.attention != Attention.Blocked && t.attention != Attention.Done) settled += address
                if (first) continue
                val see = looking(address)
                alertFor(prev, t.status, see)?.let { rings += Ring(address, t, it, see) }
            }
            if (s.terminals.isNotEmpty()) listed += host
        }
        return Tended(rings, working, settled)
    }
}

class Notifier(private val context: Context, private val live: Live, private val store: Store, private val scope: CoroutineScope) {
    /** Whether the app is on screen (MainActivity started). */
    val foreground = MutableStateFlow(false)

    private val _working = MutableStateFlow<List<Working>>(emptyList())

    /** Agents working now, across hosts: the service's notification lists them. */
    val working: StateFlow<List<Working>> = _working

    private val watch = Watch()

    private val manager = context.getSystemService(NotificationManager::class.java)

    /** Alerts on screen, to take back once seen or busy again. */
    private val posted = HashSet<String>()

    init {
        channels()
        scope.launch { live.snapshots.collect(::tend) }
        // Switched off: what's on screen goes too.
        scope.launch {
            store.prefs.collect { if (!it.notifications) posted.toList().forEach(::cancel) }
        }
    }

    /** Opted in (Settings) and allowed by Android. */
    private val on: Boolean get() = store.prefs.value.notifications && allowed()

    private fun tend(snaps: List<Pair<String, SessionSnapshot>>) {
        val visible = live.visible.value
        val t = watch.tend(snaps) { foreground.value && it in visible }
        t.settled.forEach(::cancel)
        for (r in t.rings.takeIf { on }.orEmpty()) {
            // Looking at one that needs you: the desktop rings, but there's nothing to show.
            if (!(r.alert == Alert.Blocked && r.looking)) {
                post(r.address, r.terminal, r.alert)
                scope.launch { preview(r.address)?.let { post(r.address, r.terminal, r.alert, it) } }
            }
            ring(if (r.alert == Alert.Blocked) R.raw.request else R.raw.done)
        }
        _working.value = t.working
    }

    /** What the agent shows now (what it asks, or how it finished), if its host answers. */
    private suspend fun preview(address: String): String? {
        val client = live.clientNow(hostOf(address)) ?: return null
        val read = try {
            client.call("terminal.read", buildJsonObject { put("terminal", localOf(address)) })
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            return null
        }
        return ((read as? JsonObject)?.get("text") as? JsonPrimitive)?.content?.let(::previewOf)
    }

    /** Posts (or, with what it shows, updates) the alert for a terminal. */
    private fun post(address: String, t: Terminal, alert: Alert, preview: String? = null) {
        if (!allowed()) return
        // Taken back (seen, busy again) before the preview came.
        if (preview != null && address !in posted) return
        val word = when (alert) {
            Alert.Blocked -> "Needs you"
            Alert.Done -> "Done"
        }
        val tool = t.agent ?: t.program
        val where = listOfNotNull(t.where, t.machine).joinToString(" · ").ifEmpty { null }
        val text = listOfNotNull(word, tool, where).joinToString(" · ")
        val n = NotificationCompat.Builder(context, ALERTS)
            .setSmallIcon(R.drawable.ic_notification_goat)
            .setContentTitle(titleOf(t))
            .setContentText(text)
            .apply { preview?.let { setStyle(NotificationCompat.BigTextStyle().bigText("$text\n\n$it")) } }
            // The preview updates the alert in place: no second heads-up.
            .setOnlyAlertOnce(preview != null)
            .setCategory(if (alert == Alert.Blocked) NotificationCompat.CATEGORY_REMINDER else NotificationCompat.CATEGORY_STATUS)
            .setContentIntent(open(context, address))
            .setGroup(GROUP)
            .setAutoCancel(true)
            .build()
        posted += address
        @Suppress("MissingPermission")
        NotificationManagerCompat.from(context).apply {
            notify(address, ALERT_ID, n)
            notify(SUMMARY_ID, summary())
        }
    }

    /** What the alerts fold under, two or more at once. */
    private fun summary() = NotificationCompat.Builder(context, ALERTS)
        .setSmallIcon(R.drawable.ic_notification_goat)
        .setContentTitle(if (posted.size == 1) "1 agent" else "${posted.size} agents")
        .setContentIntent(open(context, null))
        .setGroup(GROUP)
        .setGroupSummary(true)
        .setAutoCancel(true)
        .build()

    private fun cancel(address: String) {
        if (!posted.remove(address)) return
        NotificationManagerCompat.from(context).apply {
            cancel(address, ALERT_ID)
            if (posted.isEmpty()) cancel(SUMMARY_ID)
        }
    }

    /**
     * The desktop's sound for an alert, unless it's off in Settings, the
     * alerts channel is switched off in Android's, or Do Not Disturb is on.
     */
    private fun ring(sound: Int) {
        if (!store.prefs.value.sounds) return
        if (manager.getNotificationChannel(ALERTS)?.importance == NotificationManager.IMPORTANCE_NONE) return
        if (manager.currentInterruptionFilter > NotificationManager.INTERRUPTION_FILTER_ALL) return
        val player = MediaPlayer.create(context, sound, attributes, 0) ?: return
        player.setOnCompletionListener { it.release() }
        player.start()
    }

    /** Android lets the app notify: granted (13+) and not switched off for it. */
    fun allowed() = NotificationManagerCompat.from(context).areNotificationsEnabled()

    private fun channels() {
        // Before one channel: a needs-you and a done one, each with its sound.
        listOf("needs_you", "done").forEach(manager::deleteNotificationChannel)
        manager.createNotificationChannels(
            listOf(
                NotificationChannel(ALERTS, "Agents", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "An agent needs you or finished"
                    setSound(null, null)
                },
                NotificationChannel(WORKING, "Working", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Agents at work"
                    setShowBadge(false)
                },
            ),
        )
    }

    companion object {
        const val ALERTS = "agents"
        const val WORKING = "working"
        private const val ALERT_ID = 1
        private const val SUMMARY_ID = 3
        private const val GROUP = "agents"

        private val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()

        /** Opens the app on `address` (a terminal), or on the agents list. */
        fun open(context: Context, address: String?): PendingIntent {
            val link = if (address == null) "${BuildConfig.SCHEME}://agents" else "${BuildConfig.SCHEME}://terminal?address=${Uri.encode(address)}"
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(link), context, MainActivity::class.java)
            return PendingIntent.getActivity(
                context,
                address?.hashCode() ?: 0,
                intent.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        }
    }
}
