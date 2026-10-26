// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class SkillsInvocationLeaseTest {
    @Test fun review_skill_sheet_lease_closes_before_entry_without_canceling_admitted_continuation() = runBlocking {
        withTimeout(5000) {
            for(closeBeforeEntry in listOf(true,false)) coroutineScope {
                val lease=SkillInvocationLease()
                val beforeEntry=CompletableDeferred<Unit>()
                val releaseEntry=CompletableDeferred<Unit>()
                val admitted=CompletableDeferred<Unit>()
                val settle=CompletableDeferred<Unit>()
                val original=async(Dispatchers.Default) {runCatching {
                    beforeEntry.complete(Unit)
                    releaseEntry.await()
                    lease.admit {admitted.complete(Unit)}
                    settle.await()
                }}
                beforeEntry.await()
                if(closeBeforeEntry) {
                    lease.invalidate()
                    releaseEntry.complete(Unit);settle.complete(Unit)
                    assertEquals(ErrorCode.StaleSession,(original.await().exceptionOrNull() as? StorePolicy)?.code)
                    assertFalse("Closed-before-entry must not admit the continuation callback",admitted.isCompleted)
                } else {
                    releaseEntry.complete(Unit)
                    admitted.await()
                    lease.invalidate()
                    assertTrue("Invalidation cannot cancel the original admitted continuation",original.isActive)
                    settle.complete(Unit)
                    assertTrue(original.await().isSuccess)
                }
            }
            val contended=SkillInvocationLease()
            val inside=java.util.concurrent.CountDownLatch(1)
            val release=java.util.concurrent.CountDownLatch(1)
            val entry=AtomicBoolean(false)
            val invalidated=AtomicBoolean(false)
            val workerFailure=java.util.concurrent.atomic.AtomicReference<Throwable?>(null)
            val entrant=Thread {
                try {contended.admit {
                    inside.countDown()
                    check(release.await(5,java.util.concurrent.TimeUnit.SECONDS))
                    entry.set(true)
                }} catch(failure:Throwable) {workerFailure.compareAndSet(null,failure)}
            }
            val closer=Thread {
                try {contended.invalidate();invalidated.set(true)}
                catch(failure:Throwable) {workerFailure.compareAndSet(null,failure)}
            }
            try {
                entrant.start()
                assertTrue("Admission must actually hold the production gate",inside.await(5,java.util.concurrent.TimeUnit.SECONDS))
                closer.start()
                // Observe real contention or completion, never assume a scheduled worker entered.
                while(closer.state!=Thread.State.BLOCKED && !invalidated.get() && closer.isAlive)yield()
                assertEquals("Closure must block behind validation-before-entry",Thread.State.BLOCKED,closer.state)
                assertFalse("Closure cannot complete before entry",invalidated.get())
                assertFalse(entry.get())
            } finally {
                release.countDown()
                entrant.join(5000);closer.join(5000)
            }
            assertFalse(entrant.isAlive);assertFalse(closer.isAlive)
            workerFailure.get()?.let {throw AssertionError("Owned lease contention worker failed",it)}
            assertTrue(entry.get());assertTrue(invalidated.get())
        }
    }
}
