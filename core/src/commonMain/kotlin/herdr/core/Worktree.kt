// Git worktrees the way the desktop makes, lists and removes them
// (herdr-app/defaults/mods/worktree/main.js; the menu's `checkout()` in
// mods/menu/main.js): checkouts live in <home>/.herdr/worktrees/<repo>/<slug>.
// Git runs on the space's machine through `machine.exec`, as the desktop's
// `exec("git", args, {cwd, on})` does (mods.rs `run_remote`). Opening and
// closing the space is the caller's: [newSpace], [closeSpace].

package herdr.core

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64

// mods.rs `EXEC_LIMIT`: seconds a command may run.
private const val LIMIT = 30

/**
 * The repo a folder belongs to: `top` the checkout it's in, `main` the
 * repo's own checkout, `name` its folder's; `linked`: `top` is a worktree.
 */
data class Repo(val top: String, val main: String, val name: String, val linked: Boolean)

/** A checkout (`git worktree list`): `branch` null when detached; `at` its last commit, s since the epoch (0: unknown). */
data class Checkout(val path: String, val branch: String?, val main: Boolean, val at: Long)

/**
 * What removing a worktree loses: uncommitted changes, submodules with
 * commits that exist nowhere else (their names), and how many submodules
 * it has (git wants `--force` for any).
 */
data class RemoveRisk(val dirty: Boolean, val unpushed: List<String>, val modules: Int) {
    /** `git worktree remove --force` (main.js `remove`). */
    val force: Boolean get() = dirty || modules > 0
}

/**
 * Risk that appeared since [shown] was (main.js `remove`: "It changed since
 * this opened. Enter again removes it anyway.").
 */
fun changed(shown: RemoveRisk, now: RemoveRisk): Boolean =
    (now.dirty && !shown.dirty) || now.unpushed.any { it !in shown.unpushed }

/** What to show once [changed] warned: all risk seen so far, with `now`'s submodules. */
fun seen(shown: RemoveRisk, now: RemoveRisk) =
    RemoveRisk(shown.dirty || now.dirty, (shown.unpushed + now.unpushed).distinct(), now.modules)

/** A branch as a folder name (main.js `slug`). */
fun slug(branch: String): String =
    branch.replace(Regex("[^A-Za-z0-9._-]+"), "-").replace(Regex("^-+|-+$"), "")

/** Where a repo's checkouts go (main.js `ROOT`). */
fun worktreeRoot(home: String) = "$home/.herdr/worktrees"

/** Home as ~ (main.js `tilde`: git says C:/Users/x where Windows says C:\Users\x). */
fun tilde(path: String, home: String): String {
    val p = path.replace('\\', '/')
    val h = home.replace('\\', '/')
    return if (p.startsWith(h)) "~${p.substring(h.length)}" else path
}

/** How long ago `at` was, s since the epoch (main.js `ago`): 5m, 3h, 2d, 4mo. */
fun ago(at: Long, nowS: Long): String {
    val d = (nowS - at).toDouble()
    return when {
        d < 3600 -> "${maxOf(1L, kotlin.math.round(d / 60).toLong())}m"
        d < 86400 -> "${kotlin.math.round(d / 3600).toLong()}h"
        d < 86400 * 60 -> "${kotlin.math.round(d / 86400).toLong()}d"
        else -> "${kotlin.math.round(d / 86400 / 30).toLong()}mo"
    }
}

/** Where a space is now (main.js `place`): its first terminal's live folder, else where it was made. */
fun place(space: Space, terminals: List<Terminal>): String? {
    val by = terminals.associateBy { it.address }
    return space.tabs.flatMap { it.terminals }.firstNotNullOfOrNull { by[it]?.cwd?.ifEmpty { null } }
        ?: space.folder.ifEmpty { null }
}

/** The space showing a checkout (main.js `spaceOf`): its first terminal is in it, the deepest checkout winning. */
fun spaceOf(path: String, all: List<Checkout>, machine: String?, spaces: List<Space>, terminals: List<Terminal>): Space? {
    fun inside(p: String, dir: String) = p == dir || p.startsWith("$dir/")
    return spaces.find { s ->
        if (s.machine != machine) return@find false
        val at = place(s, terminals)?.replace('\\', '/')
        if (at == null || !inside(at, path)) return@find false
        all.none { it.path != path && it.path.length > path.length && inside(at, it.path) }
    }
}

/**
 * The home folder of `machine` (main.js `init`): `$HOME` from `sh`, else
 * PowerShell's (Windows), slashes forward.
 */
