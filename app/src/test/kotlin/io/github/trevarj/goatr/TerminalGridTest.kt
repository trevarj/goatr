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
}
