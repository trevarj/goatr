// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import java.security.KeyStore
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.Rule
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.test.core.app.ActivityScenario
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import android.view.WindowManager
import android.view.inspector.WindowInspector
import android.accessibilityservice.AccessibilityServiceInfo
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.window.DialogWindowProvider
import android.os.Bundle
import android.os.PersistableBundle
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import java.util.ArrayDeque
import java.util.concurrent.atomic.AtomicInteger
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.AnnotatedString

/** Real Android endpoint/GoatrConnection/SQLite/AndroidKeyStore against smoke --review-fixture.
 * The host uses actual Rust State/Security/transport, with synthetic provider-data transitions.
 * No echo server, mocked transport, production host, installed debug command or provider effect. */
class GoatrConnectionTest {
    private var privateCheckpointStage="bootstrap"
    private val composeUi=createEmptyComposeRule()
    @get:Rule val privateCheckpointRule:TestRule=RuleChain.outerRule(PrivateCheckpointSanitizer()).around(composeUi)
    private val ui get()=composeUi
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext

    /** The selected retained-pairing flow can hold a live invitation in Compose semantics.
     * Its outer rule also encloses Compose teardown, which otherwise may attach that tree to JUnit. */
    private inner class PrivateCheckpointSanitizer:TestRule {
        override fun apply(base:Statement,description:Description)=object:Statement() {
            override fun evaluate() {
                val selected=privateCheckpointSelected()
                try {
                    if(selected) {
                        privateCheckpointStage="test_body"
                        val automation=InstrumentationRegistry.getInstrumentation().uiAutomation
                        val serviceInfo=checkNotNull(automation.serviceInfo)
                        val originalFlags=serviceInfo.flags
                        try {
                            serviceInfo.flags=originalFlags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
                            automation.serviceInfo=serviceInfo
                            base.evaluate()
                        } finally {
                            serviceInfo.flags=originalFlags
                            automation.serviceInfo=serviceInfo
                        }
                    } else base.evaluate()
                }
                catch(error:Throwable) {
                    if(selected) throw AssertionError("pairing_checkpoint_$privateCheckpointStage")
                    throw error
                }
            }
        }
    }
    private fun privateCheckpointSelected():Boolean {
        val arguments=InstrumentationRegistry.getArguments()
        val selected=arguments.getString("class")
        return arguments.getString("goatrSdkForegroundBootstrap")=="main_once" && selected in setOf(
            "io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_private_pairing_checkpoint",
            "io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_owned_retirement_checkpoint",
        ) && (selected!="io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_owned_retirement_checkpoint" || arguments.getString("goatrOwnedRetirementCheckpoint")=="true")
    }
    private fun privateCheckpointRequire(stage:String,condition:Boolean) {
        privateCheckpointStage=stage
        check(condition)
    }
    private fun physicalArguments(action:String,shell:Boolean=false)=Bundle().apply {
        putString("ownedUiRunId",Uuid.random().value);putString("ownedUiAction",action)
        putString("ownedUiHostId","a".repeat(64));putString("ownedUiMvpRunId",Uuid.random().value)
        putString("ownedUiProductPid","123");putString("ownedUiProductStart","456")
        putString("ownedUiObserverPublicSha256","1".repeat(64));putString("ownedUiExpectedProductSha256","2".repeat(64))
        if(shell){putString("ownedUiShellId",Uuid.random().value);putString("ownedUiShellGeneration",Uuid.random().value)}
    }
    private fun physicalRejected(arguments:Bundle) {
        try {OwnedUiAutomationRunner.validatePhysicalArguments(arguments);fail("closed physical selectors must reject")}
        catch(_:RuntimeException) {}
    }
    @Test fun physicalOwnedUiSelectorsRequireOriginalEpochAndRejectMixedAgentShellAuthority() {
        val shell=physicalArguments("ObserveShell",true)
        OwnedUiAutomationRunner.validatePhysicalArguments(shell)
        for(key in listOf("ownedUiHostId","ownedUiMvpRunId","ownedUiProductPid","ownedUiProductStart","ownedUiObserverPublicSha256","ownedUiExpectedProductSha256","ownedUiShellId","ownedUiShellGeneration")) {
            physicalRejected(Bundle(shell).apply {remove(key)})
        }
        physicalRejected(Bundle(shell).apply {putString("ownedUiSessionId",Uuid.random().value);putString("ownedUiSessionGeneration",Uuid.random().value)})
        val agent=physicalArguments("OpenTerminal").apply {putString("ownedUiSessionId",Uuid.random().value);putString("ownedUiSessionGeneration",Uuid.random().value)}
        OwnedUiAutomationRunner.validatePhysicalArguments(agent)
        physicalRejected(Bundle(agent).apply {putString("ownedUiShellId",Uuid.random().value);putString("ownedUiShellGeneration",Uuid.random().value)})
        physicalRejected(physicalArguments("OpenNativeClose"))
        physicalRejected(Bundle(shell).apply {putString("ownedUiRawAnsi","not-authorized")})
    }
    @Test fun physicalOwnedUiKeysAndDraftCannotBecomeArbitraryNativeInput() {
        val key=physicalArguments("SendShellKey",true).apply {putString("ownedUiKey","Left")}
        OwnedUiAutomationRunner.validatePhysicalArguments(key)
        for(value in listOf("Character","c","Ctrl","Enter\n"))physicalRejected(Bundle(key).apply {putString("ownedUiKey",value)})
        OwnedUiAutomationRunner.validatePhysicalArguments(physicalArguments("SetShellModifier",true).apply {putString("ownedUiKey","Ctrl")})
        physicalRejected(physicalArguments("SetShellDraft",true).apply {putString("ownedUiComposerText","arbitrary command")})
        physicalRejected(physicalArguments("SetShellDraft",true).apply {putString("ownedUiKey","Enter")})
    }
    @Test fun physicalOwnedUiResizeUsesActual32768PtyBoundAndHistoryUsesExactUint() {
        val resize=physicalArguments("SetShellSize",true).apply {putString("ownedUiColumns","512");putString("ownedUiRows","64");putString("ownedUiSizeField","Columns")}
        OwnedUiAutomationRunner.validatePhysicalArguments(resize)
        for((key,value) in listOf("ownedUiColumns" to "513","ownedUiColumns" to "0512","ownedUiRows" to "65","ownedUiRows" to "0","ownedUiRows" to "257","ownedUiSizeField" to "Both"))
            physicalRejected(Bundle(resize).apply {putString(key,value)})
        val history=physicalArguments("ScrollShellHistory",true).apply {putString("ownedUiScrollOffset","9007199254740991");putString("ownedUiScrollDirection","Older")}
        OwnedUiAutomationRunner.validatePhysicalArguments(history)
        physicalRejected(Bundle(history).apply {putString("ownedUiScrollOffset","9007199254740992")})
        physicalRejected(Bundle(history).apply {putString("ownedUiScrollOffset","-1")})
        physicalRejected(Bundle(history).apply {putString("ownedUiScrollDirection","AutomaticBottom")})
    }
    @Test fun physicalOwnedUiCreationAndCleanupSelectorsAreClosedWithoutFakePaneFields() {
        val creation=physicalArguments("SelectTerminalProject").apply {
            putString("ownedUiServerId",Uuid.random().value);putString("ownedUiWorkspaceId","stock-workspace:7");putString("ownedUiProjectId",Uuid.random().value)
        }
        OwnedUiAutomationRunner.validatePhysicalArguments(creation)
        physicalRejected(Bundle(creation).apply {putString("ownedUiPaneId","invented-pane")})
        physicalRejected(Bundle(creation).apply {putString("ownedUiWorkspaceId","empty workspace")})
        val stop=physicalArguments("StopDelivery").apply {putString("ownedUiExpectedServiceRequested","true");putString("ownedUiOwnedStartRunId",Uuid.random().value)}
        OwnedUiAutomationRunner.validatePhysicalArguments(stop)
        physicalRejected(Bundle(stop).apply {remove("ownedUiOwnedStartRunId")})
        physicalRejected(Bundle(stop).apply {putString("ownedUiDeliveryMode","Persistent")})
        physicalRejected(physicalArguments("StartDelivery").apply {putString("ownedUiExpectedServiceRequested","true");putString("ownedUiOwnedStartRunId",Uuid.random().value)})
    }
    @Test fun physicalOwnedUiFingerprintBindsEverySelectorAndDoesNotDependOnBundleOrder() {
        val first=physicalArguments("SendShellKey",true).apply {putString("ownedUiKey","Left")}
        val second=Bundle();first.keySet().sortedDescending().forEach {second.putString(it,first.getString(it))}
        val hash=OwnedUiAutomationRunner.physicalArgumentHash(first)
        assertEquals(hash,OwnedUiAutomationRunner.physicalArgumentHash(second))
        for(key in listOf("ownedUiKey","ownedUiProductStart","ownedUiShellGeneration","ownedUiHostId","ownedUiExpectedProductSha256")) {
            val changed=Bundle(first).apply {putString(key,checkNotNull(getString(key))+"0")}
            assertNotEquals(key,hash,OwnedUiAutomationRunner.physicalArgumentHash(changed))
        }
        physicalRejected(Bundle(first).apply {putString("ownedUiKey","Left\n")})
    }
    @Test fun physicalOwnedUiDoesNotAddEpochRequirementsToLegacyUnqualifiedObserve() {
        OwnedUiAutomationRunner.validatePhysicalArguments(Bundle().apply {putString("ownedUiRunId",Uuid.random().value);putString("ownedUiAction","Observe")})
        physicalRejected(Bundle().apply {putString("ownedUiRunId",Uuid.random().value);putString("ownedUiAction","Observe");putString("ownedUiHostId","a".repeat(64))})
    }
    @Suppress("DEPRECATION")
    @Test fun actualPackagedPhysicalInstrumentationTargetsRemainProductAndStandaloneSeparate() {
        val manager=context.packageManager
        val entries=manager.getPackageInfo(PhysicalMvpPlatform.TEST,android.content.pm.PackageManager.GET_INSTRUMENTATION).instrumentation.orEmpty()
        assertEquals(2,entries.size)
        assertEquals(mapOf("io.github.trevarj.goatr.OwnedInstrumentationRunner" to PhysicalMvpPlatform.PRODUCT,
            "io.github.trevarj.goatr.OwnedUiAutomationRunner" to PhysicalMvpPlatform.TEST),entries.associate {it.name to it.targetPackage})
        assertTrue(entries.all {it.targetProcesses==null})
        val productInfo=manager.getPackageInfo(PhysicalMvpPlatform.PRODUCT,0);val testInfo=manager.getPackageInfo(PhysicalMvpPlatform.TEST,0)
        val productApp=checkNotNull(productInfo.applicationInfo);val testApp=checkNotNull(testInfo.applicationInfo)
        assertNotEquals(productApp.uid,testApp.uid);assertEquals(PhysicalMvpPlatform.PRODUCT,productApp.processName);assertEquals(PhysicalMvpPlatform.TEST,testApp.processName)
        assertNull(productInfo.sharedUserId);assertNull(testInfo.sharedUserId)
        val flags=android.content.pm.PackageManager.GET_SIGNATURES
        val product=manager.getPackageInfo(PhysicalMvpPlatform.PRODUCT,flags).signatures.orEmpty().map {PhysicalMvpPlatform.digest(it.toByteArray())}.toSet()
        val test=manager.getPackageInfo(PhysicalMvpPlatform.TEST,flags).signatures.orEmpty().map {PhysicalMvpPlatform.digest(it.toByteArray())}.toSet()
        assertEquals(1,product.size);assertEquals(product,test)
    }
    private val controlPort: Int get() = InstrumentationRegistry.getArguments().getString("reviewControlPort")
        ?.toIntOrNull()?.takeIf { it in 1..65535 }
        ?: error("Start the owned smoke --review-fixture and pass reviewControlPort; this is not an optional/ignored gate")

    private suspend fun control(method: String, vararg fields: Pair<String, JsonElement>): JsonObject = withContext(Dispatchers.IO) {
        val body = buildJsonObject { put("method", method); fields.forEach { (name, value) -> put(name, value) } }
        val bytes = (body.toString()+"\n").toByteArray(Charsets.UTF_8)
        require(bytes.size <= 4096)
        Socket().use { socket ->
            socket.connect(InetSocketAddress("127.0.0.1", controlPort), 5000)
            socket.soTimeout = 5000
            socket.getOutputStream().write(bytes)
            val input = socket.getInputStream(); val output = ByteArrayOutputStream()
            while (true) {
                val byte = input.read()
                check(byte >= 0) { "Owned native fixture closed its control channel" }
                if (byte == 10) break
                check(output.size() < 16384) { "Owned native fixture exceeded its control response bound" }
                output.write(byte)
            }
            val result = Protocol.parseRawJson(output.toByteArray()) as JsonObject
            check(result["ok"]?.jsonPrimitive?.boolean == true) { "Owned native fixture rejected control" }
            result
        }
    }
    private fun target(result: JsonObject): Target = Protocol.strictJson.decodeFromJsonElement(Target.serializer(), result.getValue("target"))
    private class Fixture(val connection: GoatrConnection, val store: SessionStore, val secrets: SecretStore,
                          val host: String, val target: Target, val hello: HelloResult, val databaseName: String) {
        fun reducer(): SessionReducer = connection.state.value.sessions[host]?.singleOrNull()
            ?: error("Expected one actual foreground session reducer")
    }
    private suspend fun withFixture(provider: Provider, block: suspend CoroutineScope.(Fixture) -> Unit) = coroutineScope {
        IrohAndroid.installAndroidContext(context)
        val wrapping = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        assertFalse("Use the caller-owned isolated app installation, never adopt its existing identity", wrapping.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir, "goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        val reset = control("reset", "provider" to JsonPrimitive(if (provider == Provider.Omp) "omp" else "codex"))
        val host = reset.getValue("host").jsonPrimitive.content
        val target = target(reset)
        val alias = SecretStore.hostAlias(host, "receipt")
        val name = "connection-review-${Uuid.random().value}.db"
        val store = SessionStore(context, name); val secrets = SecretStore(context)
        val connection = GoatrConnection(context, store, secrets)
        try {
            store.open()
            val endpoint = withTimeout(20000) { connection.start() }
            // Record this fixture's endpoint obligation immediately; failure cleanup may forget
            // only these known aliases, never an existing installation's material.
            store.host(HostMetadata(host, "synthetic native host", keyAliases = listOf(SecretStore.ENDPOINT_ALIAS)))
            secrets.material(alias, true).fill(0)
            store.host(HostMetadata(host, "synthetic native host", keyAliases = listOf(SecretStore.ENDPOINT_ALIAS, alias)))
            val authorized = control("authorize", "endpoint" to JsonPrimitive(endpoint))
            store.host(HostMetadata(host, "synthetic native host", Uuid(authorized.getValue("device").jsonPrimitive.content), listOf(SecretStore.ENDPOINT_ALIAS, alias)))
            val hello = withTimeout(30000) { connection.reconnect(host) }
            assertEquals(host, hello.hostId.value)
            val opened = connection.open(host, target)
            assertEquals(target, Target(opened.state.summary.sessionId, opened.state.summary.generation))
            val fixture = Fixture(connection, store, secrets, host, target, hello, name)
            assertTrue(fixture.reducer().mutationsEnabled)
            block(fixture)
        } finally {
            withContext(NonCancellable) {
                try { connection.shutdown() } finally {
                    try { control("forget_peer") } finally {
                        try {
                            if (store.state.value.hosts.any { it.hostId == host }) connection.forget(host)
                            for (obligation in store.state.value.keyDeletion) secrets.delete(obligation, store)
                        } finally { store.close(); context.deleteDatabase(name) }
                    }
                }
            }
        }
    }
    private suspend fun await(fixture: Fixture, label: String, ready: () -> Boolean) {
        withTimeout(10000) {
            while (!ready()) {
                assertTrue("$label must not disconnect the authenticated native host", fixture.connection.state.value.hello.containsKey(fixture.host))
                delay(10)
            }
        }
    }
    private suspend fun noWrite(fixture: Fixture, target: Target) {
        val request = Request(Uuid.random(), RequestBody.PromptSubmit(PromptSubmitParams(fixture.hello.bootId, target, PromptText("must remain local while resynchronizing"))))
        try { fixture.connection.mutate(fixture.host, request); fail("Mutation before authoritative snapshot/receipt readiness") }
        catch (failure: StorePolicy) { assertTrue(failure.code == ErrorCode.NotReady || failure.code == ErrorCode.StaleSession) }
        assertNull("Disabled mutation must not even create a local pre-send reservation", fixture.store.localAction(fixture.host, request.id))
    }
    private suspend fun retained(fixture: Fixture): Request {
        val request = Request(Uuid.random(), RequestBody.PromptSubmit(PromptSubmitParams(fixture.hello.bootId, fixture.target, PromptText("synthetic retained action"))))
        val key = fixture.secrets.receipt(fixture.host, fixture.store)
        try { fixture.store.preSend(fixture.host, request, key) } finally { key.fill(0) }
        assertNull("Local pre-send reservation is never an authoritative receipt hint",fixture.connection.state.value.latestReceipt[fixture.host])
        control("receipt", "action" to JsonPrimitive(request.id.value))
        return request
    }
    private suspend fun canceledExplicitOpenPreservesDisplacedCleanup(provider:Provider) {
        for(queuedBeforeEntry in listOf(true,false))withFixture(provider) {fixture->
            fixture.reducer().composer(fixture.target,"retain original draft while queued open is canceled")
            control("pause")
            var paused=true
            val blocker=if(queuedBeforeEntry)async(start=CoroutineStart.UNDISPATCHED){fixture.connection.snapshot(fixture.host)}else null
            if(blocker!=null)sentReplies(fixture,1)
            val opened=async(start=CoroutineStart.UNDISPATCHED){fixture.connection.open(fixture.host,fixture.target)}
            var cleanup:Deferred<Unit>?=null
            try {
                if(queuedBeforeEntry) {
                    // The existing serial-owner query queues after open's entry; operations is held by blocker.
                    assertEquals(1,fixture.connection.pendingReplies(fixture.host))
                } else sentReplies(fixture,1)
                val original=withContext(Dispatchers.Main){fixture.connection.retireConversationSubscription(fixture.host,fixture.target)}
                    ?: error("foreground loss must retain exact current cleanup ownership")
                opened.cancelAndJoin()
                cleanup=async {fixture.connection.leave(original)}
                if(!queuedBeforeEntry) {
                    // One already-sent unsubscribe plus independent displaced-owner cleanup, both real requests.
                    sentReplies(fixture,2)
                }
                control("resume");paused=false
                blocker?.await();cleanup.await()
                withTimeout(10000){while(fixture.connection.pendingReplies(fixture.host,sentOnly=false)!=0)delay(10)}
                assertTrue(fixture.connection.state.value.hello.containsKey(fixture.host))
                assertNull(fixture.connection.state.value.sources[fixture.host]?.get(fixture.target))
                assertFalse("No old transcript stream may survive canceled foreground entry",fixture.reducer().mutationsEnabled)
                val foreground=fixture.connection.open(fixture.host,fixture.target)
                fixture.connection.leave(original)
                assertEquals(foreground.subscription,fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target))
                assertTrue(fixture.reducer().mutationsEnabled)
                assertEquals("retain original draft while queued open is canceled",fixture.reducer().composer(fixture.target))
            } finally {
                if(paused)withContext(NonCancellable){control("resume")}
                opened.cancelAndJoin();blocker?.cancelAndJoin();cleanup?.cancelAndJoin()
            }
        }
    }

