// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.ui.graphics.Color
import org.junit.Assert.*
import androidx.compose.runtime.mutableStateOf
import kotlinx.coroutines.Job
import org.junit.Test

class TerminalGridTest {
    private fun cell(text:String="x",width:Long=1,attributes:GridAttributes=GridAttributes(false,false,false,false,false,GridUnderline.None),foreground:GridColor=GridColor.Default,background:GridColor=GridColor.Default)=
        GridCell(text,Uint(width),foreground,background,attributes).also {it.validate()}
    private fun frame(revision:Long,popup:Boolean)=GridBegin(GridMode.Replace,Uint(revision),Uint(2),Uint(1),null,false,false,Uint(1),popup)
    @Test fun rawInverseResolvesOriginalDefaultRolesBeforeSwappingAndNeverBrightensBold() {
        val normal=cell(attributes=GridAttributes(true,false,false,false,false,GridUnderline.None))
        val inverse=cell(attributes=normal.attributes.copy(inverse=true))
        for((foreground,background) in listOf(Color.White to Color.Black,Color.Black to Color.White)) {
            assertEquals(foreground,resolvedGridForeground(normal,foreground,background))
            assertEquals(background,resolvedGridBackground(normal,foreground,background))
            assertEquals(background,resolvedGridForeground(inverse,foreground,background))
            assertEquals(foreground,resolvedGridBackground(inverse,foreground,background))
        }
        val named=cell(attributes=normal.attributes,foreground=GridColor.Indexed(Uint(1)))
        val plain=cell(attributes=named.attributes.copy(bold=false),foreground=named.foreground)
        assertEquals(resolvedGridForeground(named,Color.White,Color.Black),resolvedGridForeground(plain,Color.White,Color.Black))
    }
    @Test fun concealedGlyphsAndDecorationsStayUndrawableAcrossThemesAndBlinkChanges() {
        val hidden=cell(attributes=GridAttributes(true,false,true,true,true,GridUnderline.Double,inverse=true,concealed=true))
        val grid=TerminalGrid(frame(0,false),listOf(hidden,hidden),true)
        val viewport=TerminalGridViewport(2,1,0f,0f,10f,20f,20f,20f)
        var backgrounds=0;var glyphs=0
        viewport.backgrounds(grid){_,_,_->backgrounds++}
        viewport.glyphs(grid){_,_,_,_->glyphs++}
        assertEquals(2,backgrounds);assertEquals(0,glyphs);assertFalse(viewport.needsBlink(grid))
        assertNotEquals(resolvedGridBackground(hidden,Color.White,Color.Black),resolvedGridBackground(hidden,Color.Black,Color.White))
        viewport.glyphs(grid){_,_,_,_->fail("Hidden glyph/decorations may never reach the painter")}
    }
    @Test fun popupStateChangesOnlyWithCompleteMatchingReplacementsAndWidePairsRemainCanonical() {
        val reducer=TerminalGridReducer();val subscription=Uuid.random()
        val lead=cell("界",2);val tail=cell("",0)
        reducer.open(frame(0,true),1)
        assertFalse(reducer.event(Event(subscription,Uint(1),EventBody.GridPart(GridPart(Uint(0),Uint(0),listOf(GridSpan(Uint(0),Uint(0),listOf(lead,tail)))))),1))
        assertNull(reducer.committed)
        assertTrue(reducer.event(Event(subscription,Uint(2),EventBody.GridCommit(GridCommit(Uint(0)))),1))
        assertTrue(reducer.committed!!.begin.popupBlocked)
        reducer.event(Event(subscription,Uint(3),EventBody.GridBegin(GridBeginEvent(frame(1,false)))),1)
        assertTrue(reducer.committed!!.begin.popupBlocked)
        reducer.event(Event(subscription,Uint(4),EventBody.GridPart(GridPart(Uint(1),Uint(0),listOf(GridSpan(Uint(0),Uint(0),listOf(cell("a"),cell("b"))))))),1)
        reducer.event(Event(subscription,Uint(5),EventBody.GridCommit(GridCommit(Uint(1)))),1)
        assertFalse(reducer.committed!!.begin.popupBlocked)
        assertEquals(listOf("a","b"),reducer.committed!!.cells.map {it.text})
        reducer.clear();assertNull(reducer.committed)
    }
    private fun shellSnapshot():ShellSnapshot {
        val id=Uuid.random();val selection=ShellSelection("desktop",ShellTarget(id,id))
        val summary=ShellSummary(selection.target,id,Id("w"),Id("t"),Id("p"),Id("n"),Label("Original terminal"),SessionConnectivity.Ready,ShellCapabilities(true,false,false,false,grid=true))
        return ShellSnapshot(selection,Uuid.random(),1,summary,TerminalGrid(frame(0,false),listOf(cell("a"),cell("b")),false),false,false,controlBegun=true)
    }
    @Test fun retiredGridClearAndErrorCompletionCannotReopenWaitingInEitherOrder() {
        for(errorFirst in listOf(true,false)) {
            val shown=shellSnapshot();val state=mutableStateOf<ShellSnapshot?>(shown);val lease=TerminalGridLease(Job())
            lease.close()
            fun error(){state.value=state.value!!.copy(grid=null,loading=false,error=true)}
            fun clear(){publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,lease,null)}
            if(errorFirst){error();clear()}else{clear();error()}
            assertNull(state.value!!.grid);assertFalse(state.value!!.loading);assertTrue(state.value!!.error)
            publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,lease,shown.grid)
            assertNull(state.value!!.grid);assertFalse(state.value!!.loading);assertTrue(state.value!!.error)
        }
    }
    @Test fun activeFreshnessPauseMayWaitButCannotEraseAnErrorOrChangeReleasedReplacement() {
        val shown=shellSnapshot();val state=mutableStateOf<ShellSnapshot?>(shown);val lease=TerminalGridLease(Job())
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,false,lease,null)
        assertNull(state.value!!.grid);assertTrue(state.value!!.loading);assertFalse(state.value!!.error)
        state.value=state.value!!.copy(loading=false,error=true)
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,false,lease,null)
        assertFalse(state.value!!.loading);assertTrue(state.value!!.error)
        lease.close()
        val released=shown.copy(grid=null,loading=false,error=false,controlBegun=false)
        state.value=released
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,false,lease,null)
        assertEquals(released,state.value)
        val replacement=released.copy(ticket=shown.ticket+1)
        state.value=replacement
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,false,lease,null)
        assertEquals(replacement,state.value)
    }
    @Test fun negotiatedAgentLongGraphemeAndCursorStylesCommitAtomicallyWhileShellRetains128() {
        val glyph="\uD834\uDD1E"+"\uD834\uDD65".repeat(64)
        assertEquals(260,utf8Size(glyph))
        val subscription=Uuid.random()
        val first=frame(0,false).copy(graphicsOmitted=true,cursor=GridCursor(Uint(0),Uint(0),true,Uint(2)))
        val part=Event(subscription,Uint(1),EventBody.GridPart(GridPart(Uint(0),Uint(0),listOf(GridSpan(Uint(0),Uint(0),listOf(cell(glyph),cell(" ")))))))
        val shell=TerminalGridReducer();shell.open(first,1)
        assertTrue(runCatching {shell.event(part,1)}.isFailure)
        val observer=TerminalGridReducer(ansiObserver=true);observer.open(first,1)
        assertFalse(observer.event(part,1));assertNull(observer.committed)
        observer.event(Event(subscription,Uint(2),EventBody.GridCommit(GridCommit(Uint(0)))),1)
        assertEquals(glyph,observer.committed!!.cells[0].text);assertEquals(Uint(2),observer.committed!!.begin.cursor!!.shape)
        val next=first.copy(revision=Uint(1),cursor=GridCursor(Uint(1),Uint(0),false,Uint(6)))
        observer.event(Event(subscription,Uint(3),EventBody.GridBegin(GridBeginEvent(next))),1)
        val attrs=GridAttributes(true,false,true,false,true,GridUnderline.Curly)
        observer.event(Event(subscription,Uint(4),EventBody.GridPart(GridPart(Uint(1),Uint(0),listOf(GridSpan(Uint(0),Uint(0),listOf(cell("界",2,attrs),cell("",0,attrs))))))),1)
        assertEquals(glyph,observer.committed!!.cells[0].text);assertEquals(Uint(2),observer.committed!!.begin.cursor!!.shape)
        observer.event(Event(subscription,Uint(5),EventBody.GridCommit(GridCommit(Uint(1)))),1)
        assertEquals("界",observer.committed!!.cells[0].text);assertEquals(attrs,observer.committed!!.cells[0].attributes)
        assertEquals(Uint(6),observer.committed!!.begin.cursor!!.shape);assertFalse(observer.committed!!.begin.cursor!!.visible)
        observer.clear();assertNull(observer.committed)
        assertTrue(runCatching {TerminalGridReducer(true).open(first.copy(columns=Uint(512),rows=Uint(17)),1)}.isFailure)
        assertTrue(runCatching {TerminalGridReducer(true).open(first.copy(alternateScreen=true),1)}.isFailure)
        assertTrue(runCatching {TerminalGridReducer(true).open(first,8*1024*1024+1)}.isFailure)
        assertTrue(runCatching {cell(glyph+"x")}.isFailure)
    }
    @Test fun agentButtonRequiresCurrentHelloFeatureRouteAndReadySourceNotLegacyCapability() {
        val id=Uuid.random();val target=Target(id,id)
        val summary=SessionSummary(id,id,id,Id("w"),Id("n"),Id("p"),Provider.Omp,Id("native"),"Agent",SessionStatus.Idle,SessionConnectivity.Ready,
            Capabilities(true,true,true,true,PromptMode.SubmitOrSteer,CancelMode.CurrentWork),
            HistoryCoverage(HistoryScope.AllBranches,HistorySource.PersistedAndLive,false,false,false))
        val pane=NativePane(Id("p"),Id("n"),Id("w"),Id("t"),focused=false,agentStatus=NativeAgentStatus.Idle,target=target)
        val old=HelloFeatures(agentAnsi=false)
        val current=HelloFeatures(agentAnsi=true)
        assertFalse(summary.capabilities.terminalRead)
        assertFalse(readOnlyAgentTerminal(old,summary,pane,true))
        assertFalse(readOnlyAgentTerminal(old,summary.copy(capabilities=summary.capabilities.copy(terminalRead=true)),pane,true))
        assertTrue(readOnlyAgentTerminal(current,summary,pane,true))
        assertFalse(readOnlyAgentTerminal(current,summary,pane,false))
        assertFalse(readOnlyAgentTerminal(current,summary.copy(connectivity=SessionConnectivity.Synchronizing),pane,true))
        assertFalse(readOnlyAgentTerminal(current,summary,pane.copy(terminalId=Id("replacement")),true))
        assertFalse(readOnlyAgentTerminal(current,summary,pane.copy(target=Target(id,Uuid.random())),true))
        assertFalse(readOnlyAgentTerminal(null,summary,pane,true))
    }
    private fun historyGrid(offset:Long?,revision:Long=0,alternate:Boolean=false)=TerminalGrid(frame(revision,false).copy(
        alternateScreen=alternate,scroll=offset?.let {GridScroll(Uint(it),Uint(20),Uint(1))}),listOf(cell(),cell()),false)
    private fun scrollReceipt(action:Uuid,offset:Long=0,state:ReceiptState=ReceiptState.Succeeded)=Receipt(
        actionId=action,method=ActionMethod.ShellScroll,bootId=Uuid.random(),shellTarget=ShellTarget(Uuid.random(),Uuid.random()),
        createdAt=Uint(1),updatedAt=Uint(2),state=state,
        result=if(state==ReceiptState.Succeeded)ActionResultShellScroll(Uint(offset),Uint(20),Uint(1))else null)
    @Test fun negotiatedShellHistoryMetricsCommitWithCellsAndNeverReachAgentOrOldReducer() {
        val begin=historyGrid(0).begin;val id=Uuid.random()
        for(reducer in listOf(TerminalGridReducer(),TerminalGridReducer(ansiObserver=true,history=true)))assertTrue(runCatching {reducer.open(begin,1)}.isFailure)
        val reducer=TerminalGridReducer(history=true);reducer.open(begin,1)
        fun cells(seq:Long,revision:Long)=Event(id,Uint(seq),EventBody.GridPart(GridPart(Uint(revision),Uint(0),listOf(GridSpan(Uint(0),Uint(0),listOf(cell(),cell()))))))
        assertFalse(reducer.event(cells(1,0),1));assertNull(reducer.committed)
        assertTrue(reducer.event(Event(id,Uint(2),EventBody.GridCommit(GridCommit(Uint(0)))),1))
        assertEquals(0L,reducer.committed!!.begin.scroll!!.offsetFromBottom.value)
        val changed=begin.copy(revision=Uint(1),scroll=GridScroll(Uint(7),Uint(20),Uint(1)))
        assertFalse(reducer.event(Event(id,Uint(3),EventBody.GridBegin(GridBeginEvent(changed))),1))
        assertEquals(0L,reducer.committed!!.begin.scroll!!.offsetFromBottom.value)
        assertFalse(reducer.event(cells(4,1),1));assertEquals(0L,reducer.committed!!.begin.scroll!!.offsetFromBottom.value)
        assertTrue(reducer.event(Event(id,Uint(5),EventBody.GridCommit(GridCommit(Uint(1)))),1))
        assertEquals(7L,reducer.committed!!.begin.scroll!!.offsetFromBottom.value)
        for(metrics in listOf(GridScroll(Uint(2),Uint(1),Uint(1)),GridScroll(Uint(0),Uint(1),Uint(0)),GridScroll(Uint(0),Uint(1),Uint(257))))assertTrue(runCatching {metrics.validate()}.isFailure)
        assertTrue(runCatching {TerminalGridReducer(history=true).open(begin.copy(alternateScreen=true),1)}.isFailure)
        assertTrue(runCatching {TerminalGridReducer(history=true).open(begin.copy(popupBlocked=true),1)}.isFailure)
        TerminalGridReducer(history=true).open(begin.copy(alternateScreen=true,scroll=null),1)
    }
    @Test fun explicitBottomNeedsSameLeaseAckAndCurrentCommitEitherOrderIncludingNoop() {
        for(ackFirst in listOf(false,true)) {
            val history=ShellHistoryState();val lease=TerminalGridLease(Job());history.committed(historyGrid(7),lease)
            history.committed(historyGrid(0),lease);assertEquals(ShellHistoryMode.Browsing,history.mode)
            history.committed(historyGrid(null,alternate=true),lease);assertEquals(ShellHistoryMode.Browsing,history.mode)
            history.committed(historyGrid(7),lease);val id=Uuid.random();history.begin(id,0)
            if(ackFirst){history.receipt(scrollReceipt(id));assertEquals(ShellHistoryMode.Browsing,history.mode);history.committed(historyGrid(0),lease)}
            else {history.committed(historyGrid(0),lease);assertEquals(ShellHistoryMode.Pending,history.mode);history.receipt(scrollReceipt(id))}
            assertEquals(ShellHistoryMode.Live,history.mode)
            history.committed(historyGrid(7),lease);history.committed(historyGrid(0),lease)
            history.receipt(scrollReceipt(id));assertEquals("An old duplicate ACK must not re-arm return-to-bottom",ShellHistoryMode.Browsing,history.mode)
            val noOp=Uuid.random();history.begin(noOp,0);history.receipt(scrollReceipt(noOp))
            assertEquals("Already committed bottom needs no later revision/frame",ShellHistoryMode.Live,history.mode)
            val uncertain=Uuid.random();history.begin(uncertain,5);history.uncertain()
            history.committed(historyGrid(null),lease);history.committed(historyGrid(0),lease);assertEquals(ShellHistoryMode.Uncertain,history.mode)
            val recover=Uuid.random();history.begin(recover,0)
            history.receipt(scrollReceipt(uncertain));assertEquals(ShellHistoryMode.Pending,history.mode)
            history.receipt(scrollReceipt(recover));assertEquals(ShellHistoryMode.Live,history.mode)
            val clamped=Uuid.random();history.begin(clamped,999);history.receipt(scrollReceipt(clamped))
            assertEquals("Older clamped to zero is not explicit bottom",ShellHistoryMode.Browsing,history.mode)
        }
    }
    @Test fun retiredOrReplacedHistoryGridCannotPublishBottomOrResumeInput() {
        val shown=shellSnapshot();val state=mutableStateOf<ShellSnapshot?>(shown);val lease=TerminalGridLease(Job())
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,lease,historyGrid(7));lease.close()
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,lease,historyGrid(0))
        assertEquals(7L,shown.history.metrics!!.offsetFromBottom.value);assertEquals(ShellHistoryMode.Browsing,shown.history.mode)
        val replacement=shown.copy(ticket=shown.ticket+1,history=ShellHistoryState());state.value=replacement
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,TerminalGridLease(Job()),historyGrid(0))
        assertSame(replacement,state.value);assertNull(replacement.history.metrics)
    }
    @Test fun lateBottomAckCannotUseARetiredCurrentlyBottomGrid() {
        val shown=shellSnapshot();val state=mutableStateOf<ShellSnapshot?>(shown);val lease=TerminalGridLease(Job())
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,lease,historyGrid(7))
        val action=Uuid.random();shown.history.begin(action,0)
        publishShellGrid(state,shown.selection,shown.source,shown.ticket,true,lease,historyGrid(0))
        assertEquals(ShellHistoryMode.Pending,shown.history.mode)
        lease.close();shown.history.receipt(scrollReceipt(action))
        assertEquals(ShellHistoryMode.Browsing,shown.history.mode);assertFalse(shown.history.active())
    }
    @Test fun shellHistorySubscriptionBuildersPreserveLegacyCanonicalAndWireBytes() {
        val id=Uuid.random();val target=ShellTarget(id,id)
        val old=Request(id,RequestBody.ShellGridSubscribe(ShellGridSubscribeParams(target)))
        val explicit=Request(id,RequestBody.ShellGridSubscribe(ShellGridSubscribeParams(target,history=false)))
        assertArrayEquals(Protocol.canonicalRequest(old),Protocol.canonicalRequest(explicit))
        val bytes=Protocol.encodePayload(Envelope.RequestFrame(explicit))
        assertFalse(bytes.toString(Charsets.UTF_8).contains("\"history\""))
        val enabled=Protocol.encodePayload(Envelope.RequestFrame(Request(id,RequestBody.ShellGridSubscribe(ShellGridSubscribeParams(target,history=true)))))
        assertTrue(enabled.toString(Charsets.UTF_8).contains("\"history\":true"))
    }
}
