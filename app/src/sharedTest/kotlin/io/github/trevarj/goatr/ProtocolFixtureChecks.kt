// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.FilterInputStream
import java.nio.ByteBuffer
import java.util.Base64
import kotlinx.serialization.*
import kotlinx.serialization.json.*
import org.junit.Assert.*

internal class ProtocolFixtureChecks(text: String) {
    private val corpus = Json.parseToJsonElement(text).jsonObject
    private fun JsonElement.text(key: String) = jsonObject.getValue(key).jsonPrimitive.content
    private fun JsonElement.flag(key: String) = jsonObject.getValue(key).jsonPrimitive.boolean
    private fun input(v: JsonElement): ByteArray = v.jsonObject["base64"]?.let { Base64.getDecoder().decode(it.jsonPrimitive.content) } ?: v.text("wire").toByteArray(Charsets.UTF_8)
    private fun method(v: JsonElement): Method? = v.jsonObject["responseMethod"]?.let { Json.decodeFromJsonElement<Method>(it) }
    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
    private fun unhex(text: String) = text.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    private fun cases(key: String) = corpus.getValue(key).jsonArray

    fun envelopes(includeNative: Boolean) {
        for (case in cases("frames")) {
            if (!includeNative && case.jsonObject["requiresNative"]?.jsonPrimitive?.boolean == true) continue
            val result = runCatching { Protocol.decodePayload(input(case),method(case)) }
            if (case.text("expected") == "accept") {
                val decoded = result.getOrElse { throw AssertionError(case.text("name"),it) }
                if (decoded is Envelope.RequestFrame && "canonical" in case.jsonObject) {
                    val bytes = Protocol.canonicalRequest(decoded.request)
                    assertArrayEquals(case.text("name"),case.text("canonical").toByteArray(Charsets.UTF_8),bytes)
                    assertEquals(case.text("sha256"),hex(Protocol.sha256(bytes)))
                    assertEquals(case.text("hmac"),hex(Protocol.hmacSha256(ByteArray(32) { 11 },bytes)))
                    assertTrue(Protocol.verifyHmacSha256(ByteArray(32) { 11 },bytes,Protocol.hmacSha256(ByteArray(32) { 11 },bytes)))
                    assertFalse(Protocol.verifyHmacSha256(ByteArray(32) { 12 },bytes,Protocol.hmacSha256(ByteArray(32) { 11 },bytes)))
                }
                val encoded = Protocol.encodePayload(decoded)
                assertArrayEquals(case.text("name"),encoded,Protocol.encodePayload(Protocol.decodePayload(encoded,method(case))))
            } else {
                val failure = result.exceptionOrNull() as? ProtocolFailure ?: throw AssertionError(case.text("name") + ": expected protocol failure",result.exceptionOrNull())
                assertEquals(case.text("name"),case.text("expected"),failure.kind.name)
                assertEquals(failure.kind != FailureKind.UnsupportedMethod,failure.close)
                if ("requestId" in case.jsonObject) assertEquals(case.text("name"),case.jsonObject.getValue("requestId").let { if (it == JsonNull) null else it.jsonPrimitive.content },failure.requestId?.value)
            }
        }
        for (case in cases("canonical")) {
            val bytes = Protocol.canonicalJson(Protocol.parseRawJson(input(case)))
            assertArrayEquals(case.text("canonical").toByteArray(Charsets.UTF_8),bytes)
            assertEquals(bytes.size.toLong(),Protocol.canonicalJsonSize(Protocol.parseRawJson(input(case))))
            assertEquals(case.text("sha256"),hex(Protocol.sha256(bytes)))
            assertEquals(case.text("hmac"),hex(Protocol.hmacSha256(ByteArray(32) { 11 },bytes)))
        }
        for (case in cases("hmacVectors")) assertEquals(case.text("hmac"),hex(Protocol.hmacSha256(unhex(case.text("keyHex")),case.text("bytes").toByteArray(Charsets.UTF_8))))
    }
    fun boundaries() {
        for (case in cases("raw")) {
            val parsed = runCatching { Protocol.parseRawJson(input(case)) }
            assertEquals(case.text("name"),case.flag("accept"),parsed.isSuccess)
            case.jsonObject["canonical"]?.let { expected ->
                assertTrue(case.text("name"),parsed.getOrThrow() is JsonObject)
                assertArrayEquals(expected.jsonPrimitive.content.toByteArray(Charsets.UTF_8),Protocol.canonicalJson(parsed.getOrThrow()))
            }
        }
        for (case in cases("rawBounds")) {
            val n = case.jsonObject.getValue("count").jsonPrimitive.int
            val text = when(case.text("kind")) {
                "depth" -> "[".repeat(n) + "0" + "]".repeat(n)
                "members" -> "{\"x\":[" + List(n) { "0" }.joinToString(",") + "]}"
                "frame" -> "{\"x\":\"" + "x".repeat(n-8) + "\"}"
                else -> throw AssertionError("unknown fixture bound")
            }
            assertEquals(case.toString(),case.flag("accept"),runCatching { Protocol.parseRawJson(text.toByteArray(Charsets.UTF_8)) }.isSuccess)
        }
        fun replace(root: JsonElement, path: List<JsonElement>, value: JsonElement): JsonElement {
            if (path.isEmpty()) return value
            val key = path.first().jsonPrimitive
            return if (key.isString) { val objectValue = root.jsonObject.toMutableMap(); objectValue[key.content] = replace(objectValue.getValue(key.content),path.drop(1),value); JsonObject(objectValue) }
            else { val xs = root.jsonArray.toMutableList(); xs[key.int] = replace(xs[key.int],path.drop(1),value); JsonArray(xs) }
        }
        for (case in cases("wireBounds")) {
            val n = case.jsonObject.getValue("count").jsonPrimitive.int
            val value = if (case.text("generator") == "text") JsonPrimitive(case.text("unit").repeat(n)) else JsonArray(List(n) { index ->
                val unit = case.jsonObject.getValue("unit")
                case.jsonObject["identityField"]?.jsonPrimitive?.content?.let { field ->
                    JsonObject(unit.jsonObject + (field to JsonPrimitive("11111111-1111-4111-8111-" + index.toString(16).padStart(12,'0'))))
                } ?: unit
            })
            val wire = replace(Json.parseToJsonElement(case.text("baseWire")),case.jsonObject.getValue("path").jsonArray,value).toString().toByteArray(Charsets.UTF_8)
            assertEquals(case.text("name"),case.flag("accept"),runCatching { Protocol.decodePayload(wire,method(case)) }.isSuccess)
        }
        val id = Uuid("ce8ae66d-3345-4810-9052-0363cb356172")
        for ((count, accepted) in listOf(Protocol.JSON_MEMBERS_MAX / 2 to true, Protocol.JSON_MEMBERS_MAX to false)) {
            val envelope = Envelope.EventFrame(Event(id, Uint(1), EventBody.RequestOpened(RequestOpenedEvent(
                PendingRequest(id, Provider.Omp, PresentationKind.OmpSelect, PresentationOmpSelect(
                    ContentText(InlineText("Choose")), listOf(SelectOption("a", Label("A"))),
                    selectionMarker = SelectionMarker.Radio,
                    checked = List(count) { Uint(0) },
                )),
            ))))
            val encoded = runCatching { Protocol.encodePayload(envelope) }
            assertEquals(accepted, encoded.isSuccess)
            if (accepted) assertEquals(envelope, Protocol.decodePayload(encoded.getOrThrow()))
        }
    }
    fun answersAndFormats() {
        for (case in cases("answers")) {
            val accepted = runCatching { validateAnswer(Protocol.decodePresentation(case.text("presentation").toByteArray(Charsets.UTF_8)),Protocol.decodeAnswer(case.text("answer").toByteArray(Charsets.UTF_8))) }
            assertEquals(case.text("name"),case.flag("accept"),accepted.isSuccess)
        }
        for (case in cases("formats")) assertEquals(case.toString(),case.flag("accept"),validFormat(Json.decodeFromJsonElement(case.jsonObject.getValue("format")),case.text("input")))
    }
    fun decimalsAndNativeTokens() {
        for ((key,native) in listOf("decimals" to false,"nativeDecimals" to true)) for (case in cases(key)) {
            val parsed = runCatching { if (native) Decimal.fromNativeToken(case.text("input")) else Decimal.parse(case.text("input")) }
            val expected = case.jsonObject.getValue("normalized")
            if (expected == JsonNull) assertTrue(case.toString(),parsed.isFailure) else { val decimal = parsed.getOrThrow(); assertEquals(expected.jsonPrimitive.content,decimal.value); assertEquals(decimal.value,decimal.nativeJsonNumber().toString()) }
        }
        for (case in cases("comparisons")) assertEquals(case.toString(),case.jsonObject.getValue("order").jsonPrimitive.int,Decimal.parse(case.text("a")).compareTo(Decimal.parse(case.text("b"))).compareTo(0))
        for (raw in cases("nativeIds")) { val text = raw.jsonPrimitive.content; val id = Json.parseToJsonElement(text).jsonPrimitive; assertEquals(text,id.toString()); assertEquals(text.startsWith('"'),id.isString) }
    }
    fun framing() {
        val payload = "{\"unicode\":\"😀 é\"}".toByteArray(Charsets.UTF_8)
        val output = ByteArrayOutputStream(); Protocol.writeFrame(output,payload); val encoded=output.toByteArray()
        val fragmented = object : FilterInputStream(ByteArrayInputStream(encoded)) { override fun read(b: ByteArray, off: Int, len: Int): Int = super.read(b,off,minOf(1,len)) }
        assertArrayEquals(payload,Protocol.readFrame(fragmented))
        assertTrue(runCatching { Protocol.readFrame(ByteArrayInputStream(encoded.copyOf(encoded.size-1))) }.isFailure)
        for (length in listOf(0,Protocol.FRAME_MAX+1,-1)) assertTrue(runCatching { Protocol.readFrame(ByteArrayInputStream(ByteBuffer.allocate(4).putInt(length).array())) }.isFailure)
        assertEquals(Protocol.FRAME_MAX,Protocol.frameLength(ByteBuffer.allocate(4).putInt(Protocol.FRAME_MAX).array()))
    }
    fun contextsAndCanonicalExpansion() {
        val id = Uuid.random(); val target = Target(Uuid.random(),Uuid.random())
        for (case in corpus.getValue("contextChecks").jsonObject.getValue("cancel").jsonArray) {
            val params = TurnCancelParams(id,target,case.jsonObject.getValue("nativeTurnId").let { if (it == JsonNull) null else Id(it.jsonPrimitive.content) })
            assertEquals(case.flag("accept"),runCatching { validateTurnCancel(Json.decodeFromJsonElement(case.jsonObject.getValue("provider")),params) }.isSuccess)
        }
        for (case in corpus.getValue("contextChecks").jsonObject.getValue("contentRead").jsonArray) {
            val params = ContentReadParams(target,id,Uint(case.jsonObject.getValue("offset").jsonPrimitive.long),Uint(1))
            assertEquals(case.flag("accept"),runCatching { validateContentRead(params,Uint(case.jsonObject.getValue("length").jsonPrimitive.long)) }.isSuccess)
        }
        assertEquals(4,java.util.UUID.fromString(id.value).version())
        for (body in listOf(
            RequestBody.PromptSubmit(PromptSubmitParams(id,target,PromptText("\n".repeat(Protocol.ANSWER_MAX)))),
            RequestBody.RequestRespond(RequestRespondParams(id,target,Uuid.random(),AnswerOmpEditor(SubmitAction.Submit,PromptText("\n".repeat(Protocol.ANSWER_MAX)))))
        )) {
            val request = Request(id,body); val encoded = Protocol.encodePayload(Envelope.RequestFrame(request))
            assertTrue(encoded.size <= Protocol.FRAME_MAX)
            assertTrue(Protocol.canonicalRequest(request).size > Protocol.FRAME_MAX)
            assertArrayEquals(encoded,Protocol.encodePayload(Protocol.decodePayload(encoded)))
        }
    }
    fun hierarchyRecoveryKeepsIndependentPhoneSelection() {
        val frame=cases("frames").first { it.text("name") == "native hierarchy shells unsupported empty order and exact associations" }
        val inventory=((Protocol.decodePayload(input(frame),Method.HostSnapshot) as Envelope.ResponseFrame).response.result as ResultBody.HostSnapshot).result
        val summary=inventory.sessions.single()
        val selected=Target(summary.sessionId,summary.generation)
        val conversation=SessionReducer()
        conversation.snapshot(SessionOpenResult(Uuid.random(),ZeroValue(0),SessionState(summary,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle)),emptyList(),emptyList()))
        conversation.recoveryReconciled();conversation.composer(selected,"independent phone draft")
        val host=HostReducer();host.snapshot(inventory)
        assertTrue(host.ready)
        assertTrue(host.sourceCurrent(summary.serverId))
        assertEquals(SessionReducer.Decision.Resnapshot,host.event(Event(inventory.subscription,Uint(1),EventBody.HostChanged(HostChangedEvent()))))
        assertFalse(host.ready)
        assertFalse(host.sourceCurrent(summary.serverId))
        assertSame(inventory,host.inventory)
        val movedFocus=inventory.copy(subscription=Uuid.random(),hierarchy=inventory.hierarchy.map { h ->
            val focused=h.panes.first { it.terminalId == Id("unsupported-terminal") }
            h.copy(workspaces=h.workspaces.map { it.copy(focused=it.workspaceId == focused.workspaceId) },
                tabs=h.tabs.map { it.copy(focused=it.tabId == focused.tabId) },
                panes=h.panes.map { it.copy(focused=it.paneId == focused.paneId) })
        })
        host.snapshot(movedFocus)
        assertTrue(host.ready)
        assertEquals(selected,conversation.state!!.summary.let { Target(it.sessionId,it.generation) })
        assertEquals("independent phone draft",conversation.composer(selected))
        assertEquals(DesktopSelection.Unknown,conversation.state!!.desktopSelection)
        val stale=movedFocus.copy(hierarchy=movedFocus.hierarchy.map { h ->
            h.copy(freshness=NativeFreshness.Stale,reason=NativeUnavailableReason.SourceUnavailable,
                panes=h.panes.map { it.copy(target=null,unboundRowId=null) })
        })
        host.snapshot(stale)
        assertTrue(host.ready)
        assertFalse(host.sourceCurrent(summary.serverId))
        val overflow=movedFocus.copy(hierarchy=movedFocus.hierarchy.map {
            it.copy(freshness=NativeFreshness.Unavailable,reason=NativeUnavailableReason.TooLarge,
                workspaces=emptyList(),tabs=emptyList(),panes=emptyList())
        })
        host.snapshot(overflow)
        assertTrue(host.sourceCurrent(summary.serverId))
        host.disconnected()
        assertFalse(host.ready)
        assertSame(overflow,host.inventory)
    }
    fun browseViewsRejectStalePagesAndPreserveTheirOrderDomain() {
        val frame = cases("frames").first { it.text("name") == "native hierarchy stale overflow remains source unavailable" }
        val inventory = ((Protocol.decodePayload(input(frame),Method.HostSnapshot) as Envelope.ResponseFrame).response.result as ResultBody.HostSnapshot).result
        val summary = inventory.sessions.single().copy(provider=Provider.Codex)
        val target = Target(summary.sessionId,summary.generation)
        val subscription = Uuid.random()
        fun row(id: String, order: Long, text: String, source: RecordSource) = HistoryRecord(
            id=Id(id), order=Uint(order), role=Role.Assistant, kind=RecordKind.Message,
            text=ContentText(InlineText(text)), source=source, complete=true, sourceTruncated=false,
        )
        val live = row("same",3,"new live fields",RecordSource.Live)
        val snapshot = SessionOpenResult(subscription,ZeroValue(0),SessionState(summary,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle)),emptyList(),listOf(live))
        val reducer = SessionReducer(); reducer.snapshot(snapshot)
        val stale = reducer.historyPage(target)
        val root = reducer.historyPage(target)
        val view = Uuid.random()
        val page = SessionHistoryResult(listOf(row("older",800,"older",RecordSource.NativePage),row("same",900,"native fields",RecordSource.NativePage)),view,Id("continue"))
        assertTrue(runCatching { reducer.page(stale,page) }.isFailure)
        reducer.page(root,page)
        assertEquals(listOf(800L,900L),reducer.history.map { it.order.value })
        assertEquals("new live fields",(reducer.history.last().text as ContentText).text.value)
        val next = reducer.historyPage(target,Id("continue"))
        assertTrue(runCatching { reducer.page(next,SessionHistoryResult(emptyList(),Uuid.random())) }.isFailure)
        assertEquals(SessionReducer.Decision.Applied,reducer.event(Event(subscription,Uint(1),EventBody.HistoryUpsert(HistoryUpsertEvent(listOf(row("late",4,"late completion",RecordSource.Live),live.copy(text=ContentText(InlineText("updated")))))))))
        assertEquals(listOf("older","same"),reducer.history.map { it.id.value })
        assertEquals(listOf(800L,900L),reducer.history.map { it.order.value })
        reducer.page(next,SessionHistoryResult(listOf(row("oldest",700,"oldest",RecordSource.NativePage)),view))
        assertEquals(listOf(700L,800L,900L),reducer.history.map { it.order.value })
        reducer.snapshot(snapshot.copy(subscription=Uuid.random()))
        assertFalse(reducer.browsingOlder)
        assertNull(reducer.viewId)
        assertNull(reducer.nextCursor)
        assertTrue(runCatching { reducer.page(next,page) }.isFailure)
        assertEquals(listOf(live),reducer.history)
        // A full display window retains the peer continuation for a later page.
        val capped = SessionReducer(); capped.snapshot(snapshot)
        val cappedView = Uuid.random()
        for (batch in 0..9) {
            val cursor = if (batch == 0) null else Id("cap-$batch")
            val ticket = capped.historyPage(target,cursor)
            val rows = (0..99).map { n -> row("cap-$batch-$n",(1000-batch*100+n).toLong(),"native",RecordSource.NativePage) }
            capped.page(ticket,SessionHistoryResult(rows,cappedView,Id("cap-"+(batch+1))))
        }
        assertEquals(1000,capped.history.size)
        assertEquals(Id("cap-10"),capped.nextCursor)
    }
    fun hierarchyOverflowKeepsSourceUnavailability() {
        val frame=cases("frames").first { it.text("name") == "native hierarchy stale overflow remains source unavailable" }
        val inventory=((Protocol.decodePayload(input(frame),Method.HostSnapshot) as Envelope.ResponseFrame).response.result as ResultBody.HostSnapshot).result
        val summary=inventory.sessions.single()
        val graph=inventory.hierarchy.single()
        assertEquals(ServerConnectivity.Online,inventory.servers.single().connectivity)
        assertEquals(NativeFreshness.Unavailable,graph.freshness)
        assertEquals(NativeUnavailableReason.SourceUnavailable,graph.reason)
        assertTrue(graph.workspaces.isEmpty() && graph.tabs.isEmpty() && graph.panes.isEmpty())
        val conversation=SessionReducer()
        conversation.snapshot(SessionOpenResult(Uuid.random(),ZeroValue(0),SessionState(summary,desktopSelection=DesktopSelection.Unknown,activity=Activity(ActivityState.Idle)),emptyList(),emptyList()))
        conversation.recoveryReconciled()
        assertTrue(conversation.mutationsEnabled)
        val host=HostReducer();host.snapshot(inventory)
        assertTrue(host.ready)
        assertFalse(host.sourceCurrent(summary.serverId))

        val currentFrame=cases("frames").first { it.text("name") == "native hierarchy unavailable is not empty native truth" }
        val current=((Protocol.decodePayload(input(currentFrame),Method.HostSnapshot) as Envelope.ResponseFrame).response.result as ResultBody.HostSnapshot).result
        assertEquals(NativeUnavailableReason.TooLarge,current.hierarchy.single().reason)
        assertEquals(summary.capabilities,current.sessions.single().capabilities)
        host.snapshot(current)
        assertTrue(host.ready)
        assertTrue(host.sourceCurrent(summary.serverId))
        assertTrue(conversation.mutationsEnabled)
    }
}
