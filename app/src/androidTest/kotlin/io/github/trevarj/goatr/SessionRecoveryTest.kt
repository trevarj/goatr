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
    @Test fun exactAuthoritativeReceiptMustBeDisplayedBeforeRecoveryDeletionAndSurvivesReopen() = runBlocking {
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
            assertEquals(listOf(request.id),store.state.value.recovery.map {it.action.actionId})
            store.close()
            val reopened=SessionStore(context,name)
            try {reopened.open();assertTrue(reopened.localAction(host,successful.id)!!.outcomeShown);assertEquals(request.id,reopened.state.value.recovery.single().action.actionId);assertEquals(warning.futureExpiry!!.expiresAt,reopened.state.value.recovery.single().expiresAt)} finally {reopened.close()}
        } finally {store.close();context.deleteDatabase(name)}
    }
}
