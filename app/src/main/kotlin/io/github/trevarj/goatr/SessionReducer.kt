// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.serialization.Serializable

/** Memory-only reducer. Pending forms/originals and unsent drafts never enter SessionStore.
 * Call on its single writer; transport decodes the strict protocol boundary first. */
class SessionReducer {
    @ConsistentCopyVisibility
    data class HistoryPageTicket internal constructor(val target: Target, internal val subscription: Uuid, internal val revision: Long, val viewId: Uuid?)
    var state: SessionState? = null
        private set
    var history: List<HistoryRecord> = emptyList()
        private set
    var pending: List<PendingRequest> = emptyList()
        private set
    var nextCursor: Id? = null
        private set
    var partial: Boolean = true
        private set
    var browsingOlder: Boolean = false
        private set
    var viewId: Uuid? = null
        private set
    var controlsRevision:Long=0
        private set
    var queueRevision:Long=0
        private set
    private var browseRevision = 0L
    private var subscription: Uuid? = null
    private var applied = 0L
    private var ready = false
    private var receiptsRecovered = false
    private val composer = mutableMapOf<Target, String>()
    private val origins = mutableMapOf<Id,HistoryOrigins>()
    val mutationsEnabled: Boolean get() = ready && receiptsRecovered && state?.summary?.connectivity == SessionConnectivity.Ready
    /** The authoritative installed subscription is also the display-content source. */
    val source: Uuid? get() = subscription.takeIf { ready }
    enum class Decision { Applied, Duplicate, Resnapshot }

