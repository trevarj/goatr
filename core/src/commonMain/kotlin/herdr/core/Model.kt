// The session the UI renders: machines, spaces with their tabs, terminals
// with attention. Fed by one Client (events, machine.list, the desktop's
// `herdr-app` layout documents); `seen` is the phone's own. Exposed as a
// StateFlow of immutable snapshots for Compose's `collectAsState()`.

package herdr.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlin.coroutines.ContinuationInterceptor
import kotlin.time.Clock
import kotlin.time.ExperimentalTime

/** Herdr's attention: blocked (incl. error) > done (finished, unseen) > working > idle. */
enum class Attention(val priority: Int) {
    Blocked(4),
    Done(3),
    Working(2),
    Idle(1),
}

enum class Connection { Idle, Connecting, Connected, Lost }

/** A program's own word for what it's doing (its 7501 `state`); one this app doesn't know is [Other]. */
enum class AgentState {
    Idle, Working, Done, Blocked, Error, Other;

    /** Needs you: blocked, or failed. */
    val blocked: Boolean get() = this == Blocked || this == Error

    companion object {
        fun of(word: String): AgentState = entries.find { it != Other && it.name.equals(word, ignoreCase = true) } ?: Other
    }
}

/** How a core reaches a machine (`machine.list` `state`); one this app doesn't know is [Other]. */
enum class Reach {
    Connecting, Connected, NeedsLogin, NeedsUpdate, Unreachable, Other;

    companion object {
        fun of(state: String): Reach = when (state) {
            "connecting" -> Connecting
            "connected" -> Connected
            "needs_login" -> NeedsLogin
            "needs_update" -> NeedsUpdate
            "unreachable" -> Unreachable
            else -> Other
        }
    }
}

data class Machine(
    val name: String,
    /** The core the phone talks to; the others are reached through it. */
    val local: Boolean,
    /** The local one's is the phone's connection to it. */
    val state: Reach,
    val host: String? = null,
    val version: String? = null,
    /** Since when it is not connected, ms. */
    val sinceMs: Long? = null,
    val why: String? = null,
) {
    /** Why it isn't connected, as the desktop says it; null when it is. */
    val reason: String?
        get() = if (state == Reach.Connected) null else why ?: when (state) {
            Reach.NeedsUpdate -> "$name needs a core update; run hc machine update $name"
            Reach.NeedsLogin -> "$name needs login; run hc machine login $name"
            Reach.Connecting -> "$name is connecting"
            else -> "$name is unreachable"
        }
}

data class Terminal(
    /** Machine-qualified (`t7`, `beta/t1`): what calls take. */
    val address: String,
    /** Null: the local core's. */
    val machine: String?,
    val name: String?,
    /** The reporting agent (`claude-code`), from its 7501 status. */
    val agent: String?,
    /** What runs in it now (`fish`, `vim`). */
    val program: String,
    /** How it shows, as on the desktop: the agent's entry, else what it runs. */
    val shown: Program,
    /** What the program set (OSC 0/2). */
    val title: String,
    /** What the desktop's agents list adds to its row: see [detailOf]. */
    val detail: String?,
    val cwd: String,
    /** The program's own word, if it reports. */
    val status: AgentState?,
    val attention: Attention?,
    /** When the word last changed, ms since the epoch (null: unknown). */
    val statusMs: Long?,
    /** Qualified ids of where it lives, if the layout places it. */
    val space: String?,
    val tab: String?,
    /** The desktop's `where` label, `<space> / <tab>`. */
    val where: String?,
    val parent: String?,
    /** Pinned to the top of the agent lists, on every screen (`herdr.pinned`). */
    val pinned: Boolean,
    val record: TerminalRecord,
)

data class Tab(
    val id: String,
    val name: String,
    /** Addresses in layout order. */
    val terminals: List<String>,
    val attention: Attention?,
)

