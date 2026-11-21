package io.github.trevarj.goatr.app.session

import herdr.core.Client
import java.security.MessageDigest
import java.util.Base64
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

@Serializable enum class ReviewMode { WorkingTree, Branch }
@Serializable enum class ReviewLayer { Staged, Unstaged, Untracked, Branch }
@Serializable enum class ReviewSide { Before, After }
@Serializable data class ReviewComparison(val mode: ReviewMode, val baseRef: String? = null)
@Serializable data class ReviewRepo(val root: String, val gitDir: String, val commonDir: String)
data class ReviewBranch(val ref: String, val oid: String, val suggested: Boolean = false)
@Serializable data class ReviewSource(val layer: ReviewLayer, val path: String?, val metadataBase64: String, val patchBase64: String)
@Serializable data class ReviewSnapshot(
    val id: String, val repo: ReviewRepo, val comparison: ReviewComparison, val headOid: String?,
    val baseOid: String?, val mergeBaseOid: String?, val statusBase64: String, val sources: List<ReviewSource>,
)
@Serializable data class ReviewFileKey(val layer: ReviewLayer, val oldPath: String?, val path: String)
@Serializable data class ReviewAnchor(val snapshotId: String, val file: ReviewFileKey, val side: ReviewSide?, val startLine: Int?, val endLine: Int?)

enum class ReviewLineKind { Context, Addition, Deletion }
data class ReviewLine(val text: String, val before: Int?, val after: Int?, val kind: ReviewLineKind, val noFinalNewline: Boolean = false)
data class ReviewHunk(val header: String, val beforeStart: Int, val beforeCount: Int, val afterStart: Int, val afterCount: Int, val lines: List<ReviewLine>)
data class ReviewFile(
    val key: ReviewFileKey, val status: String, val beforeMode: String?, val afterMode: String?, val patchHash: String,
    val binary: Boolean, val submodule: Boolean, val conflict: Boolean, val hunks: List<ReviewHunk>, val previewUnavailable: String? = null,
)

private const val MAX_BYTES = 8 * 1024 * 1024
private const val MAX_FILES = 512
private const val MOVED = "Changes moved while loading. Refresh to try again."
private const val BYTE_LIMIT = "Review capture exceeds 8 MiB."
private const val FILE_LIMIT = "Review capture exceeds 512 file-layer entries."
private val oidPattern = Regex("(?:[0-9a-f]{40}|[0-9a-f]{64})")
private val prefix = listOf("git", "--no-pager", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "color.ui=false", "-c", "core.quotePath=true", "-c", "diff.autoRefreshIndex=false")
private val diffFlags = listOf("--no-color", "--no-ext-diff", "--no-textconv", "--find-renames", "--submodule=short", "--ignore-submodules=none", "--ita-invisible-in-index")
private val patchFlags = listOf("--patch", "--full-index", "--unified=3", "--src-prefix=a/", "--dst-prefix=b/")
private fun ByteArray.b64() = Base64.getEncoder().encodeToString(this)
private fun String.bytes() = Base64.getDecoder().decode(this)
private fun ByteArray.utf8() = try { decodeToString(throwOnInvalidSequence = true) } catch (_: java.nio.charset.CharacterCodingException) { error("Review contains a path or field that is not UTF-8.") }
private fun ByteArray.record(): String {
    val s = utf8()
    require(s.endsWith('\n') && s.length > 1) { "Malformed Git response." }
    return s.dropLast(1)
}
private fun String.oid() = also { require(oidPattern.matches(it)) { "Git returned an invalid commit ID." } }
private fun String.path(directory: Boolean = false) = also {
    val bare = if (directory) removeSuffix("/") else this
    require('\u0000' !in this && bare.splitToSequence('/').none { it.isEmpty() || it == "." || it == ".." }) { "Invalid repository-relative Git path." }
}
private fun digest(block: MessageDigest.() -> Unit): String = MessageDigest.getInstance("SHA-256").apply(block).digest().joinToString("") { (it.toInt() and 255).toString(16).padStart(2, '0') }
private fun MessageDigest.field(bytes: ByteArray?) {
    val n = bytes?.size ?: -1
    for (shift in 24 downTo 0 step 8) update((n ushr shift).toByte())
    if (bytes != null) update(bytes)
}
private fun MessageDigest.text(s: String?) = field(s?.encodeToByteArray())
internal fun reviewSnapshotId(snapshot: ReviewSnapshot) = digest {
    with(snapshot) {
        text(repo.root); text(repo.gitDir); text(repo.commonDir); text(comparison.mode.name); text(comparison.baseRef)
        text(headOid); text(baseOid); text(mergeBaseOid); field(statusBase64.bytes())
        for (s in sources) { text(s.layer.name); text(s.path); field(s.metadataBase64.bytes()); field(s.patchBase64.bytes()) }
    }
}

