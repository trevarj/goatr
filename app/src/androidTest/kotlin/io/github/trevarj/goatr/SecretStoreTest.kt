// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import java.io.File
import java.security.KeyStore
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Test

class SecretStoreTest {
    private val context get()=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
    private fun host():String=computer.iroh.SecretKey.generate().use {it.public().use {id->id.toString()}}
    @Test fun realAesGcmRetainsKeyAndWrappingLossFailsClosedUntilActualForget() = runBlocking {
        IrohAndroid.installAndroidContext(context)
        val host=host();val alias=SecretStore.hostAlias(host,"receipt");val name="keys-${Uuid.random().value}.db";val store=SessionStore(context,name);val secrets=SecretStore(context)
        try {
            store.open();val key=secrets.receipt(host,store);store.host(HostMetadata(host,"owned key fixture",keyAliases=listOf(alias)))
            val retained=SecretStore(context).receipt(host,store);assertTrue(key.contentEquals(retained));retained.fill(0)
            val file=File(context.noBackupFilesDir,"goatr-keys/$alias.gcm");val original=file.readBytes();assertEquals(61,original.size)
            assertFalse("Ciphertext must not contain the retained material",original.toList().windowed(32).any {it==key.toList()})
            val wrapping=KeyStore.getInstance("AndroidKeyStore").apply {load(null)};wrapping.deleteEntry(alias)
            var lost=false;try {SecretStore(context).receipt(host,store)} catch (_:ProtectedKeyUnavailable) {lost=true};assertTrue(lost);assertTrue(original.contentEquals(file.readBytes()));assertFalse(wrapping.containsAlias(alias))
            val obligation=store.forget(host);assertEquals(listOf(alias),obligation.aliases);assertEquals(obligation,store.state.value.keyDeletion.single())
            secrets.delete(obligation,store);assertFalse(file.exists());assertTrue(store.state.value.keyDeletion.isEmpty());key.fill(0)
        } finally {if(store.state.value.hosts.any {it.hostId==host}) secrets.delete(store.forget(host),store);for(obligation in store.state.value.keyDeletion) secrets.delete(obligation,store);store.close();context.deleteDatabase(name)}
    }
    @Test fun actualNativeEndpointRecreationUsesSameProtectedIdentityAndOneOwner() = runBlocking {
        IrohAndroid.installAndroidContext(context)
        val wrapping=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        // This fixture must not adopt/delete an already installed application identity.
        assertFalse("Run this owned lifecycle fixture in a fresh isolated app installation",wrapping.containsAlias(SecretStore.ENDPOINT_ALIAS))
        val name="lifecycle-${Uuid.random().value}.db";val store=SessionStore(context,name);val secrets=SecretStore(context);val owner=GoatrConnection(context,store,secrets);val second=GoatrConnection(context,store,secrets);val host=host()
        try {
            val first=withTimeout(20000) {owner.start()};assertTrue(owner.ownsNetworkCallback())
            store.host(HostMetadata(host,"owned endpoint fixture",keyAliases=listOf(SecretStore.ENDPOINT_ALIAS)))
            var duplicate=false;try {second.start()} catch(e:StorePolicy) {duplicate=e.code==ErrorCode.Busy};assertTrue(duplicate)
            assertEquals(first,withTimeout(15000) {owner.recreate()});assertTrue(owner.ownsNetworkCallback())
            withTimeout(15000) {owner.shutdown()};assertFalse(owner.ownsNetworkCallback());assertEquals(first,withTimeout(20000) {owner.start()})
            owner.forget(host);assertFalse(owner.ownsNetworkCallback());assertFalse(wrapping.containsAlias(SecretStore.ENDPOINT_ALIAS));assertTrue(store.state.value.keyDeletion.isEmpty())
        } finally {owner.shutdown();second.shutdown();if(store.state.value.hosts.any {it.hostId==host}) secrets.delete(store.forget(host),store);for(obligation in store.state.value.keyDeletion) secrets.delete(obligation,store);store.close();context.deleteDatabase(name)}
    }
    @Test fun twoHostsActuallyDeleteOnlyUnsharedAliases() = runBlocking {
        IrohAndroid.installAndroidContext(context)
        val a=host();val b=host();val name="shared-${Uuid.random().value}.db";val store=SessionStore(context,name);val secrets=SecretStore(context)
        val shared=SecretStore.hostAlias(a,"push");val privateA=SecretStore.hostAlias(a,"receipt");val privateB=SecretStore.hostAlias(b,"receipt")
        try {
            store.open();for(alias in listOf(shared,privateA,privateB)) secrets.material(alias,true).fill(0)
            store.host(HostMetadata(a,"first",keyAliases=listOf(shared,privateA)));store.host(HostMetadata(b,"second",keyAliases=listOf(shared,privateB)))
            val first=store.forget(a);assertEquals(listOf(privateA),first.aliases);secrets.delete(first,store)
            val wrapping=KeyStore.getInstance("AndroidKeyStore").apply {load(null)};assertFalse(wrapping.containsAlias(privateA));assertTrue(wrapping.containsAlias(shared));secrets.material(shared,false).fill(0)
            val last=store.forget(b);assertEquals(listOf(shared,privateB),last.aliases);secrets.delete(last,store);assertFalse(wrapping.containsAlias(shared));assertFalse(wrapping.containsAlias(privateB));assertTrue(store.state.value.keyDeletion.isEmpty())
        } finally {for(host in store.state.value.hosts.toList()) secrets.delete(store.forget(host.hostId),store);for(obligation in store.state.value.keyDeletion) secrets.delete(obligation,store);store.close();context.deleteDatabase(name)}
    }
}
