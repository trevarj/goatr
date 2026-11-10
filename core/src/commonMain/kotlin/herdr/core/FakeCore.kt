// A scripted core on the other end of an in-memory pipe, for tests (and for
// running the UI without a machine).

package herdr.core

import kotlinx.coroutines.CoroutineScope
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64

typealias Handler = (JsonObject) -> JsonElement

/** A handler's error reply: `{code, message, data: {kind, hint}}`. */
class Fault(val code: Int, override val message: String, val kind: String? = null, val hint: String? = null) :
    Exception(message) {
    fun toJson() = buildJsonObject {
        put("code", code)
        put("message", message)
        if (kind != null || hint != null) put("data", buildJsonObject {
            kind?.let { put("kind", it) }
            hint?.let { put("hint", it) }
        })
    }
}

fun fault(kind: String) = Fault(-32000, kind, kind)

val FAKE_HELLO = Hello(
    generation = 1,
    version = "0.1.15",
    core = CoreId("fake", "fake"),
    framed = true,
    methods = listOf(
        "hello", "terminal.run", "terminal.list", "terminal.get", "terminal.kill",
        "terminal.label", "terminal.keep", "terminal.attach", "terminal.put", "doc.get", "doc.set",
        "events.subscribe", "machine.list", "machine.exec", "templates.list",
    ),
    features = listOf("framed", "attach", "events", "status", "shell"),
)

/** `handlers`: method → result; a thrown [Fault] is the error. */
class FakeCore(
    scope: CoroutineScope,
    val handlers: MutableMap<String, Handler> = mutableMapOf(),
    hello: Hello = FAKE_HELLO,
) {
    private val ends = pipe(scope)
    private val core = ends.second

    /** The client's end. */
    val duplex: Duplex = ends.first

    /** Every frame the client sent after hello. */
    val received = mutableListOf<Frame>()

    /** Requests are received but never answered, as if the core hung. */
    var mute = false

    private val frames = FrameDecoder()
    private var framed = false
    private var head = ""

    init {
        core.onData { bytes ->
            if (!framed) {
                head += bytes.decodeToString()
                val nl = head.indexOf('\n')
                if (nl < 0) return@onData
                val req = compact.parseToJsonElement(head.substring(0, nl)) as JsonObject
                framed = true
                val reply = reply(req) { put("result", compact.encodeToJsonElement(Hello.serializer(), hello)) }
                core.write("$reply\n".encodeToByteArray())
                return@onData
            }
            for (f in frames.push(bytes)) {
                received += f
                if (f.channel != 0 || f.kind != Frame.DATA || mute) continue
                val req = compact.parseToJsonElement(f.payload.decodeToString()) as JsonObject
                val method = req.string("method") ?: ""
                val params = req["params"] as? JsonObject ?: JsonObject(emptyMap())
                val reply = reply(req) {
                    try {
                        val handler = handlers[method] ?: throw Fault(-32601, "method not found")
                        put("result", handler(params))
                    } catch (e: Fault) {
                        put("error", e.toJson())
                    }
                }
                send(0, Frame.DATA, reply.toString())
            }
        }
    }

    private fun reply(req: JsonObject, body: JsonObjectBuilder.() -> Unit) = buildJsonObject {
        put("jsonrpc", "2.0")
        put("id", req["id"] ?: JsonNull)
        body()
    }

    /** Sends a frame to the client. */
    fun send(channel: Int, kind: Int, payload: ByteArray = ByteArray(0)) =
        core.write(encode(channel, kind, payload))

    fun send(channel: Int, kind: Int, payload: String) = send(channel, kind, payload.encodeToByteArray())

    /** Ends the connection, as a dropped SSH session would. */
    fun drop() = core.close()
}

data class Stored(val value: JsonElement, val version: Long)

private fun JsonObject.on() = string("on") ?: ""

/**
 * A core with state: versioned `doc.get`/`doc.set` (conflict on a stale
 * version), `terminal.run`/`kill`/`list`, `templates.list` and `machine.exec`
 * (answers the agents check with `installed`).
 */