private class GitRead(val client: Client, val machine: String?, var cwd: String) {
    private var remaining = MAX_BYTES
    init {
        check(client.has("machine.exec")) { "This desktop does not support machine.exec." }
        require(cwd.isNotEmpty() && '\u0000' !in cwd && Charsets.UTF_8.newEncoder().canEncode(cwd)) { "Invalid UTF-8 working directory." }
    }
    suspend fun run(args: List<String>, allowed: Set<Int> = setOf(0)): Pair<Int, ByteArray> {
        val r = try {
            client.call("machine.exec", buildJsonObject {
                put("argv", JsonArray((prefix + args).map(::JsonPrimitive))); put("on", machine?.let(::JsonPrimitive) ?: JsonNull); put("cwd", cwd)
                put("env", buildJsonObject { put("GIT_NO_LAZY_FETCH", "1"); put("GIT_TERMINAL_PROMPT", "0"); put("LC_ALL", "C") }); put("timeout", 30)
            }, timeoutMs = 40_000) as? JsonObject
        } catch (e: Exception) {
            currentCoroutineContext().ensureActive()
            error("Could not run Git on the selected machine. Check Git and the desktop connection.")
        } ?: error("Malformed machine.exec result.")
        fun malformed(): Nothing = error("Malformed machine.exec result.")
        fun primitive(k: String) = r[k] as? JsonPrimitive ?: malformed()
        fun flag(k: String) = primitive(k).takeUnless { it.isString }?.booleanOrNull ?: malformed()
        check(!flag("truncated")) { "Git output was truncated by the desktop." }
        check(!flag("timed_out")) { "Git timed out on the desktop." }
        check(r["signal"] == JsonNull) { "Git did not exit normally." }
        primitive("ms").takeUnless { it.isString }?.longOrNull?.takeIf { it >= 0 } ?: malformed()
        val code = primitive("code").takeUnless { it.isString }?.intOrNull ?: malformed()
        fun decode(k: String): ByteArray {
            val value = primitive(k).takeIf { it.isString }?.content ?: malformed()
            require(value.length <= ((remaining + 2) / 3) * 4) { BYTE_LIMIT }
            val bytes = try { value.bytes() } catch (_: IllegalArgumentException) { malformed() }
            remaining -= bytes.size
            require(remaining >= 0) { BYTE_LIMIT }
            return bytes
        }
        val out = decode("stdout"); decode("stderr")
        check(code in allowed) { "Git ${args.first()} failed (exit $code). Check the repository and locally available history." }
        return code to out
    }
    suspend fun out(vararg args: String) = run(args.toList()).second
    suspend fun repo(): ReviewRepo {
        check(out("rev-parse", "--is-inside-work-tree").record() == "true") { "Select a Git working tree to review." }
        cwd = out("rev-parse", "--show-toplevel").record()
        require('\u0000' !in cwd) { "Invalid Git working directory." }
        return ReviewRepo(cwd, out("rev-parse", "--path-format=absolute", "--absolute-git-dir").record(), out("rev-parse", "--path-format=absolute", "--git-common-dir").record())
    }
    suspend fun branches(): List<ReviewBranch> {
        val bytes = out("for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)", "refs/heads/", "refs/remotes/")
        val candidates = mutableListOf<ReviewBranch>()
        var origin: String? = null
        if (bytes.isNotEmpty()) for (row in bytes.record().lineSequence()) {
            val fields = row.split('\u0000')
            require(fields.size == 3 && (fields[0].startsWith("refs/heads/") || fields[0].startsWith("refs/remotes/"))) { "Invalid local branch listing." }
            val (ref, oid, sym) = fields
            if (sym.isEmpty()) candidates += ReviewBranch(ref, oid.oid()) else if (ref == "refs/remotes/origin/HEAD") origin = sym
        }
        val (code, raw) = run(listOf("symbolic-ref", "--quiet", "HEAD"), setOf(0, 1))
        val current = if (code == 0) raw.record() else null
        val suggested = listOfNotNull(origin, "refs/heads/main", "refs/heads/master").firstOrNull { it != current && candidates.any { b -> b.ref == it } }
        return candidates.map { if (it.ref == suggested) it.copy(suggested = true) else it }
    }
}

