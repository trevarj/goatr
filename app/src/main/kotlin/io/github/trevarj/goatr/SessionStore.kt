// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import android.os.ParcelFileDescriptor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString

class StorePolicy(val code: ErrorCode) : Exception(code.name)
class RecoveryApprovalRequired(val warning: RecoveryWarning) : Exception("Recovery replacement requires explicit approval")
@Serializable
data class RecoveryIdentity(val hostId: String, val actionId: Uuid)
@Serializable
data class RecoveryExpiry(val identity: RecoveryIdentity, val expiresAt: Uint)
@Serializable
data class RecoveryWarning(val replace: Set<RecoveryIdentity>, val futureExpiry: RecoveryExpiry? = null)
/** Supplied only after the exact action/deadline/affected drafts are visible. Normal Send may
 * acknowledge the visible retention notice; this never approves another action's eviction. */
data class RecoveryApproval(val replace: Set<RecoveryIdentity> = emptySet(), val expiry: RecoveryExpiry? = null)
@Serializable
data class RecoveryConsent(val warning: RecoveryWarning, val approvedAt: Uint)

@Serializable
data class HostMetadata(val hostId: String, val name: String, val deviceId: Uuid? = null, val keyAliases: List<String> = emptyList())
@Serializable
data class WorktreeRemovalIntent(val candidateId:Uuid,val previewId:Uuid)
@Serializable
data class ActionMetadata(val actionId: Uuid, val method: ActionMethod, val bootId: Uuid, val target: Target?, val createdAt: Uint, val sourceTarget: Target? = null,val shellTarget:ShellTarget? = null,val worktreeRemoval:WorktreeRemovalIntent? = null,val attachmentUpload:Uuid?=null)
data class LocalAction(val metadata: ActionMetadata, val receipt: Receipt?, val authoritative: Boolean, val outcomeShown: Boolean)
sealed interface PreSendDecision {
    data class NewAction(val metadata: ActionMetadata) : PreSendDecision
    data class ExistingAction(val action: LocalAction) : PreSendDecision
}
@Serializable
data class ComposerOptions(val images:List<Uuid>?=null,val delivery:PromptDelivery?=null,val nativeTurnId:Id?=null) {
    fun byteSize():Int=if(images==null&&delivery==null&&nativeTurnId==null)0 else utf8Size(Protocol.strictJson.encodeToString(this))
}
data class RecoveryDraft(val hostId: String, val action: ActionMetadata, val text: String, val expiresAt: Uint,val options:ComposerOptions=ComposerOptions())
data class CachedSession(val hostId: String, val summary: SessionSummary, val history: List<HistoryRecord>, val stale: Boolean = true, val partial: Boolean = true)
/** Serialized display only; never install this as a live HostReducer snapshot. */
data class CachedInventory(val hostId: String, val inventory: HostSnapshotResult, val savedAt: Uint) {
    val stale: Boolean get() = true
}
data class KeyDeletion(val hostId: String, val aliases: List<String>)
data class ComposerSnippet(val id: Uuid, val hostId: String, val title: String, val body: String)
/** A durable notification projection. It intentionally has no provider text or route authority. */
data class StoredAttentionEvent(val event: AttentionEvent, val read: Boolean)
/** Attempt accounting only; active ownership never proves OS delivery. */
enum class PresentationOutcome { BlockedBeforeAttempt,AttemptedHealthy,AttemptedUnknown }
/** Synchronous Android-only side effect invoked while the store admission lock is held. */
interface PresentationSink {
    fun post(host: String, event: AttentionEvent): PresentationOutcome
    fun postHint(host:String,hint:PushEventHint):PresentationOutcome
    fun cancel(host: String, eventId: String): Boolean
    fun noLongerPresent(host: String, eventId: String): Boolean
}
data class PresentationPolicy(val enabled: Boolean, val includeCompleted: Boolean, val suppress: (AttentionEvent) -> Boolean)
data class StoreView(val hosts: List<HostMetadata> = emptyList(), val cached: List<CachedSession> = emptyList(), val recovery: List<RecoveryDraft> = emptyList(), val keyDeletion: List<KeyDeletion> = emptyList(), val inventory: List<CachedInventory> = emptyList(), val snippets: List<ComposerSnippet> = emptyList(), val routes: List<SavedHostRoutes> = emptyList(),val attachments:List<LocalAttachmentUpload> = emptyList())

/** Adapter-owned redaction must happen before this acknowledgement: no universal scanner can
 * recognize every provider secret/capability link. Only explicit completed display projections
 * belong here; PendingRequest and Answer have deliberately no persistence API. */
class SafeDisplayHistory private constructor(val record: HistoryRecord) {
    companion object { fun redacted(record: HistoryRecord): SafeDisplayHistory { record.validate(); requireWire(record.complete); return SafeDisplayHistory(record) } }
}

/** Actual Android SQLite, one writer for reads and writes, StateFlow published after commit.
 * Keys are caller-supplied from package3's protected key provider and never retained here.
 * Unsubmitted composer/form/secret state belongs exclusively to SessionReducer's memory. */
