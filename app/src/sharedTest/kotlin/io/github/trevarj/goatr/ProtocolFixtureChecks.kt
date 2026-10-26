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
import kotlinx.coroutines.*

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
                val catalogue=((decoded as? Envelope.ResponseFrame)?.response?.result as? ResultBody.SkillsList)?.result
                if(catalogue!=null) {
                    val rows=Protocol.parseRawJson(input(case)).jsonObject.getValue("result").jsonObject.getValue("skills").jsonArray
                    val expected=rows.map {it.jsonObject["invocationRef"]?.jsonPrimitive?.content}
                    assertEquals(case.text("name")+": retain opaque native invocation references",expected,catalogue.skills.map {it.invocationRef?.value})
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
    fun gridCommitOwnsPixelsAndRejectsUncertainBoundaries() {
        val id=Uuid.random()
        val begin=GridBegin(GridMode.Replace,Uint(0),Uint(2),Uint(1),alternateScreen=false,graphicsOmitted=false,partCount=Uint(2))
        val attrs=GridAttributes(false,false,false,false,false,GridUnderline.None)
        fun cell(text:String,width:Long)=GridCell(text,Uint(width),GridColor.Default,GridColor.Default,attrs)
        fun part(seq:Long,index:Long,column:Long,cell:GridCell)=Event(id,Uint(seq),EventBody.GridPart(GridPart(Uint(0),Uint(index),listOf(GridSpan(Uint(0),Uint(column),listOf(cell))))))
        val reducer=TerminalGridReducer();reducer.open(begin,32)
        val deadline=reducer.transactionDeadline
        assertNull(reducer.committed)
        val left=part(1,0,0,cell("界",2))
        assertFalse(reducer.event(left,64));assertNull(reducer.committed)
        assertFalse(reducer.event(left,64))
        assertEquals(deadline,reducer.transactionDeadline)
        assertFalse(reducer.event(part(2,1,1,cell("",0)),64));assertNull(reducer.committed)
        assertTrue(reducer.event(Event(id,Uint(3),EventBody.GridCommit(GridCommit(Uint(0)))),32))
        assertEquals("界",reducer.committed!!.cells[0].text)
        assertEquals(0L,reducer.transactionDeadline)
        reducer.clear();assertNull(reducer.committed)
        reducer.open(begin,32)
        assertTrue(runCatching {reducer.event(part(2,0,0,cell("x",1)),64)}.isFailure)
        reducer.clear();reducer.open(begin,32)
        assertTrue(reducer.expired(Long.MAX_VALUE));reducer.clear();assertNull(reducer.committed)
        val initial=cases("frames").first {it.text("name")=="result terminal.grid.subscribe"}
        val parsed=(Protocol.decodePayload(input(initial),Method.TerminalGridSubscribe) as Envelope.ResponseFrame).response.result as ResultBody.TerminalGridSubscribe
        assertEquals(0L,parsed.result.frame.revision.value)
        val shell=ShellSummary(ShellTarget(id,id),id,Id("w"),Id("t"),Id("p"),Id("n"),Label("Shell"),SessionConnectivity.Ready,ShellCapabilities(true,true,true,false,grid=true))
        assertTrue(shell.sameLogicalIdentity(shell.copy(capabilities=shell.capabilities.copy(grid=false))))
    }
    private fun completeGrid(columns:Int,rows:Int,wide:Boolean=false,blinkColumn:Int?=null):TerminalGrid {
        val begin=GridBegin(GridMode.Replace,Uint(0),Uint(columns.toLong()),Uint(rows.toLong()),alternateScreen=false,graphicsOmitted=false,partCount=Uint(rows.toLong()))
        val cells=List(columns*rows){index->
            val row=index/columns;val column=index%columns
            val width=if(wide && row==0 && column==0)2L else if(wide && row==0 && column==1)0L else 1L
            GridCell(if(width==0L)"" else if(width==2L)"界" else "x",Uint(width),GridColor.Default,GridColor.Default,GridAttributes(false,false,false,row==0 && column==blinkColumn,false,GridUnderline.None))
        }
        return TerminalGrid(begin,cells,cells.any {it.width.value!=0L && it.attributes.blink && !it.attributes.concealed})
    }
    fun gridViewportUsesWideExtentAndOnlyVisibleBlinkingContent() {
        val grid=completeGrid(512,64,wide=true)
        val viewport=TerminalGridViewport(512,64,-15f,0f,10f,20f,10f,20f)
        var backgrounds=0;viewport.backgrounds(grid){_,_,_->backgrounds++}
        assertEquals(2,backgrounds)
        val glyphs=mutableListOf<String>()
        viewport.glyphs(grid){cell,x,_,right->
            glyphs+=cell.text
            if(cell.width.value==2L){assertEquals(-15f,x,0f);assertEquals(5f,right,0f)}
        }
        assertEquals(listOf("界","x"),glyphs)
        assertFalse(viewport.needsBlink(grid))
        val blinking=completeGrid(512,1,wide=true,blinkColumn=100)
        assertFalse(viewport.needsBlink(blinking))
        val visible=TerminalGridViewport(512,1,-995f,0f,10f,20f,10f,20f)
        assertTrue(visible.needsBlink(blinking))
        val outside=TerminalGridViewport(512,1,20f,0f,10f,20f,10f,20f)
        assertFalse(outside.needsBlink(blinking))
    }
    /** Android-only invocation: real Main dispatcher barrier through the actual production consumers. */
    suspend fun gridMainDeliveryCannotResurrectDismissedOrReplacedViewers()=coroutineScope {
        val outer=this
        val frame=completeGrid(2,1,wide=true);val source=Uuid.random()
        val key=ConversationKey("fixture",Target(Uuid.random(),Uuid.random()))
        for(mode in listOf("dismiss","background","replace","source")){
            val caller=Job();val lease=TerminalGridLease(caller);var foreground=true;var currentSource=true
            val (queued,state)=withContext(Dispatchers.Main.immediate){
                val state=androidx.compose.runtime.mutableStateOf<TerminalSnapshot?>(TerminalSnapshot(key,source,1,null,true,false))
                lease.stage(frame)
                val produced=java.util.concurrent.CountDownLatch(1)
                val queued=outer.async(Dispatchers.IO){
                    produced.countDown()
                    lease.deliver {grid,actual->publishTerminalGrid(state,key,source,1,foreground,key,Screen.Conversation,currentSource,actual,grid)}
                }
                assertTrue(produced.await(1,java.util.concurrent.TimeUnit.SECONDS))
                when(mode){
                    "dismiss"->{state.value=null;caller.cancel()}
                    "background"->foreground=false
                    "source"->currentSource=false
                    else->state.value=TerminalSnapshot(key,source,2,null,true,false)
                }
                queued to state
            }
            queued.await()
            if(mode=="dismiss")assertNull(state.value) else assertNull(state.value!!.grid)
            if(mode=="replace")assertEquals(2L,state.value!!.ticket)
            lease.close();caller.cancel()
        }
        val shellTarget=ShellTarget(Uuid.random(),Uuid.random());val selected=ShellSelection("fixture",shellTarget)
        val summary=ShellSummary(shellTarget,Uuid.random(),Id("w"),Id("t"),Id("p"),Id("n"),Label("Shell"),SessionConnectivity.Ready,ShellCapabilities(true,false,true,false,grid=true))
        val caller=Job();val lease=TerminalGridLease(caller)
        val (queued,state)=withContext(Dispatchers.Main.immediate){
            val state=androidx.compose.runtime.mutableStateOf<ShellSnapshot?>(ShellSnapshot(selected,source,1,summary,null,true,false))
            lease.stage(frame);val produced=java.util.concurrent.CountDownLatch(1)
            val queued=outer.async(Dispatchers.IO){produced.countDown();lease.deliver {grid,actual->publishShellGrid(state,selected,source,1,true,actual,grid)}}
            assertTrue(produced.await(1,java.util.concurrent.TimeUnit.SECONDS))
            state.value=ShellSnapshot(selected,source,2,summary,null,true,false)
            queued to state
        }
        queued.await();assertEquals(2L,state.value!!.ticket);assertNull(state.value!!.grid)
        lease.close()
        lease.clear {grid,actual->publishShellGrid(state,selected,source,1,true,actual,grid)}
        assertEquals(2L,state.value!!.ticket);assertNull(state.value!!.grid);caller.cancel()
    }
}
