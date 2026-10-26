// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class CreationDirectoryLeaseTest {
    private fun context(current:AtomicBoolean)=CreationDirectoriesContext(
        Any(),AgentSessionLease(),Uuid.random(),Uuid.random(),Long.MAX_VALUE,
    ) {current.get()}

    @Test fun late_directory_pages_cannot_publish_into_dismissed_or_replacement_views()=runBlocking {
        withTimeout(5000) {
            for(replaced in listOf(false,true)) coroutineScope {
                val current=AtomicBoolean(true)
                val original=context(current)
                val waiting=CompletableDeferred<Unit>()
                val response=CompletableDeferred<CreationDirectoriesResult>()
                val visible=AtomicReference<CreationDirectoriesResult?>(null)
                val consumer=async(Dispatchers.Default) {runCatching {
                    waiting.complete(Unit)
                    val page=response.await()
                    original.admitView {
                        original.references.addAll(page.entries.map {it.reference})
                        visible.set(page)
                    }
                }}
                waiting.await()
                if(replaced)current.set(false) else original.lease.invalidate()
                val replacement=context(AtomicBoolean(true))
                val retained=Uuid.random()
                replacement.admitView {replacement.references.add(retained)}
                response.complete(CreationDirectoriesResult(original.server,original.project,
                    entries=listOf(CreationDirectoryEntry(Uuid.random(),Label("actual directory label")))))
                val failure=consumer.await().exceptionOrNull() as? StorePolicy
                assertEquals(if(replaced)ErrorCode.StaleSession else ErrorCode.NotReady,failure?.code)
                assertNull(visible.get())
                assertTrue(original.references.isEmpty())
                assertEquals(setOf(retained),replacement.references)
            }
        }
    }

    @Test fun closing_directory_view_does_not_cancel_an_already_admitted_original_continuation()=runBlocking {
        withTimeout(5000) {
            val current=AtomicBoolean(true)
            val original=context(current)
            val admitted=CompletableDeferred<Unit>()
            val finish=CompletableDeferred<Unit>()
            val continuation=async(Dispatchers.Default) {
                original.admitView {admitted.complete(Unit)}
                finish.await()
                "original completion"
            }
            admitted.await()
            original.lease.invalidate();current.set(false)
            assertTrue(continuation.isActive)
            finish.complete(Unit)
            assertEquals("original completion",continuation.await())
            assertEquals(ErrorCode.NotReady,(runCatching {original.admitView {fail("No replacement admission")}}.exceptionOrNull() as? StorePolicy)?.code)
        }
    }
}
