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
import io.github.trevarj.goatr.app.session.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.combine
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

class ReviewViewModel(private val reviews: Reviews) : ViewModel() {
    private val screen = MutableStateFlow(ReviewUiState())
    val state: StateFlow<ReviewUiState> = screen
    private var operation: Job? = null
    private var request = 0L
    private var requestedSpace: String? = null
    private var detached = false
    private var pinnedRepo: ReviewRepo? = null

    private var observation: Job? = null
    private fun observe() {
        if (observation != null) return
        observation = viewModelScope.launch {
            combine(reviews.history, reviews.metadataError, reviews.writes, reviews.connections) { _, _, _, _ -> Unit }
                .collect { indicators() }
        }
    }

    private fun indicators() {
        val s = screen.value
        val space = s.spaceId ?: return
        val (current, working) = reviews.recipients(space)
        val missing = s.recipients.filter { old -> current.none { it.address == old.address } }.map {
            it.copy(rawStatus = if (reviews.connected(space)) "unavailable" else "disconnected", canSend = false)
        }
        val recipients = (current + missing).toMutableList()
        val r = s.record
        if (r?.recipientAddress != null && recipients.none { it.address == r.recipientAddress })
            recipients.add(ReviewRecipient(r.recipientAddress, r.recipientAgent.orEmpty(), r.recipientAddress, null, "unavailable", false))
        val write = reviews.writes.value[r?.id]
        screen.value = s.copy(connected = reviews.connected(space), recipients = recipients, workingAgents = working,
            history = reviews.history.value.orEmpty().filter { it.spaceId == space }.map { it.recovered() }.sortedByDescending { it.updatedAtMs },
            unreadableMetadata = reviews.metadataError.value == "Saved review metadata is unreadable.",
            saving = write != null && write.error == null, saveError = write?.error ?: s.saveError,
            error = reviews.metadataError.value ?: s.error)
    }

    private fun runOperation(submit: Boolean = false, resuming: Boolean = false, block: suspend () -> Unit) {
        if (screen.value.submitting && !resuming) return
        val previous = operation
        val myRequest = ++request
        previous?.cancel()
        screen.value = screen.value.copy(loading = !submit, submitting = submit, error = null)
        operation = viewModelScope.launch {
            try {
                previous?.join()
                block()
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (request == myRequest) screen.value = screen.value.copy(error = e.message ?: "Could not complete this review action.")
            } finally {
                if (request == myRequest) {
                    screen.value = screen.value.copy(loading = false, submitting = false)
                    requestedSpace = null
                    indicators()
                }
            }
        }
    }

    private suspend fun flush(): Boolean {
        val r = screen.value.record ?: return true
        if (r.phase == ReviewPhase.DeliveryUnknown && screen.value.saveError != null) return false
        val ok = reviews.flush(r.id)
        screen.value = screen.value.copy(saving = false, saveError = if (ok) null else reviews.writes.value[r.id]?.error ?: "Could not save the review.")
        return ok
    }
    suspend fun prepareToLeave(): Boolean {
        operation?.cancelAndJoin()
        return flush()
    }
    fun detach(spaceId: String) {
        if (spaceId != (requestedSpace ?: screen.value.spaceId)) return
        detached = true
        observation?.cancel()
        observation = null
        operation?.cancel()
        reviews.flushPending()
    }
    fun leaveWithoutSaving() {
        val r = screen.value.record
        val last = reviews.writes.value[r?.id]?.expected
        if (r != null) reviews.abandon(r)
        screen.value = screen.value.copy(record = last ?: r, saving = false, saveError = null)
        requestedSpace = null
    }

    private suspend fun display(r: ReviewRecord, content: ReviewContent? = null) {
        val restored = (reviews.writes.value[r.id]?.desired ?: r).recovered()
        pinnedRepo = restored.repo
        screen.value = screen.value.copy(record = restored, comparison = restored.comparison, snapshot = null, files = emptyList(),
            oldFiles = emptyMap(), saveError = null, error = if (restored.phase == ReviewPhase.DeliveryUnknown) "Delivery unknown. Open the agent before sending again." else null)
        val loaded = content ?: reviews.content(restored)
        currentCoroutineContext().ensureActive()
        screen.value = screen.value.copy(snapshot = loaded.snapshot, files = loaded.files, oldFiles = loaded.oldFiles)
        indicators()
    }

    fun open(spaceId: String) {
        if (!detached && (screen.value.submitting || requestedSpace == spaceId || screen.value.spaceId == spaceId && requestedSpace == null)) return
        val resuming = detached
        detached = false
        observe()
        requestedSpace = spaceId
        runOperation(resuming = resuming) {
            if (!flush()) return@runOperation
            require(' ' in spaceId && hostOf(spaceId).isNotEmpty() && localOf(spaceId).isNotEmpty()) { "Invalid original Space." }
            pinnedRepo = null
            screen.value = ReviewUiState(spaceId = spaceId, loading = true)
            indicators()
            val saved = reviews.records(spaceId).firstOrNull()
            if (saved != null) {
                display(saved)
                if (reviews.connected(spaceId)) {
                    val p = reviews.placeFor(spaceId)
                    if (saved.comparison.mode == ReviewMode.Branch) screen.value = screen.value.copy(branches = reviewBranches(p.client, p.machine, p.folder))
                    val fresh = captureReview(p.client, p.machine, p.folder, saved.comparison)
                    check(fresh.repo == saved.repo) { "This Space now points to another checkout. Start a new review to use it." }
                }
            } else captureDraft(spaceId, screen.value.comparison, null)
        }
    }

