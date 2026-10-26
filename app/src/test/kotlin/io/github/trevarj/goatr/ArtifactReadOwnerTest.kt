// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class ArtifactReadOwnerTest {
    private fun owner(source:ArtifactSource=ArtifactSource.Session(Target(Uuid.random(),Uuid.random())))=
        ArtifactBrowserState("original-host",source,ArtifactReadContext(Any()),Screen.Conversation,null,Any())
    private fun entry()=ArtifactEntry(Uuid.random(),"owned-file",ArtifactKind.File,Uint(3))
    private fun page(shown:ArtifactBrowserState,entries:List<ArtifactEntry>,next:Uuid?=null)=
        ArtifactsListResult(shown.source,entries,next)
    @Test fun pagination_keeps_original_source_and_rejects_duplicate_or_nonprogressing_pages_atomically() {
        val shown=owner();val first=entry();val cursor=Uuid.random()
        assertTrue(shown.publishPage(shown.ticket,page(shown,listOf(first),cursor),null,emptyList(),false))
        val original=shown.entries
        fun refuse(result:ArtifactsListResult,expected:ErrorCode) {
            val failure=runCatching {shown.publishPage(shown.ticket,result,cursor,emptyList(),true)}.exceptionOrNull() as? StorePolicy
            assertEquals(expected,failure?.code);assertSame(original,shown.entries)
        }
        refuse(page(shown,listOf(first)),ErrorCode.InvalidRequest)
        refuse(page(shown,listOf(entry()),cursor),ErrorCode.InvalidRequest)
        assertTrue(shown.publishPage(shown.ticket,page(shown,listOf(entry())),cursor,emptyList(),true))
        assertEquals(2,shown.entries.size);assertNull(shown.cursor)
    }
    @Test fun foreign_source_pages_cannot_retarget_the_original_breadcrumbs() {
        val shown=owner()
        shown.publishPage(shown.ticket,page(shown,listOf(entry())),null,emptyList(),false)
        val before=shown.entries
        val foreign=ArtifactsListResult(ArtifactSource.Session(Target(Uuid.random(),Uuid.random())),listOf(entry()))
        assertEquals(ErrorCode.StaleSession,(runCatching {shown.publishPage(shown.ticket,foreign,null,emptyList(),false)}.exceptionOrNull() as? StorePolicy)?.code)
        assertSame(before,shown.entries);assertTrue(shown.breadcrumbs.isEmpty())
        val child=Uuid.random();val path=listOf(child to "child")
        assertTrue(shown.publishPage(shown.ticket,page(shown,listOf(entry())),null,path,false));assertEquals(path,shown.breadcrumbs)
    }
    @Test fun real_late_page_continuation_cannot_revive_a_retired_or_replaced_original()=runBlocking {
        withTimeout(5000) {
            for(retire in listOf(false,true)) coroutineScope {
                val shown=owner();val originalTicket=shown.ticket;val waiting=CompletableDeferred<Unit>();val response=CompletableDeferred<ArtifactsListResult>()
                val consumer=async {waiting.complete(Unit);shown.publishPage(originalTicket,response.await(),null,emptyList(),false)}
                waiting.await();if(retire)shown.context.invalidate() else shown.ticket=Any()
                val replacement=owner(shown.source);val retained=entry()
                replacement.publishPage(replacement.ticket,page(replacement,listOf(retained)),null,emptyList(),false)
                response.complete(page(shown,listOf(entry())))
                assertFalse(consumer.await());assertTrue(shown.entries.isEmpty());assertTrue(shown.breadcrumbs.isEmpty())
                assertEquals(listOf(retained),replacement.entries);assertTrue(replacement.context.active.get())
            }
        }
    }
    @Test fun published_pages_keep_the_existing_one_thousand_entry_memory_bound() {
        val shown=owner()
        repeat(10) {index->val previous=shown.cursor;val next=Uuid.random();val rows=List(100){entry()}
            assertTrue(shown.publishPage(shown.ticket,page(shown,rows,next),previous,emptyList(),index!=0))
        }
        val before=shown.entries
        assertEquals(ErrorCode.ResourceLimit,(runCatching {shown.publishPage(shown.ticket,page(shown,listOf(entry())),shown.cursor,emptyList(),true)}.exceptionOrNull() as? StorePolicy)?.code)
        assertSame(before,shown.entries);assertEquals(1000,shown.entries.size)
    }
}
