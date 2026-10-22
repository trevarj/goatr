// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Application
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ViewModelProvider
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import java.io.File
import java.security.KeyStore
import java.util.Base64
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier

/** Actual exported MainActivity/native owner/rotation/foreground cleanup. Isolated installation only. */
class NavigationTest {
    @get:Rule val ui=createEmptyComposeRule()
    @Test fun realLauncherRetainsOwnerAcrossRotationAndReleasesNativeForegroundResources() = runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt another installation's protected identity",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        IrohAndroid.installAndroidContext(context)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner: GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            val endpoint=owner.connection.start()
            ui.onNodeWithTag("pair_begin").performClick()
            ui.onNodeWithTag("pair_paste").performScrollTo().performTextInput("synthetic private raw input, not an invitation")
            scenario.recreate()
            scenario.onActivity {assertSame(owner,ViewModelProvider(it)[GoatrViewModel::class.java])}
            ui.onNodeWithTag("pair_paste").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
            val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
            val json="{\"v\":1,\"host\":\"$host\",\"name\":\"Synthetic never-authorized host\",\"token\":\"${Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32){7})}\",\"expiresAt\":1}"
            val invitation="goatr://pair/v1?data="+Base64.getUrlEncoder().withoutPadding().encodeToString(json.toByteArray())
            ui.onNodeWithTag("pair_paste").performScrollTo().performTextInput(invitation)
            ui.onNodeWithTag("pair_preview").performScrollTo().performClick()
            ui.onNodeWithTag("pair_authority").assertIsDisplayed().assertTextEquals(owner.preview!!.authority)
            assertTrue(owner.store.state.value.hosts.isEmpty())
            ui.onNodeWithTag("pair_cancel").assertIsDisplayed().performClick()
            assertEquals(Screen.Connections,owner.screen)
            ui.onNodeWithTag("pair_confirm").assertDoesNotExist()
            assertTrue(owner.store.state.value.hosts.isEmpty())
            scenario.moveToState(Lifecycle.State.CREATED)
            ui.waitUntil(20000) {!owner.connection.ownsNetworkCallback() && !GoatrConnection.endpointIsOwned()}
            scenario.moveToState(Lifecycle.State.RESUMED)
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            assertEquals(endpoint,owner.connection.start())
            assertTrue(owner.store.state.value.hosts.isEmpty())
        } finally {
            scenario.moveToState(Lifecycle.State.CREATED)
            owner.connection.shutdown()
            // Delete only the alias proven absent before this test and created by this owner.
            assertTrue(owner.store.state.value.hosts.isEmpty())
            SecretStore(context).delete(KeyDeletion("synthetic-lifecycle-owner",listOf(SecretStore.ENDPOINT_ALIAS)),owner.store)
            scenario.close()
        }
    }

    @Test fun zeroHostLostWrappingKeyRequiresExplicitInstallationForgetBeforeNewIdentity() = runBlocking {
        val context=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt another installation identity",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        val cipher=File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm")
        val marker=File(cipher.path+".initialized")
        assertFalse(marker.exists())
        IrohAndroid.installAndroidContext(context)
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner: GoatrViewModel
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            val original=owner.connection.start()
            assertTrue(owner.store.state.value.hosts.isEmpty());assertTrue(cipher.isFile);assertTrue(marker.isFile)
            val retained=cipher.readBytes()
            scenario.moveToState(Lifecycle.State.CREATED)
            ui.waitUntil(20000) {!GoatrConnection.endpointIsOwned()}
            keys.deleteEntry(SecretStore.ENDPOINT_ALIAS) // Only the wrapping entry proven created by this test.
            scenario.moveToState(Lifecycle.State.RESUMED)
            ui.waitUntil(20000) {owner.foreground && owner.notice!=null}
            assertFalse(GoatrConnection.endpointIsOwned());assertTrue(owner.store.state.value.hosts.isEmpty())
            assertArrayEquals(retained,cipher.readBytes());assertTrue(marker.isFile);assertFalse(keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
            ui.onNodeWithTag("installation_forget").performScrollTo().assertIsDisplayed().performClick()
            ui.onNodeWithTag("installation_forget_cancel").performClick()
            assertArrayEquals(retained,cipher.readBytes());assertTrue(marker.isFile);assertFalse(GoatrConnection.endpointIsOwned())
            ui.onNodeWithTag("installation_forget").performScrollTo().performClick()
            ui.onNodeWithTag("installation_forget_confirm").performClick()
            ui.waitUntil(10000) {!cipher.exists() && !marker.exists() && !keys.containsAlias(SecretStore.ENDPOINT_ALIAS)}
            assertTrue(owner.store.state.value.hosts.isEmpty());assertFalse(GoatrConnection.endpointIsOwned())
            scenario.moveToState(Lifecycle.State.CREATED);scenario.moveToState(Lifecycle.State.RESUMED)
            ui.waitUntil(20000) {owner.connection.ownsNetworkCallback() && owner.foreground}
            assertNotEquals(original,owner.connection.start())
            assertTrue(owner.store.state.value.hosts.isEmpty())
        } finally {
            scenario.moveToState(Lifecycle.State.CREATED);owner.connection.shutdown()
            assertTrue(owner.store.state.value.hosts.isEmpty())
            SecretStore(context).delete(KeyDeletion("synthetic-lost-key-owner",listOf(SecretStore.ENDPOINT_ALIAS)),owner.store)
            scenario.close()
        }
    }
}