data class Space(
    /** Qualified like addresses (`beta/sp...`). */
    val id: String,
    val name: String,
    /** The name someone gave it; "" when it's named after its folder. */
    val given: String,
    val machine: String?,
    val folder: String,
    val tabs: List<Tab>,
    val attention: Attention?,
)

data class CoreInfo(val id: String, val name: String, val version: String)

data class SessionSnapshot(
    val connection: Connection = Connection.Idle,
    val lostReason: String? = null,
    val core: CoreInfo? = null,
    val machines: List<Machine> = emptyList(),
    /** Local first, then other machines by name (the desktop's order). */
    val spaces: List<Space> = emptyList(),
    /** Live terminals, in the core's order. */
    val terminals: List<Terminal> = emptyList(),
)

/** An agent's word as the phone last saw it. */
data class Agent(val state: AgentState?, val seen: Boolean, val since: Long)

@Serializable
private data class MachineListItem(
    val name: String,
    val state: String = "",
    val host: String? = null,
    val version: String? = null,
    @SerialName("since_ms") val sinceMs: Long? = null,
    val why: String? = null,
    /** An agent's short-lived sandbox: left out, as the desktop does. */
    val temporary: Boolean = false,
)

@Serializable
private data class MachineList(val machines: List<MachineListItem> = emptyList())

fun qualify(id: String, machine: String?): String = if (machine.isNullOrEmpty()) id else "$machine/$id"

/** The machine in a qualified address: `beta/t1` → `beta`. */
fun machineOf(address: String): String? {
    val at = address.lastIndexOf('/')
    return if (at > 0) address.substring(0, at) else null
}

/** Terminal addresses in a layout subtree (mirror.rs `Document` nodes), in tree order. */
fun layoutTerminals(node: JsonElement?): List<String> {
    val n = node as? JsonObject ?: return emptyList()
    val own = listOfNotNull((n["content"] as? JsonObject)?.string("terminal")?.takeIf { it.isNotEmpty() })
    val children = (n["children"] as? JsonArray).orEmpty().flatMap(::layoutTerminals)
    return own + children
}

/** A folder as a name: its last part, `~` for empty (model.rs `folder_name`, no home). */
fun folderName(path: String): String {
    if (path.isEmpty()) return "~"
    val trimmed = path.trimEnd('/', '\\')
    if (trimmed.isEmpty()) return path.substring(0, 1)
    return trimmed.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: trimmed
}

/** What the UI shows for a word and whether it was seen (model.rs `status`). */
fun attentionOf(state: AgentState?, seen: Boolean): Attention? = when {
    state == null -> null
    state == AgentState.Working -> Attention.Working
    state.blocked -> Attention.Blocked
    else -> if (seen) Attention.Idle else Attention.Done
}

private val LEADING_SYMBOLS = Regex("""^[^\p{L}\p{N}]+""")
private val PATH = Regex("""^[~/]|^[A-Za-z]:\\""")

/**
 * What the desktop's agents list adds to a terminal's row (agents mod
 * `label`): a tab someone named, else the program's own title when it tells
 * something (not its name, a path or `user@host`); never an automatic tab number.
 */
fun detailOf(title: String, tab: String?, program: Program): String? {
    tab?.takeIf { name -> !name.all(Char::isDigit) && name != program.name }?.let { return it }
    val live = title.replace(LEADING_SYMBOLS, "").trim()
    val telling = live.isNotEmpty() && !PATH.containsMatchIn(title.trim()) && '@' !in live &&
        live != program.name && !live.equals(program.key, ignoreCase = true)
    return live.takeIf { telling }
}

/** The most urgent attention among some. */
fun rollup(list: Iterable<Attention?>): Attention? = list.filterNotNull().maxByOrNull { it.priority }

/**
 * Applies a new word (model.rs `set_word`): a finish you didn't watch is
 * `done` until you look at it.
 */
