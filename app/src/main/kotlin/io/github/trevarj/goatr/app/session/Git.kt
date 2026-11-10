// Each space's git, for the spaces lists (herdr-app mods/spaces): looked up
// where a space is now (its first terminal's folder), on its machine. A new
// place (a new space, a cd) is looked at right away, every place again after
// FALLBACK_MS, and only while a list is on screen to show it. What's known
// outlives the list (the app sent to the background and back), and a host
// that connects has its places looked at again.

package io.github.trevarj.goatr.app.session

import herdr.core.PlaceGit
import herdr.core.place
import herdr.core.placeGit
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

/** Changes made elsewhere (a commit, a checkout) are looked for this often (spaces/main.js `FALLBACK`). */
private const val FALLBACK_MS = 60_000L

/** What git said about a place: null, outside git. */
private class Answer(val git: PlaceGit?)

/** Where a space is: a folder on a host's machine. */
private data class Place(val host: String, val machine: String?, val folder: String)

class SpaceGit(private val live: Live, scope: CoroutineScope, private val clock: () -> Long = System::currentTimeMillis) {
    private val places = live.snapshots.map { snaps ->
        snaps.flatMap { (host, s) ->
            s.spaces.mapNotNull { sp -> place(sp, s.terminals)?.let { ref(host, sp.id) to Place(host, sp.machine, it) } }
        }.toMap()
    }.distinctUntilChanged()

    // Only git's own "no" is kept as null; a place it couldn't answer for is asked again.
    private val known = ConcurrentHashMap<Place, Answer>()
    private val checked = ConcurrentHashMap<Place, Long>()

    private val ticks = flow {
        while (true) {
            emit(Unit)
            delay(FALLBACK_MS)
        }
    }

    /** Space id → its git; a space outside git, or git hasn't answered for yet, isn't in it. */
    val bySpace: StateFlow<Map<String, PlaceGit>> = channelFlow {
        // A host that (re)connects answers what it couldn't.
        combine(places, live.clients, ticks) { p, c, _ -> p to c }.collectLatest { (p, clients) ->
            fun shown() = p.mapNotNull { (id, at) -> known[at]?.git?.let { id to it } }.toMap()
            send(shown())
            val now = clock()
            val due = p.values.toSet().filter { at -> checked[at]?.let { now - it >= FALLBACK_MS } ?: true }
            coroutineScope {
                for (at in due) launch {
                    val client = clients[at.host] ?: return@launch
                    known[at] = try {
                        Answer(placeGit(client, at.machine, at.folder))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        return@launch
                    }
                    checked[at] = clock()
                    send(shown())
                }
            }
        }
    }.distinctUntilChanged().stateIn(scope, SharingStarted.WhileSubscribed(5_000), emptyMap())
}
