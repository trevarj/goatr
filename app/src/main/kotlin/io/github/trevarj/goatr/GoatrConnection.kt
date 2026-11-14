// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import computer.iroh.*
import herdr.core.Duplex
import io.github.trevarj.goatr.app.session.Host
import io.github.trevarj.goatr.app.session.Store
import java.io.IOException
import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

private const val BRIDGE_WRITE_BUDGET = 32L * 1024 * 1024

/** Local transport deadlines are failures; cancellation of the caller must propagate unchanged. */
internal suspend inline fun <T : Any> withIrohDeadline(
    timeoutMillis: Long,
    operationMessage: String,
    crossinline block: suspend () -> T,
): T = withTimeoutOrNull(timeoutMillis) { block() } ?: throw IOException(operationMessage)

internal fun Channel<ByteArray>.tryQueueBridgeWrite(bytes: ByteArray, retainedBytes: AtomicLong): Boolean {
    val size = bytes.size.toLong()
    while (true) {
        val retained = retainedBytes.get()
        if (size > BRIDGE_WRITE_BUDGET - retained) return false
        if (retainedBytes.compareAndSet(retained, retained + size)) break
    }
    var copy: ByteArray? = null
    var queued = false
    try {
        copy = bytes.copyOf()
        queued = trySend(copy).isSuccess
        return queued
    } finally {
        if (!queued) {
            copy?.fill(0)
            retainedBytes.addAndGet(-size)
        }
    }
}

internal fun ByteArray.releaseBridgeWrite(retainedBytes: AtomicLong) {
    fill(0)
    retainedBytes.addAndGet(-size.toLong())
}

