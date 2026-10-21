// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.util.Base64
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.serialization.json.*

class PairingInvitation private constructor(val host: EndpointId, val name: Label, val expiresAt: Uint, internal var token: Key32?) {
    companion object {
        fun parse(uri: String): PairingInvitation {
            requireWire(validUnicode(uri) && utf8Size(uri) <= 4096)
            val prefix = "goatr://pair/v1?data="
            requireWire(uri.startsWith(prefix))
            val data = uri.substring(prefix.length)
            requireWire(data.isNotEmpty() && data.length <= 2731 && data.all {it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '-' || it == '_'})
            val bytes = try {Base64.getUrlDecoder().decode(data)} catch (_: IllegalArgumentException) {invalidWire()}
            try {
                requireWire(bytes.size <= 2048 && Base64.getUrlEncoder().withoutPadding().encodeToString(bytes) == data)
                val json = Protocol.parseRawJson(bytes) as? JsonObject ?: invalidWire()
                requireWire(json.keys == setOf("v","host","name","token","expiresAt"))
                val version = json.getValue("v") as? JsonPrimitive ?: invalidWire()
                requireWire(!version.isString && version.content == "1")
                fun text(key: String): String {val value=json.getValue(key) as? JsonPrimitive ?: invalidWire();requireWire(value.isString);return value.content}
                val expiry=json.getValue("expiresAt") as? JsonPrimitive ?: invalidWire()
                requireWire(!expiry.isString && uintToken.matches(expiry.content))
                return PairingInvitation(EndpointId(text("host")),Label(text("name")),Uint(expiry.content.toLongOrNull() ?: invalidWire()),Key32(text("token")))
            } finally {bytes.fill(0)}
        }
    }
    internal fun clear() {token=null}
}

data class PairingPreview(val host: EndpointId, val name: Label, val fingerprint: String, val expiresAt: Uint) {
    val authority = "Pairing grants inventory, history, control and notifications for ALL supported same-user sessions on this host, not just configured projects."
    val relayDisclosure = "Public relays and discovery observe connection metadata. Session traffic remains end-to-end encrypted."
}

/** The scanner owner and paste screen call this SAME gate. Parsing never pairs. Posted scan
 * callbacks after Back/disposal cannot accept or connect; confirmation is an explicit action. */
class PairingGate : AutoCloseable {
    private val alive = AtomicBoolean(true)
    private var invite: PairingInvitation? = null
    private var accepted = false
    private var confirmation: Job? = null
    @Synchronized fun submit(uri: String): PairingPreview? {
        if (!alive.get() || accepted) return null
        val parsed = try {PairingInvitation.parse(uri)} catch (_: Exception) {return null}
        invite?.clear(); invite=parsed
        return PairingPreview(parsed.host,parsed.name,parsed.host.value.chunked(8).joinToString(" "),parsed.expiresAt)
    }
    suspend fun confirm(connection: GoatrConnection): HelloResult {
        currentCoroutineContext().ensureActive()
        val job=currentCoroutineContext()[Job]
        val candidate=synchronized(this) {
            check(alive.get() && !accepted) {"Pairing confirmation is inactive"}
            val candidate=invite ?: throw StorePolicy(ErrorCode.InvalidRequest)
            accepted=true; confirmation=job; candidate
        }
        var completed=false
        return try {connection.pair(candidate) {alive.get()}.also {completed=true}} finally {candidate.clear(); synchronized(this) {invite=null;confirmation=null;if(!completed && alive.get()) accepted=false}}
    }
    @Synchronized override fun close() {alive.set(false);invite?.clear();invite=null;confirmation?.cancel();confirmation=null}
}