suspend fun reviewBranches(client: Client, machine: String?, folder: String): List<ReviewBranch> = withContext(Dispatchers.Default) {
    GitRead(client, machine, folder).run { repo(); branches() }
}

private class Fields(private val data: ByteArray) {
    var offset = 0; private set
    fun more() = offset < data.size
    fun next(): String {
        val start = offset
        while (offset < data.size && data[offset] != 0.toByte()) offset++
        require(offset < data.size) { "Malformed NUL-delimited Git metadata." }
        return data.copyOfRange(start, offset++).utf8()
    }
}
private data class Meta(val path: String, val old: String?, val status: String, val before: String?, val after: String?, val bytes: ByteArray)
private val rawHeader = Regex(":([0-7]{6}) ([0-7]{6}) ([0-9a-f]{40}|[0-9a-f]{64}) ([0-9a-f]{40}|[0-9a-f]{64}) ([ACDMRTUXB][0-9]*)")
private fun metadata(bytes: ByteArray, include: ((String, String, String, String) -> Boolean)? = null): List<Meta> = buildList {
    val fields = Fields(bytes)
    while (fields.more()) {
        val start = fields.offset
        val m = rawHeader.matchEntire(fields.next())?.groupValues ?: error("Malformed raw Git metadata.")
        val first = fields.next().path(); val rename = m[5][0] in "RC"
        val path = if (rename) fields.next().path() else first
        if (include?.invoke(path, m[5], m[1], m[2]) == false) continue
        require(size < MAX_FILES * 2) { FILE_LIMIT }
        add(Meta(path, first.takeIf { rename }, m[5], m[1], m[2], bytes.copyOfRange(start, fields.offset)))
    }
}
private data class Status(val head: String?, val untracked: List<Meta>, val unstaged: Set<String>)
private val xyStatus = Regex("[.MTADRCU]{2}")
private val subStatus = Regex("(?:N\\.\\.\\.|S[.C][.M][.U])")
private val gitMode = Regex("[0-7]{6}")
private val renameScore = Regex("[RC](?:100|[0-9]{1,2})")
private fun porcelain(bytes: ByteArray): Status {
    var head: String? = null; var foundHead = false
    val untracked = mutableListOf<Meta>(); val fields = Fields(bytes)
    val unstaged = mutableSetOf<String>()
    while (fields.more()) {
        val start = fields.offset; val row = fields.next()
        when {
            row.startsWith("# branch.oid ") -> { require(!foundHead); foundHead = true; head = row.removePrefix("# branch.oid ").takeUnless { it == "(initial)" }?.oid() }
            row.startsWith("# ") -> Unit
            row.startsWith("? ") -> {
                require(untracked.size < MAX_FILES) { FILE_LIMIT }
                untracked += Meta(row.substring(2).path(directory = true), null, "?", null, null, bytes.copyOfRange(start, fields.offset))
            }
            row.startsWith("1 ") || row.startsWith("2 ") || row.startsWith("u ") -> {
                val count = when (row[0]) { '1' -> 9; '2' -> 10; else -> 11 }
                val p = row.split(' ', limit = count)
                val modes = if (row[0] == 'u') 4 else 3
                require(p.size == count && xyStatus.matches(p[1]) && subStatus.matches(p[2]) &&
                    (3 until 3 + modes).all { gitMode.matches(p[it]) } &&
                    (3 + modes until if (row[0] == '2') count - 2 else count - 1).all { oidPattern.matches(p[it]) }) { "Malformed Git status." }
                val path = p.last().path()
                if (p[1][1] != '.') unstaged += path
                if (row[0] == '2') { require(renameScore.matches(p[count - 2])); fields.next().path() }
            }
            else -> error("Malformed Git status.")
        }
    }
    check(foundHead) { "Git status is missing the checkout identity." }
    return Status(head, untracked, unstaged)
}

