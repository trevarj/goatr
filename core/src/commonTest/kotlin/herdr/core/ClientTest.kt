package herdr.core

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue

@OptIn(ExperimentalCoroutinesApi::class)
class ClientTest {
    private fun hello(generation: Int, version: String, methods: List<String>) =
        Hello(generation = generation, version = version, methods = methods)

    private fun TestScope.fake(hello: Hello = FAKE_HELLO, handlers: Map<String, Handler> = emptyMap()) =
        FakeCore(this, handlers.toMutableMap(), hello)

    private suspend fun TestScope.open(core: FakeCore, strict: Boolean = true) =
        connect(core.duplex, strict = strict, dispatcher = StandardTestDispatcher(testScheduler))

    private fun FakeCore.on(channel: Int, kind: Int) = received.filter { it.channel == channel && it.kind == kind }
    private fun credits(core: FakeCore, channel: Int) = core.on(channel, Frame.CREDIT).map { it.payload.getU32(0) }

    // hc.rs: a_core_serves_the_app_by_generation_version_and_methods
    @Test
    fun aCoreServesThePhoneByGenerationVersionAndMethods() {
        assertNull(check(hello(1, "9.9.9", USES)))
        assertEquals(Mismatch.TooOld("0.0.1", CORE_MIN), check(hello(1, "0.0.1", USES)))
        assertEquals(Mismatch.Newer(2), check(hello(2, "9.9.9", USES)))
        assertEquals(Mismatch.Missing(USES.drop(1)), check(hello(1, "9.9.9", listOf("terminal.list"))))
        // 0.10 is newer than 0.9 (numbers, not text).
        assertNull(check(hello(1, "0.10.0", USES)))
    }

    @Test
    fun helloCallsAndErrors() = runTest {
        val core = fake(handlers = mapOf(
            "terminal.get" to { p -> buildJsonObject { put("address", p["terminal"]!!) } },
            "terminal.kill" to { _ -> throw Fault(-32000, "not_found", "not_found", "no t9") },
        ))
        val c = open(core)
        assertEquals("fake", c.hello.core.name)
        assertTrue(c.has("doc.set") && !c.has("terminal.wait"))
        val a = async { c.call("terminal.get", buildJsonObject { put("terminal", "t1") }) }
        val b = async { c.call("terminal.get", buildJsonObject { put("terminal", "t2") }) }
        assertEquals(listOf("t1", "t2"), listOf(a.await(), b.await()).map { it.jsonObject.string("address") })
        val e = assertFailsWith<RpcError> { c.call("terminal.kill", buildJsonObject { put("terminal", "t9") }) }
        assertEquals("not_found", e.kind)
        assertEquals("terminal.kill: not_found (no t9)", e.message)
        assertFailsWith<RpcError> { c.call("nope") }
    }

    @Test
    fun aMismatchedCoreIsRefused() = runTest {
        val e = assertFailsWith<MismatchError> { open(fake(FAKE_HELLO.copy(generation = 2))) }
        assertIs<Mismatch.Newer>(e.mismatch)
        val lax = open(fake(FAKE_HELLO.copy(generation = 2)), strict = false)
        assertEquals(2, lax.hello.generation)
    }

    @Test
    fun attachEarlyBytesCreditOnlyAsConsumedControlsClose() = runTest {
        lateinit var core: FakeCore
        core = fake(handlers = mapOf("terminal.attach" to { p ->
            // History may come before the reply is handled.
            core.send(5, Frame.DATA, "early ")
            buildJsonObject {
                put("channel", 5)
                put("terminal", buildJsonObject {
                    put("address", p["terminal"]!!)
                    put("cols", p["cols"]!!)
                    put("rows", p["rows"]!!)
                })
            }.also { core.send(5, Frame.DATA, "later") }
        }))
        val c = open(core)
        val t = c.attach("t1", AttachOptions(cols = 80, rows = 24, history = 1000, window = 4096))
        assertEquals("t1", t.terminal.address)
        val got = mutableListOf<String>()
        t.onData { got += it.decodeToString() }
        advanceUntilIdle()
        assertEquals(listOf("early ", "later"), got)
        // The window on open, nothing more until the consumer says so.
        assertEquals(listOf(4096), credits(core, 5))
        t.consumed(11)
        t.write("ls\r")
        t.resize(100, 30)
        t.focus(true)
        advanceUntilIdle()
        assertEquals(listOf(4096, 11), credits(core, 5))
        assertEquals("ls\r", core.on(5, Frame.DATA).single().payload.decodeToString())
        assertEquals(listOf<Byte>(100, 0, 30, 0), core.on(5, Frame.RESIZE).single().payload.toList())
        assertEquals(listOf<Byte>(1), core.on(5, Frame.FOCUS).single().payload.toList())
        val req = compact.parseToJsonElement(core.received[0].payload.decodeToString()).jsonObject
        assertEquals(
            buildJsonObject {
                put("terminal", "t1")
                put("history", 1000)
                put("cols", 80)
                put("rows", 24)
            },
            req["params"],
        )
        val ends = mutableListOf<End>()
        t.onEnd { ends += it }
        t.close()
        advanceUntilIdle()
        assertTrue(core.on(5, Frame.CLOSE).isNotEmpty())
        assertEquals(1, ends.size)
    }

