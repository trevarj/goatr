package io.github.trevarj.goatr.app.session

import herdr.core.Client
import herdr.core.Duplex
import herdr.core.FAKE_HELLO
import herdr.core.FakeCore
import herdr.core.Space as CoreSpace
import herdr.core.TerminalRecord
import herdr.core.connect
import herdr.core.fault
import io.github.trevarj.goatr.app.agent
import io.github.trevarj.goatr.app.snapshot
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import kotlin.coroutines.ContinuationInterceptor
import kotlin.test.*

class ReviewTest {
    private val s = ReviewSnapshot("a".repeat(64), ReviewRepo("/repo", "/git", "/common"), ReviewComparison(ReviewMode.WorkingTree), "head", null, null, "", emptyList())
    private val staged = ReviewFileKey(ReviewLayer.Staged, "old\tpath", "new\npath")
    private val file = ReviewFile(staged, "R", "100644", "100644", "hash1", false, false, false,
        listOf(ReviewHunk("@@", 1, 10, 1, 11, (1..10).map { ReviewLine("original $it", it, it, ReviewLineKind.Context) } + ReviewLine("added", null, 11, ReviewLineKind.Addition)),
            ReviewHunk("@@", 100, 1, 101, 1, listOf(ReviewLine("distant", 100, 101, ReviewLineKind.Context)))))
    private val anchor = ReviewAnchor(s.id, staged, ReviewSide.After, 5, 6)
    private val comment = ReviewComment("comment", anchor, "explain\u001b[31m\r\nthis\u007f\u009b")
    private val r = ReviewRecord("record", "one beta/sp1", s.repo, s.comparison, s.id, 1, 2, comments = listOf(comment),
        recipientAddress = "one beta/t1", recipientAgent = "claude-code")
    private val env = ReviewEnvelope("one beta/t1", "claude-code", ReviewIntent.Discuss, "Goatr code review\nexact batch", listOf(s.id))

    @Test fun anchorUsesOneCapturedSideLayerAndHunk() {
        fun valid(a: ReviewAnchor) = validReviewAnchor(a, s.id, listOf(file))
        assertTrue(valid(anchor)); assertTrue(valid(anchor.copy(side = ReviewSide.Before)))
        assertFalse(valid(anchor.copy(snapshotId = "different")))
        assertFalse(valid(anchor.copy(file = staged.copy(layer = ReviewLayer.Unstaged))))
        assertFalse(valid(anchor.copy(endLine = 101))); assertFalse(valid(anchor.copy(startLine = 0)))
        assertFalse(valid(anchor.copy(side = ReviewSide.Before, endLine = 11)))
        assertFalse(valid(anchor.copy(side = null))); assertTrue(valid(anchor.copy(side = null, startLine = null, endLine = null)))
        assertFails { listOf(r).replaceReview(r, r.copy(spaceId = "two beta/sp1")) }
        assertFails { listOf(r).replaceReview(r, r.copy(comparison = ReviewComparison(ReviewMode.Branch, "refs/heads/main"))) }
    }

    @Test fun refreshRetainsOriginalCodeUntilAnExplicitReanchor() {
        val fresh = s.copy(id = "b".repeat(64)); val f = file.copy(patchHash = "changed")
        val fileComment = comment.copy(id = "file", anchor = anchor.copy(side = null, startLine = null, endLine = null))
        val draft = r.copy(comments = listOf(comment, fileComment))
        val unchanged = reconcileReview(draft, fresh, listOf(file), listOf(file))
        assertTrue(unchanged.comments.all { it.anchor.snapshotId == fresh.id && !it.needsReanchor })
        val updated = reconcileReview(draft, fresh, listOf(file), listOf(f))
        assertEquals(comment.copy(needsReanchor = true), updated.comments[0]); assertEquals(fresh.id, updated.comments[1].anchor.snapshotId)
        assertEquals(setOf(s.id, fresh.id), updated.snapshots()); assertFails { reviewBatch(updated, fresh, listOf(f)) }
        assertFails { updated.copy(composer = ReviewComposer(comment.id, anchor, "new")).saveReviewComment(listOf(f)) }
        val fixed = updated.copy(composer = ReviewComposer(comment.id, anchor.copy(snapshotId = fresh.id), "new")).saveReviewComment(listOf(f))
        assertEquals(setOf(fresh.id), fixed.snapshots()); assertNull(fixed.composer); assertFalse(fixed.comments[0].needsReanchor)
        assertContains(reviewBatch(fixed, fresh, listOf(f)), "new")
        val removed = updated.copy(comments = listOf(updated.comments[1])); assertContains(reviewBatch(removed, fresh, listOf(f)), "File comment")
    }