    private suspend fun captureDraft(space: String, wanted: ReviewComparison, pin: ReviewRepo?, requireNew: Boolean = false) {
        val p = reviews.placeFor(space)
        var cmp = wanted
        val branches = if (wanted.mode == ReviewMode.Branch) reviewBranches(p.client, p.machine, p.folder) else emptyList()
        if (wanted.mode == ReviewMode.Branch && branches.none { it.ref == wanted.baseRef }) {
            val last = reviews.records(space).firstOrNull { it.repo == pin && it.comparison.mode == ReviewMode.Branch &&
                branches.any { b -> b.ref == it.comparison.baseRef } }
            cmp = wanted.copy(baseRef = last?.comparison?.baseRef ?: branches.singleOrNull { it.suggested }?.ref)
        }
        if (cmp != screen.value.comparison) {
            screen.value = screen.value.copy(record = null, comparison = cmp, branches = branches, snapshot = null, files = emptyList(), oldFiles = emptyMap())
        } else screen.value = screen.value.copy(branches = branches)
        if (cmp.mode == ReviewMode.Branch && cmp.baseRef == null) return
        val s = captureReview(p.client, p.machine, p.folder, cmp)
        check(pin == null || pin == s.repo) { "The Space points to another checkout. Start a new review to use it." }
        currentCoroutineContext().ensureActive()
        val opened = reviews.create(space, s, requireNew = requireNew)
        currentCoroutineContext().ensureActive()
        screen.value = screen.value.copy(branches = branches)
        display(opened)
    }

    fun chooseComparison(comparison: ReviewComparison) {
        val s = screen.value
        val space = s.spaceId ?: return
        if (s.comparison == comparison && s.snapshot != null) return
        runOperation {
            if (!flush()) return@runOperation
            val saved = reviews.records(space).firstOrNull { it.phase == ReviewPhase.Draft && it.repo == pinnedRepo && it.comparison == comparison }
            if (!reviews.connected(space) && saved != null) display(saved)
            else captureDraft(space, comparison, pinnedRepo)
        }
    }
    fun refresh() {
        val s = screen.value
        val space = s.spaceId ?: return
        val r = s.record
        if (r?.composer != null) { screen.value = s.copy(error = "Save or discard the unfinished comment before refreshing."); return }
        if (r != null && r.phase != ReviewPhase.Draft) return
        runOperation {
            if (!flush()) return@runOperation
            if (r == null) { captureDraft(space, s.comparison, pinnedRepo); return@runOperation }
            val old = s.snapshot ?: error("Saved snapshot is unreadable. Delete the review to start again.")
            val p = reviews.placeFor(space)
            val fresh = captureReview(p.client, p.machine, p.folder, r.comparison)
            val files = if (fresh.id == old.id) s.files else withContext(Dispatchers.Default) { reviewFiles(fresh) }
            currentCoroutineContext().ensureActive()
            val updated = reviews.refresh(r, fresh, s.files, files)
            currentCoroutineContext().ensureActive()
            val oldFiles = (s.oldFiles + (old.id to s.files)).filterKeys { it != fresh.id && it in updated.snapshots() }
            display(updated, ReviewContent(fresh, files, oldFiles))
        }
    }

    private fun edit(immediate: Boolean = true, change: (ReviewRecord) -> ReviewRecord) {
        val s = screen.value; val r = s.record ?: return
        if (r.phase != ReviewPhase.Draft || s.loading || s.submitting || s.snapshot?.id != r.snapshotId) return
        try {
            val next = change(r)
            if (next == r) return
            val desired = next.copy(updatedAtMs = System.currentTimeMillis())
            reviews.queue(r, desired, immediate)
            screen.value = s.copy(record = desired, saving = true, error = null, saveError = null)
        } catch (e: IllegalStateException) { screen.value = s.copy(error = e.message) }
    }
    fun beginComment(anchor: ReviewAnchor, commentId: String? = null) = edit { r ->
        check(validReviewAnchor(anchor, r.snapshotId, screen.value.files)) { "Select a range within one displayed hunk." }
        val composer = r.composer
        if (composer != null) {
            check(composer.commentId == commentId && composer.anchor == anchor) { "Resume or discard the unfinished comment first." }
            r
        } else {
            val text = commentId?.let { id -> r.comments.singleOrNull { it.id == id }?.text ?: error("Comment no longer exists.") }.orEmpty()
            r.copy(composer = ReviewComposer(commentId, anchor, text))
        }
    }
    fun updateComment(text: String) = edit(false) { r -> r.copy(composer = r.composer?.copy(text = text)) }
    fun saveComment() = edit { it.saveReviewComment(screen.value.files) }
    fun dismissComposer(discard: Boolean = false) {
        if (discard) edit { it.copy(composer = null) }
        else reviews.flushPending()
    }
    fun removeComment(commentId: String) = edit { r ->
        check(r.composer?.commentId != commentId) { "Save or discard the unfinished edit first." }
        r.copy(comments = r.comments.filterNot { it.id == commentId })
    }
    fun updateMessage(text: String) = edit(false) { it.copy(message = text) }
    fun chooseRecipient(address: String) = edit { r ->
        val recipient = reviews.recipients(r.spaceId).first.singleOrNull { it.address == address && it.canSend } ?: error("This agent is not ready.")
        r.copy(recipientAddress = address, recipientAgent = recipient.agent)
    }
    fun chooseIntent(intent: ReviewIntent) = edit { it.copy(intent = intent) }