private suspend fun GitRead.capture(comparison: ReviewComparison): ReviewSnapshot {
    val repo = repo(); var base: String? = null; var merge: String? = null
    val sources = mutableListOf<ReviewSource>(); var entries = 0
    suspend fun diff(layer: ReviewLayer, opts: List<String>, revisions: List<String>, unstaged: Set<String>? = null) {
        val cmd = listOf("diff") + opts + diffFlags
        val raw = run(cmd + listOf("--raw", "-z", "--no-abbrev") + revisions + "--").second
        val patch = run(cmd + patchFlags + revisions + "--").second
        val headers = unstaged?.let { blocks(patch).mapTo(HashSet()) { it.header } }
        val files = metadata(raw) { path, status, before, after ->
            unstaged == null || status != "M" || before != after || path in unstaged || header(path, path) in headers.orEmpty()
        }
        entries += files.distinctBy { it.old to it.path }.size
        require(entries <= MAX_FILES) { FILE_LIMIT }
        check(files.isNotEmpty() || patch.isEmpty()) { MOVED }
        val size = files.sumOf { it.bytes.size }
        val retained = if (size == raw.size) raw else ByteArray(size).also { bytes ->
            var offset = 0
            for (file in files) {
                file.bytes.copyInto(bytes, offset)
                offset += file.bytes.size
            }
        }
        sources += ReviewSource(layer, null, retained.b64(), patch.b64())
    }
    val head: String?; var status = ""
    when (comparison.mode) {
        ReviewMode.WorkingTree -> {
            require(comparison.baseRef == null) { "Working tree reviews have no base ref." }
            val raw = out("status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all", "--ignore-submodules=none", "--renames")
            val s = porcelain(raw); head = s.head; status = raw.b64(); entries = s.untracked.size
            diff(ReviewLayer.Staged, listOf("--cached"), listOfNotNull(head))
            diff(ReviewLayer.Unstaged, emptyList(), emptyList(), s.unstaged)
            for (file in s.untracked) {
                val path = file.path
                val patch = if (path.endsWith('/')) byteArrayOf() else try {
                    val bytes = run(listOf("diff", "--no-index") + patchFlags + listOf("--no-color", "--no-ext-diff", "--no-textconv", "--", "/dev/null", "./$path"), setOf(0, 1)).second
                    validateUntracked(path, bytes); bytes
                } catch (e: CancellationException) { throw e } catch (e: Exception) { error("Cannot capture untracked path $path: ${e.message}") }
                sources += ReviewSource(ReviewLayer.Untracked, path, file.bytes.b64(), patch.b64())
            }
        }
        ReviewMode.Branch -> {
            val branch = branches().firstOrNull { it.ref == comparison.baseRef } ?: error("Select a locally available base branch.")
            check(out("rev-parse", "--is-shallow-repository").record() == "false") { "Branch comparison needs complete local history. Use Working tree for a shallow checkout." }
            val (code, rawHead) = run(listOf("rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"), setOf(0, 128))
            check(code == 0) { "Branch comparison needs a committed HEAD. Use Working tree instead." }; head = rawHead.record().oid()
            base = out("rev-parse", "--verify", "--end-of-options", "${branch.ref}^{commit}").record().oid()
            check(base == branch.oid) { MOVED }
            val (exit, bases) = run(listOf("merge-base", "--all", base, head), setOf(0, 1))
            check(exit == 0 && bases.record().lineSequence().count() == 1) { "Branch comparison requires one locally available merge base. Choose another base or use Working tree." }
            merge = bases.record().oid(); diff(ReviewLayer.Branch, emptyList(), listOf(merge, head))
        }
    }
    return ReviewSnapshot("", repo, comparison, head, base, merge, status, sources)
}