    @Test fun unfinishedComposerIsDurableButNotASavedComment() {
        val editing = r.copy(composer = ReviewComposer(comment.id, anchor, "work in progress"))
        val restored = Json.decodeFromString<ReviewRecord>(Json.encodeToString(editing))
        assertEquals(editing, restored); assertEquals(comment, restored.comments.single())
        assertFailsWith<IllegalStateException> { reconcileReview(restored, s, listOf(file), listOf(file)) }
        assertFailsWith<IllegalStateException> { reviewBatch(restored, s, listOf(file)) }
        assertFails { restored.copy(composer = restored.composer!!.copy(text = " \n")).saveReviewComment(listOf(file)) }
        val saved = restored.saveReviewComment(listOf(file)); assertNull(saved.composer)
        assertEquals(comment.copy(text = "work in progress"), Json.decodeFromString<ReviewRecord>(Json.encodeToString(saved)).comments.single())
        assertEquals(r, restored.copy(composer = null))
    }

    @Test fun batchUsesOnlyClippedCodeAndEscapesControlCharacters() {
        val batch = reviewBatch(r, s, listOf(file))
        assertContains(batch, "old\\tpath → new\\npath"); assertContains(batch, "After lines 5–6")
        assertContains(batch, "> 5 | original 5"); assertContains(batch, "  2 | original 2"); assertContains(batch, "  9 | original 9")
        assertFalse("1 | original 1" in batch); assertFalse("original 10" in batch); assertFalse("distant" in batch)
        assertContains(batch, "explain\\u001b[31m\nthis\\u007f\\u009b"); assertFalse('\u001b' in batch)
        assertContains(batch, "Discuss these comments; do not modify files yet."); assertContains(reviewBatch(r.copy(intent = ReviewIntent.Apply), s, listOf(file)), "Apply the review feedback")
        assertFails { reviewBatch(r.copy(message = "é".repeat(131072)), s, listOf(file)) }
    }

    @Test fun eligibilityUsesOriginalSpaceMachineAndRawReportingRecords() {
        fun terminal(n: Int, status: String, child: Boolean = false) = agent("beta/t$n", status, space = "beta/sp1", machine = "beta").copy(record = TerminalRecord(buildJsonObject {
            put("state", if (n == 8) "finished" else "live"); if (n != 9) put("pid", 14)
            putJsonObject("status") { put("app", "claude-code"); put("state", status) }
            if (child) putJsonObject("records") { putJsonObject("job") { put("app", "another-agent"); put("state", "working") } }
        }))
        val terminals = listOf("idle", "done", "unknown", "working", "blocked", "error", "", "idle", "idle").mapIndexed { i, state -> terminal(i+1, state) }
        val sp = CoreSpace("beta/sp1", "Space", "", "beta", "/repo", emptyList(), null)
        val snap = snapshot().copy(spaces = listOf(sp), terminals = terminals + terminal(10, "idle").copy(space = "beta/other") + terminal(11, "idle").copy(machine = "elsewhere"))
        val (listed, working) = reviewRecipients(r.spaceId, snap, true)
        assertEquals(listOf("one beta/t1", "one beta/t2"), listed.filter { it.canSend }.map { it.address }); assertEquals(9, listed.size); assertTrue(working)
        assertTrue(reviewRecipients(r.spaceId, snap.copy(terminals = listOf(terminal(1, "idle", child = true))), true).second)
        assertFalse(reviewRecipients(r.spaceId, snap, false).first.any { it.canSend }); assertTrue(reviewRecipients("one beta/other", snap, true).first.isEmpty())
        assertTrue(reviewRecipients("two beta/sp1", snap, true).first.none { it.address == r.recipientAddress })
    }

    @Test fun oldAutosaveAndConcurrentPreflightAreCompareAndSet() = runBlocking {
        val fresh = r.copy(snapshotId = "fresh"); val sent = r.copy(phase = ReviewPhase.Sending, envelope = env)
        for (newer in listOf(fresh, sent)) assertFails { listOf(newer).replaceReview(r, r.copy(message = "stale autosave")) }
        var saved = listOf(r.copy(message = "updated during preflight")); var inputs = 0
        val result = deliverReview(r, env, { saved = saved.replaceReview(r, it) }, { inputs++; ReviewSendResult(1, true) })
        assertNotNull(result.error); assertEquals(0, inputs); assertEquals("updated during preflight", saved.single().message)
    }

