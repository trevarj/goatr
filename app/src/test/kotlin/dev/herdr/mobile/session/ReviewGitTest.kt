package io.github.trevarj.goatr.app.session

import herdr.core.Client
import herdr.core.FakeCore
import herdr.core.connect
import java.io.File
import java.nio.file.Files
import java.util.Base64
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import kotlin.test.*

class ReviewGitTest {
    private val root = "/checkout \n"
    private val before = "1".repeat(40)
    private val index = "2".repeat(40)
    private val head = "3".repeat(40)
    private val base = "4".repeat(40)
    private val work = ReviewComparison(ReviewMode.WorkingTree)
    private val branch = ReviewComparison(ReviewMode.Branch, "refs/heads/main")
    private val prefix = listOf("git", "--no-pager", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "color.ui=false", "-c", "core.quotePath=true", "-c", "diff.autoRefreshIndex=false")

    private fun ByteArray.b64() = Base64.getEncoder().encodeToString(this)
    private fun reply(out: ByteArray, code: Int = 0, err: ByteArray = byteArrayOf()) = buildJsonObject {
        put("stdout", out.b64()); put("stderr", err.b64()); put("code", code); put("signal", JsonNull)
        put("truncated", false); put("timed_out", false); put("ms", 1)
    }
    private fun reply(s: String = "", code: Int = 0) = reply(s.encodeToByteArray(), code)
    private fun raw(path: String, status: String = "M", oldMode: String = "100644", mode: String = "100644") =
        ":$oldMode $mode $before $index $status\u0000$path\u0000"
    private fun patch(from: String, to: String, path: String = "same") =
        "diff --git a/$path b/$path\nindex $before..$index 100644\n--- a/$path\n+++ b/$path\n@@ -1,2 +1,2 @@\n common\n-$from\n+$to\n"
    private fun defaults(a: List<String>): JsonObject = when (a.first()) {
        "rev-parse" -> reply(when (a.last()) {
            "--is-inside-work-tree" -> "true"; "--show-toplevel" -> root; "--absolute-git-dir", "--git-common-dir" -> "$root/.git"
            "--is-shallow-repository" -> "false"
            "HEAD^{commit}" -> head; "refs/heads/main^{commit}" -> base; else -> error("Unexpected revision query")
        } + "\n")
        "for-each-ref" -> reply("refs/heads/main\u0000$base\u0000\nrefs/heads/topic\u0000$head\u0000\nrefs/remotes/origin/HEAD\u0000$base\u0000refs/remotes/origin/trunk\nrefs/remotes/origin/trunk\u0000$base\u0000\n")
        "symbolic-ref" -> reply("refs/heads/topic\n")
        "merge-base" -> reply("$before\n")
        "diff" -> reply()
        "status" -> reply("# branch.oid $head\u0000# branch.head topic\u0000")
        else -> error("Unexpected command")
    }
    private suspend fun <T> machine(
        folder: String = root,
        answer: (List<String>, String) -> JsonObject,
        use: suspend (Client) -> T,
    ): T {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
        val fake = FakeCore(scope, mutableMapOf("machine.exec" to { p ->
            check(p["on"] == JsonPrimitive("owned") && p["timeout"] == JsonPrimitive(30) && "line" !in p)
            val env = p.getValue("env").jsonObject
            check(env["GIT_NO_LAZY_FETCH"] == JsonPrimitive("1") && env["GIT_TERMINAL_PROMPT"] == JsonPrimitive("0"))
            val argv = p.getValue("argv").jsonArray.map { it.jsonPrimitive.content }
            check(argv.take(prefix.size) == prefix)
            val cwd = p.getValue("cwd").jsonPrimitive.content
            check(cwd == folder)
            answer(argv.drop(prefix.size), cwd)
        }))
        val client = connect(fake.duplex)
        return try { use(client) } finally { client.close(); scope.cancel() }
    }
    private suspend fun capture(answer: (List<String>) -> JsonObject) = machine(answer = { a, _ -> answer(a) }) {
        captureReview(it, "owned", root, work)
    }

