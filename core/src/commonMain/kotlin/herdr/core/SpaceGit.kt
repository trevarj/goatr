// The git the desktop's spaces list shows (herdr-app mods/spaces): where each
// space is now, its branch and how far it is from upstream, and worktree
// spaces moved under the space on their repo's main checkout.

package herdr.core

/**
 * A place's git (spaces/main.js `refreshGit`): `repo` names the repository
 * (its machine and common git dir), the same for every checkout of it;
 * `linked`: the place is in a worktree, not the main checkout.
 */
data class PlaceGit(
    val name: String,
    val branch: String?,
    val ahead: Int,
    val behind: Int,
    val repo: String,
    val linked: Boolean,
)

/**
 * The git at `folder` on `machine`, null outside a repo. Throws when git
 * can't answer (the machine away), so the caller asks again later instead
 * of remembering "no repo".
 */
suspend fun placeGit(client: Client, machine: String?, folder: String): PlaceGit? {
    // Writes nothing under .git (index lock, fsmonitor cookie).
    val status = git(client, machine, folder, "--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain=v2", "--branch")
    if (!status.ok) {
        // Only git's own "no" means no repo; anything else is asked again.
        if (Regex("not a git repository", RegexOption.IGNORE_CASE).containsMatchIn(status.err)) return null
        error(status.err.ifEmpty { "git status failed." })
    }
    val top = git(client, machine, folder, "rev-parse", "--show-toplevel")
    val common = git(client, machine, folder, "rev-parse", "--path-format=absolute", "--git-common-dir")
    if (!top.ok || !common.ok) error(common.err.ifEmpty { top.err.ifEmpty { "git rev-parse failed." } })
    return parsePlaceGit(status.out, top.out, common.out, machine)
}

/** `git status --porcelain=v2 --branch`, the checkout's top and the common git dir, read as [placeGit] does. */
internal fun parsePlaceGit(status: String, top: String, common: String, machine: String?): PlaceGit {
    val base = { p: String -> p.trimEnd('/').substringAfterLast('/') }
    // The repo's own .git folder names it; a worktree's top differs from it.
    val name = if (common.endsWith("/.git")) base(common.removeSuffix("/.git")) else base(common).removeSuffix(".git")
    var branch: String? = null
    var ahead = 0
    var behind = 0
    for (line in status.split("\n")) {
        if (line.startsWith("# branch.head ")) branch = line.substring(14).takeIf { it != "(detached)" }
        else if (line.startsWith("# branch.ab ")) {
            val (a, b) = line.substring(12).split(" ").map { it.drop(1).toIntOrNull() ?: 0 }.let { it.getOrElse(0) { 0 } to it.getOrElse(1) { 0 } }
            ahead = a
            behind = b
        }
    }
    return PlaceGit(name, branch, ahead, behind, "${machine ?: ""}\u0000$common", common != "$top/.git")
}

/** A row of the spaces list: `tree` when it's a worktree shown under its repo's space (`last`: the last of them). */
data class Nested<S>(val space: S, val git: PlaceGit?, val tree: Boolean = false, val last: Boolean = false)

/**
 * Spaces in order, each worktree space moved under the first space on its
 * repo's main checkout (spaces/main.js `ordered`). A worktree whose main
 * checkout no space shows stays where it is, as does one git hasn't
 * answered for yet.
 */
fun <S> nest(spaces: List<S>, git: (S) -> PlaceGit?): List<Nested<S>> {
    val list = spaces.map { it to git(it) }
    val mains = list.mapNotNull { (_, g) -> g?.takeIf { !it.linked }?.repo }.toSet()
    fun nested(g: PlaceGit?) = g != null && g.linked && g.repo in mains
    val out = mutableListOf<Nested<S>>()
    val placed = mutableSetOf<String>()
    for ((s, g) in list) {
        if (nested(g)) continue
        out += Nested(s, g)
        if (g == null || g.linked || !placed.add(g.repo)) continue
        val kids = list.filter { (_, k) -> nested(k) && k!!.repo == g.repo }
        kids.forEachIndexed { n, (k, kg) -> out += Nested(k, kg, tree = true, last = n == kids.lastIndex) }
    }
    return out
}

/**
 * What a space's row says about its git, after its name (spaces/main.js
 * `gitTail`): only what's unusual. The repo or branch goes when the name
 * already says it (a worktree is named after its branch), the repo under
 * its own space, and master/main: nearly every space is on one.
 */
fun gitTail(g: PlaceGit, tree: Boolean, name: String): String {
    val raw = g.branch ?: "detached"
    fun same(a: String) = a.equals(name, ignoreCase = true)
    val branch = if (same(raw) || raw == "master" || raw == "main") "" else raw
    val repo = if (tree || same(g.name)) "" else g.name
    return listOf(repo, branch).filter { it.isNotEmpty() }.joinToString(" · ")
}
