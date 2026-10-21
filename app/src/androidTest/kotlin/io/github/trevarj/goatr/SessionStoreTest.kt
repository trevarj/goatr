// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.BeforeClass
import org.junit.Test

class SessionStoreTest {
    companion object {
        private const val HOST = "523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        @BeforeClass @JvmStatic fun nativeContext() { IrohAndroid.installAndroidContext(InstrumentationRegistry.getInstrumentation().targetContext.applicationContext) }
    }
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun summary(provider: Provider = Provider.Codex): SessionSummary = SessionSummary(Uuid.random(),Uuid.random(),Uuid.random(),Id("workspace"),Id("terminal"),Id("pane"),provider,Id("native"),"test",SessionStatus.Idle,SessionConnectivity.Ready,
        Capabilities(true,true,true,true,PromptMode.SubmitOrSteer,if(provider == Provider.Omp) CancelMode.CurrentWork else CancelMode.NativeTurn),
        HistoryCoverage(if(provider == Provider.Omp) HistoryScope.AllBranches else HistoryScope.Thread,if(provider == Provider.Omp) HistorySource.PersistedAndLive else HistorySource.NativePages,false,false,false))
    private fun record(n: Long, source: RecordSource = RecordSource.Live, text: String = "display"): SafeDisplayHistory = SafeDisplayHistory.redacted(HistoryRecord(Id("item-$n"),order=Uint(n),role=Role.Assistant,kind=RecordKind.Message,text=ContentText(InlineText(text)),source=source,complete=true,sourceTruncated=false))
    private fun prompt(summary: SessionSummary,text: String = "ordinary composer recovery"): Request = Request(Uuid.random(),RequestBody.PromptSubmit(PromptSubmitParams(Uuid.random(),Target(summary.sessionId,summary.generation),PromptText(text))))
    private fun receipt(request: Request,state: ReceiptState,time: Long = 1): Receipt {
        val p=(request.body as RequestBody.PromptSubmit).params
        return Receipt(request.id,ActionMethod.PromptSubmit,p.bootId,p.target,Uint(0),Uint(time),state)
    }
    private suspend fun expectPolicy(code: ErrorCode, block: suspend () -> Unit) { try { block(); fail("expected policy failure") } catch(e: StorePolicy) { assertEquals(code,e.code) } }
    private suspend fun withStore(block: suspend (SessionStore, String, () -> Long, (Long) -> Unit) -> Unit) {
        val name="state-${Uuid.random().value}.db"; var time=0L; val store=SessionStore(context,name) { time }
        try { store.open(); store.host(HostMetadata(HOST,"host",Uuid.random(),listOf("goatr.test.endpoint","goatr.test.receipt"))); block(store,name,{time},{time=it}) } finally { store.close(); context.deleteDatabase(name) }
    }
    private suspend fun approved(store: SessionStore, request: Request, replace: Set<RecoveryIdentity> = emptySet()): PreSendDecision {
        val text=(request.body as RequestBody.PromptSubmit).params.text
        val warning=store.recoveryWarning(HOST,request.id,text)
        return store.preSend(HOST,request,ByteArray(32){7},true,RecoveryApproval(replace,warning.futureExpiry))
    }
    private fun referenced(n: Long, reference: ContentRef): SafeDisplayHistory = SafeDisplayHistory.redacted(record(n).record.copy(text=reference))
    @Test fun crashReopenKeepsOriginalRecoveryAndLocalReceiptNeverErases() = runBlocking {
        withStore { store,name,clock,_ ->
            val summary=summary(); val request=prompt(summary); store.cache(HOST,summary,listOf(record(0)),false)
            approved(store,request)
            store.receipt(HOST,receipt(request,ReceiptState.Reserved),authoritative=false)
            assertEquals(request.id,store.state.value.recovery.single().action.actionId)
            store.close()
            val reopened=SessionStore(context,name,clock)
            try {
                reopened.open(); val recovery=reopened.state.value.recovery.single()
                assertEquals(Target(summary.sessionId,summary.generation),recovery.action.target)
                assertTrue(reopened.state.value.cached.single().stale)
                assertEquals("ordinary composer recovery",recovery.text)
                expectPolicy(ErrorCode.NotReady) { reopened.outcomeShown(HOST,request.id,receipt(request,ReceiptState.Dispatched)) }
                val dispatched=receipt(request,ReceiptState.Dispatched,2)
                reopened.receipt(HOST,dispatched,authoritative=true)
                assertEquals(1,reopened.state.value.recovery.size)
                reopened.outcomeShown(HOST,request.id,dispatched)
                assertTrue(reopened.state.value.recovery.isEmpty())
                assertEquals(dispatched,reopened.actionStatus(HOST,request.id))
            } finally { reopened.close() }
        }
    }
    @Test fun uncertainFailedRecoveryNeverRetargetsAndSecretAnswersAreExcluded() = runBlocking {
        withStore { store,name,_,_ ->
            val summary=summary(); val request=prompt(summary)
            approved(store,request)
            store.receipt(HOST,receipt(request,ReceiptState.Uncertain),authoritative=true)
            store.outcomeShown(HOST,request.id,receipt(request,ReceiptState.Uncertain))
            val replacement=summary.copy(generation=Uuid.random()); store.cache(HOST,replacement,listOf(record(1)),true,snapshot=true)
            assertEquals(summary.generation,store.state.value.recovery.single().action.target!!.generation)
            val conflicting=Request(request.id,(request.body as RequestBody.PromptSubmit).copy(params=(request.body as RequestBody.PromptSubmit).params.copy(text=PromptText("different"))))
            expectPolicy(ErrorCode.ActionIdConflict) { store.preSend(HOST,conflicting,ByteArray(32){7},true) }
            val secret="never-persist-this-secret-answer"
            val answerRequest=Request(Uuid.random(),RequestBody.RequestRespond(RequestRespondParams(Uuid.random(),Target(summary.sessionId,summary.generation),Uuid.random(),AnswerOmpEditor(SubmitAction.Submit,PromptText(secret)))))
            store.preSend(HOST,answerRequest,ByteArray(32){7})
            assertEquals(1,store.state.value.recovery.size)
            val bytes=context.getDatabasePath(name).readBytes().toString(Charsets.ISO_8859_1)
            assertFalse(bytes.contains(secret))
        }
    }
    @Test fun recoveryCapacityAndExpiryRequireExactExplicitApproval() = runBlocking {
        withStore { store,_,_,advance ->
            val summary=summary()
            repeat(20) { approved(store,prompt(summary,"draft-$it")) }
            val next=prompt(summary,"replacement")
            var warning: RecoveryWarning?=null
            try { store.preSend(HOST,next,ByteArray(32){7},true); fail("must warn before replacing") } catch(e: RecoveryApprovalRequired) { warning=e.warning }
            assertEquals(20,store.state.value.recovery.size); assertEquals(1,warning!!.replace.size)
            store.preSend(HOST,next,ByteArray(32){7},true,RecoveryApproval(warning!!.replace,warning!!.futureExpiry))
            assertEquals(20,store.state.value.recovery.size)
            advance(SessionStore.RECOVERY_AGE)
            store.open(); assertTrue(store.state.value.recovery.isEmpty())
            val fresh=prompt(summary); val expiry=store.recoveryWarning(HOST,fresh.id,PromptText("new"))
            assertEquals(RecoveryIdentity(HOST,fresh.id),expiry.futureExpiry!!.identity)
            val name="size-${Uuid.random().value}.db"; val bytes=SessionStore(context,name)
            try {
                bytes.open(); bytes.host(HostMetadata(HOST,"host"))
                repeat(4) { approved(bytes,prompt(summary,"x".repeat(Protocol.ANSWER_MAX))) }
                try { approved(bytes,prompt(summary,"x")); fail("1MiB boundary") } catch(e: RecoveryApprovalRequired) { assertEquals(1,e.warning.replace.size) }
                assertEquals(SessionStore.RECOVERY_BYTES,bytes.state.value.recovery.sumOf { utf8Size(it.text).toLong() })
            } finally { bytes.close(); context.deleteDatabase(name) }
        }
    }
    @Test fun recordLimitSourceMergeGenerationAndAgeRemainTruthfullyPartial() = runBlocking {
        withStore { store,_,_,advance ->
            val summary=summary()
            store.cache(HOST,summary,(0L..1000L).map { record(it,RecordSource.NativePage,"older") },false)
            val cached=store.state.value.cached.single(); assertEquals(1000,cached.history.size); assertEquals(Uint(1),cached.history.first().order); assertTrue(cached.partial)
            store.cache(HOST,summary,listOf(record(1000,RecordSource.Live,"new live")),true)
            store.cache(HOST,summary,listOf(record(1000,RecordSource.NativePage,"old native page")),true)
            assertEquals(ContentText(InlineText("new live")),store.state.value.cached.single().history.last().text)
            val replaced=summary.copy(generation=Uuid.random()); store.cache(HOST,replaced,listOf(record(0)),true,snapshot=true)
            assertEquals(1,store.state.value.cached.single().history.size)
            advance(SessionStore.CACHE_AGE); store.open(); assertTrue(store.state.value.cached.single().history.isEmpty()); assertTrue(store.state.value.cached.single().partial)
        }
    }
    @Test fun contentTtlAndUnviewedSessionFirstEvictionAreRealSQLite() = runBlocking {
        withStore { store,_,_,advance ->
            val viewed=summary(); val other=summary(); val source=Uuid.random()
            store.cache(HOST,viewed,listOf(record(0)),false,snapshot=true,source=source); store.cache(HOST,other,listOf(record(0)),false,snapshot=true,source=source); store.viewing(HOST,viewed.sessionId)
            val text="x".repeat(16 * 1024 * 1024); val handles=mutableListOf<Uuid>()
            repeat(3) {
                val handle=Uuid.random(); handles+=handle
                val reference=ContentRef(handle,Uint(text.length.toLong()),Mime.TextPlain)
                store.cache(HOST,viewed,listOf(referenced(it.toLong()+1,reference)),false,source=source)
                store.cacheContent(HOST,Target(viewed.sessionId,viewed.generation),source,reference,text)
            }
            val evicted=Uuid.random(); val reference=ContentRef(evicted,Uint(text.length.toLong()),Mime.TextPlain)
            store.cache(HOST,other,listOf(referenced(1,reference)),false,source=source)
            store.cacheContent(HOST,Target(other.sessionId,other.generation),source,reference,text)
            assertNull(store.content(HOST,Target(other.sessionId,other.generation),source,evicted))
            assertTrue(store.state.value.cached.first { it.summary.sessionId==other.sessionId }.partial)
            assertEquals(text,store.content(HOST,Target(viewed.sessionId,viewed.generation),source,handles[0]))
            assertEquals(text,store.offlineContent(HOST,Target(viewed.sessionId,viewed.generation),handles[0]))
            advance(SessionStore.CONTENT_TTL)
            assertNull(store.content(HOST,Target(viewed.sessionId,viewed.generation),source,handles[0]))
            assertNull(store.offlineContent(HOST,Target(viewed.sessionId,viewed.generation),handles[0]))
        }
    }
    @Test fun authenticatedNotificationDedupReadPrivacyAndOfflineForgetAreAtomic() = runBlocking {
        withStore { store,name,clock,_ ->
            val summary=summary(); store.cache(HOST,summary,listOf(record(0)),false)
            val event=AttentionEvent(Uuid.random(),Uint(1),summary.sessionId,summary.generation,AttentionCategory.Attention,Uint(0))
            store.liveNotification(HOST,event); store.liveNotification(HOST,event)
            assertEquals(1,store.cachedNotifications(HOST).events.size)
            expectPolicy(ErrorCode.InvalidRequest) { store.readRequest(HOST,Uuid.random(),summary.sessionId,Uint(1)) }
            store.viewing(HOST,summary.sessionId)
            assertEquals(Uint(1),store.readRequest(HOST,Uuid.random(),summary.sessionId,Uint(1)).throughSequence)
            expectPolicy(ErrorCode.InvalidRequest) { store.readRequest(HOST,Uuid.random(),summary.sessionId,Uint(2)) }
            store.readConfirmed(HOST,ReadWatermark(summary.sessionId,Uint(1)))
            approved(store,prompt(summary))
            store.encryptedRegistration(HOST,Uuid.random(),byteArrayOf(1,2,3))
            val deletion=store.forget(HOST)
            assertEquals(listOf("goatr.test.endpoint","goatr.test.receipt"),deletion.aliases)
            assertTrue(store.state.value.hosts.isEmpty()); assertTrue(store.state.value.cached.isEmpty()); assertTrue(store.state.value.recovery.isEmpty())
            val reopened=SessionStore(context,name,clock)
            try {
                reopened.open(); assertEquals(deletion,reopened.state.value.keyDeletion.single())
                expectPolicy(ErrorCode.NotReady) { reopened.host(HostMetadata(HOST,"host")) }
                reopened.keysDeleted(deletion); assertTrue(reopened.state.value.keyDeletion.isEmpty())
            } finally { reopened.close() }
        }
    }
    @Test fun watermarkCapacityAndSecretPendingNeverPersist() = runBlocking {
        withStore { store,_,_,_ ->
            repeat(512) { n ->
                val event=AttentionEvent(Uuid.random(),Uint(n+1L),Uuid.random(),Uuid.random(),AttentionCategory.Attention,Uint(0))
                store.liveNotification(HOST,event); store.readConfirmed(HOST,ReadWatermark(event.sessionId,event.sequence))
            }
            val extra=AttentionEvent(Uuid.random(),Uint(513),Uuid.random(),Uuid.random(),AttentionCategory.Attention,Uint(0))
            store.liveNotification(HOST,extra)
            expectPolicy(ErrorCode.ResourceLimit) { store.readConfirmed(HOST,ReadWatermark(extra.sessionId,extra.sequence)) }
            assertEquals(Uint(513),store.cachedNotifications(HOST).latestSequence)
            val session=summary(Provider.Omp); val reducer=SessionReducer()
            reducer.snapshot(SessionOpenResult(Uuid.random(),ZeroValue(),SessionState(session,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle)),emptyList(),emptyList()))
            assertFalse(reducer.mutationsEnabled); reducer.recoveryReconciled(); assertTrue(reducer.mutationsEnabled)
            val state=SessionState(session,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Working))
            val subscription=Uuid.random(); reducer.snapshot(SessionOpenResult(subscription,ZeroValue(),state,emptyList(),emptyList())); reducer.recoveryReconciled()
            val event=Event(subscription,Uint(1),EventBody.ActivityUpdate(ActivityUpdateEvent(Activity(ActivityState.Idle))))
            assertEquals(SessionReducer.Decision.Applied,reducer.event(event)); assertEquals(SessionReducer.Decision.Duplicate,reducer.event(event))
            assertEquals(SessionReducer.Decision.Resnapshot,reducer.event(event.copy(seq=Uint(3)))); assertFalse(reducer.mutationsEnabled)
        }
    }
    @Test fun actionBoundConsentPersistsAndHardExpiryCannotRollBackWithRejectedSend() = runBlocking {
        withStore { store,name,clock,advance ->
            val request=prompt(summary()); val text=(request.body as RequestBody.PromptSubmit).params.text
            val warning=store.recoveryWarning(HOST,request.id,text)
            try { store.preSend(HOST,request,ByteArray(32){7},true); fail("must acknowledge visible expiry policy") } catch(e: RecoveryApprovalRequired) { assertEquals(warning.futureExpiry,e.warning.futureExpiry) }
            assertNull(store.localAction(HOST,request.id))
            val wrong=RecoveryExpiry(RecoveryIdentity(HOST,Uuid.random()),warning.futureExpiry!!.expiresAt)
            try { store.preSend(HOST,request,ByteArray(32){7},true,RecoveryApproval(expiry=wrong)); fail("approval cannot apply to another action") } catch(_: RecoveryApprovalRequired) { }
            assertTrue(store.preSend(HOST,request,ByteArray(32){7},true,RecoveryApproval(expiry=warning.futureExpiry)) is PreSendDecision.NewAction)
            assertEquals(warning,store.recoveryConsent(HOST,request.id)!!.warning)
            assertTrue(store.preSend(HOST,request,ByteArray(32){7},true) is PreSendDecision.ExistingAction)
            store.close()
            val reopened=SessionStore(context,name,clock)
            try {
                advance(warning.futureExpiry!!.expiresAt.value-1); reopened.open()
                assertEquals(warning.futureExpiry!!.expiresAt,reopened.state.value.recovery.single().expiresAt)
                advance(warning.futureExpiry!!.expiresAt.value)
                try { reopened.preSend(HOST,prompt(summary()),ByteArray(32){7},true); fail("new action has no expiry acknowledgment") } catch(_: RecoveryApprovalRequired) { }
                assertTrue(reopened.state.value.recovery.isEmpty())
                assertNotNull(reopened.localAction(HOST,request.id))
                assertNull(reopened.recoveryConsent(HOST,request.id))
            } finally { reopened.close() }
        }
    }
    @Test fun pendingContentIsNotCachedAndSourceEpochPreventsLateOldSourceWrites() = runBlocking {
        withStore { store,_,_,_ ->
            val summary=summary(); val target=Target(summary.sessionId,summary.generation); val source=Uuid.random()
            val text="x".repeat(Protocol.CONTENT_INLINE_MAX+1)
            val reference=ContentRef(Uuid.random(),Uint(text.length.toLong()),Mime.TextPlain)
            store.cache(HOST,summary,listOf(referenced(0,reference)),false,snapshot=true,source=source)
            val pendingReference=reference.copy(handle=Uuid.random())
            expectPolicy(ErrorCode.InvalidRequest) { store.cacheContent(HOST,target,source,pendingReference,text) }
            store.cacheContent(HOST,target,source,reference,text)
            assertEquals(text,store.offlineContent(HOST,target,reference.handle))
            store.invalidateSource(HOST,target)
            expectPolicy(ErrorCode.ContentExpired) { store.cacheContent(HOST,target,source,reference,text) }
            expectPolicy(ErrorCode.StaleSession) { store.cache(HOST,summary,listOf(record(1)),true,historical=true,source=source) }
            assertNull(store.offlineContent(HOST,target,reference.handle))
            val newSource=Uuid.random()
            store.cache(HOST,summary,listOf(record(0,RecordSource.Live,"new live")),false,snapshot=true,source=newSource)
            store.cache(HOST,summary,listOf(record(0,RecordSource.Live,"old cached page")),false,historical=true,source=newSource)
            assertEquals(ContentText(InlineText("new live")),store.state.value.cached.single().history.single().text)
        }
    }
    @Test fun utf8SplitAssemblyAndHistoricalLivePagesNeverRegressNewerRevision() = runBlocking {
        val target=Target(Uuid.random(),Uuid.random()); val reference=ContentRef(Uuid.random(),Uint(3),Mime.TextPlain)
        val assembly=ContentAssembly(target,reference)
        for((offset,chunk) in listOf("4g==","gg==","rA==").withIndex()) assembly.accept(ContentReadResult(Base64Bytes(chunk),Uint(offset+1L),offset==2))
        assertEquals("€",assembly.text()); assertEquals(Uint(3),assembly.request(1).offsetBytes)
        val session=summary(); val subscription=Uuid.random(); val reducer=SessionReducer()
        reducer.snapshot(SessionOpenResult(subscription,ZeroValue(),SessionState(session,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle)),emptyList(),listOf(record(0,RecordSource.Live,"new").record)))
        reducer.page(reducer.historyPage(Target(session.sessionId,session.generation)),SessionHistoryResult(listOf(record(0,RecordSource.Live,"old").record),Uuid.random()))
        assertEquals(ContentText(InlineText("new")),reducer.history.single().text)
        reducer.recoveryReconciled()
        assertEquals(SessionReducer.Decision.Resnapshot,reducer.event(Event(subscription,Uint(0),EventBody.ActivityUpdate(ActivityUpdateEvent(Activity(ActivityState.Idle))))))
        assertFalse(reducer.mutationsEnabled)
    }
    @Test fun review_paginated_sync_live_overlap_keeps_received_event_and_monotonic_read() = runBlocking {
        withStore { store,_,_,_ ->
            val session=summary(); store.cache(HOST,session,listOf(record(0)),false); store.viewing(HOST,session.sessionId)
            val first=AttentionEvent(Uuid.random(),Uint(1),session.sessionId,session.generation,AttentionCategory.Attention,Uint(0))
            val second=first.copy(eventId=Uuid.random(),sequence=Uint(2),category=AttentionCategory.Completed)
            val third=first.copy(eventId=Uuid.random(),sequence=Uint(3))
            store.notifications(HOST,NotificationsSyncResult(listOf(first),emptyList(),Uint(3),Uint(1),Uint(1),false))
            // latestSequence advertises the host's tail, not which page/live events we received.
            expectPolicy(ErrorCode.InvalidRequest) { store.readRequest(HOST,Uuid.random(),session.sessionId,Uint(2)) }
            store.liveNotification(HOST,second)
            store.liveNotification(HOST,second)
            store.liveNotification(HOST,third)
            assertEquals(listOf(first,second,third),store.cachedNotifications(HOST).events)
            assertEquals(Uint(3),store.readRequest(HOST,Uuid.random(),session.sessionId,Uint(3)).throughSequence)
            expectPolicy(ErrorCode.InvalidRequest) { store.readRequest(HOST,Uuid.random(),Uuid.random(),Uint(3)) }
            store.readConfirmed(HOST,ReadWatermark(session.sessionId,Uint(3)))
            // A page/response captured before the live read confirmation must not roll it back,
            // even when its advertised tail predates a newer delivered live event.
            store.notifications(HOST,NotificationsSyncResult(listOf(second),listOf(ReadWatermark(session.sessionId,Uint(1))),Uint(2),Uint(1),resetRequired=false))
            store.readConfirmed(HOST,ReadWatermark(session.sessionId,Uint(2)))
            assertEquals(listOf(first,second,third),store.cachedNotifications(HOST).events)
            assertEquals(Uint(3),store.cachedNotifications(HOST).latestSequence)
            assertEquals(listOf(ReadWatermark(session.sessionId,Uint(3))),store.cachedNotifications(HOST).read)
        }
    }
    @Test fun olderWindowRetainsRequestedRowsAcrossLiveUpdatesAndResetsOnlyOnSnapshot() = runBlocking {
        for(provider in listOf(Provider.Omp,Provider.Codex)) {
            val session=summary(provider); val target=Target(session.sessionId,session.generation)
            val state=SessionState(session,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle))
            val subscription=Uuid.random(); val reducer=SessionReducer()
            val snapshot=SessionOpenResult(subscription,ZeroValue(),state,emptyList(),(1L..100L).map {record(it).record})
            reducer.snapshot(snapshot); reducer.recoveryReconciled()
            for(batch in 1..9) {
                val records=(batch*100L+1..batch*100L+100).map {n->record(n).record.let {
                    if(n==500L) it.copy(kind=RecordKind.Tool,tool=Tool("shell",status=ToolStatus.Succeeded,output=ContentText(InlineText("new live output")))) else it
                }}
                assertEquals(SessionReducer.Decision.Applied,reducer.event(Event(subscription,Uint(batch.toLong()),EventBody.HistoryUpsert(HistoryUpsertEvent(records)))))
            }
            assertEquals(1000,reducer.history.size); assertFalse(reducer.browsingOlder)
            val ticket=reducer.historyPage(target)
            val old=record(0,if(provider==Provider.Omp) RecordSource.Persisted else RecordSource.NativePage).record
            val overlap=record(500,old.source).record.copy(kind=RecordKind.Tool,tool=Tool("shell",status=ToolStatus.Running,output=ContentText(InlineText("older output"))))
            reducer.page(ticket,SessionHistoryResult(listOf(old,overlap),Uuid.random(),Id("older-next")))
            assertTrue(reducer.browsingOlder); assertEquals(2,reducer.history.size)
            assertEquals(old.id,reducer.history.first().id); assertTrue(reducer.partial)
            assertEquals(ContentText(InlineText("new live output")),reducer.history.single {it.id==overlap.id}.tool!!.output)
            assertEquals(ToolStatus.Succeeded,reducer.history.single {it.id==overlap.id}.tool!!.status)
            assertEquals(SessionReducer.Decision.Applied,reducer.event(Event(subscription,Uint(10),EventBody.HistoryUpsert(HistoryUpsertEvent(listOf(record(1001).record))))))
            assertEquals(2,reducer.history.size); assertEquals(old.id,reducer.history.first().id)
            assertTrue(reducer.mutationsEnabled)
            reducer.snapshot(snapshot.copy(subscription=Uuid.random()))
            assertFalse(reducer.browsingOlder); assertEquals(100,reducer.history.size)
            expectPolicy(ErrorCode.CursorExpired) {reducer.page(ticket,SessionHistoryResult(listOf(old),Uuid.random()))}
            assertFalse(reducer.browsingOlder); assertFalse(reducer.history.any {it.id==old.id})
        }
    }
    @Test fun olderPagesContinuePastDisplayWindowAndKeepLiveUpdatesAndResetFences() = runBlocking {
        for(provider in listOf(Provider.Omp,Provider.Codex)) {
            val session=summary(provider); val target=Target(session.sessionId,session.generation)
            val state=SessionState(session,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle))
            val subscription=Uuid.random(); val reducer=SessionReducer()
            val snapshot=SessionOpenResult(subscription,ZeroValue(),state,emptyList(),(1L..100L).map {record(it).record})
            reducer.snapshot(snapshot); reducer.recoveryReconciled()
            for(batch in 1..12) {
                val records=(batch*100L+1..batch*100L+100).map {n->record(n).record}
                assertEquals(SessionReducer.Decision.Applied,reducer.event(Event(subscription,Uint(batch.toLong()),EventBody.HistoryUpsert(HistoryUpsertEvent(records)))))
            }
            assertEquals(1000,reducer.history.size); assertFalse(reducer.browsingOlder)
            val source=if(provider==Provider.Omp) RecordSource.Persisted else RecordSource.NativePage
            val view=Uuid.random(); var cursor: Id?=null
            repeat(13) { page ->
                val ticket=reducer.historyPage(target,cursor)
                val first=1201L-page.toLong()*100L
                val records=(first until first+100L).map {record(it,source).record}
                cursor=Id("older-${page+1}")
                reducer.page(ticket,SessionHistoryResult(records,view,cursor))
                assertTrue(reducer.browsingOlder); assertEquals(cursor,reducer.nextCursor)
            }
            assertEquals(1000,reducer.history.size); assertEquals(Id("item-1"),reducer.history.first().id); assertEquals(Id("item-1000"),reducer.history.last().id)
            val live=record(500).record.copy(kind=RecordKind.Tool,tool=Tool("shell",status=ToolStatus.Succeeded,output=ContentText(InlineText("new live output"))))
            assertEquals(SessionReducer.Decision.Applied,reducer.event(Event(subscription,Uint(13),EventBody.HistoryUpsert(HistoryUpsertEvent(listOf(live))))))
            assertEquals(ContentText(InlineText("new live output")),reducer.history.single {it.id==live.id}.tool!!.output)
            val terminal=reducer.historyPage(target,cursor)
            reducer.page(terminal,SessionHistoryResult(listOf(record(0,source).record),view))
            assertEquals(1000,reducer.history.size); assertEquals(Id("item-0"),reducer.history.first().id); assertEquals(Id("item-999"),reducer.history.last().id); assertEquals(null,reducer.nextCursor)
            assertTrue(reducer.partial)
            assertTrue(reducer.mutationsEnabled)
            reducer.snapshot(snapshot.copy(subscription=Uuid.random()))
            assertFalse(reducer.browsingOlder); assertEquals(100,reducer.history.size)
            expectPolicy(ErrorCode.CursorExpired) {reducer.page(terminal,SessionHistoryResult(listOf(record(0,source).record),view))}
            assertFalse(reducer.browsingOlder); assertFalse(reducer.history.any {it.id==Id("item-0")})
        }
    }
    @Test fun review_old_history_page_cannot_cross_source_replacement() = runBlocking {
        val session=summary(); val target=Target(session.sessionId,session.generation)
        val state=SessionState(session,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle))
        val reducer=SessionReducer(); val subscriptionA=Uuid.random()
        reducer.snapshot(SessionOpenResult(subscriptionA,ZeroValue(),state,emptyList(),listOf(record(0,RecordSource.NativePage,"source A").record)))
        reducer.recoveryReconciled()
        val ticketA=reducer.historyPage(target)
        val delayedA=SessionHistoryResult(listOf(record(1,RecordSource.NativePage,"A-only old page").record),Uuid.random(),Id("source-a-next"))
        assertEquals(SessionReducer.Decision.Resnapshot,reducer.event(Event(subscriptionA,Uint(1),EventBody.SyncRequired(SyncRequiredEvent(SyncReason.SourceGap)))))
        expectPolicy(ErrorCode.CursorExpired) { reducer.page(ticketA,delayedA) }
        expectPolicy(ErrorCode.CursorExpired) { reducer.historyPage(target) }
        val replacement=record(2,RecordSource.NativePage,"source B").record
        val cursorB=Id("source-b-older")
        reducer.snapshot(SessionOpenResult(Uuid.random(),ZeroValue(),state,emptyList(),listOf(replacement)))
        reducer.recoveryReconciled()
        // The request began under A; identical Target does not make its completion current.
        expectPolicy(ErrorCode.CursorExpired) { reducer.page(ticketA,delayedA) }
        assertEquals(listOf(replacement),reducer.history)
        assertEquals(null,reducer.nextCursor)
        assertTrue(reducer.mutationsEnabled)
        val currentPage=record(3,RecordSource.NativePage,"B older").record
        reducer.page(reducer.historyPage(target),SessionHistoryResult(listOf(currentPage),Uuid.random(),Id("source-b-next")))
        assertEquals(listOf(currentPage),reducer.history)
        assertEquals(Id("source-b-next"),reducer.nextCursor)
    }
    @Test fun review_authoritative_receipt_promotes_across_independent_clocks() = runBlocking {
        withStore { store,_,_,_ ->
            for(localState in listOf(ReceiptState.Reserved,ReceiptState.Uncertain)) {
                val request=prompt(summary()); approved(store,request)
                store.receipt(HOST,receipt(request,localState,1000),authoritative=false)
                val sent=receipt(request,ReceiptState.Dispatched,900)
                store.receipt(HOST,sent,authoritative=true)
                assertEquals(sent,store.actionStatus(HOST,request.id))
                assertTrue(store.localAction(HOST,request.id)!!.authoritative)
                assertTrue(store.state.value.recovery.any { it.action.actionId == request.id })
                expectPolicy(ErrorCode.InvalidRequest) { store.outcomeShown(HOST,request.id,sent.copy(updatedAt=Uint(901))) }
                assertTrue(store.state.value.recovery.any { it.action.actionId == request.id })
                store.outcomeShown(HOST,request.id,sent)
                assertFalse(store.state.value.recovery.any { it.action.actionId == request.id })
            }
        }
    }
    @Test fun review_equal_host_timestamp_cannot_regress_receipt() = runBlocking {
        withStore { store,_,_,_ ->
            val request=prompt(summary()); approved(store,request)
            val sent=receipt(request,ReceiptState.Dispatched,1000)
            store.receipt(HOST,sent,authoritative=true)
            store.receipt(HOST,sent,authoritative=true)
            store.receipt(HOST,receipt(request,ReceiptState.Reserved,1000),authoritative=true)
            store.receipt(HOST,receipt(request,ReceiptState.Uncertain,1001),authoritative=true)
            store.receipt(HOST,receipt(request,ReceiptState.Dispatching,999),authoritative=true)
            assertEquals(sent,store.actionStatus(HOST,request.id))
            assertTrue(store.state.value.recovery.any { it.action.actionId == request.id })
            store.outcomeShown(HOST,request.id,sent)
            assertTrue(store.state.value.recovery.isEmpty())
            store.receipt(HOST,sent,authoritative=true)
            assertTrue(store.localAction(HOST,request.id)!!.outcomeShown)
            val succeeded=sent.copy(state=ReceiptState.Succeeded)
            store.receipt(HOST,succeeded,authoritative=true)
            store.receipt(HOST,sent.copy(updatedAt=Uint(1001)),authoritative=true)
            store.receipt(HOST,succeeded.copy(state=ReceiptState.Failed,updatedAt=Uint(1002)),authoritative=true)
            assertEquals(succeeded,store.actionStatus(HOST,request.id))
            expectPolicy(ErrorCode.ActionIdConflict) { store.receipt(HOST,succeeded.copy(bootId=Uuid.random()),authoritative=true) }
            val unknown=prompt(summary()); store.preSend(HOST,unknown,ByteArray(32){7})
            val uncertain=receipt(unknown,ReceiptState.Uncertain,1000)
            store.receipt(HOST,uncertain,authoritative=true)
            val reconciled=uncertain.copy(state=ReceiptState.Succeeded)
            store.receipt(HOST,reconciled,authoritative=true)
            assertEquals(reconciled,store.actionStatus(HOST,unknown.id))
        }
    }
    @Test fun review_equal_host_timestamp_cannot_regress_creation_stage() = runBlocking {
        withStore { store,_,_,_ ->
            val params=SessionCreateParams(Uuid.random(),Uuid.random(),Uuid.random(),Provider.Codex,Label("review creation"))
            val request=Request(Uuid.random(),RequestBody.SessionCreate(params))
            store.preSend(HOST,request,ByteArray(32){7})
            val resources=CreationResources(workspaceId=Id("workspace"),paneId=Id("pane"),terminalId=Id("terminal"),ownerStarted=true)
            val progress=CreationProgress(params.serverId,params.projectId,params.provider,CreationStage.Owner,StageState.Confirmed,resources)
            val current=Receipt(request.id,ActionMethod.SessionCreate,params.bootId,createdAt=Uint(0),updatedAt=Uint(1000),state=ReceiptState.Dispatching,creation=progress)
            store.receipt(HOST,current,authoritative=true)
            store.receipt(HOST,current,authoritative=true)
            for(old in listOf(progress.copy(stageState=StageState.Dispatching),progress.copy(stage=CreationStage.Workspace))) {
                store.receipt(HOST,current.copy(creation=old),authoritative=true)
                assertEquals(current,store.actionStatus(HOST,request.id))
            }
            store.receipt(HOST,current.copy(updatedAt=Uint(1001),creation=progress.copy(resources=resources.copy(workspaceId=null))),authoritative=true)
            assertEquals(current,store.actionStatus(HOST,request.id))
            val thread=progress.copy(stage=CreationStage.Thread,stageState=StageState.Dispatching,resources=resources.copy(nativeId=Id("thread")))
            val next=current.copy(creation=thread)
            store.receipt(HOST,next,authoritative=true)
            assertEquals(next,store.actionStatus(HOST,request.id))
            store.receipt(HOST,current.copy(updatedAt=Uint(1002)),authoritative=true)
            assertEquals(next,store.actionStatus(HOST,request.id))
            val uncertain=next.copy(state=ReceiptState.Uncertain,creation=thread.copy(stageState=StageState.Uncertain))
            store.receipt(HOST,uncertain,authoritative=true)
            val target=Target(Uuid.random(),Uuid.random())
            val readyResources=thread.resources.copy(sessionId=target.sessionId,generation=target.generation,bindingId=Uuid.random(),relayStarted=true)
            val ready=thread.copy(stage=CreationStage.Ready,stageState=StageState.Confirmed,resources=readyResources)
            val reconciled=uncertain.copy(creation=ready)
            store.receipt(HOST,reconciled,authoritative=true)
            assertEquals(reconciled,store.actionStatus(HOST,request.id))
            val succeeded=next.copy(state=ReceiptState.Succeeded,creation=ready,result=ActionResultCreated(target,Id("workspace"),Id("terminal"),Id("pane"),Id("thread"),readyResources.bindingId))
            store.receipt(HOST,succeeded,authoritative=true)
            assertEquals(succeeded,store.actionStatus(HOST,request.id))
        }
    }
    @Test fun review_expired_watermarks_release_capacity_for_new_session() = runBlocking {
        withStore { store,_,_,advance ->
            val old=List(512) { n -> AttentionEvent(Uuid.random(),Uint(n+1L),Uuid.random(),Uuid.random(),AttentionCategory.Attention,Uint(0)) }
            for(event in old) {
                store.liveNotification(HOST,event)
                store.readConfirmed(HOST,ReadWatermark(event.sessionId,event.sequence))
            }
            val oldReads=old.map { ReadWatermark(it.sessionId,it.sequence) }
            advance(SessionStore.CACHE_AGE-24L*60*60*1000)
            store.notifications(HOST,NotificationsSyncResult(emptyList(),oldReads,Uint(512),Uint(1),resetRequired=false))
            assertEquals(512,store.cachedNotifications(HOST).read.size)
            val day31=SessionStore.CACHE_AGE+24L*60*60*1000; advance(day31)
            val fresh=AttentionEvent(Uuid.random(),Uint(513),Uuid.random(),Uuid.random(),AttentionCategory.Attention,Uint(day31))
            val freshRead=ReadWatermark(fresh.sessionId,fresh.sequence)
            // Complete authoritative read list omits the 512 sessions whose day-zero events expired.
            store.notifications(HOST,NotificationsSyncResult(listOf(fresh),listOf(freshRead),Uint(513),Uint(513),resetRequired=true))
            val cached=store.cachedNotifications(HOST)
            assertEquals(listOf(fresh),cached.events)
            assertEquals(listOf(freshRead),cached.read)
            assertEquals(Uint(513),cached.latestSequence)
        }
    }
    @Test fun retainedLocalActionsReopenPaginateByHostWithoutTextAndReconcileStatus() = runBlocking {
        withStore { store,name,clock,advance ->
            val otherHost="5866666666666666666666666666666666666666666666666666666666666666"
            store.host(HostMetadata(otherHost,"other host"))
            val session=summary(); val target=Target(session.sessionId,session.generation)
            val create=Request(Uuid.random(),RequestBody.SessionCreate(SessionCreateParams(Uuid.random(),Uuid.random(),Uuid.random(),Provider.Codex,Label("create"))))
            val secret="never-enumerate-or-persist-this-secret"
            val answer=AnswerCodexQuestions(listOf(QuestionAnswer(Id("secret"),listOf(secret))))
            val respond=Request(Uuid.random(),RequestBody.RequestRespond(RequestRespondParams(Uuid.random(),target,Uuid.random(),answer)))
            val cancel=Request(Uuid.random(),RequestBody.TurnCancel(TurnCancelParams(Uuid.random(),target,Id("turn"))))
            val composer=prompt(session,"ordinary text expires but its ID remains discoverable")
            val requests=listOf(create,respond,cancel)+List(101) { prompt(session,"not retained in metadata-$it") }+composer
            requests.dropLast(1).forEach { store.preSend(HOST,it,ByteArray(32){7}) }
            approved(store,composer)
            store.preSend(otherHost,create,ByteArray(32){8})
            store.close()
            advance(SessionStore.RECOVERY_AGE)
            val reopened=SessionStore(context,name,clock)
            try {
                reopened.open(); assertTrue(reopened.state.value.recovery.isEmpty())
                val first=reopened.localActions(HOST,null,100)
                val second=reopened.localActions(HOST,first.last().metadata.actionId,100)
                val all=first+second
                assertEquals(100,first.size); assertEquals(5,second.size)
                assertEquals(requests.map { it.id.value }.sorted(),all.map { it.metadata.actionId.value })
                assertTrue(reopened.localActions(HOST,all.last().metadata.actionId,100).isEmpty())
                assertEquals(listOf(create.id),reopened.localActions(otherHost,null,100).map { it.metadata.actionId })
                for(limit in listOf(0,101)) {
                    try { reopened.localActions(HOST,null,limit); fail("localActions must enforce 1..100") } catch(_: ProtocolFailure) { }
                }
                expectPolicy(ErrorCode.Unauthorized) { reopened.localActions("unknown",null,100) }
                assertTrue(all.all { it.receipt == null && !it.authoritative && !it.outcomeShown })
                val reconcileIds=setOf(create.id,respond.id,cancel.id,composer.id)
                for(action in all.filter { it.metadata.actionId in reconcileIds }) {
                    val m=action.metadata
                    val outcome=Receipt(m.actionId,m.method,m.bootId,m.target,Uint(0),Uint(clock()),ReceiptState.Uncertain)
                    reopened.receipt(HOST,outcome,authoritative=true)
                    assertEquals(outcome,reopened.localAction(HOST,m.actionId)!!.receipt)
                    assertEquals(outcome,reopened.actionStatus(HOST,m.actionId))
                }
                val reconciled=reopened.localActions(HOST,null,100)+reopened.localActions(HOST,first.last().metadata.actionId,100)
                assertEquals(all.map { it.metadata },reconciled.map { it.metadata })
                assertEquals(4,reconciled.count { it.authoritative && it.receipt != null })
                assertFalse(reconciled.toString().contains(secret))
                assertFalse(reconciled.toString().contains("ordinary text expires"))
                assertFalse(reconciled.toString().contains("fingerprint"))
                assertNull(reopened.localActions(otherHost,null,100).single().receipt)
                assertFalse(context.getDatabasePath(name).readBytes().toString(Charsets.ISO_8859_1).contains(secret))
            } finally { reopened.close() }
        }
    }
    @Test fun expiredNotificationReplayStaysConsumedAcrossReopenAndOlderReadRepliesStayMonotonic() = runBlocking {
        withStore { store,name,clock,advance ->
            val session=summary(); store.cache(HOST,session,listOf(record(0)),false)
            val old=AttentionEvent(Uuid.random(),Uint(1),session.sessionId,session.generation,AttentionCategory.Attention,Uint(0))
            store.liveNotification(HOST,old)
            advance(SessionStore.CACHE_AGE); store.open(); store.close()
            val reopened=SessionStore(context,name,clock)
            try {
                reopened.open(); reopened.viewing(HOST,session.sessionId)
                reopened.liveNotification(HOST,old)
                assertTrue(reopened.cachedNotifications(HOST).events.isEmpty())
                expectPolicy(ErrorCode.InvalidRequest) { reopened.readRequest(HOST,Uuid.random(),session.sessionId,old.sequence) }
                val next=old.copy(eventId=Uuid.random(),sequence=Uint(2),createdAt=Uint(clock()))
                val latest=next.copy(eventId=Uuid.random(),sequence=Uint(3))
                reopened.liveNotification(HOST,next)
                reopened.readConfirmed(HOST,ReadWatermark(session.sessionId,next.sequence))
                reopened.liveNotification(HOST,latest)
                reopened.readConfirmed(HOST,ReadWatermark(session.sessionId,latest.sequence))
                reopened.notifications(HOST,NotificationsSyncResult(listOf(next),listOf(ReadWatermark(session.sessionId,next.sequence)),Uint(2),Uint(2),resetRequired=true))
                reopened.readConfirmed(HOST,ReadWatermark(session.sessionId,next.sequence))
                assertEquals(listOf(next,latest),reopened.cachedNotifications(HOST).events)
                assertEquals(listOf(ReadWatermark(session.sessionId,latest.sequence)),reopened.cachedNotifications(HOST).read)
            } finally { reopened.close() }
        }
    }
    @Test fun authoritativeRetentionRetiresOmittedMarkersButNotNewerLiveReads() = runBlocking {
        withStore { store,_,_,_ ->
            val old=AttentionEvent(Uuid.random(),Uint(1),Uuid.random(),Uuid.random(),AttentionCategory.Attention,Uint(0))
            val current=old.copy(eventId=Uuid.random(),sequence=Uint(2),sessionId=Uuid.random())
            store.liveNotification(HOST,old); store.readConfirmed(HOST,ReadWatermark(old.sessionId,old.sequence))
            store.liveNotification(HOST,current); store.readConfirmed(HOST,ReadWatermark(current.sessionId,current.sequence))
            store.notifications(HOST,NotificationsSyncResult(listOf(current),listOf(ReadWatermark(current.sessionId,current.sequence)),Uint(2),Uint(2),resetRequired=true))
            assertEquals(listOf(ReadWatermark(current.sessionId,current.sequence)),store.cachedNotifications(HOST).read)
            val latest=current.copy(eventId=Uuid.random(),sequence=Uint(3))
            store.liveNotification(HOST,latest); store.readConfirmed(HOST,ReadWatermark(latest.sessionId,latest.sequence))
            // This older complete list was captured before the new live confirmation.
            store.notifications(HOST,NotificationsSyncResult(emptyList(),emptyList(),Uint(2),Uint(3),resetRequired=true))
            assertEquals(listOf(latest),store.cachedNotifications(HOST).events)
            assertEquals(listOf(ReadWatermark(latest.sessionId,latest.sequence)),store.cachedNotifications(HOST).read)
        }
    }
    @Test fun forgettingOneHostKeepsSharedEndpointUntilLastReference() = runBlocking {
        withStore { store,_,_,_ ->
            val second=computer.iroh.SecretKey.fromBytes(ByteArray(32){13}).use { it.public().use { id->id.toString() } }
            val shared="goatr.installation.endpoint"
            store.host(HostMetadata(HOST,"first",keyAliases=listOf(shared,"goatr.first.receipt")))
            store.host(HostMetadata(second,"second",keyAliases=listOf(shared,"goatr.second.receipt")))
            val first=store.forget(HOST)
            assertEquals(listOf("goatr.first.receipt"),first.aliases)
            assertEquals(listOf(second),store.state.value.hosts.map {it.hostId})
            assertTrue(shared in store.state.value.hosts.single().keyAliases)
            store.keysDeleted(first)
            val last=store.forget(second)
            assertEquals(listOf(shared,"goatr.second.receipt"),last.aliases)
            assertTrue(store.state.value.hosts.isEmpty())
            assertEquals(last,store.state.value.keyDeletion.single())
        }
    }
    @Test fun nativeInventoryCacheIsStaleAcrossReopenReplacementExpiryAndForget() = runBlocking {
        withStore { store,name,clock,advance ->
            val summary=summary(Provider.Omp)
            val target=Target(summary.sessionId,summary.generation)
            val tab=Id("native-tab")
            val graph=NativeHierarchy(summary.serverId,Uuid.random(),NativeFreshness.Current,
                workspaces=listOf(NativeWorkspace(summary.workspaceId,Uint(7),Label("Private native workspace label"),false,tab,NativeAgentStatus.Idle)),
                tabs=listOf(NativeTab(tab,summary.workspaceId,Uint(2),Label("Native tab"),false,NativeAgentStatus.Idle)),
                panes=listOf(NativePane(summary.paneId,summary.terminalId,summary.workspaceId,tab,agent=Label("omp"),focused=true,agentStatus=NativeAgentStatus.Working,target=target)))
            val inventory=HostSnapshotResult(Uuid.random(),ZeroValue(0),listOf(Server(summary.serverId,Label("Native server"),ServerConnectivity.Online,CanCreate(false,false))),emptyList(),listOf(summary),emptyList(),listOf(graph))
            store.cacheInventory(HOST,inventory)
            val cached=store.state.value.inventory.single()
            assertTrue(cached.stale)
            assertEquals(NativeFreshness.Stale,cached.inventory.hierarchy.single().freshness)
            assertNull(cached.inventory.hierarchy.single().panes.single().target)
            assertEquals(Label("Private native workspace label"),cached.inventory.hierarchy.single().workspaces.single().label)
            val reopened=SessionStore(context,name,clock)
            try {
                reopened.open()
                assertTrue(reopened.state.value.inventory.single().stale)
                assertNull(reopened.state.value.inventory.single().inventory.hierarchy.single().panes.single().target)
                val replaced=inventory.copy(sessions=listOf(summary.copy(connectivity=SessionConnectivity.Offline)),
                    hierarchy=listOf(graph.copy(serverIncarnation=Uuid.random(),panes=graph.panes.map { it.copy(target=null) })))
                reopened.cacheInventory(HOST,replaced)
                val replacement=reopened.state.value.inventory.single().inventory.hierarchy.single()
                assertEquals(replaced.hierarchy.single().serverIncarnation,replacement.serverIncarnation)
                assertTrue(replacement.panes.all { it.target == null && it.unboundRowId == null })
                advance(SessionStore.CACHE_AGE);reopened.open()
                assertTrue(reopened.state.value.inventory.isEmpty())
                reopened.cacheInventory(HOST,replaced)
                reopened.forget(HOST)
                assertTrue(reopened.state.value.inventory.isEmpty())
                assertFalse(context.getDatabasePath(name).readBytes().toString(Charsets.ISO_8859_1).contains("Private native workspace label"))
            } finally { reopened.close() }
        }
    }
}
