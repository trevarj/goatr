// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.io.IOException
import kotlin.test.Test
import kotlin.test.assertFailsWith
import kotlin.test.assertIs
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

class IrohDeadlineTest {
    @Test
    fun localDeadlineFailsButCallerCancellationPropagates() = runBlocking {
        assertFailsWith<IOException> {
            withIrohDeadline<Unit>(1, "Local operation timed out") { awaitCancellation() }
        }

        var observed: Exception? = null
        val child = launch(start = CoroutineStart.UNDISPATCHED) {
            try {
                withIrohDeadline<Unit>(60_000, "Local operation timed out") { awaitCancellation() }
            } catch (e: Exception) {
                observed = e
                throw e
            }
        }
        child.cancelAndJoin()
        assertIs<CancellationException>(observed)
        Unit
    }
}
