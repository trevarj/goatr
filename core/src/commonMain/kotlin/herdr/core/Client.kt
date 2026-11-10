// One framed connection to herdr-core over a Duplex: `hello`, JSON-RPC on
// channel 0, and streams (attach, events) on their own channels with credit.
// A port of herdr-app/src/hc.rs `Core`. All state lives on one serial
// dispatcher (`loop`); public methods are safe from any thread.

package herdr.core

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.put
import kotlin.concurrent.Volatile

/** The supported core API generation; a newer one needs an app update. */
const val GENERATION = 1

/** The oldest core the phone works with (herdr-app distribution/core-version). */
const val CORE_MIN = "0.1.11"

/**
 * What the phone can't work without. Everything else (terminal.run, kill,
 * label, keep, doc.set...) is gated with `client.has(method)`.
 */
val USES = listOf(
    "terminal.list",
    "terminal.get",
    "terminal.attach",
    "events.subscribe",
    "machine.list",
    "doc.get",
)

@Serializable
data class CoreId(val id: String = "", val name: String = "")

@Serializable
data class Hello(
    val generation: Int = 0,
    val version: String = "0.0.0",
    val core: CoreId = CoreId(),
    val framed: Boolean = false,
    val methods: List<String> = emptyList(),
    val features: List<String> = emptyList(),
)

/** Why a core can't serve the phone. */
sealed interface Mismatch {
    data class TooOld(val have: String, val need: String) : Mismatch
    data class Newer(val generation: Int) : Mismatch
    data class Missing(val methods: List<String>) : Mismatch
}

private fun semver(v: String): List<Int> {
    val p = v.trim().removePrefix("hc ").split('.', '-')
    return (0..2).map { p.getOrNull(it)?.toIntOrNull() ?: 0 }
}

private fun older(a: String, b: String): Boolean {
    val x = semver(a)
    val y = semver(b)
    val i = x.indices.firstOrNull { x[it] != y[it] } ?: return false
    return x[i] < y[i]
}

/** Whether a core's hello says it can serve the phone (hc.rs `check`). */
fun check(hello: Hello): Mismatch? {
    if (hello.generation > GENERATION) return Mismatch.Newer(hello.generation)
    if (hello.generation < GENERATION || older(hello.version, CORE_MIN)) {
        return Mismatch.TooOld(hello.version, CORE_MIN)
    }
    val missing = USES.filter { it !in hello.methods }
    return if (missing.isEmpty()) null else Mismatch.Missing(missing)
}

/** A JSON-RPC error, or `kind` "lost" / "timeout" for the connection. */
class RpcError(
    val method: String,
    reason: String?,
    val kind: String? = null,
    hint: String? = null,
    val data: JsonElement? = null,
) : Exception("$method: ${reason ?: "core call failed"}${hint?.let { " ($it)" } ?: ""}") {
    companion object {
        internal fun from(method: String, error: JsonElement?): RpcError {
            val e = error as? JsonObject ?: JsonObject(emptyMap())
            val data = e["data"] as? JsonObject
            return RpcError(method, e.string("message"), data?.string("kind"), data?.string("hint"), data)
        }
    }
}

/** The core can't serve the phone; the message says what to update (setup.rs). */
class MismatchError(val mismatch: Mismatch, val hello: Hello) : Exception(
    when (mismatch) {
        is Mismatch.TooOld ->
            "This herdr-core is ${mismatch.have}; Goatr needs ${mismatch.need} or newer. Update it with hc update; terminals keep running."
        is Mismatch.Missing ->
            "This herdr-core can't do everything Goatr needs (${mismatch.methods.joinToString()}). Update it with hc update; terminals keep running."
        is Mismatch.Newer ->
            "This herdr-core speaks a newer protocol (generation ${mismatch.generation}) than this Goatr understands. Update Goatr."
    },
)

/**
 * How a stream ended: the core closed it (the payload says why, e.g. the
 * program exited), or the connection dropped. After [Lost] the program is
 * most likely still running: reattach (with `history`) on a new connection.
 */
sealed interface End {
    /** The core closed it: the program exited, was killed, or another [reason] (`detached`). */
    data class Closed(val exitCode: Long? = null, val signal: Long? = null, val reason: String? = null) : End {
        /** What the terminal shows. */
        val summary: String
            get() = when {
                exitCode != null -> "exited $exitCode"
                signal != null -> "killed by signal $signal"
                else -> reason ?: "ended"
            }

        internal companion object {
            fun of(why: JsonElement?): Closed {
                val w = why as? JsonObject ?: return Closed()
                return Closed(w.long("exit_code"), w.long("signal"), w.string("reason"))
            }
        }
    }
    data class Lost(val reason: String? = null) : End
}

