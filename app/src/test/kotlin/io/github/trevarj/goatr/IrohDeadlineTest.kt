// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.mutablePreferencesOf
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.trevarj.goatr.app.session.Prefs
import io.github.trevarj.goatr.app.session.prefs
import java.io.IOException
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertIs
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

class IrohDeadlineTest {
    @Test
    fun persistedIrohConnectionTimeoutDefaultsAndClamps() {
        val key = stringPreferencesKey("prefs")
        assertEquals(45, emptyPreferences().prefs().irohConnectTimeoutSeconds)
        assertEquals(45, mutablePreferencesOf(key to """{"fontSize":13}""").prefs().irohConnectTimeoutSeconds)
        for ((saved, expected) in listOf(75 to 75, Int.MIN_VALUE to 5, Int.MAX_VALUE to 300)) {
            val encoded = Json.encodeToString(Prefs(irohConnectTimeoutSeconds = saved))
            assertEquals(expected, mutablePreferencesOf(key to encoded).prefs().irohConnectTimeoutSeconds)
        }
    }

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