    fun snapshot(snapshot: SessionOpenResult) {
        snapshot.validate()
        val prior = state?.summary
        if (prior != null && (prior.sessionId != snapshot.state.summary.sessionId || prior.generation != snapshot.state.summary.generation)) composer.clear()
        state = snapshot.state
        history = snapshot.history.filter { it.complete }.sortedBy { it.order.value }
        origins.clear(); history.forEach { origins[it.id] = HistoryOrigins().also { source -> source.observe(it) } }
        pending = snapshot.pendingRequests
        subscription = snapshot.subscription
        applied = 0
        controlsRevision=0;queueRevision=0
        browseRevision++
        viewId = null
        nextCursor = null
        browsingOlder = false
        partial = snapshot.state.summary.historyCoverage.olderAvailable || snapshot.state.summary.historyCoverage.sourceTruncated || snapshot.state.summary.historyCoverage.scope == HistoryScope.Unavailable || snapshot.state.summary.historyCoverage.source == HistorySource.Unavailable
        ready = true
        receiptsRecovered = false
    }
    fun recoveryReconciled() { receiptsRecovered = true }
    fun disconnected() { ready = false; receiptsRecovered = false; pending = emptyList() }
    fun composer(target: Target): String? = composer[target]
    fun composer(target: Target, text: String) { PromptText(text); composer[target] = text }
    fun clearComposer(target: Target) { composer.remove(target) }
    fun event(event: Event): Decision {
        if (event.subscription != subscription || !ready || event.seq.value==0L) { disconnected(); return Decision.Resnapshot }
        if (event.seq.value <= applied) return Decision.Duplicate
        if (event.seq.value != applied + 1 || event.body is EventBody.SyncRequired) { disconnected(); return Decision.Resnapshot }
        val current = state ?: return Decision.Resnapshot
        when (val body = event.body) {
            is EventBody.ControlsChanged->{
                if(body.data.target.sessionId!=current.summary.sessionId||body.data.target.generation!=current.summary.generation) {disconnected();return Decision.Resnapshot}
                controlsRevision=event.seq.value
            }
            is EventBody.QueueChanged->{
                if(body.data.target.sessionId!=current.summary.sessionId||body.data.target.generation!=current.summary.generation) {disconnected();return Decision.Resnapshot}
                queueRevision=event.seq.value
            }
            is EventBody.SessionState -> {
                if (body.data.state.summary.sessionId != current.summary.sessionId || body.data.state.summary.generation != current.summary.generation) { disconnected(); return Decision.Resnapshot }
                state = body.data.state
            }
            is EventBody.HistoryUpsert -> merge(body.data.records)
            is EventBody.ActivityUpdate -> state = current.copy(activity = body.data.activity)
            is EventBody.RequestOpened -> {
                val old = pending.indexOfFirst { it.requestKey == body.data.request.requestKey }
                if (old < 0 && pending.size >= 64) { disconnected(); return Decision.Resnapshot }
                pending = if (old < 0) pending + body.data.request else pending.toMutableList().also { it[old] = body.data.request }
            }
            is EventBody.RequestClosed -> pending = pending.filterNot { it.requestKey == body.data.requestKey }
            else -> { disconnected(); return Decision.Resnapshot }
        }
        applied = event.seq.value
        return Decision.Applied
    }
    /** Capture before issuing session.history; a gap or replacement snapshot expires this ticket. */
    fun historyPage(target: Target, cursor: Id? = null): HistoryPageTicket {
        val current = state ?: throw StorePolicy(ErrorCode.NotReady)
        if (!ready) throw StorePolicy(ErrorCode.CursorExpired)
        if (target != Target(current.summary.sessionId, current.summary.generation)) throw StorePolicy(ErrorCode.StaleSession)
        if (cursor != null && (!browsingOlder || viewId == null || cursor != nextCursor)) throw StorePolicy(ErrorCode.CursorExpired)
        if (cursor == null) browseRevision++
        return HistoryPageTicket(target,subscription ?: throw StorePolicy(ErrorCode.NotReady),browseRevision,if (cursor == null) null else viewId)
    }
    fun page(ticket: HistoryPageTicket, result: SessionHistoryResult) {
        val current = state ?: throw StorePolicy(ErrorCode.NotReady)
        if (!ready || ticket.subscription != subscription || ticket.revision != browseRevision) throw StorePolicy(ErrorCode.CursorExpired)
        if (ticket.target != Target(current.summary.sessionId, current.summary.generation)) throw StorePolicy(ErrorCode.StaleSession)
        result.validate()
        if (ticket.viewId != null && result.viewId != ticket.viewId) throw StorePolicy(ErrorCode.CursorExpired)
        val overlay = if (ticket.viewId == null) history.associateBy { it.id } else emptyMap()
        if (ticket.viewId == null) { history = emptyList(); origins.clear() }
        viewId = result.viewId
        browsingOlder = true
        merge(result.records,historical=true)
        if (overlay.isNotEmpty()) merge(overlay.values.filter { prior -> history.any { it.id == prior.id } })
        // The local browse window may evict newer rows, but its capacity never ends the
        // source-bound native traversal represented by the server cursor.
        nextCursor = result.nextCursor
    }
    private fun merge(records: List<HistoryRecord>, historical: Boolean = false) {
        val provider = state?.summary?.provider ?: throw StorePolicy(ErrorCode.NotReady)
        val merged = history.associateByTo(linkedMapOf()) { it.id }
        records.filter { it.complete && (historical || !browsingOlder || merged.containsKey(it.id)) }.forEach { incoming ->
            val source = origins.getOrPut(incoming.id) { HistoryOrigins() }
            merged[incoming.id] = merged[incoming.id]?.let { mergeDisplayHistory(provider, it, incoming, source, historical) } ?: incoming.also { source.observe(it) }
        }
        var rows = merged.values.sortedBy { it.order.value }
        var bytes = rows.sumOf { Protocol.strictJson.encodeToString(HistoryRecord.serializer(), it).toByteArray().size.toLong() }
        var drop = 0
        while (rows.size - drop > 1000 || bytes > 64L * 1024 * 1024) {
            val index = if (browsingOlder) rows.lastIndex-drop else drop
            bytes -= Protocol.strictJson.encodeToString(HistoryRecord.serializer(), rows[index]).toByteArray().size
            drop++; partial = true
        }
        if (drop > 0) rows = if (browsingOlder) rows.dropLast(drop) else rows.drop(drop)
        history = rows
        origins.keys.retainAll(rows.map { it.id }.toSet())
    }
}

