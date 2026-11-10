package herdr.core

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertSame
import kotlin.test.assertTrue

@OptIn(ExperimentalCoroutinesApi::class)
class ModelTest {
    private fun json(name: String) = compact.parseToJsonElement(fixture(name).decodeToString()).jsonObject

    @Test
    fun attentionFollowsHerdr() {
        assertEquals(Attention.Blocked, attentionOf(AgentState.Error, true))
        assertEquals(Attention.Blocked, attentionOf(AgentState.Blocked, true))
        assertEquals(Attention.Working, attentionOf(AgentState.Working, false))
        assertEquals(Attention.Done, attentionOf(AgentState.Idle, false))
        assertEquals(Attention.Idle, attentionOf(AgentState.Done, true))
        assertNull(attentionOf(null, false))
        assertEquals(Attention.Done, rollup(listOf(Attention.Idle, Attention.Working, null, Attention.Done)))
        assertNull(rollup(listOf(null)))
    }

    // agents mod `label`: a named tab, else a title that tells something.
    @Test
    fun rowsSayWhatTheDesktopSays() {
        val claude = programNamed("claude-code")!!
        assertEquals("review", detailOf("✳ Fix the parser", "review", claude))
        assertEquals("Fix the parser", detailOf("✳ Fix the parser", "2", claude))
        assertEquals("Fix the parser", detailOf("Fix the parser", claude.name, claude))
        assertNull(detailOf("~/P/app", null, claude))
        assertNull(detailOf("dev@alpha: ~", null, claude))
        assertNull(detailOf("Claude Code", null, claude))
        assertNull(detailOf("", "1", claude))
    }

    @Test
    fun machinesSayWhyTheyAreDown() {
        assertNull(Machine("beta", false, Reach.Connected).reason)
        assertEquals("beta needs login; run hc machine login beta", Machine("beta", false, Reach.NeedsLogin).reason)
        assertEquals("no route to host", Machine("beta", false, Reach.Unreachable, why = "no route to host").reason)
    }

    @Test
    fun wordsANewerCoreUsesAreKeptAsOther() {
        assertEquals(AgentState.Blocked, AgentState.of("blocked"))
        assertEquals(AgentState.Other, AgentState.of("compacting"))
        assertEquals(AgentState.Other, AgentState.of(""))
        assertEquals(Attention.Done, attentionOf(AgentState.Other, false))
        assertEquals(Reach.NeedsLogin, Reach.of("needs_login"))
        assertEquals(Reach.Other, Reach.of("asleep"))
    }

    // model.rs: finishes_you_did_not_watch_are_done_until_seen
    @Test
    fun aFinishYouDidNotWatchIsDoneUntilSeen() {
        var a = nextAgent(null, AgentState.Idle, false, 1)
        assertEquals(Attention.Idle, attentionOf(a.state, a.seen), "idle when first met")
        a = nextAgent(a, AgentState.Working, false, 2)
        a = nextAgent(a, AgentState.Idle, false, 3)
        assertEquals(Attention.Done, attentionOf(a.state, a.seen))
        assertSame(a, nextAgent(a, AgentState.Idle, false, 4), "same word: no change")
        a = nextAgent(nextAgent(a, AgentState.Working, true, 5), AgentState.Done, true, 6)
        assertEquals(Attention.Idle, attentionOf(a.state, a.seen), "watched while it finished")
    }

    @Test
    fun foldersNameLikeTheDesktop() {
        assertEquals("herdr-app", folderName("/home/dev/Projects/herdr-app/"))
        assertEquals("~", folderName(""))
        assertEquals("/", folderName("/"))
    }

