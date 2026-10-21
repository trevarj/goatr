// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.util.Base64
import java.security.MessageDigest
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive

@Serializable
internal data class LocalPushRegistration(
 val version:Int,val host:String,val device:Uuid,val generation:Uuid,val probe:Uuid,val key:String,
 val createdAt:Long,val endpoint:String?=null,val probeReceived:Boolean=false,val acknowledged:Boolean=false,
 val repair:Boolean=false,val unregister:Boolean=false,
 val firstHostRegistrationAt:Long?=null,
){override fun toString()="LocalPushRegistration(redacted)"}
@Serializable
internal data class PushProbeHint(val v:Int,val host:EndpointId,val device:Uuid,val endpointGeneration:Uuid,val probe:Uuid,val category:String)
/** A verified hint has no conversation generation or event creation timestamp. */
@Serializable
data class PushEventHint internal constructor(val v:Int,val host:EndpointId,val device:Uuid,val endpointGeneration:Uuid,val sessionId:Uuid,val eventId:Uuid,val sequence:Uint,val category:AttentionCategory){override fun toString()="PushEventHint(redacted)"}
internal sealed interface VerifiedPushHint {
 data class Probe(val hint:PushProbeHint):VerifiedPushHint
 data class Event(val hint:PushEventHint):VerifiedPushHint
}
internal object PushHints{
 private val flags=Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP
 internal fun encode(bytes:ByteArray):String=Base64.encodeToString(bytes,flags)
 internal fun decode(value:String):ByteArray{
  if(value.isEmpty()||!value.matches(Regex("[A-Za-z0-9_-]+")))throw StorePolicy(ErrorCode.PushRejected)
  val bytes=try{Base64.decode(value,flags)}catch(_:IllegalArgumentException){throw StorePolicy(ErrorCode.PushRejected)}
  if(encode(bytes)!=value)throw StorePolicy(ErrorCode.PushRejected);return bytes
 }
 fun verify(bytes:ByteArray,registration:LocalPushRegistration):VerifiedPushHint{
  if(bytes.size !in 1..2048||bytes.any{it.toInt() !in 0x20..0x7e})throw StorePolicy(ErrorCode.PushRejected)
  val parts=bytes.toString(Charsets.US_ASCII).split('.');if(parts.size!=3||parts[0]!="G1")throw StorePolicy(ErrorCode.PushRejected)
  val supplied=decode(parts[2]);if(supplied.size!=32)throw StorePolicy(ErrorCode.PushRejected)
  val key=decode(registration.key);if(key.size!=32)throw StorePolicy(ErrorCode.PushRejected)
  val expected=try{Mac.getInstance("HmacSHA256").run{init(SecretKeySpec(key,"HmacSHA256"));doFinal("G1.${parts[1]}".toByteArray(Charsets.US_ASCII))}}finally{key.fill(0)}
  // No JSON field or unauthenticated hint metadata is interpreted before MAC.
  if(!MessageDigest.isEqual(expected,supplied))throw StorePolicy(ErrorCode.PushRejected)
  val payload=decode(parts[1]);val raw=Protocol.parseRawJson(payload) as? JsonObject ?: throw StorePolicy(ErrorCode.PushRejected)
  val text=raw.toString()
  return if(raw["category"]?.jsonPrimitive?.content=="probe"){
   val hint=Protocol.strictJson.decodeFromString<PushProbeHint>(text)
   if(hint.v!=1||hint.category!="probe"||hint.host.value!=registration.host||hint.device!=registration.device||hint.endpointGeneration!=registration.generation||hint.probe!=registration.probe)throw StorePolicy(ErrorCode.PushRejected)
   hint.host.validateWire();hint.device.validateWire();hint.endpointGeneration.validateWire();hint.probe.validateWire();VerifiedPushHint.Probe(hint)
  }else{
   val hint=Protocol.strictJson.decodeFromString<PushEventHint>(text)
   if(hint.v!=1||hint.host.value!=registration.host||hint.device!=registration.device||hint.endpointGeneration!=registration.generation||hint.sequence.value==0L)throw StorePolicy(ErrorCode.PushRejected)
   hint.host.validateWire();hint.device.validateWire();hint.endpointGeneration.validateWire();hint.sessionId.validateWire();hint.eventId.validateWire();hint.sequence.validateWire();VerifiedPushHint.Event(hint)
  }
 }
}
