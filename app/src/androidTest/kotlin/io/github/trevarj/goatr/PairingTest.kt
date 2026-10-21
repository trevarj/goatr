// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import java.util.Base64
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class PairingTest {
    private fun uri(extra: String = "", token: String = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32){7})): String {
        val host=computer.iroh.SecretKey.fromBytes(ByteArray(32){17}).use {it.public().use {id->id.toString()}}
        val json="{\"v\":1,\"host\":\"$host\",\"name\":\"Host\",\"token\":\"$token\",\"expiresAt\":1$extra}"
        return "goatr://pair/v1?data="+Base64.getUrlEncoder().withoutPadding().encodeToString(json.toByteArray())
    }
    @Test fun exactParsingAndDisposedScanPasteCannotPair() = runBlocking {
        IrohAndroid.installAndroidContext(InstrumentationRegistry.getInstrumentation().targetContext.applicationContext)
        val complete=uri();val invitation=PairingInvitation.parse(complete)
        assertEquals("Host",invitation.name.value);assertEquals(1L,invitation.expiresAt.value)
        for(input in listOf(complete+"=",complete+"&x=1",uri(",\"extra\":true"),uri(",\"name\":\"duplicate\""),"x".repeat(4097),uri(token="A".repeat(42)),uri(token="A".repeat(43)+"="))) {
            var rejected=false;try {PairingInvitation.parse(input)} catch (_:Exception) {rejected=true};assertTrue("Malformed pairing input must fail without diagnostic capability text",rejected)
        }
        val scan=PairingGate();val paste=PairingGate();assertNotNull(scan.submit(complete));assertNotNull(paste.submit(complete));assertNull(scan.submit("not a QR"))
        scan.close();paste.close();assertNull(scan.submit(complete));assertNull(paste.submit(complete));invitation.clear()
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val name="back-${Uuid.random().value}.db";val store=SessionStore(context,name);val connection=GoatrConnection(context,store,SecretStore(context))
        try {
            store.open()
            for(gate in listOf(scan,paste)) {var rejected=false;try {gate.confirm(connection)} catch (_:IllegalStateException) {rejected=true};assertTrue(rejected)}
            assertTrue(store.state.value.hosts.isEmpty());assertFalse(connection.ownsNetworkCallback())
        } finally {connection.shutdown();store.close();context.deleteDatabase(name)}
    }
}
