// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** App-private presentation choices. They do not register push, alter pairing, or contain text. */
enum class DeliveryMode { Persistent,Ntfy }

class DeliveryPreferences(context: Context) {
    private val app=context.applicationContext
    private val preferences=app.getSharedPreferences(NAME,Context.MODE_PRIVATE)
    private val revision=AtomicLong(0)
    private val mutableChanges=MutableStateFlow(0L)
    val changes:StateFlow<Long> = mutableChanges

    internal fun mode(host:String):DeliveryMode=if(preferences.getString("mode.${key(host)}",null)=="ntfy")DeliveryMode.Ntfy else DeliveryMode.Persistent
    internal fun setMode(host:String,mode:DeliveryMode){preferences.edit().putString("mode.${key(host)}",if(mode==DeliveryMode.Ntfy)"ntfy"else"persistent").apply();changed()}
    internal fun requiresKeeper(host:String,pushVerified:Boolean):Boolean=hostEnabled(host)&&(mode(host)==DeliveryMode.Persistent||!pushVerified)
    internal fun pushHealthChanged(){changed()}
    fun serviceRequested():Boolean=preferences.getBoolean(SERVICE_REQUESTED,false)
    fun setServiceRequested(value:Boolean) {preferences.edit().putBoolean(SERVICE_REQUESTED,value).apply();changed()}
    fun hostEnabled(host:String):Boolean=preferences.getBoolean("enabled.${key(host)}",false)
    fun setHostEnabled(host:String,value:Boolean) {preferences.edit().putBoolean("enabled.${key(host)}",value).apply();changed()}
    fun notifyCompleted(host:String):Boolean=preferences.getBoolean("completed.${key(host)}",false)
    fun setNotifyCompleted(host:String,value:Boolean) {preferences.edit().putBoolean("completed.${key(host)}",value).apply();changed()}
    fun anyEnabled():Boolean=preferences.all.any { (key,value)->key.startsWith("enabled.") && value==true }
    fun clearHost(host:String) {preferences.edit().remove("enabled.${key(host)}").remove("completed.${key(host)}").remove("mode.${key(host)}").apply();changed()}
    fun canPostNotifications():Boolean=app.getSystemService(android.app.NotificationManager::class.java).areNotificationsEnabled() && (Build.VERSION.SDK_INT<33 || app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)==PackageManager.PERMISSION_GRANTED)

    private fun key(host:String):String=MessageDigest.getInstance("SHA-256").digest(host.toByteArray(Charsets.UTF_8)).joinToString("") {"%02x".format(it.toInt() and 0xff)}
    private fun changed() {mutableChanges.value=revision.incrementAndGet()}
    private companion object {const val NAME="delivery_preferences";const val SERVICE_REQUESTED="service_requested"}
}