@Serializable
data class Display(
    val bg: String? = null,
    val fg: String? = null,
    val cursor: String? = null,
    val cell: List<Int>? = null,
)

data class AttachOptions(
    val cols: Int,
    val rows: Int,
    /** Scrollback lines to send before the screen (main screen only). */
    val history: Int = 0,
    /** Watch only: the core never takes input or a size from this viewer. */
    val observe: Boolean? = null,
    /** Back after a dropped line: the size applies only once active again. */
    val resumed: Boolean? = null,
    val display: Display? = null,
    /** Extra credit granted at once. The core starts every attach with 256 KiB. */
    val window: Int = 0,
    /** Grant credit as soon as bytes are delivered (no backpressure). */
    val autoCredit: Boolean = false,
)

enum class ConnectionState { Connected, Lost }

@PublishedApi
internal val compact = Json { explicitNulls = false; ignoreUnknownKeys = true }

/**
 * One stream channel. Listeners run on the client's dispatcher; data that
 * arrived before [onData] is delivered first, and the end comes after it.
 */
open class Stream internal constructor(
    internal val client: Client,
    val channel: Int,
    private val autoCredit: Boolean,
) {
    private var dataListener: ((ByteArray) -> Unit)? = null
    private var endListener: ((End) -> Unit)? = null
    private val queue = ArrayDeque<ByteArray>()
    private var told = false

    @Volatile
    var ended: End? = null
        private set

    fun onData(listener: (ByteArray) -> Unit) = client.post {
        dataListener = listener
        while (queue.isNotEmpty()) deliver(queue.removeFirst())
        tellEnd()
    }

    fun onEnd(listener: (End) -> Unit) = client.post {
        endListener = listener
        tellEnd()
    }

    /**
     * The consumer has handled `n` bytes: the core may send that many more.
     * Not calling this is the backpressure (the core stops at the window).
     */
    fun consumed(n: Int) = client.post { credit(n) }

    fun close() = client.post {
        if (ended != null) return@post
        client.routes.remove(channel)
        client.send(channel, Frame.CLOSE)
        finish(End.Closed(reason = "detached"))
    }

    internal fun credit(n: Int) {
        if (n > 0 && ended == null) client.send(channel, Frame.CREDIT, u32(n))
    }

    internal fun data(bytes: ByteArray) {
        if (ended != null) return
        if (dataListener != null) deliver(bytes) else queue.addLast(bytes)
    }

    internal fun finish(end: End) {
        if (ended != null) return
        ended = end
        tellEnd()
    }

    private fun deliver(bytes: ByteArray) {
        dataListener?.invoke(bytes)
        if (autoCredit) credit(bytes.size)
    }

    // Once, after the data that came before it.
    private fun tellEnd() {
        val end = ended ?: return
        val l = endListener ?: return
        if (told || queue.isNotEmpty()) return
        told = true
        l(end)
    }
}

class Attached internal constructor(
    client: Client,
    channel: Int,
    autoCredit: Boolean,
    val terminal: TerminalRecord,
) : Stream(client, channel, autoCredit) {
    private fun control(kind: Int, payload: ByteArray) = client.post {
        if (ended == null) client.send(channel, kind, payload)
    }

    fun write(input: ByteArray) {
        if (input.isNotEmpty()) control(Frame.DATA, input.copyOf())
    }

    fun write(input: String) = write(input.encodeToByteArray())
    fun resize(cols: Int, rows: Int) = control(Frame.RESIZE, size(cols, rows))
    fun focus(focused: Boolean) = control(Frame.FOCUS, byteArrayOf(if (focused) 1 else 0))

    /** This viewer's display facts changed (colors, cell size). */
    fun facts(display: Display) =
        control(Frame.FACTS, compact.encodeToString(Display.serializer(), display).encodeToByteArray())
}

/**
 * An event subscription, applied as it arrives. [onChange] sees what each
 * batch of lines changed (on the client's dispatcher); [lines] holds the terminals.
 */
class Subscription internal constructor(val lines: EventLines, val stream: Stream) {
    @Volatile
    internal var change: ((Pushed) -> Unit)? = null

    fun onChange(listener: (Pushed) -> Unit) {
        change = listener
    }

    fun onEnd(listener: (End) -> Unit) = stream.onEnd(listener)
    fun close() = stream.close()
}

private class Pending(
    val method: String,
    val reply: CompletableDeferred<JsonElement>,
    // Streams are opened while handling the reply, before the next frame is read.
    val opens: ((JsonElement) -> Unit)?,
)

