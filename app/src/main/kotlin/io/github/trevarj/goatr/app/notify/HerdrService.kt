// Keeps live connections through a brief app switch or for opted-in alerts.

package io.github.trevarj.goatr.app.notify

import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.Process
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import io.github.trevarj.goatr.app.R
import io.github.trevarj.goatr.app.container
import io.github.trevarj.goatr.app.session.Session
import io.github.trevarj.goatr.app.session.activeHosts
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

class HerdrService : Service() {
    private val scope = MainScope()

    // Exit was tapped: the process ends once Android has the service stopped.
    private var exiting = false

    override fun onCreate() {
        super.onCreate()
        // Android wants the notification at once; the flows refine it.
        ServiceCompat.startForeground(this, ID, notification(emptyList(), container.live.session.value, false), type)
        scope.launch {
            combine(container.notifier.working, container.live.session, container.store.prefs) { working, session, prefs ->
                Triple(working, session, prefs.notifications)
            }.distinctUntilChanged { a, b ->
                a.third == b.third && (!a.third ||
                    a.first == b.first && a.second.host == b.second.host && a.second.connection == b.second.connection)
            }.collect { (working, session, alerts) ->
                getSystemService(NotificationManager::class.java).notify(ID, notification(working, session, alerts))
            }
        }
        scope.launch {
            val app = container
            combine(app.store.loaded, app.store.hosts, app.store.prefs, app.live.retention.lease) { loaded, hosts, prefs, _ ->
                if (loaded) activeHosts(hosts, prefs.desktop).isNotEmpty() to prefs.notifications else null
            }.collect { state ->
                if (state != null) {
                    val (hasHosts, alerts) = state
                    app.live.retention.keepForAlerts(hasHosts && alerts)
                    if (!hasHosts || !app.live.retention.active) stopSelf()
                }
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == EXIT) {
            // Stopped first: a sticky service whose process just died is started again.
            exiting = true
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        return if (intent == null || container.store.prefs.value.notifications) START_STICKY else START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        container.live.retention.keepForAlerts(false)
        scope.cancel()
        super.onDestroy()
        if (exiting) Process.killProcess(Process.myPid())
    }

    private val exit by lazy {
        PendingIntent.getService(
            this,
            0,
            Intent(this, HerdrService::class.java).setAction(EXIT),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun notification(working: List<Working>, session: Session, alerts: Boolean) =
        NotificationCompat.Builder(this, Notifier.WORKING)
            .setSmallIcon(R.drawable.ic_notification_goat)
            .setOngoing(true)
            .setSilent(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            // Shown when the notification is expanded.
            .addAction(0, "Exit", exit)
            .apply {
                if (!alerts) {
                    setContentTitle("Goatr")
                    setContentText("Keeps your connection during short app switches")
                    setContentIntent(Notifier.open(this@HerdrService, null))
                } else if (working.isEmpty()) {
                    setContentTitle(session.host.ifEmpty { "Goatr" })
                    setContentText("${session.connection.label} · no agent working")
                    setContentIntent(Notifier.open(this@HerdrService, null))
                } else {
                    setContentTitle(if (working.size == 1) "1 agent working" else "${working.size} agents working")
                    setContentText(working.joinToString(", ") { it.title })
                    setStyle(NotificationCompat.InboxStyle().also { s -> working.take(6).forEach { s.addLine(it.title) } })
                    // One: straight to it. Several: the list.
                    setContentIntent(Notifier.open(this@HerdrService, working.singleOrNull()?.address))
                }
            }
            .build()

    companion object {
        private const val ID = 2
        private const val EXIT = "io.github.trevarj.goatr.app.EXIT"
        private val type = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0

        fun start(context: Context) {
            try {
                context.startForegroundService(Intent(context, HerdrService::class.java))
            } catch (_: IllegalStateException) {
                // Android can reject a start if the activity has just left the foreground.
            }
        }
    }
}