    @Test
    fun theSessionPlacesTerminalsInSpacesAndTabsFromTheLayoutDocuments() = runTest {
        val snapshot = json("snapshot.json")
        val docs = json("docs.json")
        val core = FakeCore(this, mutableMapOf(
            "events.subscribe" to { _ -> JsonObject(snapshot + ("channel" to JsonPrimitive(1))) },
            "machine.list" to { _ -> docs["machines"]!! },
            "doc.get" to { p -> if (p.string("on") == "beta") docs["beta"]!! else docs["local"]!! },
        ))
        // Not backgroundScope: advanceUntilIdle() doesn't wait for its work.
        val session = Session(CoroutineScope(StandardTestDispatcher(testScheduler)))
        session.use(connect(core.duplex, dispatcher = StandardTestDispatcher(testScheduler)))
        advanceUntilIdle()
        var s = session.snapshot.value
        assertEquals(Connection.Connected, s.connection)
        assertEquals(
            listOf(Triple("fake", true, Reach.Connected), Triple("beta", false, Reach.Connected)),
            s.machines.map { Triple(it.name, it.local, it.state) },
        )
        assertEquals(
            listOf(
                Triple("dotfiles", null, listOf("1" to listOf("t2"), "2" to listOf("t15"))),
                Triple("webapp", null, listOf("1" to listOf("t9"), "2" to listOf("t13"), "3" to listOf("t18"))),
                // t4 is in the newer layout but not in this snapshot: not shown.
                Triple("renderer", "beta", listOf("1" to listOf("beta/t1"), "2" to emptyList())),
            ),
            s.spaces.map { sp -> Triple(sp.name, sp.machine, sp.tabs.map { it.name to it.terminals }) },
        )
        val t9 = s.terminals.find { it.address == "t9" }
        assertEquals("claude-code", t9?.agent)
        assertEquals(Attention.Working, t9?.attention)
        assertEquals("sp28196228eedbe2dc329f454b7dce0ef7", t9?.space)
        assertEquals(
            "beta/spcae1915e68e0da788a1f79297633b3c1",
            s.terminals.find { it.address == "beta/t1" }?.space,
        )
        assertEquals(Attention.Working, s.spaces[1].attention)

        fun send(seq: Long, type: String, terminal: String, data: JsonElement) = core.send(
            1,
            Frame.DATA,
            buildJsonObject {
                put("method", "event")
                putJsonObject("params") {
                    put("seq", seq)
                    put("type", type)
                    put("terminal", terminal)
                    put("data", data)
                }
            }.toString() + "\n",
        )
        val base = snapshot.long("seq")!!
        // It stops while nobody looks: done, and the tab and space say so.
        send(base + 1, "status", "t9", compact.parseToJsonElement(
            """{"status":{"state":"idle","app":"claude-code"},"records":{},"reported":true}""",
        ))
        // A sub-agent lives with its parent; a labeled terminal in its tab.
        val first = snapshot.objects("terminals")[0]
        val child = JsonObject(first + mapOf(
            "address" to JsonPrimitive("t40"),
            "state" to JsonPrimitive("live"),
            "parent" to JsonPrimitive("t9"),
            "labels" to JsonObject(emptyMap()),
        ))
        send(base + 2, "created", "t40", child)
        val labeled = JsonObject(child + mapOf(
            "address" to JsonPrimitive("t41"),
            "parent" to JsonNull,
            "labels" to buildJsonObject {
                put("herdr.space", "sp28196228eedbe2dc329f454b7dce0ef7")
                put("herdr.tab", "tb78bf4af480b18fb5941930a2fc314dc7")
            },
        ))
        send(base + 3, "created", "t41", labeled)
        advanceUntilIdle()
        s = session.snapshot.value
        fun get(a: String) = session.snapshot.value.terminals.find { it.address == a }
        assertEquals(Attention.Done, get("t9")?.attention)
        assertEquals(Attention.Done, s.spaces[1].tabs[0].attention)
        assertEquals("tb7661ff13518383114ef8f0277e86ea04", get("t40")?.tab)
        assertEquals("tb78bf4af480b18fb5941930a2fc314dc7", get("t41")?.tab)

        val before = s
        session.setVisible(listOf("t9"))
        advanceUntilIdle()
        s = session.snapshot.value
        assertEquals(Attention.Idle, get("t9")?.attention)
        assertNotEquals(before, s, "a new snapshot")
        advanceUntilIdle()
        assertSame(s, session.snapshot.value, "stable while nothing changes")

        // Blocked needs the person even while looking; error counts as blocked.
        send(base + 4, "status", "t9", compact.parseToJsonElement(
            """{"status":{"state":"error"},"records":{},"reported":true}""",
        ))
        advanceUntilIdle()
        s = session.snapshot.value
        assertEquals(Attention.Blocked, get("t9")?.attention)
        assertEquals(Attention.Blocked, s.spaces[1].attention)

        core.drop()
        advanceUntilIdle()
        s = session.snapshot.value
        assertEquals(Connection.Lost, s.connection)
        assertTrue(s.terminals.isNotEmpty(), "what we knew stays on screen")
        session.dispose()
        advanceUntilIdle()
    }
}