suspend fun captureReview(client: Client, machine: String?, folder: String, comparison: ReviewComparison): ReviewSnapshot = withContext(Dispatchers.Default) {
    val first = GitRead(client, machine, folder).capture(comparison)
    check(first == GitRead(client, machine, folder).capture(comparison)) { MOVED }
    first.copy(id = reviewSnapshotId(first))
}

private fun gitName(path: String): String {
    val bytes = path.encodeToByteArray()
    if (bytes.all { it.toInt() in 32..126 && it != 34.toByte() && it != 92.toByte() }) return path
    return buildString {
        append('"')
        for (b in bytes) when (val n = b.toInt() and 255) {
            7 -> append("\\a"); 8 -> append("\\b"); 9 -> append("\\t"); 10 -> append("\\n")
            11 -> append("\\v"); 12 -> append("\\f"); 13 -> append("\\r")
            34 -> append("\\\""); 92 -> append("\\\\")
            in 32..126 -> append(n.toChar())
            else -> { append('\\'); append(n.toString(8).padStart(3, '0')) }
        }
        append('"')
    }
}

private fun header(old: String, path: String) = "diff --git ${gitName("a/$old")} ${gitName("b/$path")}"
private data class Block(val header: String, val start: Int, val end: Int)
private fun blocks(bytes: ByteArray): List<Block> = buildList {
    var start = 0; var last = -1; var name = ""
    while (start < bytes.size) {
        var end = start
        while (end < bytes.size && bytes[end] != 10.toByte()) end++
        if (bytes[start] == 'd'.code.toByte() || bytes[start] == '*'.code.toByte()) {
            val row = bytes.decodeToString(start, end)
            if (row.startsWith("diff --") || row.startsWith("* Unmerged path ")) {
                require(size <= MAX_FILES * 2) { FILE_LIMIT }
                if (last >= 0) add(Block(name, last, start))
                last = start; name = row
            }
        }
        start = end + 1
    }
    if (last >= 0) add(Block(name, last, bytes.size))
}
private fun untrackedDestination(path: String, title: String): String {
    val dot = "./$path"
    return when (title) {
        header(dot, dot) -> dot
        header(path, path) -> path
        else -> error("Git returned an unexpected destination.")
    }
}
private fun validateUntracked(path: String, bytes: ByteArray) {
    val block = blocks(bytes).singleOrNull()
    require(block != null && block.start == 0) { "Git returned an unexpected destination." }
    val dest = untrackedDestination(path, block.header)
    val expected = gitName("b/$dest")
    val rows = bytes.toString(Charsets.ISO_8859_1).splitToSequence('\n').drop(1).takeWhile { !it.startsWith("@@") }
    var destination = false
    for (row in rows) when {
        row.startsWith("+++ ") -> {
            require(!destination && row.substring(4).removeSuffix("\t") == expected) { "Git returned an unexpected destination." }
            destination = true
        }
        row.startsWith("Binary files ") -> {
            require(!destination && row == "Binary files /dev/null and $expected differ") { "Git returned an unexpected binary destination." }
            destination = true
        }
    }
}

