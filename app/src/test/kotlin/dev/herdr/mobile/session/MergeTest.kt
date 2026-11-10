package io.github.trevarj.goatr.app.session

import io.github.trevarj.goatr.app.agent
import io.github.trevarj.goatr.app.snapshot
import herdr.core.Connection
import herdr.core.Reach
import herdr.core.Machine as CoreMachine
import kotlin.test.Test
import kotlin.test.assertEquals

class MergeTest {
    private val alpha = Host(id = "dev@alpha.test:22", user = "dev", host = "alpha.test")
    private val beta = Host(id = "dev@beta.test:22", user = "dev", host = "beta.test")

    @Test
    fun desktopSelectionNeverFallsBackOrConnectsAnUnconfirmedPin() {
        val one = Host("iroh:one", "", "one", peer = "one")
        val two = Host("iroh:two", "", "two", peer = "two")
        val pending = Host("iroh:pending", "", "pending", peer = "pending", paired = false)
        val hosts = listOf(alpha, beta.copy(off = true), one, two, pending)
        assertEquals(listOf(alpha, one), activeHosts(hosts, "one"))
        assertEquals(listOf(alpha, two), activeHosts(hosts, "two"))
        assertEquals(listOf(alpha), activeHosts(hosts, "pending"))
        assertEquals(listOf(alpha), activeHosts(hosts, "missing"))
        assertEquals(listOf(alpha), activeHosts(hosts, null))
    }

    private val reachesBeta = listOf(
        CoreMachine("alpha", local = true, state = Reach.Connected),
        CoreMachine("beta", local = false, state = Reach.Connected),
    )

    @Test
    fun machinesBehindADroppedLinkAreDownToo() {
        val up = merge(listOf(alpha to snapshot(machines = reachesBeta)), idle = false)
        assertEquals(listOf(MachineState.Connected, MachineState.Connected), up.machines.map { it.state })

        val lost = merge(listOf(alpha to snapshot(connection = Connection.Lost, machines = reachesBeta)), idle = false)
        assertEquals(listOf(MachineState.Lost, MachineState.Lost), lost.machines.map { it.state })
        assertEquals(Connection.Lost.name, lost.connection.name)

        // One host of several down: yellow, not red.
        val partial = merge(listOf(alpha to snapshot(), beta to snapshot(connection = Connection.Lost, core = "beta")), idle = false)
        assertEquals(io.github.trevarj.goatr.app.session.Connection.Partial, partial.connection)
    }

    @Test
    fun aMachineReachedTwoWaysShowsOnceDirectly() {
        val viaAlpha = snapshot(agent("beta/t1", "working", machine = "beta"), agent("t2", "idle"), machines = reachesBeta)
        val direct = snapshot(agent("t1", "working"), core = "beta")
        val s = merge(listOf(alpha to viaAlpha, beta to direct), idle = false)

        assertEquals(listOf("alpha", "beta"), s.machines.map { it.label })
        assertEquals(listOf(alpha.id, beta.id), s.machines.map { it.host })
        // beta's agent once, by its own link; alpha's own still there.
        assertEquals(listOf("${alpha.id} t2", "${beta.id} t1"), s.terminals.map { it.address })
        assertEquals("2/2 hosts", s.host)
    }

    @Test
    fun aLostHostsAgentsAreNotListedEvenPinned() {
        val up = snapshot(agent("t1", "working"))
        val lost = snapshot(agent("t1", "blocked").copy(pinned = true), connection = Connection.Lost, core = "beta")
        val s = merge(listOf(alpha to up, beta to lost), idle = false)

        assertEquals(listOf("${alpha.id} t1", "${beta.id} t1"), s.terminals.map { it.address })
        assertEquals(listOf("${alpha.id} t1"), s.agents().map { it.address })
    }

    @Test
    fun parsesWhatYouTypeToAddAHost() {
        assertEquals(Host(id = "dev@box:22", user = "dev", host = "box"), parseTarget(" dev@box "))
        assertEquals(2222, parseTarget("me@10.0.0.2:2222")?.port)
        for (bad in listOf("box", "@box", "dev@", "dev@box:port", "dev@b ox")) assertEquals(null, parseTarget(bad), bad)
    }
}