@Serializable
data class HistoryOrigins(var persistedText: Boolean = false, var persistedOriginal: Boolean = false, var liveTool: Boolean = false) {
    fun observe(record: HistoryRecord) {
        persistedText = persistedText || record.source == RecordSource.Persisted && record.text != null
        persistedOriginal = persistedOriginal || record.source == RecordSource.Persisted && record.original != null
        liveTool = liveTool || record.source == RecordSource.Live && record.tool != null
    }
}
fun mergeDisplayHistory(provider: Provider, old: HistoryRecord, newer: HistoryRecord, origins: HistoryOrigins, historical: Boolean = false): HistoryRecord {
    var incoming = newer.copy(order = old.order)
    fun fillTool(current: Tool?, older: Tool?): Tool? = current?.copy(callId=current.callId ?: older?.callId,input=current.input ?: older?.input,output=current.output ?: older?.output) ?: older
    if (provider == Provider.Codex && old.source == RecordSource.Live && (historical || incoming.source != RecordSource.Live)) {
        return old.copy(text = old.text ?: incoming.text, original = old.original ?: incoming.original,
            parentId = old.parentId ?: incoming.parentId, turnId = old.turnId ?: incoming.turnId,
            timestamp = old.timestamp ?: incoming.timestamp, tool = fillTool(old.tool,incoming.tool))
    }
    incoming = incoming.copy(text=incoming.text ?: old.text,original=incoming.original ?: old.original,
        parentId=incoming.parentId ?: old.parentId,turnId=incoming.turnId ?: old.turnId,
        timestamp=incoming.timestamp ?: old.timestamp,tool=fillTool(incoming.tool,old.tool))
    if (provider == Provider.Omp) {
        if (origins.persistedText && (historical || incoming.source == RecordSource.Live) || historical && old.source == RecordSource.Live && incoming.source == RecordSource.Live) incoming = incoming.copy(text = old.text)
        if (origins.persistedOriginal && (historical || incoming.source == RecordSource.Live)) incoming = incoming.copy(original = old.original)
        if ((historical || origins.liveTool && incoming.source == RecordSource.Persisted) && old.tool != null) {
            val base = incoming.tool ?: old.tool
            incoming = incoming.copy(tool = base.copy(status = old.tool.status, output = old.tool.output ?: base.output))
        }
    }
    origins.observe(incoming)
    return incoming
}

/** Shared source freshness predicate; display-only overflow is not source loss. */
internal fun hostSourceCurrent(ready:Boolean,inventory:HostSnapshotResult?,server:Uuid):Boolean = ready && inventory?.let {snapshot->
    snapshot.servers.none {it.serverId==server && it.connectivity!=ServerConnectivity.Online} &&
        snapshot.hierarchy.none {it.serverId==server && (it.freshness==NativeFreshness.Stale || it.reason==NativeUnavailableReason.SourceUnavailable)}
}==true

/** One host subscription, durable notification sequence remains separate from this sequence. */
class HostReducer {
    var inventory: HostSnapshotResult? = null
        private set
    var ready = false
        private set
    private var applied = 0L
    fun snapshot(snapshot: HostSnapshotResult) { snapshot.validate(); inventory = snapshot; applied = 0; ready = true }
    fun disconnected() { ready = false }
    /** Display overflow is not source loss and must not disable supported native controls. */
    internal fun sourceCurrent(server: Uuid): Boolean = hostSourceCurrent(ready,inventory,server)
    fun event(event: Event): SessionReducer.Decision {
        if (!ready || inventory?.subscription != event.subscription || event.seq.value==0L) { ready = false; return SessionReducer.Decision.Resnapshot }
        if (event.seq.value <= applied) return SessionReducer.Decision.Duplicate
        if (event.seq.value != applied + 1 || event.body is EventBody.SyncRequired || event.body is EventBody.HostChanged) { ready = false; return SessionReducer.Decision.Resnapshot }
        if (event.body !is EventBody.ActionStatus && event.body !is EventBody.NotificationUpsert && event.body !is EventBody.NotificationRead) { ready = false; return SessionReducer.Decision.Resnapshot }
        applied = event.seq.value
        return SessionReducer.Decision.Applied
    }
}

/** Byte offsets can split UTF-8. Assemble exactly, then decode strictly; never clip or decode
 * individual chunks. The owner discards this memory on generation/source change or navigation. */
class ContentAssembly(val target: Target, val reference: ContentRef) {
    private val bytes: ByteArray
    private var offset = 0
    private var complete = false
    init { target.validate(); reference.validate(); bytes = ByteArray(reference.byteLength.value.toInt()) }
    fun request(limit: Int = 65536): ContentReadParams {
        requireWire(limit in 1..65536)
        return ContentReadParams(target,reference.handle,Uint(offset.toLong()),Uint(limit.toLong()))
    }
    fun accept(result: ContentReadResult) {
        result.validate()
        val chunk = java.util.Base64.getDecoder().decode(result.bytes.value)
        try {
            requireWire(!complete && chunk.size <= bytes.size-offset && result.nextOffsetBytes.value == offset.toLong()+chunk.size)
            requireWire(result.eof == (offset+chunk.size == bytes.size) && (chunk.isNotEmpty() || result.eof))
            chunk.copyInto(bytes,offset); offset += chunk.size; complete = result.eof
        } finally {chunk.fill(0)}
    }
    fun text(): String {
        requireWire(complete)
        return try { Charsets.UTF_8.newDecoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT).onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT).decode(java.nio.ByteBuffer.wrap(bytes)).toString() } catch (_: java.nio.charset.CharacterCodingException) { invalidWire() }
    }
    fun clear() { bytes.fill(0); offset=0; complete=false }
}
