package herdr.core

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

@OptIn(ExperimentalCoroutinesApi::class)
class WorktreeTest {
    /** One `machine.exec`: what ran, where, on which machine. */
    data class Ran(val argv: List<String>, val cwd: String?, val on: String?, val timeout: Long?)

    /** A machine whose programs answer through `reply`: argv, cwd → code, stdout, stderr. */
    private suspend fun TestScope.machine(
        reply: (List<String>, String?) -> Triple<Int, String, String>,
    ): Pair<MutableList<Ran>, Client> {
        val ran = mutableListOf<Ran>()
        val handlers = mutableMapOf<String, Handler>(
            "machine.exec" to { p ->
                val argv = (p["argv"] as JsonArray).map { it.jsonPrimitive.content }
                val cwd = p.string("cwd")
                ran += Ran(argv, cwd, p.string("on"), p.long("timeout"))
                val (code, out, err) = reply(argv, cwd)
                buildJsonObject {
                    put("code", code)
                    put("signal", null as Int?)
                    put("stdout", Base64.encode(out.encodeToByteArray()))
                    put("stderr", Base64.encode(err.encodeToByteArray()))
                    put("truncated", false)
                    put("timed_out", false)
                    put("ms", 1)
                }
            },
        )
        return ran to connect(FakeCore(this, handlers).duplex, dispatcher = StandardTestDispatcher(testScheduler))
    }

    private fun ok(out: String = "") = Triple(0, out, "")
    private fun fail(err: String) = Triple(128, "", err)
    private val HOME = Triple(0, "/home/dev", "")
    private fun git(vararg a: String) = listOf("git") + a

    private val repo = Repo("/home/dev/src/goatr", "/home/dev/src/goatr", "goatr", false)
    private val linked = Repo("/home/dev/.herdr/worktrees/goatr/feat-x", "/home/dev/src/goatr", "goatr", true)

    @Test
    fun slugsLikeTheDesktop() {
        assertEquals("feat-login", slug("feat/login"))
        assertEquals("a-b-.c_d", slug("--a b//.c_d!"))
        assertEquals("v1.2", slug("v1.2"))
    }

    @Test
    fun aMainCheckoutALinkedOneAndNoneOutsideGit() = runTest {
        val (ran, client) = machine { argv, cwd ->
            when {
                cwd == "/tmp" -> fail("fatal: not a git repository")
                argv[2] == "--show-toplevel" -> ok(if ("worktrees" in cwd!!) "${cwd}\n" else "/home/dev/src/goatr\n")
                else -> ok(if ("worktrees" in cwd!!) "/home/dev/src/goatr/.git\n" else "/home/dev/src/goatr/.git")
            }
        }
        assertEquals(repo, repoOf(client, "beta", "/home/dev/src/goatr/core"))
        assertEquals(
            listOf(
                Ran(git("rev-parse", "--show-toplevel"), "/home/dev/src/goatr/core", "beta", 30),
                Ran(git("rev-parse", "--path-format=absolute", "--git-common-dir"), "/home/dev/src/goatr/core", "beta", 30),
            ),
            ran,
        )
        assertEquals(linked, repoOf(client, "beta", linked.top))
        assertNull(repoOf(client, null, "/tmp"))
        assertNull(ran.last().on)
    }

    @Test
    fun checkoutsMainFirstThenNewestSkippingBareAndPrunable() = runTest {
        val list = """
            worktree /home/dev/src/goatr
            HEAD 1111
            branch refs/heads/main

            worktree /home/dev/.herdr/worktrees/goatr/old
            HEAD 2222
            branch refs/heads/old

            worktree /home/dev/.herdr/worktrees/goatr/gone
            HEAD 3333
            branch refs/heads/gone
            prunable gitdir file points to non-existent location

            worktree /home/dev/.herdr/worktrees/goatr/detached
            HEAD 4444
            detached

            worktree /home/dev/.herdr/worktrees/goatr/feat-x
            HEAD 5555
            branch refs/heads/feat/x
        """.trimIndent()
        val refs = "main\t100\nold\t200\ngone\t900\nfeat/x\t300"
        val (ran, client) = machine { argv, _ -> if (argv[1] == "worktree") ok(list + "\n") else ok(refs + "\n") }
        assertEquals(
            listOf(
                Checkout("/home/dev/src/goatr", "main", true, 100),
                Checkout("/home/dev/.herdr/worktrees/goatr/feat-x", "feat/x", false, 300),
                Checkout("/home/dev/.herdr/worktrees/goatr/old", "old", false, 200),
                Checkout("/home/dev/.herdr/worktrees/goatr/detached", null, false, 0),
            ),
            checkouts(client, null, linked),
        )
        assertEquals(
            listOf(
                Ran(git("worktree", "list", "--porcelain"), repo.main, null, 30),
                Ran(git("for-each-ref", "--format=%(refname:short)%09%(committerdate:unix)", "refs/heads"), repo.main, null, 30),
            ),
            ran,
        )
    }