/** One deterministic actual-Compose eligibility regression; no native/provider pass. */
class CreationEligibilityTest {
    @get:Rule val ui=androidx.compose.ui.test.junit4.v2.createComposeRule()
    @Test fun firstCreationUsesCurrentCanCreateAttemptNotExistingLiveBackendReadiness() {
        val server=Server(Uuid.random(),Label("Owned component server"),ServerConnectivity.Online,CanCreate(true,false))
        val project=Project(Uuid.random(),Label("Approved component project"),true)
        val graph=NativeHierarchy(server.serverId,Uuid.random(),NativeFreshness.Current,workspaces=emptyList(),tabs=emptyList(),panes=emptyList())
        val inventory=mutableStateOf(HostSnapshotResult(Uuid.random(),ZeroValue(0),listOf(server),listOf(project),emptyList(),emptyList(),listOf(graph)))
        val enabled=mutableStateOf(true)
        val hello=HelloResult(EndpointId("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"),Uuid.random(),Uuid.random(),ProtocolVersion(),
            Provider.entries.map {Backend(it,"component only",Availability.SetupRequired,"No existing live native binding")})
        var created:Triple<Uuid,Uuid,Provider>?=null
        val memory=FormMemory()
        ui.setContent {MaterialTheme {Column(Modifier.verticalScroll(rememberScrollState())) {
            CreateSession(inventory.value,hello,enabled.value,memory,{}) {s,p,backend,_->created=Triple(s,p,backend)}
        }}}
        ui.onNodeWithTag("create_name").performScrollTo().performTextInput("First actual guarded attempt")
        ui.onNodeWithTag("session_create").performScrollTo().assertIsEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(servers=listOf(server.copy(canCreate=CanCreate(false,false))))}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(servers=listOf(server.copy(connectivity=ServerConnectivity.Offline)))}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(servers=listOf(server),hierarchy=listOf(graph.copy(freshness=NativeFreshness.Stale,reason=NativeUnavailableReason.SourceUnavailable)))}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        ui.runOnIdle {inventory.value=inventory.value.copy(hierarchy=listOf(graph));enabled.value=false}
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
        assertNull(created)
        ui.runOnIdle {enabled.value=true}
        ui.onNodeWithTag("session_create").assertIsEnabled().performClick()
        assertEquals(Triple(server.serverId,project.projectId,Provider.Omp),created)
        ui.onNodeWithTag("session_create").assertIsNotEnabled()
    }
}