fun nextAgent(prev: Agent?, state: AgentState?, looking: Boolean, now: Long): Agent {
    val a = prev ?: Agent(null, true, 0)
    if (a.state == state) return a
    fun busy(s: AgentState?) = s == AgentState.Working || s == AgentState.Blocked
    val seen = when {
        busy(state) -> true
        state != null && busy(a.state) -> looking
        else -> a.seen
    }
    return Agent(state, seen, now)
}

@OptIn(ExperimentalTime::class)
private fun epochMs() = Clock.System.now().toEpochMilliseconds()

/** A suspend call whose failure is only logged by being ignored; cancellation still cancels. */
internal suspend fun <T> attempt(block: suspend () -> T): T? = try {
    block()
} catch (e: CancellationException) {
    throw e
} catch (_: Exception) {
    null
}

internal fun JsonObject.objects(key: String): List<JsonObject> =
    (this[key] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }

/**
 * The session. All its state lives on [scope]'s dispatcher, which must run
 * one task at a time (Main in the app, the test dispatcher in tests); every
 * command is safe to call from any thread.
 */
class Session(private val scope: CoroutineScope, private val clock: () -> Long = ::epochMs) {
    private val _snapshot = MutableStateFlow(SessionSnapshot())
    val snapshot: StateFlow<SessionSnapshot> = _snapshot

    private val dispatcher = scope.coroutineContext[ContinuationInterceptor]!!
    private var connection = Connection.Idle
    private var lostReason: String? = null
    private var client: Client? = null
    private var sub: Subscription? = null
    private var watching: Job? = null
    private var records: List<TerminalRecord> = emptyList()
    private var machines: List<Machine> = emptyList()

    // Layout documents by machine ("" for the local core).
    private val docs = mutableMapOf<String, JsonObject>()
    private val agents = mutableMapOf<String, Agent>()
    private var visible = emptySet<String>()
    private var scheduled = false
    private var generation = 0

    private fun emit() {
        if (scheduled) return
        scheduled = true
        // Many events arrive together: one snapshot for the batch.
        scope.launch {
            scheduled = false
            _snapshot.value = build()
        }
    }

    private class Place(val space: String, val tab: String)

