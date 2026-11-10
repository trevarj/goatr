// The live session: one link per host (SSH, `hc connect`, its own herdr.core
// session), every host the phone has and hasn't switched off connected at
// once, merged into what the screens draw. Dropping is normal (sleep, network
// change): each link reconnects with backoff, and all of them at once when the
// app opens or comes back to the front.

package io.github.trevarj.goatr.app.session

import dev.herdr.ssh.PhoneKey
import dev.herdr.ssh.Refused
import dev.herdr.ssh.Seen
import dev.herdr.ssh.Ssh
import dev.herdr.ssh.Target
import herdr.core.Client
import herdr.core.ConnectionState
import herdr.core.MismatchError
import herdr.core.Reach
import herdr.core.SessionSnapshot
import herdr.core.connect
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import herdr.core.Connection as CoreConnection

/** A new host's key, fetched without logging in, for you to check before anything is sent to it. */
sealed interface Probe {
    data object Fetching : Probe
    data class Got(val seen: Seen) : Probe
    data class Failed(val message: String) : Probe
}

/** How one host's link is doing, for Settings. */
data class HostState(
    val connection: CoreConnection = CoreConnection.Idle,
    /** The last failure, until it connects. */
    val problem: Throwable? = null,
    /** Key refused: whether the host takes a password to install it (null while asking). */
    val password: Boolean? = null,
    val probe: Probe? = null,
    /** Its herdr-core's version, once connected. */
    val version: String? = null,
)

/** What went wrong, to show. */
val Throwable.reason: String get() = message ?: toString()

private val Host.target get() = Target(host, port, user)

/** App-wide ids: the host, then the core's own id (`dev@alpha.local:22 beta/t1`). */
fun ref(host: String, id: String) = "$host $id"
fun hostOf(ref: String) = ref.substringBefore(' ')
fun localOf(ref: String) = ref.substringAfter(' ')

private fun stateOf(reach: Reach) = when (reach) {
    Reach.Connected -> MachineState.Connected
    Reach.Connecting -> MachineState.Connecting
    Reach.NeedsLogin, Reach.NeedsUpdate, Reach.Unreachable -> MachineState.Lost
    Reach.Other -> MachineState.Asleep
}

/** What a host is called on screen: `alpha` for `dev@alpha.local`. */
fun shortName(host: Host) = host.host.removeSuffix(".local")

/**
 * Every host's snapshot as one session. A machine one host's core reaches
 * that the phone also connects to directly shows once: directly.
 */
internal fun merge(snaps: List<Pair<Host, SessionSnapshot>>, idle: Boolean): Session {
    val direct = snaps.filter { it.second.connection == CoreConnection.Connected }.mapNotNull { it.second.core?.name }.toSet()
    val several = snaps.size > 1
    fun shown(machine: String?) = machine == null || machine !in direct
    val states = snaps.map { it.second.connection }
    return Session(
        host = when {
            snaps.isEmpty() -> ""
            snaps.size == 1 -> snaps[0].second.core?.name ?: shortName(snaps[0].first)
            else -> "${states.count { it == CoreConnection.Connected }}/${snaps.size} hosts"
        },
        connection = when {
            idle -> Connection.Idle
            states.isNotEmpty() && states.all { it == CoreConnection.Connected } -> Connection.Connected
            states.isEmpty() || states.any { it == CoreConnection.Connecting || it == CoreConnection.Idle } -> Connection.Connecting
            states.any { it == CoreConnection.Connected } -> Connection.Partial
            else -> Connection.Lost
        },
        machines = snaps.flatMap { (h, s) ->
            // Every machine is reached through its host's link: while that's down, so are they.
            val link = when (s.connection) {
                CoreConnection.Connected -> null
                CoreConnection.Lost -> MachineState.Lost
                else -> MachineState.Connecting
            }
            s.machines.filter { it.local || shown(it.name) }.map {
                Machine(
                    host = h.id,
                    name = if (it.local) null else it.name,
                    label = if (it.local) s.core?.name ?: shortName(h) else it.name,
                    state = link ?: if (it.local) MachineState.Connected else stateOf(it.state),
                    // While the host's link is down, that's the news, not the machine.
                    problem = it.reason.takeIf { _ -> link == null && !it.local },
                    since = it.sinceMs.takeIf { _ -> link == null && !it.local },
                )
            }
        },
        spaces = snaps.flatMap { (h, s) ->
            s.spaces.withIndex().filter { shown(it.value.machine) }.map { (i, sp) ->
                Space(
                    id = ref(h.id, sp.id),
                    host = h.id,
                    name = sp.name,
                    machine = sp.machine,
                    // The desktop's own choice lives in its app state, not the core.
                    avatar = i,
                    tabs = sp.tabs.map { Tab(ref(h.id, it.id), it.name, it.terminals.map { t -> ref(h.id, t) }) },
                    given = sp.given,
                    folder = sp.folder,
                )
            }
        },
        terminals = snaps.flatMap { (h, s) ->
            s.terminals.filter { shown(it.machine) }.map { t ->
                Terminal(
                    address = ref(h.id, t.address),
                    host = h.id,
                    machine = t.machine ?: if (several) s.core?.name ?: shortName(h) else null,
                    name = t.agent ?: t.name ?: t.program,
                    title = t.detail,
                    agent = t.agent != null,
                    program = t.shown,
                    attention = t.attention,
                    since = t.statusMs,
                    space = t.space?.let { ref(h.id, it) },
                    tab = t.tab?.let { ref(h.id, it) },
                    pinned = t.pinned,
                )
            }
        },
    )
}

