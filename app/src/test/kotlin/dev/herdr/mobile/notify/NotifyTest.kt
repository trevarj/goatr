package io.github.trevarj.goatr.app.notify

import io.github.trevarj.goatr.app.agent
import io.github.trevarj.goatr.app.snapshot
import herdr.core.AgentState
import herdr.core.Connection
import kotlin.test.Test
import kotlin.test.assertEquals

class NotifyTest {
    private val host = "dev@alpha.test:22"
    private val nobodyLooking: (String) -> Boolean = { false }

    private fun Watch.rings(vararg snaps: herdr.core.SessionSnapshot, looking: (String) -> Boolean = nobodyLooking) =
        snaps.map { tend(listOf(host to it), looking).rings.map { r -> r.address to r.alert } }.last()

    // herdr-app's model.rs cases: the phone rings when the desktop does.
    @Test
    fun ringsAsTheDesktop() {
        assertEquals(Alert.Blocked, alertFor(AgentState.Working, AgentState.Blocked, looking = true))
        assertEquals(null, alertFor(AgentState.Blocked, AgentState.Error, looking = false))
        assertEquals(Alert.Done, alertFor(AgentState.Working, AgentState.Idle, looking = false))
        assertEquals(Alert.Done, alertFor(AgentState.Blocked, AgentState.Done, looking = false))
        assertEquals(null, alertFor(AgentState.Working, AgentState.Idle, looking = true))
        assertEquals(null, alertFor(null, AgentState.Idle, looking = false))
        assertEquals(null, alertFor(AgentState.Working, null, looking = false))
    }

    @Test
    fun whatAHostHadWhenFirstListedIsNotNews() {
        val w = Watch()
        assertEquals(emptyList(), w.rings(snapshot(agent("t1", "blocked"), agent("t2", "working"))))
        // From then on, changes ring.
        assertEquals(listOf("$host t2" to Alert.Done), w.rings(snapshot(agent("t1", "blocked"), agent("t2", "idle"))))
    }

    @Test
    fun aNewTerminalThatNeedsYouRings() {
        val w = Watch()
        w.rings(snapshot(agent("t1", "idle")))
        assertEquals(listOf("$host t2" to Alert.Blocked), w.rings(snapshot(agent("t1", "idle"), agent("t2", "blocked"))))
    }

    @Test
    fun onlyAgentsInASpaceRingAndShowAsWorking() {
        val w = Watch()
        val before = snapshot(agent("t1", "working", parent = "t9"), agent("t2", "working", space = null), agent("t3", "working"))
        val working = w.tend(listOf(host to before), nobodyLooking).working.map { it.address }
        assertEquals(listOf("$host t3"), working)
        val after = snapshot(agent("t1", "idle", parent = "t9"), agent("t2", "idle", space = null), agent("t3", "idle"))
        assertEquals(listOf("$host t3" to Alert.Done), w.rings(after))
    }

    @Test
    fun finishingWhileLookingDoesNotRing() {
        val w = Watch()
        w.rings(snapshot(agent("t1", "working")))
        assertEquals(emptyList(), w.rings(snapshot(agent("t1", "idle")), looking = { it == "$host t1" }))
    }

    @Test
    fun aReconnectRingsOnlyWhatChangedMeanwhile() {
        val w = Watch()
        w.rings(snapshot(agent("t1", "blocked"), agent("t2", "working")))
        w.rings(snapshot(connection = Connection.Lost))
        // t1 still blocked: already told. t2 finished while the link was down: news.
        assertEquals(
            listOf("$host t2" to Alert.Done),
            w.rings(snapshot(agent("t1", "blocked"), agent("t2", "idle"))),
        )
    }

    @Test
    fun aPreviewIsTheLastLinesWithWordsInThem() {
        val screen = """
            ● Edit src/main.rs
            ╭──────────────────────────────╮
            │ Do you want to make this edit? │
            │ ❯ 1. Yes                      │
            │   2. No                       │
            ╰──────────────────────────────╯

        """.trimIndent()
        assertEquals("● Edit src/main.rs\nDo you want to make this edit?\n❯ 1. Yes\n2. No", previewOf(screen))
        assertEquals(null, previewOf("\n───\n  \n"))
    }
}