    private fun build(): SessionSnapshot {
        val now = clock()
        val live = records.filter { it.state == "live" }
        val byAddress = live.associateBy { it.address }

        // Where each terminal lives: the layout documents first.
        class Draft(val id: String, val name: String, val machine: String?, val folder: String, val tabs: List<Tab>)
        val drafts = mutableListOf<Draft>()
        val place = mutableMapOf<String, Place>()
        val tabSpace = mutableMapOf<String, String>()
        val tabName = mutableMapOf<String, String>()
        val order = listOf("") + docs.keys.filter { it.isNotEmpty() }.sorted()
        for (m in order) {
            val machine = m.ifEmpty { null }
            for (s in docs[m]?.objects("spaces").orEmpty()) {
                val sid = s.string("id")?.takeIf { it.isNotEmpty() } ?: continue
                val id = qualify(sid, machine)
                val tabs = s.objects("tabs").mapIndexedNotNull { i, t ->
                    val tid = t.string("id")?.takeIf { it.isNotEmpty() } ?: return@mapIndexedNotNull null
                    val tabId = qualify(tid, machine)
                    val terminals = layoutTerminals(t["root"]).map { qualify(it, machine) }
                    for (a in terminals) place[a] = Place(id, tabId)
                    tabSpace[tabId] = id
                    val name = t.string("name")?.takeIf { it.isNotEmpty() } ?: "${i + 1}"
                    tabName[tabId] = name
                    Tab(tabId, name, terminals, null)
                }
                drafts += Draft(id, s.string("name") ?: "", machine, s.string("folder") ?: "", tabs)
            }
        }
        val spaceIds = drafts.map { it.id }.toSet()

        // Then the labels the desktop puts on terminals it starts but doesn't show yet.
        for (r in live) {
            if (r.address in place) continue
            val machine = machineOf(r.address)
            val space = r.labels["herdr.space"]?.takeIf { it.isNotEmpty() }?.let { qualify(it, machine) }
            val tab = r.labels["herdr.tab"]?.takeIf { it.isNotEmpty() }?.let { qualify(it, machine) }
            if (space != null && tab != null && space in spaceIds && tabSpace[tab] == space) {
                place[r.address] = Place(space, tab)
            }
        }

        // Sub-agents live where their parent does.
        fun placeOf(address: String): Place? {
            var cur = address
            repeat(32) {
                place[cur]?.let { return it }
                val parent = byAddress[cur]?.parent
                if (parent.isNullOrEmpty()) return null
                cur = qualify(parent, machineOf(cur))
            }
            return null
        }

        val terminals = live.map { r ->
            val status = r.status
            val state = status?.state?.let(AgentState::of)
            val prev = agents[r.address]
            val agent = nextAgent(prev, state, r.address in visible, r.statusMs ?: now)
            if (agent !== prev) agents[r.address] = agent
            val at = placeOf(r.address)
            val app = status?.app ?: r.records.values.firstOrNull { !it.app.isNullOrEmpty() }?.app
            val foreground = r.foreground
            val shown = programOf(app, foreground?.argv?.takeIf { it.isNotEmpty() } ?: r.argv)
            val title = r.title ?: ""
            Terminal(
                address = r.address,
                machine = machineOf(r.address),
                name = r.name,
                agent = app,
                program = foreground?.name ?: r.argv.firstOrNull()?.substringAfterLast('/').orEmpty(),
                shown = shown,
                title = title,
                detail = detailOf(title, at?.tab?.let(tabName::get), shown),
                cwd = foreground?.cwd ?: r.cwd,
                status = state,
                attention = attentionOf(agent.state, agent.seen),
                statusMs = r.statusMs ?: agent.since.takeIf { it != 0L },
                space = at?.space,
                tab = at?.tab,
                where = r.labels["where"],
                parent = r.parent?.takeIf { it.isNotEmpty() }?.let { qualify(it, machineOf(r.address)) },
                pinned = !r.labels["herdr.pinned"].isNullOrEmpty(),
                record = r,
            )
        }
        agents.keys.retainAll(byAddress.keys)

        val byTab = terminals.groupBy { it.tab }
        val spaces = drafts.map { d ->
            val tabs = d.tabs.map { tab ->
                tab.copy(
                    attention = rollup(byTab[tab.id].orEmpty().map { it.attention }),
                    // Gone from core but still in the layout: not shown.
                    terminals = tab.terminals.filter { it in byAddress },
                )
            }
            val s = Space(d.id, d.name, d.name, d.machine, d.folder, tabs, rollup(tabs.map { it.attention }))
            if (s.name.isEmpty()) s.copy(name = spaceName(s, terminals)) else s
        }

        val hello = client?.hello
        return SessionSnapshot(
            connection = connection,
            lostReason = lostReason,
            core = hello?.let { CoreInfo(it.core.id, it.core.name, it.version) },
            machines = listOfNotNull(hello?.let { Machine(it.core.name, local = true, state = reachOf(connection)) }) + machines,
            spaces = spaces,
            terminals = terminals,
        )
    }

    private suspend fun readDoc(c: Client, machine: String) {
        // Unavailable (not connected, an old core there): keep what we had.
        docs[machine] = attempt { readLayout(c, machine.ifEmpty { null }) }?.doc ?: return
    }

    private suspend fun readDocs(c: Client) {
        val connected = machines.filter { it.state == Reach.Connected }.map { it.name }
        docs.keys.removeAll { it.isNotEmpty() && it !in connected }
        coroutineScope { (listOf("") + connected).map { async { readDoc(c, it) } }.awaitAll() }
        emit()
    }

