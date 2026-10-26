// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Application
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import java.io.File
import java.security.KeyStore
import java.util.Base64
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier

/** Actual exported MainActivity/native owner/rotation/foreground cleanup. Isolated installation only. */
class NavigationTest {
    @get:Rule val ui=createEmptyComposeRule()
    @Test fun selectingAnotherActionRetiresTheOldReceiptBeforeRefreshWithoutLosingHistory()=runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as GoatrApplication
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Isolated fixture only; never adopt a paired installation",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        val runtime=context.runtime
        runtime.store.open()
        assertTrue(runtime.store.state.value.hosts.isEmpty())
        assertFalse(GoatrConnection.endpointIsOwned())
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val boot=Uuid.random();val server=Uuid.random();val project=Uuid.random()
        val previous=Request(Uuid.random(),RequestBody.SessionCreate(SessionCreateParams(boot,server,project,Provider.Codex,Label("Historical creation"))))
        val selected=Request(Uuid.random(),RequestBody.SessionCreate(SessionCreateParams(boot,server,project,Provider.Codex,Label("Pending creation"))))
        val receiptKey=ByteArray(32){7}
        var ownedHost=false
        var owner:GoatrViewModel?=null
        try {
            runtime.store.host(HostMetadata(host,"Owned local receipt focus fixture"));ownedHost=true
            for(request in listOf(previous,selected))runtime.store.preSend(host,request,receiptKey)
            val time=Uint(System.currentTimeMillis())
            val historicalReceipt=Receipt(previous.id,ActionMethod.SessionCreate,boot,null,time,time,ReceiptState.Failed,
                creation=CreationProgress(server,project,Provider.Codex,CreationStage.Launch,StageState.Failed,
                    CreationResources(workspaceId=Id("retained-workspace"),paneId=Id("retained-pane"),terminalId=Id("retained-terminal"),nativeId=Id("retained-thread"))))
            runtime.store.receipt(host,historicalReceipt,true)
            val historical=runtime.store.localAction(host,previous.id)!!
            val pending=runtime.store.localAction(host,selected.id)!!
            val model=withContext(Dispatchers.Main.immediate){GoatrViewModel(context).also {owner=it;it.sessions(host)}}
            fun inspect(action:Uuid):Job {
                val before=model.viewModelScope.coroutineContext[Job]!!.children.toSet()
                model.inspect(host,action)
                return model.viewModelScope.coroutineContext[Job]!!.children.single {it !in before}
            }
            withContext(Dispatchers.Main.immediate){inspect(previous.id)}.join()
            withContext(Dispatchers.Main.immediate){assertEquals(historical,model.focusedAction)}
            // Hold the real store writer: the old refresh captures its focus but cannot publish.
            val entered=java.util.concurrent.CountDownLatch(1);val release=java.util.concurrent.CountDownLatch(1)
            val oldPublication=CompletableDeferred<LocalAction?>()
            coroutineScope {
                val blocker=async(Dispatchers.IO){runtime.store.reconcilePresentations(host,{entered.countDown();check(release.await(5,java.util.concurrent.TimeUnit.SECONDS))}){PresentationPolicy(false,false){false}}}
                lateinit var stale:Job
                lateinit var current:Job
                try {
                    withContext(Dispatchers.IO){check(entered.await(5,java.util.concurrent.TimeUnit.SECONDS))}
                    withContext(Dispatchers.Main.immediate){
                        val displayed=model.focusedAction
                        stale=inspect(previous.id)
                        assertSame("Same-ID inspection retains its exact read-only record while refreshing",displayed,model.focusedAction)
                        assertFalse(stale.isCompleted)
                        stale.invokeOnCompletion {oldPublication.complete(model.focusedAction)}
                        current=inspect(selected.id)
                        assertNull("A newly selected pending action must not display the old creation receipt",model.focusedAction)
                        assertTrue(model.actions[host].orEmpty().contains(historical))
                    }
                }finally {release.countDown()}
                blocker.await();stale.join();current.join()
            }
            assertNotEquals("The completed stale refresh cannot restore its historical selection",historical,oldPublication.await())
            withContext(Dispatchers.Main.immediate){assertEquals(pending,model.focusedAction)}
            val selectedReceipt=Receipt(selected.id,ActionMethod.SessionCreate,boot,null,time,time,ReceiptState.Reserved)
            runtime.store.receipt(host,selectedReceipt,true)
            withContext(Dispatchers.Main.immediate){inspect(selected.id)}.join()
            withContext(Dispatchers.Main.immediate){
                assertEquals(selectedReceipt,model.focusedAction?.receipt)
                assertEquals(selected.id,model.focusedAction?.metadata?.actionId)
                assertNull("Inspection must not acquire a native channel",model.connection.state.value.hello[host])
            }
            withContext(Dispatchers.Main.immediate){inspect(previous.id)}.join()
            withContext(Dispatchers.Main.immediate){
                assertEquals(historical,model.focusedAction)
                model.sessions()
                assertNull("Clearing the selected action retires its displayed record",model.focusedAction)
            }
            assertEquals(historical,runtime.store.localAction(host,previous.id))
            assertEquals(setOf(previous.id,selected.id),runtime.store.localActions(host,null,100).map {it.metadata.actionId}.toSet())
            assertFalse(GoatrConnection.endpointIsOwned())
        }finally {
            receiptKey.fill(0)
            owner?.let {model->withContext(Dispatchers.Main.immediate){model.viewModelScope.coroutineContext[Job]!!.cancelAndJoin()}}
            if(ownedHost){
                val deletion=runtime.store.forget(host)
                assertTrue(deletion.aliases.isEmpty())
                runtime.store.keysDeleted(deletion)
            }
        }
    }

    @Test fun realLauncherRetainsOwnerAcrossRotationAndReleasesNativeForegroundResources() = runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt another installation's protected identity",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        IrohAndroid.installAndroidContext(context)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner: GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            val endpoint=owner.connection.start()
            ui.onNodeWithContentDescription("Goatr",useUnmergedTree=true).assertIsDisplayed()
            for((destination,screen) in listOf("hosts" to Screen.Connections,"settings" to Screen.Settings,"sessions" to Screen.Sessions)) {
                // A closed hamburger has no visible destinations or replacement bottom bar.
                ui.onNodeWithTag("navigation_${destination}_menu").assertDoesNotExist()
                ui.onNodeWithTag("navigation_menu").assertIsDisplayed().performClick()
                ui.onNodeWithTag("navigation_${destination}_menu").assertIsDisplayed().performClick()
                ui.runOnIdle {assertEquals(screen,owner.screen);assertNull(owner.selection);assertNull(owner.host)}
                ui.onNodeWithTag("navigation_${destination}_menu").assertDoesNotExist()
                ui.onNodeWithTag("nav_sessions").assertDoesNotExist()
                ui.onNodeWithTag("nav_hosts").assertDoesNotExist()
                ui.onNodeWithTag("nav_settings").assertDoesNotExist()
            }
            // This unpaired local route has no Hello/native channel. Exercise the actual
            // retained VM and production editor without granting a synthetic target authority.
            val draftHost="0".repeat(64)
            val draftTarget=Target(Uuid.random(),Uuid.random())
            val draftKey=ConversationKey(draftHost,draftTarget)
            val otherKey=ConversationKey(draftHost,Target(Uuid.random(),Uuid.random()))
            scenario.onActivity {
                assertFalse(owner.connection.state.value.hello.containsKey(draftHost))
                owner.sessionFilters.query="retained local search"
                owner.sessionFilters.provider=Provider.Codex
                owner.sessionFilters.status=SessionStatus.Attention
                owner.open(draftHost,draftTarget)
                owner.composer("Original selected draft")
                owner.composerCursor(2,8)
            }
            ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.TextSelectionRange,androidx.compose.ui.text.TextRange(2,8)))
            // Real editor input and externally supplied ordered endpoints are separate contracts.
            ui.onNodeWithTag("composer").performTextInputSelection(androidx.compose.ui.text.TextRange(1,7))
            ui.runOnIdle {assertEquals(1 to 7,owner.composerSelection(draftKey))}
            ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.TextSelectionRange,androidx.compose.ui.text.TextRange(1,7)))
            scenario.onActivity {owner.composerCursor(8,3)}
            ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.TextSelectionRange,androidx.compose.ui.text.TextRange(8,3)))
            scenario.onActivity {owner.settings()}
            ui.onNodeWithTag("settings_overview_list").assertIsDisplayed()
            ui.onNodeWithTag("composer").assertDoesNotExist()
            scenario.onActivity {owner.open(draftHost,draftTarget);assertEquals(8 to 3,owner.composerSelection(draftKey))}
            ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.TextSelectionRange,androidx.compose.ui.text.TextRange(8,3)))
            scenario.recreate()
            scenario.onActivity {
                assertSame(owner,ViewModelProvider(it)[GoatrViewModel::class.java])
                assertEquals("retained local search",owner.sessionFilters.query)
                assertEquals(Provider.Codex,owner.sessionFilters.provider)
                assertEquals(SessionStatus.Attention,owner.sessionFilters.status)
                assertEquals(8 to 3,owner.composerSelection(draftKey))
            }
            ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.TextSelectionRange,androidx.compose.ui.text.TextRange(8,3)))
            scenario.onActivity {
                owner.composer("Externally inserted draft")
                owner.composerCursor(4,4)
                owner.open(otherKey.host,otherKey.target)
                owner.composer("Other conversation remains")
                owner.open(draftHost,draftTarget)
                assertFalse(owner.clearComposerDraft(otherKey))
                val identity=ConversationReaderIdentity(Uuid.random(),Uuid.random(),Provider.Omp,Id("original-reader"))
                val original=owner.readerPosition(draftKey,identity)
                val ticket=original.attach()
                original.capture(ticket,"history_retained",1,-7,false,true,false,null)
                assertSame(original,owner.readerPosition(draftKey,null))
                assertSame(original,owner.readerPosition(draftKey,identity))
                val replacement=owner.readerPosition(draftKey,identity.copy(incarnation=Uuid.random()))
                assertNotSame(original,replacement)
                assertFalse(original.current(ticket))
                assertNull(replacement.anchor);assertTrue(replacement.following)
            }
            ui.onNodeWithTag("composer").assertTextContains("Externally inserted draft")
            ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.TextSelectionRange,androidx.compose.ui.text.TextRange(4,4)))
            ui.onNodeWithTag("composer_snippets").performClick()
            ui.onNodeWithTag("snippet_clear_draft").performScrollTo().performClick()
            ui.runOnIdle {assertEquals("Externally inserted draft",owner.composers[draftKey])}
            ui.onNodeWithText("Keep draft").performClick()
            ui.onNodeWithTag("snippet_clear_draft").performScrollTo().performClick()
            ui.onNodeWithTag("snippet_clear_draft_confirm").performClick()
            ui.runOnIdle {
                assertEquals("",owner.composers[draftKey]);assertEquals(0 to 0,owner.composerSelection(draftKey))
                assertEquals("Other conversation remains",owner.composers[otherKey])
                assertFalse(owner.connection.state.value.hello.containsKey(draftHost))
            }
            scenario.onActivity {owner.sessionFilters.clearFilters();owner.sessionFilters.query="";owner.sessions()}
            ui.onNodeWithTag("pair_begin").performClick()
            ui.onNodeWithTag("pair_paste").performScrollTo().performTextInput("synthetic private raw input, not an invitation")
            scenario.recreate()
            scenario.onActivity {assertSame(owner,ViewModelProvider(it)[GoatrViewModel::class.java])}
            ui.onNodeWithTag("pair_paste").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
            val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
            val json="{\"v\":1,\"host\":\"$host\",\"name\":\"Synthetic never-authorized host\",\"token\":\"${Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32){7})}\",\"expiresAt\":1}"
            val invitation="goatr://pair/v1?data="+Base64.getUrlEncoder().withoutPadding().encodeToString(json.toByteArray())
            ui.onNodeWithTag("pair_paste").performScrollTo().performTextInput(invitation)
            ui.onNodeWithTag("pair_preview").performScrollTo().performClick()
            ui.onNodeWithTag("pair_authority").assertIsDisplayed().assertTextEquals(owner.preview!!.authority)
            assertTrue(owner.store.state.value.hosts.isEmpty())
            ui.onNodeWithTag("pair_cancel").assertIsDisplayed().performClick()
            assertEquals(Screen.Connections,owner.screen)
            ui.onNodeWithTag("pair_confirm").assertDoesNotExist()
            assertTrue(owner.store.state.value.hosts.isEmpty())
            scenario.moveToState(Lifecycle.State.CREATED)
            ui.waitUntil(20000) {!owner.connection.ownsNetworkCallback() && !GoatrConnection.endpointIsOwned()}
            scenario.moveToState(Lifecycle.State.RESUMED)
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            assertEquals(endpoint,owner.connection.start())
            assertTrue(owner.store.state.value.hosts.isEmpty())
        } finally {
            scenario.moveToState(Lifecycle.State.CREATED)
            owner.connection.shutdown()
            // Delete only the alias proven absent before this test and created by this owner.
            assertTrue(owner.store.state.value.hosts.isEmpty())
            SecretStore(context).delete(KeyDeletion("synthetic-lifecycle-owner",listOf(SecretStore.ENDPOINT_ALIAS)),owner.store)
            scenario.close()
        }
    }

    @Test fun zeroHostLostWrappingKeyRequiresExplicitInstallationForgetBeforeNewIdentity() = runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt another installation identity",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        val cipher=File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm")
        val marker=File(cipher.path+".initialized")
        assertFalse(marker.exists())
        IrohAndroid.installAndroidContext(context)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner: GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            val original=owner.connection.start()
            assertTrue(owner.store.state.value.hosts.isEmpty());assertTrue(cipher.isFile);assertTrue(marker.isFile)
            val retained=cipher.readBytes()
            scenario.moveToState(Lifecycle.State.CREATED)
            ui.waitUntil(20000) {!GoatrConnection.endpointIsOwned()}
            keys.deleteEntry(SecretStore.ENDPOINT_ALIAS) // Only the wrapping entry proven created by this test.
            scenario.moveToState(Lifecycle.State.RESUMED)
            ui.waitUntil(20000) {owner.foreground && owner.notice!=null}
            assertFalse(GoatrConnection.endpointIsOwned());assertTrue(owner.store.state.value.hosts.isEmpty())
            assertArrayEquals(retained,cipher.readBytes());assertTrue(marker.isFile);assertFalse(keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
            ui.onNodeWithTag("navigation_menu").assertIsDisplayed().performClick()
            ui.onNodeWithTag("navigation_hosts_menu").assertIsDisplayed().performClick()
            ui.onNodeWithTag("connections_list").assertIsDisplayed()
            ui.runOnIdle {assertEquals(Screen.Connections,owner.screen);assertNull(owner.host);assertNull(owner.selection)}
            ui.onNodeWithTag("installation_forget").performScrollTo().assertIsDisplayed().performClick()
            ui.onNodeWithTag("installation_forget_cancel").performClick()
            assertArrayEquals(retained,cipher.readBytes());assertTrue(marker.isFile);assertFalse(GoatrConnection.endpointIsOwned())
            ui.onNodeWithTag("installation_forget").performScrollTo().performClick()
            ui.onNodeWithTag("installation_forget_confirm").performClick()
            ui.waitUntil(10000) {!cipher.exists() && !marker.exists() && !keys.containsAlias(SecretStore.ENDPOINT_ALIAS)}
            assertTrue(owner.store.state.value.hosts.isEmpty());assertFalse(GoatrConnection.endpointIsOwned())
            scenario.moveToState(Lifecycle.State.CREATED);scenario.moveToState(Lifecycle.State.RESUMED)
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            assertNotEquals(original,owner.connection.start())
            assertTrue(owner.store.state.value.hosts.isEmpty())
        } finally {
            scenario.moveToState(Lifecycle.State.CREATED);owner.connection.shutdown()
            assertTrue(owner.store.state.value.hosts.isEmpty())
            SecretStore(context).delete(KeyDeletion("synthetic-lost-key-owner",listOf(SecretStore.ENDPOINT_ALIAS)),owner.store)
            scenario.close()
        }
    }

    @Test fun deliveryStartKeepsTheOriginalVisibleRouteThroughARealWriterWaitAndBackground()=runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as GoatrApplication
        val keys=KeyStore.getInstance("AndroidKeyStore").apply{load(null)}
        assertFalse("Isolated fixture only; never adopt paired installation keys",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        IrohAndroid.installAndroidContext(context)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        val runtime=context.runtime
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        lateinit var owner:GoatrViewModel
        try {
            scenario.onActivity{owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000){owner.foreground&&owner.connection.ownsNetworkCallback()}
            ui.waitForIdle()
            assertTrue(runtime.store.state.value.hosts.isEmpty())
            check(runtime.deliveryPreferences.canPostNotifications()){"Existing owned fixture must grant notifications before this negative FGS boundary check"}
            check(runtime.push.ntfyInstalled()){"Qualified official ntfy is a fixture prerequisite; absence must not make the stale-route regression pass"}
            runtime.store.host(HostMetadata(host,"Owned no-channel delivery boundary",Uuid.random(),listOf(SecretStore.ENDPOINT_ALIAS)))
            runtime.deliveryPreferences.setMode(host,DeliveryMode.Ntfy)
            runtime.deliveryPreferences.setServiceRequested(false)
            ui.runOnIdle{owner.settings(host)}
            val entered=java.util.concurrent.CountDownLatch(1);val release=java.util.concurrent.CountDownLatch(1)
            coroutineScope {
                val blocker=async(Dispatchers.IO){runtime.store.reconcilePresentations(host,{entered.countDown();check(release.await(5,java.util.concurrent.TimeUnit.SECONDS))}){PresentationPolicy(false,false){false}}}
                lateinit var original:Job
                try {
                    withContext(Dispatchers.IO){check(entered.await(5,java.util.concurrent.TimeUnit.SECONDS))}
                    ui.runOnIdle{
                        val before=owner.viewModelScope.coroutineContext[Job]!!.children.toSet()
                        owner.setPersistentDelivery(host,true)
                        original=owner.viewModelScope.coroutineContext[Job]!!.children.single{it !in before}
                        owner.settings()
                        owner.settings(host)
                    }
                }finally{release.countDown()}
                blocker.await();original.join()
            }
            assertTrue(runtime.deliveryPreferences.hostEnabled(host))
            assertNull("A queued old route cannot register a new endpoint under a newer same-host screen",runtime.store.encryptedRegistration(host))
            assertFalse(runtime.deliveryPreferences.serviceRequested())
            assertFalse(DeliveryService.status.value.running)
            runtime.deliveryPreferences.setMode(host,DeliveryMode.Persistent)
            scenario.moveToState(Lifecycle.State.CREATED)
            ui.waitUntil(20000){!owner.foreground}
            InstrumentationRegistry.getInstrumentation().runOnMainSync{owner.startPersistentDelivery();owner.notificationPermissionResult()}
            assertFalse("A background call cannot turn user-start policy into a service start",runtime.deliveryPreferences.serviceRequested())
            assertFalse(DeliveryService.status.value.running)
        }finally{
            scenario.moveToState(Lifecycle.State.CREATED)
            DeliveryService.stopFromUser(context)
            owner.connection.shutdown()
            if(runtime.store.state.value.hosts.any{it.hostId==host})owner.connection.forget(host)
            for(deletion in runtime.store.state.value.keyDeletion)runtime.secrets.delete(deletion,runtime.store)
            runtime.deliveryPreferences.clearHost(host)
            if(keys.containsAlias(SecretStore.ENDPOINT_ALIAS))runtime.secrets.delete(KeyDeletion("synthetic-delivery-boundary-owner",listOf(SecretStore.ENDPOINT_ALIAS)),runtime.store)
            scenario.close()
        }
    }
}