private val hunkHeader = Regex("""@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?: .*)?""", RegexOption.DOT_MATCHES_ALL)
private val indexHeader = Regex("""index (?:[0-9a-f]{40}|[0-9a-f]{64})\.\.(?:[0-9a-f]{40}|[0-9a-f]{64})(?: [0-7]{6})?""")
private val modeHeader = Regex("""(?:new file|deleted file|old|new) mode [0-7]{6}""")
private val similarity = Regex("""(?:dis)?similarity index (?:100|[0-9]{1,2})%""")
private fun hunks(rows: Iterator<String>): List<ReviewHunk> = buildList<ReviewHunk> {
    var beforeEnded = false; var afterEnded = false
    var row: String? = if (rows.hasNext()) rows.next() else null
    while (row != null) {
        val title = row
        val parts = hunkHeader.matchEntire(title)?.groupValues ?: error("Invalid hunk header.")
        val old = parts[1].toInt(); val oldCount = parts[2].ifEmpty { "1" }.toInt()
        val new = parts[3].toInt(); val newCount = parts[4].ifEmpty { "1" }.toInt()
        require((old > 0 || oldCount == 0) && (new > 0 || newCount == 0))
        require(old.toLong() + oldCount <= Int.MAX_VALUE && new.toLong() + newCount <= Int.MAX_VALUE)
        lastOrNull()?.let { require(old >= it.beforeStart + it.beforeCount && new >= it.afterStart + it.afterCount) }
        var before = old; var after = new; var changed = false
        val lines = mutableListOf<ReviewLine>()
        row = null
        while (rows.hasNext()) {
            val line = rows.next()
            if (line.startsWith("@@")) { row = line; break }
            if (line == "\\ No newline at end of file") {
                val previous = lines.last()
                require(!previous.noFinalNewline)
                if (previous.before != null) beforeEnded = true
                if (previous.after != null) afterEnded = true
                lines[lines.lastIndex] = previous.copy(noFinalNewline = true)
                continue
            }
            val text = line.substring(1)
            require((line[0] == '+' || !beforeEnded) && (line[0] == '-' || !afterEnded))
            lines += when (line[0]) {
                ' ' -> ReviewLine(text, before++, after++, ReviewLineKind.Context)
                '+' -> { changed = true; ReviewLine(text, null, after++, ReviewLineKind.Addition) }
                '-' -> { changed = true; ReviewLine(text, before++, null, ReviewLineKind.Deletion) }
                else -> error("Invalid hunk line.")
            }
            require(before - old <= oldCount && after - new <= newCount)
        }
        require(changed && before - old == oldCount && after - new == newCount)
        add(ReviewHunk(title, old, oldCount, new, newCount, lines))
    }
}