class SessionStore(context: Context, name: String = "goatr-private.db", private val now: () -> Long = System::currentTimeMillis) {
    companion object {
        /** LOCAL deletion-obligation scope, deliberately impossible as a remote EndpointId.
         * It is never inserted into hosts and grants no pairing/session authority. */
        const val INSTALLATION_DELETION_SCOPE = "local.installation"
        const val CACHE_BYTES = 50L * 1024 * 1024
        const val CACHE_AGE = 30L * 24 * 60 * 60 * 1000
        const val RECOVERY_AGE = 7L * 24 * 60 * 60 * 1000
        const val RECOVERY_BYTES = 1024L * 1024
        const val RECOVERY_COUNT = 20
        const val CONTENT_TTL = 15L * 60 * 1000
    }
    private val helper = Database(context.applicationContext, name)
    private val mutex = Mutex()
    private val writer = Dispatchers.IO.limitedParallelism(1)
    private val mutable = MutableStateFlow(StoreView())
    val state: StateFlow<StoreView> = mutable.asStateFlow()
    private var opened = false
    private var closed = false
    /** Writer-owned invalidation keys, not retained history or a deferred-write queue. */
    private val historyChanges=linkedSetOf<Pair<String,String>>()
    private val committedHistoryChanges=linkedSetOf<Pair<String,String>>()
    private var collectingHistoryChanges=false
    private fun historyChanged(host:String,session:String){check(collectingHistoryChanges);historyChanges.add(host to session)}
    private data class StoredRecord(val data:String,val origins:String,val order:Long,val bytes:Long,val at:Long)
    @Volatile private var presentationSink: PresentationSink? = null
    /** Installed once by the application runtime, never by a UI or service. */
    fun installPresentationSink(sink: PresentationSink) { presentationSink = sink }
    private class Database(context: Context, name: String) : SQLiteOpenHelper(context, name, null, 8) {
        override fun onConfigure(db: SQLiteDatabase) {
            db.setForeignKeyConstraintsEnabled(true)
            // This PRAGMA returns a row; Android execSQL rejects row-returning statements.
            db.rawQuery("PRAGMA secure_delete=ON", null).use {
                check(it.moveToFirst() && it.getInt(0) == 1) { "SQLite secure deletion unavailable" }
            }
        }
        override fun onCreate(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE hosts(host TEXT PRIMARY KEY,metadata TEXT NOT NULL)")
            db.execSQL("CREATE TABLE sessions(host TEXT NOT NULL,session TEXT NOT NULL,generation TEXT NOT NULL,summary TEXT NOT NULL,partial INTEGER NOT NULL DEFAULT 1,viewed INTEGER NOT NULL DEFAULT 0,used INTEGER NOT NULL,source TEXT,PRIMARY KEY(host,session),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE records(host TEXT NOT NULL,session TEXT NOT NULL,id TEXT NOT NULL,ord INTEGER NOT NULL,data TEXT NOT NULL,bytes INTEGER NOT NULL,at INTEGER NOT NULL,origins TEXT NOT NULL,PRIMARY KEY(host,session,id),FOREIGN KEY(host,session) REFERENCES sessions(host,session) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE content(host TEXT NOT NULL,session TEXT NOT NULL,generation TEXT NOT NULL,source TEXT NOT NULL,handle TEXT NOT NULL,data BLOB NOT NULL,bytes INTEGER NOT NULL,at INTEGER NOT NULL,expires INTEGER NOT NULL,used INTEGER NOT NULL,PRIMARY KEY(host,handle),FOREIGN KEY(host,session) REFERENCES sessions(host,session) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE actions(host TEXT NOT NULL,action TEXT NOT NULL,metadata TEXT NOT NULL,fingerprint BLOB NOT NULL,receipt TEXT,state TEXT,authoritative INTEGER NOT NULL DEFAULT 0,shown INTEGER NOT NULL DEFAULT 0,updated INTEGER NOT NULL,PRIMARY KEY(host,action),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE recovery(host TEXT NOT NULL,action TEXT NOT NULL,text TEXT NOT NULL,bytes INTEGER NOT NULL,at INTEGER NOT NULL,expires INTEGER NOT NULL,consent TEXT NOT NULL,options TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(host,action),FOREIGN KEY(host,action) REFERENCES actions(host,action) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE notifications(host TEXT NOT NULL,sequence INTEGER NOT NULL,id TEXT NOT NULL,event TEXT NOT NULL,session TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(host,sequence),UNIQUE(host,id),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE notification_state(host TEXT PRIMARY KEY,highwater INTEGER NOT NULL DEFAULT 0,retained INTEGER NOT NULL DEFAULT 1,presentation_floor INTEGER NOT NULL DEFAULT 0,FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            presentationTables(db)
            db.execSQL("CREATE TABLE readwater(host TEXT NOT NULL,session TEXT NOT NULL,through INTEGER NOT NULL,last_event INTEGER NOT NULL,PRIMARY KEY(host,session),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            // Encrypted opaque bytes only. Package3/7 own encryption, registration/probe semantics.
            db.execSQL("CREATE TABLE private_registration(host TEXT PRIMARY KEY,generation TEXT NOT NULL,ciphertext BLOB NOT NULL,FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE retired_push(host TEXT NOT NULL,generation TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(host,generation),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            db.execSQL("CREATE TABLE key_deletion(host TEXT PRIMARY KEY,aliases TEXT NOT NULL)")
            inventoryTable(db)
            snippetsTable(db)
            routesTable(db)
            attachmentTable(db)
            pushTables(db)
        }
        private fun inventoryTable(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE inventory(host TEXT PRIMARY KEY,payload TEXT NOT NULL,bytes INTEGER NOT NULL,at INTEGER NOT NULL,FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
        }
        private fun snippetsTable(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE snippets(id TEXT PRIMARY KEY,host TEXT NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL,title_bytes INTEGER NOT NULL,body_bytes INTEGER NOT NULL,FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
        }
        private fun presentationTables(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE notification_presentations(host TEXT NOT NULL,event TEXT NOT NULL,sequence INTEGER NOT NULL,session TEXT NOT NULL,disposition TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 0,attempted_at INTEGER NOT NULL,PRIMARY KEY(host,event),UNIQUE(host,sequence),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
            // Deliberately no host FK: explicit Forget must retain an uncertain OS-cancel obligation.
            db.execSQL("CREATE TABLE notification_cancel_pending(host TEXT NOT NULL,event TEXT NOT NULL,attempted_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(host,event))")
        }
        private fun routesTable(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE host_routes(host TEXT PRIMARY KEY,revision INTEGER NOT NULL,direct TEXT NOT NULL,relay TEXT,FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")
        }
        private fun attachmentTable(db:SQLiteDatabase){db.execSQL("CREATE TABLE attachment_local(host TEXT NOT NULL,upload TEXT NOT NULL,record TEXT NOT NULL,PRIMARY KEY(host,upload))")}
        private fun pushTables(db:SQLiteDatabase){db.execSQL("CREATE TABLE push_hints(host TEXT NOT NULL,event TEXT NOT NULL,sequence INTEGER NOT NULL,session TEXT NOT NULL,category TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(host,event),UNIQUE(host,sequence),FOREIGN KEY(host) REFERENCES hosts(host) ON DELETE CASCADE)")}
        override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
            check(oldVersion in 1..7 && newVersion==8) {"Unsupported private state version"}
            if(oldVersion<2) inventoryTable(db)
            if(oldVersion<3) snippetsTable(db)
            if(oldVersion<4) {
                db.execSQL("ALTER TABLE notification_state ADD COLUMN presentation_floor INTEGER NOT NULL DEFAULT 0")
                db.execSQL("UPDATE notification_state SET presentation_floor=highwater")
                presentationTables(db)
            }
            if(oldVersion<5)routesTable(db)
            if(oldVersion<6)attachmentTable(db)
            if(oldVersion<7)pushTables(db)
            if(oldVersion<8)db.execSQL("ALTER TABLE recovery ADD COLUMN options TEXT NOT NULL DEFAULT '{}'")
        }
    }
    private fun clock(): Long = now().also { Uint(it) }
    private suspend fun <T> write(block: (SQLiteDatabase, Long) -> T): T = withContext(writer) {
        mutex.withLock {
            check(!closed)
            val db = helper.writableDatabase
            if (!opened) { db.execSQL("UPDATE sessions SET viewed=0"); opened = true }
            val time = clock()
            val committed=committedHistoryChanges
            val attempted=historyChanges
            check(committed.isEmpty()&&attempted.isEmpty())
            collectingHistoryChanges=true
            try {
                // Expiry commits independently, even when the requested operation is rejected.
                db.beginTransaction()
                var success=false
                try {trim(db,time);db.setTransactionSuccessful();success=true}
                finally {db.endTransaction();if(success)committed.addAll(attempted)}
                attempted.clear()
                db.beginTransaction()
                success=false
                try {val value=block(db,time);trim(db,time);db.setTransactionSuccessful();success=true;value}
                finally {db.endTransaction();if(success)committed.addAll(attempted)}
            } finally {
                collectingHistoryChanges=false
                try {mutable.value=load(db,time,committed)}
                finally {committed.clear();attempted.clear()}
            }
        }
    }
    suspend fun open() { write { _, _ -> } }
    /** App-private cleanup only. It is independent of paired-host rows, so Forget of the last
     * host cannot strand an acknowledged Goatr notification cancellation. */
    suspend fun drainPresentationCancellations() = withContext(writer) {
        mutex.withLock {
            check(!closed)
            val sink=presentationSink ?: return@withLock
            val db=helper.writableDatabase
            val time=clock()
            drainPendingCancels(db,time,sink)
            mutable.value=load(db,time)
        }
    }
    suspend fun close() = withContext(writer) { mutex.withLock { helper.close(); closed = true } }
    private fun requireHost(db: SQLiteDatabase, host: String) {
        if (scalar(db, "SELECT count(*) FROM hosts WHERE host=?", host) != 1L) throw StorePolicy(ErrorCode.Unauthorized)
    }
    private fun requirePushDevice(db:SQLiteDatabase,host:String,device:Uuid) {
        val metadata=string(db,"SELECT metadata FROM hosts WHERE host=?",host)?.let {Protocol.strictJson.decodeFromString<HostMetadata>(it)} ?: throw StorePolicy(ErrorCode.NotReady)
        if(metadata.deviceId!=device)throw StorePolicy(ErrorCode.NotReady)
    }
    private fun validateSnippet(title: String, body: String) {
        val titleBytes = title.toByteArray(Charsets.UTF_8).size
        val bodyBytes = body.toByteArray(Charsets.UTF_8).size
        if (!validUnicode(title) || !validUnicode(body) || title.codePointCount(0, title.length) > 80 || titleBytes > 320 || body.isEmpty() || bodyBytes > 8192) throw StorePolicy(ErrorCode.InvalidRequest)
    }
    /** Separate from Hello metadata so a concurrent authenticated update cannot erase hints. */
    suspend fun saveHostRoutes(host: String, expectedRevision: Long, direct: List<String>, relay: String?): SavedHostRoutes = write { db, _ ->
        requireHost(db,host)
        val metadata=Protocol.strictJson.decodeFromString<HostMetadata>(string(db,"SELECT metadata FROM hosts WHERE host=?",host) ?: throw StorePolicy(ErrorCode.Unauthorized))
        if(metadata.deviceId==null) throw StorePolicy(ErrorCode.Unauthorized)
        val (addresses,relayUrl)=HostRoutes.validate(direct,relay)
        val revision=scalar(db,"SELECT revision FROM host_routes WHERE host=?",host)
        if(revision!=expectedRevision) throw StorePolicy(ErrorCode.NotReady)
        if(revision==Long.MAX_VALUE) throw StorePolicy(ErrorCode.ResourceLimit)
        val next=revision+1
        db.execSQL("INSERT OR REPLACE INTO host_routes(host,revision,direct,relay) VALUES(?,?,?,?)",arrayOf<Any?>(host,next,Protocol.strictJson.encodeToString(addresses),relayUrl))
        SavedHostRoutes(host,next,addresses,relayUrl)
    }
    /** Explicit local user-authored content only; it never imports drafts, forms, or provider data. */
    suspend fun saveSnippet(host: String, id: Uuid, title: String, body: String) = write { db, _ ->
        requireHost(db, host); validateSnippet(title, body)
        val owner = string(db, "SELECT host FROM snippets WHERE id=?", id.value)
        if (owner != null && owner != host) throw StorePolicy(ErrorCode.InvalidRequest)
        if (owner == null && scalar(db, "SELECT count(*) FROM snippets WHERE host=?", host) >= 32L) throw StorePolicy(ErrorCode.ResourceLimit)
        if (owner == null && scalar(db, "SELECT count(*) FROM snippets") >= 128L) throw StorePolicy(ErrorCode.ResourceLimit)
        db.execSQL("INSERT OR REPLACE INTO snippets(id,host,title,body,title_bytes,body_bytes) VALUES(?,?,?,?,?,?)", arrayOf<Any?>(id.value, host, title, body, title.toByteArray(Charsets.UTF_8).size, body.toByteArray(Charsets.UTF_8).size))
    }
    suspend fun deleteSnippet(host: String, id: Uuid) = write { db, _ ->
        requireHost(db, host)
        db.execSQL("DELETE FROM snippets WHERE host=? AND id=?", arrayOf(host, id.value))
    }
    /** Persist known endpoint identity even before pair.confirm reply/hello device ID is received.
     * Never persist token/QR/raw capability, and never trust an unknown hello automatically. */
    suspend fun host(metadata: HostMetadata) = write { db, _ ->
        EndpointId(metadata.hostId); Label(metadata.name)
        requireWire(metadata.keyAliases.size <= 8 && metadata.keyAliases.distinct().size == metadata.keyAliases.size && metadata.keyAliases.all { it.matches(Regex("[a-zA-Z0-9_.-]{1,128}")) })
        if (scalar(db, "SELECT count(*) FROM key_deletion WHERE host=?", metadata.hostId) > 0) throw StorePolicy(ErrorCode.NotReady)
        if (scalar(db, "SELECT count(*) FROM hosts WHERE host=?", metadata.hostId) == 0L && scalar(db, "SELECT count(*) FROM hosts") >= 32) throw StorePolicy(ErrorCode.ResourceLimit)
        val encoded = Protocol.strictJson.encodeToString(metadata)
        db.execSQL("INSERT OR IGNORE INTO hosts VALUES(?,?)", arrayOf(metadata.hostId, encoded))
        db.execSQL("UPDATE hosts SET metadata=? WHERE host=?", arrayOf(encoded, metadata.hostId))
        db.execSQL("INSERT OR IGNORE INTO notification_state(host) VALUES(?)", arrayOf(metadata.hostId))
    }
    /** One last authenticated inventory per paired host, sharing cache age/bytes/Forget.
     * Source freshness and ephemeral associations become stale display before persistence. */
    suspend fun cacheInventory(host: String, snapshot: HostSnapshotResult) = write { db, time ->
        requireHost(db,host)
        Protocol.encodePayload(Envelope.ResponseFrame(Response(snapshot.subscription,ResultBody.HostSnapshot(snapshot))))
        val stale = snapshot.copy(shells=snapshot.shells.map {it.copy(connectivity=SessionConnectivity.Offline,capabilities=ShellCapabilities(false,false,false,false))},hierarchy=snapshot.hierarchy.map { h ->
            if(h.freshness != NativeFreshness.Current) h else h.copy(
                freshness=NativeFreshness.Stale,reason=NativeUnavailableReason.SourceUnavailable,
                panes=h.panes.map { it.copy(target=null,unboundRowId=null,shellTarget=null) })
        })
        val payload = Protocol.strictJson.encodeToString(stale)
        // Private display can add at most64 coarse stale reasons to a valid1MiB source frame.
        val size = utf8Size(payload)
        requireWire(size <= Protocol.FRAME_MAX + 4096)
        db.execSQL("INSERT INTO inventory VALUES(?,?,?,?) ON CONFLICT(host) DO UPDATE SET payload=excluded.payload,bytes=excluded.bytes,at=excluded.at",
            arrayOf<Any?>(host,payload,size,time))
    }
    /** Snapshot replaces the old source window; ordinary upserts cannot erase a partial marker.
     * Historical responses fill missing fields but never outrank already-applied live events. */
    suspend fun cache(host: String, summary: SessionSummary, records: List<SafeDisplayHistory>, partial: Boolean, snapshot: Boolean = false, historical: Boolean = false, source: Uuid? = null) = write { db, time ->
        requireHost(db, host); summary.validate()
        val previous = string(db, "SELECT generation FROM sessions WHERE host=? AND session=?", host, summary.sessionId.value)
        val oldSource = string(db,"SELECT source FROM sessions WHERE host=? AND session=?",host,summary.sessionId.value)
        if(!snapshot && previous != null && (previous != summary.generation.value || source != null && oldSource != source.value)) throw StorePolicy(ErrorCode.StaleSession)
        val activeSource = if(snapshot || previous != summary.generation.value) source?.value else source?.value ?: oldSource
        if(snapshot && oldSource != activeSource) {
            db.execSQL("DELETE FROM content WHERE host=? AND session=?",arrayOf(host,summary.sessionId.value))
            historyChanged(host,summary.sessionId.value)
        }
        if (previous != null && previous != summary.generation.value) {
            db.execSQL("DELETE FROM content WHERE host=? AND session=?", arrayOf(host, summary.sessionId.value))
            db.execSQL("DELETE FROM records WHERE host=? AND session=?", arrayOf(host, summary.sessionId.value))
            historyChanged(host,summary.sessionId.value)
        }
        if(snapshot) {
            val retained=records.mapTo(HashSet()) {it.record.id.value}
            val absent=db.rawQuery("SELECT id FROM records WHERE host=? AND session=?",arrayOf(host,summary.sessionId.value)).use {rows->buildList {
                while(rows.moveToNext()){val id=rows.getString(0);if(id !in retained)add(id)}
            }}
            for(id in absent)db.execSQL("DELETE FROM records WHERE host=? AND session=? AND id=?",arrayOf(host,summary.sessionId.value,id))
            if(absent.isNotEmpty())historyChanged(host,summary.sessionId.value)
        }
        // Do not REPLACE a parent row: SQLite REPLACE would cascade-delete existing history.
        db.execSQL("INSERT OR IGNORE INTO sessions(host,session,generation,summary,used) VALUES(?,?,?,?,?)", arrayOf<Any?>(host,summary.sessionId.value,summary.generation.value,Protocol.strictJson.encodeToString(summary),time))
        val incomplete = partial || summary.historyCoverage.olderAvailable || summary.historyCoverage.sourceTruncated || summary.historyCoverage.scope == HistoryScope.Unavailable || summary.historyCoverage.source == HistorySource.Unavailable
        val sticky = !snapshot && previous == summary.generation.value && scalar(db,"SELECT partial FROM sessions WHERE host=? AND session=?",host,summary.sessionId.value) != 0L
        db.execSQL("UPDATE sessions SET generation=?,summary=?,partial=?,used=?,source=? WHERE host=? AND session=?", arrayOf<Any?>(summary.generation.value,Protocol.strictJson.encodeToString(summary),if (incomplete || sticky) 1 else 0,time,activeSource,host,summary.sessionId.value))
        var displayChanged=false
        records.forEach { safe ->
            val record=safe.record
            val previous=db.rawQuery("SELECT data,origins,ord,bytes,at FROM records WHERE host=? AND session=? AND id=?",arrayOf(host,summary.sessionId.value,record.id.value)).use {row->
                if(row.moveToFirst())StoredRecord(row.getString(0),row.getString(1),row.getLong(2),row.getLong(3),row.getLong(4)) else null
            }
            // An authoritative snapshot resets provenance; never merge an old source into it.
            val oldRecord=previous?.takeIf {!snapshot}?.let {Protocol.strictJson.decodeFromString<HistoryRecord>(it.data)}
            val origins=previous?.takeIf {!snapshot}?.let {Protocol.strictJson.decodeFromString<HistoryOrigins>(it.origins)} ?: HistoryOrigins()
            val merged=oldRecord?.let {mergeDisplayHistory(summary.provider,it,record,origins,historical)} ?: record.also {origins.observe(it)}
            val data=Protocol.strictJson.encodeToString(merged)
            val encodedOrigins=Protocol.strictJson.encodeToString(origins)
            val size=if(previous!=null && previous.data==data)previous.bytes else utf8Size(data).toLong()
            if(previous==null) {
                db.execSQL("INSERT INTO records VALUES(?,?,?,?,?,?,?,?)",arrayOf<Any?>(host,summary.sessionId.value,record.id.value,merged.order.value,data,size,time,encodedOrigins))
                displayChanged=true
            } else if(previous.data!=data || previous.origins!=encodedOrigins || previous.order!=merged.order.value || previous.bytes!=size) {
                val update=android.content.ContentValues(5)
                if(previous.data!=data)update.put("data",data)
                if(previous.origins!=encodedOrigins)update.put("origins",encodedOrigins)
                if(previous.order!=merged.order.value)update.put("ord",merged.order.value)
                if(previous.bytes!=size)update.put("bytes",size)
                if(previous.at!=time)update.put("at",time)
                db.update("records",update,"host=? AND session=? AND id=?",arrayOf(host,summary.sessionId.value,record.id.value))
                if(previous.data!=data || previous.order!=merged.order.value)displayChanged=true
            } else if(previous.at!=time) {
                // Incoming/snapshot retention refresh is real; unchanged payload is not rewritten.
                db.execSQL("UPDATE records SET at=? WHERE host=? AND session=? AND id=?",arrayOf<Any?>(time,host,summary.sessionId.value,record.id.value))
            }
        }
        if(displayChanged)historyChanged(host,summary.sessionId.value)
        val count = scalar(db,"SELECT count(*) FROM records WHERE host=? AND session=?",host,summary.sessionId.value)
        if (count > 1000) {
            db.execSQL("DELETE FROM records WHERE host=? AND session=? AND id IN (SELECT id FROM records WHERE host=? AND session=? ORDER BY ord LIMIT ?)", arrayOf<Any?>(host,summary.sessionId.value,host,summary.sessionId.value,count - 1000))
            markPartial(db,host,summary.sessionId.value)
            historyChanged(host,summary.sessionId.value)
        }
    }
    suspend fun viewing(host: String, session: Uuid?) = write { db, time ->
        requireHost(db,host); db.execSQL("UPDATE sessions SET viewed=0 WHERE host=?",arrayOf(host))
        if (session != null) db.execSQL("UPDATE sessions SET viewed=1,used=? WHERE host=? AND session=?",arrayOf<Any?>(time,host,session.value))
    }
    /** Loaded redacted display content only, generation/source scoped; raw pending originals are
     * intentionally not accepted. Chunk assembly and strict UTF-8 decode are completed first. */
    suspend fun cacheContent(host: String, target: Target, source: Uuid, reference: ContentRef, safeText: String) = write { db, time ->
        requireHost(db,host); target.validate(); reference.validate(); requireWire(validUnicode(safeText))
        if (string(db,"SELECT generation FROM sessions WHERE host=? AND session=?",host,target.sessionId.value) != target.generation.value) throw StorePolicy(ErrorCode.StaleSession)
        if (string(db,"SELECT source FROM sessions WHERE host=? AND session=?",host,target.sessionId.value) != source.value) throw StorePolicy(ErrorCode.ContentExpired)
        val displayReference = db.rawQuery("SELECT data FROM records WHERE host=? AND session=? AND instr(data,?)>0",arrayOf(host,target.sessionId.value,reference.handle.value)).use { c ->
            var matched = false
            while(c.moveToNext() && !matched) {
                val r = Protocol.strictJson.decodeFromString<HistoryRecord>(c.getString(0))
                matched = r.complete && (r.text == reference || r.original == reference || r.tool?.input == reference || r.tool?.output == reference)
            }
            matched
        }
        if(!displayReference) throw StorePolicy(ErrorCode.InvalidRequest)
        val bytes = safeText.toByteArray(Charsets.UTF_8)
        requireWire(bytes.size.toLong() == reference.byteLength.value && bytes.size <= Protocol.CONTENT_MAX)
        db.execSQL("INSERT OR REPLACE INTO content VALUES(?,?,?,?,?,?,?,?,?,?)", arrayOf(host,target.sessionId.value,target.generation.value,source.value,reference.handle.value,bytes,bytes.size,time,time + CONTENT_TTL,time))
        historyChanged(host,target.sessionId.value)
    }
    suspend fun content(host: String, target: Target, source: Uuid, handle: Uuid): String? = write { db, time ->
        requireHost(db,host)
        val bytes = contentBytes(db,"host=? AND session=? AND generation=? AND source=? AND handle=? AND expires>?",host,target.sessionId.value,target.generation.value,source.value,handle.value,time.toString())
        if (bytes != null) db.execSQL("UPDATE content SET used=? WHERE host=? AND handle=?",arrayOf<Any?>(time,host,handle.value))
        bytes?.toString(Charsets.UTF_8)
    }
    suspend fun offlineContent(host: String, target: Target, handle: Uuid): String? = write { db, time ->
        requireHost(db,host)
        val bytes = contentBytes(db,"host=? AND session=? AND generation=? AND handle=? AND expires>?",host,target.sessionId.value,target.generation.value,handle.value,time.toString())
        if(bytes != null) db.execSQL("UPDATE content SET used=? WHERE host=? AND handle=?",arrayOf<Any?>(time,host,handle.value))
        bytes?.toString(Charsets.UTF_8)
    }
    private fun contentBytes(db: SQLiteDatabase, where: String, vararg args: String): ByteArray? {
        val length = db.rawQuery("SELECT length(data) FROM content WHERE $where",args).use { row -> if(row.moveToFirst()) row.getLong(0) else return null }
        requireWire(length in 0..Protocol.CONTENT_MAX.toLong())
        // Native SQLite's read-only BLOB descriptor avoids CursorWindow's smaller row ceiling.
        return db.compileStatement("SELECT data FROM content WHERE $where").use { statement ->
            args.forEachIndexed { index,value -> statement.bindString(index+1,value) }
            val descriptor = statement.simpleQueryForBlobFileDescriptor() ?: return null
            ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { input ->
                val bytes = ByteArray(length.toInt())
                var offset = 0
                while(offset < bytes.size) {
                    val count = input.read(bytes,offset,bytes.size-offset)
                    check(count > 0) { "Incomplete cached content" }
                    offset += count
                }
                check(input.read() == -1) { "Oversized cached content" }
                bytes
            }
        }
    }
    suspend fun invalidateSource(host: String, target: Target) = write { db, _ ->
        requireHost(db,host); db.execSQL("DELETE FROM content WHERE host=? AND session=?",arrayOf(host,target.sessionId.value)); markPartial(db,host,target.sessionId.value)
        db.execSQL("UPDATE sessions SET source=NULL WHERE host=? AND session=?",arrayOf(host,target.sessionId.value))
        historyChanged(host,target.sessionId.value)
    }
    /** Atomic pre-send metadata + keyed fingerprint + composer recovery. No Answer/raw request
     * enters SQLite. This is not permission to send or retry; package6 checks authoritative state. */
    suspend fun preSend(host: String, request: Request, protectedReceiptKey: ByteArray, ordinaryComposer: Boolean = false, approval: RecoveryApproval = RecoveryApproval()): PreSendDecision = write { db, time ->
        requireHost(db,host); requireWire(protectedReceiptKey.size == 32)
        Protocol.encodePayload(Envelope.RequestFrame(request))
        val metadata = action(request,time)
        val canonical = Protocol.canonicalRequest(request)
        val fingerprint = try { Protocol.hmacSha256(protectedReceiptKey,canonical) } finally { canonical.fill(0) }
        val old = db.rawQuery("SELECT fingerprint FROM actions WHERE host=? AND action=?",arrayOf(host,request.id.value)).use { if (it.moveToFirst()) it.getBlob(0) else null }
        if (old != null) {
            if (!java.security.MessageDigest.isEqual(old,fingerprint)) throw StorePolicy(ErrorCode.ActionIdConflict)
            return@write PreSendDecision.ExistingAction(localAction(db,host,request.id) ?: throw StorePolicy(ErrorCode.NotFound))
        }
        if (scalar(db,"SELECT count(*) FROM actions WHERE host=?",host) >= 10000) throw StorePolicy(ErrorCode.ActionCapacity)
        val text = if (ordinaryComposer) (request.body as? RequestBody.PromptSubmit)?.params?.text?.value ?: throw StorePolicy(ErrorCode.InvalidRequest) else null
        val prompt=(request.body as? RequestBody.PromptSubmit)?.params
        val options=if(ordinaryComposer&&prompt!=null)ComposerOptions(prompt.images,prompt.delivery,prompt.nativeTurnId)else ComposerOptions()
        var consent: RecoveryConsent? = null
        var deadline: Long? = null
        if (text != null) {
            val identity = RecoveryIdentity(host,request.id)
            val expiry = approval.expiry?.takeIf { it.identity == identity && it.expiresAt.value > time && it.expiresAt.value <= time + RECOVERY_AGE }
            val warning = recoveryWarning(db,(utf8Size(text)+options.byteSize()).toLong()).copy(futureExpiry=expiry ?: RecoveryExpiry(identity,Uint(time + RECOVERY_AGE)))
            if (expiry == null || approval.replace != warning.replace) throw RecoveryApprovalRequired(warning)
            warning.replace.forEach { id -> db.execSQL("DELETE FROM recovery WHERE host=? AND action=?",arrayOf(id.hostId,id.actionId.value)) }
            consent = RecoveryConsent(warning,Uint(time)); deadline = expiry.expiresAt.value
        }
        db.execSQL("INSERT INTO actions(host,action,metadata,fingerprint,updated) VALUES(?,?,?,?,?)",arrayOf(host,request.id.value,Protocol.strictJson.encodeToString(metadata),fingerprint,time))
        if (text != null) db.execSQL("INSERT INTO recovery(host,action,text,bytes,at,expires,consent,options) VALUES(?,?,?,?,?,?,?,?)",arrayOf<Any?>(host,request.id.value,text,utf8Size(text)+options.byteSize(),time,deadline,Protocol.strictJson.encodeToString(consent ?: throw StorePolicy(ErrorCode.InvalidRequest)),Protocol.strictJson.encodeToString(options)))
        PreSendDecision.NewAction(metadata)
    }
    suspend fun recoveryWarning(host: String, action: Uuid, text: PromptText,options:ComposerOptions=ComposerOptions()): RecoveryWarning = write { db,time ->
        requireHost(db,host); recoveryWarning(db,(utf8Size(text.value)+options.byteSize()).toLong()).copy(futureExpiry=RecoveryExpiry(RecoveryIdentity(host,action),Uint(time + RECOVERY_AGE)))
    }
    suspend fun recoveryConsent(host: String, action: Uuid): RecoveryConsent? = write { db,_ ->
        requireHost(db,host); string(db,"SELECT consent FROM recovery WHERE host=? AND action=?",host,action.value)?.let { Protocol.strictJson.decodeFromString<RecoveryConsent>(it) }
    }
    /** Lookup only: an old action may be inspected after its original viewer/chooser retires.
     * Absence never reserves a replacement or grants permission to send. */
    internal suspend fun matchingAction(host:String,request:Request,protectedReceiptKey:ByteArray):Boolean=write {db,_->
        requireHost(db,host);requireWire(protectedReceiptKey.size==32)
        val old=db.rawQuery("SELECT fingerprint FROM actions WHERE host=? AND action=?",arrayOf(host,request.id.value)).use {if(it.moveToFirst())it.getBlob(0) else null}
            ?: return@write false
        Protocol.encodePayload(Envelope.RequestFrame(request))
        val canonical=Protocol.canonicalRequest(request)
        val fingerprint=try {Protocol.hmacSha256(protectedReceiptKey,canonical)}finally {canonical.fill(0)}
        try {if(!java.security.MessageDigest.isEqual(old,fingerprint))throw StorePolicy(ErrorCode.ActionIdConflict)}finally {fingerprint.fill(0)}
        true
    }
    suspend fun localAction(host: String, action: Uuid): LocalAction? = write { db,_ -> requireHost(db,host); localAction(db,host,action) }
    /** Host-scoped UUID keyset page for action.status recovery, including actions without drafts.
     * No text, answer, request or keyed fingerprint enters this public projection. */
    suspend fun localActions(host: String, after: Uuid?, limit: Int): List<LocalAction> = write { db,_ ->
        requireHost(db,host); requireWire(limit in 1..100)
        db.rawQuery("SELECT metadata,receipt,authoritative,shown FROM actions WHERE host=? AND action>? ORDER BY action LIMIT ?",arrayOf(host,after?.value ?: "",limit.toString())).use { row ->
            buildList { while(row.moveToNext()) add(LocalAction(Protocol.strictJson.decodeFromString(row.getString(0)),if(row.isNull(1)) null else Protocol.strictJson.decodeFromString(row.getString(1)),row.getInt(2)!=0,row.getInt(3)!=0)) }
        }
    }
    private fun localAction(db: SQLiteDatabase, host: String, action: Uuid): LocalAction? =
        db.rawQuery("SELECT metadata,receipt,authoritative,shown FROM actions WHERE host=? AND action=?",arrayOf(host,action.value)).use {
            if(it.moveToFirst()) LocalAction(Protocol.strictJson.decodeFromString(it.getString(0)),if(it.isNull(1)) null else Protocol.strictJson.decodeFromString(it.getString(1)),it.getInt(2)!=0,it.getInt(3)!=0) else null
        }
    private fun recoveryWarning(db: SQLiteDatabase, incoming: Long): RecoveryWarning {
        val live = db.rawQuery("SELECT host,action,bytes,at FROM recovery ORDER BY at,host,action",null).use { c -> buildList { while(c.moveToNext()) add(Triple(RecoveryIdentity(c.getString(0),Uuid(c.getString(1))),c.getLong(2),c.getLong(3))) } }.toMutableList()
        var bytes = live.sumOf { it.second } + incoming
        val replace = linkedSetOf<RecoveryIdentity>()
        while (live.size + 1 > RECOVERY_COUNT || bytes > RECOVERY_BYTES) { val row = live.removeAt(0); bytes -= row.second; replace += row.first }
        return RecoveryWarning(replace)
    }
    /** A local receipt NEVER removes recovery. Record authoritative dispatched/succeeded first,
     * then package6 calls outcomeShown only after that exact outcome is visible to the user. */
    suspend fun receipt(host: String, receipt: Receipt, authoritative: Boolean) = write { db, time ->
        requireHost(db,host); receipt.validate()
        val metadata = string(db,"SELECT metadata FROM actions WHERE host=? AND action=?",host,receipt.actionId.value)?.let { Protocol.strictJson.decodeFromString<ActionMetadata>(it) } ?: throw StorePolicy(ErrorCode.NotFound)
        if (metadata.method != receipt.method || metadata.bootId != receipt.bootId || metadata.target != receipt.target || metadata.shellTarget != receipt.shellTarget || metadata.sourceTarget != receipt.creation?.sourceTarget) throw StorePolicy(ErrorCode.ActionIdConflict)
        if(receipt.result is ActionResultAttachment && (metadata.attachmentUpload!=receipt.result.progress.uploadId || receipt.target!=receipt.result.target))throw StorePolicy(ErrorCode.ActionIdConflict)
        if(receipt.method==ActionMethod.WorktreesRemove && receipt.result!=null){val result=receipt.result as? ActionResultWorktreeRemoval ?: throw StorePolicy(ErrorCode.ActionIdConflict);if(metadata.worktreeRemoval!=WorktreeRemovalIntent(result.candidateId,result.previewId))throw StorePolicy(ErrorCode.ActionIdConflict)}
        val prior = string(db,"SELECT receipt FROM actions WHERE host=? AND action=?",host,receipt.actionId.value)?.let { Protocol.strictJson.decodeFromString<Receipt>(it) }
        val priorAuthority = scalar(db,"SELECT authoritative FROM actions WHERE host=? AND action=?",host,receipt.actionId.value) == 1L
        if (priorAuthority && !authoritative) return@write
        if (prior == receipt && priorAuthority == authoritative) return@write
        // Local timestamps are a different clock. Only authoritative observations constrain
        // one another; a delayed status response must not overwrite a newer live outcome.
        if (priorAuthority && prior != null && !receiptAdvances(prior,receipt)) return@write
        db.execSQL("UPDATE actions SET receipt=?,state=?,authoritative=?,shown=0,updated=? WHERE host=? AND action=?",arrayOf<Any?>(Protocol.strictJson.encodeToString(receipt),receipt.state.name,if(authoritative) 1 else 0,if(authoritative) receipt.updatedAt.value else time,host,receipt.actionId.value))
    }
    private fun receiptAdvances(old: Receipt, next: Receipt): Boolean {
        if (next.createdAt != old.createdAt || next.updatedAt.value < old.updatedAt.value) return false
        val forward = when(old.state) {
            ReceiptState.Reserved -> true
            ReceiptState.Dispatching -> next.state != ReceiptState.Reserved
            ReceiptState.Dispatched -> next.state == ReceiptState.Dispatched || next.state == ReceiptState.Succeeded
            ReceiptState.Uncertain -> next.state == ReceiptState.Uncertain || next.state == ReceiptState.Succeeded || next.state == ReceiptState.Failed
            ReceiptState.Succeeded, ReceiptState.Failed -> false
        }
        if (!forward) return false
        if(old.result is ActionResultAttachment && (next.result !is ActionResultAttachment || old.result.target!=next.result.target || !next.result.progress.retains(old.result.progress)))return false
        if(old.method==ActionMethod.WorktreesRemove){val oldResult=old.result as? ActionResultWorktreeRemoval;val newResult=next.result as? ActionResultWorktreeRemoval;if(oldResult!=null && (newResult==null || oldResult.target!=newResult.target || oldResult.candidateId!=newResult.candidateId || oldResult.previewId!=newResult.previewId || !newResult.progress.retains(oldResult.progress)))return false}
        if(old.method==ActionMethod.AgentRename && old.result!=null && old.result!=next.result)return false
        if(old.method in listOf(ActionMethod.ShellCreate,ActionMethod.TerminalCreate)) {
            val known=(old.result as? ActionResultShellCreation)?.resources
            val nextFacts=(next.result as? ActionResultShellCreation)?.resources
            if(known!=null && (nextFacts==null || !shellFactsRetained(known,nextFacts))) return false
        }
        if(old.method==ActionMethod.ShellInput) {
            val known=(old.result as? ActionResultShellInput)?.progress
            val nextFacts=(next.result as? ActionResultShellInput)?.progress
            if(known!=null && (nextFacts==null || !nextFacts.retains(known))) return false
            val dispatch=(old.result as? ActionResultShellDispatch)?.progress
            val nextDispatch=(next.result as? ActionResultShellDispatch)?.progress
            if(dispatch!=null && (nextDispatch==null || !nextDispatch.retains(dispatch)))return false
        }
        val prior = old.creation ?: return true
        val progress = next.creation ?: return false
        if (prior.serverId != progress.serverId || prior.projectId != progress.projectId || prior.provider != progress.provider || prior.sourceTarget != progress.sourceTarget || progress.stage.ordinal < prior.stage.ordinal) return false
        if (progress.stage == prior.stage) {
            val stageForward = when(prior.stageState) {
                StageState.NotStarted -> true
                StageState.Dispatching -> progress.stageState != StageState.NotStarted
                StageState.Confirmed -> progress.stageState == StageState.Confirmed ||
                    (old.state == ReceiptState.Dispatching && prior.stage == CreationStage.Workspace &&
                        progress.stageState in listOf(StageState.Dispatching,StageState.Failed,StageState.Uncertain))
                StageState.Uncertain -> progress.stageState == StageState.Uncertain || progress.stageState == StageState.Confirmed || progress.stageState == StageState.Failed
                StageState.Failed -> progress.stageState == StageState.Failed
            }
            if (!stageForward) return false
        } else if (prior.stageState == StageState.Failed ||
            prior.stageState == StageState.Uncertain && !(
                progress.stage == CreationStage.Ready && progress.stageState == StageState.Confirmed &&
                (next.state == ReceiptState.Succeeded || old.state == ReceiptState.Uncertain && next.state == ReceiptState.Uncertain)
            )
        ) return false
        val a = prior.resources; val b = progress.resources
        if(a.checkout!=null) {
            val next=b.checkout ?: return false
            if(a.checkout.kind!=next.kind || a.checkout.branch!=next.branch || a.checkout.requestedBase!=next.requestedBase || (a.checkout.resolvedCommit!=null && a.checkout.resolvedCommit!=next.resolvedCommit)) return false
        }
        return (a.workspaceId == null || a.workspaceId == b.workspaceId) &&
            (a.paneId == null || a.paneId == b.paneId) &&
            (a.terminalId == null || a.terminalId == b.terminalId) &&
            (a.nativeId == null || a.nativeId == b.nativeId) &&
            (a.bindingId == null || a.bindingId == b.bindingId) &&
            (a.sessionId == null || a.sessionId == b.sessionId) &&
            (a.generation == null || a.generation == b.generation) &&
            (a.ownerStarted == null || a.ownerStarted == b.ownerStarted) &&
            (a.relayStarted == null || a.relayStarted == b.relayStarted)
    }
    suspend fun outcomeShown(host: String, action: Uuid, authoritativeOutcome: Receipt) = write { db, _ ->
        requireHost(db,host)
        val receipt = string(db,"SELECT receipt FROM actions WHERE host=? AND action=? AND authoritative=1",host,action.value)?.let { Protocol.strictJson.decodeFromString<Receipt>(it) } ?: throw StorePolicy(ErrorCode.NotReady)
        if (receipt != authoritativeOutcome || receipt.actionId != action) throw StorePolicy(ErrorCode.InvalidRequest)
        db.execSQL("UPDATE actions SET shown=1 WHERE host=? AND action=?",arrayOf(host,action.value))
        val options=string(db,"SELECT options FROM recovery WHERE host=? AND action=?",host,action.value)?.let {Protocol.strictJson.decodeFromString<ComposerOptions>(it)}
        if (receipt.state == ReceiptState.Succeeded&&options?.images==null) db.execSQL("DELETE FROM recovery WHERE host=? AND action=?",arrayOf(host,action.value))
    }
    suspend fun actionStatus(host: String, action: Uuid): Receipt? = write { db, _ ->
        requireHost(db,host); string(db,"SELECT receipt FROM actions WHERE host=? AND action=?",host,action.value)?.let { Protocol.strictJson.decodeFromString<Receipt>(it) }
    }
    /** Explicit discard after inspection. Does not resend/retarget; a new send needs a new action. */
    suspend fun discardRecovery(host: String, action: Uuid, approved: Boolean) = write { db, _ ->
        requireHost(db,host); if (!approved) throw StorePolicy(ErrorCode.InvalidRequest)
        db.execSQL("DELETE FROM recovery WHERE host=? AND action=?",arrayOf(host,action.value))
    }
    suspend fun notifications(host: String, sync: NotificationsSyncResult,admitted:()->Boolean={true}) = write { db, time ->
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
        requireHost(db,host); sync.validate()
        if (sync.resetRequired) {
            // A provider reset invalidates presentation eligibility, never event/read authority.
            cancelActiveHost(db,host,time)
            db.execSQL("UPDATE notification_state SET presentation_floor=max(presentation_floor,?) WHERE host=?",arrayOf<Any?>(sync.latestSequence.value,host))
        }
        // highwater is advertised, not received. retained also persists our consumed eviction
        // floor, so missing page/live events remain admissible but expired replays do not.
        db.execSQL("UPDATE notification_state SET highwater=max(highwater,?),retained=max(retained,?) WHERE host=?",arrayOf<Any?>(sync.latestSequence.value,sync.retainedFrom.value,host))
        val floor = scalar(db,"SELECT retained FROM notification_state WHERE host=?",host)
        db.execSQL("DELETE FROM notifications WHERE host=? AND sequence<?",arrayOf<Any?>(host,floor))
        sync.events.forEach { addNotification(db,host,it,time) }
        val authoritativeSessions = sync.read.mapTo(hashSetOf()) { it.sessionId.value }
        val obsolete = db.rawQuery("SELECT session FROM readwater WHERE host=? AND through<? AND NOT EXISTS(SELECT 1 FROM notifications WHERE notifications.host=readwater.host AND notifications.session=readwater.session)",arrayOf(host,sync.retainedFrom.value.toString())).use { row -> buildList { while(row.moveToNext()) if(row.getString(0) !in authoritativeSessions) add(row.getString(0)) } }
        obsolete.forEach { db.execSQL("DELETE FROM readwater WHERE host=? AND session=?",arrayOf(host,it)) }
        sync.read.forEach { read ->
            val old = scalar(db,"SELECT coalesce(max(through),0) FROM readwater WHERE host=? AND session=?",host,read.sessionId.value)
            if (sync.latestSequence.value < floor && old == 0L) return@forEach
            val lastEvent = db.rawQuery("SELECT max(at) FROM notifications WHERE host=? AND session=?",arrayOf(host,read.sessionId.value)).use { row -> if(row.moveToFirst() && !row.isNull(0)) row.getLong(0) else time }
            ensureWatermark(db,host,read.sessionId,lastEvent)
            db.execSQL("UPDATE readwater SET through=? WHERE host=? AND session=?",arrayOf<Any?>(maxOf(old,read.throughSequence.value),host,read.sessionId.value))
        }
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
    }
    suspend fun liveNotification(host: String, event: AttentionEvent,admitted:()->Boolean={true}) = write { db,time ->
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
        requireHost(db,host); event.validate()
        addNotification(db,host,event,time)
        db.execSQL("UPDATE notification_state SET highwater=max(highwater,?) WHERE host=?",arrayOf<Any?>(event.sequence.value,host))
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
    }
    /** Local stale cache projection, not a transport handler or push notification authority. */
    suspend fun cachedNotifications(host: String, after: Uint = Uint(0), limit: Int = 100): NotificationsSyncResult = write { db,_ ->
        requireHost(db,host); requireWire(limit in 1..100)
        val latest=scalar(db,"SELECT highwater FROM notification_state WHERE host=?",host)
        if(after.value>latest) throw StorePolicy(ErrorCode.InvalidRequest)
        val events=db.rawQuery("SELECT event FROM notifications WHERE host=? AND sequence>? ORDER BY sequence LIMIT ?",arrayOf(host,after.value.toString(),(limit+1).toString())).use { c -> buildList { while(c.moveToNext()) add(Protocol.strictJson.decodeFromString<AttentionEvent>(c.getString(0))) } }
        val page=events.take(limit)
        val read=db.rawQuery("SELECT session,through FROM readwater WHERE host=? ORDER BY session",arrayOf(host)).use { c -> buildList { while(c.moveToNext()) add(ReadWatermark(Uuid(c.getString(0)),Uint(c.getLong(1)))) } }
        val first=db.rawQuery("SELECT min(sequence) FROM notifications WHERE host=?",arrayOf(host)).use { if(it.moveToFirst() && !it.isNull(0)) it.getLong(0) else latest+1 }
        NotificationsSyncResult(page,read,Uint(latest),Uint(first),if(events.size>limit) page.last().sequence else null,after.value+1<first)
    }
    /** Exact retained event lookup for notification entry. The caller must still prove a current Target. */
    suspend fun notificationEvent(host: String, eventId: Uuid): StoredAttentionEvent? = write { db,_ ->
        requireHost(db,host)
        db.rawQuery(
            "SELECT n.event,n.sequence,n.session,coalesce(w.through,0) FROM notifications n LEFT JOIN readwater w ON w.host=n.host AND w.session=n.session WHERE n.host=? AND n.id=?",
            arrayOf(host,eventId.value),
        ).use { row ->
            if (!row.moveToFirst()) return@write null
            val event=Protocol.strictJson.decodeFromString<AttentionEvent>(row.getString(0)).also {it.validate()}
            if (event.sequence.value!=row.getLong(1) || event.sessionId.value!=row.getString(2)) throw StorePolicy(ErrorCode.InvalidRequest)
            StoredAttentionEvent(event,row.getLong(3)>=event.sequence.value)
        }
    }
    /** Bounded durable unread projection for OS presentation; retention and read watermarks stay authoritative. */
    suspend fun latestUnreadNotifications(host: String, limit: Int = 100): List<StoredAttentionEvent> = write { db,_ ->
        requireHost(db,host);requireWire(limit in 1..100)
        db.rawQuery(
            "SELECT n.event,n.sequence,n.session,coalesce(w.through,0) FROM notifications n LEFT JOIN readwater w ON w.host=n.host AND w.session=n.session WHERE n.host=? AND n.sequence>coalesce(w.through,0) ORDER BY n.sequence DESC LIMIT ?",
            arrayOf(host,limit.toString()),
        ).use { rows -> buildList {
            while(rows.moveToNext()) {
                val event=Protocol.strictJson.decodeFromString<AttentionEvent>(rows.getString(0)).also {it.validate()}
                if (event.sequence.value!=rows.getLong(1) || event.sessionId.value!=rows.getString(2)) throw StorePolicy(ErrorCode.InvalidRequest)
                add(StoredAttentionEvent(event,false))
            }
        } }
    }
    /**
     * Durable, at-most-once OS presentation. The event/read ledger remains the only source of
     * truth: this table records a presentation decision so a process restart cannot re-alert an
     * event. A crash after the write-ahead commit may omit an OS alert, but never fabricates one.
     */
    suspend fun reconcilePresentations(host: String, mutatePolicy: (() -> Unit)? = null, policy: () -> PresentationPolicy) = withContext(writer) {
        mutex.withLock {
            check(!closed)
            val db=helper.writableDatabase
            if (!opened) { db.execSQL("UPDATE sessions SET viewed=0"); opened=true }
            val time=clock()
            val sink=presentationSink ?: return@withLock
            val committed=committedHistoryChanges
            val attempted=historyChanges
            check(committed.isEmpty()&&attempted.isEmpty())
            collectingHistoryChanges=true
            fun transaction(block: () -> Unit) {
                check(attempted.isEmpty())
                db.beginTransaction()
                var success=false
                try {block();trim(db,time);db.setTransactionSuccessful();success=true}
                finally {
                    try {db.endTransaction();if(success)committed.addAll(attempted)}
                    finally {attempted.clear()}
                }
            }
            try {
            drainPendingCancels(db,time,sink)
            mutatePolicy?.invoke()
            if (scalar(db,"SELECT count(*) FROM hosts WHERE host=?",host)==0L) return@withLock
            val current=policy()
            val active=db.rawQuery("SELECT p.event,n.event FROM notification_presentations p LEFT JOIN notifications n ON n.host=p.host AND n.id=p.event WHERE p.host=? AND p.active=1",arrayOf(host)).use { rows->buildList {while(rows.moveToNext()) add(rows.getString(0) to rows.getString(1))} }
            active.forEach { (event,payload) ->
                val unread=scalar(db,"SELECT count(*) FROM (SELECT host,id AS event,session,sequence FROM notifications UNION ALL SELECT host,event,session,sequence FROM push_hints) n LEFT JOIN readwater w ON w.host=n.host AND w.session=n.session WHERE n.host=? AND n.event=? AND n.sequence>coalesce(w.through,0)",host,event)>0
                val completed=payload?.let {Protocol.strictJson.decodeFromString<AttentionEvent>(it).category==AttentionCategory.Completed} ?: (string(db,"SELECT category FROM push_hints WHERE host=? AND event=?",host,event)==AttentionCategory.Completed.name)
                if (!current.enabled || !current.includeCompleted && completed || !unread) cancelPresentation(db,host,event,time,sink)
            }
            val floor=scalar(db,"SELECT presentation_floor FROM notification_state WHERE host=?",host)
            // Consume the exact generationless hint for EVERY disposition, even while disabled/read.
            val hints=db.rawQuery("SELECT n.event FROM push_hints h JOIN notifications n ON n.host=h.host AND n.id=h.event WHERE h.host=? ORDER BY h.sequence DESC LIMIT 100",arrayOf(host)).use {rows->buildList {
                while(rows.moveToNext())add(Protocol.strictJson.decodeFromString<AttentionEvent>(rows.getString(0)).also {it.validate()})
            }}
            hints.forEach {event->
                val exact=scalar(db,"SELECT count(*) FROM push_hints WHERE host=? AND event=? AND sequence=? AND session=? AND category=?",host,event.eventId.value,event.sequence.value.toString(),event.sessionId.value,event.category.name)==1L
                if(!exact)throw StorePolicy(ErrorCode.InvalidRequest)
                val hinted=string(db,"SELECT disposition FROM notification_presentations WHERE host=? AND event=?",host,event.eventId.value) ?: throw StorePolicy(ErrorCode.InvalidRequest)
                val priorActive=scalar(db,"SELECT active FROM notification_presentations WHERE host=? AND event=?",host,event.eventId.value)==1L
                val disposition=if(hinted!="hint_posted")hinted else when {
                    !current.enabled||event.category==AttentionCategory.Completed&&!current.includeCompleted -> "suppressed_policy"
                    event.sequence.value<=floor||event.sequence.value<=scalar(db,"SELECT coalesce(max(through),0) FROM readwater WHERE host=? AND session=?",host,event.sessionId.value) -> "suppressed_read"
                    current.suppress(event) -> "suppressed_visible"
                    else -> "posted"
                }
                transaction {
                    if(hinted=="hint_posted")db.execSQL("UPDATE notification_presentations SET disposition=?,active=? WHERE host=? AND event=?",arrayOf<Any?>(disposition,if(disposition=="posted"||priorActive)1 else 0,host,event.eventId.value))
                    db.execSQL("DELETE FROM push_hints WHERE host=? AND event=? AND sequence=? AND session=? AND category=?",arrayOf<Any?>(host,event.eventId.value,event.sequence.value,event.sessionId.value,event.category.name))
                }
                if(hinted=="hint_posted") {
                    if(disposition=="posted") {
                        val outcome=try {sink.post(host,event)}catch(_:Exception){PresentationOutcome.AttemptedUnknown}
                        settlePresentation(db,host,event.eventId.value,disposition,priorActive,outcome)
                    }else if(priorActive)cancelPresentation(db,host,event.eventId.value,time,sink)
                }
            }
            if (!current.enabled) return@withLock
            val unread=db.rawQuery("SELECT n.event FROM notifications n LEFT JOIN readwater w ON w.host=n.host AND w.session=n.session WHERE n.host=? AND n.sequence>coalesce(w.through,0) AND n.sequence>? ORDER BY n.sequence DESC LIMIT 100",arrayOf(host,floor.toString())).use { rows->buildList {while(rows.moveToNext()) add(Protocol.strictJson.decodeFromString<AttentionEvent>(rows.getString(0)))} }
            unread.reversed().forEach { event ->
                event.validate()
                if(string(db,"SELECT disposition FROM notification_presentations WHERE host=? AND event=?",host,event.eventId.value)!=null)return@forEach
                val disposition=when {
                    event.category==AttentionCategory.Completed && !current.includeCompleted -> "suppressed_policy"
                    current.suppress(event) -> "suppressed_visible"
                    else -> "posted"
                }
                transaction {
                    db.execSQL("INSERT INTO notification_presentations(host,event,sequence,session,disposition,active,attempted_at) VALUES(?,?,?,?,?,?,?)",arrayOf<Any?>(host,event.eventId.value,event.sequence.value,event.sessionId.value,disposition,if(disposition=="posted")1 else 0,time))
                }
                if (disposition=="posted") {
                    // A blocked/unknown result consumes this decision; restoring health never replays it.
                    val outcome=try {sink.post(host,event)}catch(_:Exception){PresentationOutcome.AttemptedUnknown}
                    settlePresentation(db,host,event.eventId.value,disposition,false,outcome)
                }
            }
            } finally {
                collectingHistoryChanges=false
                try {
                    // A no-transaction policy/cancellation path still projects hard content TTL.
                    db.rawQuery("SELECT DISTINCT host,session FROM content WHERE expires<=?",arrayOf(time.toString())).use {rows->
                        while(rows.moveToNext())committed.add(rows.getString(0) to rows.getString(1))
                    }
                    mutable.value=load(db,time,committed)
                } finally {committed.clear();attempted.clear()}
            }
        }
    }
    /** Second bounded transaction under the same writer/mutex; active means possible owned presence. */
    private fun settlePresentation(db:SQLiteDatabase,host:String,event:String,posted:String,priorActive:Boolean,outcome:PresentationOutcome) {
        val disposition=when(outcome) {
            PresentationOutcome.AttemptedHealthy->posted
            PresentationOutcome.AttemptedUnknown->"attempted_unknown"
            PresentationOutcome.BlockedBeforeAttempt->"blocked_before_attempt"
        }
        val active=outcome!=PresentationOutcome.BlockedBeforeAttempt||priorActive
        db.beginTransaction()
        try {
            db.execSQL("UPDATE notification_presentations SET disposition=?,active=? WHERE host=? AND event=?",arrayOf<Any?>(disposition,if(active)1 else 0,host,event))
            db.setTransactionSuccessful()
        }finally{db.endTransaction()}
    }
    private fun cancelPresentation(db: SQLiteDatabase, host:String, event:String, time:Long, sink:PresentationSink) {
        try {
            if (sink.cancel(host,event)) {
                db.execSQL("UPDATE notification_presentations SET active=0 WHERE host=? AND event=?",arrayOf(host,event))
            }
        } catch (_: Exception) { enqueueCancel(db,host,event,time) }
    }
    private fun enqueueCancel(db:SQLiteDatabase,host:String,event:String,time:Long) {
        if (scalar(db,"SELECT count(*) FROM notification_cancel_pending")>=10000L && scalar(db,"SELECT count(*) FROM notification_cancel_pending WHERE host=? AND event=?",host,event)==0L) throw StorePolicy(ErrorCode.ResourceLimit)
        db.execSQL("INSERT OR IGNORE INTO notification_cancel_pending(host,event,attempted_at) VALUES(?,?,?)",arrayOf<Any?>(host,event,time))
    }
    private fun drainPendingCancels(db:SQLiteDatabase,time:Long,sink:PresentationSink) {
        db.rawQuery("SELECT host,event FROM notification_cancel_pending ORDER BY attempted_at LIMIT 100",null).use { rows ->
            while(rows.moveToNext()) { val host=rows.getString(0);val event=rows.getString(1)
                try { if(sink.cancel(host,event) || sink.noLongerPresent(host,event)) db.execSQL("DELETE FROM notification_cancel_pending WHERE host=? AND event=?",arrayOf(host,event)) else db.execSQL("UPDATE notification_cancel_pending SET attempts=attempts+1,attempted_at=? WHERE host=? AND event=?",arrayOf<Any?>(time,host,event)) }
                catch(_:Exception) {db.execSQL("UPDATE notification_cancel_pending SET attempts=attempts+1,attempted_at=? WHERE host=? AND event=?",arrayOf<Any?>(time,host,event))}
            }
        }
    }
    private fun addNotification(db: SQLiteDatabase, host: String, event: AttentionEvent, time: Long) {
        val data = Protocol.strictJson.encodeToString(event)
        val previous = string(db,"SELECT event FROM notifications WHERE host=? AND (sequence=? OR id=?)",host,event.sequence.value.toString(),event.eventId.value)
        if (previous != null && previous != data) throw StorePolicy(ErrorCode.InvalidRequest)
        if (previous == null && (event.sequence.value < scalar(db,"SELECT retained FROM notification_state WHERE host=?",host) || event.createdAt.value <= time-CACHE_AGE)) return
        db.execSQL("UPDATE readwater SET last_event=max(last_event,?) WHERE host=? AND session=?",arrayOf<Any?>(event.createdAt.value,host,event.sessionId.value))
        db.execSQL("INSERT OR IGNORE INTO notifications VALUES(?,?,?,?,?,?)",arrayOf<Any?>(host,event.sequence.value,event.eventId.value,data,event.sessionId.value,event.createdAt.value))
    }
    private fun ensureWatermark(db: SQLiteDatabase, host: String, session: Uuid, time: Long) {
        val exists = scalar(db,"SELECT count(*) FROM readwater WHERE host=? AND session=?",host,session.value) > 0
        if (!exists && scalar(db,"SELECT count(*) FROM readwater WHERE host=?",host) >= 512) throw StorePolicy(ErrorCode.ResourceLimit)
        db.execSQL("INSERT OR IGNORE INTO readwater VALUES(?,?,0,?)",arrayOf<Any?>(host,session.value,time))
    }
    /** Returns a request only for a session actually displayed, and only a sequence received
     * from authenticated sync/live state. Push hints never call addNotification or mark read. */
    suspend fun readRequest(host: String, bootId: Uuid, session: Uuid, through: Uint): NotificationsReadParams = write { db,_ ->
        requireHost(db,host)
        val viewed = scalar(db,"SELECT count(*) FROM sessions WHERE host=? AND session=? AND viewed=1",host,session.value) == 1L
        val received = scalar(db,"SELECT count(*) FROM notifications WHERE host=? AND session=? AND sequence=?",host,session.value,through.value.toString()) == 1L
        if (!viewed || !received) throw StorePolicy(ErrorCode.InvalidRequest)
        NotificationsReadParams(bootId,session,through)
    }
    suspend fun readConfirmed(host: String, read: ReadWatermark,admitted:()->Boolean={true}) = write { db,_ ->
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
        requireHost(db,host); read.validate()
        val received = scalar(db,"SELECT count(*) FROM notifications WHERE host=? AND session=? AND sequence=?",host,read.sessionId.value,read.throughSequence.value.toString()) == 1L
        val prior = scalar(db,"SELECT coalesce(max(through),0) FROM readwater WHERE host=? AND session=?",host,read.sessionId.value)
        val exists = scalar(db,"SELECT count(*) FROM readwater WHERE host=? AND session=?",host,read.sessionId.value)>0
        if (exists && read.throughSequence.value <= prior) return@write
        if (!received) throw StorePolicy(ErrorCode.InvalidRequest)
        cancelReadPresentations(db,host,read.sessionId.value,read.throughSequence.value,clock())
        val at = scalar(db,"SELECT coalesce(max(at),0) FROM notifications WHERE host=? AND session=?",host,read.sessionId.value)
        ensureWatermark(db,host,read.sessionId,at)
        db.execSQL("UPDATE readwater SET through=max(through,?) WHERE host=? AND session=?",arrayOf<Any?>(read.throughSequence.value,host,read.sessionId.value))
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
    }
    private fun cancelReadPresentations(db:SQLiteDatabase,host:String,session:String,through:Long,time:Long) {
        val active=db.rawQuery("SELECT event FROM notification_presentations WHERE host=? AND session=? AND sequence<=? AND active=1",arrayOf(host,session,through.toString())).use {rows->buildList {while(rows.moveToNext()) add(rows.getString(0))}}
        val sink=presentationSink ?: return
        active.forEach { event -> cancelPresentation(db,host,event,time,sink) }
    }
    private fun cancelActiveHost(db:SQLiteDatabase,host:String,time:Long) {
        val sink=presentationSink ?: return
        db.rawQuery("SELECT event FROM notification_presentations WHERE host=? AND active=1",arrayOf(host)).use {rows->while(rows.moveToNext()) cancelPresentation(db,host,rows.getString(0),time,sink)}
    }
    internal suspend fun encryptedRegistration(host:String,generation:Uuid,device:Uuid,expectedGeneration:Uuid?,encrypted:ByteArray,admitted:()->Boolean={true}) = write { db,time ->
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
        requirePushDevice(db,host,device);requireWire(encrypted.isNotEmpty()&&encrypted.size<=65536)
        if (scalar(db,"SELECT count(*) FROM retired_push WHERE host=? AND generation=?",host,generation.value) > 0) throw StorePolicy(ErrorCode.StaleRegistration)
        val prior = string(db,"SELECT generation FROM private_registration WHERE host=?",host)
        if(prior!=expectedGeneration?.value)throw StorePolicy(ErrorCode.StaleRegistration)
        if(prior != null && prior != generation.value) {
            db.execSQL("DELETE FROM retired_push WHERE host=? AND at<=?",arrayOf<Any?>(host,time-CACHE_AGE))
            if(scalar(db,"SELECT count(*) FROM retired_push WHERE host=?",host)>=10000) throw StorePolicy(ErrorCode.ResourceLimit)
            db.execSQL("INSERT OR IGNORE INTO retired_push VALUES(?,?,?)",arrayOf<Any?>(host,prior,time))
        }
        db.execSQL("INSERT OR REPLACE INTO private_registration VALUES(?,?,?)",arrayOf(host,generation.value,encrypted))
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
    }
    internal suspend fun ensurePushAlias(host:String,device:Uuid,alias:String):Boolean=write {db,_->
        requirePushDevice(db,host,device);requireWire(alias==SecretStore.hostAlias(host,"push"))
        val metadata=Protocol.strictJson.decodeFromString<HostMetadata>(string(db,"SELECT metadata FROM hosts WHERE host=?",host) ?: throw StorePolicy(ErrorCode.NotReady));val retained=alias in metadata.keyAliases
        if(!retained){if(metadata.keyAliases.size>=8)throw StorePolicy(ErrorCode.ResourceLimit);db.execSQL("UPDATE hosts SET metadata=? WHERE host=?",arrayOf(Protocol.strictJson.encodeToString(metadata.copy(keyAliases=metadata.keyAliases+alias)),host))};retained
    }
    /** Hint metadata is presentation-only: never inserts AttentionEvent/read/sync state. */
    internal suspend fun pushHint(host:String,hint:PushEventHint,enabled:()->Boolean,completed:()->Boolean,suppressed:(AttentionEvent)->Boolean)=withContext(writer){mutex.withLock {
        check(!closed);val db=helper.writableDatabase;requirePushDevice(db,host,hint.device);val time=clock()
        if(string(db,"SELECT generation FROM private_registration WHERE host=?",host)!=hint.endpointGeneration.value)throw StorePolicy(ErrorCode.StaleRegistration)
        val through=scalar(db,"SELECT coalesce(max(through),0) FROM readwater WHERE host=? AND session=?",host,hint.sessionId.value)
        val floor=maxOf(scalar(db,"SELECT retained FROM notification_state WHERE host=?",host)-1,scalar(db,"SELECT presentation_floor FROM notification_state WHERE host=?",host))
        if(hint.sequence.value<=through||hint.sequence.value<=floor||scalar(db,"SELECT count(*) FROM notification_presentations WHERE host=? AND (event=? OR sequence=?)",host,hint.eventId.value,hint.sequence.value.toString())>0)return@withLock
        val count=scalar(db,"SELECT count(*) FROM notification_presentations WHERE host=?",host);if(count>=10000)throw StorePolicy(ErrorCode.ResourceLimit)
        val authenticated=string(db,"SELECT event FROM notifications WHERE host=? AND id=?",host,hint.eventId.value)?.let {Protocol.strictJson.decodeFromString<AttentionEvent>(it)}
        if(authenticated!=null&&(authenticated.sequence!=hint.sequence||authenticated.sessionId!=hint.sessionId||authenticated.category!=hint.category))throw StorePolicy(ErrorCode.InvalidRequest)
        val disposition=if(!enabled()||hint.category==AttentionCategory.Completed&&!completed())"suppressed_policy"else if(authenticated!=null&&suppressed(authenticated))"suppressed_visible"else if(authenticated!=null)"posted"else"hint_posted"
        db.beginTransaction();try{
            if(authenticated==null)db.execSQL("INSERT INTO push_hints VALUES(?,?,?,?,?,?)",arrayOf<Any?>(host,hint.eventId.value,hint.sequence.value,hint.sessionId.value,hint.category.name,time))
            db.execSQL("INSERT INTO notification_presentations VALUES(?,?,?,?,?,?,?)",arrayOf<Any?>(host,hint.eventId.value,hint.sequence.value,hint.sessionId.value,disposition,if(disposition=="hint_posted"||disposition=="posted")1 else 0,time));db.setTransactionSuccessful()
        }finally{db.endTransaction()}
        if(disposition=="hint_posted"||disposition=="posted") {
            val sink=presentationSink
            val outcome=if(sink==null)PresentationOutcome.BlockedBeforeAttempt else try {
                if(disposition=="hint_posted")sink.postHint(host,hint)else sink.post(host,checkNotNull(authenticated))
            }catch(_:Exception){PresentationOutcome.AttemptedUnknown}
            settlePresentation(db,host,hint.eventId.value,disposition,false,outcome)
        }
    }}
    internal suspend fun retirePush(host:String,generation:Uuid,device:Uuid,admitted:()->Boolean) = write { db,time ->
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
        requirePushDevice(db,host,device)
        if(string(db,"SELECT generation FROM private_registration WHERE host=?",host)!=generation.value)throw StorePolicy(ErrorCode.StaleRegistration)
        db.execSQL("DELETE FROM retired_push WHERE host=? AND at<=?",arrayOf<Any?>(host,time - CACHE_AGE))
        if (scalar(db,"SELECT count(*) FROM retired_push WHERE host=? AND generation=?",host,generation.value) == 0L && scalar(db,"SELECT count(*) FROM retired_push WHERE host=?",host) >= 10000) throw StorePolicy(ErrorCode.ResourceLimit)
        db.execSQL("INSERT OR IGNORE INTO retired_push VALUES(?,?,?)",arrayOf<Any?>(host,generation.value,time))
        db.execSQL("DELETE FROM private_registration WHERE host=? AND generation=?",arrayOf(host,generation.value))
        if(!admitted())throw StorePolicy(ErrorCode.NotReady)
    }
    suspend fun encryptedRegistration(host: String): Pair<Uuid,ByteArray>? = write { db,_ ->
        requireHost(db,host)
        db.rawQuery("SELECT generation,ciphertext FROM private_registration WHERE host=?",arrayOf(host)).use { if(it.moveToFirst()) Uuid(it.getString(0)) to it.getBlob(1) else null }
    }
    suspend fun retiredPush(host: String, generation: Uuid): Boolean = write { db,_ -> requireHost(db,host); scalar(db,"SELECT count(*) FROM retired_push WHERE host=? AND generation=?",host,generation.value)>0 }
    /** All host rows cascade in one transaction. The exact app-owned key aliases are handed to3
     * as a durable obligation; this operation does not claim actual key erasure/server revoke. */
    suspend fun forget(host: String): KeyDeletion = write { db,_ ->
        // Retain OS cancellation obligations outside the host FK before the cascade can erase them.
        val time=clock()
        val sink=presentationSink
        db.rawQuery("SELECT event FROM notification_presentations WHERE host=? AND active=1",arrayOf(host)).use { rows ->
            while(rows.moveToNext()) {
                val event=rows.getString(0)
                enqueueCancel(db,host,event,time)
                try { if(sink?.cancel(host,event)==true) db.execSQL("DELETE FROM notification_cancel_pending WHERE host=? AND event=?",arrayOf(host,event)) } catch(_:Exception) {}
            }
        }
        val metadata = string(db,"SELECT metadata FROM hosts WHERE host=?",host)?.let { Protocol.strictJson.decodeFromString<HostMetadata>(it) }
        val pending = string(db,"SELECT aliases FROM key_deletion WHERE host=?",host)?.let { Protocol.strictJson.decodeFromString<List<String>>(it) }
        val forgottenAliases = metadata?.keyAliases ?: pending ?: emptyList()
        db.rawQuery("SELECT upload,record FROM attachment_local WHERE host=?",arrayOf(host)).use {cursor->while(cursor.moveToNext()){val record=Protocol.strictJson.decodeFromString<LocalAttachmentUpload>(cursor.getString(1));db.execSQL("UPDATE attachment_local SET record=? WHERE host=? AND upload=?",arrayOf(Protocol.strictJson.encodeToString(record.copy(forgetPending=true)),host,cursor.getString(0)))}}
        db.execSQL("DELETE FROM hosts WHERE host=?",arrayOf(host))
        val retainedAliases = db.rawQuery("SELECT metadata FROM hosts",null).use { rows ->
            buildSet { while(rows.moveToNext()) addAll(Protocol.strictJson.decodeFromString<HostMetadata>(rows.getString(0)).keyAliases) }
        }
        val aliases = forgottenAliases.filterNot { it in retainedAliases }
        db.execSQL("INSERT OR REPLACE INTO key_deletion VALUES(?,?)",arrayOf(host,Protocol.strictJson.encodeToString(aliases)))
        KeyDeletion(host,aliases)
    }
    /** Explicit unpaired-installation Forget uses the existing durable obligation table.
     * Crash recovery may finish this consented deletion; loss alone never creates the marker. */
    suspend fun forgetInstallation():KeyDeletion = write {db,_->
        if(scalar(db,"SELECT count(*) FROM hosts")!=0L) throw StorePolicy(ErrorCode.Busy)
        val aliases=listOf(SecretStore.ENDPOINT_ALIAS)
        db.execSQL("INSERT OR REPLACE INTO key_deletion VALUES(?,?)",arrayOf(INSTALLATION_DELETION_SCOPE,Protocol.strictJson.encodeToString(aliases)))
        KeyDeletion(INSTALLATION_DELETION_SCOPE,aliases)
    }
    suspend fun keysDeleted(obligation: KeyDeletion) = write { db,_ ->
        val expected = string(db,"SELECT aliases FROM key_deletion WHERE host=?",obligation.hostId)?.let { Protocol.strictJson.decodeFromString<List<String>>(it) } ?: return@write
        if (expected != obligation.aliases) throw StorePolicy(ErrorCode.InvalidRequest)
        db.execSQL("DELETE FROM key_deletion WHERE host=?",arrayOf(obligation.hostId))
    }
    internal suspend fun attachmentUploads(host:String?=null):List<LocalAttachmentUpload> = write {db,_->
        val result=db.rawQuery(if(host==null)"SELECT record FROM attachment_local"else"SELECT record FROM attachment_local WHERE host=?",host?.let {arrayOf(it)}).use {cursor->buildList {while(cursor.moveToNext())add(Protocol.strictJson.decodeFromString<LocalAttachmentUpload>(cursor.getString(0)))}}
        if(result.size>32)throw StorePolicy(ErrorCode.ResourceLimit);result.forEach {it.validateLocal()};result
    }
    internal suspend fun saveAttachment(record:LocalAttachmentUpload)=write {db,_->
        if(!record.forgetPending)requireHost(db,record.host)
        record.validateLocal()
        if(record.size !in 1..8L*1024*1024||record.committedBytes !in 0..record.size||!record.sha256.matches(Regex("[0-9a-f]{64}")))throw StorePolicy(ErrorCode.InvalidRequest)
        val old=string(db,"SELECT record FROM attachment_local WHERE host=? AND upload=?",record.host,record.uploadId.value)?.let {Protocol.strictJson.decodeFromString<LocalAttachmentUpload>(it)}
        if(old!=null&&(old.target!=record.target||old.source!=record.source||old.size!=record.size||old.sha256!=record.sha256||record.committedBytes<old.committedBytes||old.providerPath!=null&&record.providerPath!=old.providerPath||old.published&&!record.published||old.discarded&&!record.discarded||old.forgetPending&&!record.forgetPending||record.local!=null&&record.local!=old.local))throw StorePolicy(ErrorCode.InvalidRequest)
        if(old==null&&scalar(db,"SELECT COUNT(*) FROM attachment_local")>=32)throw StorePolicy(ErrorCode.ResourceLimit)
        db.execSQL("INSERT OR REPLACE INTO attachment_local(host,upload,record) VALUES(?,?,?)",arrayOf(record.host,record.uploadId.value,Protocol.strictJson.encodeToString(record)))
    }
    /** Retire only local descriptors; receipts and action latches remain untouched. */
    internal suspend fun retireAttachment(record:LocalAttachmentUpload):Boolean=write {db,_->
        if(record.local!=null||record.forgetPending)return@write false
        val current=string(db,"SELECT record FROM attachment_local WHERE host=? AND upload=?",record.host,record.uploadId.value)?.let {Protocol.strictJson.decodeFromString<LocalAttachmentUpload>(it)} ?: return@write false
        if(current!=record)return@write false
        var matched=false;var unresolved=false;var currentAction=record.actionId==null
        db.rawQuery("SELECT metadata,receipt,authoritative FROM actions WHERE host=?",arrayOf(record.host)).use {cursor->while(cursor.moveToNext()){
            val metadata=Protocol.strictJson.decodeFromString<ActionMetadata>(cursor.getString(0))
            if(metadata.attachmentUpload!=record.uploadId)continue
            matched=true;if(metadata.actionId==record.actionId)currentAction=true
            val receipt=if(cursor.isNull(1))null else Protocol.strictJson.decodeFromString<Receipt>(cursor.getString(1))
            if(metadata.target!=record.target||cursor.getInt(2)!=1||receipt?.state !in listOf(ReceiptState.Succeeded,ReceiptState.Failed))unresolved=true
        }}
        if(unresolved||!currentAction||!record.discarded&&(matched||record.actionId!=null)||record.discarded&&!matched)return@write false
        db.execSQL("DELETE FROM attachment_local WHERE host=? AND upload=?",arrayOf(record.host,record.uploadId.value));true
    }
    internal suspend fun removeAttachment(record:LocalAttachmentUpload)=write {db,_->
        if(record.local!=null)throw StorePolicy(ErrorCode.InvalidRequest)
        db.execSQL("DELETE FROM attachment_local WHERE host=? AND upload=?",arrayOf(record.host,record.uploadId.value))
    }
    private fun action(request: Request,time: Long): ActionMetadata {
        val (method,boot,target) = when(val body=request.body) {
            is RequestBody.SessionControlsApply->Triple(ActionMethod.SessionControlsApply,body.params.bootId,body.params.target)
            is RequestBody.SessionQueueUpdate->Triple(ActionMethod.SessionQueueUpdate,body.params.bootId,body.params.target)
            is RequestBody.SessionQueueDelete->Triple(ActionMethod.SessionQueueDelete,body.params.bootId,body.params.target)
            is RequestBody.SessionQueueReorder->Triple(ActionMethod.SessionQueueReorder,body.params.bootId,body.params.target)
            is RequestBody.SessionQueueStart->Triple(ActionMethod.SessionQueueStart,body.params.bootId,body.params.target)
            is RequestBody.PromptSubmit -> Triple(ActionMethod.PromptSubmit,body.params.bootId,body.params.target)
            is RequestBody.TurnCancel -> Triple(ActionMethod.TurnCancel,body.params.bootId,body.params.target)
            is RequestBody.RequestRespond -> Triple(ActionMethod.RequestRespond,body.params.bootId,body.params.target)
            is RequestBody.SessionCreate -> Triple(ActionMethod.SessionCreate,body.params.bootId,null)
            is RequestBody.TerminalCreate->Triple(ActionMethod.TerminalCreate,body.params.bootId,null)
            is RequestBody.ShellInput -> Triple(ActionMethod.ShellInput,body.params.bootId,null)
            is RequestBody.ShellResize->Triple(ActionMethod.ShellResize,body.params.bootId,null)
            is RequestBody.ShellScroll->Triple(ActionMethod.ShellScroll,body.params.bootId,null)
            is RequestBody.AttachmentsBegin->Triple(ActionMethod.AttachmentsBegin,body.params.bootId,body.params.target)
            is RequestBody.AttachmentsChunk->Triple(ActionMethod.AttachmentsChunk,body.params.bootId,body.params.target)
            is RequestBody.AttachmentsComplete->Triple(ActionMethod.AttachmentsComplete,body.params.bootId,body.params.target)
            is RequestBody.AttachmentsDiscard->Triple(ActionMethod.AttachmentsDiscard,body.params.bootId,body.params.target)
            is RequestBody.AgentRename -> Triple(ActionMethod.AgentRename,body.params.bootId,body.params.target)
            is RequestBody.WorkspaceRename -> Triple(ActionMethod.WorkspaceRename,body.params.bootId,body.params.target)
            else -> throw StorePolicy(ErrorCode.InvalidRequest)
        }
        return ActionMetadata(request.id,method,boot,target,Uint(time),
            sourceTarget=(request.body as? RequestBody.SessionCreate)?.params?.placement?.sourceTarget,
            shellTarget=when(val body=request.body){is RequestBody.ShellInput->body.params.target;is RequestBody.ShellResize->body.params.target;is RequestBody.ShellScroll->body.params.target;else->null},
            attachmentUpload=request.body.attachmentUpload())
    }
    private fun markPartial(db: SQLiteDatabase,host: String,session: String) { db.execSQL("UPDATE sessions SET partial=1 WHERE host=? AND session=?",arrayOf(host,session)) }
    private fun trim(db: SQLiteDatabase,time: Long) {
        db.rawQuery("SELECT host,session FROM records WHERE at<=? UNION SELECT host,session FROM content WHERE expires<=? OR at<=?",arrayOf((time-CACHE_AGE).toString(),time.toString(),(time-CACHE_AGE).toString())).use {rows->
            while(rows.moveToNext())historyChanged(rows.getString(0),rows.getString(1))
        }
        db.execSQL("DELETE FROM push_hints WHERE at<=?",arrayOf<Any?>(time-CACHE_AGE))
        db.execSQL("DELETE FROM recovery WHERE expires<=?",arrayOf(time))
        db.execSQL("DELETE FROM inventory WHERE at<=?",arrayOf(time-CACHE_AGE))
        db.execSQL("UPDATE sessions SET partial=1 WHERE EXISTS(SELECT 1 FROM records WHERE records.host=sessions.host AND records.session=sessions.session AND at<=?)",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM records WHERE at<=?",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM content WHERE expires<=? OR at<=?",arrayOf(time,time-CACHE_AGE))
        db.execSQL("UPDATE notification_state SET retained=max(retained,coalesce((SELECT max(sequence)+1 FROM notifications WHERE host=notification_state.host AND at<=?),1),coalesce((SELECT sequence+1 FROM notifications WHERE host=notification_state.host ORDER BY sequence DESC LIMIT 1 OFFSET 10000),1))",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM notifications WHERE at<=?",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM notifications WHERE rowid IN (SELECT rowid FROM notifications n WHERE sequence NOT IN (SELECT sequence FROM notifications WHERE host=n.host ORDER BY sequence DESC LIMIT 10000))")
        db.execSQL("DELETE FROM push_hints WHERE at<=?",arrayOf<Any?>(time-CACHE_AGE))
        // Floors preserve at-most-once admission after normal presentation-ledger retention.
        db.execSQL("UPDATE notification_state SET presentation_floor=max(presentation_floor,coalesce((SELECT max(sequence) FROM notification_presentations p WHERE p.host=notification_state.host AND p.active=0 AND p.attempted_at<=?),0),coalesce((SELECT sequence FROM notification_presentations p WHERE p.host=notification_state.host AND p.active=0 ORDER BY sequence DESC LIMIT 1 OFFSET 10000),0))",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM notification_presentations WHERE active=0 AND (attempted_at<=? OR rowid IN (SELECT rowid FROM notification_presentations p WHERE p.host=notification_presentations.host AND p.active=0 ORDER BY sequence DESC LIMIT -1 OFFSET 10000))",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM readwater WHERE last_event<=? AND NOT EXISTS(SELECT 1 FROM notifications WHERE notifications.host=readwater.host AND notifications.session=readwater.session)",arrayOf(time-CACHE_AGE))
        db.execSQL("DELETE FROM actions WHERE updated<=? AND receipt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM recovery WHERE recovery.host=actions.host AND recovery.action=actions.action) AND state IN ('Dispatched','Succeeded','Failed','Uncertain')",arrayOf(time-CACHE_AGE))
        while (scalar(db,"SELECT coalesce((SELECT sum(bytes) FROM records),0)+coalesce((SELECT sum(bytes) FROM content),0)+coalesce((SELECT sum(bytes) FROM inventory),0)") > CACHE_BYTES) {
            val unviewed = db.rawQuery("SELECT host,session FROM sessions WHERE viewed=0 AND (EXISTS(SELECT 1 FROM records WHERE records.host=sessions.host AND records.session=sessions.session) OR EXISTS(SELECT 1 FROM content WHERE content.host=sessions.host AND content.session=sessions.session)) ORDER BY used LIMIT 1",null).use { if(it.moveToFirst()) it.getString(0) to it.getString(1) else null }
            if (unviewed != null) {
                historyChanged(unviewed.first,unviewed.second)
                db.execSQL("DELETE FROM records WHERE host=? AND session=?",arrayOf(unviewed.first,unviewed.second)); db.execSQL("DELETE FROM content WHERE host=? AND session=?",arrayOf(unviewed.first,unviewed.second)); markPartial(db,unviewed.first,unviewed.second)
            } else if(scalar(db,"SELECT count(*) FROM inventory") > 0) {
                db.execSQL("DELETE FROM inventory WHERE rowid IN (SELECT rowid FROM inventory ORDER BY at LIMIT 1)")
            } else {
                val oldest = db.rawQuery("SELECT host,session,id FROM records ORDER BY at,ord LIMIT 1",null).use { if(it.moveToFirst()) Triple(it.getString(0),it.getString(1),it.getString(2)) else null }
                if (oldest != null) {historyChanged(oldest.first,oldest.second);db.execSQL("DELETE FROM records WHERE host=? AND session=? AND id=?",arrayOf(oldest.first,oldest.second,oldest.third));markPartial(db,oldest.first,oldest.second)}
                else {
                    db.rawQuery("SELECT host,session FROM content ORDER BY used LIMIT 1",null).use {rows->if(rows.moveToFirst())historyChanged(rows.getString(0),rows.getString(1))}
                    db.execSQL("DELETE FROM content WHERE rowid IN (SELECT rowid FROM content ORDER BY used LIMIT 1)")
                }
            }
        }
    }
    private fun load(db: SQLiteDatabase,time: Long,changed:Set<Pair<String,String>>? = null): StoreView {
        // The cancellation owner intentionally keeps its existing nontransactional OS boundary.
        val reload=changed ?: db.rawQuery("SELECT DISTINCT host,session FROM content WHERE expires<=?",arrayOf(time.toString())).use {rows->buildSet {
            while(rows.moveToNext())add(rows.getString(0) to rows.getString(1))
        }}
        val previous=mutable.value.cached.associateBy {it.hostId to it.summary.sessionId.value}
        val hosts = db.rawQuery("SELECT metadata FROM hosts ORDER BY host",null).use { c -> buildList { while(c.moveToNext()) add(Protocol.strictJson.decodeFromString<HostMetadata>(c.getString(0))) } }
        val cached = db.rawQuery("SELECT host,session,summary,partial FROM sessions ORDER BY used DESC",null).use { c -> buildList {
            while(c.moveToNext()) {
                // Old display-only capability keys are ignored, never restored as authority.
                val host=c.getString(0); val session=c.getString(1); val summary=Protocol.responseJson.decodeFromString<SessionSummary>(c.getString(2)).also {it.validate()}
                val key=host to session
                val retained=previous[key]
                val history=if(key !in reload && retained!=null && retained.summary.generation==summary.generation)retained.history else db.rawQuery("SELECT data FROM records WHERE host=? AND session=? ORDER BY ord",arrayOf(host,session)).use { rows -> buildList { while(rows.moveToNext()) {
                    val record = Protocol.strictJson.decodeFromString<HistoryRecord>(rows.getString(0))
                    fun offline(value: Content?): Content? = if(value is ContentRef && scalar(db,"SELECT count(*) FROM content WHERE host=? AND session=? AND generation=? AND handle=? AND expires>?",host,session,summary.generation.value,value.handle.value,time.toString()) == 0L) ContentUnavailable(ContentUnavailableReason.NotCached,value.byteLength) else value
                    add(record.copy(text=offline(record.text),original=offline(record.original),tool=record.tool?.let { it.copy(input=offline(it.input),output=offline(it.output)) }))
                } } }
                val partial=c.getInt(3)!=0 || !(retained!=null && retained.history===history && !retained.partial) &&
                    history.any {it.text is ContentUnavailable || it.original is ContentUnavailable || it.tool?.input is ContentUnavailable || it.tool?.output is ContentUnavailable}
                add(if(retained!=null && retained.summary==summary && retained.history===history && retained.partial==partial)retained else CachedSession(host,summary,history,partial=partial))
            }
        } }
        val recovery=db.rawQuery("SELECT r.host,a.metadata,r.text,r.expires,r.options FROM recovery r JOIN actions a ON a.host=r.host AND a.action=r.action ORDER BY r.at",null).use { c -> buildList { while(c.moveToNext()) add(RecoveryDraft(c.getString(0),Protocol.strictJson.decodeFromString(c.getString(1)),c.getString(2),Uint(c.getLong(3)),Protocol.strictJson.decodeFromString(c.getString(4)))) } }
        val deletion=db.rawQuery("SELECT host,aliases FROM key_deletion",null).use { c -> buildList { while(c.moveToNext()) add(KeyDeletion(c.getString(0),Protocol.strictJson.decodeFromString(c.getString(1)))) } }
        val inventory=db.rawQuery("SELECT host,payload,at FROM inventory ORDER BY at DESC",null).use { c -> buildList {
            while(c.moveToNext()) {
                val inventory=Protocol.responseJson.decodeFromString<HostSnapshotResult>(c.getString(1)).also { it.validate() }
                add(CachedInventory(c.getString(0),inventory,Uint(c.getLong(2))))
            }
        } }
        val snippets=db.rawQuery("SELECT id,host,title,body FROM snippets ORDER BY host,title,id",null).use { c -> buildList {
            while(c.moveToNext()) add(ComposerSnippet(Uuid(c.getString(0)),c.getString(1),c.getString(2),c.getString(3)))
        } }
        val routes=db.rawQuery("SELECT host,revision,direct,relay FROM host_routes ORDER BY host",null).use { c -> buildList {
            while(c.moveToNext()) {
                val direct=Protocol.strictJson.decodeFromString<List<String>>(c.getString(2))
                val relay=if(c.isNull(3)) null else c.getString(3)
                val canonical=HostRoutes.validate(direct,relay)
                check(c.getLong(1)>0 && canonical.first==direct && canonical.second==relay)
                add(SavedHostRoutes(c.getString(0),c.getLong(1),direct,relay))
            }
        } }
        val attachments=db.rawQuery("SELECT record FROM attachment_local ORDER BY host,upload",null).use {cursor->buildList {while(cursor.moveToNext())add(Protocol.strictJson.decodeFromString<LocalAttachmentUpload>(cursor.getString(0)))}}
        requireWire(attachments.size<=32);attachments.forEach {it.validateLocal()}
        return StoreView(hosts,cached,recovery,deletion,inventory,snippets,routes,attachments)
    }
    private fun scalar(db: SQLiteDatabase,sql: String,vararg args: String): Long = db.rawQuery(sql,args).use { if(it.moveToFirst() && !it.isNull(0)) it.getLong(0) else 0 }
    private fun string(db: SQLiteDatabase,sql: String,vararg args: String): String? = db.rawQuery(sql,args).use { if(it.moveToFirst() && !it.isNull(0)) it.getString(0) else null }
}

private fun shellFactsRetained(old:ShellCreationResources,next:ShellCreationResources):Boolean =
    (old.serverId==null || old.serverId==next.serverId) && (old.workspaceId==null || old.workspaceId==next.workspaceId) &&
    (old.tabId==null || old.tabId==next.tabId) && (old.paneId==null || old.paneId==next.paneId) &&
    (old.terminalId==null || old.terminalId==next.terminalId) && (old.createdTarget==null || old.createdTarget==next.createdTarget)