/** One deterministic actual-Compose eligibility regression; no native/provider pass. */
class CreationEligibilityTest {
    @get:Rule val ui=androidx.compose.ui.test.junit4.v2.createComposeRule()
    @Test fun firstCreationUsesCurrentCanCreateAttemptNotExistingLiveBackendReadiness() {
        val server=Server(Uuid.random(),Label("Owned component server"),ServerConnectivity.Online,CanCreate(true,false))
        val project=Project(Uuid.random(),Label("Approved component project"),true)
        val graph=NativeHierarchy(server.serverId,Uuid.random(),NativeFreshness.Current,workspaces=emptyList(),tabs=emptyList(),panes=emptyList())
        val inventory=mutableStateOf(HostSnapshotResult(Uuid.random(),ZeroValue(0),listOf(server),listOf(project),emptyList(),emptyList(),listOf(graph)))
        val enabled=mutableStateOf(true)
        val hello=HelloResult(EndpointId("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"),Uuid.random(),Uuid.random(),ProtocolVersion(),
            Provider.entries.map {Backend(it,"component only",Availability.SetupRequired,"No existing live native binding")})
        var created:Triple<Uuid,Uuid,Provider>?=null
        val memory=FormMemory()
        ui.setContent {MaterialTheme {Column(Modifier.verticalScroll(rememberScrollState())) {
            CreateSession(inventory.value,hello,enabled.value,memory,{}) {choice->created=Triple(choice.server,choice.project,choice.provider)}
        }}}
        ui.onNodeWithTag("create_name").performScrollTo().performTextInput("First actual guarded attempt")
        ui.onNodeWithTag("session_create").performScrollTo().assertIsEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(servers=listOf(server.copy(canCreate=CanCreate(false,false))))}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(servers=listOf(server.copy(connectivity=ServerConnectivity.Offline)))}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(servers=listOf(server),hierarchy=listOf(graph.copy(freshness=NativeFreshness.Stale,reason=NativeUnavailableReason.SourceUnavailable)))}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(hierarchy=listOf(graph));enabled.value=false}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        assertNull(created)
        ui.runOnIdle {enabled.value=true}
        ui.onNodeWithTag("session_create").assertIsEnabled().performClick()
        assertEquals(Triple(server.serverId,project.projectId,Provider.Omp),created)
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
    }
}