private fun display(meta: Meta, layer: ReviewLayer, raw: List<Meta>, bytes: ByteArray, unavailable: Boolean): ReviewFile {
    val untracked = layer == ReviewLayer.Untracked
    val key = ReviewFileKey(layer, meta.old, meta.path)
    val hash = digest { raw.forEach { field(it.bytes) }; field(bytes) }
    val conflict = raw.any { it.status == "U" }; val submodule = raw.any { it.before == "160000" || it.after == "160000" }
    val file = ReviewFile(key, if (conflict) "U" else meta.status, meta.before, meta.after, hash, false, submodule, conflict, emptyList())
    if (conflict) return file.copy(previewUnavailable = "Unresolved conflict; no inline preview.")
    if (submodule) return file.copy(previewUnavailable = "Gitlink; nested submodule contents are excluded.")
    if (untracked && meta.path.endsWith('/')) return file.copy(previewUnavailable = "Nested repository or directory; contents are excluded.")
    val unsupported = "Unsupported or malformed patch; no inline preview."
    if (unavailable || bytes.isEmpty()) return file.copy(previewUnavailable = unsupported)
    if (0.toByte() in bytes) return file.copy(binary = true, previewUnavailable = "Binary contents; no inline preview.")
    val patch = try { bytes.decodeToString(throwOnInvalidSequence = true) } catch (_: java.nio.charset.CharacterCodingException) {
        return file.copy(previewUnavailable = "Non-UTF-8 contents; no inline preview.")
    }
    return try {
        require(patch.endsWith('\n'))
        val rows = patch.dropLast(1).splitToSequence('\n').iterator()
        val title = rows.next()
        val path = if (untracked) untrackedDestination(meta.path, title) else meta.path; val old = meta.old ?: path
        require(title == header(old, path))
        val a = if (meta.status == "A" || untracked) "/dev/null" else gitName("a/$old")
        val b = if (meta.status == "D") "/dev/null" else gitName("b/$path")
        var beforeHeader = false; var afterHeader = false; var afterMode = meta.after; var metadataOnly = false
        while (rows.hasNext()) {
            val row = rows.next()
            when {
                row.startsWith("@@") -> {
                    require(beforeHeader && afterHeader)
                    return file.copy(afterMode = afterMode, hunks = hunks((sequenceOf(row) + rows.asSequence()).iterator()))
                }
                row.startsWith("--- ") -> { require(!beforeHeader && row.substring(4).removeSuffix("\t") == a); beforeHeader = true }
                row.startsWith("+++ ") -> { require(beforeHeader && !afterHeader && row.substring(4).removeSuffix("\t") == b); afterHeader = true }
                row == "Binary files $a and $b differ" -> {
                    require(!rows.hasNext()); return file.copy(afterMode = afterMode, binary = true, previewUnavailable = "Binary contents; no inline preview.")
                }
                modeHeader.matches(row) -> {
                    metadataOnly = true
                    if (untracked && row.startsWith("new file mode ")) afterMode = row.substringAfterLast(' ')
                }
                indexHeader.matches(row) || similarity.matches(row) -> Unit
                meta.old != null && (row == "rename from ${gitName(old)}" || row == "rename to ${gitName(path)}" ||
                    row == "copy from ${gitName(old)}" || row == "copy to ${gitName(path)}") -> metadataOnly = true
                else -> error("Unknown patch header.")
            }
        }
        require(metadataOnly && !beforeHeader && !afterHeader)
        file.copy(afterMode = afterMode, previewUnavailable = if (untracked) "Empty file." else "File metadata change; no text hunks.")
    } catch (_: RuntimeException) {
        file.copy(previewUnavailable = unsupported)
    }
}

fun reviewFiles(snapshot: ReviewSnapshot): List<ReviewFile> = buildList {
    for (source in snapshot.sources) {
        val bytes = source.patchBase64.bytes(); val raw = source.metadataBase64.bytes()
        if (source.layer == ReviewLayer.Untracked) {
            val path = requireNotNull(source.path).path(directory = true)
            require(raw.utf8() == "? $path\u0000")
            val meta = Meta(path, null, "?", null, null, raw)
            add(display(meta, source.layer, listOf(meta), bytes, false))
        } else {
            val groups = metadata(raw).groupBy { it.old to it.path }
            val names = mutableMapOf<String, Pair<String?, String>>()
            val ambiguous = mutableSetOf<String>()
            for ((key, metas) in groups) {
                val name = header(key.first ?: key.second, key.second)
                if (names.put(name, key) != null) ambiguous += name
                if (metas.any { it.status == "U" }) {
                    names["diff --cc ${gitName(key.second)}"] = key
                    names["diff --combined ${gitName(key.second)}"] = key
                    names["* Unmerged path ${key.second}"] = key
                }
            }
            val chunks = blocks(bytes)
            val unknown = (bytes.isNotEmpty() && chunks.firstOrNull()?.start != 0) || chunks.any { it.header !in names || it.header in ambiguous }
            val byPath = chunks.groupBy { names[it.header] }
            for ((key, metas) in groups) {
                val parts = byPath[key].orEmpty()
                val patch = when {
                    unknown -> bytes
                    parts.isEmpty() -> byteArrayOf()
                    parts.size == 1 -> bytes.copyOfRange(parts[0].start, parts[0].end)
                    else -> java.io.ByteArrayOutputStream().apply { parts.forEach { write(bytes, it.start, it.end - it.start) } }.toByteArray()
                }
                add(display(metas.first(), source.layer, metas, patch, unknown || parts.size != 1))
            }
        }
        require(size <= MAX_FILES) { FILE_LIMIT }
    }
}