    fun showRecord(recordId: String) {
        val space = screen.value.spaceId ?: return
        runOperation {
            if (flush()) display(reviews.records(space).singleOrNull { it.id == recordId } ?: error("This review no longer exists."))
        }
    }
    fun newDraft(copyFromId: String? = null) {
        val s = screen.value; val space = s.spaceId ?: return
        runOperation {
            if (!flush()) return@runOperation
            if (copyFromId == null) captureDraft(space, s.comparison, null, requireNew = true)
            else {
                val source = reviews.records(space).singleOrNull { it.id == copyFromId }?.recovered() ?: error("The original review no longer exists.")
                check(source.phase != ReviewPhase.Draft) { "Open the existing draft instead." }
                val content = reviews.content(source)
                val draft = reviews.create(space, content.snapshot, source)
                display(draft, content)
            }
        }
    }
    fun deleteRecord(recordId: String) {
        val space = screen.value.spaceId ?: return
        runOperation {
            if (recordId != screen.value.record?.id && !flush()) return@runOperation
            reviews.delete(reviews.records(space).singleOrNull { it.id == recordId } ?: error("This review no longer exists."))
            if (screen.value.record?.id == recordId) {
                screen.value = ReviewUiState(spaceId = space, loading = true)
                val next = reviews.records(space).firstOrNull()
                if (next != null) display(next)
            }
        }
    }
    fun deleteUnreadableReviews() = runOperation {
        reviews.deleteUnreadable()
        screen.value = ReviewUiState(spaceId = screen.value.spaceId)
    }

    fun submit() {
        val s = screen.value; val r = s.record ?: return; val snapshot = s.snapshot ?: return
        if (s.submitting || s.loading || r.phase != ReviewPhase.Draft) return
        if (r.composer != null) { screen.value = s.copy(error = "Save or discard the unfinished comment before sending."); return }
        runOperation(submit = true) {
            if (!flush()) return@runOperation
            val saved = reviews.records(r.spaceId).singleOrNull { it.id == r.id }
            check(saved == r) { "The review changed. Reopen the latest draft." }
            try {
                val result = reviews.submit(r, snapshot, s.files)
                if (result.record.phase == ReviewPhase.Draft && result.error != null) {
                    val latest = reviews.records(r.spaceId).find { it.id == r.id }
                    if (latest != null && (latest != r || reviews.writes.value[r.id] != null)) display(latest)
                    screen.value = screen.value.copy(error = result.error, saveError = result.saveError)
                } else screen.value = screen.value.copy(record = result.record, error = result.error, saveError = result.saveError)
            } catch (e: CancellationException) {
                withContext(kotlinx.coroutines.NonCancellable) {
                    val last = runCatching { reviews.records(r.spaceId).find { it.id == r.id } }.getOrNull()
                    if (last != null && last.phase != ReviewPhase.Draft)
                        screen.value = screen.value.copy(record = last.recovered(),
                            saveError = if (last.phase == ReviewPhase.Sending) "Could not save the delivery outcome." else null)
                }
                throw e
            }
        }
    }
}

val AppViewModels: ViewModelProvider.Factory = viewModelFactory {
    fun container(extras: CreationExtras) = (extras[APPLICATION_KEY] as HerdrApp).container
    initializer { container(this).run { MainViewModel(live, store, selection, started, localNetwork) } }
    initializer { container(this).run { ReviewViewModel(reviews) } }
    initializer { container(this).run { AgentsViewModel(live, actions, store, selection) } }
    initializer { container(this).run { SpacesViewModel(live, actions, store, selection, git) } }
    initializer { container(this).run { TerminalViewModel(live, actions, store, selection) } }
    initializer { container(this).run { SettingsViewModel(live, store, notifier, key, localNetwork) } }
}

/** The screen's ViewModel, from the app's container. */
@Composable
inline fun <reified VM : ViewModel> appViewModel(): VM = viewModel(factory = AppViewModels)