/** One host: its SSH connection, its client and its core session, retried with backoff. */
private class Link(
    val id: String,
    private val store: Store,
    private val key: PhoneKey,
    private val localNetwork: StateFlow<Boolean>,
    private val scope: CoroutineScope,
    private val iroh: io.github.trevarj.goatr.GoatrConnection,
) {
    val core = herdr.core.Session(scope)
    val client = MutableStateFlow<Client?>(null)
    val problem = MutableStateFlow<Throwable?>(null)
    val probe = MutableStateFlow<Probe?>(null)
    val password = MutableStateFlow<Boolean?>(null)

    private var attempt = 0
    private var job: Job? = null
    private var opening = false
    private var closed = false
    private var closeCurrent: (() -> Unit)? = null

    private fun host() = store.hosts.value.find { it.id == id }

    /** Connects now, dropping the connection in hand. */
    fun reconnect() {
        if (closed) return
        job?.cancel()
        closeCurrent?.invoke()
        closeCurrent = null
        client.value = null
        problem.value = null
        // Asked again if the key's still refused: the host's setup may have changed.
        password.value = null
        restart()
    }

    /**
     * The app is in front again: a link that's down tries now, unless it
     * waits on you (a new host's key to trust, a key to install, a changed
     * host key): trying again would only spend a login attempt.
     */
    fun wake() {
        if (closed || opening || client.value != null) return
        val p = problem.value
        if (probe.value is Probe.Got || p is Refused || p is MismatchError) return
        restart()
    }

    private fun restart() {
        attempt = 0
        job?.cancel()
        job = scope.launch { start() }
    }

    fun close() {
        closed = true
        job?.cancel()
        closeCurrent?.invoke()
        client.value = null
        core.dispose()
    }

    private suspend fun start() {
        val host = host() ?: return
        // Blocked by Android: waits for the permission (Live wakes it), no retries.
        if (host.peer == null && !localNetwork.value && isLocal(host.host)) {
            problem.value = LocalNetworkBlocked()
            return
        }
        // Allowed now; a new host's key fetch doesn't clear problems itself.
        if (problem.value is LocalNetworkBlocked) problem.value = null
        if (host.peer == null && host.hostKey == null) return probe(host)
        opening = true
        try {
            open(host)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            problem.value = e
            // A refused key, a changed host key or a core to update won't fix itself by retrying.
            when (e) {
                is Refused.Key -> askPassword(host)
                is Refused.HostKeyChanged, is MismatchError -> {}
                else -> retry()
            }
        } finally {
            opening = false
        }
    }

    // Whether the key can be installed with a password from here, or by hand only.
    private suspend fun askPassword(host: Host) {
        if (password.value != null) return
        password.value = runCatching { Ssh.takesPassword(host.target, checkNotNull(host.hostKey)) }
            .getOrDefault(false)
    }

    private suspend fun open(host: Host) {
        core.connecting()
        val duplex = host.peer?.let { iroh.open(it) } ?: Ssh.open(host.target, key, checkNotNull(host.hostKey))
        closeCurrent = { duplex.close() }
        if (closed) return duplex.close()
        val next = connect(duplex)
        scope.launch {
            next.state.first { it == ConnectionState.Lost }
            if (client.value === next) client.value = null
            if (!closed) retry()
        }
        core.use(next)
        client.value = next
        attempt = 0
        problem.value = null
    }

    private fun retry() {
        job?.cancel()
        // 1s, 2s, 4s … up to 30s; coming back to the app tries at once.
        val wait = minOf(30_000L, 1000L shl minOf(attempt++, 15))
        job = scope.launch {
            delay(wait)
            start()
        }
    }

    private suspend fun probe(host: Host) {
        if (probe.value is Probe.Got) return
        probe.value = Probe.Fetching
        probe.value = try {
            Probe.Got(Ssh.probe(host.target))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Probe.Failed(e.reason)
        }
    }
}

internal fun activeHosts(hosts: List<Host>, desktop: String?) =
    hosts.filter { !it.off && (it.peer == null || it.paired && it.peer == desktop) }