suspend fun homeOf(client: Client, machine: String?): String {
    var r = attempt { exec(client, machine, listOf("sh", "-c", "printf %s \"\$HOME\""), null) }
    if (r == null || r.code != 0 || r.out.isBlank()) {
        r = exec(client, machine, listOf("powershell.exe", "-NoProfile", "-Command", "[Console]::Write(\$HOME)"), null)
    }
    if (r.code != 0 || r.out.isBlank()) error("Could not find the remote home folder.")
    return r.out.trim().replace('\\', '/')
}

/**
 * The repo `folder` is in (main.js `repoOf`), null outside git. Throws when
 * git can't run there (no such folder, machine unreachable).
 */
suspend fun repoOf(client: Client, machine: String?, folder: String): Repo? {
    val top = git(client, machine, folder, "rev-parse", "--show-toplevel")
    val common = git(client, machine, folder, "rev-parse", "--path-format=absolute", "--git-common-dir")
    if (!top.ok || !common.ok) return null
    val main = common.out.removeSuffix("/.git")
    return Repo(top.out, main, main.substringAfterLast('/'), main != top.out)
}

/** Every checkout of a repo (main.js `checkouts`): the main one first, then the most recently committed. */
suspend fun checkouts(client: Client, machine: String?, repo: Repo): List<Checkout> {
    val list = git(client, machine, repo.main, "worktree", "list", "--porcelain")
    if (!list.ok) return emptyList()
    val refs = git(client, machine, repo.main, "for-each-ref", "--format=%(refname:short)%09%(committerdate:unix)", "refs/heads")
    return parseCheckouts(list.out, refs.out)
}

/** `git worktree list --porcelain` and `for-each-ref` (branch TAB time) read as [checkouts] does. */
internal fun parseCheckouts(list: String, refs: String): List<Checkout> {
    val times = refs.split("\n").associate { l ->
        val f = l.split("\t")
        f[0] to (f.getOrNull(1)?.toLongOrNull() ?: 0L)
    }
    val out = mutableListOf<Checkout>()
    for (block in list.split(Regex("\n\n+"))) {
        val lines = block.split("\n")
        val path = lines.find { it.startsWith("worktree ") }?.substring(9)
        if (path.isNullOrEmpty() || lines.any { it == "bare" || it.startsWith("prunable") }) continue
        val branch = lines.find { it.startsWith("branch ") }?.substring(7)?.removePrefix("refs/heads/")
        out += Checkout(path, branch, out.isEmpty(), branch?.let { times[it] } ?: 0L)
    }
    return out.sortedWith(compareByDescending<Checkout> { it.main }.thenByDescending { it.at })
}

/**
 * A checkout of `branch` in [worktreeRoot]/<repo>/<slug> (main.js `create`):
 * the branch if it exists, else a new one from `repo.top`'s HEAD. Its path;
 * throws with git's message.
 */
suspend fun addWorktree(client: Client, machine: String?, repo: Repo, branch: String): String {
    val b = branch.trim()
    if (b.isEmpty()) error("Type a branch name.")
    val path = "${worktreeRoot(homeOf(client, machine))}/${repo.name}/${slug(b)}"
    val exists = git(client, machine, repo.main, "show-ref", "--verify", "--quiet", "refs/heads/$b").ok
    val add = if (exists) listOf("worktree", "add", path, b) else listOf("worktree", "add", "-b", b, path, "HEAD")
    val r = git(client, machine, repo.top, *add.toTypedArray())
    if (!r.ok) error(r.err.ifEmpty { "git worktree add failed." })
    return path
}

/**
 * What removing `repo.top` loses (main.js `init` and `remove`). Read it to
 * show, and again just before removing: see [changed].
 */
suspend fun removeRisk(client: Client, machine: String?, repo: Repo): RemoveRisk {
    val status = git(client, machine, repo.top, "--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain")
    if (!status.ok) error(status.err.ifEmpty { "git status failed." })
    val (mods, unpushed) = submodules(client, machine, repo.top)
    return RemoveRisk(status.out.isNotEmpty(), unpushed, mods.size)
}

/**
 * `git worktree remove` of `repo.top` from the main checkout (main.js
 * `remove`); `force`: [RemoveRisk.force]. Close its space first: a shell in
 * the folder keeps it busy on Windows. Throws with git's message.
 */
suspend fun removeWorktree(client: Client, machine: String?, repo: Repo, force: Boolean) {
    val args = listOf("worktree", "remove") + (if (force) listOf("--force") else emptyList()) + repo.top
    val r = git(client, machine, repo.main, *args.toTypedArray())
    if (!r.ok) error(r.err.ifEmpty { "git worktree remove failed." })
}

