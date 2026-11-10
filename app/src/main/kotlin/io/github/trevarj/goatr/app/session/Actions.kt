// New and close, as the desktop does them (herdr.core's Layout.kt writes the
// desktop's layout document), on the host the space or tab is on. No UI here:
// the screens ask before closing (with [closePrompt]) and show what failed.

package io.github.trevarj.goatr.app.session

import herdr.core.addWorktree
import herdr.core.Checkout
import herdr.core.Choice
import herdr.core.Client
import herdr.core.confirmClose
import herdr.core.Created
import herdr.core.homeOf
import herdr.core.place
import herdr.core.RemoveRisk
import herdr.core.Repo
import herdr.core.spaceFolder
import herdr.core.spaceOf
import herdr.core.tabFolder
import kotlinx.coroutines.CancellationException

/** What closing asks before it happens. */
data class Prompt(val title: String, val detail: String, val confirm: String)

class Actions(private val live: Live) {
    private fun client(host: String): Client = live.clientNow(host) ?: throw IllegalStateException("Not connected to ${host.substringAfter('@').substringBefore(':')}")

    private fun liveRecords(host: String) = live.core(host)?.snapshot?.value?.terminals.orEmpty().map { it.record }

    // What the core made, in app-wide ids.
    private fun Created.on(host: String) = Created(ref(host, address), ref(host, space), ref(host, tab))

    /**
     * What closing ends: the space as the desktop words it, or the tab and the
     * programs in it (a tab close kills them, so it asks too).
     */
    fun closePrompt(space: String, tab: String?): Prompt {
        val host = hostOf(space)
        val model = live.core(host)?.snapshot?.value?.spaces?.find { it.id == localOf(space) }
        val whole = model?.let { confirmClose(it, tab?.let(::localOf)) }
        if (whole != null || model == null || tab == null) {
            return Prompt(whole?.title ?: "Close space?", whole?.detail ?: "", "Close space")
        }
        val i = model.tabs.indexOfFirst { it.id == localOf(tab) }
        val names = live.session.value.terminals.associate { it.address to it.name }
        val shut = model.tabs.getOrNull(i)
        val programs = shut?.terminals.orEmpty().mapNotNull { names[ref(host, it)] }
        val name = shut?.name?.ifEmpty { null } ?: "Tab ${i + 1}"
        val ends = if (programs.isEmpty()) "" else " — ends ${programs.joinToString(", ")}"
        return Prompt("Close tab?", "$name in ${model.name}$ends", "Close tab")
    }

    // The terminal shown (an app-wide address), as its host's core has it.
    private fun shownOn(host: String, shown: String?) =
        shown?.takeIf { hostOf(it) == host }?.let { s -> live.core(host)?.snapshot?.value?.terminals?.find { it.address == localOf(s) } }

    suspend fun choices(host: String, machine: String?): List<Choice> = herdr.core.templates(client(host), machine)

    /** Starts where `shown` is, if it's in this space (the desktop's "follow"). */
    suspend fun newTab(machine: String?, space: String, after: String?, shown: String?, c: Choice): Created {
        val host = hostOf(space)
        val folder = tabFolder(shownOn(host, shown), localOf(space))
        return herdr.core.newTab(client(host), machine, localOf(space), name = c.tab, folder = folder, after = after?.let(::localOf), run = c.run).on(host)
    }

    /** Starts where `shown` is, if it's on this machine. */
    suspend fun newSpace(host: String, machine: String?, shown: String?, c: Choice): Created =
        herdr.core.newSpace(client(host), machine, folder = spaceFolder(shownOn(host, shown), machine), tab = c.tab, run = c.run).on(host)

    suspend fun closeTab(machine: String?, tab: String) {
        val host = hostOf(tab)
        herdr.core.closeTab(client(host), machine, localOf(tab), liveRecords(host))
    }

    suspend fun closeSpace(machine: String?, space: String) {
        val host = hostOf(space)
        herdr.core.closeSpace(client(host), machine, localOf(space), liveRecords(host))
    }

    /** "": named after its folder again. */
    suspend fun renameSpace(machine: String?, space: String, name: String) =
        herdr.core.renameSpace(client(hostOf(space)), machine, localOf(space), name)

    /** "": its position again. */
    suspend fun renameTab(machine: String?, tab: String, name: String) =
        herdr.core.renameTab(client(hostOf(tab)), machine, localOf(tab), name)

    /** To `index` among its machine's spaces. */
    suspend fun moveSpace(machine: String?, space: String, index: Int) =
        herdr.core.moveSpace(client(hostOf(space)), machine, localOf(space), index)

    suspend fun pin(terminal: String, pinned: Boolean) =
        herdr.core.pin(client(hostOf(terminal)), localOf(terminal), pinned)

    // Worktrees (herdr-app mods/worktree), for a space's checkout.

    private fun snapshot(host: String) = live.core(host)?.snapshot?.value

    /** The checkout a space is in, if it's git: null too when the machine can't tell (as the desktop's menu). */
    suspend fun repoOf(space: String): Repo? {
        val host = hostOf(space)
        val s = snapshot(host) ?: return null
        val sp = s.spaces.find { it.id == localOf(space) } ?: return null
        val folder = place(sp, s.terminals) ?: return null
        return try {
            herdr.core.repoOf(client(host), sp.machine, folder)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
    }

    suspend fun home(host: String, machine: String?): String = homeOf(client(host), machine)

    suspend fun checkouts(host: String, machine: String?, repo: Repo): List<Checkout> =
        herdr.core.checkouts(client(host), machine, repo)

    /** A new branch's checkout, in a new space there (left unnamed, so it follows its folder). */
    suspend fun newWorktree(host: String, machine: String?, repo: Repo, branch: String): Created {
        val c = client(host)
        val path = addWorktree(c, machine, repo, branch)
        return herdr.core.newSpace(c, machine, folder = path).on(host)
    }

    /** The space showing a checkout, else a new one there: what to open. */
    suspend fun openCheckout(host: String, machine: String?, checkout: Checkout, all: List<Checkout>): String? {
        val s = snapshot(host)
        val there = s?.let { spaceOf(checkout.path, all, machine, it.spaces, it.terminals) }
        if (there != null) return there.tabs.firstOrNull()?.terminals?.firstOrNull()?.let { ref(host, it) }
        return herdr.core.newSpace(client(host), machine, folder = checkout.path).on(host).address
    }

    suspend fun removeRisk(host: String, machine: String?, repo: Repo): RemoveRisk =
        herdr.core.removeRisk(client(host), machine, repo)

    /** Closes the space first (a shell in the folder keeps it busy), then removes its checkout. */
    suspend fun removeWorktree(machine: String?, space: String, repo: Repo, force: Boolean) {
        closeSpace(machine, space)
        herdr.core.removeWorktree(client(hostOf(space)), machine, repo, force)
    }
}
