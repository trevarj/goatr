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
}
