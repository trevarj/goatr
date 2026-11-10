// What the screens draw: herdr.core's session, mapped in Live.kt.

package io.github.trevarj.goatr.app.session

import herdr.core.Attention
import herdr.core.Program

/** Partial: several hosts, some up and some lost. */
enum class Connection(val label: String) {
    Idle("no host"), Connected("connected"), Connecting("connecting"), Partial("partly connected"), Lost("lost")
}

enum class MachineState(val label: String) {
    Connected("connected"), Connecting("connecting"), Asleep("asleep"), Lost("lost")
}

data class Machine(
    /** The host (Settings' id) whose core reaches it. */
    val host: String,
    /** null: the host itself. */
    val name: String?,
    /** What to call it: its name, or the host core's. */
    val label: String,
    val state: MachineState,
    /** Why it isn't connected, as the desktop says it (one the host's core reaches). */
    val problem: String? = null,
    /** Since when it isn't connected (ms since the epoch). */
    val since: Long? = null,
) {
    /** Its state to show: `lost · 12m`. */
    fun stateLabel(now: Long) = state.label + (since?.let { " · " + duration(now - it) } ?: "")
}

data class Tab(val id: String, val name: String, val terminals: List<String>)

data class Space(
    /** Host-qualified, as every id here (see `ref`). */
    val id: String,
    val host: String,
    val name: String,
    /** The machine as the host's core names it (null: the host itself). */
    val machine: String?,
    /** Its avatar color: its position, in the desktop's avatar order. */
    val avatar: Int,
    val tabs: List<Tab>,
    /** The name someone gave it; "" when it's named after its folder. */
    val given: String = name,
    /** Where it was made (its terminals may have moved on). */
    val folder: String = "",
)

data class Terminal(
    /** Host-qualified (`dev@alpha.local:22 beta/t1`). */
    val address: String,
    val host: String,
    /** Where it runs, to show: another machine, or the host when there are several. */
    val machine: String?,
    /** The program's name (claude-code, fish, …) or the terminal's own name. */
    val name: String,
    /** What it says it is doing, when that adds something. */
    val title: String?,
    val agent: Boolean,
    /** How it shows, as on the desktop: name, icon and tile. */
    val program: Program,
    val attention: Attention?,
    /** When its attention last changed (ms since epoch). */
    val since: Long?,
    val space: String?,
    val tab: String?,
    /** Pinned to the top of the agent lists, on every screen. */
    val pinned: Boolean = false,
)

data class Session(
    /** What the connection badge says: the host's name, or how many hosts are up. */
    val host: String = "",
    /** Idle: no host added (or on) yet. */
    val connection: Connection = Connection.Idle,
    val machines: List<Machine> = emptyList(),
    val spaces: List<Space> = emptyList(),
    val terminals: List<Terminal> = emptyList(),
)

/** The space's most urgent agent, which is what its row speaks up about. */
fun spaceAttention(space: Space, terminals: List<Terminal>): Attention? =
    terminals.filter { it.space == space.id }.mapNotNull { it.attention }.maxByOrNull { it.priority }

/**
 * Agents by priority: most urgent first, then the most recent change. Only
 * those in a space, as the desktop's agents(): one outside the layout (its
 * pane closed, an old status left behind) has no tabs to open into.
 */
fun byPriority(terminals: List<Terminal>): List<Terminal> =
    terminals.filter { it.agent && it.attention != null && it.space != null }
        .sortedWith(compareByDescending<Terminal> { it.attention!!.priority }.thenByDescending { it.since ?: 0 })

/**
 * The agents to list, [byPriority]: only those of hosts that are connected.
 * A lost host's are what it last said, not what they're doing, pinned or not.
 */
fun Session.agents(): List<Terminal> {
    val up = machines.filter { it.name == null && it.state == MachineState.Connected }.mapTo(HashSet()) { it.host }
    return byPriority(terminals.filter { it.host in up })
}

fun duration(ms: Long): String {
    val m = ms / 60_000
    if (m < 1) return "<1m"
    if (m < 60) return "${m}m"
    val h = m / 60
    return if (h < 24) "${h}h" else "${h / 24}d"
}
