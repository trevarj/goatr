// The app's dependencies, made once and handed to ViewModels by constructor
// (manual dependency injection: an app this size needs no framework).

package io.github.trevarj.goatr.app

import android.content.Context
import android.os.Build
import io.github.trevarj.goatr.app.notify.HerdrService
import io.github.trevarj.goatr.app.notify.Notifier
import io.github.trevarj.goatr.app.session.Actions
import io.github.trevarj.goatr.app.session.Live
import io.github.trevarj.goatr.app.session.LocalNetwork
import io.github.trevarj.goatr.app.session.SpaceGit
import io.github.trevarj.goatr.app.session.Store
import dev.herdr.ssh.PhoneKey
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The terminal last opened: the Terminal tab shows it, the lists mark it.
 * Kept in the prefs, so the next start opens it again.
 */
class Selection(private val store: Store, private val scope: CoroutineScope) {
    private val _terminal = MutableStateFlow<String?>(null)
    val terminal: StateFlow<String?> = _terminal

    init {
        // Unless a link already picked one while the prefs were loading.
        scope.launch {
            store.loaded.first { it }
            if (_terminal.value == null) _terminal.value = store.prefs.value.terminal
        }
    }

    fun select(address: String) {
        if (_terminal.value == address) return
        _terminal.value = address
        scope.launch { store.setPrefs { it.copy(terminal = address) } }
    }
}

class AppContainer(context: Context) {
    private val scope = MainScope()
    val store = Store(context, scope)
    // The SSH key lives in :ssh; the app only ever gets its public half.
    val key = PhoneKey(context, comment = "goatr@" + (Build.MODEL ?: "phone").replace(Regex("""\s+"""), "-"))
    val localNetwork = LocalNetwork(context)
    val iroh = io.github.trevarj.goatr.GoatrConnection(context, store)
    val live = Live(store, key, localNetwork, scope, iroh)
    val actions = Actions(live)
    val selection = Selection(store, scope)
    val git = SpaceGit(live, scope)

    /** Past the opening screen: once per run, not again each time the activity is made. */
    val started = MutableStateFlow(false)
    val notifier = Notifier(context, live, store, scope)

    init {
        // Opted in, on screen, with a host to reach: keep it reached once out of
        // sight too. (A foreground service may only start while the app is in front.)
        scope.launch {
            combine(notifier.foreground, store.hosts, store.prefs) { front, hosts, prefs ->
                prefs.notifications && front && hosts.any { !it.off }
            }
                .distinctUntilChanged()
                .collect { if (it) HerdrService.start(context) }
        }
    }
}
