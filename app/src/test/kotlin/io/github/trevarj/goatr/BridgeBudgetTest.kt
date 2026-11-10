// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.util.concurrent.atomic.AtomicLong
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

class BridgeBudgetTest {
    @Test
    fun stockFramesFitAndEveryOwnershipPathRestoresTheByteBudget() = runBlocking {
        val budget = 32L * 1024 * 1024
        val stockFrameSize = 16_777_225 // hc: 16 MiB payload plus nine-byte header.
        val retained = AtomicLong()
        val writes = Channel<ByteArray>(1, onUndeliveredElement = { it.releaseBridgeWrite(retained) })
        val source = ByteArray(stockFrameSize)
        assertTrue(writes.tryQueueBridgeWrite(source, retained))
        val inFlight = writes.tryReceive().getOrThrow()
        assertEquals(stockFrameSize.toLong(), retained.get())

        // Dequeue is not completion: two stock-max frames exceed the budget by 18 bytes.
        assertFalse(writes.tryQueueBridgeWrite(source, retained))
        assertEquals(stockFrameSize.toLong(), retained.get())
        val remainder = ByteArray((budget - stockFrameSize).toInt())
        assertTrue(writes.tryQueueBridgeWrite(remainder, retained))
        assertEquals(budget, retained.get())
        assertFalse(writes.tryQueueBridgeWrite(byteArrayOf(1), retained))
        assertEquals(budget, retained.get())
        inFlight.releaseBridgeWrite(retained)
        assertEquals(remainder.size.toLong(), retained.get())
        writes.tryReceive().getOrThrow().releaseBridgeWrite(retained)
        assertEquals(0L, retained.get())

        // Full and closed channels must undo their byte reservations.
        assertTrue(writes.tryQueueBridgeWrite(byteArrayOf(1), retained))
        assertFalse(writes.tryQueueBridgeWrite(byteArrayOf(2), retained))
        assertEquals(1L, retained.get())
        writes.close()
        assertFalse(writes.tryQueueBridgeWrite(byteArrayOf(3, 4), retained))
        assertEquals(1L, retained.get())
        writes.tryReceive().getOrThrow().releaseBridgeWrite(retained)
        assertEquals(0L, retained.get())

        val canceledWrites = Channel<ByteArray>(1, onUndeliveredElement = { it.releaseBridgeWrite(retained) })
        // Cancel after delivery to a waiting iterator, before its body can take ownership.
        val waitingWriter = launch(start = CoroutineStart.UNDISPATCHED) {
            for (bytes in canceledWrites) {
                try {
                    error("Canceled writer must not resume")
                } finally {
                    bytes.releaseBridgeWrite(retained)
                }
            }
        }
        assertTrue(canceledWrites.tryQueueBridgeWrite(byteArrayOf(5), retained))
        assertEquals(1L, retained.get())
        waitingWriter.cancel()
        waitingWriter.join()
        assertEquals(0L, retained.get())

        // Cancellation after body entry is instead owned by its finally block.
        assertTrue(canceledWrites.tryQueueBridgeWrite(byteArrayOf(6), retained))
        val activeWriter = launch(start = CoroutineStart.UNDISPATCHED) {
            for (bytes in canceledWrites) {
                try {
                    awaitCancellation()
                } finally {
                    bytes.releaseBridgeWrite(retained)
                }
            }
        }
        assertEquals(1L, retained.get())
        activeWriter.cancel()
        activeWriter.join()
        assertEquals(0L, retained.get())
        canceledWrites.close()
        Unit
    }
}