    @Test
    fun theCoreEndingAStreamSaysWhy() = runTest {
        val core = fake(handlers = mapOf("terminal.attach" to { _ ->
            buildJsonObject {
                put("channel", 2)
                put("terminal", JsonObject(emptyMap()))
            }
        }))
        val c = open(core)
        val t = c.attach("t1", AttachOptions(80, 24))
        val ends = mutableListOf<End>()
        t.onEnd { ends += it }
        core.send(2, Frame.CLOSE, """{"reason":"exited","exit_code":3}""")
        advanceUntilIdle()
        assertEquals(listOf<End>(End.Closed(exitCode = 3, reason = "exited")), ends)
        assertEquals("exited 3", (ends[0] as End.Closed).summary)
    }

    @Test
    fun aDroppedConnectionIsAStateCallsFailFastStreamsEndLost() = runTest {
        val core = fake(handlers = mapOf("terminal.attach" to { _ ->
            buildJsonObject {
                put("channel", 3)
                put("terminal", JsonObject(emptyMap()))
            }
        }))
        val c = open(core)
        val t = c.attach("t1", AttachOptions(80, 24))
        val ends = mutableListOf<End>()
        t.onEnd { ends += it }
        core.mute = true
        val waiting = async { runCatching { c.call("doc.get", timeoutMs = 60_000) } }
        runCurrent()
        core.drop()
        advanceUntilIdle()
        assertEquals("lost", (waiting.await().exceptionOrNull() as RpcError).kind)
        assertEquals(ConnectionState.Lost, c.state.value)
        assertIs<End.Lost>(ends.single())
        val e = assertFailsWith<RpcError> { c.call("terminal.list") }
        assertEquals("lost", e.kind)
        // Controls on a lost stream are no-ops, not throws.
        t.write("x")
        t.resize(1, 1)
        advanceUntilIdle()
    }

    @Test
    fun subscribeSnapshotEventsAppliedCreditReturned() = runTest {
        val core = fake(handlers = mapOf("events.subscribe" to { _ ->
            compact.parseToJsonElement(
                """{"channel":1,"seq":3,"terminals":[{"address":"t1","state":"live","labels":{},"title":""}]}""",
            )
        }))
        val c = open(core)
        val sub = c.subscribe()
        val changes = mutableListOf<Boolean>()
        sub.onChange { changes += it.terminals }
        val line = """{"method":"event","params":{"seq":4,"type":"title","terminal":"t1","data":{"title":"hi"}}}""" + "\n"
        core.send(1, Frame.DATA, line.substring(0, 20))
        core.send(1, Frame.DATA, line.substring(20))
        advanceUntilIdle()
        assertEquals("hi", sub.lines.live()[0].title)
        assertEquals(listOf(true), changes)
        assertEquals(listOf(1024 * 1024, 20, line.length - 20), credits(core, 1))
    }

    @Test
    fun aCallTimesOut() = runTest {
        val core = fake()
        val c = open(core)
        core.mute = true
        val e = assertFailsWith<RpcError> { c.call("x", timeoutMs = 10) }
        assertEquals("timeout", e.kind)
    }

    @Test
    fun callAsDecodesTheResult() = runTest {
        val core = fake(handlers = mapOf("terminal.get" to { _ -> buildJsonObject { put("x", JsonPrimitive(1)) } }))
        val c = open(core)
        assertEquals(1, c.callAs<Map<String, Int>>("terminal.get")["x"])
    }
}
