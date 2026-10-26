// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import org.junit.Assert.*
import org.junit.Test

class ConversationActivityTest {
    private fun completedTool()=HistoryRecord(Id("native-tool"),order=Uint(1),role=Role.Tool,kind=RecordKind.Tool,tool=Tool("Native tool",status=ToolStatus.Succeeded),source=RecordSource.Live,complete=true,sourceTruncated=false)

    @Test fun onlyCompletedNativeSuccessToolOnlyRecordsFold() {
        val record=completedTool()
        assertTrue(isRoutineToolRecord(record,null))
        val output=ContentText(InlineText("Native successful OMP output"))
        assertTrue(isRoutineToolRecord(record.copy(text=output,tool=record.tool!!.copy(output=output)),null))
        assertFalse(isRoutineToolRecord(record.copy(text=output,tool=record.tool!!.copy(output=ContentText(InlineText("Other native text")))),null))
        ToolStatus.entries.filter {it!=ToolStatus.Succeeded}.forEach {status->assertFalse(isRoutineToolRecord(record.copy(tool=record.tool!!.copy(status=status)),null))}
        assertFalse(isRoutineToolRecord(record.copy(complete=false),null))
        assertFalse(isRoutineToolRecord(record,record.id))
        Role.entries.filter {it!=Role.Tool}.forEach {role->assertFalse(isRoutineToolRecord(record.copy(role=role),null))}
        RecordKind.entries.filter {it!=RecordKind.Tool}.forEach {kind->assertFalse(isRoutineToolRecord(record.copy(kind=kind),null))}
    }

    @Test fun textTruncationAndUnavailableContentStayVisible() {
        val record=completedTool()
        assertFalse(isRoutineToolRecord(record.copy(sourceTruncated=true),null))
        assertFalse(isRoutineToolRecord(record.copy(text=ContentText(InlineText("Native notice"))),null))
        val unavailable=ContentUnavailable(ContentUnavailableReason.NotCached)
        assertFalse(isRoutineToolRecord(record.copy(tool=record.tool!!.copy(output=unavailable)),null))
        assertFalse(isRoutineToolRecord(record.copy(tool=record.tool!!.copy(input=unavailable)),null))
        assertFalse(isRoutineToolRecord(record.copy(original=unavailable),null))
        assertFalse(isRoutineToolRecord(record.copy(tool=null),null))
    }

    private fun reader()=ConversationReaderPosition(ConversationReaderIdentity(Uuid.random(),Uuid.random(),Provider.Omp,Id("original-native")))
    private fun message(id:String,order:Long)=completedTool().copy(id=Id(id),order=Uint(order),role=Role.Assistant,kind=RecordKind.Message,tool=null)

    @Test fun originalReaderReentryResolvesStableNativeRecordRatherThanOldIndex() {
        val position=reader();val original=position.attach()
        assertTrue(position.capture(original,"history_b",20,-9,false,true,false,Id("c")))
        position.toolsExpanded=true
        position.release(original)
        val reconnect=position.attach()
        val headExpanded=listOf(message("earlier",0),message("a",10),message("b",20),message("c",30))
        assertEquals(3,position.locate(headExpanded,null,emptyList()))
        assertEquals(9,position.anchor!!.offset)
        assertFalse(position.following);assertTrue(position.toolsExpanded)
        assertFalse(position.capture(original,"history_c",30,0,true,true,false,Id("c")))
        assertEquals("history_b",position.anchor!!.row)
        assertTrue(position.current(reconnect))
        // A receipt becoming focused changes display indices but not this record's identity.
        assertEquals(4,position.locate(headExpanded,Uuid.random(),emptyList()))
    }

    @Test fun missingWindowAnchorStaysTruthfulUntilAnExplicitUserReadingGesture() {
        val position=reader();val ticket=position.attach()
        position.capture(ticket,"history_b",20,-11,false,true,false,Id("c"))
        val expired=listOf(message("c",30),message("d",40))
        assertEquals(1,position.locate(expired,null,emptyList()))
        assertTrue(position.anchorMissing);assertFalse(position.following)
        assertFalse(position.capture(ticket,"history_d",40,0,true,false,false,Id("d")))
        assertEquals("history_b",position.anchor!!.row)
        assertEquals(1,position.locate(expired,null,emptyList()))
        assertTrue(position.capture(ticket,"history_c",30,0,false,true,false,Id("d")))
        assertFalse(position.anchorMissing);assertFalse(position.following)
        position.latest(Id("d"))
        assertTrue(position.following);assertEquals(Id("d"),position.readTail)
    }

    @Test fun windowShrinkAndOfflineLateLayoutsCannotSilentlyTurnFollowingBackOn() {
        val position=reader();val ticket=position.attach()
        position.capture(ticket,"history_b",20,-3,false,true,false,Id("c"))
        assertTrue(position.capture(ticket,"history_b",20,0,true,false,false,Id("c")))
        assertFalse(position.following)
        val offline=position.attach()
        assertFalse(position.capture(ticket,"history_c",30,0,true,true,false,Id("c")))
        assertEquals("history_b",position.anchor!!.row)
        assertTrue(position.capture(offline,"history_b",20,-5,false,false,false,Id("c")))
        assertEquals(5,position.anchor!!.offset)
        val reconnect=position.attach()
        assertTrue(position.capture(reconnect,"history_c",30,0,true,true,true,Id("c")))
        assertFalse(position.following) // Reaching an older page's end is not Return to latest.
        position.release(ticket)
        assertTrue(position.current(reconnect))
        position.release(reconnect)
        assertFalse(position.current(reconnect))
    }

    @Test fun receiptAnchorsMoveBetweenFocusedAndHistoricalSlotsWithoutRetargeting() {
        val position=reader();val ticket=position.attach();val receipt=Uuid.random()
        position.capture(ticket,"receipt_${receipt.value}",null,-4,false,true,false,null)
        val history=listOf(message("a",10))
        assertEquals(1,position.locate(history,receipt,emptyList()))
        assertEquals(3,position.locate(history,null,listOf(receipt)))
        assertEquals(4,position.anchor!!.offset)
        position.locate(history,null,emptyList())
        assertTrue(position.anchorMissing);assertFalse(position.following)
    }

    @Test fun routeFiltersRetainSearchButClearAllAndForgetHaveDistinctLocalLifetimes() {
        val filters=SessionListFilters()
        filters.host="a";filters.query="native label search";filters.provider=Provider.Codex;filters.status=SessionStatus.Attention
        filters.retainHosts(setOf("a","b"))
        assertEquals("a",filters.host);assertEquals("native label search",filters.query)
        filters.clearFilters()
        assertNull(filters.host);assertNull(filters.provider);assertNull(filters.status)
        assertEquals("native label search",filters.query)
        filters.host="b";filters.forgetHost("a")
        assertEquals("b",filters.host);assertEquals("",filters.query)
        filters.query="removed host search";filters.retainHosts(setOf("a"))
        assertNull(filters.host);assertEquals("",filters.query)
    }

    @Test fun olderHistoryExpansionIsAnEntryTransitionAndRetainsTheUserCollapse() {
        val position=reader()
        position.observeHistoryView(true)
        assertTrue(position.toolsExpanded)
        position.toolsExpanded=false
        position.observeHistoryView(true)
        assertFalse(position.toolsExpanded)
        position.observeHistoryView(false)
        assertFalse(position.toolsExpanded)
        position.observeHistoryView(true)
        assertTrue(position.toolsExpanded)
    }
}
