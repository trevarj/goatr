// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Application
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** A caller-owned lease for the process-wide store and Iroh endpoint. */
data class RuntimeClaim internal constructor(internal val id: Uuid, internal val kind: RuntimeClaimKind)
enum class RuntimeClaimKind { Ui, Delivery }

/**
 * The application is the only lifetime owner of native transport and durable state. Claims are
 * minted before a suspension, so an old lifecycle teardown cannot release a new lifecycle's
 * transport. No claim changes pairing, keys, or retained state.
 */
class GoatrRuntime(private val app: Application) {
    val store = SessionStore(app)
    val secrets = SecretStore(app)
    val connection = GoatrConnection(app, store, secrets)
    val deliveryPreferences = DeliveryPreferences(app)
    /** Runtime-owned presentation sink; no service or view model makes another coordinator. */
    val notifications = NotificationCoordinator(app, this)
    val cleanupScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    internal val push=PushCoordinator(app,this)
    private val claimsMutex = Mutex()
    private val claims = mutableSetOf<Uuid>()
    @Volatile private var visibleConversation: ConversationKey? = null

    init { store.installPresentationSink(notifications) }

    /** This is deliberately synchronous: callers must retain the exact token before suspending. */
    fun newClaim(kind: RuntimeClaimKind): RuntimeClaim = RuntimeClaim(Uuid.random(), kind)

    suspend fun acquire(claim: RuntimeClaim) = claimsMutex.withLock {
        if (claim.id in claims) return@withLock
        val first = claims.isEmpty()
        claims += claim.id
        if (!first) return@withLock
        try {
            store.open()
            store.drainPresentationCancellations()
            connection.start()
        } catch (failure: Throwable) {
            claims -= claim.id
            // A partly started endpoint is shut down only when no later claimant arrived.
            if (claims.isEmpty()) withContext(NonCancellable) { try { connection.shutdown() } catch (_: Exception) {} }
            throw failure
        }
    }

    suspend fun release(claim: RuntimeClaim) = claimsMutex.withLock {
        if (!claims.remove(claim.id) || claims.isNotEmpty()) return@withLock
        withContext(NonCancellable) { connection.shutdown() }
    }

    suspend fun forgetDeliveryHost(host: String) {
        push.forget(host)
        notifications.forgetHost(host)
        deliveryPreferences.clearHost(host)
        if (!deliveryPreferences.anyEnabled()) DeliveryService.stopWhenUnused(app)
    }

    /** UI-only memory, never persisted or used as session routing authority. */
    fun setVisibleConversation(key: ConversationKey?) { visibleConversation = key }
    fun suppressesAlert(host: String, event: AttentionEvent): Boolean =
        visibleConversation == ConversationKey(host, Target(event.sessionId, event.generation))
}
