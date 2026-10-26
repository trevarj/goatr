// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.provider.Settings

internal enum class NotificationRepair(val channelId:String?,val label:String) {
    App(null,"App notifications"),Attention(NotificationCoordinator.ALERT_CHANNEL,"Attention channel"),Delivery(NotificationCoordinator.FOREGROUND_CHANNEL,"Delivery channel");
    fun intent(context:Context):Intent =
        if(channelId==null)Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE,context.packageName)
        else Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE,context.packageName).putExtra(Settings.EXTRA_CHANNEL_ID,channelId)
}
internal enum class NotificationDeliveryState(val label:String) {
    Off("Off"),GlobalBlocked("App notifications blocked"),AttentionBlocked("Attention channel blocked"),
    DeliveryBlocked("Delivery channel blocked"),BothChannelsBlocked("Attention and delivery channels blocked"),
    SocketFree("No socket keeper required · ntfy registration"),Stopped("Socket keeper stopped"),
    Reconnecting("Socket keeper reconnecting"),Connected("Socket keeper connected")
}
/** Ephemeral OS facts, not preferences, registration proof or confirmation of notification delivery. */
internal data class NotificationHealth(val globalAvailable:Boolean,val runtimePermissionRequired:Boolean,val attentionChannelEnabled:Boolean,val deliveryChannelEnabled:Boolean) {
    fun allowsAttention(hostEnabled:Boolean)=hostEnabled&&globalAvailable&&attentionChannelEnabled
    val keeperAvailable get()=globalAvailable&&deliveryChannelEnabled
    fun repairs():List<NotificationRepair> = buildList {
        if(!globalAvailable)add(NotificationRepair.App)
        if(!attentionChannelEnabled)add(NotificationRepair.Attention)
        if(!deliveryChannelEnabled)add(NotificationRepair.Delivery)
    }
    fun deliveryState(enabled:Boolean,keeperRequired:Boolean,requested:Boolean,running:Boolean,reconnecting:Boolean):NotificationDeliveryState = when {
        !enabled->NotificationDeliveryState.Off
        !globalAvailable->NotificationDeliveryState.GlobalBlocked
        !attentionChannelEnabled&&!deliveryChannelEnabled&&keeperRequired->NotificationDeliveryState.BothChannelsBlocked
        !attentionChannelEnabled->NotificationDeliveryState.AttentionBlocked
        !keeperRequired->NotificationDeliveryState.SocketFree
        !deliveryChannelEnabled->NotificationDeliveryState.DeliveryBlocked
        !requested||!running->NotificationDeliveryState.Stopped
        reconnecting->NotificationDeliveryState.Reconnecting
        else->NotificationDeliveryState.Connected
    }
    companion object {
        fun measured(api:Int,appEnabled:Boolean,permissionGranted:Boolean,attentionImportance:Int?,deliveryImportance:Int?):NotificationHealth {
            val permissionRequired=api>=33&&!permissionGranted
            return NotificationHealth(appEnabled&&!permissionRequired,permissionRequired,
                api<26||attentionImportance!=null&&attentionImportance!=NotificationManager.IMPORTANCE_NONE,
                api<26||deliveryImportance!=null&&deliveryImportance!=NotificationManager.IMPORTANCE_NONE)
        }
    }
}
/** Shared synchronous OS-attempt boundary. Unknown effects are consumed, never retried as no effect. */
internal fun notificationAttempt(healthy:()->Boolean,post:()->Unit):PresentationOutcome = try {
    if(!healthy())PresentationOutcome.BlockedBeforeAttempt
    else {post();if(healthy())PresentationOutcome.AttemptedHealthy else PresentationOutcome.AttemptedUnknown}
}catch(_:Exception){PresentationOutcome.AttemptedUnknown}

/**
 * The runtime's one OS presentation sink. It cannot create events, make provider requests, or
 * decide duplicate/read retention; SessionStore admits each durable presentation before calling it.
 */
class NotificationCoordinator(private val context: Context, private val runtime: GoatrRuntime) : PresentationSink {
    private val manager=context.getSystemService(NotificationManager::class.java)
    private val preferences=runtime.deliveryPreferences

    init { ensureChannels() }
    internal fun health():NotificationHealth=NotificationHealth.measured(
        Build.VERSION.SDK_INT,manager.areNotificationsEnabled(),
        Build.VERSION.SDK_INT<33||context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)==PackageManager.PERMISSION_GRANTED,
        manager.getNotificationChannel(ALERT_CHANNEL)?.importance,manager.getNotificationChannel(FOREGROUND_CHANNEL)?.importance)

    fun foreground():Notification=Notification.Builder(context,FOREGROUND_CHANNEL)
        .setSmallIcon(R.drawable.ic_notification_goat).setContentTitle("Goatr delivery is running")
        .setContentText("Keeping selected desktops connected for notifications.")
        .setOngoing(true).setCategory(Notification.CATEGORY_SERVICE).setContentIntent(openAppIntent())
        .addAction(Notification.Action.Builder(Icon.createWithResource(context,R.drawable.ic_material_stop),"Stop",stopIntent()).build()).build()

    suspend fun reconcile(host:String) = runtime.store.reconcilePresentations(host, policy = {
        PresentationPolicy(
            enabled=health().allowsAttention(preferences.hostEnabled(host)),
            includeCompleted=preferences.notifyCompleted(host),
            suppress={ event -> runtime.suppressesAlert(host,event) },
        )
    })

    /** Preferences are only a policy input; the store serializes its effect with cancellation. */
    suspend fun setHostEnabled(host:String,enabled:Boolean) {
        runtime.store.reconcilePresentations(host, { preferences.setHostEnabled(host,enabled) }) {
            PresentationPolicy(health().allowsAttention(preferences.hostEnabled(host)),preferences.notifyCompleted(host)) { event -> runtime.suppressesAlert(host,event) }
        }
    }
    suspend fun setCompleted(host:String,enabled:Boolean) {
        runtime.store.reconcilePresentations(host, { preferences.setNotifyCompleted(host,enabled) }) {
            PresentationPolicy(health().allowsAttention(preferences.hostEnabled(host)),preferences.notifyCompleted(host)) { event -> runtime.suppressesAlert(host,event) }
        }
    }
    suspend fun forgetHost(host:String) { reconcile(host) }

    override fun post(host:String,event:AttentionEvent):PresentationOutcome {
        val name=runtime.store.state.value.hosts.firstOrNull {it.hostId==host}?.name ?: return PresentationOutcome.BlockedBeforeAttempt
        return notificationAttempt({health().allowsAttention(preferences.hostEnabled(host))}) {
            manager.notify(tag(host,event.eventId.value),ALERT_ID,alert(name,host,event))
        }
    }
    override fun postHint(host:String,hint:PushEventHint):PresentationOutcome {
        val name=runtime.store.state.value.hosts.firstOrNull{it.hostId==host}?.name ?: return PresentationOutcome.BlockedBeforeAttempt
        val completed=hint.category==AttentionCategory.Completed
        val alert=Notification.Builder(context,ALERT_CHANNEL).setSmallIcon(R.drawable.ic_notification_goat)
            .setContentTitle(if(completed)"Work finished on desktop"else"Desktop needs attention").setContentText("$name · Open to synchronize")
            .setAutoCancel(true).setOnlyAlertOnce(true).setCategory(Notification.CATEGORY_STATUS)
            .setContentIntent(attentionIntent(host,hint.eventId.value)).build()
        return notificationAttempt({health().allowsAttention(preferences.hostEnabled(host))}) {manager.notify(tag(host,hint.eventId.value),ALERT_ID,alert)}
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
