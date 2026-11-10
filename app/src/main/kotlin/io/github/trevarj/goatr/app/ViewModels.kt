// Screen state: one ViewModel per screen, built from the AppContainer.
// Dialogs (confirm a close, show what failed) are state here, drawn by the
// screen; opening a terminal is an event the screen hands to navigation.

package io.github.trevarj.goatr.app

import androidx.compose.runtime.Composable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.CreationExtras
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import io.github.trevarj.goatr.app.notify.Notifier
import io.github.trevarj.goatr.app.session.Actions
import io.github.trevarj.goatr.app.session.Host
import io.github.trevarj.goatr.app.session.HostState
import io.github.trevarj.goatr.app.session.Live
import io.github.trevarj.goatr.app.session.LocalNetwork
import io.github.trevarj.goatr.app.session.Machine
import io.github.trevarj.goatr.app.session.Prefs
import io.github.trevarj.goatr.app.session.Session
import io.github.trevarj.goatr.app.session.Space
import io.github.trevarj.goatr.app.session.SpaceGit
import io.github.trevarj.goatr.app.session.Store
import io.github.trevarj.goatr.app.session.hostOf
import io.github.trevarj.goatr.app.session.localOf
import io.github.trevarj.goatr.app.session.parseTarget
import io.github.trevarj.goatr.app.session.reason
import dev.herdr.ssh.PhoneKey
import dev.herdr.ssh.Seen
import herdr.core.Checkout
import herdr.core.Choice
import herdr.core.Client
import herdr.core.PlaceGit
import herdr.core.Repo
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** A dialog the screen shows: a confirmation (with [confirm]) or a notice. */
data class DialogState(
    val title: String,
    val detail: String? = null,
    val confirm: String? = null,
    val destructive: Boolean = false,
    val onConfirm: () -> Unit = {},
)

class AgentsViewModel(live: Live, actions: Actions, store: Store, selection: Selection) :
    LayoutViewModel(live, actions, store, selection) {
    /** To the top of the agent lists, on every screen; or back. */
    fun pin(terminal: String, pinned: Boolean) = attempt(if (pinned) "Could not pin it" else "Could not unpin it") {
        actions.pin(terminal, pinned)
    }
}