    @Test fun realUnbornLayersPreserveUnusualNamesBinarySymlinkAndEmptyFiles() = runBlocking {
        val temp = Files.createTempDirectory("review-git").toFile()
        val repo = temp.resolve("checkout").apply { mkdir() }
        fun git(argv: List<String>): Pair<Int, ByteArray> {
            val err = temp.resolve("stderr")
            val p = ProcessBuilder(argv).directory(repo).redirectError(err).apply {
                val path = environment()["PATH"]!!
                environment().clear(); environment().putAll(mapOf("PATH" to path, "HOME" to temp.path, "GIT_CONFIG_NOSYSTEM" to "1", "LC_ALL" to "C"))
            }.start()
            p.outputStream.close()
            val bytes = p.inputStream.use { it.readBytes() }
            return p.waitFor() to bytes
        }
        fun setup(vararg args: String) = check(git(listOf("git") + args).first == 0)
        try {
            setup("init", "--quiet")
            repo.resolve("same").writeText("common\r\nv2")
            repo.resolve("stat-only").writeText("unchanged\n")
            setup("add", "--", "same", "stat-only")
            val indexFile = repo.resolve(".git/index")
            val savedIndex = indexFile.readBytes()
            check(repo.resolve("stat-only").setLastModified(1_600_000_000_000))
            repo.resolve("same").writeText("common\r\nv3")
            val odd = "a space\t\"\\é\n-"
            repo.resolve(odd).writeText("review\r\nlast")
            repo.resolve("empty").createNewFile(); repo.resolve("-").writeText("dash\n")
            repo.resolve("binary").writeBytes(byteArrayOf(0, 1)); repo.resolve("invalid").writeBytes(byteArrayOf(-1, 10))
            Files.createSymbolicLink(repo.resolve("link").toPath(), repo.resolve("same").toPath().fileName)
            setup("init", "--quiet", "nested"); repo.resolve("nested/private").writeText("not captured")
            machine(repo.path, { a, _ ->
                check(a.last() != "./nested/")
                val (exit, out) = git(prefix + a); reply(out, exit)
            }) { client ->
                val snap = captureReview(client, "owned", repo.path, work); assertNull(snap.headOid)
                assertContentEquals(savedIndex, indexFile.readBytes())
                val files = reviewFiles(snap).associateBy { it.key.layer to it.key.path }
                assertTrue(ReviewLayer.Staged to "stat-only" in files)
                assertFalse(ReviewLayer.Unstaged to "stat-only" in files)
                fun untracked(path: String) = files.getValue(ReviewLayer.Untracked to path)
                val staged = files.getValue(ReviewLayer.Staged to "same").hunks.single()
                assertEquals(0, staged.beforeCount); assertEquals(listOf("common\r", "v2"), staged.lines.map { it.text })
                val unstaged = files.getValue(ReviewLayer.Unstaged to "same").hunks.single().lines
                assertEquals(ReviewLine("v2", 2, null, ReviewLineKind.Deletion, true), unstaged[1])
                assertEquals(ReviewLine("v3", null, 2, ReviewLineKind.Addition, true), unstaged[2])
                assertEquals("last", untracked(odd).hunks.single().lines.last().text)
                assertEquals("dash", untracked("-").hunks.single().lines.single().text)
                assertEquals("120000", untracked("link").afterMode); assertEquals("same", untracked("link").hunks.single().lines.single().text)
                assertTrue(untracked("binary").binary); assertTrue(untracked("binary").hunks.isEmpty())
                assertTrue(untracked("empty").hunks.isEmpty()); assertNotNull(untracked("empty").previewUnavailable)
                assertTrue(untracked("invalid").previewUnavailable!!.contains("UTF-8"))
                assertTrue(untracked("nested/").previewUnavailable!!.contains("excluded"))
                assertFalse(files.keys.any { it.second.endsWith("private") })
            }
        } finally { temp.deleteRecursively() }
    }

    @Test fun layeredSnapshotsKeepStageDeletionsRenamesConflictsAndGitlinksDistinct() = runBlocking {
        val stagedRaw = raw("same") + raw("again", "D", mode = "000000") + raw("old name", "R100") + "renamed\u0000" + raw("exec", mode = "100755")
        val stagedPatch = patch("v1", "v2") + "diff --git a/again b/again\ndeleted file mode 100644\nindex $before..${"0".repeat(40)}\n--- a/again\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n" +
            "diff --git a/old name b/renamed\nsimilarity index 100%\nrename from old name\nrename to renamed\ndiff --git a/exec b/exec\nold mode 100644\nnew mode 100755\n"
        val snap = capture { a -> when {
            a.first() == "status" -> reply("# branch.oid $head\u0000? again\u0000")
            "--no-index" in a -> reply("diff --git a/./again b/./again\nnew file mode 100644\n--- /dev/null\n+++ b/./again\n@@ -0,0 +1 @@\n+back\n", 1)
            "--cached" in a -> reply(if ("--raw" in a) stagedRaw else stagedPatch)
            a.first() == "diff" -> reply(if ("--raw" in a) raw("same") + raw("clash", "U") + raw("sub", oldMode = "160000", mode = "160000") else
                patch("v2", "v3") + "diff --cc clash\nindex $before,$index..$head\n@@@ -1 -1 +1 @@@\n++conflict\ndiff --git a/sub b/sub\n--- a/sub\n+++ b/sub\n@@ -1 +1 @@\n-Subproject commit $head\n+Subproject commit $head-dirty\n")
            else -> defaults(a)
        } }
        assertEquals(root, snap.repo.root); assertEquals(reviewSnapshotId(snap), snap.id); assertTrue(snap.id.matches(Regex("[0-9a-f]{64}")))
        val files = reviewFiles(snap)
        val layers = files.filter { it.key.path == "same" }
        assertEquals(listOf(ReviewLayer.Staged, ReviewLayer.Unstaged), layers.map { it.key.layer })
        assertEquals(listOf("v1", "v2"), layers.map { it.hunks.single().lines[1].text })
        assertEquals(listOf("v2", "v3"), layers.map { it.hunks.single().lines[2].text })
        assertEquals(listOf(2, 2), layers.map { it.hunks.single().lines[2].after })
        assertEquals(listOf(ReviewLayer.Staged, ReviewLayer.Untracked), files.filter { it.key.path == "again" }.map { it.key.layer })
        assertEquals("old name", files.single { it.key.path == "renamed" }.key.oldPath)
        assertEquals("100755", files.single { it.key.path == "exec" }.afterMode)
        assertTrue(files.single { it.conflict }.hunks.isEmpty()); assertTrue(files.single { it.submodule }.hunks.isEmpty())
    }

