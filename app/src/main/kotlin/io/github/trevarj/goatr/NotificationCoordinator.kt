// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build

/**
 * The runtime's one OS presentation sink. It cannot create events, make provider requests, or
 * decide duplicate/read retention; SessionStore admits each durable presentation before calling it.
 */
class NotificationCoordinator(private val context: Context, private val runtime: GoatrRuntime) : PresentationSink {
    private val manager=context.getSystemService(NotificationManager::class.java)
    private val preferences=runtime.deliveryPreferences

    init { ensureChannels() }

    fun foreground():Notification=Notification.Builder(context,FOREGROUND_CHANNEL)
        .setSmallIcon(R.drawable.ic_notification_goat).setContentTitle("Goatr delivery is running")
        .setContentText("Keeping selected desktops connected for notifications.")
        .setOngoing(true).setCategory(Notification.CATEGORY_SERVICE).setContentIntent(openAppIntent())
        .addAction(Notification.Action.Builder(Icon.createWithResource(context,R.drawable.ic_material_stop),"Stop",stopIntent()).build()).build()

    suspend fun reconcile(host:String) = runtime.store.reconcilePresentations(host, policy = {
        PresentationPolicy(
            enabled=preferences.hostEnabled(host) && preferences.canPostNotifications(),
            includeCompleted=preferences.notifyCompleted(host),
            suppress={ event -> runtime.suppressesAlert(host,event) },
        )
    })

    /** Preferences are only a policy input; the store serializes its effect with cancellation. */
    suspend fun setHostEnabled(host:String,enabled:Boolean) {
        runtime.store.reconcilePresentations(host, { preferences.setHostEnabled(host,enabled) }) {
            PresentationPolicy(preferences.hostEnabled(host) && preferences.canPostNotifications(),preferences.notifyCompleted(host)) { event -> runtime.suppressesAlert(host,event) }
        }
    }
    suspend fun setCompleted(host:String,enabled:Boolean) {
        runtime.store.reconcilePresentations(host, { preferences.setNotifyCompleted(host,enabled) }) {
            PresentationPolicy(preferences.hostEnabled(host) && preferences.canPostNotifications(),preferences.notifyCompleted(host)) { event -> runtime.suppressesAlert(host,event) }
        }
    }
    suspend fun forgetHost(host:String) { reconcile(host) }

    override fun post(host:String,event:AttentionEvent):Boolean {
        val name=runtime.store.state.value.hosts.firstOrNull {it.hostId==host}?.name ?: return false
        manager.notify(tag(host,event.eventId.value),ALERT_ID,alert(name,host,event))
        return true
    }
    override fun postHint(host:String,hint:PushEventHint):Boolean {
        val name=runtime.store.state.value.hosts.firstOrNull{it.hostId==host}?.name ?: return false
        val completed=hint.category==AttentionCategory.Completed
        val alert=Notification.Builder(context,ALERT_CHANNEL).setSmallIcon(R.drawable.ic_notification_goat)
            .setContentTitle(if(completed)"Work finished on desktop"else"Desktop needs attention").setContentText("$name · Open to synchronize")
            .setAutoCancel(true).setOnlyAlertOnce(true).setCategory(Notification.CATEGORY_STATUS)
            .setContentIntent(attentionIntent(host,hint.eventId.value)).build()
        manager.notify(tag(host,hint.eventId.value),ALERT_ID,alert);return true
    }
    override fun cancel(host:String,eventId:String):Boolean {
        manager.cancel(tag(host,eventId),ALERT_ID)
        return true
    }
    /** Android does not expose a safe per-tag query, so only a successful app-owned cancel settles it. */
    override fun noLongerPresent(host:String,eventId:String):Boolean=false

    private fun alert(name:String,host:String,event:AttentionEvent):Notification {
        val completed=event.category==AttentionCategory.Completed
        return Notification.Builder(context,ALERT_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification_goat)
            .setContentTitle(if(completed) "Agent finished" else "Agent needs attention")
            .setContentText(name)
            .setAutoCancel(true).setOnlyAlertOnce(true)
            .setCategory(if(completed) Notification.CATEGORY_STATUS else Notification.CATEGORY_MESSAGE)
            .setContentIntent(attentionIntent(host,event.eventId.value)).build()
    }
    private fun attentionIntent(host:String,event:String):PendingIntent {
        val intent=Intent(context,MainActivity::class.java).setAction(ACTION_OPEN_ATTENTION)
            .setData(Uri.Builder().scheme("goatr").authority("attention").appendPath(host).appendPath(event).build())
            .putExtra(EXTRA_HOST,host).putExtra(EXTRA_EVENT,event)
        return PendingIntent.getActivity(context,0,intent,PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }
    private fun openAppIntent():PendingIntent=PendingIntent.getActivity(context,0,Intent(context,MainActivity::class.java),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    private fun stopIntent():PendingIntent=PendingIntent.getService(context,1,Intent(context,DeliveryService::class.java).setAction(DeliveryService.ACTION_STOP),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    private fun tag(host:String,event:String)="goatr.attention.$host.$event"
    private fun ensureChannels() {if(Build.VERSION.SDK_INT>=26) {
        manager.createNotificationChannel(NotificationChannel(FOREGROUND_CHANNEL,"Goatr delivery",NotificationManager.IMPORTANCE_LOW).apply {setShowBadge(false)})
        manager.createNotificationChannel(NotificationChannel(ALERT_CHANNEL,"Goatr attention",NotificationManager.IMPORTANCE_DEFAULT))
    }}
    companion object {
        private const val ALERT_ID=1
        const val ACTION_OPEN_ATTENTION="io.github.trevarj.goatr.OPEN_ATTENTION"
        const val EXTRA_HOST="host"
        const val EXTRA_EVENT="event"
        const val FOREGROUND_CHANNEL="goatr_delivery"
        const val ALERT_CHANNEL="goatr_attention"
    }
}