/** New, rename, move and close for spaces and tabs, with their dialogs. */
abstract class LayoutViewModel(
    protected val live: Live,
    protected val actions: Actions,
    private val store: Store,
    selection: Selection,
) : ViewModel() {
    val session: StateFlow<Session> = live.session
    val current: StateFlow<String?> = selection.terminal

    private val _dialog = MutableStateFlow<DialogState?>(null)
    val dialog: StateFlow<DialogState?> = _dialog

    private val _opened = Channel<String>(Channel.BUFFERED)

    /** Terminals made here, to open. */
    val opened: Flow<String> = _opened.receiveAsFlow()

    fun dismiss() {
        _dialog.value = null
    }

    protected fun attempt(what: String, block: suspend () -> Unit) {
        viewModelScope.launch {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _dialog.value = DialogState(what, e.reason)
            }
        }
    }

    private fun confirm(space: String, tab: String?, run: () -> Unit) {
        if (!store.prefs.value.confirmClose) return run()
        val p = actions.closePrompt(space, tab)
        _dialog.value = DialogState(p.title, p.detail, p.confirm, destructive = true) {
            _dialog.value = null
            run()
        }
    }

    suspend fun choices(host: String, machine: String?): List<Choice> = actions.choices(host, machine)

    /** Dials a machine now: one the host's core reaches, through it; else the host's own link. */
    fun reconnect(m: Machine) {
        val client = live.clientNow(m.host)
        if (m.name == null || client == null) return live.reconnect(m.host)
        attempt("Could not reach ${m.label}") {
            client.call("machine.connect", buildJsonObject { put("machine", m.name) })
            live.core(m.host)?.refreshMachines()
        }
    }

    /** `shown`: the terminal on screen, whose folder the tab starts in. */
    fun newTab(machine: String?, space: String, after: String?, shown: String?, c: Choice) = attempt("Could not open a tab") {
        _opened.send(actions.newTab(machine, space, after, shown, c).address)
    }

    /** Starts in the current terminal's folder, if it's on that machine. */
    fun newSpace(host: String, machine: String?, c: Choice) = attempt("Could not make a space") {
        _opened.send(actions.newSpace(host, machine, current.value, c).address)
    }

    fun closeTab(machine: String?, space: String, tab: String) = confirm(space, tab) {
        attempt("Could not close the tab") { actions.closeTab(machine, tab) }
    }

    fun closeSpace(machine: String?, space: String) = confirm(space, null) {
        attempt("Could not close the space") { actions.closeSpace(machine, space) }
    }

    fun renameSpace(machine: String?, space: String, name: String) = attempt("Could not rename the space") {
        actions.renameSpace(machine, space, name)
    }

    fun renameTab(machine: String?, tab: String, name: String) = attempt("Could not rename the tab") {
        actions.renameTab(machine, tab, name)
    }

    fun moveSpace(machine: String?, space: String, index: Int) = attempt("Could not move the space") {
        actions.moveSpace(machine, space, index)
    }

    // Worktrees: the dialogs show what goes wrong themselves, so these throw.

    suspend fun repoOf(space: String) = actions.repoOf(space)

    suspend fun home(space: Space) = actions.home(space.host, space.machine)

    suspend fun checkouts(space: Space, repo: Repo) = actions.checkouts(space.host, space.machine, repo)

    suspend fun newWorktree(space: Space, repo: Repo, branch: String) {
        _opened.send(actions.newWorktree(space.host, space.machine, repo, branch).address)
    }

    suspend fun openCheckout(space: Space, checkout: Checkout, all: List<Checkout>) {
        actions.openCheckout(space.host, space.machine, checkout, all)?.let { _opened.send(it) }
    }

    suspend fun removeRisk(space: Space, repo: Repo) = actions.removeRisk(space.host, space.machine, repo)

    /**
     * Here, not in the dialog: closing the space first takes its row, the
     * dialog and their scope with it, before git would remove the checkout.
     */
    fun removeWorktree(space: Space, repo: Repo, force: Boolean) = attempt("Could not remove the worktree") {
        actions.removeWorktree(space.machine, space.id, repo, force)
    }
}

class SpacesViewModel(live: Live, actions: Actions, store: Store, selection: Selection, git: SpaceGit) :
    LayoutViewModel(live, actions, store, selection) {
    /** Each space's git, to nest worktrees under their repo and say where it is. */
    val git: StateFlow<Map<String, PlaceGit>> = git.bySpace
}

class TerminalViewModel(live: Live, actions: Actions, store: Store, private val selection: Selection) :
    LayoutViewModel(live, actions, store, selection) {
    fun select(address: String) = selection.select(address)

    /** Every host, with its link's state: the screen while they connect. */
    val hosts: StateFlow<List<Host>> = store.hosts
    val hostStates: StateFlow<Map<String, HostState>> = live.hostStates

    fun reconnect(id: String) = live.reconnect(id)

    /** What's on screen counts as seen: a done agent stops asking for attention. */
    fun setVisible(addresses: List<String>) = live.setVisible(addresses)
}