    private suspend fun backgroundRetiresOnlyOriginalConversation(provider:Provider) = withFixture(provider) {fixture->
        fixture.reducer().composer(fixture.target,"keep this original unsent draft")
        val original=withContext(Dispatchers.Main){fixture.connection.retireConversationSubscription(fixture.host,fixture.target)}
            ?: error("an installed original conversation must have cleanup ownership")
        control("source_begin")
        await(fixture,"retired original source observes immediate native loss"){!fixture.reducer().mutationsEnabled}
        control("ready")
        fixture.connection.snapshot(fixture.host)
        assertNull("Delivery may keep the endpoint, never silently reopen the retired conversation",fixture.connection.state.value.sources[fixture.host]?.get(fixture.target))
        assertTrue(fixture.connection.state.value.hello.containsKey(fixture.host))
        val foreground=fixture.connection.open(fixture.host,fixture.target)
        fixture.connection.leave(original)
        assertEquals("Late background cleanup must not unsubscribe the newer foreground route",foreground.subscription,fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target))
        assertTrue(fixture.reducer().mutationsEnabled)
        assertEquals("keep this original unsent draft",fixture.reducer().composer(fixture.target))
        val background=fixture.connection.retireConversationSubscription(fixture.host,fixture.target) ?: error("foreground route missing")
        fixture.connection.leave(background)
        val retainedActivity=fixture.reducer().state?.activity
        control("activity","text" to JsonPrimitive("background endpoint remains alive"))
        fixture.connection.snapshot(fixture.host)
        assertTrue(fixture.connection.state.value.hello.containsKey(fixture.host))
        assertNull(fixture.connection.state.value.sources[fixture.host]?.get(fixture.target))
        assertEquals(retainedActivity,fixture.reducer().state?.activity)
        val resumed=fixture.connection.open(fixture.host,fixture.target)
        assertEquals(fixture.target,Target(resumed.state.summary.sessionId,resumed.state.summary.generation))
        assertEquals("keep this original unsent draft",fixture.reducer().composer(fixture.target))
    }

    private suspend fun reopen(provider: Provider) = withFixture(provider) { fixture ->
        control("pause")
        var paused = true
        try {
            // State enqueues the old SUB frame while actual transport output is held behind
            // its SQLite guard. It must precede any later unsubscribe acknowledgement.
            control("activity", "text" to JsonPrimitive("already queued before unsubscribe ack"))
            val reopened = async { fixture.connection.open(fixture.host, fixture.target) }
            control("resume")
            paused = false
            val snapshot = reopened.await()
            assertEquals(fixture.target, Target(snapshot.state.summary.sessionId, snapshot.state.summary.generation))
        } finally {
            if (paused) withContext(NonCancellable) { control("resume") }
        }
        control("activity", "text" to JsonPrimitive("only current subscription may deliver"))
        // This real request is an ordered transport barrier. A leaked old subscription causes
        // an unknown-SUB frame before this response, disconnecting the current implementation.
        fixture.connection.snapshot(fixture.host)
        await(fixture, "reopen") { (fixture.reducer().state?.activity?.text as? ContentText)?.text?.value == "only current subscription may deliver" }
        assertTrue(fixture.reducer().mutationsEnabled)
    }
    private suspend fun boundedResnapshots(provider: Provider) = withFixture(provider) { fixture ->
        repeat(40) { index ->
            val title = "actual resnapshot $index"
            control("resnapshot", "title" to JsonPrimitive(title))
            await(fixture, "repeated resnapshot $index") { fixture.reducer().state?.summary?.title == title && fixture.reducer().mutationsEnabled }
            fixture.connection.snapshot(fixture.host)
        }
        // Forty real opens plus a live host subscription exceed the host's actual32 bound
        // unless replacement unsubscribes old streams. No private subscription counter needed.
        assertEquals(1, fixture.connection.state.value.sessions.getValue(fixture.host).size)
    }
    private suspend fun recover(provider: Provider, generation: Boolean) = withFixture(provider) { fixture ->
        val retained = retained(fixture)
        val originalSource=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
        val originalHistory=fixture.reducer().history.toList()
        fixture.reducer().composer(fixture.target, "never retarget this draft")
        val replacement = target(control(if (generation) "generation_begin" else "source_begin"))
        await(fixture, "source invalidation") { !fixture.reducer().mutationsEnabled }
        noWrite(fixture, fixture.target)
        if (generation) {
            await(fixture, "authoritative replacement inventory") {
                fixture.connection.state.value.inventory[fixture.host]?.sessions?.singleOrNull()?.generation == replacement.generation
            }
            assertNotEquals(fixture.target.generation, replacement.generation)
        } else assertEquals(fixture.target, replacement)
        try { fixture.connection.open(fixture.host, fixture.target); fail("Install must reject the unavailable snapshot") }
        catch (failure: RemoteFailure) { assertEquals(if (generation) ErrorCode.StaleSession else ErrorCode.NotReady, failure.code) }
        // Read-only refresh must recover the same Target, never install a new generation.
        // A replacement stays visible in inventory for a separate explicit user selection.
        fixture.connection.snapshot(fixture.host)
        assertNull("Supported failed reopen leaves no installed route, but must retain recovery ownership",fixture.connection.state.value.sources[fixture.host]?.get(replacement))
        control("ready")
        if(generation){
            await(fixture,"ready replacement is inventory-only") {
                fixture.connection.state.value.inventory[fixture.host]?.sessions?.singleOrNull()?.let {it.generation==replacement.generation&&it.connectivity==SessionConnectivity.Ready}==true&&
                    fixture.connection.state.value.mutationReady[fixture.host]==true
            }
            assertNull(fixture.connection.state.value.sources[fixture.host]?.get(replacement))
            assertFalse(fixture.reducer().mutationsEnabled)
            assertEquals(originalHistory,fixture.reducer().history)
            assertEquals("never retarget this draft",fixture.reducer().composer(fixture.target))
            noWrite(fixture,replacement)
        }else{
            await(fixture,"same original Target recovery") {
                val summary=fixture.reducer().state?.summary
                summary!=null&&Target(summary.sessionId,summary.generation)==fixture.target&&fixture.reducer().mutationsEnabled
            }
            val recoveredSource=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
            assertNotEquals("Same-original recovery installs a fresh route",originalSource,recoveredSource)
        }
        val recovered = fixture.store.localAction(fixture.host, retained.id)!!
        assertTrue("Writes enable only after retained receipt reconciliation", recovered.authoritative)
        assertEquals(ReceiptState.Uncertain, recovered.receipt!!.state)
        assertEquals("Published hint must be the canonical committed durable outcome",recovered.receipt,fixture.connection.state.value.latestReceipt[fixture.host])
        assertEquals(fixture.target, recovered.metadata.target)
        assertEquals(fixture.target, recovered.receipt!!.target)
        if(generation)assertNull(fixture.reducer().composer(replacement))
        assertEquals("never retarget this draft",fixture.reducer().composer(fixture.target))
    }
    private suspend fun postPreSendContinuationWaitsForExistingRecovery(provider: Provider) = withFixture(provider) { fixture ->
        val request = Request(Uuid.random(), RequestBody.PromptSubmit(PromptSubmitParams(
            fixture.hello.bootId, fixture.target, PromptText("continue one original action after recovery"),
        )))
        val barrier = fixture.connection.pauseNewActionContinuation(fixture.host, request.id, NewActionContinuationBarrierPoint.BeforePreSend)
        val mutation = async { fixture.connection.mutate(fixture.host, request) }
        barrier.reached.await()
        assertNull("The unsent continuation waits after the initial local lookup, before pre-send commits an action", fixture.store.localAction(fixture.host, request.id))
        control("source_begin")
        await(fixture, "existing recovery becomes unavailable") { !fixture.reducer().mutationsEnabled }
        barrier.release.complete(Unit)
        barrier.recoveryWaited.await()
        control("ready")
        barrier.frameStarted.await()
        control("receipt", "action" to JsonPrimitive(request.id.value))
        val receipt = mutation.await()
        assertEquals(request.id, receipt.actionId)
        assertEquals(1, barrier.frameStarts.get())
        assertEquals(1, fixture.store.localActions(fixture.host, null, 100).count { it.metadata.actionId == request.id })
    }
    private suspend fun postPreSendContinuationCancellationDoesNotStartFrame(provider: Provider) = withFixture(provider) { fixture ->
        val request = Request(Uuid.random(), RequestBody.PromptSubmit(PromptSubmitParams(
            fixture.hello.bootId, fixture.target, PromptText("cancel before the original frame"),
        )))
        val barrier = fixture.connection.pauseNewActionContinuation(fixture.host, request.id, NewActionContinuationBarrierPoint.BeforePreSend)
        val mutation = async { fixture.connection.mutate(fixture.host, request) }
        barrier.reached.await()
        mutation.cancelAndJoin()
        barrier.release.complete(Unit)
        assertEquals(0, barrier.frameStarts.get())
        assertEquals(0, fixture.store.localActions(fixture.host, null, 100).count { it.metadata.actionId == request.id })
    }

    private suspend fun skillSelectorRequiresOriginalPublicationAndRetainsFingerprintPriority(provider:Provider) = withFixture(provider) {fixture->
        val params=PromptSubmitParams(fixture.hello.bootId,fixture.target,PromptText(""),SkillSelector(Uuid.random(),Uuid.random()))
        val request=Request(Uuid.random(),RequestBody.PromptSubmit(params))
        val barrier=fixture.connection.pauseNewActionContinuation(fixture.host,request.id,NewActionContinuationBarrierPoint.BeforePreSend)
        val mutation=async {runCatching {fixture.connection.mutate(fixture.host,request,ordinaryComposer=false)}}
        barrier.reached.await()
        assertNull(fixture.store.localAction(fixture.host,request.id))
        barrier.release.complete(Unit)
        val rejected=mutation.await().exceptionOrNull() as? StorePolicy ?: error("missing original skill publication must reject admission")
        assertEquals(ErrorCode.NotReady,rejected.code)
        assertEquals(0,barrier.frameStarts.get())
        assertNull(fixture.store.localAction(fixture.host,request.id))
        val key=fixture.secrets.receipt(fixture.host,fixture.store)
        try {assertTrue(fixture.store.preSend(fixture.host,request,key,ordinaryComposer=false) is PreSendDecision.NewAction)}
        finally {key.fill(0)}
        val changed=request.copy(body=RequestBody.PromptSubmit(params.copy(skill=params.skill!!.copy(reference=Uuid.random()))))
        val conflictBarrier=fixture.connection.pauseNewActionContinuation(fixture.host,request.id)
        val conflict=runCatching {fixture.connection.mutate(fixture.host,changed,ordinaryComposer=false)}.exceptionOrNull() as? StorePolicy ?: error("a changed selector must retain keyed UUID/body conflict precedence")
        assertEquals(ErrorCode.ActionIdConflict,conflict.code)
        assertEquals(0,conflictBarrier.frameStarts.get())
        assertFalse(fixture.store.localAction(fixture.host,request.id)!!.authoritative)
    }

    private suspend fun postPreSendContinuationReturnsRecoveredReceiptWithoutFrame(provider: Provider) = withFixture(provider) { fixture ->
        val request = Request(Uuid.random(), RequestBody.PromptSubmit(PromptSubmitParams(
            fixture.hello.bootId, fixture.target, PromptText("return recovered canonical receipt without replay"),
        )))
        val barrier = fixture.connection.pauseNewActionContinuation(fixture.host, request.id)
        val mutation = async { fixture.connection.mutate(fixture.host, request) }
        barrier.reached.await()
        control("source_begin")
        await(fixture, "existing recovery becomes unavailable") { !fixture.reducer().mutationsEnabled }
        barrier.release.complete(Unit)
        barrier.recoveryWaited.await()
        control("receipt", "action" to JsonPrimitive(request.id.value))
        control("ready")
        val receipt = mutation.await()
        assertEquals(request.id, receipt.actionId)
        assertEquals(0, barrier.frameStarts.get())
        assertTrue(fixture.store.localAction(fixture.host, request.id)?.authoritative == true)
    }
    private suspend fun postPreSendContinuationRejectsReplacementTarget(provider: Provider) = withFixture(provider) { fixture ->
        val request = Request(Uuid.random(), RequestBody.PromptSubmit(PromptSubmitParams(
            fixture.hello.bootId, fixture.target, PromptText("never send against a replacement target"),
        )))
        val barrier = fixture.connection.pauseNewActionContinuation(fixture.host, request.id)
        val mutation = async { fixture.connection.mutate(fixture.host, request) }
        barrier.reached.await()
        val replacement = target(control("generation_begin"))
        assertNotEquals(fixture.target, replacement)
        await(fixture, "replacement recovery becomes unavailable") { !fixture.reducer().mutationsEnabled }
        barrier.release.complete(Unit)
        barrier.recoveryWaited.await()
        control("ready")
        try { mutation.await(); fail("Replacement target must reject the original unsent action") }
        catch(failure: StorePolicy) { assertEquals(ErrorCode.StaleSession, failure.code) }
        assertEquals(0, barrier.frameStarts.get())
    }

    private suspend fun recoveryPublicationWithholdsMutationReady(provider: Provider) = withFixture(provider) { fixture ->
        assertTrue("Fixture begins only after its host receipt reconciliation is published",fixture.connection.state.value.mutationReady[fixture.host]==true)
        val withheldWhileSessionReady=AtomicBoolean(false)
        val observer=launch(Dispatchers.Unconfined) {
            fixture.connection.state.collect { view ->
                val reducer=view.sessions[fixture.host]?.singleOrNull()
                if(view.mutationReady[fixture.host]!=true && reducer?.mutationsEnabled==true) withheldWhileSessionReady.set(true)
            }
        }
        try {
            // snapshot clears host receipt readiness before its first awaited request, so the
            // published UI gate must close while the existing session reducer remains ready.
            fixture.connection.snapshot(fixture.host)
            assertTrue("Host receipt reconciliation must withhold publication while the existing session remains ready",withheldWhileSessionReady.get())
            assertTrue("Writes re-enable only after host receipt reconciliation",fixture.connection.state.value.mutationReady[fixture.host]==true)
        } finally {observer.cancelAndJoin()}
    }
    @Test fun review_stateflow_observes_snapshot_before_live_subscription_event() = runBlocking {
        withFixture(Provider.Omp) { fixture ->
            val title = "observer snapshot seq0"
            val activity = "observer live event seq1"
            val observations = mutableListOf<Pair<String, String?>>()
            val observer = launch(start = CoroutineStart.UNDISPATCHED) {
                fixture.connection.state.collect { view ->
                    val state = view.sessions[fixture.host]?.singleOrNull()?.state
                    if (state != null) observations += state.summary.title to (state.activity.text as? ContentText)?.text?.value
                }
            }
            try {
                // This new title is installed only by the real session.open seq0 response.
                control("resnapshot", "title" to JsonPrimitive(title))
                await(fixture, "observable authoritative snapshot") {
                    fixture.reducer().mutationsEnabled && observations.any { it.first == title }
                }
                // The replacement source's first live event follows its snapshot on the same stream.
                control("activity", "text" to JsonPrimitive(activity))
                await(fixture, "observable live event") { observations.any { it.first == title && it.second == activity } }
                val snapshot = observations.indexOfFirst { it.first == title && it.second != activity }
                val event = observations.indexOfFirst { it.first == title && it.second == activity }
                assertTrue("The StateFlow consumer must observe the snapshot before the seq1 activity", snapshot >= 0 && event > snapshot)
            } finally { observer.cancelAndJoin() }
        }
    }
    @Test fun review_actual_subscription_sources_cache_and_navigation_follow_automatic_reopen() = runBlocking {
        withFixture(Provider.Omp) {fixture->
            val initial=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
            val reopened=fixture.connection.open(fixture.host,fixture.target)
            assertEquals(reopened.subscription,fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target))
            assertNotEquals(initial,reopened.subscription)
            control("resnapshot","title" to JsonPrimitive("source-aware automatic reopen"))
            await(fixture,"actual source replacement") {fixture.reducer().mutationsEnabled && fixture.reducer().state?.summary?.title=="source-aware automatic reopen"}
            val current=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
            assertNotEquals(reopened.subscription,current)
            try {
                fixture.connection.cacheContent(fixture.host,fixture.target,reopened.subscription,ContentRef(Uuid.random(),Uint(1),Mime.TextPlain),"x")
                fail("A previous subscription must never relabel loaded content as the new source")
            } catch(failure: StorePolicy) {assertEquals(ErrorCode.ContentExpired,failure.code)}
            fixture.connection.retireConversationSubscription(fixture.host,fixture.target)?.let {fixture.connection.leave(it)}
            assertNull(fixture.connection.state.value.sources[fixture.host]?.get(fixture.target))
            assertFalse(fixture.reducer().mutationsEnabled)
            control("activity","text" to JsonPrimitive("after leaving current automatic subscription"))
            fixture.connection.snapshot(fixture.host)
            assertTrue(fixture.connection.state.value.hello.containsKey(fixture.host))
            assertNull(fixture.connection.state.value.sources[fixture.host]?.get(fixture.target))
        }
    }
    @Test fun review_real_content_reads_source_safe_cache_and_sqlite_reopen() = runBlocking {
        withFixture(Provider.Codex) {fixture->
            suspend fun sample(name: String): ContentRef {
                val response=control("completed_content","sample" to JsonPrimitive(name))
                val reference=Protocol.strictJson.decodeFromJsonElement(Content.serializer(),response.getValue("content")) as ContentRef
                await(fixture,"completed native fixture reference") {fixture.reducer().history.singleOrNull()?.text==reference}
                return reference
            }
            suspend fun load(source: Uuid,reference: ContentRef): String {
                val assembly=ContentAssembly(fixture.target,reference)
                try {
                    do {val result=fixture.connection.content(fixture.host,source,assembly.request());assembly.accept(result)} while(!result.eof)
                    return assembly.text()
                } finally {assembly.clear()}
            }
            val reference=sample("original")
            val source=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
            val completed=load(source,reference)
            assertEquals("synthetic redacted completed body A\n"+"€".repeat(21846),completed)
            fixture.connection.cacheContent(fixture.host,fixture.target,source,reference,completed)
            // A separately reopened actual SQLite helper must recover completed text from disk.
            val reopened=SessionStore(context,fixture.databaseName)
            try {reopened.open();assertEquals(completed,reopened.offlineContent(fixture.host,fixture.target,reference.handle))}
            finally {reopened.close()}
            control("resnapshot","title" to JsonPrimitive("loaded-content source replacement"))
            await(fixture,"loaded source replacement") {fixture.reducer().mutationsEnabled && fixture.reducer().state?.summary?.title=="loaded-content source replacement"}
            val replacementSource=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
            assertNotEquals(source,replacementSource)
            assertNull(fixture.store.offlineContent(fixture.host,fixture.target,reference.handle))
            val replacementReference=sample("replacement")
            val replacement=load(replacementSource,replacementReference)
            fixture.connection.cacheContent(fixture.host,fixture.target,replacementSource,replacementReference,replacement)
            try {fixture.connection.cacheContent(fixture.host,fixture.target,source,reference,completed);fail("Late prior-source completed text must not overwrite current cache")}
            catch(failure: StorePolicy) {assertEquals(ErrorCode.ContentExpired,failure.code)}
            try {fixture.connection.content(fixture.host,source,ContentReadParams(fixture.target,reference.handle,Uint(0),Uint(65536)));fail("Previous subscription cannot read into current display")}
            catch(failure: StorePolicy) {assertEquals(ErrorCode.ContentExpired,failure.code)}
            assertEquals(replacement,fixture.store.offlineContent(fixture.host,fixture.target,replacementReference.handle))
            assertEquals(replacement,fixture.connection.cachedContent(fixture.host,fixture.target,replacementSource,replacementReference))
            control("completed_content","sample" to JsonPrimitive("inline_replacement"))
            await(fixture,"new current completed history body") {fixture.reducer().history.singleOrNull()?.text==ContentText(InlineText("synthetic current inline completed body"))}
            try {fixture.connection.cacheContent(fixture.host,fixture.target,replacementSource,replacementReference,replacement);fail("A late body for a removed current reference must not replace the new history body")}
            catch(failure: StorePolicy) {assertEquals(ErrorCode.InvalidRequest,failure.code)}
            val expired=sample("expired")
            try {load(replacementSource,expired);fail("Actual host TTL expiry must remain unavailable, not cached as success")}
            catch(failure: RemoteFailure) {assertEquals(ErrorCode.ContentExpired,failure.code)}
            fixture.connection.shutdown()
            val offline=SessionStore(context,fixture.databaseName)
            try {offline.open();assertEquals(replacement,offline.offlineContent(fixture.host,fixture.target,replacementReference.handle));assertTrue(offline.state.value.cached.single().stale)}
            finally {offline.close()}
        }
    }

    private suspend fun sentReplies(fixture:Fixture,count:Int) {
        withTimeout(10000) {
            while(fixture.connection.pendingReplies(fixture.host)!=count) {
                assertTrue("Canceled native read must retain its authenticated transport",fixture.host in fixture.connection.state.value.hello)
                delay(10)
            }
        }
    }
    private suspend fun canceledReads(provider:Provider) = withFixture(provider) {fixture->
        val source=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
        // Eighty canceled real requests exceed the owner's64 slots unless validated late
        // responses release them. Each wave stays below the host's own64-request bound.
        repeat(2) {
            control("pause")
            var paused=true
            try {
                repeat(40) {index->
                    val read=async {fixture.connection.notificationsSync(fixture.host,fixture.hello)}
                    try {
                        sentReplies(fixture,index+1)
                        read.cancelAndJoin()
                        assertEquals(index+1,fixture.connection.pendingReplies(fixture.host))
                        assertTrue(fixture.host in fixture.connection.state.value.hello)
                    } finally {read.cancelAndJoin()}
                }
                control("resume");paused=false
                fixture.connection.snapshot(fixture.host)
                sentReplies(fixture,0)
                assertEquals(source,fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target))
                assertTrue(fixture.reducer().mutationsEnabled)
            } finally {if(paused) withContext(NonCancellable) {control("resume")}}
        }
    }
    private suspend fun timedOutRead(provider:Provider) = withFixture(provider) {fixture->
        control("pause")
        var paused=true
        val read=async {fixture.connection.notificationsSync(fixture.host,fixture.hello)}
        try {
            sentReplies(fixture,1)
            try {withTimeout(20000) {read.await()};fail("The actual native read must retain its existing15-second caller deadline")}
            catch(_:TimeoutCancellationException) {assertTrue("The request deadline, not the test wait, must expire",read.isCancelled)}
            assertEquals("A response-timeout reply remains strictly correlated",1,fixture.connection.pendingReplies(fixture.host))
            assertTrue(fixture.host in fixture.connection.state.value.hello)
            control("resume");paused=false
            fixture.connection.snapshot(fixture.host)
            sentReplies(fixture,0)
            assertTrue(fixture.reducer().mutationsEnabled)
        } finally {
            read.cancelAndJoin()
            if(paused) withContext(NonCancellable) {control("resume")}
        }
    }
    private suspend fun canceledOpens(provider:Provider) = withFixture(provider) {fixture->
        fixture.connection.retireConversationSubscription(fixture.host,fixture.target)?.let {fixture.connection.leave(it)}
        // A late open must retire only its own source, never overwrite/retire the newer
        // installed open. Forty iterations also exceed the native32-subscription bound.
        repeat(40) {index->
            control("pause")
            var paused=true
            val opened=async {fixture.connection.open(fixture.host,fixture.target)}
            try {
                sentReplies(fixture,1)
                opened.cancelAndJoin()
                fixture.connection.retireConversationSubscription(fixture.host,fixture.target)?.let {fixture.connection.leave(it)}
                assertEquals(1,fixture.connection.pendingReplies(fixture.host))
                val replacement=async {fixture.connection.open(fixture.host,fixture.target)}
                try {
                    sentReplies(fixture,2)
                    control("resume");paused=false
                    val current=replacement.await()
                    // Cleanup cannot block the reader on operations while replacement owns
                    // that mutex waiting for its own native response.
                    fixture.connection.snapshot(fixture.host)
                    withTimeout(10000) {while(fixture.connection.pendingReplies(fixture.host,sentOnly=false)!=0) delay(10)}
                    assertEquals(current.subscription,fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target))
                    assertTrue(fixture.reducer().mutationsEnabled)
                    control("activity","text" to JsonPrimitive("after canceled old open $index"))
                    fixture.connection.snapshot(fixture.host)
                    await(fixture,"newer subscription survives old-source cleanup") {(fixture.reducer().state?.activity?.text as? ContentText)?.text?.value=="after canceled old open $index"}
                    assertEquals(current.subscription,fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target))
                    fixture.connection.retireConversationSubscription(fixture.host,fixture.target)?.let {fixture.connection.leave(it)}
                    assertNull(fixture.connection.state.value.sources[fixture.host]?.get(fixture.target))
                } finally {replacement.cancelAndJoin()}
            } finally {
                opened.cancelAndJoin()
                if(paused) withContext(NonCancellable) {control("resume")}
            }
        }
        fixture.connection.open(fixture.host,fixture.target)
        assertTrue(fixture.reducer().mutationsEnabled)
    }
    private suspend fun lateHistory(provider:Provider,cancel:Boolean) = withFixture(provider) {fixture->
        val source=fixture.connection.state.value.sources.getValue(fixture.host).getValue(fixture.target)
        control("pause")
        var paused=true
        val history=async {
            try {fixture.connection.history(fixture.host,fixture.target);null}
            catch(failure:StorePolicy) {failure}
        }
        try {
            sentReplies(fixture,1)
            if(cancel) history.cancelAndJoin()
            control("source_begin");control("ready")
            control("resume");paused=false
            if(!cancel) {
                val failure=history.await()
                assertNotNull("A reply cannot merge through its replaced subscription ticket",failure)
                assertEquals(ErrorCode.CursorExpired,failure!!.code)
            }
            await(fixture,"late history ticket rejection preserves native source recovery") {
                val current=fixture.connection.state.value.sources[fixture.host]?.get(fixture.target)
                current!=null && current!=source && fixture.reducer().mutationsEnabled
            }
            fixture.connection.snapshot(fixture.host)
            sentReplies(fixture,0)
            assertTrue(fixture.host in fixture.connection.state.value.hello)
        } finally {
            history.cancelAndJoin()
            if(paused) withContext(NonCancellable) {control("resume")}
        }
    }
    @Test fun review_canceled_native_reads_validate_late_replies_and_reuse_slots_omp() = runBlocking {canceledReads(Provider.Omp)}
    @Test fun review_canceled_native_reads_validate_late_replies_and_reuse_slots_codex() = runBlocking {canceledReads(Provider.Codex)}
    @Test fun review_native_read_timeout_keeps_late_reply_correlated_omp() = runBlocking {timedOutRead(Provider.Omp)}
    @Test fun review_native_read_timeout_keeps_late_reply_correlated_codex() = runBlocking {timedOutRead(Provider.Codex)}
    @Test fun review_canceled_native_open_cleans_only_its_returned_subscription_omp() = runBlocking {canceledOpens(Provider.Omp)}
    @Test fun review_canceled_native_open_cleans_only_its_returned_subscription_codex() = runBlocking {canceledOpens(Provider.Codex)}
    @Test fun review_late_native_history_guard_rejects_without_transport_teardown_omp() = runBlocking {lateHistory(Provider.Omp,false)}
    @Test fun review_late_native_history_guard_rejects_without_transport_teardown_codex() = runBlocking {lateHistory(Provider.Codex,false)}
    @Test fun review_canceled_late_native_history_keeps_source_recovery_omp() = runBlocking {lateHistory(Provider.Omp,true)}
    @Test fun review_canceled_late_native_history_keeps_source_recovery_codex() = runBlocking {lateHistory(Provider.Codex,true)}

    @Test fun review_reopen_preserves_queued_old_frames_and_unsubscribes_omp() = runBlocking { reopen(Provider.Omp) }
    @Test fun review_reopen_preserves_queued_old_frames_and_unsubscribes_codex() = runBlocking { reopen(Provider.Codex) }
    @Test fun review_more_than_32_automatic_resnapshots_stay_bounded_omp() = runBlocking { boundedResnapshots(Provider.Omp) }
    @Test fun review_more_than_32_automatic_resnapshots_stay_bounded_codex() = runBlocking { boundedResnapshots(Provider.Codex) }
    @Test fun review_generation_changed_keeps_original_read_only_without_retargeting_omp() = runBlocking {recover(Provider.Omp,true)}
    @Test fun review_generation_changed_keeps_original_read_only_without_retargeting_codex() = runBlocking {recover(Provider.Codex,true)}
    @Test fun review_source_install_not_ready_then_ready_recovers_omp() = runBlocking { recover(Provider.Omp, false) }
    @Test fun review_source_install_not_ready_then_ready_recovers_codex() = runBlocking { recover(Provider.Codex, false) }
    @Test fun review_recovery_publication_withholds_mutation_ready_until_receipts_reconciled_omp() = runBlocking { recoveryPublicationWithholdsMutationReady(Provider.Omp) }
    @Test fun review_recovery_publication_withholds_mutation_ready_until_receipts_reconciled_codex() = runBlocking { recoveryPublicationWithholdsMutationReady(Provider.Codex) }
    @Test fun review_post_pre_send_continuation_waits_for_existing_recovery_omp() = runBlocking { postPreSendContinuationWaitsForExistingRecovery(Provider.Omp) }
    @Test fun review_post_pre_send_continuation_cancel_before_frame_omp() = runBlocking { postPreSendContinuationCancellationDoesNotStartFrame(Provider.Omp) }
    @Test fun review_post_pre_send_continuation_returns_recovered_receipt_without_frame_omp() = runBlocking { postPreSendContinuationReturnsRecoveredReceiptWithoutFrame(Provider.Omp) }
    @Test fun review_post_pre_send_continuation_rejects_replacement_target_omp() = runBlocking { postPreSendContinuationRejectsReplacementTarget(Provider.Omp) }
    @Test fun review_skill_selector_requires_original_publication_and_fingerprint_codex() = runBlocking {skillSelectorRequiresOriginalPublicationAndRetainsFingerprintPriority(Provider.Codex)}
    @Test fun review_skill_selector_requires_original_publication_and_fingerprint_omp() = runBlocking {skillSelectorRequiresOriginalPublicationAndRetainsFingerprintPriority(Provider.Omp)}
    @Test fun review_background_original_subscription_cleanup_cannot_close_new_foreground_omp() = runBlocking {backgroundRetiresOnlyOriginalConversation(Provider.Omp)}
    @Test fun review_background_original_subscription_cleanup_cannot_close_new_foreground_codex() = runBlocking {backgroundRetiresOnlyOriginalConversation(Provider.Codex)}
    @Test fun review_canceled_explicit_open_preserves_original_cleanup_omp() = runBlocking {canceledExplicitOpenPreservesDisplacedCleanup(Provider.Omp)}
    @Test fun review_canceled_explicit_open_preserves_original_cleanup_codex() = runBlocking {canceledExplicitOpenPreservesDisplacedCleanup(Provider.Codex)}
    @Test fun review_history_upsert_preserves_unaffected_sqlite_history_and_offline_content() = runBlocking {
        withFixture(Provider.Omp) {fixture->
            control("pending_view","sample" to JsonPrimitive("history"))
            await(fixture,"actual completed source window cached") {
                fixture.reducer().history.size==12 && fixture.reducer().mutationsEnabled &&
                    fixture.store.state.value.cached.firstOrNull {it.hostId==fixture.host}?.history?.size==12
            }
            fun persistedRecords():Map<Id,Pair<Long,HistoryRecord>> = android.database.sqlite.SQLiteDatabase.openDatabase(context.getDatabasePath(fixture.databaseName).path,null,android.database.sqlite.SQLiteDatabase.OPEN_READONLY).use {db->
                db.rawQuery("SELECT id,at,data FROM records WHERE host=? AND session=?",arrayOf(fixture.host,fixture.target.sessionId.value)).use {rows->
                    buildMap {while(rows.moveToNext()) put(Id(rows.getString(0)),rows.getLong(1) to Protocol.strictJson.decodeFromString(HistoryRecord.serializer(),rows.getString(2)))}
                }
            }
            val before=persistedRecords()
            assertEquals(12,before.size)
            delay(25)
            val response=control("completed_content","sample" to JsonPrimitive("inline_replacement"))
            val content=Protocol.strictJson.decodeFromJsonElement(Content.serializer(),response.getValue("content"))
            val changed=Id("fixture-completed-body")
            await(fixture,"actual upsert committed to completed cache") {
                fixture.store.state.value.cached.firstOrNull {it.hostId==fixture.host}?.history?.any {it.id==changed && it.text==content}==true
            }
            val merged=fixture.reducer().history.single {it.id==changed}
            val after=persistedRecords()
            assertEquals("A single native upsert must preserve every unrelated completed record and its persistence time",before,after.filterKeys {it in before})
            assertEquals("Cache must contain the installed post-merge record",merged,after.getValue(changed).second)
            fixture.connection.shutdown()
            val offline=SessionStore(context,fixture.databaseName)
            try {
                offline.open()
                val retained=offline.state.value.cached.single {it.hostId==fixture.host}
                assertTrue(retained.stale)
                assertEquals(before.values.map {it.second}.toSet()+merged,retained.history.toSet())
            } finally {offline.close()}
        }
    }

    @Test fun review_transient_activity_never_rewrites_completed_sqlite_cache() = runBlocking {
        withFixture(Provider.Omp) {fixture->
            fun used():Long = android.database.sqlite.SQLiteDatabase.openDatabase(context.getDatabasePath(fixture.databaseName).path,null,android.database.sqlite.SQLiteDatabase.OPEN_READONLY).use {db->
                db.rawQuery("SELECT used FROM sessions WHERE host=? AND session=?",arrayOf(fixture.host,fixture.target.sessionId.value)).use {row->assertTrue(row.moveToFirst());row.getLong(0)}
            }
            val before=used()
            delay(25)
            repeat(4) {control("activity","text" to JsonPrimitive("transient native display $it"))}
            fixture.connection.snapshot(fixture.host)
            await(fixture,"actual transient activity") {(fixture.reducer().state?.activity?.text as? ContentText)?.text?.value=="transient native display 3"}
            assertEquals("Transient activity must not perform a per-token completed-cache write",before,used())
        }
    }
    @Test fun review_actual_conversation_keeps_new_pending_forms_and_composer_reachable_at_history_top() = runBlocking {
        val wrapping=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt an existing installation identity",wrapping.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        IrohAndroid.installAndroidContext(context)
        val reset=control("reset","provider" to JsonPrimitive("omp"))
        val host=reset.getValue("host").jsonPrimitive.content
        val selected=target(reset)
        control("pending_view","sample" to JsonPrimitive("history"))
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner:GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.foreground && owner.connection.ownsNetworkCallback()}
            assertTrue(owner.store.state.value.hosts.isEmpty())
            val endpoint=owner.connection.start()
            val alias=SecretStore.hostAlias(host,"receipt")
            owner.store.host(HostMetadata(host,"synthetic native UI host",keyAliases=listOf(SecretStore.ENDPOINT_ALIAS)))
            owner.connection.secrets.material(alias,true).fill(0)
            owner.store.host(HostMetadata(host,"synthetic native UI host",keyAliases=listOf(SecretStore.ENDPOINT_ALIAS,alias)))
            val authorized=control("authorize","endpoint" to JsonPrimitive(endpoint))
            owner.store.host(HostMetadata(host,"synthetic native UI host",Uuid(authorized.getValue("device").jsonPrimitive.content),listOf(SecretStore.ENDPOINT_ALIAS,alias)))
            owner.connection.reconnect(host)
            ui.runOnIdle {owner.open(host,selected)}
            ui.waitUntil(20000) {owner.connection.state.value.sources[host]?.get(selected)!=null && !owner.busy}
            ui.onNodeWithTag("conversation_list").performScrollToIndex(0)
            val previousSource=owner.connection.state.value.sources[host]?.get(selected)
            control("pending_view","sample" to JsonPrimitive("open"))
            // Reducers are retained mutable references: pending can change before the owner
            // commits its completed cache and publishes the replacement source to GoatrApp.
            ui.waitUntil(20000) {
                val view=owner.connection.state.value
                val source=view.sources[host]?.get(selected)
                source!=null && source!=previousSource && view.inventoryReady[host]==true &&
                    view.sessions[host]?.singleOrNull()?.let {it.pending.size==2 && it.mutationsEnabled}==true
            }
            // This remains the actual retained GoatrApp/ConversationScreen and authenticated owner.
            // A lazy tail after twelve tall records must not hide new blocking requests.
            scenario.onActivity {activity->activity.setContent {
                val density=LocalDensity.current
                GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,2f)) {
                    // Full production scaffold in short available space, including app chrome.
                    Box(Modifier.size(320.dp,320.dp)) {GoatrApp(owner) {sensitive->
                        if(sensitive) activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
                        else activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                    }}
                }}
            }}
            ui.onNodeWithTag("conversation_list").performScrollToIndex(12)
            ui.onNodeWithTag("conversation_refresh").assertDoesNotExist()
            stockAwait(owner,host,selected,refreshReady=true)
            ui.onNodeWithTag("conversation_refresh").assertIsDisplayed().assertIsEnabled()
            ui.onNodeWithTag("request_count").assertIsDisplayed().assertTextEquals("2 need your answer")
            ui.onNodeWithTag("prompt_send").assertIsDisplayed()
            ui.onNodeWithTag("turn_stop").assertIsDisplayed()
            ui.onNodeWithTag("omp_branches").performScrollTo().assertIsDisplayed()
            ui.onAllNodesWithTag("form_OmpEditor").assertCountEquals(0)
            val requests=owner.connection.state.value.sessions[host]!!.single().pending
            for(request in requests) {
                ui.onNodeWithTag("request_count").performScrollTo()
                ui.onNodeWithTag("request_select_${request.requestKey.value}").performScrollTo().performClick()
                ui.onAllNodesWithTag("form_OmpEditor").assertCountEquals(1)
                ui.onNodeWithTag("form_OmpEditor").assertIsDisplayed()
                ui.onNodeWithTag("request_editor_${request.requestKey.value}").assertIsDisplayed()
                ui.onNodeWithTag("request_stop").assertIsDisplayed()
                assertEquals(ConversationKey(host,selected),owner.selection)
                ui.onNodeWithTag("request_close").performClick()
            }
            assertTrue("Request selection must not retarget the phone conversation",owner.selection==ConversationKey(host,selected))
            assertEquals(2,owner.connection.state.value.sessions[host]?.singleOrNull()?.pending?.size)
            ui.onNodeWithTag("composer").performScrollTo().assertIsDisplayed().performClick().performTextInput("ordinary unsent native UI canary")
            ui.onNodeWithTag("prompt_send").assertIsDisplayed()
            ui.onNodeWithTag("turn_stop").assertIsDisplayed()
            assertEquals("ordinary unsent native UI canary",owner.composers[ConversationKey(host,selected)])
            assertTrue(owner.store.state.value.recovery.isEmpty())
        } finally {
            withContext(NonCancellable) {
                scenario.moveToState(Lifecycle.State.CREATED)
                owner.connection.shutdown()
                try {control("forget_peer")} finally {
                    if(owner.store.state.value.hosts.any {it.hostId==host}) owner.connection.forget(host)
                    for(obligation in owner.store.state.value.keyDeletion) owner.connection.secrets.delete(obligation,owner.store)
                    if(owner.store.state.value.hosts.isEmpty() && wrapping.containsAlias(SecretStore.ENDPOINT_ALIAS)) owner.connection.secrets.delete(KeyDeletion("synthetic-pending-ui-owner",listOf(SecretStore.ENDPOINT_ALIAS)),owner.store)
                    scenario.close()
                }
            }
        }
    }

    /** Actual stock checkpoint consumer. Unlike review_* this never authorizes a fake
     * provider projection: the same-package runner starts packaged serve/stock wrappers,
     * proves kernel/native/Herdr joins and independently checks native effects. */
    private suspend fun stockControl(method:String,vararg fields:Pair<String,JsonElement>):JsonObject=withContext(Dispatchers.IO) {
        val port=InstrumentationRegistry.getArguments().getString("stockControlPort")?.toIntOrNull()
            ?.takeIf {it in 1..65535} ?: error("Actual owned stock runner control is required; no fixture fallback")
        val bytes=buildJsonObject {put("method",method);fields.forEach {(key,value)->put(key,value)}}.toString().toByteArray(Charsets.UTF_8)
        require(bytes.size in 1..65536)
        val runner=InstrumentationRegistry.getInstrumentation() as? OwnedInstrumentationRunner
            ?: error("Owned lifecycle/signing runner required; no authorization fallback")
        try {Socket().use {socket->
            socket.connect(InetSocketAddress("127.0.0.1",port),5000);socket.soTimeout=5000
            val input=java.io.DataInputStream(socket.getInputStream())
            val challenge=ByteArray(32).also {input.readFully(it)}
            val signature=runner.controlSignature(challenge,bytes)
            java.io.DataOutputStream(socket.getOutputStream()).apply {
                write(runner.controlIdentity());writeByte(signature.size);write(signature);writeInt(bytes.size);write(bytes);flush()
            }
            challenge.fill(0);signature.fill(0);socket.soTimeout=240000
            val buffer=ByteArrayOutputStream()
            while(true) {val b=socket.getInputStream().read();check(b>=0) {"Owned actual stock control closed"};if(b==10) break;check(buffer.size()<65536);buffer.write(b)}
            val raw=buffer.toByteArray()
            try {Protocol.parseRawJson(raw) as JsonObject} finally {raw.fill(0);buffer.reset()}
        }} finally {bytes.fill(0)}
    }
    private fun stockTarget(row:JsonObject)=Protocol.strictJson.decodeFromJsonElement(Target.serializer(),row.getValue("target"))
    private fun stockProvider(row:JsonObject)=Protocol.strictJson.decodeFromJsonElement(Provider.serializer(),row.getValue("provider"))
    /** Closed, nonsecret progress only for the development continuity consumer. */
    private class DevelopmentAutomationCapture {
        private var count=0
        fun snapshot(phase:String,provider:Provider?,state:String) {
            check(phase in setOf("pairing","session_open","history_attempt","refresh","refresh_timeout","stock_await_timeout","send_wait","send_ready","send_admission","send_clicked","receipt_polling","receipt_timeout","receipt_display","native_observe","tool_history","desktop","reconnect","cleanup","cleanup_failure"))
            check(provider==null || provider in setOf(Provider.Omp,Provider.Codex))
            check(phase!="receipt_display" || provider!=null && state=="canonical_recovery_cleared")
            check(phase!="stock_await_timeout" || provider!=null)
            val line="$phase|${provider?.name?.lowercase() ?: "none"}|$state"
            check(state.isNotEmpty() && state.all {it.code in 33..126 && it!='|'})
            check(line.length<=if(phase=="receipt_timeout") 384 else 256)
            check(++count<=64) {"Development automation status bound"}
            InstrumentationRegistry.getInstrumentation().sendStatus(2,Bundle().apply {
                putString("goatrAutomationSnapshot",line)
            })
        }
    }
    private fun stockTap(tag:String,requireEnabled:Boolean=false,afterClick:(() -> Unit)?=null) {
        val node=ui.onNodeWithTag(tag)
        if(runCatching {node.assertIsDisplayed()}.isFailure) node.performScrollTo()
        if(requireEnabled) node.assertIsEnabled()
        node.performClick();afterClick?.invoke();ui.waitForIdle()
    }

    private data class PrivateInputEpoch(
        val activity:MainActivity,
        val window:android.view.Window,
        val decor:android.view.View,
        val token:android.os.IBinder,
        val baseWindow:Int,
    )

    private fun privateCheckpointWindow(
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
    ):PrivateInputEpoch? {
        var epoch:PrivateInputEpoch?=null
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val activity=runCatching {runner.sdkOwnedFixtureMainActivity()}.getOrNull() ?: return@runOnMainSync
            val window=activity.window;val decor=window.decorView;val token=decor.windowToken
            if(activity.lifecycle.currentState!=Lifecycle.State.RESUMED || activity.isFinishing || activity.isDestroyed ||
                ViewModelProvider(activity)[GoatrViewModel::class.java]!==owner || !decor.isAttachedToWindow || token==null ||
                !activity.hasWindowFocus() || !decor.hasWindowFocus()) return@runOnMainSync
            epoch=PrivateInputEpoch(activity,window,decor,token,-1)
        }
        val candidate=epoch ?: return null
        val root=InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: return null
        return try {
            if(root.packageName?.toString()!=candidate.activity.packageName || root.window?.type!=android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION || root.windowId<0) null
            else candidate.copy(baseWindow=root.windowId)
        } finally {root.recycle()}
    }

    private enum class PrivateCheckpointEligibility(val closed:String) {
        PreviewIdentity("preview_identity"), ActivityIdentity("activity_identity"), ActivityState("activity_state"), OwnerIdentity("owner_identity"), OwnerScene("owner_scene"), SecureState("secure_state"), WindowFocus("window_focus"), Interactive("interactive"), Keyguard("keyguard"), ActiveRoot("active_root"), RootIdentity("root_identity"), RootWindow("root_window"), Eligible("eligible"),
    }

    private fun privateCheckpointInputEligibility(
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        screen:Screen,
        preview:Any?,
        secure:Boolean,
        popup:Boolean=false,
    ):PrivateCheckpointEligibility {
        var cause=PrivateCheckpointEligibility.ActivityIdentity
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val activity=runCatching {runner.sdkOwnedFixtureMainActivity()}.getOrNull()
            cause=when {
                activity!==epoch.activity || activity?.window!==epoch.window || epoch.window.decorView!==epoch.decor || !epoch.decor.isAttachedToWindow || epoch.decor.windowToken!==epoch.token -> PrivateCheckpointEligibility.ActivityIdentity
                epoch.activity.isFinishing || epoch.activity.isDestroyed || epoch.activity.lifecycle.currentState!=Lifecycle.State.RESUMED -> PrivateCheckpointEligibility.ActivityState
                ViewModelProvider(epoch.activity)[GoatrViewModel::class.java]!==owner -> PrivateCheckpointEligibility.OwnerIdentity
                owner.preview!==preview -> PrivateCheckpointEligibility.PreviewIdentity
                !owner.foreground || owner.busy || owner.screen!=screen -> PrivateCheckpointEligibility.OwnerScene
                (epoch.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE!=0)!=secure -> PrivateCheckpointEligibility.SecureState
                !popup && (!epoch.activity.hasWindowFocus() || !epoch.decor.hasWindowFocus()) -> PrivateCheckpointEligibility.WindowFocus
                else -> PrivateCheckpointEligibility.Eligible
            }
        }
        if(cause!=PrivateCheckpointEligibility.Eligible)return cause
        val app=epoch.activity.applicationContext
        val power=app.getSystemService(android.os.PowerManager::class.java)
        val keyguard=app.getSystemService(android.app.KeyguardManager::class.java)
        if(power?.isInteractive!=true)return PrivateCheckpointEligibility.Interactive
        if(keyguard?.isKeyguardLocked!=false)return PrivateCheckpointEligibility.Keyguard
        val root=InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: return PrivateCheckpointEligibility.ActiveRoot
        return try {
            when {
                root.packageName?.toString()!=epoch.activity.packageName || root.window?.type!=android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION -> PrivateCheckpointEligibility.RootIdentity
                root.windowId<0 || if(popup) root.windowId==epoch.baseWindow else root.windowId!=epoch.baseWindow -> PrivateCheckpointEligibility.RootWindow
                else -> PrivateCheckpointEligibility.Eligible
            }
        } finally {root.recycle()}
    }

    private fun privateCheckpointGuard(
        stage:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        screen:Screen,
        preview:Any?,
        secure:Boolean,
        popup:Boolean=false,
    )=privateCheckpointRequire(stage,privateCheckpointInputEligibility(runner,owner,epoch,screen,preview,secure,popup)==PrivateCheckpointEligibility.Eligible)

    private fun privateCheckpointDeadline(stage:String,deadline:Long) {
        privateCheckpointRequire(stage,android.os.SystemClock.elapsedRealtime()<deadline)
    }

    private fun privateCheckpointTap(
        stage:String,
        tag:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        screen:Screen,
        preview:Any?,
        secure:Boolean,
        popup:Boolean=false,
        deadline:Long?=null,
        beforeClick:(()->Unit)?=null,
    ) {
        deadline?.let {privateCheckpointDeadline(stage,it)}
        privateCheckpointGuard(stage,runner,owner,epoch,screen,preview,secure,popup)
        val node=ui.onNodeWithTag(tag)
        if(runCatching {node.assertIsDisplayed()}.isFailure) node.performScrollTo()
        deadline?.let {privateCheckpointDeadline(stage,it)}
        node.assertIsDisplayed().assertIsEnabled()
        deadline?.let {privateCheckpointDeadline(stage,it)}
        privateCheckpointGuard(stage,runner,owner,epoch,screen,preview,secure,popup)
        beforeClick?.invoke()
        node.performClick();ui.waitForIdle();deadline?.let {privateCheckpointDeadline(stage,it)}
    }

    private fun privateCheckpointModalWindow(
        stage:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        screen:Screen,
        tag:String,
        deadline:Long,
    ):Int? {
        privateCheckpointDeadline(stage,deadline)
        if(privateCheckpointInputEligibility(runner,owner,epoch,screen,null,false,true)!=PrivateCheckpointEligibility.Eligible)return null
        val root=InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: return null
        return try {
            val window=root.window
            if(root.packageName?.toString()!=epoch.activity.packageName || window?.let {it.type==android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION && it.isActive && it.isFocused}!=true || root.windowId<0 || root.windowId==epoch.baseWindow) null
            else root.windowId.takeIf {privateCheckpointRawTag(root,epoch.activity.packageName,it,tag)}
        } finally {root.recycle()}
    }

    /** Compose exposes raw test tags as virtual view IDs, not framework resource IDs. */
    private fun privateCheckpointRawTag(root:AccessibilityNodeInfo,expectedPackage:String,expectedWindow:Int,tag:String):Boolean {
        val queue=java.util.ArrayDeque<AccessibilityNodeInfo>();queue.addLast(AccessibilityNodeInfo.obtain(root));var discovered=1;var matches=0
        try {
            while(queue.isNotEmpty()) {
                val node=queue.removeFirst()
                try {
                    if(node.packageName?.toString()!=expectedPackage || node.windowId!=expectedWindow)return false
                    if(node.viewIdResourceName==tag && node.isVisibleToUser) {matches++;if(matches>1)return false}
                    repeat(node.childCount) {index->
                        if(discovered>=512)return false
                        discovered++
                        val child=node.getChild(index) ?: return false
                        queue.addLast(child)
                    }
                } finally {node.recycle()}
            }
            return matches==1
        } finally {while(queue.isNotEmpty()) queue.removeFirst().recycle()}
    }

    private fun privateCheckpointModalGuard(
        stage:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        screen:Screen,
        window:Int,
        tag:String,
        deadline:Long,
    ) {
        privateCheckpointDeadline(stage,deadline)
        privateCheckpointRequire(stage,privateCheckpointInputEligibility(runner,owner,epoch,screen,null,false,true)==PrivateCheckpointEligibility.Eligible)
        val root=InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow
        val exact=if(root==null) false else try {
            val active=root.window
            root.packageName?.toString()==epoch.activity.packageName && active?.let {it.type==android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION && it.isActive && it.isFocused}==true && root.windowId==window && root.windowId!=epoch.baseWindow && privateCheckpointRawTag(root,epoch.activity.packageName,window,tag)
        } finally {root.recycle()}
        privateCheckpointRequire(stage,exact)
    }

    private data class PrivateTerminalDialog(
        val window:android.view.Window,
        val decor:android.view.View,
        val providerView:android.view.View,
        val windowToken:android.os.IBinder,
        val applicationWindowToken:android.os.IBinder,
        val baseApplicationToken:android.os.IBinder,
        val accessibilityWindowId:Int,
    )

    private fun privateCheckpointTerminalBaseApplicationToken(
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
    ):android.os.IBinder? {
        var token:android.os.IBinder?=null
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val activity=runCatching {runner.sdkOwnedFixtureMainActivity()}.getOrNull()
            if(activity===epoch.activity && activity.window===epoch.window && epoch.window.decorView===epoch.decor && epoch.decor.windowToken===epoch.token && ViewModelProvider(epoch.activity)[GoatrViewModel::class.java]===owner)token=epoch.window.attributes.token
        }
        return token
    }

    private fun privateCheckpointTerminalProviderContextIsActivity(context:android.content.Context,activity:MainActivity):Boolean {
        val seen=java.util.IdentityHashMap<android.content.Context,Boolean>();var current:android.content.Context?=context
        repeat(16) {
            val value=current ?: return false
            if(value===activity)return true
            if(seen.put(value,true)!=null)return false
            current=(value as? android.content.ContextWrapper)?.baseContext ?: return false
        }
        return false
    }

    private fun privateCheckpointTerminalPopupFailure(stage:String,suffix:String) {
        if(stage=="terminal_popup")privateCheckpointStage="terminal_popup_window_$suffix"
    }

    private fun privateCheckpointTerminalDialogWindow(
        stage:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        dialogTag:String,
        targetTag:String,
        controlTag:String,
        baseApplicationToken:android.os.IBinder,
        deadline:Long,
        screen:Screen=Screen.Conversation,
    ):PrivateTerminalDialog? {
        privateCheckpointTerminalPopupFailure(stage,"deadline");privateCheckpointDeadline(if(stage=="terminal_popup") "terminal_popup_window_deadline" else stage,deadline)
        if(android.os.Build.VERSION.SDK_INT<29) {privateCheckpointTerminalPopupFailure(stage,"sdk");return null}
        val eligibility=privateCheckpointInputEligibility(runner,owner,epoch,screen,null,true,true)
        if(eligibility!=PrivateCheckpointEligibility.Eligible) {privateCheckpointTerminalPopupFailure(stage,"eligibility_${eligibility.closed}");return null}
        val automation=InstrumentationRegistry.getInstrumentation().uiAutomation
        val popup=automation.rootInActiveWindow ?: run {privateCheckpointTerminalPopupFailure(stage,"active_root");return null}
        val popupWindowId=try {
            val window=popup.window
            when {
                popup.packageName?.toString()!=epoch.activity.packageName || window?.let {it.type==android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION && it.isActive && it.isFocused}!=true || popup.windowId<0 || popup.windowId==epoch.baseWindow -> {privateCheckpointTerminalPopupFailure(stage,"exact_popup_window");null}
                !privateCheckpointRawTag(popup,epoch.activity.packageName,popup.windowId,dialogTag) -> {privateCheckpointTerminalPopupFailure(stage,"dialog_tag");null}
                !privateCheckpointRawTag(popup,epoch.activity.packageName,popup.windowId,targetTag) -> {privateCheckpointTerminalPopupFailure(stage,"target_tag");null}
                !privateCheckpointRawTag(popup,epoch.activity.packageName,popup.windowId,controlTag) -> {privateCheckpointTerminalPopupFailure(stage,"control_tag");null}
                else -> popup.windowId
            }
        } finally {popup.recycle()}
        if(popupWindowId==null)return null
        var pinned:PrivateTerminalDialog?=null
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val roots=WindowInspector.getGlobalWindowViews()
            if(roots.size>64) {privateCheckpointTerminalPopupFailure(stage,"root_count");return@runOnMainSync}
            val matched=roots.filter {root->
                val node=root.createAccessibilityNodeInfo()
                try {node?.windowId==popupWindowId} finally {node?.recycle()}
            }
            if(matched.size!=1) {privateCheckpointTerminalPopupFailure(stage,"window_id_match");return@runOnMainSync}
            val root=matched.single()
            val queue=java.util.ArrayDeque<android.view.View>();queue.addLast(root);val providers=mutableListOf<android.view.View>();var seen=0
            while(queue.isNotEmpty()) {
                val view=queue.removeFirst()
                if(++seen>512) {privateCheckpointTerminalPopupFailure(stage,"root_tree_bound");return@runOnMainSync}
                if(view is DialogWindowProvider)providers+=view
                if(view is android.view.ViewGroup)repeat(view.childCount) {index->queue.addLast(view.getChildAt(index))}
            }
            if(providers.size!=1) {privateCheckpointTerminalPopupFailure(stage,"provider_count");return@runOnMainSync}
            val providerView=providers.single();val provider=providerView as DialogWindowProvider;val window=provider.window;val decor=window.decorView
            val token=root.windowToken;val applicationToken=root.applicationWindowToken;val currentBaseToken=epoch.window.attributes.token;val providerBaseToken=window.attributes.token
            if(decor!==root || providerView.rootView!==root) {privateCheckpointTerminalPopupFailure(stage,"decor");return@runOnMainSync}
            if(!privateCheckpointTerminalProviderContextIsActivity(providerView.context,epoch.activity)) {privateCheckpointTerminalPopupFailure(stage,"context");return@runOnMainSync}
            if(!root.isAttachedToWindow || !decor.isAttachedToWindow) {privateCheckpointTerminalPopupFailure(stage,"attach");return@runOnMainSync}
            if(token==null || applicationToken==null || currentBaseToken==null || providerBaseToken==null || currentBaseToken!=baseApplicationToken || providerBaseToken!=baseApplicationToken || decor.windowToken!=token || applicationToken!=token) {privateCheckpointTerminalPopupFailure(stage,"tokens");return@runOnMainSync}
            if(window.attributes.type!=WindowManager.LayoutParams.TYPE_APPLICATION) {privateCheckpointTerminalPopupFailure(stage,"type");return@runOnMainSync}
            if((window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE)==0) {privateCheckpointTerminalPopupFailure(stage,"secure");return@runOnMainSync}
            if(token==epoch.token) {privateCheckpointTerminalPopupFailure(stage,"tokens");return@runOnMainSync}
            pinned=PrivateTerminalDialog(window,decor,providerView,token,applicationToken,baseApplicationToken,popupWindowId)
        }
        return pinned
    }

    private fun privateCheckpointTerminalDialogGuard(
        stage:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        pinned:PrivateTerminalDialog,
        dialogTag:String,
        targetTag:String,
        controlTag:String,
        baseApplicationToken:android.os.IBinder,
        deadline:Long,
        screen:Screen=Screen.Conversation,
    ) {
        val current=checkNotNull(privateCheckpointTerminalDialogWindow(stage,runner,owner,epoch,dialogTag,targetTag,controlTag,baseApplicationToken,deadline,screen))
        val identical=current.window===pinned.window && current.decor===pinned.decor && current.providerView===pinned.providerView && current.windowToken==pinned.windowToken && current.applicationWindowToken==pinned.applicationWindowToken && current.baseApplicationToken==pinned.baseApplicationToken && current.accessibilityWindowId==pinned.accessibilityWindowId
        if(stage=="terminal_popup") {privateCheckpointStage="terminal_popup_window_identity";check(identical)} else privateCheckpointRequire(stage,identical)
    }

    private fun privateCheckpointModalTap(
        stage:String,
        tag:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        screen:Screen,
        window:Int,
        deadline:Long,
        scopeTag:String=tag,
        useUnmergedTree:Boolean=false,
        beforeClick:(()->Unit)?=null,
    ) {
        privateCheckpointModalGuard(stage,runner,owner,epoch,screen,window,scopeTag,deadline)
        val node=ui.onNodeWithTag(tag,useUnmergedTree=useUnmergedTree)
        if(runCatching {node.assertIsDisplayed()}.isFailure) node.performScrollTo()
        privateCheckpointDeadline(stage,deadline)
        node.assertIsDisplayed().assertIsEnabled()
        privateCheckpointModalGuard(stage,runner,owner,epoch,screen,window,scopeTag,deadline)
        beforeClick?.invoke()
        node.performClick();ui.waitForIdle();privateCheckpointDeadline(stage,deadline)
    }

    /** Selected-checkpoint-only close instrumentation; it keeps the normal modal tap's reads and order. */
    private fun privateCheckpointCloseCreationGuard(
        phase:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        window:Int,
        scopeTag:String,
        deadline:Long,
    ) {
        privateCheckpointStage="${phase}_deadline"
        privateCheckpointDeadline("${phase}_deadline",deadline)
        val eligibility=privateCheckpointInputEligibility(runner,owner,epoch,Screen.Sessions,null,false,true)
        privateCheckpointRequire("${phase}_eligibility_${eligibility.closed}",eligibility==PrivateCheckpointEligibility.Eligible)
        val root=InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: run {
            privateCheckpointRequire("${phase}_window",false)
            return
        }
        try {
            val active=root.window
            val qualified=root.packageName?.toString()==epoch.activity.packageName && active?.let {it.type==android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION && it.isActive && it.isFocused}==true && root.windowId==window && root.windowId!=epoch.baseWindow
            privateCheckpointRequire("${phase}_window",qualified)
            privateCheckpointRequire("${phase}_scope",privateCheckpointRawTag(root,epoch.activity.packageName,window,scopeTag))
        } finally {root.recycle()}
    }

    private fun privateCheckpointPickerTap(
        stage:String,
        optionTag:String,
        runner:OwnedInstrumentationRunner,
        owner:GoatrViewModel,
        epoch:PrivateInputEpoch,
        window:Int,
        deadline:Long,
    ) {
        privateCheckpointModalGuard(stage,runner,owner,epoch,Screen.Sessions,window,optionTag,deadline)
        val option=ui.onNode(hasAnyDescendant(hasTestTag(optionTag)) and hasClickAction(),useUnmergedTree=true)
        option.assertIsDisplayed().assertIsEnabled()
        privateCheckpointModalGuard(stage,runner,owner,epoch,Screen.Sessions,window,optionTag,deadline)
        option.performClick();ui.waitForIdle();privateCheckpointDeadline(stage,deadline)
    }

    private suspend fun privateCheckpointActions(owner:GoatrViewModel,host:String):List<LocalAction> {
        val actions=mutableListOf<LocalAction>();var after:Uuid?=null
        while(true) {
            val page=owner.store.localActions(host,after,100);actions+=page
            if(page.size<100)return actions
            after=page.last().metadata.actionId
        }
    }


    private fun privateCheckpointOwnedClip(clipboard:ClipboardManager,nonce:String)=clipboard.primaryClipDescription?.let {description->
        description.label?.toString()==nonce&&description.mimeTypeCount==1&&description.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN)&&description.extras?.getString("goatrOwnedClipboardNonce")==nonce
    }==true
    private enum class PrivateCheckpointDirectInputOutcome(val closed:String) {Success("success"),Authority("authority"),Window("direct_window"),Lookup("direct_lookup"),Focus("direct_focus"),Connection("direct_connection"),Commit("direct_commit")}
    private data class PrivateCheckpointShellReceiptObservation(val closed:String,val exact:Boolean)
    private fun privateCheckpointShellReceiptError(code:ErrorCode)=when(code) {
        ErrorCode.InvalidRequest->"error_invalid_request";ErrorCode.UnsupportedVersion->"error_unsupported_version";ErrorCode.UnsupportedMethod->"error_unsupported_method";ErrorCode.Unauthorized->"error_unauthorized";ErrorCode.PairingFailed->"error_pairing_failed";ErrorCode.StaleHost->"error_stale_host";ErrorCode.StaleSession->"error_stale_session";ErrorCode.NotFound->"error_not_found";ErrorCode.NotReady->"error_not_ready";ErrorCode.InvalidAnswer->"error_invalid_answer";ErrorCode.AlreadyResolved->"error_already_resolved";ErrorCode.ActionIdConflict->"error_action_id_conflict";ErrorCode.ActionCapacity->"error_action_capacity";ErrorCode.ContentExpired->"error_content_expired";ErrorCode.CursorExpired->"error_cursor_expired";ErrorCode.Busy->"error_busy";ErrorCode.ResourceLimit->"error_resource_limit";ErrorCode.RateLimited->"error_rate_limited";ErrorCode.Timeout->"error_timeout";ErrorCode.BackendError->"error_backend_error";ErrorCode.PushRejected->"error_push_rejected";ErrorCode.StaleRegistration->"error_stale_registration"
    }
    private fun privateCheckpointConnectionFailurePhase(phase:ConnectionFailurePhase)=when(phase) {
        ConnectionFailurePhase.Dial->"dial";ConnectionFailurePhase.OpenStream->"open_stream";ConnectionFailurePhase.Hello->"hello";ConnectionFailurePhase.Snapshot->"snapshot";ConnectionFailurePhase.Reader->"reader";ConnectionFailurePhase.Recovery->"recovery"
    }
    private fun privateCheckpointConnectionFailureCategory(category:ConnectionFailureCategory)=when(category) {
        ConnectionFailureCategory.Timeout->"timeout";ConnectionFailureCategory.Known->"known";ConnectionFailureCategory.Protocol->"protocol";ConnectionFailureCategory.Transport->"transport";ConnectionFailureCategory.Other->"other"
    }
    private fun privateCheckpointCaughtAbsentObservation(notice:String?,callbackOwned:Boolean):String {
        val code=ErrorCode.entries.firstOrNull {notice==safeFailure(StorePolicy(it))}
        if(code!=null)return when(code) {
            ErrorCode.NotReady->if(callbackOwned)"confirm_postclick_caught_absent_error_not_ready_callback_owned" else "confirm_postclick_caught_absent_error_not_ready_callback_absent"
            else->"confirm_postclick_caught_absent_${privateCheckpointShellReceiptError(code)}"
        }
        if(notice==safeFailure(ProtectedKeyUnavailable()))return "confirm_postclick_caught_absent_protected_identity"
        if(notice==safeFailure(IllegalStateException()))return "confirm_postclick_caught_absent_generic"
        return "confirm_postclick_caught_absent_unclassified"
    }
    private fun privateCheckpointConfirmPostclickObservation(helloPresent:Boolean,screen:Screen,busy:Boolean,previewCurrent:Boolean,hostStored:Boolean,startupConnecting:Boolean,failure:ConnectionFailure?,dialSupplied:Boolean?,caughtAbsentNotice:String?,callbackOwned:Boolean):String {
        if(helloPresent&&screen!=Screen.Sessions)return "confirm_postclick_hello_present_screen_mismatch"
        if(helloPresent&&busy)return "confirm_postclick_hello_present_busy"
        failure?.code?.let {return "confirm_postclick_${privateCheckpointShellReceiptError(it)}"}
        failure?.let {if(it.phase==ConnectionFailurePhase.Dial&&it.category==ConnectionFailureCategory.Timeout)return when(dialSupplied) {false->"confirm_postclick_failure_dial_timeout_endpoint_setup";true->"confirm_postclick_failure_dial_timeout_connect_admitted";null->"confirm_postclick_failure_dial_timeout_origin_unavailable"};return "confirm_postclick_failure_${privateCheckpointConnectionFailurePhase(it.phase)}_${privateCheckpointConnectionFailureCategory(it.category)}"}
        if(hostStored&&busy)return "confirm_postclick_stored_busy_nofailure"
        if(!hostStored&&busy&&previewCurrent)return if(startupConnecting)"confirm_postclick_prestore_startup_connecting" else "confirm_postclick_prestore_no_startup"
        if(!busy&&!previewCurrent)return if(hostStored)"confirm_postclick_caught_stored" else privateCheckpointCaughtAbsentObservation(caughtAbsentNotice,callbackOwned)
        if(screen==Screen.Sessions)return "confirm_postclick_sessions_without_hello"
        if(screen==Screen.Pairing&&!busy)return "confirm_postclick_pairing_idle"
        return "confirm_postclick_other_model"
    }
    private fun privateCheckpointShellInteractionReceiptObservation(actions:List<LocalAction>,before:Set<Uuid>,boot:Uuid,target:ShellTarget,shellOwner:TerminalInputOwner,pause:TerminalInputPause?,exact:(LocalAction)->Boolean):PrivateCheckpointShellReceiptObservation {
        val delta=actions.filter {it.metadata.actionId !in before}
        if(delta.size>1)return PrivateCheckpointShellReceiptObservation("multiple",false)
        if(delta.isEmpty()) {
            val closed=when {
                shellOwner.paused->when(pause) {TerminalInputPause.Capacity->"paused_capacity";TerminalInputPause.SourceLost->"paused_source_lost";TerminalInputPause.Uncertain->"paused_uncertain";TerminalInputPause.Refused->"paused_refused";TerminalInputPause.Limit->"paused_limit";TerminalInputPause.Unsupported->"paused_unsupported";TerminalInputPause.Closed->"paused_closed";null->"paused_unknown"}
                !shellOwner.retained()->"unretained"
                shellOwner.historyBlocked->"history_blocked"
                shellOwner.pendingEvents>0||shellOwner.pendingTextBytes>0->"queued"
                shellOwner.accepting()->"accepting_empty"
                else->"not_accepting"
            }
            return PrivateCheckpointShellReceiptObservation(closed,false)
        }
        val action=delta.single();val metadata=action.metadata;val receipt=action.receipt
        if(metadata.method!=ActionMethod.ShellInput||metadata.bootId!=boot||metadata.target!=null||metadata.sourceTarget!=null||metadata.shellTarget!=target||receipt!=null&&(receipt.actionId!=metadata.actionId||receipt.method!=ActionMethod.ShellInput||receipt.bootId!=boot||receipt.target!=null||receipt.shellTarget!=target))return PrivateCheckpointShellReceiptObservation("metadata_binding",false)
        if(!action.authoritative)return PrivateCheckpointShellReceiptObservation("non_authoritative",false)
        if(receipt==null)return PrivateCheckpointShellReceiptObservation("no_receipt",false)
        if(exact(action))return PrivateCheckpointShellReceiptObservation("wait",true)
        receipt.error?.let {return PrivateCheckpointShellReceiptObservation(privateCheckpointShellReceiptError(it.code),false)}
        val closed=when(receipt.state) {
            ReceiptState.Reserved->"reserved";ReceiptState.Dispatching->"dispatching"
            ReceiptState.Dispatched->when(val result=receipt.result) {null->"dispatched_none";is ActionResultShellDispatch->"dispatched_dispatch_partial";is ActionResultShellInput->if(result.progress.accepted())"dispatched_input_accepted" else "dispatched_input_partial";else->"dispatched_none"}
            ReceiptState.Succeeded->when(val result=receipt.result) {null->"succeeded_none";is ActionResultShellDispatch->if(result.progress.complete())"succeeded_dispatch_complete" else "succeeded_dispatch_partial";is ActionResultShellInput->"succeeded_input_partial";else->"succeeded_none"}
            ReceiptState.Failed->when(receipt.result) {is ActionResultShellDispatch->"failed_dispatch";is ActionResultShellInput->"failed_input";else->"failed_none"}
            ReceiptState.Uncertain->when(receipt.result) {is ActionResultShellDispatch->"uncertain_dispatch";is ActionResultShellInput->"uncertain_input";else->"uncertain_none"}
        }
        return PrivateCheckpointShellReceiptObservation(closed,false)
    }
    private fun privateCheckpointDirectTerminalInput(pinned:PrivateTerminalDialog,admitted:()->Boolean):PrivateCheckpointDirectInputOutcome {
        var outcome=PrivateCheckpointDirectInputOutcome.Success
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            fun pinnedMetadata()=pinned.window.decorView===pinned.decor&&pinned.providerView.rootView===pinned.decor&&pinned.decor.isAttachedToWindow&&pinned.decor.hasWindowFocus()&&pinned.window.attributes.type==WindowManager.LayoutParams.TYPE_APPLICATION&&pinned.window.attributes.token==pinned.baseApplicationToken&&(pinned.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE)!=0&&pinned.decor.windowToken==pinned.windowToken&&pinned.decor.applicationWindowToken==pinned.applicationWindowToken
            if(!admitted()){outcome=PrivateCheckpointDirectInputOutcome.Authority;return@runOnMainSync}
            if(!pinnedMetadata()){outcome=PrivateCheckpointDirectInputOutcome.Window;return@runOnMainSync}
            val queue=ArrayDeque<View>();queue.add(pinned.decor);var direct:DirectTerminalInputView?=null;var visited=0
            while(queue.isNotEmpty()&&visited++<512) {val view=queue.removeFirst();if(view is DirectTerminalInputView) {if(direct!=null){direct=null;break};direct=view};if(view is ViewGroup)for(index in 0 until view.childCount)queue.add(view.getChildAt(index))}
            if(direct?.rootView!==pinned.decor||direct?.isAttachedToWindow!=true){outcome=PrivateCheckpointDirectInputOutcome.Lookup;return@runOnMainSync}
            if(!direct.requestFocus()||!direct.hasWindowFocus()){outcome=PrivateCheckpointDirectInputOutcome.Focus;return@runOnMainSync}
            if(!admitted()){outcome=PrivateCheckpointDirectInputOutcome.Authority;return@runOnMainSync}
            if(!pinnedMetadata()){outcome=PrivateCheckpointDirectInputOutcome.Window;return@runOnMainSync}
            val connection=direct.onCreateInputConnection(EditorInfo()) ?: run {outcome=PrivateCheckpointDirectInputOutcome.Connection;return@runOnMainSync}
            if(!admitted()){outcome=PrivateCheckpointDirectInputOutcome.Authority;return@runOnMainSync}
            if(!pinnedMetadata()){outcome=PrivateCheckpointDirectInputOutcome.Window;return@runOnMainSync}
            if(!connection.commitText(" ",1))outcome=PrivateCheckpointDirectInputOutcome.Commit
        }
        return outcome
    }

    /** A single real private-paste pairing checkpoint. The host owns the invitation lifetime
     * and only releases its finish acknowledgement after the exact authenticated device delta. */
    @Test fun installed_subject_private_pairing_checkpoint()=runBlocking<Unit> {
        privateCheckpointRequire("selector",privateCheckpointSelected())
        val runner=InstrumentationRegistry.getInstrumentation() as? OwnedInstrumentationRunner
        privateCheckpointRequire("runner",runner!=null)
        // The empty Compose rule has now installed its real test environment. Launching begins
        // only from the stock runner lifecycle callback; no test-owned composition is created.
        privateCheckpointStage="awaiting_foreground"
        runner!!.prepareSdkOwnedFixture()
        privateCheckpointStage="activity_capture_fixture"
        val activity=runner!!.sdkOwnedFixtureMainActivity()
        lateinit var owner:GoatrViewModel
        privateCheckpointStage="activity_capture_owner"
        runner.runOnMainSync {owner=ViewModelProvider(activity)[GoatrViewModel::class.java]}
        privateCheckpointStage="activity_capture_foreground"
        ui.waitUntil(20_000) {owner.foreground && owner.connection.ownsNetworkCallback()}
        lateinit var epoch:PrivateInputEpoch
        privateCheckpointStage="activity_capture_window"
        ui.waitUntil(10_000) {
            val current=privateCheckpointWindow(runner,owner)
            if(current==null) false else {epoch=current;true}
        }
        privateCheckpointGuard("initial_scene",runner,owner,epoch,Screen.Sessions,null,false)
        val beforeHosts=owner.store.state.value.hosts.toList()
        val beforeAliases=beforeHosts.associate {it.hostId to it.keyAliases.toList()}
        val beforeIds=beforeHosts.map {it.hostId}.toSet()
        privateCheckpointStage="creation_selection"
        val shellCheckpoint=InstrumentationRegistry.getArguments().getString("goatrPairingShellCheckpoint")=="true"
        val shellInteractionCheckpoint=InstrumentationRegistry.getArguments().getString("goatrPairingShellInteractionCheckpoint")=="true"
        val requestedCreationProvider=when(InstrumentationRegistry.getArguments().getString("goatrPairingProvider")) {
            "none"->null
            "omp"->Provider.Omp
            "codex"->Provider.Codex
            else->{privateCheckpointRequire("creation_selection",false);null}
        }
        privateCheckpointRequire("creation_selection",!(shellCheckpoint&&requestedCreationProvider!=null)&&(!shellInteractionCheckpoint||shellCheckpoint))
        privateCheckpointStage="live_info"
        val infoStarted=android.os.SystemClock.elapsedRealtime()
        val info=stockControl("live_info")
        val expectedHost=info.getValue("host").jsonPrimitive.content
        val expectedName=info.getValue("name").jsonPrimitive.content
        val checkpointBudget=info.getValue("deadlineMs").jsonPrimitive.long
        privateCheckpointRequire("creation_selection",checkpointBudget in 1L..120_000L && info.getValue("provider").jsonPrimitive.content==when(requestedCreationProvider) {null->"none";Provider.Omp->"omp";Provider.Codex->"codex"} && info.getValue("shellCheckpoint").jsonPrimitive.boolean==shellCheckpoint && info.getValue("shellInteractionCheckpoint").jsonPrimitive.boolean==shellInteractionCheckpoint)
        val checkpointDeadline=infoStarted+checkpointBudget
        privateCheckpointRequire("host_fresh",expectedHost !in beforeIds)

        privateCheckpointTap("navigate_menu","navigation_menu",runner,owner,epoch,Screen.Sessions,null,false)
        privateCheckpointTap("navigate_hosts","navigation_hosts_menu",runner,owner,epoch,Screen.Sessions,null,false,popup=true)
        ui.waitUntil(10_000) {owner.screen==Screen.Connections}
        val pairBeginAdmission=privateCheckpointInputEligibility(runner,owner,epoch,Screen.Connections,null,false)
        privateCheckpointRequire("pair_begin_admission_${pairBeginAdmission.closed}",pairBeginAdmission==PrivateCheckpointEligibility.Eligible)
        val pairBeginNodeAdmission=privateCheckpointInputEligibility(runner,owner,epoch,Screen.Connections,null,false)
        privateCheckpointRequire("pair_begin_node_admission_${pairBeginNodeAdmission.closed}",pairBeginNodeAdmission==PrivateCheckpointEligibility.Eligible)
        privateCheckpointStage="pair_begin_node_lookup"
        val pairBegin=ui.onNodeWithTag("pair_begin")
        privateCheckpointStage="pair_begin_node_visibility"
        if(runCatching {pairBegin.assertIsDisplayed()}.isFailure) {
            privateCheckpointStage="pair_begin_node_scroll"
            privateCheckpointGuard("pair_begin_node_scroll",runner,owner,epoch,Screen.Connections,null,false)
            ui.onNodeWithTag("connections_list").performScrollToNode(hasTestTag("pair_begin"))
        }
        privateCheckpointStage="pair_begin_node_enabled"
        pairBegin.assertIsDisplayed().assertIsEnabled()
        val pairBeginRevalidation=privateCheckpointInputEligibility(runner,owner,epoch,Screen.Connections,null,false)
        privateCheckpointRequire("pair_begin_revalidation_${pairBeginRevalidation.closed}",pairBeginRevalidation==PrivateCheckpointEligibility.Eligible)
        privateCheckpointStage="pair_begin_click"
        pairBegin.performClick()
        privateCheckpointStage="pair_begin_idle"
        ui.waitForIdle()
        privateCheckpointStage="pair_begin_postclick_wait"
        ui.waitUntil(10_000) {owner.screen==Screen.Pairing && owner.preview==null && activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE!=0}
        privateCheckpointGuard("pairing_secure",runner,owner,epoch,Screen.Pairing,null,true)

        var invitation:String?=null
        var pasted=false
        var previewed=false
        try {
            privateCheckpointStage="invite_once"
            invitation=stockControl("live_pair").getValue("uri").jsonPrimitive.content
            privateCheckpointRequire("invite_nonempty",invitation!!.isNotEmpty())
            privateCheckpointGuard("paste_admission",runner,owner,epoch,Screen.Pairing,null,true)
            val paste=ui.onNodeWithTag("pair_paste").performScrollTo().assertIsDisplayed().assertIsEnabled()
            privateCheckpointGuard("paste_admission",runner,owner,epoch,Screen.Pairing,null,true)
            pasted=true
            paste
                .performSemanticsAction(SemanticsActions.SetText) {it(AnnotatedString(checkNotNull(invitation)))}
            privateCheckpointGuard("preview_admission",runner,owner,epoch,Screen.Pairing,null,true)
            privateCheckpointRequire("preview_once",!previewed)
            previewed=true
            privateCheckpointTap("preview_admission","pair_preview",runner,owner,epoch,Screen.Pairing,null,true)
        } finally {invitation=null}

        privateCheckpointStage="confirm_admission"
        var confirmWait=PrivateCheckpointEligibility.PreviewIdentity
        try {
            ui.waitUntil(10_000) {
                val candidate=owner.preview
                val previewExact=candidate?.host?.value==expectedHost && candidate?.name?.value==expectedName && candidate?.fingerprint==expectedHost.chunked(8).joinToString(" ")
                if(!previewExact) {confirmWait=PrivateCheckpointEligibility.PreviewIdentity;false}
                else {
                    val eligibility=privateCheckpointInputEligibility(runner,owner,epoch,Screen.Pairing,candidate,false)
                    confirmWait=eligibility
                    eligibility==PrivateCheckpointEligibility.Eligible
                }
            }
        } catch(error:ComposeTimeoutException) {
            privateCheckpointStage="confirm_wait_${confirmWait.closed}"
            throw error
        }
        val preview=owner.preview
        privateCheckpointRequire("preview_exact",preview?.let {it.host.value==expectedHost && it.name.value==expectedName && it.fingerprint==expectedHost.chunked(8).joinToString(" ")}==true && owner.store.state.value.hosts==beforeHosts && owner.connection.state.value.hello.keys.none {it==expectedHost})
        privateCheckpointGuard("confirm_admission",runner,owner,epoch,Screen.Pairing,preview,false)
        privateCheckpointGuard("confirm_admission",runner,owner,epoch,Screen.Pairing,preview,false)
        val confirm=ui.onNodeWithTag("pair_confirm")
        if(runCatching {confirm.assertIsDisplayed()}.isFailure) confirm.performScrollTo()
        confirm.assertIsDisplayed().assertIsEnabled()
        privateCheckpointGuard("confirm_admission",runner,owner,epoch,Screen.Pairing,preview,false)
        privateCheckpointStage="confirm_click"
        confirm.performClick()
        privateCheckpointStage="confirm_idle"
        ui.waitForIdle()
        privateCheckpointStage="confirm_postclick_wait"
        privateCheckpointDeadline("confirm_postclick_wait",checkpointDeadline)
        var confirmPostclickObservation="confirm_postclick_other_model"
        try {
            ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {
                val connection=owner.connection.state.value
                val observedHello=connection.hello[expectedHost]
                val observedScreen=owner.screen
                val observedBusy=owner.busy
                val observedPreviewCurrent=owner.preview===preview
                val observedHostStored=owner.store.state.value.hosts.any {it.hostId==expectedHost}
                val observedStartupConnecting=owner.connectingHosts.isNotEmpty()
                val observedFailure=connection.connectionFailures[expectedHost]
                val observedCaughtAbsent=observedFailure==null&&!observedBusy&&!observedPreviewCurrent&&!observedHostStored
                val observedCaughtAbsentNotice=if(observedCaughtAbsent)owner.notice else null
                val observedCaughtAbsentCallbackOwned=if(observedCaughtAbsent)owner.connection.ownsNetworkCallback() else false
                confirmPostclickObservation=privateCheckpointConfirmPostclickObservation(observedHello!=null,observedScreen,observedBusy,observedPreviewCurrent,observedHostStored,observedStartupConnecting,observedFailure,connection.routeAttempts[expectedHost]?.supplied,observedCaughtAbsentNotice,observedCaughtAbsentCallbackOwned)
                observedHello!=null && observedScreen==Screen.Sessions && !observedBusy
            }
        } catch(error:ComposeTimeoutException) {
            privateCheckpointStage=confirmPostclickObservation
            throw error
        }
        privateCheckpointStage="confirm_postclick_wait"
        privateCheckpointDeadline("confirm_postclick_wait",checkpointDeadline)
        val hello=owner.connection.state.value.hello[expectedHost]
        val paired=owner.store.state.value.hosts.firstOrNull {it.hostId==expectedHost}
        val device=hello?.deviceId
        val afterHosts=owner.store.state.value.hosts
        val expectedAliases=listOf(SecretStore.ENDPOINT_ALIAS,SecretStore.hostAlias(expectedHost,"receipt"))
        privateCheckpointRequire("pairing_exact",hello?.hostId?.value==expectedHost && device!=null && paired?.let {it.hostId==expectedHost && it.name==expectedName && it.deviceId==device && it.keyAliases==expectedAliases}==true && afterHosts.map {it.hostId}.toSet()==beforeIds+expectedHost &&
            beforeHosts.all {prior->afterHosts.firstOrNull {it.hostId==prior.hostId}===prior || afterHosts.firstOrNull {it.hostId==prior.hostId}==prior} &&
            beforeAliases.all {(host,aliases)->afterHosts.firstOrNull {it.hostId==host}?.keyAliases==aliases} &&
            afterHosts.count {it.hostId==expectedHost}==1)
        val pairedHello=checkNotNull(hello);val pairedMetadata=checkNotNull(paired);val pairedDevice=checkNotNull(device)
        privateCheckpointGuard("finish_admission",runner,owner,epoch,Screen.Sessions,null,false)
        privateCheckpointRequire("finish_publication",owner.connection.state.value.hello[expectedHost]===pairedHello &&
            owner.store.state.value.hosts.firstOrNull {it.hostId==expectedHost}===pairedMetadata && pairedHello.hostId.value==expectedHost &&
            pairedMetadata.name==expectedName && pairedMetadata.deviceId==pairedDevice && pairedMetadata.keyAliases==expectedAliases && owner.host==expectedHost)
        privateCheckpointStage="finish_once"
        val finish=stockControl("pairing_finish","device_id" to JsonPrimitive(pairedDevice.value))
        privateCheckpointRequire("finish_exact",finish.getValue("retained").jsonPrimitive.boolean)
        if(shellCheckpoint) {
            val shellTarget=ShellTarget(Uuid(info.getValue("shellId").jsonPrimitive.content),Uuid(info.getValue("shellGeneration").jsonPrimitive.content))
            val shellServer=Uuid(info.getValue("serverId").jsonPrimitive.content)
            val shellWorkspace=Id(info.getValue("workspaceId").jsonPrimitive.content)
            val shellTab=Id(info.getValue("tabId").jsonPrimitive.content)
            val shellPane=Id(info.getValue("paneId").jsonPrimitive.content)
            val shellTerminal=Id(info.getValue("terminalId").jsonPrimitive.content)
            privateCheckpointRequire("shell_admission",finish.getValue("shellCheckpoint").jsonPrimitive.boolean && finish.getValue("shellInteractionCheckpoint").jsonPrimitive.boolean==shellInteractionCheckpoint)
            fun shellCurrent()=owner.connection.state.value.inventory[expectedHost]?.let {inventory->
                inventory.sessions.isEmpty() && owner.connection.state.value.inventoryReady[expectedHost]==true && inventory.servers.singleOrNull {it.serverId==shellServer && it.connectivity==ServerConnectivity.Online}!=null &&
                    inventory.hierarchy.singleOrNull {it.serverId==shellServer && it.freshness==NativeFreshness.Current}?.let {graph->graph.panes.singleOrNull {it.paneId==shellPane && it.workspaceId==shellWorkspace && it.tabId==shellTab && it.terminalId==shellTerminal && it.shellTarget==shellTarget && it.target==null && it.unboundRowId==null}!=null}==true &&
                    inventory.shells.singleOrNull {it.target==shellTarget && it.serverId==shellServer && it.workspaceId==shellWorkspace && it.tabId==shellTab && it.paneId==shellPane && it.terminalId==shellTerminal && it.connectivity==SessionConnectivity.Ready && it.capabilities.read && it.capabilities.input && it.capabilities.grid}!=null
            }==true
            val shellBeforeActions=privateCheckpointActions(owner,expectedHost)
            privateCheckpointRequire("shell_admission",shellBeforeActions.isEmpty())
            privateCheckpointDeadline("shell_admission",checkpointDeadline)
            privateCheckpointRequire("shell_admission",shellCurrent() && owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.connection.state.value.mutationReady[expectedHost]==true)
            privateCheckpointStage="shell_open"
            privateCheckpointTap("shell_open","host_terminals_$expectedHost",runner,owner,epoch,Screen.Sessions,null,false,deadline=checkpointDeadline)
            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {owner.screen==Screen.Terminals && owner.host==expectedHost && shellCurrent()}
            privateCheckpointDeadline("shell_open",checkpointDeadline)
            privateCheckpointGuard("shell_open",runner,owner,epoch,Screen.Terminals,null,false)
            val shellBaseToken=checkNotNull(privateCheckpointTerminalBaseApplicationToken(runner,owner,epoch))
            val shellOpen=ui.onNodeWithTag("shell_open_${shellTarget.shellId.value}")
            shellOpen.assertIsDisplayed().assertIsEnabled()
            privateCheckpointGuard("shell_open",runner,owner,epoch,Screen.Terminals,null,false)
            shellOpen.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_open",checkpointDeadline)
            val shellScope="shell_scope_${expectedHost}_${shellTarget.shellId.value}_${shellTarget.generation.value}"
            privateCheckpointStage="shell_viewer"
            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {owner.shell?.let {it.selection==ShellSelection(expectedHost,shellTarget) && it.grid==null && !it.controlBegun && runCatching {ui.onNodeWithTag("shell_viewer").assertIsDisplayed();ui.onNodeWithTag(shellScope,useUnmergedTree=true).assertIsDisplayed()}.isSuccess}==true}
            val shellViewer=checkNotNull(privateCheckpointTerminalDialogWindow("shell_viewer",runner,owner,epoch,"shell_viewer",shellScope,"shell_begin_control",shellBaseToken,checkpointDeadline,Screen.Terminals))
            privateCheckpointTerminalDialogGuard("shell_viewer",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_begin_control",shellBaseToken,checkpointDeadline,Screen.Terminals)
            privateCheckpointStage="shell_begin"
            val shellBegin=ui.onNodeWithTag("shell_begin_control")
            shellBegin.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_begin",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_begin_control",shellBaseToken,checkpointDeadline,Screen.Terminals);shellBegin.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_begin",checkpointDeadline)
            val beginScope="shell_begin_scope_${expectedHost}_${shellTarget.shellId.value}_${shellTarget.generation.value}"
            privateCheckpointStage="shell_confirm"
            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(beginScope).assertIsDisplayed();ui.onNodeWithTag("shell_begin_confirm").assertIsDisplayed()}.isSuccess}
            val confirmWindow=checkNotNull(privateCheckpointTerminalDialogWindow("shell_confirm",runner,owner,epoch,beginScope,beginScope,"shell_begin_confirm",shellBaseToken,checkpointDeadline,Screen.Terminals))
            val shellConfirm=ui.onNodeWithTag("shell_begin_confirm");shellConfirm.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_confirm",runner,owner,epoch,confirmWindow,beginScope,beginScope,"shell_begin_confirm",shellBaseToken,checkpointDeadline,Screen.Terminals);shellConfirm.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_confirm",checkpointDeadline)
            privateCheckpointStage="shell_grid"
            ui.waitUntil(minOf(20_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {
                val shown=owner.shell
                val observation=when {
                    shown==null->"shell_grid_missing_shell"
                    shown.selection!=ShellSelection(expectedHost,shellTarget)->"shell_grid_selection"
                    !shown.controlBegun->"shell_grid_control"
                    shown.loading->"shell_grid_loading"
                    shown.error->"shell_grid_error"
                    else->shown.grid?.let {grid->when {
                        !runCatching {grid.begin.validate()}.isSuccess->"shell_grid_invalid_grid"
                        grid.begin.columns.value<=0L->"shell_grid_columns"
                        grid.begin.rows.value<=0L->"shell_grid_rows"
                        grid.cells.size.toLong()!=grid.begin.columns.value*grid.begin.rows.value->"shell_grid_cardinality"
                        !runCatching {ui.onNodeWithTag("terminal_grid").assertIsDisplayed()}.isSuccess->"shell_grid_terminal_grid"
                        else->{
                            val enter=runCatching {ui.onNodeWithTag("shell_key_Enter")}.getOrNull()
                            when {
                                enter==null->"shell_grid_enter_displayed"
                                !runCatching {enter.assertIsDisplayed()}.isSuccess->"shell_grid_enter_displayed"
                                !runCatching {enter.assertIsEnabled()}.isSuccess->"shell_grid_enter_enabled"
                                else->"shell_grid"
                            }
                        }
                    }} ?: "shell_grid_missing_grid"
                }
                privateCheckpointStage=observation
                observation=="shell_grid"
            }
            privateCheckpointStage="shell_grid"
            val committedShell=checkNotNull(owner.shell);val committedGrid=checkNotNull(committedShell.grid);val shellScopeState=ShellControlScope(committedShell.selection,committedShell.source,committedShell.ticket);val shellOwner=checkNotNull(owner.terminalOwner(shellScopeState));val shellBeforeActionIds=shellBeforeActions.map {it.metadata.actionId}.toSet()
            privateCheckpointRequire("shell_grid",committedShell.selection==ShellSelection(expectedHost,shellTarget) && committedShell.controlBegun && !committedShell.loading && !committedShell.error && committedGrid.begin.revision.value>=0L && owner.shellInputAllowed(shellScopeState) && owner.terminalOwner(shellScopeState)===shellOwner && shellBeforeActions==privateCheckpointActions(owner,expectedHost))
            fun exactShellInput(action:LocalAction)=action.authoritative&&action.metadata.method==ActionMethod.ShellInput&&action.metadata.bootId==pairedHello.bootId&&action.metadata.target==null&&action.metadata.sourceTarget==null&&action.metadata.shellTarget==shellTarget&&action.receipt?.let {receipt->receipt.actionId==action.metadata.actionId&&receipt.method==ActionMethod.ShellInput&&receipt.bootId==pairedHello.bootId&&receipt.target==null&&receipt.shellTarget==shellTarget&&receipt.error==null&&when(val result=receipt.result) {is ActionResultShellDispatch->receipt.state==ReceiptState.Dispatched&&result.progress.complete();is ActionResultShellInput->receipt.state==ReceiptState.Succeeded&&result.progress.accepted();else->false}}==true
            fun shellInteractionAuthority(expectedActionIds:Set<Uuid>)=shellCurrent()&&owner.host==expectedHost&&owner.connection.state.value.hello[expectedHost]===pairedHello&&owner.connection.state.value.mutationReady[expectedHost]==true&&owner.shell?.let {it.selection==committedShell.selection&&it.source==committedShell.source&&it.ticket==committedShell.ticket&&it.controlBegun&&!it.loading&&!it.error}==true&&owner.terminalOwner(shellScopeState)===shellOwner&&shellOwner.retained()&&owner.actions[expectedHost].orEmpty().map {it.metadata.actionId}.toSet()==expectedActionIds
            var interactionInputs:List<LocalAction>?=null;var interactionPreEnterIds:Set<Uuid>?=null
            suspend fun shellInteractionInputCheckpoint() {
                privateCheckpointRequire("shell_interaction_admission",pairedHello.features.shellHistory && committedGrid.begin.scroll!=null)
                val fixture="i=0; while [ \"\$i\" -lt 300 ]; do printf '.\\n'; i=\$((i+1)); done; unset i"
                privateCheckpointRequire("shell_interaction_fixture",fixture.encodeToByteArray().size==71)
                privateCheckpointTerminalDialogGuard("shell_interaction_clipboard_empty",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_draft",shellBaseToken,checkpointDeadline,Screen.Terminals)
                privateCheckpointRequire("shell_interaction_clipboard_empty",shellInteractionAuthority(shellBeforeActionIds))
                val clipboard=checkNotNull(context.getSystemService(ClipboardManager::class.java));privateCheckpointRequire("shell_interaction_clipboard_empty",!clipboard.hasPrimaryClip())
                val nonce=Uuid.random().value;val changes=AtomicInteger();val listener=ClipboardManager.OnPrimaryClipChangedListener {changes.incrementAndGet()};clipboard.addPrimaryClipChangedListener(listener)
                var draftInput:LocalAction?=null;var directInput:LocalAction?=null
                try {
                    val description=ClipDescription(nonce,arrayOf(ClipDescription.MIMETYPE_TEXT_PLAIN)).apply {extras=PersistableBundle().apply {putString("goatrOwnedClipboardNonce",nonce)}}
                    privateCheckpointTerminalDialogGuard("shell_interaction_clipboard_owned",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_draft",shellBaseToken,checkpointDeadline,Screen.Terminals)
                    privateCheckpointRequire("shell_interaction_clipboard_owned",shellInteractionAuthority(shellBeforeActionIds))
                    InstrumentationRegistry.getInstrumentation().runOnMainSync {
                        // Clipboard installation must still belong to the admitted secure viewer.
                        privateCheckpointRequire("shell_interaction_clipboard_owned",shellInteractionAuthority(shellBeforeActionIds)&&shellViewer.window.decorView===shellViewer.decor&&shellViewer.decor.isAttachedToWindow&&shellViewer.decor.hasWindowFocus()&&shellViewer.decor.windowToken==shellViewer.windowToken&&shellViewer.decor.applicationWindowToken==shellViewer.applicationWindowToken&&shellViewer.window.attributes.token==shellBaseToken&&(shellViewer.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE)!=0&&!clipboard.hasPrimaryClip())
                        clipboard.setPrimaryClip(ClipData(description,ClipData.Item(fixture)))
                    }
                    privateCheckpointStage="shell_interaction_clipboard_owned";ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {changes.get()==1&&privateCheckpointOwnedClip(clipboard,nonce)};privateCheckpointDeadline("shell_interaction_clipboard_owned",checkpointDeadline)
                    privateCheckpointRequire("shell_interaction_clipboard_owned",changes.get()==1&&privateCheckpointOwnedClip(clipboard,nonce))
                    privateCheckpointStage="shell_interaction_draft";val draft=ui.onNodeWithTag("shell_input_draft");draft.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_interaction_draft",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_draft",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointRequire("shell_interaction_draft",shellInteractionAuthority(shellBeforeActionIds));draft.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_interaction_draft",checkpointDeadline)
                    privateCheckpointStage="shell_interaction_paste";val paste=ui.onNodeWithTag("shell_input_paste");paste.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_interaction_paste",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_paste",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointRequire("shell_interaction_paste",privateCheckpointOwnedClip(clipboard,nonce)&&changes.get()==1&&shellInteractionAuthority(shellBeforeActionIds));paste.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_interaction_paste",checkpointDeadline)
                    privateCheckpointRequire("shell_interaction_paste",owner.shellDraft(committedShell.selection).value.text==fixture&&owner.shellDraft(committedShell.selection).revision>0L&&privateCheckpointActions(owner,expectedHost).map {it.metadata.actionId}.toSet()==shellBeforeActionIds)
                    privateCheckpointTerminalDialogGuard("shell_interaction_clipboard_clear",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_paste",shellBaseToken,checkpointDeadline,Screen.Terminals)
                    privateCheckpointRequire("shell_interaction_clipboard_clear",privateCheckpointOwnedClip(clipboard,nonce)&&changes.get()==1&&shellInteractionAuthority(shellBeforeActionIds))
                    InstrumentationRegistry.getInstrumentation().runOnMainSync {if(privateCheckpointOwnedClip(clipboard,nonce)&&changes.get()==1)clipboard.clearPrimaryClip()}
                    privateCheckpointStage="shell_interaction_clipboard_clear";ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {!clipboard.hasPrimaryClip()&&changes.get()==2};privateCheckpointDeadline("shell_interaction_clipboard_clear",checkpointDeadline)
                    privateCheckpointRequire("shell_interaction_clipboard_clear",!clipboard.hasPrimaryClip()&&changes.get()==2)
                    privateCheckpointStage="shell_interaction_draft_send";val send=ui.onNodeWithTag("shell_input_send");send.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_interaction_draft_send",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_send",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointRequire("shell_interaction_draft_send",shellInteractionAuthority(shellBeforeActionIds));send.performClick();ui.waitForIdle();ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {owner.actions[expectedHost].orEmpty().singleOrNull {it.metadata.actionId !in shellBeforeActionIds}?.let(::exactShellInput)==true};privateCheckpointDeadline("shell_interaction_draft_send",checkpointDeadline);draftInput=privateCheckpointActions(owner,expectedHost).singleOrNull {it.metadata.actionId !in shellBeforeActionIds};privateCheckpointRequire("shell_interaction_draft_send",draftInput?.let(::exactShellInput)==true)
                    val draftActionIds=shellBeforeActionIds+checkNotNull(draftInput).metadata.actionId
                    privateCheckpointStage="shell_interaction_live";val live=ui.onNodeWithTag("shell_input_live");live.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_interaction_live",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_live",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointRequire("shell_interaction_live",shellInteractionAuthority(draftActionIds));live.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_interaction_live",checkpointDeadline)
                    privateCheckpointStage="shell_interaction_commit_wait";ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {shellInteractionAuthority(draftActionIds)&&owner.shellInputAllowed(shellScopeState)};privateCheckpointDeadline("shell_interaction_commit_wait",checkpointDeadline);privateCheckpointStage="shell_interaction_commit_dialog";privateCheckpointTerminalDialogGuard("shell_interaction_commit_dialog",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_live",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointStage="shell_interaction_commit_authority";privateCheckpointRequire("shell_interaction_commit_authority",shellInteractionAuthority(draftActionIds)&&owner.shellInputAllowed(shellScopeState));val directOutcome=privateCheckpointDirectTerminalInput(shellViewer){shellInteractionAuthority(draftActionIds)&&owner.shellInputAllowed(shellScopeState)};privateCheckpointRequire("shell_interaction_commit_${directOutcome.closed}",directOutcome==PrivateCheckpointDirectInputOutcome.Success);privateCheckpointStage="shell_interaction_commit_receipt_wait";var directReceiptObservation=PrivateCheckpointShellReceiptObservation("wait",false);ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {directReceiptObservation=privateCheckpointShellInteractionReceiptObservation(owner.actions[expectedHost].orEmpty(),draftActionIds,pairedHello.bootId,shellTarget,shellOwner,owner.terminalInputPause,::exactShellInput);privateCheckpointStage="shell_interaction_commit_receipt_${directReceiptObservation.closed}";directReceiptObservation.exact};privateCheckpointDeadline("shell_interaction_commit_receipt_${directReceiptObservation.closed}",checkpointDeadline);privateCheckpointStage="shell_interaction_commit_receipt_exact";directInput=privateCheckpointActions(owner,expectedHost).singleOrNull {it.metadata.actionId !in draftActionIds};privateCheckpointRequire("shell_interaction_commit_receipt_exact",directInput?.let(::exactShellInput)==true)
                } finally {
                    try {
                        if(privateCheckpointOwnedClip(clipboard,nonce)) {
                            privateCheckpointStage="shell_interaction_clipboard_clear";ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {changes.get()==1&&privateCheckpointOwnedClip(clipboard,nonce)};privateCheckpointDeadline("shell_interaction_clipboard_clear",checkpointDeadline)
                            privateCheckpointTerminalDialogGuard("shell_interaction_clipboard_clear",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_input_paste",shellBaseToken,checkpointDeadline,Screen.Terminals)
                            InstrumentationRegistry.getInstrumentation().runOnMainSync {if(privateCheckpointOwnedClip(clipboard,nonce)&&changes.get()==1)clipboard.clearPrimaryClip()}
                            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {!clipboard.hasPrimaryClip()&&changes.get()==2};privateCheckpointDeadline("shell_interaction_clipboard_clear",checkpointDeadline)
                        }
                    } finally {InstrumentationRegistry.getInstrumentation().runOnMainSync {clipboard.removePrimaryClipChangedListener(listener)}}
                }
                val firstInput=checkNotNull(draftInput);val secondInput=checkNotNull(directInput);interactionInputs=listOf(firstInput,secondInput);interactionPreEnterIds=shellBeforeActionIds+firstInput.metadata.actionId+secondInput.metadata.actionId
            }
            if(shellInteractionCheckpoint)shellInteractionInputCheckpoint()
            val preEnterShell=if(shellInteractionCheckpoint) checkNotNull(owner.shell) else committedShell;val preEnterGrid=if(shellInteractionCheckpoint) checkNotNull(preEnterShell.grid) else committedGrid;val preEnterRevision=preEnterGrid.begin.revision
            val preEnterActionIds=if(shellInteractionCheckpoint) checkNotNull(interactionPreEnterIds) else shellBeforeActionIds
            val enter=ui.onNodeWithTag("shell_key_Enter")
            privateCheckpointStage="shell_enter"
            privateCheckpointDeadline("shell_enter",checkpointDeadline);privateCheckpointTerminalDialogGuard("shell_enter",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_key_Enter",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointRequire("shell_enter",shellCurrent() && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.connection.state.value.mutationReady[expectedHost]==true && preEnterShell.selection==committedShell.selection && preEnterShell.source==committedShell.source && preEnterShell.ticket==committedShell.ticket && preEnterShell.controlBegun && !preEnterShell.loading && !preEnterShell.error && owner.shell?.let {it.selection==preEnterShell.selection && it.source==preEnterShell.source && it.ticket==preEnterShell.ticket && it.grid===preEnterGrid && it.controlBegun && !it.loading && !it.error}==true && owner.terminalOwner(shellScopeState)===shellOwner && (if(shellInteractionCheckpoint) shellInteractionAuthority(preEnterActionIds)&&preEnterActionIds.size==shellBeforeActionIds.size+2&&shellBeforeActionIds.all {it in preEnterActionIds} else preEnterActionIds==shellBeforeActionIds));enter.assertIsDisplayed().assertIsEnabled();enter.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_enter",checkpointDeadline)
            privateCheckpointStage="shell_enter_receipt_count_wait"
            var shellEnterReceiptObservation=PrivateCheckpointShellReceiptObservation("wait",false)
            ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {val actions=owner.actions[expectedHost].orEmpty();shellEnterReceiptObservation=privateCheckpointShellInteractionReceiptObservation(actions,preEnterActionIds,pairedHello.bootId,shellTarget,shellOwner,owner.terminalInputPause,::exactShellInput);privateCheckpointStage="shell_enter_receipt_count_${if(shellEnterReceiptObservation.exact)"exact" else shellEnterReceiptObservation.closed}";shellEnterReceiptObservation.exact&&(actions.count {it.metadata.method==ActionMethod.ShellInput && it.metadata.shellTarget==shellTarget}==if(shellInteractionCheckpoint) 3 else 1)}
            val shellRevision=preEnterRevision
            privateCheckpointStage="shell_enter_receipt_grid_wait"
            var shellEnterGridLastObservation:String?=null
            try {
                ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {val actions=owner.actions[expectedHost].orEmpty();shellEnterReceiptObservation=privateCheckpointShellInteractionReceiptObservation(actions,preEnterActionIds,pairedHello.bootId,shellTarget,shellOwner,owner.terminalInputPause,::exactShellInput);privateCheckpointStage="shell_enter_receipt_grid_${if(shellEnterReceiptObservation.exact)"exact" else shellEnterReceiptObservation.closed}";if(!shellEnterReceiptObservation.exact) {shellEnterGridLastObservation=null;false} else {val currentShell=owner.shell;val currentGrid=currentShell?.grid;val currentRevision=currentGrid?.begin?.revision?.value?:Long.MIN_VALUE;shellEnterGridLastObservation=when {currentShell==null->"shell_missing";currentShell.selection!=preEnterShell.selection||currentShell.source!=preEnterShell.source||currentShell.ticket!=preEnterShell.ticket||!currentShell.controlBegun->"binding_changed";currentShell.loading->"loading";currentShell.error->"error";currentGrid==null->"grid_missing";currentRevision<shellRevision.value->"revision_regressed";currentRevision==shellRevision.value&&currentGrid===preEnterGrid->"same_revision_original_grid";currentRevision==shellRevision.value->"same_revision_replaced_grid";else->"advanced"};currentRevision>shellRevision.value}}
            } catch(error:Throwable) {shellEnterGridLastObservation?.let {privateCheckpointStage="shell_enter_receipt_grid_last_observation_$it"};throw error}
            privateCheckpointStage="shell_enter_receipt_deadline"
            privateCheckpointDeadline("shell_enter_receipt_deadline",checkpointDeadline)
            privateCheckpointStage="shell_enter_receipt_lookup"
            var afterEnterActions=privateCheckpointActions(owner,expectedHost);var afterEnterIds=afterEnterActions.map {it.metadata.actionId}.toSet()
            val enterInput=if(shellInteractionCheckpoint) afterEnterActions.singleOrNull {it.metadata.actionId !in preEnterActionIds} else null
            val inputActions=if(shellInteractionCheckpoint) checkNotNull(interactionInputs)+checkNotNull(enterInput) else afterEnterActions.filter {current->shellBeforeActions.none {it.metadata.actionId==current.metadata.actionId}}
            val shellInput=if(shellInteractionCheckpoint) checkNotNull(enterInput) else checkNotNull(inputActions.singleOrNull())
            privateCheckpointStage="shell_enter_receipt_exact"
            privateCheckpointRequire("shell_enter_receipt_exact",afterEnterIds.size==afterEnterActions.size && (if(shellInteractionCheckpoint) afterEnterIds==preEnterActionIds+shellInput.metadata.actionId&&inputActions.size==3&&inputActions.all(::exactShellInput) else inputActions.size==1&&exactShellInput(shellInput)))
            suspend fun shellInteractionHistoryCheckpoint() {
                privateCheckpointRequire("shell_interaction_inputs",inputActions.map {it.metadata.actionId}.toSet().size==3&&inputActions.all(::exactShellInput))
                var outputObservation="shell_interaction_output_missing_shell"
                ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {
                    val candidate=owner.shell
                    val candidateGrid=candidate?.grid
                    val candidateScroll=candidateGrid?.begin?.scroll
                    outputObservation=when {
                        candidate==null->"shell_interaction_output_missing_shell"
                        candidateGrid==null->"shell_interaction_output_missing_grid"
                        candidateScroll==null->"shell_interaction_output_missing_scroll"
                        candidate.selection!=committedShell.selection||candidate.source!=committedShell.source||candidate.ticket!=committedShell.ticket||!candidate.controlBegun||candidate.loading||candidate.error->"shell_interaction_output_binding"
                        candidateGrid.begin.revision.value<=shellRevision.value->"shell_interaction_output_revision"
                        candidateScroll.maxOffsetFromBottom.value<=0L->"shell_interaction_output_history_empty"
                        candidateScroll.viewportRows.value<=0L->"shell_interaction_output_viewport"
                        candidateScroll.maxOffsetFromBottom.value<candidateScroll.viewportRows.value->"shell_interaction_output_history_empty"
                        candidateScroll.offsetFromBottom.value!=0L->"shell_interaction_output_not_bottom"
                        owner.terminalOwner(shellScopeState)!==shellOwner->"shell_interaction_output_owner"
                        !owner.shellInputAllowed(shellScopeState)->"shell_interaction_output_not_accepting"
                        else->"shell_interaction_output"
                    }
                    privateCheckpointStage=outputObservation
                    outputObservation=="shell_interaction_output"
                }
                privateCheckpointStage="shell_interaction_output"
                val outputShell=checkNotNull(owner.shell);val outputGrid=checkNotNull(outputShell.grid);val outputScroll=checkNotNull(outputGrid.begin.scroll)
                privateCheckpointRequire("shell_interaction_output",outputShell.selection==committedShell.selection&&outputShell.source==committedShell.source&&outputShell.ticket==committedShell.ticket&&outputShell.controlBegun&&!outputShell.loading&&!outputShell.error&&outputGrid.begin.revision.value>shellRevision.value&&outputScroll.maxOffsetFromBottom.value>0L&&outputScroll.viewportRows.value>0L&&outputScroll.maxOffsetFromBottom.value>=outputScroll.viewportRows.value&&outputScroll.offsetFromBottom.value==0L&&owner.terminalOwner(shellScopeState)===shellOwner&&owner.shellInputAllowed(shellScopeState))
                fun olderReady():Boolean {val history=owner.shell?.history ?: return false;val metrics=history.metrics ?: return false;return shellInteractionAuthority(afterEnterIds)&&!owner.busy&&owner.shellHistoryAllowed(shellScopeState)&&history.mode==ShellHistoryMode.Live&&metrics.offsetFromBottom.value==0L&&metrics.viewportRows.value>0L&&metrics.maxOffsetFromBottom.value>=metrics.viewportRows.value}
                privateCheckpointStage="shell_interaction_older_ready";ui.waitUntil(minOf(10_000L,(checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L))) {olderReady()};privateCheckpointDeadline("shell_interaction_older_ready",checkpointDeadline)
                privateCheckpointStage="shell_interaction_older_reveal_admission";privateCheckpointRequire("shell_interaction_older_reveal_admission",olderReady());ui.onNodeWithTag("shell_essential_rail").assertIsDisplayed();privateCheckpointTerminalDialogGuard("shell_interaction_older_reveal_admission",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_essential_rail",shellBaseToken,checkpointDeadline,Screen.Terminals)
                privateCheckpointStage="shell_interaction_older_lookup";val older=ui.onNodeWithTag("shell_history_older");older.assertExists()
                privateCheckpointStage="shell_interaction_older_scroll_ready";privateCheckpointRequire("shell_interaction_older_scroll_ready",olderReady());privateCheckpointStage="shell_interaction_older_scroll_rail";ui.onNodeWithTag("shell_essential_rail").assertIsDisplayed();privateCheckpointStage="shell_interaction_older_scroll_dialog";privateCheckpointTerminalDialogGuard("shell_interaction_older_scroll_dialog",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_essential_rail",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointStage="shell_interaction_older_scroll_admission";privateCheckpointRequire("shell_interaction_older_scroll_admission",olderReady())
                privateCheckpointStage="shell_interaction_older_scroll";older.performScrollTo();ui.waitForIdle();privateCheckpointDeadline("shell_interaction_older_scroll",checkpointDeadline)
                privateCheckpointStage="shell_interaction_older_display";older.assertIsDisplayed();privateCheckpointStage="shell_interaction_older_enabled";older.assertIsEnabled();privateCheckpointStage="shell_interaction_older_dialog";privateCheckpointTerminalDialogGuard("shell_interaction_older_dialog",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_history_older",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointStage="shell_interaction_older_final_admission";older.assertIsDisplayed().assertIsEnabled();privateCheckpointRequire("shell_interaction_older_final_admission",olderReady());privateCheckpointStage="shell_interaction_older_authority";privateCheckpointRequire("shell_interaction_older_authority",shellInteractionAuthority(afterEnterIds));privateCheckpointStage="shell_interaction_older_click";val olderHistory=owner.shell?.history;val olderHistoryAction=olderHistory?.action;older.performClick();ui.waitForIdle();val requestedOlderViewport=outputScroll.viewportRows.value
                fun exactOlderReceipt(action:LocalAction?):Boolean=action?.let {candidate->candidate.authoritative&&candidate.metadata.method==ActionMethod.ShellScroll&&candidate.metadata.bootId==pairedHello.bootId&&candidate.metadata.target==null&&candidate.metadata.shellTarget==shellTarget&&candidate.receipt?.let {receipt->receipt.actionId==candidate.metadata.actionId&&receipt.method==ActionMethod.ShellScroll&&receipt.bootId==pairedHello.bootId&&receipt.target==null&&receipt.shellTarget==shellTarget&&receipt.error==null&&receipt.state==ReceiptState.Succeeded&&(receipt.result as? ActionResultShellScroll)?.let {it.offsetFromBottom.value>0L&&it.offsetFromBottom.value<=it.maxOffsetFromBottom.value&&it.maxOffsetFromBottom.value>=it.viewportRows.value&&it.viewportRows.value==requestedOlderViewport}==true}==true}==true&&owner.shell?.history?.mode==ShellHistoryMode.Browsing&&owner.terminalOwner(shellScopeState)===shellOwner&&!owner.shellInputAllowed(shellScopeState)
                fun olderReceiptObservation(actions:List<LocalAction>,candidate:LocalAction?):String {
                    if(actions.size!=shellBeforeActions.size+4) {
                        val currentIds=actions.map {it.metadata.actionId}
                        if(currentIds.toSet().size!=currentIds.size)return "count_duplicate_current_ids"
                        if(!afterEnterIds.all {it in currentIds})return "count_baseline_ids_missing"
                        val newIds=currentIds.filter {it !in afterEnterIds}.toSet()
                        if(newIds.size>1)return "count_multiple_new_ids"
                        if(newIds.isEmpty()) {
                            val currentHistory=owner.shell?.history
                            val busy=if(owner.busy)"busy" else "idle"
                            if(currentHistory?.action==olderHistoryAction)return "count_no_new_id_history_unchanged_$busy"
                            return if(currentHistory?.mode==ShellHistoryMode.Pending)"count_no_new_id_history_changed_pending_$busy" else "count_no_new_id_history_changed_non_pending_$busy"
                        }
                        return "count_inconsistent"
                    }
                    val action=candidate ?: return "selection"
                    if(!action.authoritative||action.metadata.method!=ActionMethod.ShellScroll||action.metadata.bootId!=pairedHello.bootId||action.metadata.target!=null||action.metadata.shellTarget!=shellTarget)return "metadata"
                    val receipt=action.receipt ?: return "receipt_absent"
                    if(receipt.state in setOf(ReceiptState.Reserved,ReceiptState.Dispatching,ReceiptState.Dispatched))return "pending"
                    if(receipt.state!=ReceiptState.Succeeded)return "non_success"
                    if(receipt.error!=null)return "error"
                    if(receipt.actionId!=action.metadata.actionId||receipt.method!=ActionMethod.ShellScroll||receipt.bootId!=pairedHello.bootId||receipt.target!=null||receipt.shellTarget!=shellTarget)return "receipt_identity"
                    val result=receipt.result as? ActionResultShellScroll ?: return "scrollresult"
                    if(result.offsetFromBottom.value<=0L||result.offsetFromBottom.value>result.maxOffsetFromBottom.value)return "offset"
                    if(result.maxOffsetFromBottom.value<result.viewportRows.value)return "max"
                    if(result.viewportRows.value!=outputScroll.viewportRows.value)return "rows"
                    val currentHistory=owner.shell?.history ?: return "history_absent"
                    if(currentHistory!==olderHistory)return "history_replaced"
                    val currentAction=currentHistory.action==action.metadata.actionId
                    if(currentHistory.mode==ShellHistoryMode.Pending)return if(currentAction) {if(owner.busy)"history_pending_action_busy" else if(owner.shellControlAllowed(shellScopeState))"history_pending_action_idle_admitted" else "history_pending_action_idle_unadmitted"} else "history_pending_other"
                    if(currentHistory.mode==ShellHistoryMode.Uncertain)return if(currentAction)"history_uncertain_action" else "history_uncertain_other"
                    if(currentHistory.mode==ShellHistoryMode.Live)return if(currentAction)"history_live_action" else "history_live_other"
                    if(owner.terminalOwner(shellScopeState)!==shellOwner)return "owner"
                    if(owner.shellInputAllowed(shellScopeState))return "input"
                    return "state_changed_or_unclassified"
                }
                var olderLastObservation="base"
                try {
                    privateCheckpointStage="shell_interaction_older_receipt_wait"
                    ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {
                        val actions=owner.actions[expectedHost].orEmpty()
                        val candidate=actions.singleOrNull {it.metadata.actionId !in afterEnterIds}
                        val exact=actions.size==shellBeforeActions.size+4&&exactOlderReceipt(candidate)
                        olderLastObservation=if(exact)"settled" else olderReceiptObservation(actions,candidate)
                        exact
                    }
                    privateCheckpointDeadline("shell_interaction_older_receipt_wait",checkpointDeadline)
                } catch(error:Throwable) {
                    privateCheckpointStage="shell_interaction_older_receipt_last_observation_${olderLastObservation}"
                    throw error
                }
                privateCheckpointStage="shell_interaction_older_receipt_exact";val olderAction=checkNotNull(privateCheckpointActions(owner,expectedHost).singleOrNull {it.metadata.actionId !in afterEnterIds})
                privateCheckpointRequire("shell_interaction_older_receipt_exact",exactOlderReceipt(olderAction))
                val afterOlderIds=afterEnterIds+olderAction.metadata.actionId
                fun bottomReady():Boolean {val history=owner.shell?.history ?: return false;return shellInteractionAuthority(afterOlderIds)&&!owner.busy&&owner.shellHistoryAllowed(shellScopeState)&&history.mode==ShellHistoryMode.Browsing&&history.metrics!=null}
                privateCheckpointStage="shell_interaction_bottom_ready";ui.waitUntil(minOf(10_000L,(checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L))) {bottomReady()};privateCheckpointDeadline("shell_interaction_bottom_ready",checkpointDeadline)
                privateCheckpointStage="shell_interaction_bottom_reveal_admission";privateCheckpointRequire("shell_interaction_bottom_reveal_admission",bottomReady());ui.onNodeWithTag("shell_essential_rail").assertIsDisplayed();privateCheckpointTerminalDialogGuard("shell_interaction_bottom_reveal_admission",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_essential_rail",shellBaseToken,checkpointDeadline,Screen.Terminals)
                privateCheckpointStage="shell_interaction_bottom_lookup";val bottom=ui.onNodeWithTag("shell_history_bottom");bottom.assertExists()
                privateCheckpointStage="shell_interaction_bottom_scroll_ready";privateCheckpointRequire("shell_interaction_bottom_scroll_ready",bottomReady());privateCheckpointStage="shell_interaction_bottom_scroll_rail";ui.onNodeWithTag("shell_essential_rail").assertIsDisplayed();privateCheckpointStage="shell_interaction_bottom_scroll_dialog";privateCheckpointTerminalDialogGuard("shell_interaction_bottom_scroll_dialog",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_essential_rail",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointStage="shell_interaction_bottom_scroll_admission"
                // Preserve bottomReady's first-refusal order while recording one local admission observation.
                val bottomScrollAdmission=owner.shell?.history?.let {history->when { !shellInteractionAuthority(afterOlderIds)->"authority" to false;owner.busy->"busy" to false;!owner.shellHistoryAllowed(shellScopeState)->"history_disallowed" to false;else->when(history.mode) {ShellHistoryMode.Live->"mode_live" to false;ShellHistoryMode.Pending->"mode_pending" to false;ShellHistoryMode.Uncertain->"mode_uncertain" to false;ShellHistoryMode.Browsing->if(history.metrics==null)"metrics_absent" to false else "ready" to true}}}?: ("history_absent" to false)
                privateCheckpointStage="shell_interaction_bottom_scroll_admission_${bottomScrollAdmission.first}";privateCheckpointRequire("shell_interaction_bottom_scroll_admission_${bottomScrollAdmission.first}",bottomScrollAdmission.second)
                privateCheckpointStage="shell_interaction_bottom_scroll";bottom.performScrollTo();ui.waitForIdle();privateCheckpointDeadline("shell_interaction_bottom_scroll",checkpointDeadline)
                privateCheckpointStage="shell_interaction_bottom_ready";ui.waitUntil(minOf(10_000L,(checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L))) {bottomReady()};privateCheckpointDeadline("shell_interaction_bottom_ready",checkpointDeadline)
                privateCheckpointStage="shell_interaction_bottom_display";bottom.assertIsDisplayed();privateCheckpointStage="shell_interaction_bottom_enabled";bottom.assertIsEnabled();privateCheckpointStage="shell_interaction_bottom_dialog";privateCheckpointTerminalDialogGuard("shell_interaction_bottom_dialog",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_history_bottom",shellBaseToken,checkpointDeadline,Screen.Terminals);privateCheckpointStage="shell_interaction_bottom_final_admission";bottom.assertIsDisplayed().assertIsEnabled();privateCheckpointRequire("shell_interaction_bottom_final_admission",bottomReady());privateCheckpointStage="shell_interaction_bottom_authority";privateCheckpointRequire("shell_interaction_bottom_authority",shellInteractionAuthority(afterOlderIds));privateCheckpointStage="shell_interaction_bottom_click";bottom.performClick();ui.waitForIdle()
                fun exactBottomReceipt(action:LocalAction?,actionIds:Set<Uuid>):Boolean=action?.let {candidate->actionIds==afterOlderIds+candidate.metadata.actionId&&candidate.authoritative&&candidate.metadata.bootId==pairedHello.bootId&&candidate.metadata.target==null&&candidate.metadata.shellTarget==shellTarget&&candidate.receipt?.let {receipt->receipt.actionId==candidate.metadata.actionId&&receipt.method==ActionMethod.ShellScroll&&receipt.bootId==pairedHello.bootId&&receipt.target==null&&receipt.shellTarget==shellTarget&&receipt.error==null&&receipt.state==ReceiptState.Succeeded&&(receipt.result as? ActionResultShellScroll)?.let {it.offsetFromBottom.value==0L&&it.maxOffsetFromBottom.value>=it.viewportRows.value&&it.viewportRows.value==outputScroll.viewportRows.value}==true}==true}==true&&owner.shell?.history?.mode==ShellHistoryMode.Live&&owner.terminalOwner(shellScopeState)===shellOwner&&owner.shellInputAllowed(shellScopeState)
                privateCheckpointStage="shell_interaction_bottom_receipt_wait";ui.waitUntil((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {val actions=owner.actions[expectedHost].orEmpty();val actionIds=actions.map {it.metadata.actionId}.toSet();actions.size==shellBeforeActions.size+5&&exactBottomReceipt(actions.singleOrNull {it.metadata.actionId !in shellBeforeActionIds&&it.metadata.method==ActionMethod.ShellScroll&&it.metadata.actionId!=olderAction.metadata.actionId},actionIds)};privateCheckpointDeadline("shell_interaction_bottom_receipt_wait",checkpointDeadline)
                privateCheckpointStage="shell_interaction_bottom_receipt_exact";afterEnterActions=privateCheckpointActions(owner,expectedHost);afterEnterIds=afterEnterActions.map {it.metadata.actionId}.toSet();val bottomAction=checkNotNull(afterEnterActions.singleOrNull {it.metadata.actionId !in shellBeforeActionIds&&it.metadata.method==ActionMethod.ShellScroll&&it.metadata.actionId!=olderAction.metadata.actionId})
                privateCheckpointRequire("shell_interaction_bottom_receipt_exact",exactBottomReceipt(bottomAction,afterEnterIds))
                privateCheckpointStage="shell_interaction_verify";privateCheckpointRequire("shell_interaction_verify",stockControl("verify_shell_interaction","target" to Protocol.strictJson.encodeToJsonElement(ShellTarget.serializer(),shellTarget),"inputs" to JsonArray(inputActions.map {Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),checkNotNull(it.receipt))}),"older" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),checkNotNull(olderAction.receipt)),"bottom" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),checkNotNull(bottomAction.receipt)),"draft_bytes" to JsonPrimitive(71),"start_revision" to JsonPrimitive(shellRevision.value),"output_revision" to JsonPrimitive(outputGrid.begin.revision.value),"columns" to JsonPrimitive(outputGrid.begin.columns.value),"rows" to JsonPrimitive(outputGrid.begin.rows.value),"max_offset" to JsonPrimitive(outputScroll.maxOffsetFromBottom.value),"viewport" to JsonPrimitive(outputScroll.viewportRows.value)).getValue("verified").jsonPrimitive.boolean)
            }
            if(shellInteractionCheckpoint)shellInteractionHistoryCheckpoint()
            val release=ui.onNodeWithTag("shell_release_control")
            privateCheckpointStage="shell_release_reveal_guard";privateCheckpointTerminalDialogGuard("shell_release_reveal_guard",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_release_control",shellBaseToken,checkpointDeadline,Screen.Terminals)
            privateCheckpointStage="shell_release_reveal_scroll";release.performScrollTo()
            privateCheckpointStage="shell_release_reveal_idle";ui.waitForIdle()
            privateCheckpointStage="shell_release_reveal_deadline";privateCheckpointDeadline("shell_release_reveal_deadline",checkpointDeadline)
            privateCheckpointStage="shell_release_reveal_display";release.assertIsDisplayed()
            privateCheckpointStage="shell_release_reveal_enabled";release.assertIsEnabled()
            privateCheckpointStage="shell_release_dialog";privateCheckpointTerminalDialogGuard("shell_release_dialog",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_release_control",shellBaseToken,checkpointDeadline,Screen.Terminals)
            val postEnterShell=checkNotNull(owner.shell);val postEnterGrid=checkNotNull(postEnterShell.grid)
            privateCheckpointRequire("shell_release_admission_shell",postEnterShell.selection==committedShell.selection && postEnterShell.source==committedShell.source && postEnterShell.ticket==committedShell.ticket && postEnterShell.controlBegun && !postEnterShell.loading && !postEnterShell.error && postEnterGrid!==committedGrid && postEnterGrid.begin.revision.value>shellRevision.value)
            privateCheckpointStage="shell_release_admission_ticket";check(committedShell.ticket<Long.MAX_VALUE)
            privateCheckpointStage="shell_release_admission_current";check(shellCurrent())
            privateCheckpointStage="shell_release_admission_hello";check(owner.connection.state.value.hello[expectedHost]===pairedHello)
            privateCheckpointStage="shell_release_admission_mutation";check(owner.connection.state.value.mutationReady[expectedHost]==true)
            privateCheckpointStage="shell_release_admission_shell";check(owner.shell?.let {it.selection==postEnterShell.selection && it.source==postEnterShell.source && it.ticket==postEnterShell.ticket && it.grid===postEnterGrid && it.controlBegun && !it.loading && !it.error}==true)
            privateCheckpointStage="shell_release_admission_owner";check(owner.terminalOwner(shellScopeState)===shellOwner)
            privateCheckpointStage="shell_release_admission_owner_retained";check(shellOwner.retained())
            privateCheckpointStage="shell_release_admission_actions";check(privateCheckpointActions(owner,expectedHost).map {it.metadata.actionId}.toSet()==afterEnterIds)
            privateCheckpointStage="shell_release_revalidate";release.assertIsDisplayed().assertIsEnabled()
            privateCheckpointStage="shell_release_click";release.performClick()
            privateCheckpointStage="shell_release_idle";ui.waitForIdle();privateCheckpointDeadline("shell_release_idle",checkpointDeadline)
            privateCheckpointStage="shell_release_posteffect_shell";check(owner.shell?.let {it.selection==ShellSelection(expectedHost,shellTarget) && it.source==committedShell.source && it.ticket==committedShell.ticket+1L && !it.controlBegun && it.grid==null && owner.terminalOwner(shellScopeState)==null && !owner.shellInputAllowed(shellScopeState)}==true)
            privateCheckpointStage="shell_release_posteffect_owner_retired";check(!shellOwner.retained())
            privateCheckpointStage="shell_release_posteffect_actions";check(privateCheckpointActions(owner,expectedHost).map {it.metadata.actionId}.toSet()==afterEnterIds)
            privateCheckpointStage="shell_close"
            val close=ui.onNodeWithTag("shell_viewer_back");close.assertIsDisplayed().assertIsEnabled();privateCheckpointTerminalDialogGuard("shell_close",runner,owner,epoch,shellViewer,"shell_viewer",shellScope,"shell_viewer_back",shellBaseToken,checkpointDeadline,Screen.Terminals);close.performClick();ui.waitForIdle();privateCheckpointDeadline("shell_close",checkpointDeadline)
            privateCheckpointRequire("shell_close",owner.shell==null && owner.screen==Screen.Terminals && owner.host==expectedHost && shellCurrent() && privateCheckpointActions(owner,expectedHost).map {it.metadata.actionId}.toSet()==afterEnterIds)
            privateCheckpointGuard("shell_close",runner,owner,epoch,Screen.Terminals,null,false)
            privateCheckpointStage="shell_complete"
            privateCheckpointRequire("shell_complete",stockControl("complete").getValue("complete").jsonPrimitive.boolean)
        } else
        requestedCreationProvider?.let {expectedProvider->
            val expectedServer=info.getValue("serverId").jsonPrimitive.content
            val expectedProject=info.getValue("projectId").jsonPrimitive.content
            val expectedServerName=info.getValue("serverName").jsonPrimitive.content
            val expectedProjectLabel=info.getValue("projectLabel").jsonPrimitive.content
            val finishRemaining=finish.getValue("creationTimeoutMs").jsonPrimitive.long
            val creationDeadline=minOf(checkpointDeadline,android.os.SystemClock.elapsedRealtime()+finishRemaining)
            val inventory=owner.connection.state.value.inventory[expectedHost]
            val sourceAdmitted=inventory?.servers?.singleOrNull {it.serverId.value==expectedServer}?.let {it.name.value==expectedServerName && it.connectivity==ServerConnectivity.Online && (if(expectedProvider==Provider.Omp) it.canCreate.omp else it.canCreate.codex)}==true && inventory.projects.singleOrNull {it.projectId.value==expectedProject}?.let {it.label.value==expectedProjectLabel && it.available}==true
            privateCheckpointStage="creation_source_admission"
            privateCheckpointRequire("creation_source_admission",finishRemaining in 1L..120_000L && sourceAdmitted && owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.connection.state.value.inventoryReady[expectedHost]==true && owner.connection.state.value.mutationReady[expectedHost]==true && owner.creationActions[expectedHost]==null)
            privateCheckpointDeadline("creation_source_admission",creationDeadline)
            privateCheckpointStage="creation_open_new_session"
            privateCheckpointTap("creation_open_new_session","new_session",runner,owner,epoch,Screen.Sessions,null,false,deadline=creationDeadline)
            val chooser=ui.onNodeWithTag("landing_new_session_host_"+expectedHost)
            privateCheckpointStage="creation_host_chooser"
            if(runCatching {chooser.assertIsDisplayed()}.isSuccess) {
                val chooserTag="landing_new_session_host_"+expectedHost
                val chooserWindow=privateCheckpointModalWindow("creation_host_chooser",runner,owner,epoch,Screen.Sessions,chooserTag,creationDeadline)
                privateCheckpointRequire("creation_host_chooser",chooserWindow!=null)
                privateCheckpointModalTap("creation_host_chooser",chooserTag,runner,owner,epoch,Screen.Sessions,chooserWindow!!,creationDeadline)
            }
            val formStaticTag="new_session_close"
            privateCheckpointStage="creation_form_opening"
            privateCheckpointDeadline("creation_form_opening",creationDeadline)
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(formStaticTag).assertIsDisplayed()}.isSuccess}
            privateCheckpointDeadline("creation_form_opening",creationDeadline)
            privateCheckpointRequire("creation_form_opening",owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello)
            val formWindow=privateCheckpointModalWindow("creation_form_opening",runner,owner,epoch,Screen.Sessions,formStaticTag,creationDeadline)
            privateCheckpointRequire("creation_form_opening",formWindow!=null)
            privateCheckpointModalGuard("creation_form_opening",runner,owner,epoch,Screen.Sessions,formWindow!!,formStaticTag,creationDeadline)
            privateCheckpointStage="creation_server_picker"
            privateCheckpointModalTap("creation_server_picker","create_server",runner,owner,epoch,Screen.Sessions,formWindow,creationDeadline,scopeTag=formStaticTag)
            val serverOptionTag="creation_server_option_id_"+expectedServer
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(serverOptionTag,useUnmergedTree=true).assertIsDisplayed()}.isSuccess}
            val serverPickerWindow=privateCheckpointModalWindow("creation_server_picker",runner,owner,epoch,Screen.Sessions,serverOptionTag,creationDeadline)
            privateCheckpointRequire("creation_server_picker",serverPickerWindow!=null)
            privateCheckpointPickerTap("creation_server_picker",serverOptionTag,runner,owner,epoch,serverPickerWindow!!,creationDeadline)
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(formStaticTag).assertIsDisplayed()}.isSuccess}
            privateCheckpointModalGuard("creation_server_picker",runner,owner,epoch,Screen.Sessions,formWindow,formStaticTag,creationDeadline)
            privateCheckpointStage="creation_project_picker"
            privateCheckpointModalTap("creation_project_picker","create_project",runner,owner,epoch,Screen.Sessions,formWindow,creationDeadline,scopeTag=formStaticTag)
            val projectOptionTag="creation_project_option_id_"+expectedProject
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(projectOptionTag,useUnmergedTree=true).assertIsDisplayed()}.isSuccess}
            val projectPickerWindow=privateCheckpointModalWindow("creation_project_picker",runner,owner,epoch,Screen.Sessions,projectOptionTag,creationDeadline)
            privateCheckpointRequire("creation_project_picker",projectPickerWindow!=null)
            privateCheckpointPickerTap("creation_project_picker",projectOptionTag,runner,owner,epoch,projectPickerWindow!!,creationDeadline)
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(formStaticTag).assertIsDisplayed()}.isSuccess}
            privateCheckpointModalGuard("creation_project_picker",runner,owner,epoch,Screen.Sessions,formWindow,formStaticTag,creationDeadline)
            val formSource="creation_form_source_"+expectedHost+"_"+pairedHello.bootId.value+"_"+expectedServer+"_"+expectedProject
            privateCheckpointStage="creation_exact_form_source"
            privateCheckpointDeadline("creation_exact_form_source",creationDeadline)
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {ui.onNodeWithTag(formSource).assertIsDisplayed()}.isSuccess}
            privateCheckpointDeadline("creation_exact_form_source",creationDeadline)
            val exactFormWindow=privateCheckpointModalWindow("creation_exact_form_source",runner,owner,epoch,Screen.Sessions,formSource,creationDeadline)
            privateCheckpointRequire("creation_exact_form_source",exactFormWindow==formWindow)
            privateCheckpointModalGuard("creation_exact_form_source",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline)
            privateCheckpointStage="creation_selected_fields"
            ui.onNodeWithTag(formSource).assertIsDisplayed()
            ui.onNode(hasTestTag("creation_server_selected_id_"+expectedServer) and hasText(expectedServerName,substring=true),useUnmergedTree=true).assertIsDisplayed()
            ui.onNode(hasTestTag("creation_project_selected_id_"+expectedProject) and hasText(expectedProjectLabel,substring=true),useUnmergedTree=true).assertIsDisplayed()
            val provider=ui.onNode(hasTestTag("create_provider") and hasText(providerLabel(expectedProvider),substring=true))
            privateCheckpointModalGuard("creation_selected_fields",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline)
            privateCheckpointStage="creation_provider"
            provider.performScrollTo();privateCheckpointDeadline("creation_provider",creationDeadline);privateCheckpointModalGuard("creation_provider",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline);provider.assertIsDisplayed().assertIsEnabled().performClick();ui.waitForIdle();privateCheckpointDeadline("creation_provider",creationDeadline)
            privateCheckpointModalGuard("creation_provider",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline)
            val name="Retained "+providerLabel(expectedProvider)+" "+java.util.UUID.randomUUID()
            privateCheckpointStage="creation_name"
            val nameField=ui.onNodeWithTag("create_name");nameField.performScrollTo();privateCheckpointDeadline("creation_name",creationDeadline);privateCheckpointModalGuard("creation_name",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline);nameField.assertIsDisplayed().assertIsEnabled().performTextReplacement(name);privateCheckpointDeadline("creation_name",creationDeadline)
            privateCheckpointModalGuard("creation_name",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline)
            privateCheckpointStage="creation_enabled_wait"
            privateCheckpointDeadline("creation_enabled_wait",creationDeadline)
            ui.waitUntil(minOf(10_000L,creationDeadline-android.os.SystemClock.elapsedRealtime())) {runCatching {privateCheckpointModalGuard("creation_enabled_wait",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline);ui.onNodeWithTag("session_create").assertIsDisplayed().assertIsEnabled();true}.getOrDefault(false)}
            val submit=ui.onNodeWithTag("session_create");privateCheckpointDeadline("creation_enabled_wait",creationDeadline);privateCheckpointModalGuard("creation_enabled_wait",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline);submit.assertIsDisplayed().assertIsEnabled()
            privateCheckpointStage="creation_final_submit_admission"
            val beforeActions=privateCheckpointActions(owner,expectedHost)
            val beforeActionIds=beforeActions.map {it.metadata.actionId}.toSet()
            val currentInventory=owner.connection.state.value.inventory[expectedHost]
            val currentSourceAdmitted=currentInventory?.let {current->
                current.servers.singleOrNull {it.serverId.value==expectedServer}?.let {it.name.value==expectedServerName && it.connectivity==ServerConnectivity.Online && (if(expectedProvider==Provider.Omp) it.canCreate.omp else it.canCreate.codex)}==true &&
                    current.projects.singleOrNull {it.projectId.value==expectedProject}?.let {it.label.value==expectedProjectLabel && it.available}==true
            }==true
            privateCheckpointRequire("creation_final_submit_admission",currentSourceAdmitted && beforeActionIds.size==beforeActions.size && owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.connection.state.value.inventoryReady[expectedHost]==true && owner.connection.state.value.mutationReady[expectedHost]==true && owner.creationActions[expectedHost]==null)
            privateCheckpointModalGuard("creation_final_submit_admission",runner,owner,epoch,Screen.Sessions,formWindow,formSource,creationDeadline)
            privateCheckpointStage="creation_click"
            submit.performClick()
            privateCheckpointStage="creation_idle"
            ui.waitForIdle();privateCheckpointDeadline("creation_idle",creationDeadline)
            privateCheckpointStage="creation_receipt"
            // Use the remaining signed checkpoint budget to observe the original creation.
            val settleDeadline=creationDeadline
            lateinit var action:Uuid
            var local:LocalAction?=null
            privateCheckpointDeadline("creation_receipt",settleDeadline)
            withTimeout(settleDeadline-android.os.SystemClock.elapsedRealtime()) {while(true) {
                privateCheckpointDeadline("creation_receipt",settleDeadline)
                val fresh=privateCheckpointActions(owner,expectedHost).filter {it.metadata.actionId !in beforeActionIds && it.metadata.method==ActionMethod.SessionCreate}
                privateCheckpointRequire("creation_receipt",fresh.size<=1)
                val candidate=fresh.singleOrNull()
                if(candidate!=null && (owner.creationActions[expectedHost]==null||owner.creationActions[expectedHost]==candidate.metadata.actionId)) {
                    action=candidate.metadata.actionId;local=owner.store.localAction(expectedHost,action)
                    if(local?.authoritative==true&&local?.receipt?.state !in listOf(ReceiptState.Reserved,ReceiptState.Dispatching)) break
                }
                delay(25)
            }}
            val receipt=local!!.receipt!!
            val creation=receipt.creation
            val post=privateCheckpointActions(owner,expectedHost).filter {it.metadata.actionId !in beforeActionIds}
            privateCheckpointRequire("creation_exact",post.size==1 && post.single().metadata.method==ActionMethod.SessionCreate && post.single().metadata.actionId==action && post.single().metadata.bootId==pairedHello.bootId && post.single().metadata.target==null && post.single().metadata.sourceTarget==null && post.single().metadata.shellTarget==null && owner.creationActions[expectedHost]==action && receipt.actionId==action && receipt.bootId==pairedHello.bootId && receipt.method==ActionMethod.SessionCreate && receipt.state==ReceiptState.Succeeded && receipt.error==null && receipt.target==null && receipt.shellTarget==null && creation?.let {it.provider==expectedProvider && it.serverId.value==expectedServer && it.projectId.value==expectedProject && it.resources.sessionId!=null && it.resources.generation!=null}==true)
            privateCheckpointStage="creation_verify"
            privateCheckpointDeadline("creation_verify",creationDeadline)
            val verified=stockControl("verify_create","receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt))
            privateCheckpointRequire("creation_verify",verified.getValue("verified").jsonPrimitive.boolean)
            val createdTarget=Target(checkNotNull(creation?.resources?.sessionId),checkNotNull(creation?.resources?.generation))
            val chatMarker=info.getValue("chatMarker").jsonPrimitive.content
            privateCheckpointCloseCreationGuard("chat_close_initial",runner,owner,epoch,formWindow,formStaticTag,checkpointDeadline)
            privateCheckpointStage="chat_close_node_query"
            val close=ui.onNodeWithTag(formStaticTag)
            privateCheckpointStage="chat_close_initial_visibility"
            if(runCatching {close.assertIsDisplayed()}.isFailure) {
                privateCheckpointStage="chat_close_scroll"
                close.performScrollTo()
            }
            privateCheckpointStage="chat_close_enabled"
            privateCheckpointDeadline("chat_close_enabled",checkpointDeadline)
            close.assertIsDisplayed().assertIsEnabled()
            privateCheckpointStage="chat_close_final_creation_admission"
            val currentCreate=owner.store.localAction(expectedHost,action)
            privateCheckpointDeadline("chat_close_final_creation_admission",checkpointDeadline)
            privateCheckpointRequire("chat_close_final_creation_admission",owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.creationActions[expectedHost]==action && currentCreate?.authoritative==true && currentCreate.receipt==receipt && currentCreate.receipt?.creation?.resources?.let {it.sessionId==createdTarget.sessionId && it.generation==createdTarget.generation}==true)
            privateCheckpointCloseCreationGuard("chat_close_final",runner,owner,epoch,formWindow,formStaticTag,checkpointDeadline)
            privateCheckpointStage="chat_close_click"
            close.performClick()
            privateCheckpointStage="chat_close_idle"
            ui.waitForIdle()
            privateCheckpointStage="chat_close_postclick"
            privateCheckpointDeadline("chat_close_postclick",checkpointDeadline)
            privateCheckpointStage="chat_close_posteffect_scene"
            privateCheckpointRequire("chat_close_posteffect_scene",owner.screen==Screen.Sessions && owner.host==expectedHost && owner.selection==null)
            privateCheckpointGuard("chat_close_posteffect_base_window",runner,owner,epoch,Screen.Sessions,null,false)
            privateCheckpointStage="chat_close_posteffect_modal_absent"
            ui.onNodeWithTag(formStaticTag).assertDoesNotExist()
            privateCheckpointStage="chat_close_posteffect_actions"
            val afterCloseActions=privateCheckpointActions(owner,expectedHost)
            privateCheckpointDeadline("chat_close_posteffect_actions",checkpointDeadline)
            privateCheckpointRequire("chat_close_posteffect_actions",post.size==1 && afterCloseActions.size==beforeActions.size+1 && afterCloseActions.map {it.metadata.actionId}.toSet()==beforeActionIds+action && afterCloseActions.singleOrNull {it.metadata.actionId==action}?.receipt==receipt)
            val sessionTag="session_${createdTarget.sessionId.value}_${createdTarget.generation.value}"
            fun preOpenCreatedReady()=owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.connection.state.value.inventoryReady[expectedHost]==true && owner.connection.state.value.mutationReady[expectedHost]==true && owner.connection.state.value.inventory[expectedHost]?.sessions?.singleOrNull {summary->Target(summary.sessionId,summary.generation)==createdTarget}?.let {it.provider==expectedProvider && it.serverId.value==expectedServer && it.connectivity==SessionConnectivity.Ready}==true
            fun openedCreatedReady()=owner.host==expectedHost && owner.connection.state.value.hello[expectedHost]===pairedHello && owner.connection.state.value.inventoryReady[expectedHost]==true && owner.connection.state.value.mutationReady[expectedHost]==true && owner.connection.state.value.inventory[expectedHost]?.sessions?.singleOrNull {summary->Target(summary.sessionId,summary.generation)==createdTarget}?.let {it.provider==expectedProvider && it.serverId.value==expectedServer && it.connectivity==SessionConnectivity.Ready}==true && owner.screen==Screen.Conversation && owner.selection==ConversationKey(expectedHost,createdTarget) && owner.connection.state.value.sources[expectedHost]?.containsKey(createdTarget)==true && stockReducer(owner,expectedHost,createdTarget)?.let {reducer->reducer.mutationsEnabled && reducer.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==createdTarget && summary.provider==expectedProvider && summary.serverId.value==expectedServer && summary.connectivity==SessionConnectivity.Ready}==true}==true && !owner.busy
            privateCheckpointStage="chat_session_admission"
            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {preOpenCreatedReady()}
            privateCheckpointDeadline("chat_session_admission",checkpointDeadline)
            privateCheckpointRequire("chat_session_admission",preOpenCreatedReady())
            privateCheckpointGuard("chat_session_admission",runner,owner,epoch,Screen.Sessions,null,false)
            val session=ui.onNodeWithTag(sessionTag)
            if(runCatching {session.assertIsDisplayed()}.isFailure) {
                privateCheckpointStage="chat_session_scroll"
                privateCheckpointGuard("chat_session_scroll",runner,owner,epoch,Screen.Sessions,null,false)
                session.performScrollTo()
            }
            privateCheckpointStage="chat_session_click"
            privateCheckpointDeadline("chat_session_click",checkpointDeadline);privateCheckpointGuard("chat_session_click",runner,owner,epoch,Screen.Sessions,null,false);session.assertIsDisplayed().assertIsEnabled();privateCheckpointDeadline("chat_session_click",checkpointDeadline);privateCheckpointRequire("chat_session_admission",preOpenCreatedReady());privateCheckpointGuard("chat_session_click",runner,owner,epoch,Screen.Sessions,null,false);session.performClick();ui.waitForIdle();privateCheckpointDeadline("chat_session_click",checkpointDeadline)
            privateCheckpointStage="chat_session_wait"
            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {openedCreatedReady()}
            privateCheckpointDeadline("chat_session_wait",checkpointDeadline)
            privateCheckpointRequire("chat_session_wait",openedCreatedReady())
            val conversationTag="conversation_target_${createdTarget.sessionId.value}_${createdTarget.generation.value}"
            privateCheckpointGuard("chat_prompt_admission",runner,owner,epoch,Screen.Conversation,null,false)
            ui.onNodeWithTag(conversationTag).assertIsDisplayed()
            val beforePromptActions=privateCheckpointActions(owner,expectedHost);val beforePromptIds=beforePromptActions.map {it.metadata.actionId}.toSet()
            fun promptBaselineCurrent(currentActions:List<LocalAction>)=beforeActionIds.size==beforeActions.size && beforePromptIds.size==beforePromptActions.size && post.size==1 && post.single().metadata.actionId==action && beforePromptActions.size==beforeActions.size+1 && beforePromptIds==beforeActionIds+action && beforePromptActions.singleOrNull {it.metadata.actionId==action}?.receipt==receipt && currentActions.map {it.metadata.actionId}.toSet().let {currentIds->currentIds.size==currentActions.size && currentIds==beforePromptIds} && currentActions.singleOrNull {it.metadata.actionId==action}?.receipt==receipt
            val prompt="Reply only $chatMarker in this exact session."
            privateCheckpointStage="chat_prompt_text"
            val composer=ui.onNodeWithTag("composer");composer.assertIsDisplayed().assertIsEnabled();privateCheckpointDeadline("chat_prompt_admission",checkpointDeadline);privateCheckpointRequire("chat_prompt_admission",openedCreatedReady()&&promptBaselineCurrent(beforePromptActions));privateCheckpointGuard("chat_prompt_text",runner,owner,epoch,Screen.Conversation,null,false);composer.performTextReplacement(prompt);privateCheckpointDeadline("chat_prompt_text",checkpointDeadline)
            privateCheckpointStage="chat_prompt_reservation"
            ui.waitUntil(minOf(10_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {owner.reservation?.let {it.key==ConversationKey(expectedHost,createdTarget)&&it.text==prompt}==true&&runCatching {ui.onNodeWithTag("prompt_send").assertIsDisplayed().assertIsEnabled()}.isSuccess}
            privateCheckpointDeadline("chat_prompt_reservation",checkpointDeadline)
            val promptAction=checkNotNull(owner.reservation?.takeIf {it.key==ConversationKey(expectedHost,createdTarget)&&it.text==prompt}?.action)
            val currentPromptActions=privateCheckpointActions(owner,expectedHost)
            privateCheckpointDeadline("chat_prompt_admission",checkpointDeadline)
            privateCheckpointRequire("chat_prompt_admission",openedCreatedReady()&&promptBaselineCurrent(currentPromptActions))
            val send=ui.onNodeWithTag("prompt_send")
            privateCheckpointStage="chat_prompt_click"
            privateCheckpointDeadline("chat_prompt_click",checkpointDeadline);privateCheckpointGuard("chat_prompt_click",runner,owner,epoch,Screen.Conversation,null,false);send.assertIsDisplayed().assertIsEnabled();privateCheckpointGuard("chat_prompt_click",runner,owner,epoch,Screen.Conversation,null,false);send.performClick();ui.waitForIdle();privateCheckpointDeadline("chat_prompt_click",checkpointDeadline)
            privateCheckpointStage="chat_prompt_receipt"
            lateinit var promptLocal:LocalAction
            withTimeout((checkpointDeadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {while(true) {
                privateCheckpointDeadline("chat_prompt_receipt",checkpointDeadline)
                val after=privateCheckpointActions(owner,expectedHost).filter {it.metadata.actionId !in beforePromptIds}
                privateCheckpointRequire("chat_prompt_receipt",after.size<=1)
                val candidate=after.singleOrNull()
                if(candidate?.metadata?.actionId==promptAction&&candidate.metadata.method==ActionMethod.PromptSubmit&&candidate.authoritative&&candidate.receipt?.state in listOf(ReceiptState.Dispatched,ReceiptState.Succeeded)) {promptLocal=candidate;break}
                delay(25)
            }}
            val promptReceipt=checkNotNull(promptLocal.receipt)
            privateCheckpointRequire("chat_prompt_exact",promptReceipt.actionId==promptAction&&promptReceipt.method==ActionMethod.PromptSubmit&&promptReceipt.bootId==pairedHello.bootId&&promptReceipt.target==createdTarget&&promptReceipt.shellTarget==null&&promptReceipt.creation==null&&promptReceipt.error==null&&promptReceipt.state in listOf(ReceiptState.Dispatched,ReceiptState.Succeeded)&&promptLocal.metadata.bootId==pairedHello.bootId&&promptLocal.metadata.target==createdTarget&&promptLocal.metadata.sourceTarget==null&&promptLocal.metadata.shellTarget==null)
            privateCheckpointStage="chat_history"
            lateinit var userRecord:HistoryRecord;lateinit var assistantRecord:HistoryRecord
            ui.waitUntil(minOf(60_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {
                val records=stockReducer(owner,expectedHost,createdTarget)?.history.orEmpty()
                val users=records.filter {it.role==Role.User&&it.complete&&(it.text as? ContentText)?.text?.value?.contains(chatMarker)==true}
                val assistants=records.filter {it.role==Role.Assistant&&it.complete&&(it.text as? ContentText)?.text?.value?.contains(chatMarker)==true}
                if(users.size==1&&assistants.size==1) {userRecord=users.single();assistantRecord=assistants.single();true}else false
            }
            privateCheckpointDeadline("chat_history",checkpointDeadline);privateCheckpointGuard("chat_history",runner,owner,epoch,Screen.Conversation,null,false)
            val userNode=ui.onNodeWithTag("history_record_${userRecord.id.value}");if(runCatching {userNode.assertIsDisplayed()}.isFailure) {privateCheckpointGuard("chat_history",runner,owner,epoch,Screen.Conversation,null,false);userNode.performScrollTo()};userNode.assertIsDisplayed()
            val assistantNode=ui.onNodeWithTag("history_record_${assistantRecord.id.value}");if(runCatching {assistantNode.assertIsDisplayed()}.isFailure) {privateCheckpointGuard("chat_history",runner,owner,epoch,Screen.Conversation,null,false);assistantNode.performScrollTo()};assistantNode.assertIsDisplayed()
            privateCheckpointStage="chat_observe"
            privateCheckpointDeadline("chat_observe",checkpointDeadline);privateCheckpointGuard("chat_observe",runner,owner,epoch,Screen.Conversation,null,false)
            val observed=stockControl("observe","target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),createdTarget),"marker" to JsonPrimitive(chatMarker),"receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),promptReceipt))
            privateCheckpointRequire("chat_observe",observed.getValue("verified").jsonPrimitive.boolean)
            val terminalKey=ConversationKey(expectedHost,createdTarget)
            val terminalSource=checkNotNull(owner.connection.state.value.sources[expectedHost]?.get(createdTarget))
            val terminalActions=privateCheckpointActions(owner,expectedHost)
            val terminalActionMetadata=terminalActions.associate {it.metadata.actionId to it.metadata}
            fun terminalActionMapCurrent(current:List<LocalAction>)=terminalActionMetadata.size==terminalActions.size && current.associate {it.metadata.actionId to it.metadata}.let {metadata->metadata.size==current.size && metadata==terminalActionMetadata}
            fun terminalCreationCurrent(current:List<LocalAction>)=current.singleOrNull {it.metadata.actionId==action}?.let {latest->latest.authoritative && latest.metadata.method==ActionMethod.SessionCreate && latest.metadata.bootId==pairedHello.bootId && latest.metadata.target==null && latest.metadata.sourceTarget==null && latest.metadata.shellTarget==null && latest.receipt==receipt}==true
            fun terminalPromptCurrent(current:List<LocalAction>)=current.singleOrNull {it.metadata.actionId==promptAction}?.let {latest->latest.authoritative && latest.metadata.method==ActionMethod.PromptSubmit && latest.metadata.bootId==pairedHello.bootId && latest.metadata.target==createdTarget && latest.metadata.sourceTarget==null && latest.metadata.shellTarget==null && latest.receipt?.let {receipt->receipt.actionId==promptAction && receipt.method==ActionMethod.PromptSubmit && receipt.bootId==pairedHello.bootId && receipt.target==createdTarget && receipt.shellTarget==null && receipt.creation==null && receipt.error==null && receipt.state in listOf(ReceiptState.Dispatched,ReceiptState.Succeeded)}==true}==true
            fun terminalInMemoryAuthority()=openedCreatedReady() && owner.connection.state.value.sources[expectedHost]?.get(createdTarget)==terminalSource && owner.readOnlyAgentTerminal(terminalKey)
            fun terminalStoreCurrent(current:List<LocalAction>)=terminalActionMapCurrent(current) && terminalCreationCurrent(current) && terminalPromptCurrent(current)
            privateCheckpointStage="terminal_admission"
            privateCheckpointDeadline("terminal_admission",checkpointDeadline)
            privateCheckpointRequire("terminal_admission",terminalInMemoryAuthority()&&terminalStoreCurrent(terminalActions))
            val terminalOpen=ui.onNodeWithTag("terminal_open")
            privateCheckpointStage="terminal_open"
            privateCheckpointDeadline("terminal_open",checkpointDeadline);privateCheckpointRequire("terminal_open",terminalInMemoryAuthority()&&terminalStoreCurrent(terminalActions));privateCheckpointGuard("terminal_open",runner,owner,epoch,Screen.Conversation,null,false);val terminalBaseApplicationToken=checkNotNull(privateCheckpointTerminalBaseApplicationToken(runner,owner,epoch));terminalOpen.assertIsDisplayed().assertIsEnabled();privateCheckpointDeadline("terminal_open",checkpointDeadline);val terminalOpenActions=privateCheckpointActions(owner,expectedHost);privateCheckpointRequire("terminal_open",terminalInMemoryAuthority()&&terminalStoreCurrent(terminalOpenActions));privateCheckpointGuard("terminal_open",runner,owner,epoch,Screen.Conversation,null,false);privateCheckpointRequire("terminal_open",privateCheckpointTerminalBaseApplicationToken(runner,owner,epoch)==terminalBaseApplicationToken);terminalOpen.performClick();ui.waitForIdle();privateCheckpointDeadline("terminal_open",checkpointDeadline)
            val terminalDialogTag="terminal_dialog"
            val terminalTargetTag="terminal_target_"+createdTarget.sessionId.value+"_"+createdTarget.generation.value
            privateCheckpointStage="terminal_popup_wait"
            ui.waitUntil(minOf(20_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {
                val shown=owner.terminal
                shown?.key==terminalKey && shown.source==terminalSource && terminalInMemoryAuthority() && runCatching {ui.onNodeWithTag(terminalDialogTag).assertIsDisplayed();ui.onNodeWithTag(terminalTargetTag).assertIsDisplayed()}.isSuccess
            }
            privateCheckpointDeadline("terminal_popup_store",checkpointDeadline)
            val terminalPopupActions=privateCheckpointActions(owner,expectedHost)
            val terminalWindow=privateCheckpointTerminalDialogWindow("terminal_popup",runner,owner,epoch,terminalDialogTag,terminalTargetTag,terminalDialogTag,terminalBaseApplicationToken,checkpointDeadline)
            checkNotNull(terminalWindow)
            val terminalSnapshot=checkNotNull(owner.terminal)
            privateCheckpointRequire("terminal_popup_store",terminalSnapshot.key==terminalKey && terminalSnapshot.source==terminalSource && terminalInMemoryAuthority()&&terminalStoreCurrent(terminalPopupActions))
            privateCheckpointTerminalDialogGuard("terminal_popup",runner,owner,epoch,terminalWindow!!,terminalDialogTag,terminalTargetTag,terminalTargetTag,terminalBaseApplicationToken,checkpointDeadline)
            privateCheckpointStage="terminal_grid"
            ui.waitUntil(minOf(20_000L,checkpointDeadline-android.os.SystemClock.elapsedRealtime())) {
                val shown=owner.terminal
                val grid=shown?.grid
                shown?.key==terminalKey && shown.source==terminalSource && shown.ticket==terminalSnapshot.ticket && !shown.loading && !shown.error && grid?.let {frame->runCatching {frame.begin.validate()}.isSuccess && frame.begin.columns.value>0L && frame.begin.rows.value>0L && frame.cells.size.toLong()==frame.begin.columns.value*frame.begin.rows.value}==true && terminalInMemoryAuthority() && runCatching {ui.onNodeWithTag("terminal_grid").assertIsDisplayed()}.isSuccess
            }
            privateCheckpointDeadline("terminal_grid",checkpointDeadline)
            val terminalGridActions=privateCheckpointActions(owner,expectedHost)
            val committedTerminal=checkNotNull(owner.terminal)
            val committedGrid=checkNotNull(committedTerminal.grid)
            committedGrid.begin.validate()
            privateCheckpointRequire("terminal_grid",committedTerminal.key==terminalKey && committedTerminal.source==terminalSource && committedTerminal.ticket==terminalSnapshot.ticket && !committedTerminal.loading && !committedTerminal.error && committedGrid.begin.columns.value>0L && committedGrid.begin.rows.value>0L && committedGrid.cells.size.toLong()==committedGrid.begin.columns.value*committedGrid.begin.rows.value && terminalInMemoryAuthority()&&terminalStoreCurrent(terminalGridActions))
            privateCheckpointTerminalDialogGuard("terminal_grid",runner,owner,epoch,terminalWindow,terminalDialogTag,terminalTargetTag,"terminal_grid",terminalBaseApplicationToken,checkpointDeadline)
            val terminalGrid=ui.onNodeWithTag("terminal_grid");terminalGrid.assertIsDisplayed();val terminalBounds=terminalGrid.fetchSemanticsNode().boundsInRoot
            privateCheckpointRequire("terminal_grid",terminalBounds.width>0f&&terminalBounds.height>0f)
            ui.onNodeWithTag("terminal_selected").assertIsDisplayed().assertIsNotEnabled()
            val terminalCurrentActions=privateCheckpointActions(owner,expectedHost)
            privateCheckpointDeadline("terminal_grid",checkpointDeadline)
            privateCheckpointRequire("terminal_grid",terminalInMemoryAuthority()&&terminalStoreCurrent(terminalCurrentActions))
            privateCheckpointStage="terminal_close"
            val terminalClose=ui.onNodeWithTag("terminal_close")
            privateCheckpointDeadline("terminal_close",checkpointDeadline);privateCheckpointTerminalDialogGuard("terminal_close",runner,owner,epoch,terminalWindow,terminalDialogTag,terminalTargetTag,"terminal_close",terminalBaseApplicationToken,checkpointDeadline);terminalClose.assertIsDisplayed().assertIsEnabled();privateCheckpointDeadline("terminal_close",checkpointDeadline);privateCheckpointTerminalDialogGuard("terminal_close",runner,owner,epoch,terminalWindow,terminalDialogTag,terminalTargetTag,"terminal_close",terminalBaseApplicationToken,checkpointDeadline);terminalClose.performClick();ui.waitForIdle();privateCheckpointDeadline("terminal_close",checkpointDeadline)
            privateCheckpointStage="terminal_posteffect"
            privateCheckpointRequire("terminal_posteffect",owner.terminal==null&&terminalInMemoryAuthority())
            privateCheckpointGuard("terminal_posteffect",runner,owner,epoch,Screen.Conversation,null,false)
            ui.onNodeWithTag(terminalDialogTag).assertDoesNotExist()
            ui.onNodeWithTag(conversationTag).assertIsDisplayed()
            val afterTerminalActions=privateCheckpointActions(owner,expectedHost)
            privateCheckpointDeadline("terminal_posteffect",checkpointDeadline)
            privateCheckpointRequire("terminal_posteffect",terminalStoreCurrent(afterTerminalActions))
        }
    }
    private fun stockReducer(owner:GoatrViewModel,host:String,target:Target)=owner.connection.state.value.sessions[host]
        ?.firstOrNull {it.state?.summary?.let {s->Target(s.sessionId,s.generation)==target}==true}
    private data class StockAwaitState(
        val hello:Boolean,
        val inventoryReady:Boolean,
        val sourcePresent:Boolean,
        val exactReducer:Boolean,
        val mutationsEnabled:Boolean,
        val busy:Boolean,
        val refreshEnabled:Boolean,
        val subscriptionPresent:Boolean,
        val exactSelection:Boolean,
        val connectivity:String,
        val inventoryTarget:String,
    ) {
        private fun bit(value:Boolean)=if(value) 1 else 0
        fun closed()="hello=${bit(hello)},inventory_ready=${bit(inventoryReady)},source_present=${bit(sourcePresent)},exact_reducer=${bit(exactReducer)},mutations_enabled=${bit(mutationsEnabled)},busy=${bit(busy)},refresh_enabled=${bit(refreshEnabled)},subscription_present=${bit(subscriptionPresent)},exact_selection=${bit(exactSelection)},connectivity=$connectivity,inventory_target=$inventoryTarget"
    }
    private data class StockRefreshState(
        val selection:Boolean,
        val busy:Boolean,
        val hello:Boolean,
        val inventoryReady:Boolean,
        val mutationReady:Boolean,
        val source:String,
        val reducer:String,
        val older:String,
        val notice:String,
    ) {
        private fun bit(value:Boolean)=if(value) 1 else 0
        fun ready()=selection && !busy && source=="changed" && older=="false"
        fun closed(failure:String)="selection=${bit(selection)},busy=${bit(busy)},hello=${bit(hello)},inventory_ready=${bit(inventoryReady)},mutation_ready=${bit(mutationReady)},source=$source,reducer=$reducer,older=$older,notice=$notice,failure=$failure"
    }
    private fun stockAwait(owner:GoatrViewModel,host:String,target:Target,refreshReady:Boolean=false,
        diagnostics:DevelopmentAutomationCapture?=null,captureProvider:Provider?=null):Uuid {
        val deadline=android.os.SystemClock.elapsedRealtime()+30_000L
        var readySource:Uuid?=null
        var last:StockAwaitState?=null
        try {
            ui.waitUntil(30_000L) {
                val view=owner.connection.state.value
                val source=view.sources[host]?.get(target)
                val reducer=view.sessions[host]?.firstOrNull {it.state?.summary?.let {summary->
                    Target(summary.sessionId,summary.generation)==target}==true}
                val hello=view.hello.containsKey(host)
                val inventoryReady=view.inventoryReady[host]==true
                val mutationReady=view.mutationReady[host]==true
                val mutationsEnabled=reducer?.mutationsEnabled==true
                val busy=owner.busy
                val baseReady=hello && inventoryReady && mutationReady && source!=null && mutationsEnabled && !busy
                // Preserve the existing conditional UI assertion: it is not evaluated until its
                // preceding readiness conjuncts pass and this caller actually requires refresh.
                val refreshSource=if(baseReady && refreshReady) {
                    runCatching {
                        ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("conversation_refresh"))
                        ui.onNodeWithTag("conversation_refresh").performScrollTo().assertIsDisplayed().assertIsEnabled()
                    }.getOrNull()?.let {
                        val refreshedView=owner.connection.state.value
                        val refreshedSource=refreshedView.sources[host]?.get(target)
                        val refreshedReducer=refreshedView.sessions[host]?.firstOrNull {it.state?.summary?.let {summary->
                            Target(summary.sessionId,summary.generation)==target}==true}
                        if(refreshedView.hello.containsKey(host) && refreshedView.inventoryReady[host]==true &&
                            refreshedView.mutationReady[host]==true && refreshedSource!=null &&
                            refreshedReducer?.mutationsEnabled==true && !owner.busy) refreshedSource else null
                    }
                } else null
                if(android.os.SystemClock.elapsedRealtime()>=deadline) return@waitUntil false
                val refreshEnabled=if(baseReady && refreshReady) refreshSource!=null else true
                val inventoryTarget=when {
                    view.inventory[host]?.sessions?.any {Target(it.sessionId,it.generation)==target}==true -> "same"
                    view.inventory[host]?.sessions?.any {it.sessionId==target.sessionId}==true -> "replaced"
                    else -> "absent"
                }
                last=StockAwaitState(
                    hello=hello,inventoryReady=inventoryReady,
                    sourcePresent=source!=null,exactReducer=reducer!=null,
                    mutationsEnabled=mutationsEnabled,busy=busy,
                    refreshEnabled=refreshEnabled,subscriptionPresent=reducer?.source!=null,
                    exactSelection=owner.selection==ConversationKey(host,target),
                    connectivity=when(reducer?.state?.summary?.connectivity) {
                        SessionConnectivity.Ready -> "ready"
                        SessionConnectivity.Synchronizing -> "synchronizing"
                        SessionConnectivity.Offline -> "offline"
                        SessionConnectivity.SetupRequired -> "setup_required"
                        SessionConnectivity.Unavailable -> "unavailable"
                        null -> "absent"
                    },inventoryTarget=inventoryTarget)
                val ready=baseReady && (!refreshReady || refreshEnabled)
                if(ready) readySource=if(refreshReady) checkNotNull(refreshSource) else source
                ready
            }
        } catch(timeout:ComposeTimeoutException) {
            last?.let {snapshot->diagnostics?.let {capture->captureProvider?.let {provider->
                runCatching {capture.snapshot("stock_await_timeout",provider,snapshot.closed())}
            }}}
            throw timeout
        }
        return checkNotNull(readySource)
    }
    private fun stockRefreshState(owner:GoatrViewModel,host:String,target:Target,previous:Uuid):StockRefreshState {
        val view=owner.connection.state.value
        val sourceMap=view.sources[host]
        val source=when(val current=sourceMap?.get(target)) {
            null -> if(sourceMap==null) "unobserved" else "absent"
            previous -> "same"
            else -> "changed"
        }
        val reducer=stockReducer(owner,host,target)
        return StockRefreshState(
            selection=owner.selection==ConversationKey(host,target),
            busy=owner.busy,
            hello=view.hello.containsKey(host),
            inventoryReady=view.inventoryReady[host]==true,
            mutationReady=view.mutationReady[host]==true,
            source=source,
            reducer=if(reducer==null) "absent" else "present",
            older=when(reducer?.browsingOlder) {true->"true";false->"false";null->"unobserved"},
            notice=stockNoticeCategory(owner.notice),
        )
    }
    private fun stockRefreshInitialState(owner:GoatrViewModel,host:String,target:Target):StockRefreshState {
        val view=owner.connection.state.value
        return StockRefreshState(
            selection=owner.selection==ConversationKey(host,target),
            busy=owner.busy,
            hello=view.hello.containsKey(host),
            inventoryReady=view.inventoryReady[host]==true,
            mutationReady=view.mutationReady[host]==true,
            source="unobserved",
            reducer=if(stockReducer(owner,host,target)==null) "absent" else "present",
            older="unobserved",
            notice=stockNoticeCategory(owner.notice),
        )
    }
    private fun stockRefreshFailure(failure:Throwable)=when(failure) {
        is ComposeTimeoutException -> "compose_timeout"
        is CancellationException -> "cancelled"
        else -> "other"
    }
    private fun observeStockRefresh(owner:GoatrViewModel,host:String,target:Target,previous:Uuid,
        diagnostics:DevelopmentAutomationCapture,provider:Provider) {
        var last=stockRefreshInitialState(owner,host,target)
        val deadline=android.os.SystemClock.elapsedRealtime()+120_000L
        try {
            stockTap("conversation_refresh",requireEnabled=true)
            ui.waitUntil((deadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {
                val snapshot=stockRefreshState(owner,host,target,previous)
                last=snapshot
                android.os.SystemClock.elapsedRealtime()<deadline && snapshot.ready()
            }
            check(android.os.SystemClock.elapsedRealtime()<deadline) {"Development refresh observation exceeded its absolute deadline"}
        } catch(failure:Throwable) {
            runCatching {diagnostics.snapshot("refresh_timeout",provider,last.closed(stockRefreshFailure(failure)))}
            throw failure
        }
    }
    private fun stockSendReadiness(owner:GoatrViewModel,host:String,target:Target,text:String):String {
        val key=ConversationKey(host,target)
        val selection=owner.selection==key
        val reservation=owner.reservation?.let {it.key==key && it.text==text}==true
        val current=owner.connection.state.value.mutationReady[host]==true && stockReducer(owner,host,target)?.let {!it.browsingOlder && it.mutationsEnabled}==true
        val enabled=runCatching {ui.onNodeWithTag("prompt_send").assertIsEnabled()}.isSuccess
        return "selection=${if(selection) 1 else 0},reservation=${if(reservation) 1 else 0},current=${if(current) 1 else 0},send_enabled=${if(enabled) 1 else 0}"
    }
    private fun stockReceiptState(receipt:Receipt?,authoritative:Boolean):String {
        val state=when(receipt?.state) {
            null -> "absent"
            ReceiptState.Reserved -> "reserved"
            ReceiptState.Dispatching -> "dispatching"
            ReceiptState.Dispatched -> "dispatched"
            ReceiptState.Succeeded -> "succeeded"
            ReceiptState.Failed -> "failed"
            ReceiptState.Uncertain -> "uncertain"
        }
        return "state=$state,authoritative=${if(authoritative) 1 else 0}"
    }
    private fun stockReceiptState(action:LocalAction?):String =
        if(action?.receipt==null) "state=${if(action==null) "absent" else "reserved"},authoritative=${if(action?.authoritative==true) 1 else 0}"
        else stockReceiptState(action.receipt,action.authoritative)
    private fun stockActionIdentity(actual:Uuid?,expected:Uuid)=when {
        actual==null -> "absent"
        actual==expected -> "same"
        else -> "other"
    }
    private fun stockNoticeCategory(notice:String?):String {
        if(notice==null) return "none"
        if(notice=="Protected identity is inaccessible. No replacement identity was created. Restore access or explicitly Forget.") return "protected_key"
        if(notice=="Recovery capacity changed. Review a new action-specific replacement warning before sending.") return "recovery_approval"
        if(notice=="Operation did not complete. Delivery may be unknown; inspect receipts. No automatic replay.") return "generic"
        ErrorCode.entries.firstOrNull {notice=="Operation unavailable: $it. Inspect receipts; no automatic replay."}?.let {return "store_policy:$it"}
        ErrorCode.entries.firstOrNull {notice=="Host operation: $it. No automatic retry or replay."}?.let {return "remote_failure:$it"}
        return "other"
    }
    private fun stockAdmissionFailure(failure:ActionAdmissionFailure?):String = failure?.let {
        "${it.phase.closed}:${it.guard.closed}:${it.code}:${stockAdmissionFact(it.sent)}:${stockAdmissionFact(it.frameStarted)}"
    } ?: "none"
    private fun stockAdmissionFact(value:Boolean?):String = when(value) {true->"1";false->"0";null->"unknown"}
    private fun stockSendAdmissionState(point:String,reservation:String)=
        "point=$point,reservation=$reservation"
    private fun stockReceiptTimeoutState(owner:GoatrViewModel,host:String,target:Target,action:Uuid,local:LocalAction?,
        beforeReservation:String,afterReservation:String):String {
        val key=ConversationKey(host,target)
        val current=stockReducer(owner,host,target)?.let {!it.browsingOlder && it.mutationsEnabled}==true
        return "foreground=${if(owner.foreground) 1 else 0},busy=${if(owner.busy) 1 else 0},exact_selection=${if(owner.selection==key) 1 else 0},current=${if(current) 1 else 0},before_reservation=$beforeReservation,after_reservation=$afterReservation,reservation=${stockActionIdentity(owner.reservation?.action,action)},replacement=${stockActionIdentity(owner.replacement?.action,action)},focused=${stockActionIdentity(owner.focusedAction?.metadata?.actionId,action)},local_row=${stockActionIdentity(local?.metadata?.actionId,action)},notice=${stockNoticeCategory(owner.notice)},admission=${stockAdmissionFailure(owner.connection.actionAdmissionFailure(host,action))}"
    }
    private sealed class StockToolTerminal {
        object Pending:StockToolTerminal()
        object Rejected:StockToolTerminal()
        data class Ready(val call:Id):StockToolTerminal()
    }
    private data class StockToolExecution(val records:List<HistoryRecord>) {
        private val calls get()=records.mapNotNull {it.tool?.callId}.distinct()
        private val unkeyed get()=records.count {it.tool?.callId==null}
        private fun count(value:Int)=when(value) {0->"zero";1->"one";else->"many"}
        fun terminal():StockToolTerminal {
            if(records.isEmpty()) return StockToolTerminal.Pending
            var call:Id?=null
            var pending=false
            records.forEach {record->
                val tool=record.tool ?: return StockToolTerminal.Rejected
                val current=tool.callId ?: return StockToolTerminal.Rejected
                if(call==null) call=current else if(call!=current) return StockToolTerminal.Rejected
                when(tool.status) {
                    ToolStatus.Running,ToolStatus.Unknown -> pending=true
                    ToolStatus.Failed -> return StockToolTerminal.Rejected
                    ToolStatus.Succeeded -> Unit
                }
            }
            return if(pending) StockToolTerminal.Pending else StockToolTerminal.Ready(call!!)
        }
        fun state(expected:Id?):String {
            val identity=when {
                calls.size!=1 || unkeyed!=0 -> "missing"
                expected==null || calls.single()==expected -> "same"
                else -> "changed"
            }
            return "rows=${count(records.size)},calls=${count(calls.size)},unkeyed=${count(unkeyed)},identity=$identity"
        }
        fun requireIdentity(expected:Id?=null):Id {
            assertTrue("Matching native tool output must be completed",records.isNotEmpty())
            assertTrue("Matching native tool output must all be successful",records.all {it.tool?.status==ToolStatus.Succeeded})
            assertEquals("Matching native tool output must all carry native call identities",0,unkeyed)
            assertEquals("Matching native tool output must describe one native execution",1,calls.size)
            val terminal=terminal()
            assertTrue("Matching native tool output must contain one terminal successful execution",terminal is StockToolTerminal.Ready)
            val actual=(terminal as StockToolTerminal.Ready).call
            if(expected!=null) assertTrue("Reconnected native tool output must retain the original native execution identity",actual==expected)
            return actual
        }
    }
    private fun stockToolExecution(owner:GoatrViewModel,host:String,target:Target,marker:String)=StockToolExecution(
        stockReducer(owner,host,target)?.history.orEmpty().filter {record->
            record.complete && ((record.tool?.output as? ContentText)?.text?.value?.contains(marker)==true)
        }
    )
    private fun stockToolRecord(id:String,call:Id?,status:ToolStatus)=HistoryRecord(
        id=Id(id),order=Uint(0),role=Role.Tool,kind=RecordKind.Tool,
        tool=Tool("commandExecution",call,status,output=ContentText(InlineText("goatr-terminal-marker"))),
        source=RecordSource.Live,complete=true,sourceTruncated=false,
    )
    /** Installed-subject cleanup is intentionally UI-only: names arrive only in authenticated
     * control memory, are resolved once before mutation, and never leave this process. */
    @Test fun installed_subject_owned_retirement_checkpoint()=runBlocking<Unit> {
        privateCheckpointRequire("retirement_selector",privateCheckpointSelected())
        val runner=InstrumentationRegistry.getInstrumentation() as? OwnedInstrumentationRunner
        privateCheckpointRequire("retirement_runner",runner!=null)
        privateCheckpointStage="retirement_awaiting_foreground";runner!!.prepareSdkOwnedFixture()
        privateCheckpointStage="retirement_activity";val activity=runner.sdkOwnedFixtureMainActivity()
        lateinit var owner:GoatrViewModel;activity.runOnUiThread {owner=ViewModelProvider(activity)[GoatrViewModel::class.java]}
        ui.waitUntil(20_000) {owner.foreground&&owner.connection.ownsNetworkCallback()}
        lateinit var epoch:PrivateInputEpoch;ui.waitUntil(10_000) {privateCheckpointWindow(runner,owner)?.let {epoch=it;true}?:false}
        privateCheckpointGuard("retirement_initial",runner,owner,epoch,Screen.Sessions,null,false)
        val deadline=android.os.SystemClock.elapsedRealtime()+900_000L
        privateCheckpointStage="retirement_inventory"
        val inventory=stockControl("retirement_inventory")
        val targetCount=inventory.getValue("targetCount").jsonPrimitive.int
        val candidates=inventory.getValue("targets").jsonArray.map {row->
            val value=row.jsonObject
            Triple(value.getValue("pairingName").jsonPrimitive.content,value["expectedHostId"]?.jsonPrimitive?.contentOrNull,value["expectedDeviceId"]?.jsonPrimitive?.contentOrNull)
        }
        privateCheckpointRequire("retirement_inventory",targetCount in 1..128&&candidates.size==targetCount&&candidates.map {it.first}.toSet().size==candidates.size)
        val originalHosts=owner.store.state.value.hosts.toList()
        val originalActions=originalHosts.associate {host->host.hostId to privateCheckpointActions(owner,host.hostId).toSet()}
        val originalActionIds=originalActions.values.flatten().map {it.metadata.actionId}.toSet()
        suspend fun currentActionIds()=owner.store.state.value.hosts.flatMap {host->privateCheckpointActions(owner,host.hostId)}.map {it.metadata.actionId}.toSet()
        val resolved=candidates.map {candidate->
            val matches=originalHosts.filter {host->host.name==candidate.first&&(candidate.second==null||host.hostId==candidate.second)&&(candidate.third==null||host.deviceId?.value==candidate.third)}
            privateCheckpointRequire("retirement_inventory",matches.size<=1)
            candidate to matches.singleOrNull()
        }
        val present=resolved.mapNotNull {it.second}
        privateCheckpointRequire("retirement_inventory",present.size<=32&&present.map {it.hostId}.toSet().size==present.size)
        val originalHostsById=originalHosts.associateBy {it.hostId}
        val originalHostIds=originalHosts.map {it.hostId}.toSet()
        val targetHostIds=present.map {it.hostId}.toSet()
        val removedHostIds=mutableSetOf<String>()
        var removed=0;var alreadyAbsent=0
        for ((candidate,target) in resolved) {
            val (name,expectedHost,expectedDevice)=candidate
            privateCheckpointStage="retirement_next";val next=stockControl("retirement_next")
            privateCheckpointRequire("retirement_next",next.getValue("pairingName").jsonPrimitive.content==name)
            if(target==null) {privateCheckpointRequire("retirement_actions",currentActionIds().all {it in originalActionIds});privateCheckpointRequire("retirement_result",stockControl("retirement_result", "removed" to JsonPrimitive(false)).getValue("accepted").jsonPrimitive.boolean);alreadyAbsent++;continue}

            privateCheckpointDeadline("retirement_deadline",deadline)
            fun retirementTargetPresent()=owner.store.state.value.hosts.singleOrNull {it.hostId==target.hostId}==target&&(expectedHost==null||target.hostId==expectedHost)&&(expectedDevice==null||target.deviceId?.value==expectedDevice)&&owner.foreground&&!owner.busy
            fun retirementTargetCurrent()=owner.host==target.hostId&&retirementTargetPresent()
            privateCheckpointRequire("retirement_authority",retirementTargetPresent())
            if(owner.screen==Screen.Sessions) {
                privateCheckpointTap("retirement_menu","navigation_menu",runner,owner,epoch,Screen.Sessions,null,false,deadline=deadline)
                privateCheckpointTap("retirement_hosts","navigation_hosts_menu",runner,owner,epoch,Screen.Sessions,null,false,popup=true,deadline=deadline)
                ui.waitUntil(minOf(10_000L,(deadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L))) {owner.screen==Screen.Connections}
            }
            privateCheckpointGuard("retirement_host",runner,owner,epoch,Screen.Connections,null,false)
            val settings=ui.onNode(hasTestTag("host_settings") and hasAnyAncestor(hasTestTag("host_${target.hostId}")),useUnmergedTree=true)
            if(runCatching {settings.assertIsDisplayed()}.isFailure) ui.onNodeWithTag("connections_list").performScrollToNode(hasTestTag("host_${target.hostId}"))
            privateCheckpointGuard("retirement_host",runner,owner,epoch,Screen.Connections,null,false)
            settings.assertIsDisplayed().assertIsEnabled().performClick();ui.waitForIdle()
            ui.waitUntil(minOf(10_000L,(deadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L))) {owner.screen==Screen.Settings}
            privateCheckpointGuard("retirement_forget",runner,owner,epoch,Screen.Settings,null,false)
            privateCheckpointRequire("retirement_forget",owner.host==target.hostId&&owner.store.state.value.hosts.singleOrNull {it.hostId==target.hostId}==target)
            if(runCatching {ui.onNodeWithTag("host_forget").assertIsDisplayed()}.isFailure) {
                privateCheckpointDeadline("retirement_forget",deadline)
                privateCheckpointGuard("retirement_forget",runner,owner,epoch,Screen.Settings,null,false)
                privateCheckpointRequire("retirement_forget",retirementTargetCurrent())
                ui.onNodeWithTag("host_settings_list").performScrollToNode(hasTestTag("host_forget"))
            }
            privateCheckpointTap("retirement_forget","host_forget",runner,owner,epoch,Screen.Settings,null,false,deadline=deadline,beforeClick={
                privateCheckpointRequire("retirement_forget",retirementTargetCurrent())
            })
            ui.waitUntil(minOf(10_000L,(deadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L))) {runCatching {ui.onNodeWithTag("forget_confirm").assertIsDisplayed()}.isSuccess}
            val dialog=privateCheckpointModalWindow("retirement_confirm",runner,owner,epoch,Screen.Settings,"forget_confirm",deadline)
            privateCheckpointRequire("retirement_confirm",dialog!=null)
            privateCheckpointRequire("retirement_confirm",owner.host==target.hostId&&owner.store.state.value.hosts.singleOrNull {it.hostId==target.hostId}==target)
            privateCheckpointModalTap("retirement_confirm","forget_confirm",runner,owner,epoch,Screen.Settings,dialog!!,deadline,beforeClick={
                privateCheckpointRequire("retirement_confirm",retirementTargetCurrent())
            })
            ui.waitUntil((deadline-android.os.SystemClock.elapsedRealtime()).coerceAtLeast(0L)) {
                !owner.busy&&owner.screen==Screen.Connections&&owner.host==null&&owner.store.state.value.hosts.none {it.hostId==target.hostId}&&owner.store.state.value.keyDeletion.none {it.hostId==target.hostId}
            }
            privateCheckpointGuard("retirement_posteffect",runner,owner,epoch,Screen.Connections,null,false)
            removedHostIds+=target.hostId
            privateCheckpointRequire("retirement_preserve",owner.store.state.value.hosts.map {it.hostId}.toSet()==originalHostIds-removedHostIds&&owner.store.state.value.hosts.all {current->current==originalHostsById[current.hostId]})
            for(current in owner.store.state.value.hosts) if(current.hostId !in targetHostIds) privateCheckpointRequire("retirement_actions",privateCheckpointActions(owner,current.hostId).toSet()==originalActions.getValue(current.hostId))
            privateCheckpointStage="retirement_result";privateCheckpointRequire("retirement_actions",currentActionIds().all {it in originalActionIds})
            privateCheckpointRequire("retirement_result",stockControl("retirement_result", "removed" to JsonPrimitive(true)).getValue("accepted").jsonPrimitive.boolean)
            removed++
        }
        privateCheckpointStage="retirement_finish"
        privateCheckpointRequire("retirement_finish",owner.store.state.value.hosts.map {it.hostId}.toSet()==originalHostIds-removedHostIds&&owner.store.state.value.hosts.all {current->current==originalHostsById[current.hostId]}&&owner.store.state.value.hosts.filter {it.hostId !in targetHostIds}.all {current->privateCheckpointActions(owner,current.hostId).toSet()==originalActions.getValue(current.hostId)}&&currentActionIds().all {it in originalActionIds})
        val finish=stockControl("retirement_finish","removed" to JsonPrimitive(removed),"already_absent" to JsonPrimitive(alreadyAbsent))
        privateCheckpointRequire("retirement_finish",finish.getValue("targetCount").jsonPrimitive.int==targetCount&&finish.getValue("removed").jsonPrimitive.int==removed&&finish.getValue("alreadyAbsent").jsonPrimitive.int==alreadyAbsent)
    }

    @Test fun review_completed_tool_output_requires_terminal_singleton_success() {
        val call=Id("call-a")
        val running=stockToolRecord("stable",call,ToolStatus.Running)
        val replacement=mutableListOf(running)
        assertSame(StockToolTerminal.Pending,StockToolExecution(replacement).terminal())
        replacement[0]=stockToolRecord("stable",call,ToolStatus.Succeeded)
        assertEquals(StockToolTerminal.Ready(call),StockToolExecution(replacement).terminal())
        assertSame(StockToolTerminal.Pending,StockToolExecution(listOf(stockToolRecord("unknown",call,ToolStatus.Unknown))).terminal())
        assertSame(StockToolTerminal.Rejected,StockToolExecution(listOf(stockToolRecord("failed",call,ToolStatus.Failed))).terminal())
        assertSame(StockToolTerminal.Rejected,StockToolExecution(listOf(stockToolRecord("unkeyed",null,ToolStatus.Succeeded))).terminal())
        assertSame(StockToolTerminal.Rejected,StockToolExecution(listOf(
            stockToolRecord("succeeded",call,ToolStatus.Succeeded),
            stockToolRecord("failed",call,ToolStatus.Failed),
        )).terminal())
        assertSame(StockToolTerminal.Rejected,StockToolExecution(listOf(
            stockToolRecord("first",call,ToolStatus.Succeeded),
            stockToolRecord("second",Id("call-b"),ToolStatus.Succeeded),
        )).terminal())
    }
    private suspend fun stockReceipt(owner:GoatrViewModel,host:String,id:Uuid,
        diagnostics:DevelopmentAutomationCapture?=null,provider:Provider?=null,target:Target?=null,
        beforeReservation:String?=null,afterReservation:String?=null):Receipt {
        var action:LocalAction?=null
        var captured=false
        try {
            withTimeout(130000) {
                while(true) {
                    action=owner.store.localAction(host,id)
                    if(!captured) {diagnostics?.snapshot("receipt_polling",provider,stockReceiptState(action));captured=true}
                    if(action?.authoritative==true && action?.receipt?.state !in listOf(null,ReceiptState.Reserved,ReceiptState.Dispatching)) break
                    delay(25)
                }
            }
        } catch(failure:Throwable) {
            runCatching {diagnostics?.snapshot("receipt_polling",provider,stockReceiptState(action))}
            if(target!=null && beforeReservation!=null && afterReservation!=null) {
                runCatching {diagnostics?.snapshot("receipt_timeout",provider,stockReceiptTimeoutState(owner,host,target,id,action,beforeReservation,afterReservation))}
            }
            throw failure
        }
        return action!!.receipt!!
    }
    private suspend fun stockSend(owner:GoatrViewModel,host:String,target:Target,text:String,
        diagnostics:DevelopmentAutomationCapture?=null,captureProvider:Provider?=null,
        requireCurrentHistory:Boolean=false,verifyCanonicalDetails:Boolean=true):Pair<Uuid,Receipt> {
        stockAwait(owner,host,target,diagnostics=diagnostics,captureProvider=captureProvider)
        val provider=stockReducer(owner,host,target)?.state?.summary?.provider
            ?: error("Actual stock send lost its current provider")
        ui.onNodeWithTag("composer").performScrollTo().performTextReplacement(text)
        diagnostics?.snapshot("send_wait",provider,stockSendReadiness(owner,host,target,text))
        try {
            ui.waitUntil(10000) {
                val readiness=stockSendReadiness(owner,host,target,text)
                readiness=="selection=1,reservation=1,current=1,send_enabled=1" &&
                    (!requireCurrentHistory || stockReducer(owner,host,target)?.browsingOlder==false)
            }
        } catch(failure:Throwable) {
            runCatching {diagnostics?.snapshot("send_wait",provider,stockSendReadiness(owner,host,target,text))}
            throw failure
        }
        val action=owner.reservation!!.action
        owner.connection.beginActionAdmissionCapture(host,action)
        try {
            ui.onNodeWithTag("send_deadline").performScrollTo().assertIsDisplayed()
            diagnostics?.snapshot("send_ready",provider,"selected_reserved_current_enabled")
            val beforeReservation=stockActionIdentity(owner.reservation?.action,action)
            diagnostics?.snapshot("send_admission",provider,stockSendAdmissionState("before",beforeReservation))
            var afterReservation="absent"
            stockTap("prompt_send",requireEnabled=true) {
                afterReservation=stockActionIdentity(owner.reservation?.action,action)
                diagnostics?.snapshot("send_admission",provider,stockSendAdmissionState("after",afterReservation))
            }
            diagnostics?.snapshot("send_clicked",provider,"clicked")
            val receipt=stockReceipt(owner,host,action,diagnostics,provider,target,beforeReservation,afterReservation)
            // Keep the final snapshot tied to the canonical receipt rather than the initial poll.
            diagnostics?.snapshot("receipt_polling",provider,stockReceiptState(receipt,true))
            if(!verifyCanonicalDetails) return action to receipt
            // The production tail follows the canonical receipt only while the user follows the
            // transcript. Do not simulate an inspection scroll over a long virtualized history.
            val details="receipt_details_${action.value}"
            val backing=Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt).toString()
            var detailsVisible=false
            ui.waitUntil(10000) {
                detailsVisible=runCatching {ui.onNodeWithTag(details).assertIsDisplayed()}.isSuccess
                detailsVisible || runCatching {ui.onNodeWithTag("jump_latest").assertIsDisplayed()}.isSuccess
            }
            if(!detailsVisible && !runCatching {ui.onNodeWithTag(details).assertIsDisplayed()}.isSuccess) {
                // Returning from older history keeps the reader's anchor. The production control,
                // rather than a test scroll, explicitly returns to the receipt at the live tail.
                ui.onNodeWithTag("jump_latest").performClick()
            }
            ui.waitUntil(10000) {runCatching {ui.onNodeWithTag(details).assertIsDisplayed()}.isSuccess}
            ui.onNodeWithTag(details).performClick()
            ui.waitUntil(10000) {runCatching {ui.onNode(hasTestTag("receipt_exact") and hasText(backing)).assertIsDisplayed()}.isSuccess}
            withTimeout(10000) {while(owner.store.localAction(host,action)?.outcomeShown!=true) delay(20)}
            if(receipt.state in listOf(ReceiptState.Dispatched,ReceiptState.Succeeded)) {
                withTimeout(10000) {while(owner.store.state.value.recovery.any {it.action.actionId==action}) delay(20)}
                diagnostics?.snapshot("receipt_display",provider,"canonical_recovery_cleared")
            }
            return action to receipt
        } finally {
            owner.connection.clearActionAdmissionCapture(host,action)
        }
    }
    private suspend fun stockCapture(owner:GoatrViewModel,scenario:ActivityScenario<MainActivity>,scene:String) {
        assertTrue("Capture only sanitized nonsecret scenes",owner.screen!=Screen.Pairing && owner.preview==null)
        assertTrue("No secret or URL card may reach screenshot evidence",
            owner.selection?.let {key->stockReducer(owner,key.host,key.target)?.pending.orEmpty().none {r->
                r.presentation is PresentationCodexUrl || (r.presentation as? PresentationCodexQuestions)?.questions?.any {it.isSecret}==true
            }}!=false)
        scenario.onActivity {assertTrue("Private scene must not be captured",it.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE==0)}
        stockControl("capture","scene" to JsonPrimitive(scene))
    }
    private suspend fun stockVisibleDialog(owner:GoatrViewModel,host:String,selected:Target,provider:Provider,kind:PresentationKind) {
        val trigger=stockControl("trigger_dialog","provider" to Protocol.strictJson.encodeToJsonElement(Provider.serializer(),provider),
            "kind" to Protocol.strictJson.encodeToJsonElement(PresentationKind.serializer(),kind))
        val key=Uuid(trigger.getValue("requestKey").jsonPrimitive.content)
        stockAnswerExistingDialog(owner,host,selected,key,kind)
    }
    private suspend fun stockAnswerExistingDialog(owner:GoatrViewModel,host:String,selected:Target,key:Uuid,kind:PresentationKind) {
        ui.waitUntil(30000) {stockReducer(owner,host,selected)?.pending?.any {it.requestKey==key && it.kind==kind}==true}
        val request=stockReducer(owner,host,selected)!!.pending.first {it.requestKey==key}
        val before=owner.connection.state.value.latestReceipt[host]?.actionId
        stockTap("request_select_${key.value}")
        when(val p=request.presentation) {
            is PresentationOmpSelect -> stockTap("omp_option_0")
            is PresentationOmpEditor -> {
                ui.onNodeWithTag("editor_input").performScrollTo().performTextReplacement("owned nonsecret stock editor answer")
                stockTap("form_submit")
            }
            is PresentationCodexApproval -> {
                val index=p.choices.indexOfFirst {it.scope==ApprovalScope.Once && (it.label.value.startsWith("Cancel;") || it.label.value.startsWith("Decline;") || it.label.value=="Deny permissions")}
                assertTrue("Actual stock approval must offer a safe decline/cancel; no inferred grant",index>=0)
                stockTap("approval_$index")
            }
            is PresentationCodexQuestions -> {
                p.questions.forEachIndexed {index,q->
                    if(q.options.isNotEmpty()) stockTap("question_${index}_option_0") else {
                        // Never print/assert the secret editable value; production memory owns it.
                        ui.onNodeWithTag("question_${index}_input").performScrollTo().performTextInput("owned nonsecret answer")
                        stockTap("question_${index}_add")
                    }
                }
                stockTap("form_submit")
            }
            is PresentationCodexForm -> stockTap("form_decline")
            is PresentationCodexUrl -> stockTap("url_Decline")
            else -> fail("Requested supported actual stock card is desktop-only; no synthetic answer")
        }
        ui.waitUntil(30000) {owner.connection.state.value.latestReceipt[host]?.let {it.actionId!=before && it.method==ActionMethod.RequestRespond}==true}
        val receipt=owner.connection.state.value.latestReceipt.getValue(host)
        assertTrue("First-response receipt must not claim this phone won",receipt.result !is ActionResultResponse || receipt.state in listOf(ReceiptState.Dispatched,ReceiptState.Succeeded,ReceiptState.Uncertain))
        stockControl("dialog_closed","target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),selected),
            "request_key" to JsonPrimitive(key.value),"receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt))
        ui.waitUntil(10000) {stockReducer(owner,host,selected)?.pending?.none {it.requestKey==key}==true}
        assertTrue(owner.selection==ConversationKey(host,selected))
    }
    @Test fun development_fresh_sessions_continue_from_android_to_desktop()=runBlocking<Unit> {developmentContinuity(false)}
    @Test fun development_feedback_android_to_desktop()=runBlocking<Unit> {developmentContinuity(true)}
    private suspend fun developmentContinuity(feedback:Boolean) {
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Development consumer must start from its own cleared debug installation",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        IrohAndroid.installAndroidContext(context)
        val info=stockControl("live_info")
        val host=info.getValue("host").jsonPrimitive.content
        val rows=info.getValue("sessions").jsonArray.map {it.jsonObject}
        assertEquals(setOf(Provider.Omp,Provider.Codex),rows.map(::stockProvider).toSet())
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner:GoatrViewModel
        var retained=false
        val diagnostics=DevelopmentAutomationCapture()
        var primary:Throwable?=null
        try {
            diagnostics.snapshot("pairing",null,"startup")
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.foreground && owner.connection.ownsNetworkCallback()}
            var uri=stockControl("live_pair").getValue("uri").jsonPrimitive.content
            diagnostics.snapshot("pairing",null,"begin")
            stockTap("pair_begin")
            try {
                ui.onNodeWithTag("pair_paste").performScrollTo().performSemanticsAction(SemanticsActions.SetText) {it(AnnotatedString(uri))}
                stockTap("pair_preview")
            } catch(_:Throwable) {
                uri="";fail("Actual development pairing paste failed; no fallback")
            }
            uri=""
            ui.waitUntil(10000) {owner.preview?.host?.value==host}
            diagnostics.snapshot("pairing",null,"preview")
            stockTap("pair_confirm")
            ui.waitUntil(30000) {owner.connection.state.value.hello[host]!=null && owner.screen==Screen.Sessions}
            diagnostics.snapshot("pairing",null,"confirmed")
            val device=owner.connection.state.value.hello.getValue(host).deviceId
            data class SubmittedTool(val target:Target,val action:Uuid,val receipt:Receipt,val marker:String,val nativeCallId:Id)
            val submitted=mutableListOf<SubmittedTool>()
            val historyObservations=mutableListOf<JsonObject>()
            val summaries=rows.map {row->
                val target=stockTarget(row)
                val provider=stockProvider(row)
                diagnostics.snapshot("session_open",provider,"requested")
                ui.onNodeWithTag("session_${target.sessionId.value}").performScrollTo().performClick()
                stockAwait(owner,host,target,diagnostics=diagnostics,captureProvider=provider)
                diagnostics.snapshot("session_open",provider,"ready")
                val reducer=stockReducer(owner,host,target)!!
                assertEquals(provider,reducer.state!!.summary.provider)
                assertEquals(row.getValue("nativeId").jsonPrimitive.content,reducer.state!!.summary.nativeId.value)
                diagnostics.snapshot("history_attempt",provider,"requested")
                ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("history_browse"))
                ui.onNodeWithTag("history_browse").performClick()
                val historyStatus=try {
                    ui.waitUntil(30000) {stockReducer(owner,host,target)?.browsingOlder==true}
                    "observed"
                } catch(timeout:ComposeTimeoutException) {
                    if(!feedback) throw timeout
                    "unproven"
                }
                diagnostics.snapshot("history_attempt",provider,historyStatus)
                historyObservations+=buildJsonObject {
                    put("provider",Protocol.strictJson.encodeToJsonElement(Provider.serializer(),provider))
                    put("status",historyStatus)
                }
                // The older window may outlive a temporary authoritative-source gate.
                // Capture the original source only at the same exact-Target readiness boundary.
                assertEquals(ConversationKey(host,target),owner.selection)
                val previousSource=stockAwait(owner,host,target,refreshReady=true,diagnostics=diagnostics,captureProvider=provider)
                diagnostics.snapshot("refresh",provider,"requested")
                observeStockRefresh(owner,host,target,previousSource,diagnostics,provider)
                stockAwait(owner,host,target,diagnostics=diagnostics,captureProvider=provider)
                assertFalse("Development send requires the current history window after refresh",stockReducer(owner,host,target)!!.browsingOlder)
                diagnostics.snapshot("refresh",provider,"current")
                val marker="goatr-development-ui-${provider.name}-${target.sessionId.value.take(8)}"
                val (action,receipt)=stockSend(owner,host,target,"Run the native shell tool printf '%s\\n' '$marker' in THIS exact existing session. Do not write files or create/switch sessions. Repeat $marker.",diagnostics=diagnostics,captureProvider=provider,requireCurrentHistory=true,verifyCanonicalDetails=!feedback)
                diagnostics.snapshot("native_observe",provider,"requested")
                stockControl("observe","target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),target),"marker" to JsonPrimitive(marker),
                    "receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt))
                diagnostics.snapshot("native_observe",provider,"verified")
                diagnostics.snapshot("tool_history",provider,"awaiting")
                ui.waitUntil(180000) {
                    when(stockToolExecution(owner,host,target,marker).terminal()) {
                        is StockToolTerminal.Ready -> true
                        StockToolTerminal.Pending -> false
                        StockToolTerminal.Rejected -> throw AssertionError("Matching native tool output was rejected or ambiguous")
                    }
                }
                val execution=stockToolExecution(owner,host,target,marker)
                diagnostics.snapshot("tool_history",provider,execution.state(null))
                val nativeCallId=execution.requireIdentity()
                submitted+=SubmittedTool(target,action,receipt,marker,nativeCallId)
                diagnostics.snapshot("desktop",provider,"requested")
                val desktop=stockControl("desktop","provider" to Protocol.strictJson.encodeToJsonElement(Provider.serializer(),provider)).getValue("marker").jsonPrimitive.content
                ui.waitUntil(30000) {stockReducer(owner,host,target)?.history?.any {(it.text as? ContentText)?.text?.value?.contains(desktop)==true}==true}
                diagnostics.snapshot("desktop",provider,"observed")
                stockTap("connections");stockTap("host_sessions")
                reducer.state!!.summary
            }
            diagnostics.snapshot("reconnect",null,"background")
            scenario.moveToState(Lifecycle.State.CREATED)
            ui.waitUntil(30000) {!owner.foreground}
            scenario.moveToState(Lifecycle.State.RESUMED)
            diagnostics.snapshot("reconnect",null,"resume_wait")
            ui.waitUntil(30000) {owner.foreground && owner.connection.ownsNetworkCallback() && owner.connection.state.value.hello[host]?.deviceId==device}
            for(submission in submitted) {
                val provider=rows.single {stockTarget(it)==submission.target}.let(::stockProvider)
                diagnostics.snapshot("reconnect",provider,"requested")
                stockTap("connections");stockTap("host_sessions")
                ui.onNodeWithTag("session_${submission.target.sessionId.value}").performScrollTo().performClick()
                stockAwait(owner,host,submission.target,diagnostics=diagnostics,captureProvider=provider)
                assertEquals("Foreground reconnect must reconcile the exact prior action without resubmit",submission.receipt,owner.store.localAction(host,submission.action)?.receipt)
                assertTrue("Reconnected receipt must remain authoritative",owner.store.localAction(host,submission.action)?.authoritative==true)
                if(!feedback) assertTrue("Only the displayed canonical admitted outcome clears this exact recovery copy",owner.store.localAction(host,submission.action)?.outcomeShown==true && owner.store.state.value.recovery.none {it.action.actionId==submission.action})
                val execution=stockToolExecution(owner,host,submission.target,submission.marker)
                diagnostics.snapshot("reconnect",provider,execution.state(submission.nativeCallId))
                execution.requireIdentity(submission.nativeCallId)
                stockTap("connections");stockTap("host_sessions")
            }
            stockControl("development_ready","device_id" to JsonPrimitive(device.value),
                "sessions" to JsonArray(summaries.map {Protocol.strictJson.encodeToJsonElement(SessionSummary.serializer(),it)}),
                "history" to JsonArray(historyObservations))
            retained=true
        } catch(failure:Throwable) {
            primary=failure
            throw failure
        } finally {
            var cleanupFailure:Throwable?=null
            withContext(NonCancellable) {
                runCatching {diagnostics.snapshot("cleanup",null,if(primary==null) "normal" else "primary_failed")}
                try {
                    scenario.moveToState(Lifecycle.State.CREATED)
                    owner.connection.shutdown()
                    if(!retained) {
                        if(owner.store.state.value.hosts.any {it.hostId==host}) owner.connection.forget(host)
                        if(owner.store.state.value.hosts.isEmpty()) owner.connection.forgetInstallation()
                    }
                    scenario.close()
                } catch(failure:Throwable) {
                    cleanupFailure=failure
                    runCatching {diagnostics.snapshot("cleanup_failure",null,"closed")}
                }
            }
            if(primary==null) cleanupFailure?.let {throw it}
        }
    }
    @Test fun stock_packaged_host_actual_main_activity_both_selected_providers()=runBlocking<Unit> {
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt an existing protected installation",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        IrohAndroid.installAndroidContext(context)
        val info=stockControl("live_info");val host=info.getValue("host").jsonPrimitive.content
        // Only the authenticated owned runner can select this explicit evidence deferral.
        val deferRetentionCrossing=info.getValue("deferRetentionCrossing").jsonPrimitive.boolean
        val run=info.getValue("runId").jsonPrimitive.content
        val rows=info.getValue("sessions").jsonArray.map {it.jsonObject}
        assertTrue("Each selected actual backend must be present, not ignored",rows.isNotEmpty())
        assertEquals(rows.size,rows.map(::stockProvider).distinct().size)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner:GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.foreground && owner.connection.ownsNetworkCallback()}
            assertTrue(owner.store.state.value.hosts.isEmpty())
            var raw=stockControl("live_pair").getValue("uri").jsonPrimitive.content
            stockTap("pair_begin")
            try {
                ui.onNodeWithTag("pair_paste").performScrollTo().performSemanticsAction(SemanticsActions.SetText) {it(AnnotatedString(raw))}
                stockTap("pair_preview")
            } catch(_:Throwable) {
                ui.runOnIdle {owner.connections()};raw=""
                fail("Actual private paste action failed; raw input cleared (no capability diagnostic)")
            }
            raw=""
            ui.waitUntil(10000) {owner.preview?.host?.value==host}
            ui.onNodeWithTag("pair_authority").assertIsDisplayed()
            assertTrue("Preview must not authorize or create host state",owner.store.state.value.hosts.isEmpty() && owner.connection.state.value.hello.isEmpty())
            stockTap("pair_cancel")
            assertTrue(owner.preview==null && owner.store.state.value.hosts.isEmpty())
            // A new explicit local pairing window, not token reuse or automatic retry.
            raw=stockControl("live_pair").getValue("uri").jsonPrimitive.content
            stockTap("pair_begin")
            try {
                ui.onNodeWithTag("pair_paste").performScrollTo().performSemanticsAction(SemanticsActions.SetText) {it(AnnotatedString(raw))}
                stockTap("pair_preview")
            } catch(_:Throwable) {ui.runOnIdle {owner.connections()};raw="";fail("Actual private pairing paste failed; no raw diagnostic")}
            raw=""
            ui.waitUntil(10000) {owner.preview?.host?.value==host}
            stockTap("pair_confirm")
            ui.waitUntil(30000) {owner.connection.state.value.hello[host]!=null && owner.screen==Screen.Sessions}
            val device=owner.connection.state.value.hello.getValue(host).deviceId
            assertTrue(owner.store.state.value.hosts.single().hostId==host)
            for(row in rows) {
                val selected=stockTarget(row);val provider=stockProvider(row);val native=row.getValue("nativeId").jsonPrimitive.content
                ui.onNodeWithTag("session_${selected.sessionId.value}").performScrollTo().performClick()
                stockAwait(owner,host,selected)
                assertTrue(stockReducer(owner,host,selected)!!.state!!.summary.nativeId.value==native)
                if(provider==Provider.Omp) ui.onNodeWithTag("omp_branches").assertExists()
                if(!deferRetentionCrossing) {
                val olderProof=stockControl("older","target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),selected))
                assertTrue(Protocol.strictJson.decodeFromJsonElement(Target.serializer(),olderProof.getValue("target"))==selected)
                assertTrue(olderProof.getValue("source").jsonPrimitive.content==if(provider==Provider.Omp) "persisted" else "native_page")
                assertTrue("Strict native boundary witness needs at least 1,000 later completed records",olderProof.getValue("qualifiedLaterCompletedLowerBound").jsonPrimitive.long>=1000L)
                val older=olderProof.getValue("id").jsonPrimitive.content
                val pagingSource=stockReducer(owner,host,selected)!!.source
                assertNotNull(pagingSource)
                val pagingCursors=mutableSetOf<Id>()
                var pagingTaps=1
                ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("history_browse"))
                ui.onNodeWithTag("history_browse").performClick()
                ui.waitUntil(30000) {stockReducer(owner,host,selected)?.browsingOlder==true}
                while(stockReducer(owner,host,selected)!!.history.none {it.id.value==older}) {
                    assertEquals(ConversationKey(host,selected),owner.selection)
                    val current=stockReducer(owner,host,selected)!!
                    assertEquals(pagingSource,current.source)
                    assertTrue("Actual native paging exceeded 64 visible taps",pagingTaps++<64)
                    val cursor=current.nextCursor ?: error("Actual native older cursor ended before proved ID")
                    assertTrue("Actual native cursor cycle",pagingCursors.add(cursor))
                    ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("history_older"))
                    ui.onNodeWithTag("history_older").performClick()
                    ui.waitUntil(30000) {stockReducer(owner,host,selected)?.let {it.source!=pagingSource ||
                        it.history.any {row->row.id.value==older} || it.nextCursor!=cursor}==true}
                    assertEquals(pagingSource,stockReducer(owner,host,selected)!!.source)
                }
                assertTrue("Native older proof must require the actual UI control",pagingTaps>0)
                assertTrue(stockReducer(owner,host,selected)!!.history.size<=1000)
                ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("history_record_$older"))
                ui.onNodeWithTag("history_record_$older").assertIsDisplayed()
                stockCapture(owner,scenario,"older")
                ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("conversation_refresh"))
                ui.onNodeWithTag("conversation_refresh").performClick()
                ui.waitUntil(30000) {stockReducer(owner,host,selected)?.let {it.source!=null && it.source!=pagingSource && !it.browsingOlder}==true}
                stockAwait(owner,host,selected)
                assertEquals(ConversationKey(host,selected),owner.selection)
                assertFalse(stockReducer(owner,host,selected)!!.browsingOlder)
                assertTrue(stockReducer(owner,host,selected)!!.history.size<=1000)
                }
                val marker="goatr-ui-${run.take(8)}-${provider.name}"
                val (_,receipt)=stockSend(owner,host,selected,"Run the native shell tool printf '%s\\n' '$marker' in THIS existing session, without files/other sessions. Repeat $marker.")
                stockControl("observe","target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),selected),"marker" to JsonPrimitive(marker),
                    "receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt))
                ui.waitUntil(180000) {stockReducer(owner,host,selected)?.history?.any {it.tool!=null}==true}
                val toolRecord=stockReducer(owner,host,selected)!!.history.last {it.tool!=null}
                ui.onNodeWithTag("conversation_list").performScrollToNode(hasTestTag("history_record_${toolRecord.id.value}"))
                ui.onNode(hasTestTag("tool_expand") and hasAnyAncestor(hasTestTag("history_record_${toolRecord.id.value}"))).performScrollTo().performClick()
                ui.onNodeWithTag("history_record_${toolRecord.id.value}").assertIsDisplayed()
                stockCapture(owner,scenario,"tools")
                val desktop=stockControl("desktop","provider" to Protocol.strictJson.encodeToJsonElement(Provider.serializer(),provider)).getValue("marker").jsonPrimitive.content
                ui.waitUntil(30000) {stockReducer(owner,host,selected)?.history?.any {(it.text as? ContentText)?.text?.value?.contains(desktop)==true}==true}
                val kinds=if(provider==Provider.Omp) listOf(PresentationKind.OmpSelect,PresentationKind.OmpEditor) else
                    listOf(PresentationKind.CodexApproval,PresentationKind.CodexQuestions,PresentationKind.CodexForm,PresentationKind.CodexUrl)
                for(kind in kinds) stockVisibleDialog(owner,host,selected,provider,kind)
                stockSend(owner,host,selected,"Run the native shell tool sleep 30, then report done; no files.")
                ui.waitUntil(60000) {stockReducer(owner,host,selected)?.state?.let {it.summary.status==SessionStatus.Working &&
                    (provider==Provider.Omp || it.nativeTurnId!=null)}==true}
                val prior=owner.connection.state.value.latestReceipt[host]?.actionId
                stockTap("turn_stop")
                ui.waitUntil(30000) {owner.connection.state.value.latestReceipt[host]?.let {it.actionId!=prior && it.method==ActionMethod.TurnCancel}==true}
                assertTrue(owner.selection==ConversationKey(host,selected))
                // Rotation, source-bound ordinary unsent memory and visible action remain real.
                scenario.recreate();scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
                stockAwait(owner,host,selected);stockCapture(owner,scenario,"conversation")
                stockTap("connections");stockTap("host_sessions")
                ui.waitUntil(10000) {owner.screen==Screen.Sessions}
            }
            // Every selected provider creates through the visible guarded controls exactly once.
            stockTap("new_session")
            for(row in rows) {
                val provider=stockProvider(row)
                ui.onNode(hasTestTag("create_server") and hasText(info.getValue("serverName").jsonPrimitive.content,substring=true)).performScrollTo().performClick()
                ui.onNode(hasTestTag("create_project") and hasText(info.getValue("projectLabel").jsonPrimitive.content,substring=true)).performScrollTo().performClick()
                ui.onNode(hasTestTag("create_provider") and hasText(provider.name,substring=true)).performScrollTo().performClick()
                ui.onNodeWithTag("create_name").performScrollTo().performTextReplacement("UI owned ${provider.name} ${run.take(8)}")
                ui.onNodeWithTag("session_create").assertIsEnabled();stockTap("session_create")
                ui.waitUntil(10000) {owner.creationActions[host]!=null}
                val action=owner.creationActions.getValue(host);val receipt=stockReceipt(owner,host,action)
                assertTrue("Ready requires actual native binding; partial is not successful creation",receipt.state==ReceiptState.Succeeded)
                stockControl("verify_create","receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt))
                stockCapture(owner,scenario,"creation")
                stockTap("receipt_details_${action.value}")
                ui.onNode(hasTestTag("receipt_exact") and hasText(action.value,substring=true)).performScrollTo().assertIsDisplayed()
                withTimeout(10000) {while(owner.store.localAction(host,action)?.outcomeShown!=true) delay(20)}
                stockTap("create_new_action")
            }
            // Actual crash AFTER a retained native workspace, no response-loss retry/rollback.
            val provider=stockProvider(rows.first())
            ui.onNode(hasTestTag("create_provider") and hasText(provider.name,substring=true)).performScrollTo().performClick()
            ui.onNodeWithTag("create_name").performScrollTo().performTextReplacement("UI partial ${run.take(8)}")
            stockTap("session_create");ui.waitUntil(10000) {owner.creationActions[host]!=null}
            val partialId=owner.creationActions.getValue(host)
            var partial:Receipt?=null
            withTimeout(30000) {while(true) {
                partial=owner.store.localAction(host,partialId)?.receipt
                if(partial?.state==ReceiptState.Dispatching && partial?.creation?.resources?.workspaceId!=null) break
                if(partial?.state in listOf(ReceiptState.Succeeded,ReceiptState.Failed,ReceiptState.Uncertain)) fail("Actual partial fault window was missed; no retry or fabricated uncertainty")
                delay(10)
            }}
            stockControl("fault_create","receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),partial!!))
            ui.waitUntil(10000) {host !in owner.connection.state.value.hello}
            stockCapture(owner,scenario,"offline")
            stockControl("restart")
            stockTap("connections");stockTap("host_reconnect")
            ui.waitUntil(30000) {host in owner.connection.state.value.hello && !owner.busy}
            stockTap("host_sessions")
            ui.runOnIdle {owner.inspect(host,partialId)}
            val uncertain=stockReceipt(owner,host,partialId)
            assertTrue("Real crash leaves actual partial/uncertain receipt, not a fake success",uncertain.state==ReceiptState.Uncertain && uncertain.creation?.resources?.workspaceId!=null)
            stockControl("verify_create","receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),uncertain))
            stockTap("receipt_details_${partialId.value}")
            ui.onNode(hasTestTag("receipt_exact") and hasText(partialId.value,substring=true)).performScrollTo().assertIsDisplayed()
            assertTrue(owner.creationActions[host]==partialId)
            // Real OS process death with an actual committed recovery copy still OFFSCREEN.
            // The runner kills only this owned APK process, then invokes the separate restore
            // consumer; ActivityScenario.recreate is never mislabeled process-death proof.
            val restored=stockControl("live_info").getValue("sessions").jsonArray.first().jsonObject
            val originalTarget=stockTarget(restored)
            ui.onNodeWithTag("session_${originalTarget.sessionId.value}").performScrollTo().performClick()
            stockAwait(owner,host,originalTarget)
            val records=stockReducer(owner,host,originalTarget)!!.history.size
            assertTrue("Actual completed history is required before hiding the receipt header",records>0)
            ui.onNodeWithTag("conversation_list").performScrollToIndex(records)
            val marker="goatr-ui-death-${run.take(8)}"
            ui.onNodeWithTag("composer").performScrollTo().performTextReplacement("Reply only $marker in this exact existing session.")
            ui.waitUntil(10000) {owner.reservation?.key==ConversationKey(host,originalTarget)}
            val original=owner.reservation!!.action
            stockTap("prompt_send")
            ui.waitUntil(10000) {owner.store.state.value.recovery.any {it.action.actionId==original}}
            val copy=owner.store.state.value.recovery.first {it.action.actionId==original}
            assertTrue(copy.action.target==originalTarget && copy.expiresAt.value>System.currentTimeMillis())
            stockControl("process_death","device_id" to JsonPrimitive(device.value),"action_id" to JsonPrimitive(original.value),
                "target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),originalTarget),"expires_at" to JsonPrimitive(copy.expiresAt.value))
            fail("Owned runner must terminate the actual APK process; a surviving test is not process-death evidence")
        } finally {
            withContext(NonCancellable) {
                scenario.moveToState(Lifecycle.State.CREATED);owner.connection.shutdown()
                // Only the fresh installation proved absent at entry is ours to Forget.
                if(owner.store.state.value.hosts.isNotEmpty()) {
                    try {owner.connection.unpair(host)} catch(_:Exception) {}
                    owner.connection.forget(host)
                }
                if(owner.store.state.value.hosts.isEmpty()) owner.connection.forgetInstallation()
                scenario.close()
            }
        }
    }

    // Separate consumer: the controller exposes this only after the entire ordinary gate,
    // including Forget/revoke/shutdown, has passed in the same still-owned helper process.
    @Test fun stock_post_pass_fresh_pair_retains_both_original_sessions_for_feedback()=runBlocking<Unit> {
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Feedback must start after the real Forget gate",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        IrohAndroid.installAndroidContext(context)
        val info=stockControl("live_info")
        val host=info.getValue("host").jsonPrimitive.content
        val rows=info.getValue("sessions").jsonArray.map {it.jsonObject}
        assertEquals(setOf(Provider.Omp,Provider.Codex),rows.map(::stockProvider).toSet())
        assertEquals(2,rows.size)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        var owned:GoatrViewModel?=null
        var retained=false
        try {
            scenario.onActivity {owned=ViewModelProvider(it)[GoatrViewModel::class.java]}
            val owner=owned!!
            ui.waitUntil(20000) {owner.foreground && owner.connection.ownsNetworkCallback()}
            assertTrue(owner.store.state.value.hosts.isEmpty())
            var raw=stockControl("live_pair").getValue("uri").jsonPrimitive.content
            stockTap("pair_begin")
            try {
                ui.onNodeWithTag("pair_paste").performScrollTo().performSemanticsAction(SemanticsActions.SetText) {it(AnnotatedString(raw))}
                stockTap("pair_preview")
            } catch(_:Throwable) {
                ui.runOnIdle {owner.connections()};raw=""
                fail("Private feedback pairing failed; capability diagnostic suppressed")
            }
            raw=""
            ui.waitUntil(10000) {owner.preview?.host?.value==host}
            ui.onNodeWithTag("pair_authority").assertIsDisplayed()
            assertTrue(owner.store.state.value.hosts.isEmpty() && owner.connection.state.value.hello.isEmpty())
            stockTap("pair_confirm")
            ui.waitUntil(30000) {owner.connection.state.value.hello[host]!=null && owner.screen==Screen.Sessions}
            val device=owner.connection.state.value.hello.getValue(host).deviceId
            assertTrue(owner.store.state.value.hosts.single().hostId==host)
            stockControl("feedback_paired","device_id" to JsonPrimitive(device.value))
            val summaries=rows.map {row->
                val target=stockTarget(row)
                ui.onNodeWithTag("session_${target.sessionId.value}").performScrollTo().performClick()
                stockAwait(owner,host,target)
                val summary=stockReducer(owner,host,target)!!.state!!.summary
                assertTrue(summary.provider==stockProvider(row) && summary.nativeId.value==row.getValue("nativeId").jsonPrimitive.content)
                assertTrue(Target(summary.sessionId,summary.generation)==target)
                stockTap("connections");stockTap("host_sessions")
                summary
            }
            assertNull(owner.preview)
            assertTrue(owner.forms.isEmpty())
            // Actual installed artifacts, not just paths to the earlier host build outputs.
            fun apkHash(path:String):String {
                val digest=java.security.MessageDigest.getInstance("SHA-256")
                File(path).inputStream().use {input->
                    val bytes=ByteArray(65536)
                    while(true) {val count=input.read(bytes);if(count<0) break;digest.update(bytes,0,count)}
                    bytes.fill(0)
                }
                return digest.digest().joinToString("") {"%02x".format(it.toInt() and 255)}
            }
            stockControl("feedback_ready","device_id" to JsonPrimitive(device.value),
                "sessions" to JsonArray(summaries.map {Protocol.strictJson.encodeToJsonElement(SessionSummary.serializer(),it)}),
                "apk_sha256" to JsonPrimitive(apkHash(context.applicationInfo.sourceDir)),
                "test_apk_sha256" to JsonPrimitive(apkHash(InstrumentationRegistry.getInstrumentation().context.applicationInfo.sourceDir)))
            retained=true
        } finally {
            withContext(NonCancellable) {
                scenario.moveToState(Lifecycle.State.CREATED)
                owned?.let {owner->
                    owner.connection.shutdown()
                    if(!retained) {
                        if(owner.store.state.value.hosts.any {it.hostId==host}) owner.connection.forget(host)
                        if(owner.store.state.value.hosts.isEmpty()) owner.connection.forgetInstallation()
                    }
                }
                scenario.close()
            }
        }
    }

    @Test fun stock_process_death_restores_original_target_receipt_without_replay_then_forgets()=runBlocking<Unit> {
        val info=stockControl("recovery_info")
        assertTrue("Only this runner's observed prior process-death lease permits existing-key reuse",info.getValue("ownedProcessDied").jsonPrimitive.boolean)
        val host=info.getValue("host").jsonPrimitive.content
        val action=Uuid(info.getValue("actionId").jsonPrimitive.content)
        val originalTarget=Protocol.strictJson.decodeFromJsonElement(Target.serializer(),info.getValue("target"))
        val expires=info.getValue("expiresAt").jsonPrimitive.long
        val device=Uuid(info.getValue("deviceId").jsonPrimitive.content)
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertTrue("Retain the actual protected installation identity, never rekey",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner:GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(30000) {owner.foreground && owner.connection.state.value.hello[host]!=null && !owner.busy}
            assertTrue(owner.store.state.value.hosts.single().hostId==host && owner.store.state.value.hosts.single().deviceId==device)
            assertTrue(owner.connection.state.value.hello.getValue(host).deviceId==device)
            assertNull("Unsent selection/composer is not retargeted by OS restore",owner.selection)
            assertTrue(owner.composers.isEmpty() && owner.forms.isEmpty())
            val original=owner.store.localAction(host,action)!!
            assertTrue(original.metadata.target==originalTarget && original.metadata.actionId==action)
            val copy=owner.store.state.value.recovery.firstOrNull {it.action.actionId==action}
            assertTrue("An unshown pre-send copy keeps exact Target/UUID/deadline through actual process death",copy!=null && copy.action.target==originalTarget && copy.expiresAt.value==expires)
            stockTap("host_settings")
            ui.runOnIdle {owner.inspect(host,action)}
            val receipt=stockReceipt(owner,host,action)
            assertTrue(receipt.actionId==action && receipt.target==originalTarget)
            stockTap("receipt_details_${action.value}")
            ui.onNode(hasTestTag("receipt_exact") and hasText(action.value,substring=true)).performScrollTo().assertIsDisplayed()
            stockControl("verify_no_replay","action_id" to JsonPrimitive(action.value),"target" to Protocol.strictJson.encodeToJsonElement(Target.serializer(),originalTarget),
                "receipt" to Protocol.strictJson.encodeToJsonElement(Receipt.serializer(),receipt))
            stockCapture(owner,scenario,"recovered")
            stockControl("revoke","device_id" to JsonPrimitive(device.value))
            ui.waitUntil(10000) {host !in owner.connection.state.value.hello && !owner.busy}
            ui.onNodeWithTag("host_unpair").assertIsNotEnabled()
            stockTap("host_forget");stockTap("forget_cancel")
            assertTrue(owner.store.state.value.hosts.any {it.hostId==host})
            stockTap("host_forget");stockTap("forget_confirm")
            ui.waitUntil(10000) {owner.store.state.value.hosts.isEmpty()}
            assertTrue(owner.store.state.value.recovery.none {it.hostId==host} && owner.store.state.value.cached.none {it.hostId==host} && owner.store.state.value.keyDeletion.isEmpty())
            stockControl("complete")
        } finally {
            withContext(NonCancellable) {
                scenario.moveToState(Lifecycle.State.CREATED);owner.connection.shutdown()
                if(owner.store.state.value.hosts.any {it.hostId==host}) owner.connection.forget(host)
                if(owner.store.state.value.hosts.isEmpty()) owner.connection.forgetInstallation()
                scenario.close()
            }
        }
    }

    private suspend fun attentionRpcKeepsItsCapturedHello(provider:Provider)=withFixture(provider){fixture->
        val old=fixture.hello
        val current=fixture.connection.reconnect(fixture.host)
        assertNotSame(old,current)
        fixture.connection.snapshot(fixture.host)
        val registration=LocalPushRegistration(1,fixture.host,old.deviceId,Uuid.random(),Uuid.random(),PushHints.encode(ByteArray(32){12}),0,endpoint="https://ntfy.sh/no-dispatch-fixture")
        suspend fun stale(operation:suspend ()->Unit) {
            val before=fixture.connection.pendingReplies(fixture.host,sentOnly=false)
            try{operation();fail("Old authenticated owner reached the native request path")}catch(error:StorePolicy){assertEquals(ErrorCode.NotReady,error.code)}
            assertEquals(before,fixture.connection.pendingReplies(fixture.host,sentOnly=false))
            assertSame(current,fixture.connection.state.value.hello[fixture.host])
        }
        stale{fixture.connection.notificationsSync(fixture.host,old)}
        stale{fixture.connection.markRead(fixture.host,old,fixture.target.sessionId,Uint(1))}
        stale{fixture.connection.registerPush(fixture.host,old,registration)}
        stale{fixture.connection.acknowledgePush(fixture.host,old,registration)}
        stale{fixture.connection.unregisterPush(fixture.host,old,registration.generation)}
        assertNull(fixture.store.encryptedRegistration(fixture.host))
        fixture.connection.notificationsSync(fixture.host,current)
        assertSame(current,fixture.connection.state.value.hello[fixture.host])
    }
    @Test fun review_attention_rpc_captured_hello_omp()=runBlocking{attentionRpcKeepsItsCapturedHello(Provider.Omp)}
    @Test fun review_attention_rpc_captured_hello_codex()=runBlocking{attentionRpcKeepsItsCapturedHello(Provider.Codex)}

    private suspend fun backgroundRegistration(runtime:GoatrRuntime,host:String):LocalPushRegistration? {
        val stored=runtime.store.encryptedRegistration(host) ?: return null
        val plain=runtime.secrets.pushOpen(host,stored.first,stored.second)
        return try {Protocol.strictJson.decodeFromString(LocalPushRegistration.serializer(),plain.toString(Charsets.UTF_8))}
        catch(_:Exception){error("Protected registration metadata unavailable; no private values retained")}
        finally{plain.fill(0)}
    }
    private fun backgroundApkSha(path:String):String {
        val digest=java.security.MessageDigest.getInstance("SHA-256")
        File(path).inputStream().use{input->val bytes=ByteArray(65536);try{while(true){val size=input.read(bytes);if(size<0)break;digest.update(bytes,0,size)}}finally{bytes.fill(0)}}
        return digest.digest().joinToString(""){"%02x".format(it.toInt() and 255)}
    }
    @Test fun stock_background_persistent_real_native_both_providers()=runBlocking<Unit>{stockBackgroundDelivery("persistent")}
    @Test fun stock_background_ntfy_real_native_both_providers()=runBlocking<Unit>{stockBackgroundDelivery("ntfy")}

    private fun stockRenderedConversation(owner:GoatrViewModel,host:String,target:Target) {
        ui.runOnIdle {assertEquals(host,owner.host);assertEquals(ConversationKey(host,target),owner.selection);assertEquals(Screen.Conversation,owner.screen)}
        ui.onNodeWithTag("conversation_list").assertIsDisplayed()
        ui.onNodeWithTag("conversation_target_${target.sessionId.value}_${target.generation.value}",useUnmergedTree=true).assertIsDisplayed()
    }

    /** Router components AFTER a genuine OS tap, never reported as notification-tap acceptance. */
    private suspend fun stockPendingAttentionComponents(scenario:ActivityScenario<MainActivity>,owner:GoatrViewModel,host:String,target:Target,event:Uuid) {
        val runtime=(context as GoatrApplication).runtime
        for(invalidation in listOf("none","navigation","foreground")) {
            ui.runOnIdle {owner.settings(host)}
            val entered=java.util.concurrent.CountDownLatch(1);val release=java.util.concurrent.CountDownLatch(1)
            coroutineScope {
                val barrier=async(Dispatchers.IO){runtime.store.reconcilePresentations(host,{entered.countDown();check(release.await(5,java.util.concurrent.TimeUnit.SECONDS))}){PresentationPolicy(true,true){runtime.suppressesAlert(host,it)}}}
                try {
                    withContext(Dispatchers.IO){check(entered.await(5,java.util.concurrent.TimeUnit.SECONDS))}
                    ui.runOnIdle {owner.refresh()}
                    ui.waitUntil(10000){owner.busy}
                    ui.runOnIdle {
                        owner.openNotification(host,event.value)
                        assertTrue(owner.busy);assertNull(owner.selection)
                        if(invalidation=="navigation")owner.settings(host)
                    }
                    if(invalidation=="foreground")scenario.moveToState(Lifecycle.State.CREATED)
                }finally{release.countDown()}
                barrier.await()
            }
            if(invalidation=="foreground") {
                ui.waitUntil(30000){!owner.foreground&&!owner.busy}
                val previous=withContext(Dispatchers.Main.immediate){owner.viewModelScope.coroutineContext[Job]!!.children.toSet()}
                scenario.moveToState(Lifecycle.State.RESUMED)
                // foreground(true) synchronously installs any resumed route operation before
                // returning. Join that real owner if present, not a status-string observation.
                val resumed=withContext(Dispatchers.Main.immediate){owner.viewModelScope.coroutineContext[Job]!!.children.filter{it !in previous}.toList()}
                withTimeout(30000){resumed.forEach{it.join()}}
            }
            ui.waitUntil(30000){owner.foreground&&!owner.busy}
            if(invalidation=="none") {
                stockAwait(owner,host,target)
                stockRenderedConversation(owner,host,target)
            }else ui.runOnIdle {assertEquals(Screen.Settings,owner.screen);assertEquals(host,owner.host);assertNull(owner.selection)}
        }
        for(loss in listOf("disconnect","replacement")) {
            ui.runOnIdle{owner.settings(host)}
            val captured=owner.connection.state.value.hello.getValue(host)
            val entered=java.util.concurrent.CountDownLatch(1);val release=java.util.concurrent.CountDownLatch(1)
            lateinit var route:Job
            lateinit var previous:Set<Job>
            coroutineScope {
                val barrier=async(Dispatchers.IO){runtime.store.reconcilePresentations(host,{entered.countDown();check(release.await(5,java.util.concurrent.TimeUnit.SECONDS))}){PresentationPolicy(true,true){runtime.suppressesAlert(host,it)}}}
                var retired:Deferred<Unit>?=null
                try {
                    withContext(Dispatchers.IO){check(entered.await(5,java.util.concurrent.TimeUnit.SECONDS))}
                    ui.runOnIdle {
                        previous=owner.viewModelScope.coroutineContext[Job]!!.children.toSet()
                        owner.openNotification(host,event.value)
                        assertTrue(owner.busy);assertNull(owner.selection)
                        route=owner.viewModelScope.coroutineContext[Job]!!.children.first{it !in previous}
                    }
                    // The existing real reply owner proves a read was admitted while SQLite
                    // publication is queued; no production hook or fake source is supplied.
                    withTimeout(3000) {
                        while(owner.connection.pendingReplies(host,sentOnly=false)==0)delay(10)
                        assertSame(captured,owner.connection.state.value.hello[host])
                        retired=this@coroutineScope.async{owner.connection.disconnect(host)}
                        while(owner.connection.state.value.hello[host]===captured)delay(10)
                    }
                }finally{release.countDown()}
                barrier.await()
                // Original retirement publishes before its reader cleanup, without opening
                // Store. Reconnect must wait until this real writer/mutex owner has released.
                val replacing=if(loss=="replacement")async{owner.connection.reconnect(host)}else null
                withTimeout(30000){checkNotNull(retired).await();replacing?.await()?.let{assertNotSame(captured,it)};route.join()}
            }
            // Join any job the original perform-finally actually scheduled. An unconsumed
            // entry must not hide behind a transient idle observation or be canceled by us.
            withTimeout(30000){
                val followups=withContext(Dispatchers.Main.immediate){owner.viewModelScope.coroutineContext[Job]!!.children.filter{it !in previous}.toList()}
                followups.forEach{it.join()}
            }
            ui.runOnIdle{assertEquals(Screen.Settings,owner.screen);assertEquals(host,owner.host);assertNull(owner.selection)}
            val replacement=owner.connection.reconnect(host)
            assertNotSame(captured,replacement)
            owner.connection.snapshot(host)
            ui.runOnIdle{assertEquals(Screen.Settings,owner.screen);assertNull(owner.selection)}
            assertEquals(event,checkNotNull(owner.store.notificationEvent(host,event)).event.eventId)
            // Only a fresh explicit lookup gesture may admit the replacement/current owner.
            ui.runOnIdle{owner.openNotification(host,event.value)}
            stockAwait(owner,host,target)
            stockRenderedConversation(owner,host,target)
        }
    }

    private suspend fun stockBackgroundDelivery(mode:String) {
        val keys=KeyStore.getInstance("AndroidKeyStore").apply{load(null)}
        assertFalse("Only a fresh explicitly owned emulator fixture is admitted",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        assertEquals(34,android.os.Build.VERSION.SDK_INT)
        IrohAndroid.installAndroidContext(context)
        val info=stockControl("live_info")
        assertEquals(mode,info.getValue("background").jsonPrimitive.content)
        assertEquals(info.getValue("appApkSha256").jsonPrimitive.content,backgroundApkSha(context.applicationInfo.sourceDir))
        assertEquals(info.getValue("testApkSha256").jsonPrimitive.content,backgroundApkSha(InstrumentationRegistry.getInstrumentation().context.applicationInfo.sourceDir))
        val host=info.getValue("host").jsonPrimitive.content
        val rows=info.getValue("sessions").jsonArray.map{it.jsonObject}
        assertEquals(2,rows.size)
        assertEquals(setOf(Provider.Omp,Provider.Codex),rows.map(::stockProvider).toSet())
        val runtime=(context as GoatrApplication).runtime
        val manager=checkNotNull(context.getSystemService(android.app.NotificationManager::class.java))
        check(runtime.notifications.health().globalAvailable){"Owned delivery requires already granted notification permission; never skip or bypass a user dialog"}
        if(mode=="ntfy") {
            val packageInfo=context.packageManager.getPackageInfo("io.heckel.ntfy",android.content.pm.PackageManager.PackageInfoFlags.of(android.content.pm.PackageManager.GET_SIGNING_CERTIFICATES.toLong()))
            assertTrue(checkNotNull(packageInfo.applicationInfo).flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE==0)
            val signers=checkNotNull(packageInfo.signingInfo).apkContentsSigners
            assertEquals(1,signers.size)
            val signer=java.security.MessageDigest.getInstance("SHA-256").digest(signers.single().toByteArray()).joinToString(""){"%02x".format(it.toInt() and 255)}
            assertEquals(info.getValue("ntfySignerSha256").jsonPrimitive.content,signer)
            assertEquals(info.getValue("ntfyApkSha256").jsonPrimitive.content,backgroundApkSha(checkNotNull(packageInfo.applicationInfo).sourceDir))
            assertTrue(runtime.push.ntfyInstalled())
        }
        var scenario:ActivityScenario<MainActivity>?=ActivityScenario.launch(MainActivity::class.java)
        var coldActivity:MainActivity?=null
        lateinit var owner:GoatrViewModel
        try {
            scenario!!.onActivity{owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000){owner.foreground&&owner.connection.ownsNetworkCallback()}
            var invitation=stockControl("live_pair").getValue("uri").jsonPrimitive.content
            stockTap("pair_begin")
            try{ui.onNodeWithTag("pair_paste").performScrollTo().performSemanticsAction(SemanticsActions.SetText){it(AnnotatedString(invitation))};stockTap("pair_preview")}
            catch(_:Throwable){ui.runOnIdle{owner.connections()};invitation="";fail("Private background pairing paste failed; no capability retained")}
            invitation=""
            ui.waitUntil(10000){owner.preview?.host?.value==host}
            ui.onNodeWithTag("pair_authority").assertIsDisplayed()
            assertTrue(owner.store.state.value.hosts.isEmpty())
            stockTap("pair_confirm")
            ui.waitUntil(30000){owner.connection.state.value.hello[host]!=null&&!owner.busy}
            val device=owner.connection.state.value.hello.getValue(host).deviceId
            ui.runOnIdle{owner.settings(host)}
            stockTap("delivery_mode_${mode}_$host")
            if(!owner.deliveryEnabled(host))stockTap("delivery_enabled_$host")
            if(!owner.deliveryCompleted(host))stockTap("delivery_completed_$host")
            if(mode=="persistent") {
                stockTap("delivery_start_$host")
                ui.waitUntil(30000){DeliveryService.status.value.running&&owner.connection.state.value.hello[host]!=null}
                assertEquals(DeliveryMode.Persistent,runtime.deliveryPreferences.mode(host))
            } else {
                withTimeout(60000){while(true){
                    val registration=backgroundRegistration(runtime,host)
                    if(owner.connection.state.value.hello[host]!=null&&runtime.push.verified(host)&&registration?.let{it.device==device&&it.probeReceived&&it.acknowledged&&!it.repair&&!it.unregister}==true)break
                    delay(50)
                }}
                assertEquals(DeliveryMode.Ntfy,runtime.deliveryPreferences.mode(host))
            }
            val prefix="goatr.attention.$host."
            val seen=manager.activeNotifications.filter{it.tag?.startsWith(prefix)==true}.mapTo(mutableSetOf()){it.tag}
            var delivered=0
            for((providerIndex,row) in rows.withIndex()) {
                val target=stockTarget(row);val provider=stockProvider(row);val native=row.getValue("nativeId").jsonPrimitive.content
                for(category in listOf(AttentionCategory.Completed,AttentionCategory.Attention)) {
                    if(mode=="ntfy")ui.waitUntil(30000){owner.connection.state.value.hello[host]!=null&&runtime.push.verified(host)}
                    val beforeActions=owner.store.localActions(host,null,100).map{it.metadata.actionId}.toSet()
                    ui.runOnIdle{owner.open(host,target)}
                    stockAwait(owner,host,target)
                    stockRenderedConversation(owner,host,target)
                    OwnedUiAutomationRunner.backgroundOwnedDeliveryConversation(InstrumentationRegistry.getInstrumentation(),target.sessionId.value,target.generation.value)
                    val cold=providerIndex==rows.lastIndex&&category==AttentionCategory.Attention
                    if(cold){scenario!!.close();scenario=null}
                    ui.waitUntil(30000){!owner.foreground}
                    if(mode=="ntfy") {
                        ui.waitUntil(30000){owner.connection.state.value.hello[host]==null&&!DeliveryService.status.value.running}
                        assertTrue(runtime.push.verified(host))
                    } else assertTrue("Persistent delivery retains its actual keeper claim",DeliveryService.status.value.running)
                    val kind=if(provider==Provider.Omp)PresentationKind.OmpSelect else PresentationKind.CodexForm
                    val trigger=if(category==AttentionCategory.Completed)stockControl("desktop","provider" to Protocol.strictJson.encodeToJsonElement(Provider.serializer(),provider))
                        else stockControl("trigger_dialog","provider" to Protocol.strictJson.encodeToJsonElement(Provider.serializer(),provider),"kind" to Protocol.strictJson.encodeToJsonElement(PresentationKind.serializer(),kind))
                    assertEquals(target,Protocol.strictJson.decodeFromJsonElement(Target.serializer(),trigger.getValue("target")))
                    var alert:android.service.notification.StatusBarNotification?=null
                    withTimeout(180000){while(alert==null){alert=manager.activeNotifications.firstOrNull{it.tag?.startsWith(prefix)==true&&it.tag !in seen};if(alert==null)delay(50)}}
                    val actual=checkNotNull(alert);seen+=actual.tag
                    val eventId=Uuid(actual.tag.removePrefix(prefix))
                    assertEquals(1,manager.activeNotifications.count{it.tag==actual.tag})
                    val monitor=if(cold)InstrumentationRegistry.getInstrumentation().addMonitor(MainActivity::class.java.name,null,false)else null
                    try {
                        OwnedUiAutomationRunner.tapOwnedDeliveryNotification(InstrumentationRegistry.getInstrumentation(),manager,actual,host,eventId.value)
                        if(monitor!=null) {
                            coldActivity=withContext(Dispatchers.IO){monitor.waitForActivityWithTimeout(15000)} as? MainActivity
                            val opened=checkNotNull(coldActivity){"The native notification-row click did not create MainActivity"}
                            InstrumentationRegistry.getInstrumentation().runOnMainSync{owner=ViewModelProvider(opened)[GoatrViewModel::class.java]}
                        }
                    }finally{monitor?.let{InstrumentationRegistry.getInstrumentation().removeMonitor(it)}}
                    ui.waitUntil(30000){owner.foreground&&owner.selection==ConversationKey(host,target)&&owner.connection.state.value.mutationReady[host]==true&&!owner.busy}
                    stockRenderedConversation(owner,host,target)
                    val authenticated=checkNotNull(owner.store.notificationEvent(host,eventId)).event
                    assertEquals(category,authenticated.category)
                    assertEquals(target,Target(authenticated.sessionId,authenticated.generation))
                    assertEquals(native,stockReducer(owner,host,target)!!.state!!.summary.nativeId.value)
                    assertEquals("Notification entry must not send, answer, or create any action",beforeActions,owner.store.localActions(host,null,100).map{it.metadata.actionId}.toSet())
                    val hello=owner.connection.state.value.hello.getValue(host)
                    owner.connection.notificationsSync(host,hello)
                    assertEquals(1,owner.store.cachedNotifications(host).events.count{it.eventId==eventId})
                    owner.viewed(ConversationKey(host,target))
                    withTimeout(10000){while(manager.activeNotifications.any{it.tag==actual.tag})delay(20)}
                    if(category==AttentionCategory.Attention) {
                        val request=Uuid(trigger.getValue("requestKey").jsonPrimitive.content)
                        assertTrue("A tap never answers the native request",stockReducer(owner,host,target)!!.pending.any{it.requestKey==request})
                        stockAnswerExistingDialog(owner,host,target,request,kind)
                    }
                    if(providerIndex==0&&category==AttentionCategory.Completed)stockPendingAttentionComponents(checkNotNull(scenario),owner,host,target,eventId)
                    assertEquals(if(mode=="ntfy")DeliveryMode.Ntfy else DeliveryMode.Persistent,runtime.deliveryPreferences.mode(host))
                    delivered++
                }
            }
            assertEquals(4,delivered)
            stockControl("complete")
        }finally {
            // Normal UI release only. Preserve pairing, registration, receipts, keeper choice
            // and all native originals for Main-reviewed inspection/cleanup and other scenarios.
            scenario?.moveToState(Lifecycle.State.CREATED);scenario?.close()
            coldActivity?.let{activity->InstrumentationRegistry.getInstrumentation().runOnMainSync{activity.finish()}}
        }
    }
}

/** Pure runner admission/key regressions. This class deliberately has no Compose rule or Activity launch. */
class SdkForegroundBootstrapContractTest {
    @Test fun exact_main_once_selector_requires_authenticated_control_and_prepares_p256_material() {
        fun arguments(mode:String,method:String,port:String?=null)=Bundle().apply {
            putString("goatrSdkForegroundBootstrap",mode);putString("class",method);port?.let {putString("stockControlPort",it)}
        }
        val main="io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_private_pairing_checkpoint"
        val component="io.github.trevarj.goatr.NativeInventoryTest#directoryRowKeepsStatusAndDetailsContainedAtCompactTwoHundredPercent"
        val selected=OwnedInstrumentationRunner.sdkFixtureSelection(arguments("main_once",main,"41029"))
        assertNotNull(selected);assertTrue(selected!!.stockControl);assertEquals("io.github.trevarj.goatr.MainActivity",selected.activityClass)
        assertNull(OwnedInstrumentationRunner.sdkFixtureSelection(arguments("main_once",main)))
        assertNull(OwnedInstrumentationRunner.sdkFixtureSelection(arguments("main_once",component,"41029")))
        assertNull(OwnedInstrumentationRunner.sdkFixtureSelection(arguments("main_once",main,"0")))
        val retirement="io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_owned_retirement_checkpoint"
        assertNull(OwnedInstrumentationRunner.sdkFixtureSelection(arguments("main_once",retirement,"41029")))
        assertNotNull(OwnedInstrumentationRunner.sdkFixtureSelection(arguments("main_once",retirement,"41029").apply {putString("goatrOwnedRetirementCheckpoint","true")}))
        assertNull(OwnedInstrumentationRunner.sdkFixtureSelection(arguments("unknown",main,"41029")))
        val componentSelection=OwnedInstrumentationRunner.sdkFixtureSelection(arguments("component_once",component))
        assertNotNull(componentSelection);assertFalse(componentSelection!!.stockControl)
        val material=OwnedInstrumentationRunner.freshStockControlKey()
        assertEquals(65,material.publicIdentity.size);assertEquals(4,material.publicIdentity[0].toInt())
        val message="GOATR-STOCK-CONTROL-v1".toByteArray(Charsets.US_ASCII)
        val signature=java.security.Signature.getInstance("SHA256withECDSA").run {initSign(material.pair.private);update(message);sign()}
        assertTrue(java.security.Signature.getInstance("SHA256withECDSA").run {initVerify(material.pair.public);update(message);verify(signature)})
        message.fill(0);signature.fill(0)
    }
}
