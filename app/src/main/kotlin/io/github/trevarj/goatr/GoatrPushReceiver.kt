// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.MessagingReceiver
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/** Policy precedes SDK token admission, including explicit-component broadcasts. */
@Suppress("DEPRECATION")
class GoatrPushReceiver:BroadcastReceiver(){
 private val delegate=object:MessagingReceiver(){
  private fun push(context:Context)=(context.applicationContext as GoatrApplication).runtime.push
  override fun onNewEndpoint(context:Context,endpoint:PushEndpoint,instance:String){push(context).callbackEndpoint(endpoint,instance)}
  override fun onMessage(context:Context,message:PushMessage,instance:String){push(context).callbackMessage(message,instance)}
  override fun onRegistrationFailed(context:Context,reason:FailedReason,instance:String){push(context).callbackFailure(instance)}
  override fun onUnregistered(context:Context,instance:String){push(context).callbackFailure(instance)}
  override fun onTempUnavailable(context:Context,instance:String){push(context).callbackFailure(instance)}
 }
 override fun onReceive(context:Context,intent:Intent){
  try{
   if(intent.action !in actions)return
   if(intent.action in switching&&intent.hasExtra("newDistributor"))return
   if(intent.getByteArrayExtra("bytesMessage")?.size?.let{it>4096}==true)return
   if(intent.getStringExtra("endpoint")?.length?.let{it>2048}==true)return
  }catch(_:Exception){return}
  val pending=goAsync();val deadline=android.os.SystemClock.elapsedRealtime()+8000
  try{executor.execute{try{if(android.os.SystemClock.elapsedRealtime()<deadline)delegate.onReceive(context.applicationContext,intent)}catch(_:Exception){}finally{pending.finish()}}}catch(_:java.util.concurrent.RejectedExecutionException){pending.finish()}
 }
 private companion object{
  const val prefix="org.unifiedpush.android.connector."
  val actions=setOf(prefix+"MESSAGE",prefix+"NEW_ENDPOINT",prefix+"REGISTRATION_FAILED",prefix+"TEMP_UNAVAILABLE",prefix+"UNREGISTERED")
  val switching=setOf(prefix+"TEMP_UNAVAILABLE",prefix+"UNREGISTERED")
  val executor=ThreadPoolExecutor(1,1,30,TimeUnit.SECONDS,ArrayBlockingQueue<Runnable>(8),ThreadPoolExecutor.AbortPolicy()).apply{allowCoreThreadTimeOut(true)}
 }
}