class SettingsViewModel(
    private val live: Live,
    private val store: Store,
    private val notifier: Notifier,
    private val key: PhoneKey,
    localNetwork: LocalNetwork,
) : ViewModel() {
    val session: StateFlow<Session> = live.session
    val localNetworkAllowed: StateFlow<Boolean> = localNetwork.allowed
    val hosts: StateFlow<List<Host>> = store.hosts
    val prefs: StateFlow<Prefs> = store.prefs
    val hostStates: StateFlow<Map<String, HostState>> = live.hostStates

    /** The phone's public key, null until it's loaded (or made). */
    val publicKey: StateFlow<String?> = key.public

    init {
        viewModelScope.launch { runCatching { key.publicKey() } }
    }

    /** The private key, to show: the screen asks for the device's lock first. */
    suspend fun privateKey(): String = key.privateKey()

    /** A new key in place of this one: then every host logs in with it. */
    suspend fun regenerateKey() {
        key.regenerate()
        live.reconnectAll()
    }

    /** Adds `user@host[:port]` (it connects once added); false when it isn't one. */
    fun addHost(target: String): Boolean {
        val parsed = parseTarget(target) ?: return false
        viewModelScope.launch { store.addHost(parsed) }
        return true
    }

    fun setOn(id: String, on: Boolean) {
        viewModelScope.launch { live.setOn(id, on) }
    }

    fun reconnect(id: String) = live.reconnect(id)

    fun removeHost(id: String) {
        viewModelScope.launch { live.removeHost(id) }
    }

    suspend fun installKey(id: String, password: String) = live.installKey(id, password)

    suspend fun trust(id: String, seen: Seen) = live.trust(id, seen)

    fun setPrefs(patch: (Prefs) -> Prefs) {
        viewModelScope.launch { store.setPrefs(patch) }
    }

    /** Whether Android lets the app notify (asked for when notifications are switched on). */
    fun notificationsAllowed() = notifier.allowed()
}

/** The activity's own: the theme, the opening screen, and the terminal links and taps pick. */
class MainViewModel(
    private val live: Live,
    private val store: Store,
    private val selection: Selection,
    private val startedState: MutableStateFlow<Boolean>,
    private val localNetwork: LocalNetwork,
) : ViewModel() {
    val prefs: StateFlow<Prefs> = store.prefs
    val loaded: StateFlow<Boolean> = store.loaded
    val session: StateFlow<Session> = live.session
    val hosts: StateFlow<List<Host>?> = kotlinx.coroutines.flow.combine(store.savedHosts, store.prefs) { hosts, prefs ->
        hosts?.let { io.github.trevarj.goatr.app.session.activeHosts(it, prefs.desktop) }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, null)
    val hostStates: StateFlow<Map<String, HostState>> = live.hostStates

    /** Past the opening screen: once per run, not again each time the activity is made. */
    val started: StateFlow<Boolean> = startedState

    fun start() {
        startedState.value = true
    }

    fun select(address: String) = selection.select(address)

    /** A host's client while it's connected, for the live terminal. */
    fun client(host: String): Flow<Client?> = live.client(host)

    /** A terminal's size (cols, rows) as its core last reported it, if it has. */
    fun sizeOf(address: String): Pair<Int, Int>? =
        live.core(hostOf(address))?.snapshot?.value?.terminals?.find { it.address == localOf(address) }?.record
            ?.takeIf { it.cols > 0 && it.rows > 0 }?.let { it.cols to it.rows }

    fun reconnect(id: String) = live.reconnect(id)

    /** Android lets the app reach the local network (always, before Android 17). */
    val localNetworkAllowed: StateFlow<Boolean> = localNetwork.allowed

    fun refreshLocalNetwork() = localNetwork.refresh()

    fun toggleSidebar() {
        viewModelScope.launch { store.setPrefs { it.copy(sidebar = !it.sidebar) } }
    }
}

val AppViewModels: ViewModelProvider.Factory = viewModelFactory {
    fun container(extras: CreationExtras) = (extras[APPLICATION_KEY] as HerdrApp).container
    initializer { container(this).run { MainViewModel(live, store, selection, started, localNetwork) } }
    initializer { container(this).run { AgentsViewModel(live, actions, store, selection) } }
    initializer { container(this).run { SpacesViewModel(live, actions, store, selection, git) } }
    initializer { container(this).run { TerminalViewModel(live, actions, store, selection) } }
    initializer { container(this).run { SettingsViewModel(live, store, notifier, key, localNetwork) } }
}

/** The screen's ViewModel, from the app's container. */
@Composable
inline fun <reified VM : ViewModel> appViewModel(): VM = viewModel(factory = AppViewModels)
