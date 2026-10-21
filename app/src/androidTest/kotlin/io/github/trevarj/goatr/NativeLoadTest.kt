// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.Endpoint
import computer.iroh.EndpointOptions
import computer.iroh.IrohAndroid
import computer.iroh.presetMinimal
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Test

class NativeLoadTest {
    @Test fun realBindingLoadsAndRetainedEndpointIdentitySurvivesShutdown() = runBlocking {
        IrohAndroid.installAndroidContext(InstrumentationRegistry.getInstrumentation().targetContext.applicationContext)
        val secret = ByteArray(32) { 7 }
        suspend fun bind(): Endpoint {
            val options = EndpointOptions(preset = presetMinimal(),bindAddr = "127.0.0.1:0",secretKey = secret,alpns = listOf("goatr/2".toByteArray(Charsets.UTF_8)))
            return try { withTimeout(15000) { Endpoint.bind(options) } } finally { options.destroy() }
        }
        suspend fun shutdown(endpoint: Endpoint) { withTimeout(5000) { endpoint.shutdown() }; assertTrue(endpoint.isClosed()) }
        val first = bind()
        val identity: String
        try {
            identity = first.id().use { it.toString() }
            assertEquals(identity,EndpointId(identity).value)
            withTimeout(5000) { first.networkChange() }
        } finally { try { shutdown(first) } finally { first.close() } }
        val recreated = bind()
        try { assertEquals(identity,recreated.id().use { it.toString() }) }
        finally { try { shutdown(recreated) } finally { recreated.close() } }
    }
}