    @Test
    fun aBareRepoIsNotACheckoutAndAFailedListIsNone() = runTest {
        assertEquals(
            listOf(Checkout("/w/a", "a", true, 0)),
            parseCheckouts("worktree /r.git\nbare\n\nworktree /w/a\nHEAD 1\nbranch refs/heads/a", ""),
        )
        val (_, client) = machine { _, _ -> fail("fatal") }
        assertEquals(emptyList(), checkouts(client, null, repo))
    }

    @Test
    fun aNewBranchStartsFromHereAnOldOneIsCheckedOut() = runTest {
        var has = false
        val (ran, client) = machine { argv, _ ->
            when {
                argv[0] == "sh" -> HOME
                argv[1] == "show-ref" -> if (has) ok() else Triple(1, "", "")
                else -> ok()
            }
        }
        assertEquals("/home/dev/.herdr/worktrees/goatr/feat-login", addWorktree(client, "beta", linked, " feat/login "))
        assertEquals(
            listOf(
                Ran(listOf("sh", "-c", "printf %s \"\$HOME\""), null, "beta", 30),
                Ran(git("show-ref", "--verify", "--quiet", "refs/heads/feat/login"), "/home/dev/src/goatr", "beta", 30),
                Ran(
                    git("worktree", "add", "-b", "feat/login", "/home/dev/.herdr/worktrees/goatr/feat-login", "HEAD"),
                    linked.top, "beta", 30,
                ),
            ),
            ran,
        )
        ran.clear()
        has = true
        addWorktree(client, null, repo, "old")
        assertEquals(Ran(git("worktree", "add", "/home/dev/.herdr/worktrees/goatr/old", "old"), repo.top, null, 30), ran.last())
    }

    @Test
    fun gitsMessageWhenAddFailsAndAWindowsHome() = runTest {
        val (ran, client) = machine { argv, _ ->
            when {
                argv[0] == "sh" -> throw Fault(-32000, "cannot start sh", "not_found")
                argv[0] == "powershell.exe" -> ok("C:\\Users\\dev")
                argv[1] == "show-ref" -> Triple(1, "", "")
                else -> fail("fatal: 'x' is already used by worktree at '/elsewhere'\n")
            }
        }
        val e = assertFailsWith<IllegalStateException> { addWorktree(client, "win", repo, "x") }
        assertEquals("fatal: 'x' is already used by worktree at '/elsewhere'", e.message)
        assertEquals(listOf("powershell.exe", "-NoProfile", "-Command", "[Console]::Write(\$HOME)"), ran[1].argv)
        assertEquals(git("worktree", "add", "-b", "x", "C:/Users/dev/.herdr/worktrees/goatr/x", "HEAD"), ran.last().argv)
        assertEquals("Type a branch name.", assertFailsWith<IllegalStateException> { addWorktree(client, null, repo, "  ") }.message)
    }

    @Test
    fun aCleanWorktreeWithoutSubmodulesIsRemovedUnforced() = runTest {
        val (ran, client) = machine { argv, _ ->
            when {
                "--absolute-git-dir" in argv -> ok("/home/dev/src/goatr/.git/worktrees/feat-x\n")
                "config" in argv -> Triple(1, "", "")
                else -> ok()
            }
        }
        val risk = removeRisk(client, null, linked)
        assertEquals(RemoveRisk(false, emptyList(), 0), risk)
        assertFalse(risk.force)
        assertEquals(
            listOf(
                Ran(git("--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain"), linked.top, null, 30),
                Ran(git("rev-parse", "--absolute-git-dir"), linked.top, null, 30),
                Ran(
                    git(
                        "--git-dir", "/home/dev/src/goatr/.git/worktrees/feat-x", "config",
                        "--file", "${linked.top}/.gitmodules", "-z", "--get-regexp", "^submodule\\..*\\.path$",
                    ),
                    "/home/dev/src/goatr/.git/worktrees/feat-x", null, 30,
                ),
            ),
            ran,
        )
        ran.clear()
        removeWorktree(client, null, linked, risk.force)
        assertEquals(listOf(Ran(git("worktree", "remove", linked.top), linked.main, null, 30)), ran)
    }

