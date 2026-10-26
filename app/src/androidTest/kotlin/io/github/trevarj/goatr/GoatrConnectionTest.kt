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
import android.os.Bundle
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.AnnotatedString

/** Real Android endpoint/GoatrConnection/SQLite/AndroidKeyStore against smoke --review-fixture.
 * The host uses actual Rust State/Security/transport, with synthetic provider-data transitions.
 * No echo server, mocked transport, production host, installed debug command or provider effect. */
class GoatrConnectionTest {
    @get:Rule val ui=createEmptyComposeRule()
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
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
        check(runtime.deliveryPreferences.canPostNotifications()){"Owned delivery requires already granted notification permission; never skip or bypass a user dialog"}
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
