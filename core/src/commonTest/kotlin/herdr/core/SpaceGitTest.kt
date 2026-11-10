package herdr.core

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class SpaceGitTest {
    private val main = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... file"

    @Test
    fun readsTheMainCheckout() {
        val g = parsePlaceGit(main, "/home/dev/src/goatr", "/home/dev/src/goatr/.git", null)
        assertEquals("goatr", g.name)
        assertEquals("main", g.branch)
        assertEquals(2, g.ahead)
        assertEquals(1, g.behind)
        assertFalse(g.linked)
    }

    @Test
    fun aWorktreeIsLinkedAndSharesItsRepo() {
        val tree = parsePlaceGit("# branch.head feat/x", "/home/dev/.herdr/worktrees/goatr/feat-x", "/home/dev/src/goatr/.git", null)
        val home = parsePlaceGit(main, "/home/dev/src/goatr", "/home/dev/src/goatr/.git", null)
        assertTrue(tree.linked)
        assertEquals("goatr", tree.name)
        assertEquals(home.repo, tree.repo)
        assertEquals(0, tree.ahead)
    }

    @Test
    fun theSameFolderOnAnotherMachineIsAnotherRepo() {
        val here = parsePlaceGit(main, "/srv/app", "/srv/app/.git", null)
        val there = parsePlaceGit(main, "/srv/app", "/srv/app/.git", "box")
        assertFalse(here.repo == there.repo)
    }

    @Test
    fun detachedHasNoBranch() {
        assertNull(parsePlaceGit("# branch.head (detached)", "/r", "/r/.git", null).branch)
    }

    @Test
    fun aBareRepoIsNamedAfterItsFolder() {
        assertEquals("app", parsePlaceGit("", "/srv/wt/x", "/srv/app.git", null).name)
    }

    private fun g(repo: String, linked: Boolean, branch: String = "b") = PlaceGit(repo, branch, 0, 0, repo, linked)

    @Test
    fun worktreesGoUnderTheirRepo() {
        val git = mapOf(
            "wt1" to g("goatr", true),
            "notes" to null,
            "goatr" to g("goatr", false),
            "wt2" to g("goatr", true),
            "other" to g("other", false),
        )
        val rows = nest(listOf("wt1", "notes", "goatr", "wt2", "other")) { git[it] }
        assertEquals(listOf("notes", "goatr", "wt1", "wt2", "other"), rows.map { it.space })
        assertEquals(listOf(false, false, true, true, false), rows.map { it.tree })
        assertEquals(listOf(false, false, false, true, false), rows.map { it.last })
    }

    @Test
    fun onlyUnderTheFirstSpaceOnTheMainCheckout() {
        val git = mapOf("a" to g("r", false), "b" to g("r", false), "w" to g("r", true))
        assertEquals(listOf("a", "w", "b"), nest(listOf("a", "b", "w")) { git[it] }.map { it.space })
    }

    @Test
    fun aWorktreeWithoutItsRepoStaysPut() {
        val git = mapOf("w" to g("r", true), "x" to null)
        val rows = nest(listOf("w", "x")) { git[it] }
        assertEquals(listOf("w", "x"), rows.map { it.space })
        assertFalse(rows[0].tree)
    }

    @Test
    fun theTailSaysOnlyWhatsUnusual() {
        assertEquals("", gitTail(g("goatr", false, "main"), tree = false, name = "goatr"))
        assertEquals("goatr", gitTail(g("goatr", false, "master"), tree = false, name = "notes"))
        assertEquals("goatr · spike", gitTail(g("goatr", false, "spike"), tree = false, name = "notes"))
        // A worktree named after its branch, under its repo: nothing to add.
        assertEquals("", gitTail(g("goatr", true, "feat-x"), tree = true, name = "feat-x"))
        assertEquals("feature/mb-1549", gitTail(g("goatr", true, "feature/mb-1549"), tree = true, name = "conflict-fixes"))
        assertEquals("detached", gitTail(PlaceGit("goatr", null, 0, 0, "goatr", true), tree = true, name = "x"))
    }
}