// A submodule repo git keeps (main.js `modules`): `pinned` the commit its parent records ("" if none).
private data class Module(val name: String, val dir: String, val pinned: String)

/**
 * The submodule repos git keeps for a checkout, nested ones too, checked
 * out or not (main.js `modules`). `config`: the args that read its .gitmodules.
 */
private suspend fun modules(
    client: Client,
    machine: String?,
    gitDir: String,
    config: List<String>,
    out: MutableList<Module> = mutableListOf(),
    parent: String = "",
): List<Module> {
    val keys = git(client, machine, gitDir, "--git-dir", gitDir, "config", *config.toTypedArray(), "-z", "--get-regexp", "^submodule\\..*\\.path$")
    if (!keys.ok) return out // no .gitmodules, or no submodules in it
    // -z: "key\nvalue" records ended by NUL, so names and paths may hold spaces.
    for (record in keys.out.split("\u0000").filter { it.isNotEmpty() }) {
        val at = record.indexOf('\n')
        val name = record.substring("submodule.".length, at - ".path".length)
        val dir = "$gitDir/modules/$name"
        if (!git(client, machine, gitDir, "--git-dir", dir, "rev-parse", "--git-dir").ok) continue // never checked out
        val pinned = git(client, machine, gitDir, "--git-dir", gitDir, "rev-parse", "HEAD:${record.substring(at + 1)}")
        // Names are only unique within their parent: nested ones carry its name.
        out += Module(parent + name, dir, if (pinned.ok) pinned.out else "")
        if (git(client, machine, gitDir, "--git-dir", dir, "cat-file", "-e", "HEAD:.gitmodules").ok) {
            modules(client, machine, dir, listOf("--blob", "HEAD:.gitmodules"), out, "$parent$name/")
        }
    }
    return out
}

/**
 * The worktree's submodules, and the names of those with commits that exist
 * nowhere else: not on a remote, and not the commit the parent pins (main.js
 * `submodules`).
 */
private suspend fun submodules(client: Client, machine: String?, top: String): Pair<List<Module>, List<String>> {
    val gitDir = git(client, machine, top, "rev-parse", "--absolute-git-dir")
    if (!gitDir.ok) error(gitDir.err.ifEmpty { "git rev-parse failed." })
    val mods = modules(client, machine, gitDir.out, listOf("--file", "$top/.gitmodules"))
    val unpushed = mutableListOf<String>()
    for (m in mods) {
        val pin = if (m.pinned.isNotEmpty()) arrayOf(m.pinned) else emptyArray()
        val local = git(client, machine, gitDir.out, "--git-dir", m.dir, "rev-list", "-n1", "--all", "--not", "--remotes", *pin)
        if (!local.ok) error(local.err.ifEmpty { "Couldn't check submodule ${m.name}." })
        if (local.out.isNotEmpty()) unpushed += m.name
    }
    return mods to unpushed
}

internal class Ran(val code: Int, val out: String, val err: String) {
    val ok get() = code == 0
}

// main.js `git`: output trimmed.
internal suspend fun git(client: Client, machine: String?, cwd: String, vararg args: String): Ran =
    exec(client, machine, listOf("git") + args, cwd).let { Ran(it.code, it.out.trim(), it.err.trim()) }

/**
 * `exec(argv[0], rest, {cwd, on})` (mods.rs `run_remote`): `machine.exec`
 * with the desktop's limit. Throws when it can't start or runs out of time.
 */
private suspend fun exec(client: Client, machine: String?, argv: List<String>, cwd: String?): Ran {
    if (!client.has("machine.exec")) error("this core can't machine.exec")
    val r = client.call("machine.exec", buildJsonObject {
        put("argv", JsonArray(argv.map(::JsonPrimitive)))
        put("on", machine?.let(::JsonPrimitive) ?: JsonNull)
        if (cwd != null) put("cwd", cwd)
        put("timeout", LIMIT)
    }, timeoutMs = (LIMIT + 10) * 1000L) as? JsonObject ?: error("machine.exec: no reply")
    if ((r["timed_out"] as? JsonPrimitive)?.booleanOrNull == true) {
        error("${argv[0]} took longer than $LIMIT s${machine?.let { " on $it" } ?: ""} and was stopped")
    }
    fun text(key: String) = Base64.decode(r.string(key) ?: "").decodeToString()
    return Ran(r.long("code")?.toInt() ?: -1, text("stdout"), text("stderr"))
}
