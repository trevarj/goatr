// The event stream of `events.subscribe`, applied to the terminals it
// describes. A port of herdr-app/src/hc.rs `EventLines`.

package herdr.core

import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.longOrNull

internal val lenient = Json { ignoreUnknownKeys = true }

/** One 7501 record: the program's own word for what it's doing. */
@Serializable
data class Status(
    val state: String = "",
    val app: String? = null,
    val msg: String? = null,
    val title: String? = null,
    val kind: String? = null,
    val progress: Double? = null,
)

@Serializable
data class Foreground(
    val pid: Long = 0,
    val name: String = "",
    val argv: List<String> = emptyList(),
    val cwd: String = "",
)

/**
 * A terminal as the core describes it (`Terminal` in `rpc.discover`). The JSON
 * is kept whole so fields the phone doesn't read pass through; the ones it
 * does read are typed below. Records are replaced, never mutated: the model
 * compares them by identity.
 */
@Serializable(with = TerminalRecord.Serializer::class)
class TerminalRecord(val json: JsonObject) {
    val address: String get() = json.string("address") ?: ""
    val state: String get() = json.string("state") ?: ""
    val argv: List<String> get() = json.decode("argv") ?: emptyList()
    val cwd: String get() = json.string("cwd") ?: ""
    val labels: Map<String, String> get() = json.decode("labels") ?: emptyMap()
    val cols: Int get() = json.long("cols")?.toInt() ?: 0
    val rows: Int get() = json.long("rows")?.toInt() ?: 0
    val name: String? get() = json.string("name")
    val title: String? get() = json.string("title")
    val status: Status? get() = json.decode("status")
    val statusMs: Long? get() = json.long("status_ms")
    val records: Map<String, Status> get() = json.decode("records") ?: emptyMap()
    val foreground: Foreground? get() = json.decode("foreground")
    val parent: String? get() = json.string("parent")

    override fun equals(other: Any?) = other is TerminalRecord && other.json == json
    override fun hashCode() = json.hashCode()
    override fun toString() = json.toString()

    object Serializer : KSerializer<TerminalRecord> {
        override val descriptor: SerialDescriptor = JsonObject.serializer().descriptor
        override fun deserialize(decoder: Decoder) =
            TerminalRecord(decoder.decodeSerializableValue(JsonObject.serializer()))
        override fun serialize(encoder: Encoder, value: TerminalRecord) =
            encoder.encodeSerializableValue(JsonObject.serializer(), value.json)
    }
}

/** `events.subscribe`'s reply. */
@Serializable
data class Snapshot(
    val channel: Int? = null,
    val seq: Long = 0,
    val terminals: List<TerminalRecord> = emptyList(),
)

/** What a push changed. */
data class Pushed(
    /** A layout document changed, or events were missed: read them again. */
    val docs: Boolean = false,
    val terminals: Boolean = false,
    /** The core's 15 s heartbeat: a cheap moment to refresh what has no events (machines). */
    val heartbeat: Boolean = false,
)

class EventLines(snapshot: Snapshot? = null) {
    private var pending = EMPTY
    private var list = snapshot?.terminals?.toMutableList() ?: mutableListOf()

    var seq: Long = snapshot?.seq ?: 0
        private set

    /** Every terminal the stream describes, finished ones included, in its order. */
    val terminals: List<TerminalRecord> get() = list

    /** The live ones, as `terminal.list` puts them. */
    fun live(): List<TerminalRecord> = list.filter { it.state == "live" }

    fun push(bytes: ByteArray): Pushed {
        val buf = pending + bytes
        var docs = false
        var terminals = false
        var heartbeat = false
        var start = 0
        while (true) {
            val i = buf.indexOf(NL, start)
            if (i < 0) break
            val raw = buf.decodeToString(start, i)
            start = i + 1
            // A line we can't read is skipped, like the desktop's.
            val line = runCatching { lenient.parseToJsonElement(raw) as? JsonObject }.getOrNull()
                ?: continue
            val p = line["params"] as? JsonObject ?: JsonObject(emptyMap())
            val lineSeq = p.long("seq")
            when (line.string("method")) {
                "missed" -> {
                    list = p.decode<List<TerminalRecord>>("terminals")?.toMutableList()
                        ?: mutableListOf()
                    seq = lineSeq ?: seq
                    docs = true
                    terminals = true
                }
                "event" -> if (lineSeq == null || lineSeq > seq) {
                    seq = lineSeq ?: seq
                    if (p.string("type") == "doc") docs = true
                    else terminals = apply(p) || terminals
                }
                "heartbeat" -> heartbeat = true
            }
        }
        pending = buf.copyOfRange(start, buf.size)
        return Pushed(docs, terminals, heartbeat)
    }

    // One event's change to the terminal it names.
    private fun apply(p: JsonObject): Boolean {
        val kind = p.string("type") ?: ""
        val data = p["data"] as? JsonObject ?: JsonObject(emptyMap())
        val terminal = p.string("terminal")
        val at = list.indexOfFirst { it.address == terminal }
        val whole = when (kind) {
            "created", "restarted" -> data
            "labels" -> data["record"] as? JsonObject
            else -> null
        }
        if (whole != null) {
            val record = TerminalRecord(whole)
            if (at >= 0) list[at] = record else list += record
            return true
        }
        if (at < 0) return false
        if (kind == "removed") {
            list.removeAt(at)
            return true
        }
        val t = list[at].json.toMutableMap()
        fun set(vararg keys: String) = keys.forEach { t[it] = data[it] ?: JsonNull }
        when (kind) {
            "finished", "lost" -> {
                set("reason", "exit_code", "signal", "finished_ms")
                t["state"] = JsonPrimitive(if (kind == "lost") "live" else "finished")
                if (kind == "lost") t["pid"] = JsonNull
                t.remove("foreground")
            }
            // status_ms isn't in the desktop's list, but it says when the word changed.
            "status" -> {
                set("status", "records", "reported")
                data["status_ms"]?.let { t["status_ms"] = it }
            }
            "turn" -> set("turn")
            "title" -> set("title")
            "labels" -> set("labels")
            "keep" -> set("keep")
            "size" -> set("cols", "rows")
            "foreground" -> t["foreground"] = data
            else -> return false
        }
        list[at] = TerminalRecord(JsonObject(t))
        return true
    }
}

private fun ByteArray.indexOf(b: Byte, from: Int): Int {
    for (i in from until size) if (this[i] == b) return i
    return -1
}

internal fun JsonObject.string(key: String): String? =
    (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content

internal fun JsonObject.long(key: String): Long? = (this[key] as? JsonPrimitive)?.longOrNull

internal fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull

internal inline fun <reified T> JsonObject.decode(key: String): T? {
    val v: JsonElement = this[key]?.takeIf { it != JsonNull } ?: return null
    return runCatching { lenient.decodeFromJsonElement<T>(v) }.getOrNull()
}