@OptIn(ExperimentalCoroutinesApi::class)
class Live(
    private val store: Store,
    private val key: PhoneKey,
    private val localNetwork: LocalNetwork,
    private val scope: CoroutineScope,
    private val iroh: io.github.trevarj.goatr.GoatrConnection,
) {
    private val links = MutableStateFlow<Map<String, Link>>(emptyMap())

    init {
        // Allowed at last: the hosts it blocked connect now.
        scope.launch { localNetwork.allowed.collect { if (it) foreground() } }
        // A link per host that's on, made (and connecting) as hosts are added or switched on.
        scope.launch {
            combine(store.hosts, store.prefs) { hosts, prefs -> hosts to prefs.desktop }.collect { (hosts, desktop) ->
                val had = links.value
                val want = activeHosts(hosts, desktop)
                for ((id, l) in had) if (want.none { it.id == id }) l.close()
                links.value = want.associate { h -> h.id to (had[h.id] ?: Link(h.id, store, key, localNetwork.allowed, scope, iroh).also { it.reconnect() }) }
            }
        }
    }

    // Each link's `each`, as (host id, value) pairs, following hosts coming and going.
    private fun <T> perLink(each: (Link) -> Flow<T>): Flow<List<Pair<String, T>>> = links.flatMapLatest { ls ->
        if (ls.isEmpty()) flowOf(emptyList())
        else combine(ls.values.map { l -> each(l).map { l.id to it } }) { it.toList() }
    }

    /** What the screens draw. */
    // A link that failed (and waits to retry, or on you) counts as lost, not connecting.
    val session: StateFlow<Session> = combine(
        perLink { l ->
            combine(l.core.snapshot, l.problem) { s, p ->
                if (p != null && s.connection != CoreConnection.Connected) s.copy(connection = CoreConnection.Lost) else s
            }
        },
        combine(store.hosts, store.prefs) { hosts, prefs -> activeHosts(hosts, prefs.desktop) },
        store.loaded,
    ) { snaps, hosts, loaded ->
        val byId = hosts.associateBy { it.id }
        merge(snaps.mapNotNull { (id, s) -> byId[id]?.let { it to s } }, idle = loaded && hosts.none { !it.off })
    }.stateIn(scope, SharingStarted.Eagerly, Session())

    /** Each host's link, by id, for Settings. */
    val hostStates: StateFlow<Map<String, HostState>> = perLink { l ->
        combine(l.core.snapshot, l.problem, l.probe, l.password) { s, p, pr, pw -> HostState(s.connection, p, password = pw, probe = pr, version = s.core?.version) }
    }.map { it.toMap() }.stateIn(scope, SharingStarted.Eagerly, emptyMap())

    /** The host's client while it's connected, for screens that call or attach. */
    fun client(host: String): Flow<Client?> = links.flatMapLatest { it[host]?.client ?: flowOf(null) }

    fun clientNow(host: String): Client? = links.value[host]?.client?.value

    /** Each host's client while it's connected, by host id: one (re)connecting is a new client. */
    val clients: Flow<Map<String, Client?>> = perLink { it.client }.map { it.toMap() }.distinctUntilChanged()

    fun core(host: String): herdr.core.Session? = links.value[host]?.core

    /** Makes the phone's key if there's none yet (the hosts connect as they load). */
    fun boot() {
        scope.launch { runCatching { key.publicKey() } }
    }

    /** The app opened or came back to the front: every host that's down tries now. */
    fun foreground() = links.value.values.forEach { it.wake() }

    /** The phone's key changed: every host logs in again with it. */
    fun reconnectAll() = links.value.values.forEach { it.reconnect() }

    /** Connects to `id` again now. */
    fun reconnect(id: String) {
        links.value[id]?.reconnect()
    }

    suspend fun setOn(id: String, on: Boolean) = store.updateHost(id) { it.copy(off = !on) }

    private suspend fun pin(id: String, seen: Seen) {
        store.updateHost(id) { it.copy(hostKey = seen.hostKey, fingerprint = seen.fingerprint) }
        links.value[id]?.probe?.value = null
    }

    /** You checked the new host's key and the phone's key is already on it: pin it and connect. */
    suspend fun trust(id: String, seen: Seen) {
        pin(id, seen)
        reconnect(id)
    }

    /**
     * ssh-copy-id with a password, then connect with the key. Only to the
     * host key you checked and pinned; the password goes nowhere else.
     */
    suspend fun installKey(id: String, password: String) {
        val host = store.hosts.value.find { it.id == id } ?: return
        val hostKey = host.hostKey ?: error("check the host's key first")
        Ssh.installKey(host.target, password, hostKey, authorizedLine(key.publicKey()))
        reconnect(id)
    }

    /** Each host's core session as it comes, by host id: what notifications watch. */
    val snapshots: Flow<List<Pair<String, SessionSnapshot>>> = perLink { it.core.snapshot }

    private val _visible = MutableStateFlow<Set<String>>(emptySet())

    /** The terminals on screen now (app-wide addresses). */
    val visible: StateFlow<Set<String>> = _visible

    /** What's on screen counts as seen: a done agent stops asking for attention. */
    fun setVisible(addresses: List<String>) {
        _visible.value = addresses.toSet()
        val byHost = addresses.groupBy(::hostOf)
        for ((id, l) in links.value) l.core.setVisible(byHost[id].orEmpty().map(::localOf))
    }

    /** Forgets a host, dropping its connection. */
    suspend fun removeHost(id: String) = store.removeHost(id)
}
