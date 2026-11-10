// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright 2026 Trevor Arjeski. Adapted 2026-10-08 from MOTD GPL-3.0-or-later,
// f348f44d3b5ca747e3a8751f13a277fd601ffd73 ui/invite/QrInviteScannerScreen.kt.
// Changes: shared Goatr preview gate, all queued callback guards, owned-only unbind,
// no logging, private paste memory and bounded zeroized frame decoding.
package io.github.trevarj.goatr

import android.Manifest
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.CameraState
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.*
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.LiveData
import androidx.lifecycle.Observer
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** The same production guard protects permission, bind, frame, status and failure callbacks. */
internal class ScannerCallbacks(private val executor: Executor,private val routeActive: ()->Boolean) : AutoCloseable {
    private val disposed=AtomicBoolean(false)
    fun active(): Boolean = !disposed.get() && routeActive()
    fun post(callback: ()->Unit) {if(!disposed.get()) executor.execute {if(active()) callback()}}
    override fun close() {disposed.set(true)}
}

/** Shared CameraInfo replays the prior binding's terminal error before a fresh open begins. */
internal class ScannerCameraAttempt(private val callbacks: ScannerCallbacks,private val bindingActive: ()->Boolean,private val onFailure: ()->Unit) {
    private val entered=AtomicBoolean(false)
    fun frame() {entered.set(true)}
    fun observe(state: CameraState) {
        if(!bindingActive() || !callbacks.active()) return
        if(state.type==CameraState.Type.PENDING_OPEN || state.type==CameraState.Type.OPENING || state.type==CameraState.Type.OPEN) entered.set(true)
        // Once this binding enters/produces a frame, every live error is authoritative,
        // including CLOSING/CLOSED failures. Provider/bind/analyzer exceptions bypass this guard.
        if(state.error!=null && entered.get()) callbacks.post {if(bindingActive()) onFailure()}
    }
}

@Composable
fun PairScanner(active: Boolean,onValue: (String)->Boolean) {
    val context=LocalContext.current
    val currentActive by rememberUpdatedState(active)
    val accept by rememberUpdatedState(onValue)
    val main=remember {val handler=Handler(Looper.getMainLooper());Executor {handler.post(it)}}
    val callbacks=remember {ScannerCallbacks(main) {currentActive}}
    var granted by remember {mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA)==PackageManager.PERMISSION_GRANTED)}
    var denied by remember {mutableStateOf(false)}
    var cameraFailed by remember {mutableStateOf(false)}
    var retry by remember {mutableIntStateOf(0)}
    var paste by remember {mutableStateOf("")}
    var pasteLifetime by remember {mutableIntStateOf(0)}
    val hasCamera=remember {context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)}
    val permission=rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {value->
        if(callbacks.active()) {granted=value;denied=!value}
    }
    DisposableEffect(callbacks) {onDispose {callbacks.close();paste=""}}
    Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
        if(hasCamera && granted && !cameraFailed && active) {
            key(retry) {CameraPreview(callbacks,{raw->if(accept(raw)) {paste="";pasteLifetime++} else cameraFailed=false},{cameraFailed=true})}
        } else {
            UiText(when {cameraFailed->"Camera unavailable. Paste still works; retry camera explicitly.";denied->"Camera permission denied. Paste remains available.";!hasCamera->"No camera. Paste an invitation below.";else->"Camera access is optional; scan or paste a desktop invitation."},modifier=Modifier.semantics {liveRegion=LiveRegionMode.Polite})
        }
        if(hasCamera && (!granted || cameraFailed)) Action(if(granted) "Retry camera" else "Allow camera","camera_permission",enabled=active) { if(granted) {cameraFailed=false;retry++} else permission.launch(Manifest.permission.CAMERA) }
        // Never save capability input or expose it via selectable backing-copy controls.
        key(pasteLifetime) {OutlinedTextField(value=paste,onValueChange={paste=it},label={UiText("Paste pairing invitation (private memory only)")},modifier=Modifier.fillMaxWidth().testTag("pair_paste"),enabled=active,minLines=2,maxLines=3)}
        Action("Preview invitation","pair_preview",enabled=active && paste.isNotEmpty()) { val raw=paste;paste="";pasteLifetime++;accept(raw) }
    }
}

