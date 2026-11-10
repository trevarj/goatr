package herdr.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.long
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotSame
import kotlin.test.assertTrue

class EventsTest {
    private fun bytes(s: String) = s.encodeToByteArray()

    private fun event(seq: Long, type: String, terminal: String, data: JsonObject) =
        buildJsonObject {
            put("method", "event")
            putJsonObject("params") {
                put("seq", seq)
                put("type", type)
                put("terminal", terminal)
                put("data", data)
            }
        }.toString() + "\n"

    private fun obj(json: String) = Json.parseToJsonElement(json).jsonObject

    private fun rec(address: String) =
        TerminalRecord(obj("""{"address":"$address","state":"live","title":"","labels":{},"status":null}"""))

    private fun addresses(l: EventLines) = l.live().map { it.address }

    // hc.rs: the_event_stream_keeps_the_terminals_as_the_core_has_them
    @Test
    fun theEventStreamKeepsTheTerminalsAsTheCoreHasThem() {
        val lines = EventLines(Snapshot(seq = 5, terminals = listOf(rec("t1"))))
        assertEquals(listOf("t1"), addresses(lines))
        assertFalse(lines.push(bytes(event(5, "title", "t1", obj("""{"title":"old"}""")))).terminals)
        val got = lines.push(
            bytes(event(6, "created", "t2", rec("t2").json) + event(7, "title", "t1", obj("""{"title":"hi"}"""))),
        )
        assertTrue(got.terminals && !got.docs)
        assertEquals(listOf("t1", "t2"), addresses(lines))
        assertEquals("hi", lines.live()[0].title)
        lines.push(
            bytes(event(8, "status", "t2", obj("""{"status":{"state":"working"},"records":{},"reported":true}"""))),
        )
        lines.push(
            bytes(event(9, "foreground", "t2", obj("""{"pid":7,"name":"vim","argv":["vim"],"cwd":"/src"}"""))),
        )
        assertEquals("working", lines.live()[1].status?.state)
        assertEquals("/src", lines.live()[1].foreground?.cwd)
        lines.push(
            bytes(
                event(10, "finished", "t1", obj("""{"reason":"exited","exit_code":0,"signal":null,"finished_ms":1}""")),
            ),
        )
        assertEquals(listOf("t2"), addresses(lines), "finished: no longer live")
        lines.push(bytes(event(11, "removed", "t2", JsonObject(emptyMap()))))
        assertEquals(emptyList(), addresses(lines))
        val missed = buildJsonObject {
            put("method", "missed")
            putJsonObject("params") {
                put("seq", 20)
                putJsonArray("terminals") { add(rec("t9").json) }
            }
        }.toString() + "\n"
        val again = lines.push(bytes(missed))
        assertTrue(again.terminals && again.docs, "missed: everything again")
        assertEquals(listOf("t9"), addresses(lines))
        assertEquals(20, lines.seq)
        assertTrue(lines.push(bytes("""{"method":"event","params":{"seq":21,"type":"doc"}}""" + "\n")).docs)
    }

    @Test
    fun recordsAreReplacedNotMutated() {
        val lines = EventLines(Snapshot(seq = 0, terminals = listOf(rec("t1"))))
        val before = lines.terminals[0]
        lines.push(bytes(event(1, "title", "t1", obj("""{"title":"x"}"""))))
        assertEquals("", before.title)
        assertNotSame(before, lines.terminals[0])
    }

    // hc.rs: document_and_missed_events_survive_arbitrary_frame_boundaries
    @Test
    fun documentAndMissedEventsSurviveArbitraryFrameBoundaries() {
        for (line in listOf(
            """{"method":"event","params":{"type":"doc"}}""" + "\n",
            """{"method":"missed","params":{"terminals":[]}}""" + "\n",
        )) {
            val b = bytes(line)
            for (at in 1 until b.size) {
                val lines = EventLines()
                assertFalse(lines.push(b.copyOfRange(0, at)).docs)
                assertTrue(lines.push(b.copyOfRange(at, b.size)).docs)
            }
        }
    }

    @Test
    fun multiByteCharactersSplitAcrossFrames() {
        val b = bytes(event(1, "title", "t1", obj("""{"title":"café ☕"}""")))
        val lines = EventLines(Snapshot(seq = 0, terminals = listOf(rec("t1"))))
        val cut = b.indexOf(0xe2.toByte()) + 1
        lines.push(b.copyOfRange(0, cut))
        lines.push(b.copyOfRange(cut, b.size))
        assertEquals("café ☕", lines.live()[0].title)
    }

    @Test
    fun heartbeatsAreReportedGarbageLinesAreSkipped() {
        val got = EventLines().push(bytes("not json\n" + """{"method":"heartbeat","params":{"seq":3}}""" + "\n"))
        assertTrue(got.heartbeat && !got.terminals && !got.docs)
    }

    @Test
    fun aRecordedStreamFromARealCoreAppliesToItsSnapshot() {
        val snapshot = lenient.decodeFromString(Snapshot.serializer(), fixture("snapshot.json").decodeToString())
        val raw = fixture("events.raw")
        val lines = EventLines(snapshot)
        // Split into uneven frames, as the transport would.
        var terminals = false
        var docs = false
        for (at in raw.indices step 97) {
            val p = lines.push(raw.copyOfRange(at, minOf(at + 97, raw.size)))
            terminals = terminals || p.terminals
            docs = docs || p.docs
        }
        assertTrue(terminals && docs)
        // A remote terminal created, then labeled by the desktop with where it lives.
        val t4 = lines.live().find { it.address == "beta/t4" }
        assertEquals("renderer / 2", t4?.labels?.get("where"))
        val seqs = raw.decodeToString().trim().split("\n").map {
            obj(it)["params"]!!.jsonObject["seq"]?.jsonPrimitive?.long ?: 0
        }
        assertEquals(maxOf(snapshot.seq, seqs.max()), lines.seq)
        // The test terminal was created, titled, killed and removed.
        assertFalse(lines.terminals.any { it.labels["test"] == "webapp-core" })
    }
}