    private suspend fun readMachines(c: Client) {
        // Lost connection: the state says so.
        val r = attempt { compact.decodeFromJsonElement(MachineList.serializer(), c.call("machine.list")) } ?: return
        val before = machines.associate { it.name to it.state }
        machines = r.machines.filter { !it.temporary }.map {
            Machine(it.name, false, Reach.of(it.state), it.host, it.version, it.sinceMs, it.why)
        }
        emit()
        // A machine that came back may have a different layout now.
        if (machines.any { before[it.name] != it.state }) readDocs(c)
    }

    private suspend fun follow(c: Client, mine: Int) {
        val s = c.subscribe()
        if (mine != generation) return s.close()
        sub = s
        // The lines are the client's: read them on its dispatcher, keep a copy.
        val first = CompletableDeferred<List<TerminalRecord>>()
        c.post { first.complete(s.lines.terminals.toList()) }
        records = first.await()
        s.onChange { pushed ->
            val now = s.lines.terminals.toList()
            scope.launch {
                if (mine != generation) return@launch
                records = now
                if (pushed.terminals) emit()
                if (pushed.docs) launch { readDocs(c) }
                if (pushed.heartbeat) launch { readMachines(c) }
            }
        }
        s.onEnd { end ->
            scope.launch {
                if (mine != generation || end is End.Lost) return@launch
                // A stream the core ended is subscribed to again, with a fresh snapshot.
                delay(1000)
                if (mine == generation && c.state.value == ConnectionState.Connected) attempt { follow(c, mine) }
            }
        }
        emit()
    }

    private fun reachOf(c: Connection) = when (c) {
        Connection.Connected -> Reach.Connected
        Connection.Connecting -> Reach.Connecting
        Connection.Idle, Connection.Lost -> Reach.Unreachable
    }

    private fun stateOf(c: Client) =
        if (c.state.value == ConnectionState.Connected) Connection.Connected else Connection.Lost

    /** The app is (re)connecting: shown until [use] gets the client. */
    fun connecting() {
        scope.launch {
            connection = Connection.Connecting
            emit()
        }
    }

    /**
     * Feeds the session from a connected client; the previous one is dropped.
     * Returns once the first snapshot is in.
     */
    suspend fun use(c: Client) = withContext(dispatcher) {
        val mine = ++generation
        sub?.close()
        sub = null
        watching?.cancel()
        client = c
        connection = stateOf(c)
        lostReason = c.lostReason
        watching = scope.launch {
            c.state.collect {
                if (mine != generation) return@collect
                connection = stateOf(c)
                lostReason = c.lostReason
                emit()
            }
        }
        emit()
        follow(c, mine)
        readMachines(c)
        readDocs(c)
    }

    /** Lists the machines again now (one was just dialed). */
    fun refreshMachines() {
        scope.launch { client?.let { readMachines(it) } }
    }

    /** What the person is looking at now: those are seen, and finish as seen. */
    fun setVisible(addresses: Collection<String>) {
        val set = addresses.toSet()
        scope.launch {
            visible = set
            seen(set)
        }
    }

    fun markSeen(addresses: Collection<String>) {
        val list = addresses.toList()
        scope.launch { seen(list) }
    }

    private fun seen(addresses: Collection<String>) {
        var changed = false
        for (a in addresses) {
            val agent = agents[a] ?: continue
            if (!agent.seen) {
                agents[a] = agent.copy(seen = true)
                changed = true
            }
        }
        if (changed) emit()
    }

    fun dispose() {
        scope.launch {
            generation++
            watching?.cancel()
            sub?.close()
            client?.close()
        }
    }
}

/**
 * An unnamed space is named like the desktop names it: the `where` label the
 * desktop wrote on its terminals (`<space> / <tab>`), else its folder.
 */
private fun spaceName(s: Space, terminals: List<Terminal>): String {
    for (tab in s.tabs) {
        val suffix = " / ${tab.name}"
        val where = terminals.firstOrNull { it.tab == tab.id && it.where?.endsWith(suffix) == true }?.where
        if (where != null) return where.dropLast(suffix.length)
    }
    return folderName(s.folder)
}