@Composable
private fun CameraPreview(callbacks: ScannerCallbacks,onValue: (String)->Unit,onFailure: ()->Unit) {
    val context=LocalContext.current
    val owner=LocalLifecycleOwner.current
    val previewView=remember {PreviewView(context)}
    val value by rememberUpdatedState(onValue)
    val failure by rememberUpdatedState(onFailure)
    var status by remember {mutableStateOf("Starting camera")}
    Column {
        UiText(status,modifier=Modifier.testTag("camera_status"))
        AndroidView(factory={previewView},modifier=Modifier.fillMaxWidth().height(220.dp).testTag("pair_camera"))
    }
    DisposableEffect(previewView,owner,callbacks) {
        val disposed=AtomicBoolean(false)
        val consumed=AtomicBoolean(false)
        val firstFrame=AtomicBoolean(false)
        val executor=Executors.newSingleThreadExecutor()
        val handler=Handler(Looper.getMainLooper())
        val main=Executor {handler.post(it)}
        val providerFuture=ProcessCameraProvider.getInstance(context)
        var bound: ProcessCameraProvider?=null
        val preview=Preview.Builder().build().also {it.surfaceProvider=previewView.surfaceProvider}
        val analysis=ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
        fun active()=!disposed.get() && callbacks.active()
        var observed:LiveData<CameraState>?=null
        val attempt=ScannerCameraAttempt(callbacks,{!disposed.get()}) {failure()}
        val cameraObserver=Observer<CameraState>(attempt::observe)
        providerFuture.addListener({
            if(active()) {
                try {
                    val provider=providerFuture.get()
                    if(active()) {
                        analysis.setAnalyzer(executor) {image->
                            try {
                                if(!active() || consumed.get()) return@setAnalyzer
                                if(firstFrame.compareAndSet(false,true)) {
                                    attempt.frame()
                                    callbacks.post {if(active()) status="Scanning QR · preview only"}
                                }
                                val plane=image.planes.firstOrNull() ?: return@setAnalyzer
                                val bytes=ByteArray(plane.buffer.remaining()).also(plane.buffer::get)
                                val raw=try {val crop=image.cropRect;decodeQrFrame(bytes,image.width,image.height,plane.rowStride,image.imageInfo.rotationDegrees,plane.pixelStride,crop.left,crop.top,crop.width(),crop.height())} finally {bytes.fill(0)}
                                if(raw!=null && consumed.compareAndSet(false,true)) callbacks.post {
                                    if(active()) {status="QR decoded · validating preview";value(raw);consumed.set(false)}
                                }
                            } catch(_: Exception) {callbacks.post {if(active()) failure()}}
                            finally {image.close()}
                        }
                        if(active()) {
                            bound=provider
                            val camera=provider.bindToLifecycle(owner,CameraSelector.DEFAULT_BACK_CAMERA,preview,analysis)
                            if(active()) {
                                observed=camera.cameraInfo.cameraState
                                observed?.observe(owner,cameraObserver)
                            }
                        }
                    }
                } catch(_: Exception) {callbacks.post {if(active()) failure()}}
            }
        },main)
        onDispose {
            disposed.set(true);observed?.removeObserver(cameraObserver);observed=null
            analysis.clearAnalyzer();bound?.unbind(preview,analysis)
            // Accepted CameraX runnables must execute their disposed/finally path and close proxies.
            // shutdownNow would discard those runnables before CameraX can complete its frame future.
            executor.shutdown();previewView.controller=null
        }
    }
}

@Composable
private fun Action(label: String, tag: String, enabled: Boolean = true, action: () -> Unit) {
    androidx.compose.material3.TextButton(onClick = action, enabled = enabled, modifier = Modifier.testTag(tag)) { UiText(label) }
}