private fun rpc(id: Long, method: String, params: JsonObject) = buildJsonObject {
    put("jsonrpc", "2.0")
    put("id", id)
    put("method", method)
    put("params", params)
}.toString()

private val HELLO = (rpc(0, "hello", buildJsonObject {
    put("framed", true)
    put("generation", GENERATION)
}) + "\n").encodeToByteArray()

/**
 * Says `hello` on a fresh duplex and switches it to framed mode. Throws if
 * the line drops first, the core refuses, or (with [strict]) [check] finds a
 * mismatch. [dispatcher] must run one task at a time, in order.
 */
suspend fun connect(
    duplex: Duplex,
    timeoutMs: Long = 10_000,
    strict: Boolean = true,
    dispatcher: CoroutineDispatcher = Dispatchers.Default.limitedParallelism(1),
): Client = Client(duplex, dispatcher, timeoutMs).start(strict)

class Client internal constructor(
    private val duplex: Duplex,
    private val loop: CoroutineDispatcher,
    private val timeoutMs: Long,
) {
    private val scope = CoroutineScope(loop + SupervisorJob())

    lateinit var hello: Hello
        private set

    private val _state = MutableStateFlow(ConnectionState.Connected)
    val state: StateFlow<ConnectionState> = _state

    /** Why the connection ended, once lost. */
    @Volatile
    var lostReason: String? = null
        private set

    private val pending = mutableMapOf<Long, Pending>()
    internal val routes = mutableMapOf<Int, Stream>()

    // Frames for a channel whose reply hasn't been handled yet: ByteArray or End.
    private val early = mutableMapOf<Int, MutableList<Any>>()
    private val frames = FrameDecoder()
    private var nextId = 1L
    private var head = EMPTY
    private val greeted = CompletableDeferred<Hello>()

    fun has(method: String) = method in hello.methods

    internal fun post(block: () -> Unit) {
        scope.launch { block() }
    }

    internal fun send(channel: Int, kind: Int, payload: ByteArray = EMPTY) {
        if (_state.value == ConnectionState.Connected) duplex.write(encode(channel, kind, payload))
    }

    internal suspend fun start(strict: Boolean): Client {
        withContext(loop) {
            duplex.onClose { reason ->
                post {
                    greeted.completeExceptionally(RpcError("hello", reason ?: "connection closed", "lost"))
                    lose(reason)
                }
            }
            duplex.onData { bytes -> post { onBytes(bytes, strict) } }
            duplex.write(HELLO)
        }
        val got = withTimeoutOrNull(timeoutMs) { greeted.await() }
        if (got == null) {
            duplex.close()
            throw RpcError("hello", "no hello from core", "timeout")
        }
        return this
    }

    private fun onBytes(input: ByteArray, strict: Boolean) {
        var bytes = input
        if (!greeted.isCompleted) {
            // Before framing: one JSON line, the hello reply.
            head += bytes
            val nl = head.indexOf(NL)
            if (nl < 0) return
            val line = head.decodeToString(0, nl)
            bytes = head.copyOfRange(nl + 1, head.size)
            head = EMPTY
            val got = runCatching { readHello(line, strict) }.getOrElse {
                greeted.completeExceptionally(it)
                duplex.close()
                return
            }
            hello = got
            greeted.complete(got)
            if (bytes.isEmpty()) return
        }
        for (f in frames.push(bytes)) onFrame(f)
    }

    // The hello reply, or why the core is refused (thrown).
    private fun readHello(line: String, strict: Boolean): Hello {
        fun bad() = RpcError("hello", "bad hello reply: ${line.take(200)}")
        val reply = runCatching { compact.parseToJsonElement(line) as JsonObject }.getOrNull() ?: throw bad()
        val result = reply["result"]
        if (reply["error"] != null || result == null || result == JsonNull) {
            throw RpcError.from("hello", reply["error"] ?: buildJsonObject { put("message", "empty hello reply") })
        }
        val got = runCatching { compact.decodeFromJsonElement<Hello>(result) }.getOrNull() ?: throw bad()
        if (strict) check(got)?.let { throw MismatchError(it, got) }
        return got
    }

    private fun onFrame(f: Frame) {
        if (f.channel == 0) {
            val msg = runCatching { compact.parseToJsonElement(f.payload.decodeToString()) as JsonObject }
                .getOrNull() ?: return
            val id = msg.long("id") ?: return
            val p = pending.remove(id) ?: return
            val error = msg["error"]
            if (error != null && error != JsonNull) {
                p.reply.completeExceptionally(RpcError.from(p.method, error))
            } else {
                val result = msg["result"] ?: JsonNull
                p.opens?.invoke(result)
                p.reply.complete(result)
            }
            return
        }
        val item: Any = when (f.kind) {
            Frame.DATA -> f.payload
            Frame.CLOSE -> End.Closed.of(parse(f.payload))
            else -> return
        }
        val route = routes[f.channel]
        if (route == null) early.getOrPut(f.channel) { mutableListOf() } += item
        else take(route, item)
    }

    private fun take(stream: Stream, item: Any) {
        if (item is ByteArray) stream.data(item)
        else {
            routes.remove(stream.channel)
            stream.finish(item as End)
        }
    }

    private fun parse(payload: ByteArray): JsonElement? =
        if (payload.isEmpty()) null
        else runCatching { compact.parseToJsonElement(payload.decodeToString()) }.getOrNull()

    private fun lose(reason: String?) {
        if (_state.value == ConnectionState.Lost) return
        // Fail calls still waiting, end every stream: the UI shows it, nothing throws.
        lostReason = reason
        _state.value = ConnectionState.Lost
        for (p in pending.values) p.reply.completeExceptionally(lostError(p.method))
        pending.clear()
        val end = End.Lost(reason)
        for (r in routes.values) r.finish(end)
        routes.clear()
        early.clear()
    }

    private fun lostError(method: String) = RpcError(method, "lost the core", "lost")

    // Opens the route for a stream channel; frames that came early go first.
    private fun <S : Stream> route(stream: S): S {
        routes[stream.channel] = stream
        early.remove(stream.channel)?.forEach { take(stream, it) }
        return stream
    }

    suspend fun call(
        method: String,
        params: JsonObject = JsonObject(emptyMap()),
        timeoutMs: Long = this.timeoutMs,
    ): JsonElement = request(method, params, timeoutMs, null)

    private suspend fun request(
        method: String,
        params: JsonObject,
        timeoutMs: Long,
        opens: ((JsonElement) -> Unit)?,
    ): JsonElement {
        val reply = CompletableDeferred<JsonElement>()
        val id = withContext(loop) {
            if (_state.value == ConnectionState.Lost) throw lostError(method)
            val id = nextId++
            pending[id] = Pending(method, reply, opens)
            send(0, Frame.DATA, rpc(id, method, params).encodeToByteArray())
            id
        }
        return withTimeoutOrNull(timeoutMs) { reply.await() } ?: run {
            withContext(loop) { pending.remove(id) }
            throw RpcError(method, "no reply from core", "timeout")
        }
    }

    // A call whose reply names a stream channel: the stream is routed before the next frame is read.
    private suspend fun <S : Stream> open(
        method: String,
        params: JsonObject,
        stream: (channel: Int, reply: JsonObject) -> S,
    ): Pair<S, JsonElement> {
        var opened: S? = null
        val reply = request(method, params, timeoutMs) { r ->
            val o = r as? JsonObject
            o?.long("channel")?.let { opened = route(stream(it.toInt(), o)) }
        }
        return (opened ?: throw RpcError(method, "no channel in reply")) to reply
    }

    suspend fun subscribe(
        params: JsonObject = JsonObject(emptyMap()),
        window: Int = 1024 * 1024,
    ): Subscription {
        val (s, reply) = open("events.subscribe", params) { channel, _ -> Stream(this, channel, autoCredit = false) }
        val snapshot = compact.decodeFromJsonElement<Snapshot>(reply)
        val sub = Subscription(EventLines(snapshot), s)
        s.onData { bytes ->
            val pushed = sub.lines.push(bytes)
            // Parsed means consumed: the model is cheap, the stream is small.
            s.credit(bytes.size)
            if (pushed.docs || pushed.terminals || pushed.heartbeat) sub.change?.invoke(pushed)
        }
        s.consumed(window)
        return sub
    }

    suspend fun attach(address: String, o: AttachOptions): Attached {
        val params = buildJsonObject {
            put("terminal", address)
            put("history", o.history)
            put("cols", o.cols)
            put("rows", o.rows)
            o.observe?.let { put("observe", it) }
            o.resumed?.let { put("resumed", it) }
            o.display?.let { put("display", compact.encodeToJsonElement(it)) }
        }
        val (s, _) = open("terminal.attach", params) { channel, reply ->
            val terminal = TerminalRecord(reply["terminal"] as? JsonObject ?: JsonObject(emptyMap()))
            Attached(this, channel, o.autoCredit, terminal)
        }
        if (o.window > 0) s.consumed(o.window)
        return s
    }

    fun close() = duplex.close()
}

/** [Client.call], its result decoded leniently. */
suspend inline fun <reified T> Client.callAs(
    method: String,
    params: JsonObject = JsonObject(emptyMap()),
): T = compact.decodeFromJsonElement(call(method, params))