    @Test fun branchPinsMergeBaseExcludesDirtyEditsAndRejectsMovingOrAmbiguousRefs() = runBlocking {
        var refs = 0; var move = false; var multiple = false; var dirty = "v2"
        machine(answer = { a, _ -> when {
            a.first() == "for-each-ref" -> { refs++; if (move && refs % 2 == 0) reply("refs/heads/main\u0000$index\u0000\n") else defaults(a) }
            a.first() == "merge-base" && multiple -> reply("$before\n$index\n")
            a.first() == "diff" -> reply(if ("--raw" in a) raw("same") else if (a.takeLast(3) == listOf(before, head, "--")) patch("base", "committed") else patch("v1", dirty))
            else -> defaults(a)
        } }) { client ->
            val branches = reviewBranches(client, "owned", root)
            assertEquals("refs/remotes/origin/trunk", branches.single { it.suggested }.ref); assertFalse(branches.any { it.ref.endsWith("/HEAD") })
            suspend fun snap() = captureReview(client, "owned", root, branch)
            val first = snap(); dirty = "other"; assertEquals(first, snap()); assertEquals(before, first.mergeBaseOid); assertEquals(base, first.baseOid)
            assertEquals("committed", reviewFiles(first).single().hunks.single().lines.last().text)
            refs = 0; move = true; assertTrue(assertFails { snap() }.message!!.contains("moved"))
            move = false; multiple = true; assertTrue(assertFails { snap() }.message!!.contains("one locally available merge base"))
            assertFails { captureReview(client, "owned", root, branch.copy(baseRef = "HEAD~1")) }
        }
        Unit
    }

    @Test fun movedTextAndBinaryContentNeverBecomeAStableCapture() = runBlocking {
        for (binary in listOf(false, true)) {
            var n = 0
            val ex = assertFails { capture { a -> when {
                a.first() == "diff" && "--cached" !in a -> if ("--raw" in a) reply(raw("same")) else {
                    n++; reply(if (binary) "diff --git a/same b/same\nindex $before..${if (n == 1) index else head} 100644\nBinary files a/same and b/same differ\n" else patch("v2", "v$n"))
                }
                else -> defaults(a)
            } } }
            assertEquals("Changes moved while loading. Refresh to try again.", ex.message)
        }
    }

    @Test fun malformedTruncatedTimedOutAndUnexpectedDestinationsFailClosed() = runBlocking {
        for ((key, value) in listOf("truncated" to JsonPrimitive(true), "timed_out" to JsonPrimitive(true), "code" to JsonPrimitive("0"), "stdout" to JsonPrimitive("?"), "stderr" to JsonNull, "code" to JsonPrimitive(127))) {
            assertFails { capture { a -> if (a.first() == "status") JsonObject(defaults(a) + (key to value)) else defaults(a) } }
        }
        val good = "diff --git a/./only b/./only\nnew file mode 100644\n"
        for (bad in listOf(good + good, good + "--- /dev/null\n+++ b/./another\n", good.replace("only", "only/child"), "")) {
            assertTrue(assertFails { capture { a -> when {
                a.first() == "status" -> reply("# branch.oid $head\u0000? only\u0000")
                "--no-index" in a -> reply(bad, 1)
                else -> defaults(a)
            } } }.message!!.contains("only"))
        }
        val malformed = capture { a -> when {
            "--cached" in a -> reply(if ("--raw" in a) raw("same") else patch("v1", "v2").replace("+1,2", "+1,3"))
            else -> defaults(a)
        } }
        val file = reviewFiles(malformed).single(); assertNotNull(file.previewUnavailable); assertTrue(file.hunks.isEmpty())
    }

    @Test fun fileAndAggregateByteBoundsAreAppliedBeforeSnapshotAcceptance() = runBlocking {
        suspend fun files(n: Int) = capture { a -> if ("--raw" in a && "--cached" in a) reply((0 until n).joinToString("") { raw("f$it") }) else defaults(a) }
        assertEquals(512, reviewFiles(files(512)).size); assertTrue(assertFails { files(513) }.message!!.contains("512"))
        val status = "# branch.oid $head\u0000" + (1..9).joinToString("") { "? f$it\u0000" }
        val big = ByteArray(1024 * 1024)
        var invoked = 0
        val ex = assertFails { capture { a -> when {
            a.first() == "status" -> reply(status)
            "--no-index" in a -> { invoked++; val p = a.last(); reply("diff --git a/$p b/$p\nnew file mode 100644\n".encodeToByteArray(), 1, big) }
            else -> defaults(a)
        } } }
        assertEquals(8, invoked); assertTrue(ex.message!!.contains("8 MiB"))
    }
}
