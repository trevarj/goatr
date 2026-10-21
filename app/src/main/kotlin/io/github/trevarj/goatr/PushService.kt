// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/** Nonexported connector service; receiving never acquires an Iroh runtime claim. */
class PushService:org.unifiedpush.android.connector.PushService(){
 private val push get()=(application as GoatrApplication).runtime.push
 override fun onNewEndpoint(endpoint:PushEndpoint,instance:String){push.callbackEndpoint(endpoint,instance)}
 override fun onMessage(message:PushMessage,instance:String){push.callbackMessage(message,instance)}
 override fun onRegistrationFailed(reason:FailedReason,instance:String){push.callbackFailure(instance)}
 override fun onUnregistered(instance:String){push.callbackFailure(instance)}
 override fun onTempUnavailable(instance:String){push.callbackFailure(instance)}
}
