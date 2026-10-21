// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.Manifest
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.WindowManager
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.camera.core.CameraSelector
import androidx.camera.core.CameraState
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ViewModelProvider
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import java.io.File
import java.security.KeyStore
import java.util.ArrayDeque
import java.util.Base64
import java.util.concurrent.Executor
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Production scan/paste gate and actual CameraX/ZXing consumer. Synthetic never-issued
 * invitations only: scanner/paste preview is not host authorization or live pairing proof. */
class PairingUiTest {
    @get:Rule val ui=createComposeRule()
    @Test fun deniedCameraLeavesActualPasteAndDisposedQueuedCallbacksCannotAccept() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val context=instrumentation.targetContext
        IrohAndroid.installAndroidContext(context)
        assertNotEquals("Coordinator must revoke this owned app's CAMERA permission before instrumentation; revoking a granted permission here can kill the test process",android.content.pm.PackageManager.PERMISSION_GRANTED,context.checkSelfPermission(Manifest.permission.CAMERA))
        val gate=PairingGate()
        var route by mutableStateOf(true)
        var preview by mutableStateOf<PairingPreview?>(null)
        ui.setContent {MaterialTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            if(route) PairScanner(true) {raw->gate.submit(raw)?.let {preview=it;true} ?: false}
        }}}
        ui.onNodeWithTag("camera_permission").performScrollTo().performClick()
        ui.waitUntil(5000) {instrumentation.uiAutomation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_deny_button")?.isNotEmpty()==true}
        val deny=instrumentation.uiAutomation.rootInActiveWindow.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_deny_button").first()
        assertTrue(deny.performAction(AccessibilityNodeInfo.ACTION_CLICK));deny.recycle()
        ui.onNodeWithText("Camera permission denied. Paste remains available.").assertExists()
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val body="{\"v\":1,\"host\":\"$host\",\"name\":\"Synthetic preview\",\"token\":\"${Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32){11})}\",\"expiresAt\":1}"
        val uri="goatr://pair/v1?data="+Base64.getUrlEncoder().withoutPadding().encodeToString(body.toByteArray())
        ui.onNodeWithTag("pair_paste").performScrollTo().performTextInput(uri)
        ui.onNodeWithTag("pair_preview").performScrollTo().performClick()
        ui.runOnIdle {assertEquals(host,preview!!.host.value);assertEquals("Synthetic preview",preview!!.name.value)}
        val queued=ArrayDeque<Runnable>();val active=AtomicBoolean(true)
        val callbacks=ScannerCallbacks(Executor {queued.add(it)}) {active.get()}
        var accepted=0
        callbacks.post {if(gate.submit(uri)!=null) accepted++}
        ui.runOnIdle {route=false;active.set(false);callbacks.close();gate.close()}
        while(queued.isNotEmpty()) queued.removeFirst().run()
        assertEquals(0,accepted);assertNull(gate.submit(uri))
        ui.onNodeWithTag("pair_paste").assertDoesNotExist()
    }

    @Test fun cameraRetryOwnsFreshErrorsAndDisposedBindingsCannotFailTheNextAttempt() {
        val queued=ArrayDeque<Runnable>()
        val callbacks=ScannerCallbacks(Executor {queued.add(it)}) {true}
        val bindingActive=AtomicBoolean(true)
        var failures=0
        val attempt=ScannerCameraAttempt(callbacks,{bindingActive.get()}) {failures++}
        val disabled=CameraState.StateError.create(CameraState.ERROR_CAMERA_DISABLED)
        val stale=CameraState.create(CameraState.Type.CLOSED,disabled)
        fun drain() {while(queued.isNotEmpty()) queued.removeFirst().run()}
        // Actual failing Retry began with this shared prior-attempt CLOSED/error5 replay.
        attempt.observe(stale);drain()
        assertEquals(0,failures)
        attempt.observe(CameraState.create(CameraState.Type.OPENING))
        for(type in CameraState.Type.entries) {
            attempt.observe(CameraState.create(type,disabled));drain()
            assertEquals(type.ordinal+1,failures)
        }
        val liveFailures=failures
        attempt.observe(stale)
        bindingActive.set(false)
        drain()
        attempt.observe(CameraState.create(CameraState.Type.OPENING,disabled));drain()
        assertEquals(liveFailures,failures)
        // A new attempt has independent authority; a real frame also admits terminal errors.
        val fresh=ScannerCameraAttempt(callbacks,{true}) {failures++}
        fresh.observe(stale);drain()
        assertEquals(liveFailures,failures)
        fresh.frame()
        fresh.observe(stale);drain()
        assertEquals(liveFailures+1,failures)
        fresh.observe(stale)
        callbacks.close()
        drain()
        assertEquals(liveFailures+1,failures)
    }

    private companion object {
        // Coordinator derives the memory-only wall QR from these source constants; never
        // put the encoded invitation/token/QR pixels in argv, status, logs or evidence.
        const val CAMERA_FIXTURE_HOST="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        const val CAMERA_FIXTURE_NAME="Never-issued camera fixture"
        const val CAMERA_FIXTURE_TOKEN_BYTE=11
        const val CAMERA_FIXTURE_EXPIRY=1L
        const val CAMERA_FRESH_FIXTURE_NAME="Never-issued fresh camera fixture"
        const val CAMERA_FRESH_FIXTURE_EXPIRY=2L
        fun cameraInvitation():String {
            val token=Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32) {CAMERA_FIXTURE_TOKEN_BYTE.toByte()})
            val json="{\"v\":1,\"host\":\"$CAMERA_FIXTURE_HOST\",\"name\":\"$CAMERA_FIXTURE_NAME\",\"token\":\"$token\",\"expiresAt\":$CAMERA_FIXTURE_EXPIRY}"
            return "goatr://pair/v1?data="+Base64.getUrlEncoder().withoutPadding().encodeToString(json.toByteArray(Charsets.UTF_8))
        }
    }
    /** Test-only feed scheduling, never camera/decode/readiness evidence by itself. */
    private fun awaitCameraFeed(phase:String) {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val context=instrumentation.targetContext
        val action=context.packageName+".CAMERA_ACCEPTANCE_ACK"
        val ready=CountDownLatch(1)
        val receiver=object:BroadcastReceiver() {
            override fun onReceive(context:Context,intent:Intent) {
                if(intent.action==action && intent.getStringExtra("phase")==phase) ready.countDown()
            }
        }
        ContextCompat.registerReceiver(context,receiver,IntentFilter(action),ContextCompat.RECEIVER_EXPORTED)
        try {
            instrumentation.sendStatus(1,Bundle().apply {putString("goatrCameraPhase",phase)})
            assertTrue("Coordinator must complete the owned memory-only virtual-scene feed for this fixed phase",ready.await(30,TimeUnit.SECONDS))
        } finally {context.unregisterReceiver(receiver)}
    }
    @Test fun realCameraFramesDecodeSyntheticInvitationAndBackReentryPasteNeverPair() = runBlocking {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        var failureStage="before-scenario"
        var failureReported=false
        fun reportFailure(failure:Throwable) {
            instrumentation.sendStatus(1,Bundle().apply {
                putString("goatrCameraFailureStage",failureStage)
                putString("goatrCameraFailureClass",failure.javaClass.name)
            })
            failureReported=true
        }
        try {
        val context=instrumentation.targetContext.applicationContext
        assertEquals("Coordinator grants CAMERA only after the separate denied-camera gate",PackageManager.PERMISSION_GRANTED,context.checkSelfPermission(Manifest.permission.CAMERA))
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assertFalse("Never adopt an existing protected installation",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        assertFalse(File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}.gcm.initialized").exists())
        val process=android.os.Process.myPid()
        val stat=File("/proc/self/stat").readText()
        val start=stat.substring(stat.lastIndexOf(')')+2).trim().split(Regex("\\s+"))[19]
        assertTrue("Camera instrumentation must report its actual kernel process",stat.substringBefore(' ').toInt()==process)
        instrumentation.sendStatus(1,Bundle().apply {
            putString("goatrCameraOwner","fresh-start")
            putInt("goatrCameraPid",process)
            putString("goatrCameraStart",start)
        })
        failureStage="policy"
        val policy=try {
            instrumentation.uiAutomation.adoptShellPermissionIdentity("android.permission.MANAGE_SENSOR_PRIVACY","android.permission.OBSERVE_SENSOR_PRIVACY")
            val manager=context.getSystemService("sensor_privacy")
            manager.javaClass.getMethod("isSensorPrivacyEnabled",Int::class.javaPrimitiveType).invoke(manager,2) as Boolean
        } catch(_:Exception) {
            instrumentation.sendStatus(1,Bundle().apply {putString("goatrCameraPolicyResult","capture-unavailable")})
            throw AssertionError("Actual native sensor policy capture unavailable; no assumed disabled/default state")
        } finally {instrumentation.uiAutomation.dropShellPermissionIdentity()}
        instrumentation.sendStatus(1,Bundle().apply {
            putString("goatrCameraPolicy",if(policy) "enabled" else "disabled")
            putInt("goatrCameraUser",android.os.Process.myUid()/100000)
        })
        failureStage="before-scenario"
        IrohAndroid.installAndroidContext(context)
        val provider=ProcessCameraProvider.getInstance(context).get(10,TimeUnit.SECONDS)
        val camera=CameraSelector.DEFAULT_BACK_CAMERA.filter(provider.availableCameraInfos).single()
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var owner:GoatrViewModel
        fun rawSceneProtected() {
            scenario.onActivity {activity->assertTrue("Raw scanner/paste scene must be FLAG_SECURE",activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE!=0)}
        }
        fun noPairEffect() {
            assertTrue("Preview/Back must not create paired-host metadata",owner.store.state.value.hosts.isEmpty())
            assertTrue("Preview/Back must not authenticate a host",owner.connection.state.value.hello.isEmpty())
            assertTrue(owner.store.state.value.recovery.isEmpty())
            assertFalse(owner.busy)
        }
        fun awaitRealFrames() {
            ui.waitUntil(20000) {
                camera.cameraState.value?.type==CameraState.Type.OPEN &&
                    ui.onAllNodes(hasTestTag("camera_status") and hasText("Scanning QR · preview only")).fetchSemanticsNodes(atLeastOneRootRequired=false).isNotEmpty()
            }
            ui.onNodeWithTag("pair_camera").assertIsDisplayed()
        }
        try {
            scenario.onActivity {owner=ViewModelProvider(it)[GoatrViewModel::class.java]}
            ui.waitUntil(20000) {owner.foreground && owner.connection.ownsNetworkCallback()}
            noPairEffect()
            // Main preloads a neutral wall while camera is OFF before this isolated test.
            failureStage="first-native-open"
            ui.onNodeWithTag("pair_begin").performClick()
            awaitRealFrames();rawSceneProtected()
            failureStage="first-qr-preview"
            awaitCameraFeed("first-qr")
            ui.waitUntil(20000) {owner.preview!=null}
            failureStage="first-qr-synthetic-identity"
            ui.runOnIdle {
                val preview=owner.preview!!
                assertTrue("Only the coordinator's never-issued fixture may produce this preview",preview.host.value==CAMERA_FIXTURE_HOST && preview.name.value==CAMERA_FIXTURE_NAME && preview.expiresAt.value==CAMERA_FIXTURE_EXPIRY)
            }
            failureStage="first-qr-authority-displayed"
            ui.onNodeWithTag("pair_authority").assertIsDisplayed().assertTextEquals(owner.preview!!.authority)
            failureStage="first-qr-confirm-present"
            ui.onNodeWithTag("pair_confirm").assertExists()
            failureStage="first-qr-no-pair"
            noPairEffect()
            failureStage="back"
            ui.onNodeWithTag("pair_cancel").performScrollTo().performClick()
            ui.waitUntil(20000) {owner.screen==Screen.Connections && owner.preview==null && camera.cameraState.value?.type==CameraState.Type.CLOSED}
            ui.onNodeWithTag("pair_confirm").assertDoesNotExist()
            noPairEffect()
            // QR remains on the wall until CLOSED is observed; late disposed callbacks
            // cannot reopen a preview. Neutral feed is acknowledged only while camera is OFF.
            awaitCameraFeed("neutral-after-back")
            assertNull(owner.preview);assertEquals(Screen.Connections,owner.screen)
            failureStage="reentry"
            ui.onNodeWithTag("pair_begin").performClick()
            awaitRealFrames()
            failureStage="paste-source-privacy"
            rawSceneProtected()
            failureStage="paste-empty-edit"
            ui.onNodeWithTag("pair_paste").performScrollTo().assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
            failureStage="paste-manual-preview"
            ui.onNodeWithTag("pair_paste").performTextInput(cameraInvitation())
            ui.onNodeWithTag("pair_preview").performScrollTo().performClick()
            ui.waitUntil(10000) {owner.preview!=null}
            failureStage="paste-authority"
            ui.onNodeWithTag("pair_authority").assertIsDisplayed().assertTextEquals(owner.preview!!.authority)
            failureStage="paste-no-pair"
            noPairEffect()
            failureStage="paste-closed"
            ui.onNodeWithTag("pair_cancel").performScrollTo().performClick()
            ui.waitUntil(20000) {owner.preview==null && owner.screen==Screen.Connections && camera.cameraState.value?.type==CameraState.Type.CLOSED}
            failureStage="paste-no-pair"
            noPairEffect()
            failureStage="rapid-back"
            repeat(2) {
                ui.onNodeWithTag("pair_begin").performClick();awaitRealFrames()
                ui.onNodeWithTag("pair_cancel").performScrollTo().performClick()
                ui.waitUntil(10000) {camera.cameraState.value?.type==CameraState.Type.CLOSED && owner.preview==null}
                noPairEffect()
            }
            failureStage="policy"
            ui.onNodeWithTag("pair_begin").performClick();awaitRealFrames()
            val errorObserved=AtomicBoolean(false)
            val nativeError=androidx.lifecycle.Observer<CameraState> {if(it.error!=null) errorObserved.set(true)}
            scenario.onActivity {camera.cameraState.observe(it,nativeError)}
            try {
                awaitCameraFeed("policy-enable")
                failureStage="error"
                val deadline=android.os.SystemClock.elapsedRealtime()+5000
                while(!errorObserved.get() && android.os.SystemClock.elapsedRealtime()<deadline) Thread.sleep(20)
                if(!errorObserved.get()) {
                    instrumentation.sendStatus(1,Bundle().apply {putString("goatrCameraPolicyResult","unsupported-error-trigger")})
                    throw AssertionError("Sensor privacy did not produce actual CameraState error; black frames are not async-error proof")
                }
                ui.waitUntil(10000) {ui.onAllNodes(hasTestTag("camera_permission") and hasText("Retry camera")).fetchSemanticsNodes(atLeastOneRootRequired=false).isNotEmpty()}
                failureStage="paste"
                ui.onNodeWithTag("pair_paste").performScrollTo().assertIsDisplayed().performTextInput(cameraInvitation())
                noPairEffect()
                failureStage="retry"
                awaitCameraFeed("policy-restore")
                ui.onNodeWithTag("camera_permission").performScrollTo().performClick()
                awaitRealFrames()
                failureStage="fresh-qr"
                awaitCameraFeed("second-qr")
                ui.waitUntil(20000) {owner.preview?.name?.value==CAMERA_FRESH_FIXTURE_NAME && owner.preview?.expiresAt?.value==CAMERA_FRESH_FIXTURE_EXPIRY}
                noPairEffect()
                failureStage="back"
                ui.onNodeWithTag("pair_cancel").performScrollTo().performClick()
                ui.waitUntil(10000) {camera.cameraState.value?.type==CameraState.Type.CLOSED && owner.preview==null}
                noPairEffect()
            } finally {scenario.onActivity {camera.cameraState.removeObserver(nativeError)}}
        } catch(failure:Throwable) {
            reportFailure(failure)
            throw failure
        } finally {
            failureStage="final-restoration"
            try {
            scenario.moveToState(Lifecycle.State.CREATED)
            owner.connection.shutdown()
            ui.waitUntil(10000) {camera.cameraState.value?.type==CameraState.Type.CLOSED}
            awaitCameraFeed("policy-original")
            try {
                instrumentation.uiAutomation.adoptShellPermissionIdentity("android.permission.MANAGE_SENSOR_PRIVACY","android.permission.OBSERVE_SENSOR_PRIVACY")
                val manager=context.getSystemService("sensor_privacy")
                assertTrue("Restore the exact original native camera policy",(manager.javaClass.getMethod("isSensorPrivacyEnabled",Int::class.javaPrimitiveType).invoke(manager,2) as Boolean)==policy)
            } finally {instrumentation.uiAutomation.dropShellPermissionIdentity()}
            instrumentation.sendStatus(1,Bundle().apply {
                putString("goatrCameraOwner","fresh-closed")
                putInt("goatrCameraPid",process)
                putString("goatrCameraStart",start)
            })
            assertTrue(owner.store.state.value.hosts.isEmpty())
            // Forget only the zero-host installation proved absent and created by this test.
            owner.connection.forgetInstallation()
            scenario.close()
            } catch(failure:Throwable) {
                reportFailure(failure)
                throw failure
            }
        }
        } catch(failure:Throwable) {
            if(!failureReported) reportFailure(failure)
            throw failure
        }
    }
}
