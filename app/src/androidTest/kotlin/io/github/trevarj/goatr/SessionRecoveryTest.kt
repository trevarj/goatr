// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Actual Compose receipt visibility drives actual SQLite outcomeShown; no native-provider claim. */
class SessionRecoveryTest {
    @get:Rule val ui=createComposeRule()
    @Test fun exactAuthoritativeDispatchedReceiptVisibilityRetainsItsRecoveryAcrossReopen() = runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        IrohAndroid.installAndroidContext(context)
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val name="surface-receipts-${Uuid.random().value}.db"
        val store=SessionStore(context,name)
        try {
            store.open();store.host(HostMetadata(host,"synthetic receipt host"))
            val target=Target(Uuid.random(),Uuid.random());val request=Request(Uuid.random(),RequestBody.PromptSubmit(PromptSubmitParams(Uuid.random(),target,PromptText("ordinary synthetic recovery, never secret"))))
            val warning=store.recoveryWarning(host,request.id,(request.body as RequestBody.PromptSubmit).params.text)
            store.preSend(host,request,ByteArray(32){7},true,RecoveryApproval(expiry=warning.futureExpiry))
            val initial=store.localAction(host,request.id)!!
            var current by mutableStateOf(initial)
            ui.setContent {MaterialTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                Spacer(Modifier.height(1800.dp))
                ReceiptPanel(host,current,foreground=true,onDisplayed={receipt->runBlocking {store.outcomeShown(host,receipt.actionId,receipt)}})
            }}}
            ui.onNodeWithText("Outcome unknown · local reservation is not Sent. Inspect only; never replay this UUID.").performScrollTo().assertExists()
            assertEquals(1,store.state.value.recovery.size)
            val receiptTime=System.currentTimeMillis()
            val failed=Receipt(request.id,ActionMethod.PromptSubmit,(request.body as RequestBody.PromptSubmit).params.bootId,target,Uint(receiptTime),Uint(receiptTime),ReceiptState.Failed)
            store.receipt(host,failed,true)
            ui.runOnIdle {current=runBlocking {store.localAction(host,request.id)!!}}
            ui.onNodeWithTag("receipt_exact").assertDoesNotExist()
            assertFalse(store.localAction(host,request.id)!!.outcomeShown)
            ui.onNodeWithTag("receipt_details_${request.id.value}").performScrollTo().performClick()
            ui.onNodeWithTag("receipt_exact").performScrollTo().assertTextEquals(Protocol.strictJson.encodeToString(failed))
            ui.waitForIdle();assertEquals(1,store.state.value.recovery.size)
            // A separate successful action avoids illegally changing a terminal Failed receipt.
            val successful=request.copy(id=Uuid.random())
            val nextWarning=store.recoveryWarning(host,successful.id,(successful.body as RequestBody.PromptSubmit).params.text)
            store.preSend(host,successful,ByteArray(32){7},true,RecoveryApproval(expiry=nextWarning.futureExpiry))
            val dispatched=failed.copy(actionId=successful.id,state=ReceiptState.Dispatched,updatedAt=Uint(System.currentTimeMillis()))
            store.receipt(host,dispatched,true)
            ui.runOnIdle {current=runBlocking {store.localAction(host,successful.id)!!}}
            ui.onNodeWithTag("receipt_exact").assertDoesNotExist()
            assertFalse(store.localAction(host,successful.id)!!.outcomeShown)
            assertTrue(store.state.value.recovery.any {it.action.actionId==successful.id})
            ui.onNodeWithTag("receipt_details_${successful.id.value}").performScrollTo().performClick()
            ui.onNodeWithTag("receipt_exact").performScrollTo().assertTextEquals(Protocol.strictJson.encodeToString(dispatched))
            ui.waitForIdle()
            assertEquals(listOf(request.id,successful.id),store.state.value.recovery.map {it.action.actionId})
            store.close()
            val reopened=SessionStore(context,name)
            try {reopened.open();assertTrue(reopened.localAction(host,successful.id)!!.outcomeShown);assertEquals(setOf(request.id,successful.id),reopened.state.value.recovery.map {it.action.actionId}.toSet());assertEquals(warning.futureExpiry!!.expiresAt,reopened.state.value.recovery.first {it.action.actionId==request.id}.expiresAt)} finally {reopened.close()}
        } finally {store.close();context.deleteDatabase(name)}
    }
    @Test fun imageDeliverySnapshotSurvivesDisplayedOutcomesAndExistingFingerprintNeverReservesAgain()=runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        IrohAndroid.installAndroidContext(context)
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val name="image-recovery-${Uuid.random().value}.db";var store=SessionStore(context,name)
        val target=Target(Uuid.random(),Uuid.random());val boot=Uuid.random();val images=listOf(Uuid.random(),Uuid.random())
        val options=ComposerOptions(images,PromptDelivery.Steer,Id("selected-original-turn"))
        val requests=mutableListOf<Request>()
        try {
            store.open();store.host(HostMetadata(host,"image recovery host"))
            for(state in listOf(ReceiptState.Dispatched,ReceiptState.Uncertain,ReceiptState.Succeeded)) {
                val params=PromptSubmitParams(boot,target,PromptText("original submitted text"),delivery=options.delivery,nativeTurnId=options.nativeTurnId,images=images)
                val request=Request(Uuid.random(),RequestBody.PromptSubmit(params));requests+=request
                val warning=store.recoveryWarning(host,request.id,params.text,options)
                store.preSend(host,request,ByteArray(32){7},true,RecoveryApproval(expiry=warning.futureExpiry))
                val time=Uint(System.currentTimeMillis())
                val receipt=Receipt(request.id,ActionMethod.PromptSubmit,boot,target,time,time,state)
                store.receipt(host,receipt,true);store.outcomeShown(host,request.id,receipt)
                val retained=store.state.value.recovery.single {it.action.actionId==request.id}
                assertEquals(params.text.value,retained.text);assertEquals(options,retained.options)
            }
            store.close();store=SessionStore(context,name);store.open()
            assertEquals(requests.size,store.state.value.recovery.size)
            for(request in requests) {
                assertEquals(options,store.state.value.recovery.single {it.action.actionId==request.id}.options)
                assertTrue(store.preSend(host,request,ByteArray(32){7},true) is PreSendDecision.ExistingAction)
            }
            store.discardRecovery(host,requests[0].id,true)
            assertEquals(requests.drop(1).map {it.id}.toSet(),store.state.value.recovery.map {it.action.actionId}.toSet())
        }finally {store.close();context.deleteDatabase(name)}
    }
    @Test fun explicitImageReselectionUsesFreshViewerWithoutRewritingUploadProvenance() {
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val key=ConversationKey(host,Target(Uuid.random(),Uuid.random()))
        val uploadSource=Uuid.random();val reopenedSource=Uuid.random()
        val record=LocalAttachmentUpload(host,key.target,uploadSource,Uuid.random(),null,32,"ab".repeat(32),committedBytes=32,published=true)
        val oldSelection=ComposerImageInput(record,key,uploadSource)
        assertFalse(oldSelection.current(key,reopenedSource))
        val reselected=ComposerImageInput(record,key,reopenedSource)
        assertTrue(reselected.current(key,reopenedSource))
        assertSame(record,reselected.record)
        assertEquals(uploadSource,reselected.record.source)
        assertFalse(reselected.current(key,Uuid.random()))
        assertFalse(reselected.current(key,null))
        assertFalse(reselected.current(key.copy(target=key.target.copy(generation=Uuid.random())),reopenedSource))
        assertFalse(reselected.current(key.copy(host="a".repeat(64)),reopenedSource))
        assertFalse(reselected.copy(record=record.copy(target=Target(Uuid.random(),Uuid.random()))).current(key,reopenedSource))
        assertFalse(reselected.copy(record=record.copy(discarded=true)).current(key,reopenedSource))
        assertFalse(reselected.copy(record=record.copy(published=false)).current(key,reopenedSource))
    }

    @Test fun nativeChangesDuringReadsSurviveFailureAndRetirementCannotReopenPanel() {
        val key=ConversationKey("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea",Target(Uuid.random(),Uuid.random()))
        val source=Uuid.random();val shown=SessionControlsState(key,source,Any())
        shown.queueOpen=true
        assertTrue(shown.beginControlsRead(10))
        assertTrue(shown.beginQueueRead(20))
        shown.observeRevisions(11,21)
        shown.observeRevisions(12,22)
        assertFalse(shown.controlsReadCurrent(10))
        assertFalse(shown.queueReadCurrent(20))
        assertFalse(shown.controlsRefreshNeeded)
        assertFalse(shown.queueRefreshNeeded)
        // The stale reads fail without publishing either a catalogue or a queue.
        assertNull(shown.catalogue);assertFalse(shown.queueLoaded)
        shown.settleControlsRead();shown.settleQueueRead()
        assertTrue(shown.controlsRefreshNeeded)
        assertTrue(shown.queueRefreshNeeded)
        shown.pending=true
        assertFalse(shown.controlsRefreshNeeded);assertFalse(shown.queueRefreshNeeded)
        shown.pending=false
        assertTrue(shown.beginControlsRead(12));assertTrue(shown.beginQueueRead(22))
        assertTrue(shown.controlsReadCurrent(12));assertTrue(shown.queueReadCurrent(22))
        assertFalse(shown.controlsRefreshNeeded);assertFalse(shown.queueRefreshNeeded)
        shown.observeRevisions(13,23)
        shown.retire()
        shown.settleControlsRead();shown.settleQueueRead()
        shown.observeRevisions(14,24)
        assertFalse(shown.controlsRefreshNeeded);assertFalse(shown.queueRefreshNeeded)
        assertFalse(shown.beginControlsRead(14));assertFalse(shown.beginQueueRead(24))
        assertFalse(shown.controlsReadCurrent(12));assertFalse(shown.queueReadCurrent(22))
        assertEquals(source,shown.source)
        val replacement=SessionControlsState(key,Uuid.random(),Any())
        assertFalse(replacement.controlsRefreshNeeded);assertFalse(replacement.queueRefreshNeeded)
    }

    @Test fun imageOnlyUnlatchedDraftRequiresExplicitPreparationOfFreshAction() {
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val key=ConversationKey(host,Target(Uuid.random(),Uuid.random()));val source=Uuid.random()
        val record=LocalAttachmentUpload(host,key.target,source,Uuid.random(),null,32,"ab".repeat(32),committedBytes=32,published=true)
        val images=listOf(ComposerImageInput(record,key,source))
        val previous=Uuid.random();val prepared=mutableListOf<Uuid>()
        var submitted by mutableStateOf(true)
        var reserved by mutableStateOf(false)
        var enabled by mutableStateOf(false)
        ui.setContent {MaterialTheme {
            PromptActionPreparation("",images,submitted,reserved,enabled) {
                prepared+=Uuid.random();reserved=true
            }
        }}
        ui.onNodeWithTag("prompt_new_action").assertDoesNotExist()
        // Definitive refusal releases the submitted latch, but never prepares or sends again.
        ui.runOnIdle {submitted=false}
        ui.onNodeWithTag("prompt_new_action").assertIsNotEnabled()
        assertTrue(prepared.isEmpty())
        ui.runOnIdle {enabled=true}
        ui.onNodeWithTag("prompt_new_action").assertIsEnabled()
        assertTrue(prepared.isEmpty())
        ui.onNodeWithTag("prompt_new_action").performClick()
        ui.onNodeWithTag("prompt_new_action").assertDoesNotExist()
        assertEquals(1,prepared.size);assertNotEquals(previous,prepared.single())
        assertSame(record,images.single().record)
        ui.runOnIdle {reserved=false;submitted=true}
        ui.onNodeWithTag("prompt_new_action").assertDoesNotExist()
        assertEquals(1,prepared.size)
    }

    @Test fun queueChangedBeforeCollectorDiscardsDisplayedContinuationAndReadsHead() {
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val summary=SessionSummary(Uuid.random(),Uuid.random(),Uuid.random(),Id("workspace"),Id("terminal"),Id("pane"),Provider.Codex,Id("original-thread"),"queue ordering",SessionStatus.Idle,SessionConnectivity.Ready,
            Capabilities(true,true,true,true,PromptMode.SubmitOrSteer,CancelMode.NativeTurn,controls=true),
            HistoryCoverage(HistoryScope.Thread,HistorySource.NativePages,false,false,false))
        val target=Target(summary.sessionId,summary.generation);val subscription=Uuid.random()
        val reducer=SessionReducer()
        reducer.snapshot(SessionOpenResult(subscription,ZeroValue(),SessionState(summary,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle)),emptyList(),emptyList()))
        reducer.recoveryReconciled()
        val shown=SessionControlsState(ConversationKey(host,target),subscription,Any())
        shown.queueOpen=true
        assertTrue(shown.beginQueueRead(reducer.queueRevision))
        val cursor=Id("old-native-continuation")
        val page=SessionQueueListResult(target,listOf(SessionQueueEntry(Id("old-entry"),ContentText(InlineText("old queued text")),true)),cursor)
        shown.queueEntries=page.entries;shown.queueText=mapOf(page.entries.single().id to "old queued text")
        shown.next=page.nextCursor;shown.queueLoaded=true;shown.settleQueueRead()
        assertEquals(cursor,nativeQueueReadCursor(shown,reducer.queueRevision,cursor))

        // Accepted native ingress precedes the VM collector: the displayed cursor is still old.
        assertEquals(SessionReducer.Decision.Applied,reducer.event(Event(subscription,Uint(1),EventBody.QueueChanged(QueueChangedEvent(target)))))
        assertEquals(cursor,shown.next);assertTrue(shown.queueLoaded)
        assertFalse(shown.queueRefreshNeeded)
        val loadedRevision=shown.queueRevision
        val readCursor=nativeQueueReadCursor(shown,reducer.queueRevision,shown.next)
        assertNull(readCursor)
        assertEquals(loadedRevision,shown.queueRevision)
        assertTrue(shown.queueEntries.isEmpty());assertTrue(shown.queueText.isEmpty())
        assertNull(shown.next);assertFalse(shown.queueLoaded);assertTrue(shown.queueRefreshNeeded)

        // A blocked admission cannot consume the invalidation or resurrect its stale cursor.
        shown.pending=true
        assertFalse(shown.beginQueueRead(reducer.queueRevision))
        shown.pending=false
        assertTrue(shown.queueRefreshNeeded)
        assertTrue(shown.beginQueueRead(reducer.queueRevision))
        assertTrue(shown.queueReadCurrent(reducer.queueRevision))
        assertNull(readCursor);assertNull(shown.next)
        shown.retire();shown.settleQueueRead()
        assertFalse(shown.queueRefreshNeeded)
        assertFalse(shown.beginQueueRead(reducer.queueRevision))
    }
}
