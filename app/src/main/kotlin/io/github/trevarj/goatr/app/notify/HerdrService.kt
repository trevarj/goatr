// Keeps the hosts connected while the app is out of sight, so alerts still
// come: Android freezes a background app's sockets, not a foreground
// service's. Its notification is the one that's always there: the agents
// working now, else where the phone is connected; expanded, it has Exit,
// which ends the app until it's opened again.

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
        ServiceCompat.startForeground(this, ID, notification(emptyList(), container.live.session.value), type)
        scope.launch {
            combine(container.notifier.working, container.live.session) { w, s -> w to s }
                .distinctUntilChanged { a, b -> a.first == b.first && a.second.host == b.second.host && a.second.connection == b.second.connection }
                .collect { (w, s) -> getSystemService(NotificationManager::class.java).notify(ID, notification(w, s)) }
        }
        // Switched off, or no host left to stay connected to: nothing to keep alive.
        scope.launch {
            combine(container.store.hosts, container.store.prefs) { hosts, prefs -> !prefs.notifications || hosts.none { !it.off } }
                .collect { done -> if (done && container.store.loaded.value) stopSelf() }
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
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
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

    private fun notification(working: List<Working>, session: Session) =
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
                if (working.isEmpty()) {
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
            context.startForegroundService(Intent(context, HerdrService::class.java))
        }
    }
}