    @Test fun writeFailureBeforeSendAndAfterReceiptNeverOffersAnAutomaticRetry() = runBlocking {
        var inputs = 0; var disk = r
        val before = deliverReview(r, env, { error("disk full") }, { inputs++; ReviewSendResult(2, true) })
        assertEquals(0, inputs); assertNotNull(before.saveError); assertEquals(r, before.record)
        val final = deliverReview(r, env, { if (it.phase != ReviewPhase.Sending) error("disk full"); disk = it }, { inputs++; ReviewSendResult(2, true) })
        assertEquals(1, inputs); assertNotNull(final.saveError); assertEquals(ReviewPhase.Sending, disk.phase)
        assertEquals(final.record, Json.decodeFromString<ReviewRecord>(Json.encodeToString(disk)).recovered())
        assertEquals(env, final.record.envelope)
        val retry = deliverReview(disk, env, { fail("unexpected write") }, { fail("unexpected input") })
        assertEquals(ReviewPhase.DeliveryUnknown, retry.record.phase)
    }

    @Test fun nativeBlockedAndStartedFalseHaveDifferentOutcomes() = runBlocking {
        val loop = coroutineContext[ContinuationInterceptor] as CoroutineDispatcher
        val owned = CoroutineScope(SupervisorJob() + loop); val core = FakeCore(owned, hello = FAKE_HELLO.copy(methods = FAKE_HELLO.methods + "terminal.send"))
        val client = connect(core.duplex, dispatcher = loop); var disk = r; var calls = 0
        try {
            core.handlers["terminal.send"] = { params ->
                assertEquals(ReviewPhase.Sending, disk.phase); assertEquals(env, disk.envelope)
                assertEquals(buildJsonObject { put("text", env.batch); put("terminal", "beta/t1"); put("force", false); put("wait", false) }, params)
                if (++calls == 1) throw fault("blocked")
                buildJsonObject { put("turn", 7); put("started", false) }
            }
            suspend fun submit() = deliverReview(r, env, { disk = it }, { sendReviewInput(client, it) })
            assertEquals(r, submit().record); assertEquals(r, disk)
            val done = submit(); assertEquals(2, calls); assertEquals(ReviewPhase.Sent, done.record.phase)
            assertEquals(env.copy(turn = 7, started = false), disk.envelope)
        } finally { client.close(); owned.cancel() }
    }

    @Test fun lossOrCancellationAfterNativeReceiptIsUnknownAndNeverReplayed() = runBlocking {
        val loop = coroutineContext[ContinuationInterceptor] as CoroutineDispatcher
        for (cancel in listOf(true, false)) withTimeout(5_000) {
            val owned = CoroutineScope(SupervisorJob() + loop); val core = FakeCore(owned, hello = FAKE_HELLO.copy(methods = FAKE_HELLO.methods + "terminal.send"))
            val received = CompletableDeferred<Unit>(); var calls = 0; var disk = r
            val lostReply = object : Duplex by core.duplex {
                override fun onData(listener: (ByteArray) -> Unit) { core.duplex.onData { if (!received.isCompleted) listener(it) } }
            }
            val client: Client = connect(lostReply, dispatcher = loop)
            try {
                core.handlers["terminal.send"] = {
                    calls++; received.complete(Unit); buildJsonObject { put("turn", 1); put("started", true) }
                }
                val outcome = async { deliverReview(r, env, { disk = it }, { sendReviewInput(client, it) }) }
                received.await(); assertEquals(ReviewPhase.Sending, disk.phase)
                if (cancel) { outcome.cancelAndJoin(); assertTrue(outcome.isCancelled) } else { core.drop(); assertNotNull(outcome.await().error) }
                assertEquals(ReviewPhase.DeliveryUnknown, disk.phase); assertEquals(env, disk.envelope)
                val restored = Json.decodeFromString<ReviewRecord>(Json.encodeToString(disk)).recovered()
                assertEquals(disk, deliverReview(restored, env, { fail("replayed persist") }, { fail("replayed send") }).record); assertEquals(1, calls)
            } finally { client.close(); owned.cancel() }
        }
    }
}
