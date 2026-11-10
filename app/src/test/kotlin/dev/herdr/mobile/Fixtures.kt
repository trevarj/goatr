// Core snapshots as a host's core would send them, for tests.

package io.github.trevarj.goatr.app

import herdr.core.AgentState
import herdr.core.Connection
import herdr.core.CoreInfo
import herdr.core.Machine
import herdr.core.Reach
import herdr.core.SessionSnapshot
import herdr.core.Terminal
import herdr.core.TerminalRecord
import herdr.core.attentionOf
import herdr.core.programNamed
import kotlinx.serialization.json.JsonObject

/** An agent in space `sp1` (or none), saying `status`; seen unless told otherwise. */
fun agent(
    address: String,
    status: String?,
    space: String? = "sp1",
    parent: String? = null,
    seen: Boolean = false,
    machine: String? = null,
) = Terminal(
    address = address,
    machine = machine,
    name = null,
    agent = "claude-code",
    program = "claude",
    shown = programNamed("claude-code")!!,
    title = "",
    cwd = "",
    status = status?.let(AgentState::of),
    detail = "task $address",
    attention = attentionOf(status?.let(AgentState::of), seen),
    statusMs = null,
    space = space,
    tab = space?.let { "tb1" },
    where = null,
    parent = parent,
    pinned = false,
    record = TerminalRecord(JsonObject(emptyMap())),
)

fun snapshot(
    vararg terminals: Terminal,
    connection: Connection = Connection.Connected,
    core: String = "alpha",
    machines: List<Machine> = listOf(Machine(core, local = true, state = Reach.Connected)),
) = SessionSnapshot(
    connection = connection,
    core = CoreInfo(core, core, "0.1.15"),
    machines = machines,
    terminals = terminals.toList(),
)