/** Installation-scoped endpoint. No invitation, keys or terminal bytes are logged. */
class GoatrConnection(context: Context, private val store: Store) {
    private val app = context.applicationContext
    private val secrets = SecretStore(app)
    private val preferences = app.getSharedPreferences("local_transport", Context.MODE_PRIVATE)
    private val lock = Mutex()
    private var endpoint: Endpoint? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val connectivity = app.getSystemService(ConnectivityManager::class.java)
    private val changed = Channel<Unit>(Channel.CONFLATED)
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { changed.trySend(Unit) }
        override fun onCapabilitiesChanged(network: Network, capabilities: android.net.NetworkCapabilities) { changed.trySend(Unit) }
        override fun onLinkPropertiesChanged(network: Network, properties: android.net.LinkProperties) { changed.trySend(Unit) }
        override fun onLost(network: Network) { changed.trySend(Unit) }
    }
    private val alpn = "goatr/hc/1".toByteArray()
    val relayLabels = linkedMapOf(
        "https://use1-1.relay.n0.iroh.link./" to "North America East",
        "https://usw1-1.relay.n0.iroh.link./" to "North America West",
        "https://euc1-1.relay.n0.iroh.link./" to "Europe",
        "https://aps1-1.relay.n0.iroh.link./" to "Asia-Pacific",
    )
    fun savedRelay(): String? = preferences.getString("home_relay_region", null)
    init {
        connectivity.registerDefaultNetworkCallback(callback)
        scope.launch { for (ignored in changed) lock.withLock {
            endpoint?.let { runCatching { withTimeout(5000) { it.networkChange() } } }
        } }
    }
    suspend fun relayRegions(): Map<String, String> = withContext(Dispatchers.IO) {
        IrohAndroid.installAndroidContext(app)
        RelayMode.defaultMode().use { mode -> mode.relayMap().use { map ->
            relayLabels.filterKeys { map.get(it) != null }
        } }
    }
    /** Saving retires every old stream before a new endpoint can use the new region. */
    suspend fun saveRelay(url: String?) = lock.withLock {
        require(url == null || relayRegions().containsKey(url)) { "Unavailable relay region" }
        check(preferences.edit().apply { if (url == null) remove("home_relay_region") else putString("home_relay_region", url) }.commit())
        retire()
    }
    private suspend fun retire() = withContext(NonCancellable) {
        val old = endpoint; endpoint = null
        if (old != null) try { withTimeoutOrNull(5000) { old.shutdown() } } finally { old.close() }
    }
    private suspend fun owner(): Endpoint {
        endpoint?.let { return it }
        val savedHosts = checkNotNull(store.savedHosts.first { it != null })
        IrohAndroid.installAndroidContext(app)
        val selected = savedRelay()
        val relay = if (selected == null) null else RelayMode.defaultMode().use { mode ->
            mode.relayMap().use { catalog ->
                require(selected in relayLabels) { "Unavailable saved relay region" }
                val config = requireNotNull(catalog.get(selected)) { "Unavailable saved relay region" }
                RelayMap.empty().use { map -> map.insert(config.copy()); RelayMode.custom(map) }
            }
        }
        val key = secrets.material(SecretStore.ENDPOINT_ALIAS, savedHosts.none { it.peer != null })
        val options = EndpointOptions(
            preset = object : Preset { override fun apply(builder: EndpointBuilder) { builder.applyN0(); builder.disablePortMapping() } },
            secretKey = key, alpns = listOf(alpn), relayMode = relay,
        )
        return try {
            withIrohDeadline(15000, "Iroh endpoint initialization timed out") {
                Endpoint.bind(options)
            }.also { endpoint = it }
        }
        finally { key.fill(0); options.destroy() }
    }
    private class Link(val connection: Connection, val stream: BiStream, val send: SendStream, val recv: RecvStream) {
        private val closed = AtomicBoolean()
        fun close() { if (closed.compareAndSet(false, true)) {
            connection.close(0L, "closed".toByteArray()); send.close(); recv.close(); stream.close(); connection.close()
        } }
    }
    private suspend fun dial(peer: String): Link {
        val id = computer.iroh.EndpointId.fromString(peer)
        val address = EndpointAddr(id, null, emptyList())
        var connection: Connection? = null
        try {
            val endpoint = owner()
            val timeoutMillis = store.currentPrefs().irohConnectTimeoutSeconds * 1000L
            val c = withIrohDeadline(timeoutMillis, "Desktop connection timed out") { endpoint.connect(address, alpn) }
            connection = c
            require(c.alpn().contentEquals(alpn) && c.remoteId().use { it.toString() } == peer) { "Desktop identity mismatch" }
            val bi = withIrohDeadline(10000, "Desktop stream opening timed out") { c.openBi() }
            return Link(c, bi, bi.send(), bi.recv())
        } catch (e: Exception) { connection?.let { it.close(0L, "closed".toByteArray()); it.close() }; throw e }
        finally { address.close(); id.close() }
    }
    private suspend fun request(link: Link, body: RequestBody): ResultBody = withIrohDeadline(10000, "Desktop handshake timed out") {
        val id = Uuid(UUID.randomUUID().toString())
        val bytes = Protocol.encodePayload(Envelope.RequestFrame(Request(id, body)))
        try {
            link.send.writeAll(ByteBuffer.allocate(4).putInt(bytes.size).array())
            link.send.writeAll(bytes)
        } finally { bytes.fill(0) }
        val size = ByteBuffer.wrap(link.recv.readExact(4u)).int
        require(size in 1..65536) { "Invalid desktop handshake" }
        val responseBytes = link.recv.readExact(size.toUInt())
        try {
            val response = (Protocol.decodePayload(responseBytes, body.method) as? Envelope.ResponseFrame)?.response
                ?: error("Invalid desktop handshake")
            require(response.id == id) { "Unexpected desktop response" }
            check(response.error == null) { "Desktop refused authorization" }
            checkNotNull(response.result)
        } finally { responseBytes.fill(0) }
    }
    private suspend fun hello(link: Link, peer: String): HelloResult {
        val result = (request(link, RequestBody.Hello(HelloParams(io.github.trevarj.goatr.app.BuildConfig.VERSION_NAME, Label("Goatr Android")))) as ResultBody.Hello).result
        require(result.hostId.value == peer) { "Desktop identity mismatch" }
        return result
    }
    /** Pin BEFORE spending the token. On response loss retry only authenticated known-peer hello. */
    suspend fun pair(invite: PairingInvitation, active: () -> Boolean = { true }): HelloResult = lock.withLock {
        require(invite.expiresAt.value > System.currentTimeMillis()) { "Invitation expired" }
        check(active())
        // Initialize the installation identity before adding a retained pin; never rotate a lost identity.
        owner()
        currentCoroutineContext().ensureActive(); check(active())
        store.addHost(Host("iroh:${invite.host.value}", "", invite.name.value,
            peer = invite.host.value, fingerprint = invite.host.value, paired = false))
        var link = dial(invite.host.value)
        try {
            try {
                check(active()); currentCoroutineContext().ensureActive()
                val paired = request(link, RequestBody.PairConfirm(PairConfirmParams(checkNotNull(invite.token), Label("Goatr Android")))) as ResultBody.PairConfirm
                require(paired.result.hostId == invite.host)
            } catch (e: Exception) {
                currentCoroutineContext().ensureActive()
                link.close(); check(active()); link = dial(invite.host.value)
            } finally { invite.clear() }
            val result = hello(link, invite.host.value)
            check(active()); store.updateHost("iroh:${invite.host.value}") { it.copy(paired = true) }
            store.setPrefs { it.copy(desktop = invite.host.value) }
            result
        } finally { link.close() }
    }
    suspend fun open(peer: String): Duplex = lock.withLock {
        val saved = store.savedHosts.first { it != null }!!.singleOrNull { it.peer == peer }
        require(saved != null && saved.paired) { "Desktop is not paired" }
        val link = dial(peer)
        try { hello(link, peer); Bridge(link) } catch (e: Exception) { link.close(); throw e }
    }
    private class Bridge(private val link: Link) : Duplex {
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        private val ended = AtomicBoolean()
        private val data = CompletableDeferred<(ByteArray) -> Unit>()
        // Bound queued + in-flight copies, not individual frames: hc uploads are single base64 RPCs.
        private val retainedBytes = AtomicLong()
        private val writes = Channel<ByteArray>(32, onUndeliveredElement = { it.releaseBridgeWrite(retainedBytes) })
        private var listener: ((String?) -> Unit)? = null
        init {
            scope.launch { try { for (bytes in writes) { try { link.send.writeAll(bytes) } finally { bytes.releaseBridgeWrite(retainedBytes) } } }
                catch (_: Exception) { end() } }
            scope.launch { try {
                val deliver = data.await()
                while (true) { val bytes = link.recv.read(16384u); if (bytes.isEmpty()) break; deliver(bytes) }
            } catch (_: Exception) { } finally { end() } }
        }
        @Synchronized private fun end() {
            if (!ended.compareAndSet(false, true)) return
            writes.close(); while (true) { val bytes = writes.tryReceive().getOrNull() ?: break; bytes.releaseBridgeWrite(retainedBytes) }
            link.close(); scope.cancel(); listener?.invoke("Desktop connection ended")
        }
        override fun write(bytes: ByteArray) {
            if (ended.get()) return
            if (!writes.tryQueueBridgeWrite(bytes, retainedBytes)) end()
        }
        override fun onData(listener: (ByteArray) -> Unit) { check(data.complete(listener)) }
        @Synchronized override fun onClose(listener: (String?) -> Unit) {
            check(this.listener == null); this.listener = listener
            if (ended.get()) listener("Desktop connection ended")
        }
        override fun close() = end()
    }
}