    @Test
    fun dirtyWithSubmodulesNestedOnesAndUnpushedCommitsIsForced() = runTest {
        val gd = "/g"
        val (ran, client) = machine { argv, _ ->
            val a = argv.drop(1)
            when {
                a[0] == "--no-optional-locks" -> ok(" M README.md\n")
                a == listOf("rev-parse", "--absolute-git-dir") -> ok("$gd\n")
                a.take(3) == listOf("--git-dir", gd, "config") ->
                    ok("submodule.lib one.path\nvendor/lib one\u0000submodule.never.path\nnever\u0000")
                a.take(3) == listOf("--git-dir", "$gd/modules/lib one", "config") ->
                    ok("submodule.deep.path\ndeep\u0000")
                a[2] == "rev-parse" && a[3] == "--git-dir" -> if ("never" in a[1]) fail("not a git repo") else ok(a[1])
                a.take(3) == listOf("--git-dir", gd, "rev-parse") -> ok("aaaa\n")
                a.take(3) == listOf("--git-dir", "$gd/modules/lib one", "rev-parse") -> fail("fatal: bad object")
                a[2] == "cat-file" -> if (a[1] == "$gd/modules/lib one") ok() else Triple(1, "", "")
                a[2] == "rev-list" -> if ("deep" in a[1]) ok("bbbb\n") else ok()
                a[0] == "worktree" -> ok()
                else -> fail("unexpected $argv")
            }
        }
        val risk = removeRisk(client, "beta", linked)
        assertEquals(RemoveRisk(true, listOf("lib one/deep"), 2), risk)
        assertTrue(risk.force)
        assertTrue(ran.all { it.on == "beta" })
        assertTrue(Ran(git("--git-dir", gd, "rev-parse", "HEAD:vendor/lib one"), gd, "beta", 30) in ran)
        assertTrue(
            Ran(
                git("--git-dir", "$gd/modules/lib one", "config", "--blob", "HEAD:.gitmodules", "-z", "--get-regexp", "^submodule\\..*\\.path$"),
                "$gd/modules/lib one", "beta", 30,
            ) in ran,
        )
        assertTrue(Ran(git("--git-dir", "$gd/modules/lib one", "rev-list", "-n1", "--all", "--not", "--remotes", "aaaa"), gd, "beta", 30) in ran)
        assertTrue(Ran(git("--git-dir", "$gd/modules/lib one/modules/deep", "rev-list", "-n1", "--all", "--not", "--remotes"), gd, "beta", 30) in ran)
        ran.clear()
        removeWorktree(client, "beta", linked, risk.force)
        assertEquals(listOf(Ran(git("worktree", "remove", "--force", linked.top), linked.main, "beta", 30)), ran)
    }

    @Test
    fun removeFailuresCarryGitsMessage() = runTest {
        val (_, client) = machine { _, _ -> fail("fatal: '/w' contains modified or untracked files, use --force to delete it\n") }
        val e = assertFailsWith<IllegalStateException> { removeWorktree(client, null, linked, false) }
        assertEquals("fatal: '/w' contains modified or untracked files, use --force to delete it", e.message)
        assertFailsWith<IllegalStateException> { removeRisk(client, null, linked) }
    }

    @Test
    fun onlyNewRiskWarnsAgain() {
        val shown = RemoveRisk(true, listOf("a"), 1)
        assertFalse(changed(shown, RemoveRisk(true, listOf("a"), 1)))
        assertFalse(changed(shown, RemoveRisk(false, emptyList(), 1)))
        assertTrue(changed(RemoveRisk(false, listOf("a"), 1), RemoveRisk(true, listOf("a"), 1)))
        assertTrue(changed(shown, RemoveRisk(true, listOf("a", "b"), 2)))
        assertEquals(RemoveRisk(true, listOf("a", "b"), 2), seen(shown, RemoveRisk(false, listOf("b"), 2)))
    }

    @Test
    fun tildeAndAgo() {
        assertEquals("~/.herdr/worktrees", tilde("/home/dev/.herdr/worktrees", "/home/dev"))
        assertEquals("~/x", tilde("C:\\Users\\dev\\x", "C:/Users/dev"))
        assertEquals("/srv/x", tilde("/srv/x", "/home/dev"))
        assertEquals("1m", ago(1000, 1010))
        assertEquals("3h", ago(0, 3 * 3600))
        assertEquals("2d", ago(0, 2 * 86400))
        assertEquals("4mo", ago(0, 120L * 86400))
    }

    private fun term(address: String, cwd: String) = Terminal(
        address = address, machine = machineOf(address), name = null, agent = null, program = "fish",
        shown = programOf(null, listOf("fish")), title = "", detail = null, cwd = cwd, status = null,
        attention = null, statusMs = null, space = null, tab = null, where = null, parent = null, pinned = false,
        record = TerminalRecord(JsonObject(emptyMap())),
    )

    private fun space(id: String, machine: String?, folder: String, vararg terminals: String) =
        Space(id, "", "", machine, folder, listOf(Tab("tb", "", terminals.toList(), null)), null)

    @Test
    fun theSpaceShowingACheckoutTheDeepestWins() {
        val all = listOf(
            Checkout("/r", "main", true, 0),
            Checkout("/r/.wt/x", "x", false, 0),
        )
        val terminals = listOf(term("t1", "/r/src"), term("t2", ""), term("t3", "/r/.wt/x/a"), term("beta/t4", "/r"))
        val spaces = listOf(
            space("sp1", null, "/r/.wt/x", "t2", "t3"), // first terminal has no folder: the next one's
            space("sp2", null, "/elsewhere", "t1"),
            space("beta/sp3", "beta", "/r", "beta/t4"),
            space("sp4", null, "/r"),
        )
        assertEquals("/r/.wt/x/a", place(spaces[0], terminals))
        assertEquals("/r", place(spaces[3], terminals))
        assertEquals("sp1", spaceOf("/r/.wt/x", all, null, spaces, terminals)?.id)
        assertEquals("sp2", spaceOf("/r", all, null, spaces, terminals)?.id)
        assertEquals("beta/sp3", spaceOf("/r", all, "beta", spaces, terminals)?.id)
        assertNull(spaceOf("/r/.wt/y", all, null, spaces, terminals))
    }
}