class FakeHerdr(
    docs: Map<String, Stored> = emptyMap(),
    private val templates: List<JsonObject> = emptyList(),
    private val installed: List<String> = emptyList(),
    live: List<JsonObject> = emptyList(),
) {
    /** Layout documents by machine ("" for this core), as written. */
    val docs = docs.toMutableMap()

    /** Every terminal started, live or killed. */
    val terminals = live.toMutableList()
    val runs = mutableListOf<JsonObject>()
    val sets = mutableListOf<JsonObject>()
    val kills = mutableListOf<String>()

    /** Called before each doc.set is checked: another screen writing first. */
    var beforeSet: ((machine: String) -> Unit)? = null

    /** Makes a method fail with this error kind (e.g. `doc.set` → `unavailable`). */
    val failing = mutableMapOf<String, String>()

    /** Writes a document as another screen would (bumps its version). */
    fun write(machine: String, value: JsonElement) {
        docs[machine] = Stored(value, (docs[machine]?.version ?: 0) + 1)
    }

    private var next = 100

    private fun guard(method: String) {
        failing[method]?.let { throw fault(it) }
    }

    private fun stored(s: Stored?) = buildJsonObject {
        put("value", s?.value ?: JsonNull)
        put("version", s?.version ?: 0)
    }

    /** Handlers for [FakeCore]: documents, terminals, templates, exec. */
    val handlers: MutableMap<String, Handler> = mutableMapOf(
        "doc.get" to { p ->
            guard("doc.get")
            stored(this.docs[p.on()])
        },
        "doc.set" to { p ->
            guard("doc.set")
            beforeSet?.invoke(p.on())
            val cur = this.docs[p.on()]?.version ?: 0
            if (p.long("version") != cur) throw fault("conflict")
            sets += p
            this.docs[p.on()] = Stored(p["value"] ?: JsonNull, cur + 1)
            buildJsonObject { put("version", cur + 1) }
        },
        "terminal.run" to { p ->
            guard("terminal.run")
            runs += p
            val m = p.on()
            val n = next++
            val record = buildJsonObject {
                put("address", if (m.isNotEmpty()) "$m/t$n" else "t$n")
                put("state", "live")
                put("argv", p["argv"] ?: JsonArray(emptyList()))
                put("cwd", p["cwd"]?.takeIf { it != JsonNull } ?: JsonPrimitive(if (m.isNotEmpty()) "/home/$m" else "/home/fake"))
                put("labels", p["labels"] ?: JsonObject(emptyMap()))
                put("keep", p.bool("keep") == true)
                put("parent", JsonNull)
                put("created_ms", 0)
                put("cols", 80)
                put("rows", 24)
            }
            terminals += record
            record
        },
        "terminal.kill" to { p ->
            guard("terminal.kill")
            val at = terminals.indexOfFirst { it.string("address") == p.string("terminal") }
            if (at < 0) throw fault("not_found")
            val t = JsonObject(terminals[at] + ("state" to JsonPrimitive("finished")))
            terminals[at] = t
            kills += t.string("address")!!
            t
        },
        "terminal.list" to { _ ->
            buildJsonObject { put("terminals", JsonArray(terminals.filter { it.string("state") == "live" })) }
        },
        "templates.list" to { _ -> buildJsonObject { put("templates", JsonArray(templates)) } },
        "machine.exec" to { p ->
            val argv = (p["argv"] as? JsonArray)?.map { it.jsonPrimitive.content } ?: emptyList()
            val found = argv.drop(4).filter { it in installed }
            val stdout = Base64.encode(found.joinToString("") { "herdr-has:$it\n" }.encodeToByteArray())
            buildJsonObject {
                put("code", 0)
                put("stdout", stdout)
                put("stderr", "")
                put("truncated", false)
                put("timed_out", false)
                put("ms", 1)
            }
        },
    )
}
