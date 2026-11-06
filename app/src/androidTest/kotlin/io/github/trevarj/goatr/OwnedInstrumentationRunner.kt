// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Activity
import android.app.Application
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.AndroidJUnitRunner
import java.io.File
import java.lang.ref.WeakReference
import java.nio.ByteBuffer
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONObject

/** Test-only original-process rendezvous; the signing private key never leaves this invocation. */
class OwnedInstrumentationRunner : AndroidJUnitRunner() {
    internal data class SdkFixtureSelection(val method:String,val activityClass:String,val stockControl:Boolean)
    internal data class StockControlKey(val pair:KeyPair,val publicIdentity:ByteArray)
    internal companion object {
        internal fun sdkFixtureSelection(arguments:Bundle):SdkFixtureSelection? {
            val selection=when(arguments.getString("goatrSdkForegroundBootstrap")) {
                "component_once" -> arguments.getString("class")?.takeIf {
                    it=="io.github.trevarj.goatr.NativeInventoryTest#directoryRowKeepsStatusAndDetailsContainedAtCompactTwoHundredPercent" ||
                        it=="io.github.trevarj.goatr.ConversationContentTest#assistantMessageBackgroundIsTransparentWhileUserAndNativeToolStayContained" ||
                        it=="io.github.trevarj.goatr.ConversationContentTest#productionConversationFrameReservesActionsWithOmpRequestsAtTwoHundredPercent" ||
                        it=="io.github.trevarj.goatr.ConversationContentTest#syntheticGalleryShowsStateBaselineAndLightDarkTextContrast" ||
                        it=="io.github.trevarj.goatr.ConversationContentTest#terminalDisplayScaleRestoresFromIsolatedPreferencesAndKeepsItsCanvasAndControlSynchronized" ||
                        it=="io.github.trevarj.goatr.ConversationContentTest#terminalDisplayScaleFlushesItsCurrentValueWhenCanvasDisposesBeforeDebounceAndRestoresIt"
                }?.let {SdkFixtureSelection(it,"androidx.activity.ComponentActivity",false)} ?: return null
                "main_once" -> arguments.getString("class")?.takeIf {
                    it=="io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_private_pairing_checkpoint" ||
                        it=="io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_owned_retirement_checkpoint"
                }?.let {SdkFixtureSelection(it,"io.github.trevarj.goatr.MainActivity",true)} ?: return null
                else -> return null
            }
            return selection.takeIf {arguments.getString("class")==it.method && (!it.stockControl || arguments.getString("stockControlPort")?.toIntOrNull() in 1..65535) && (it.method!="io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_owned_retirement_checkpoint" || arguments.getString("goatrOwnedRetirementCheckpoint")=="true")}
        }
        internal fun freshStockControlKey():StockControlKey {
            val pair=KeyPairGenerator.getInstance("EC").apply {initialize(ECGenParameterSpec("secp256r1"))}.generateKeyPair()
            val prefix=byteArrayOf(0x30,0x59,0x30,0x13,0x06,0x07,0x2a,0x86.toByte(),0x48,0xce.toByte(),0x3d,0x02,0x01,0x06,0x08,0x2a,0x86.toByte(),0x48,0xce.toByte(),0x3d,0x03,0x01,0x07,0x03,0x42,0x00)
            val encoded=pair.public.encoded
            check(encoded.size==prefix.size+65&&encoded.copyOfRange(0,prefix.size).contentEquals(prefix))
            val public=encoded.copyOfRange(prefix.size,encoded.size)
            check(public[0]==4.toByte())
            return StockControlKey(pair,public)
        }
    }
    private var controlKey: KeyPair? = null
    private var publicIdentity: ByteArray? = null
    private var physicalRequested=false
    private sealed interface PhysicalPreflight {
        data class Ready(val request:JSONObject,val baseline:JSONObject):PhysicalPreflight
        data class Blocked(val code:String):PhysicalPreflight
    }
    private var physicalPreflight:PhysicalPreflight?=null
    private var physicalApplication:Application?=null
    private val applicationCreated=CountDownLatch(1)
    private enum class SdkFixtureState { NotRequested, Requested, Waiting, Ready, Failed }
    private data class SdkFixtureRequest(
        val application:Application,
        val method:String,
        val activityClass:String,
        val stockControl:Boolean,
    )
    private val sdkFixtureLock=Any()
    private var sdkFixtureState=SdkFixtureState.NotRequested
    private var sdkFixtureRequest:SdkFixtureRequest?=null
    private var sdkForegroundActivity:WeakReference<Activity>?=null
    private var sdkForegroundApplication:WeakReference<Application>?=null

    override fun onCreate(arguments:Bundle?) {
        val supplied=arguments ?: Bundle()
        physicalRequested=PhysicalMvpPlatform.requested(supplied)
        if(!physicalRequested) {super.onCreate(arguments);return}
        physicalPreflight=try {
            val request=PhysicalMvpPlatform.arguments(supplied)
            PhysicalMvpPlatform.qualify(context,targetContext,request)
            val baseline=PhysicalMvpPlatform.preRuntime(targetContext,request)
            prepareControlKey()
            PhysicalPreflight.Ready(request,baseline)
        }catch(error:Throwable) {
            PhysicalPreflight.Blocked(if(error.message=="physical_retained_snapshot_unavailable")"retained_state_snapshot_unavailable" else "physical_preflight")
        }
        // Physical state owns the request; never pass rejected JUnit options into SDK initialization.
        super.onCreate(Bundle())
    }

    override fun callApplicationOnCreate(app:Application) {
        if(physicalRequested) {
            val ready=physicalPreflight as? PhysicalPreflight.Ready ?: return
            if(ready.request.getString("mode")!="observe")return // Failed fresh admission/baseline never initializes normal Goatr runtime.
            super.callApplicationOnCreate(app)
            physicalApplication=app
            applicationCreated.countDown()
            return
        }
        super.callApplicationOnCreate(app)
    }

    private fun prepareControlKey() {
        if(controlKey!=null)return
        val key=freshStockControlKey()
        publicIdentity=key.publicIdentity
        controlKey=key.pair
    }

    private fun physicalStart(ready:PhysicalPreflight.Ready) {
        val request=ready.request
        try {
            val process=PhysicalMvpPlatform.process()
            sendStatus(1,Bundle().apply {
                putString("goatrOwner",if(request.getString("mode")=="baseline")"physical-baseline-temporary" else "physical-observer-instrumented")
                putInt("goatrPid",process.getInt("pid"));putString("goatrStart",process.getString("start"))
                putInt("goatrUid",process.getInt("uid"))
                putString("goatrControlPublic",controlIdentity().joinToString("") {"%02x".format(it.toInt() and 255)})
            })
            if(request.getString("mode")=="baseline") {
                val report=JSONObject().put("method","physical_report").put("runId",request.getString("runId")).put("sequence",0)
                    .put("report",JSONObject().put("kind","baseline").put("process",process).put("facts",ready.baseline)
                        .put("dependencyProof",request.getString("physicalMvpDependencyProof")))
                val reply=PhysicalMvpPlatform.exchange(this,request.getInt("port"),report)
                check(reply.keys().asSequence().toSet()==setOf("kind","runId","sequence")&&reply.getString("kind")=="finished"
                    &&reply.getString("runId")==request.getString("runId")&&reply.getLong("sequence")==0L)
            }else {
                check(applicationCreated.await(30,TimeUnit.SECONDS)) {"physical_application_deadline"}
                PhysicalMvpObserver(this,request,checkNotNull(physicalApplication)).run()
            }
            finish(Activity.RESULT_OK,Bundle().apply {putString("goatrPhysicalOutcome",JSONObject().put("status","finished")
                .put("mode",request.getString("mode")).put("pid",process.getInt("pid")).put("start",process.getString("start")).toString())})
        }catch(_:Throwable) {
            finish(Activity.RESULT_CANCELED,Bundle().apply {putString("goatrPhysicalOutcome","{\"status\":\"blocked\",\"code\":\"physical_observer\"}")})
        }
    }

    private fun sdkBootstrapStatus(status:String) {
        sendStatus(1,Bundle().apply {putString("goatrSdkBootstrap",status)})
    }

    private fun snapshotSdkOwnedFixture(arguments:Bundle):SdkFixtureRequest? {
        val selection=sdkFixtureSelection(arguments) ?: return null
        val application=targetContext.applicationContext as? Application
        if(application==null || targetContext.packageName!="io.github.trevarj.goatr.debug" || application.packageName!="io.github.trevarj.goatr.debug") {
            return null
        }
        synchronized(sdkFixtureLock) {
            if(sdkFixtureState!=SdkFixtureState.NotRequested)return null
            sdkFixtureRequest=SdkFixtureRequest(application,selection.method,selection.activityClass,selection.stockControl)
            sdkFixtureState=SdkFixtureState.Requested
        }
        return checkNotNull(sdkFixtureRequest)
    }

    fun isSdkOwnedFixtureRequested():Boolean = synchronized(sdkFixtureLock) {sdkFixtureState==SdkFixtureState.Requested && sdkFixtureRequest!=null}

    fun prepareSdkOwnedFixture() {
        val request=synchronized(sdkFixtureLock) {
            check(sdkFixtureState==SdkFixtureState.Requested)
            sdkFixtureState=SdkFixtureState.Waiting
            checkNotNull(sdkFixtureRequest)
        }
        val application=request.application
        val foreground=CountDownLatch(1)
        val outcome=AtomicReference<String?>(null)
        val callback=object:Application.ActivityLifecycleCallbacks {
            override fun onActivityResumed(activity:Activity) {
                val accepted=activity.javaClass.name==request.activityClass && activity.packageName=="io.github.trevarj.goatr.debug" &&
                    activity.application===application && !activity.isFinishing && !activity.isDestroyed
                val observed=if(accepted) "foreground_resumed" else "mismatch"
                if(outcome.compareAndSet(null,observed)) {
                    if(accepted) {
                        sdkForegroundActivity=WeakReference(activity)
                        sdkForegroundApplication=WeakReference(application)
                    }
                    foreground.countDown()
                }
            }
            override fun onActivityCreated(activity:Activity,savedInstanceState:Bundle?) {}
            override fun onActivityStarted(activity:Activity) {}
            override fun onActivityPaused(activity:Activity) {}
            override fun onActivityStopped(activity:Activity) {}
            override fun onActivitySaveInstanceState(activity:Activity,outState:Bundle) {}
            override fun onActivityDestroyed(activity:Activity) {}
        }
        var registered=false
        var result:String?=null
        try {
            application.registerActivityLifecycleCallbacks(callback)
            registered=true
            sdkBootstrapStatus("awaiting_foreground")
            result=try {
                if(!foreground.await(10,TimeUnit.SECONDS)) "timeout" else outcome.get() ?: "mismatch"
            }catch(_:InterruptedException) {
                Thread.currentThread().interrupt()
                "interruption"
            }
        }catch(_:Throwable) {
            result="admission_failure"
        }finally {
            if(registered) try {application.unregisterActivityLifecycleCallbacks(callback)} catch(_:Throwable) {result="admission_failure"}
        }
        val terminalResult=result ?: "admission_failure"
        when(terminalResult) {
            "foreground_resumed" -> {
                sdkBootstrapStatus("foreground_resumed")
                synchronized(sdkFixtureLock) {check(sdkFixtureState==SdkFixtureState.Waiting);sdkFixtureState=SdkFixtureState.Ready}
            }
            else -> {
                sdkBootstrapStatus("blocked_$terminalResult")
                synchronized(sdkFixtureLock) {sdkFixtureState=SdkFixtureState.Failed}
                throw AssertionError("sdk_fixture_$terminalResult")
            }
        }
    }

    fun sdkOwnedFixtureActivity():ComponentActivity {
        val arguments=InstrumentationRegistry.getArguments()
        check(arguments.getString("goatrSdkForegroundBootstrap")=="component_once" && arguments.getString("class") in setOf(
            "io.github.trevarj.goatr.NativeInventoryTest#directoryRowKeepsStatusAndDetailsContainedAtCompactTwoHundredPercent",
            "io.github.trevarj.goatr.ConversationContentTest#assistantMessageBackgroundIsTransparentWhileUserAndNativeToolStayContained",
            "io.github.trevarj.goatr.ConversationContentTest#productionConversationFrameReservesActionsWithOmpRequestsAtTwoHundredPercent",
            "io.github.trevarj.goatr.ConversationContentTest#syntheticGalleryShowsStateBaselineAndLightDarkTextContrast",
            "io.github.trevarj.goatr.ConversationContentTest#terminalDisplayScaleRestoresFromIsolatedPreferencesAndKeepsItsCanvasAndControlSynchronized",
            "io.github.trevarj.goatr.ConversationContentTest#terminalDisplayScaleFlushesItsCurrentValueWhenCanvasDisposesBeforeDebounceAndRestoresIt",
        ))
        check(synchronized(sdkFixtureLock) {sdkFixtureState==SdkFixtureState.Ready})
        val activity=checkNotNull(sdkForegroundActivity?.get()) as? ComponentActivity
            ?: error("SDK component fixture did not retain a ComponentActivity")
        val application=checkNotNull(sdkForegroundApplication?.get())
        check(activity.javaClass.name=="androidx.activity.ComponentActivity" && activity.packageName=="io.github.trevarj.goatr.debug" &&
            activity.application===application && application===targetContext.applicationContext && application.packageName=="io.github.trevarj.goatr.debug" &&
            !activity.isFinishing && !activity.isDestroyed)
        return activity
    }

    fun sdkOwnedFixtureMainActivity():MainActivity {
        val arguments=InstrumentationRegistry.getArguments()
        check(arguments.getString("goatrSdkForegroundBootstrap")=="main_once" &&
            arguments.getString("class") in setOf(
                "io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_private_pairing_checkpoint",
                "io.github.trevarj.goatr.GoatrConnectionTest#installed_subject_owned_retirement_checkpoint",
            )
        )
        val request=synchronized(sdkFixtureLock) {
            check(sdkFixtureState==SdkFixtureState.Ready)
            checkNotNull(sdkFixtureRequest)
        }
        check(request.activityClass=="io.github.trevarj.goatr.MainActivity" && request.stockControl)
        val activity=checkNotNull(sdkForegroundActivity?.get()) as? MainActivity
            ?: error("SDK pairing fixture did not retain the original MainActivity")
        val application=checkNotNull(sdkForegroundApplication?.get())
        check(activity.javaClass.name==request.activityClass && activity.packageName=="io.github.trevarj.goatr.debug" &&
            activity.application===application && application===targetContext.applicationContext && application.packageName=="io.github.trevarj.goatr.debug" &&
            !activity.isFinishing && !activity.isDestroyed)
        return activity
    }

    override fun onStart() {
        if(physicalRequested) {
            when(val preflight=physicalPreflight) {
                is PhysicalPreflight.Ready -> physicalStart(preflight)
                is PhysicalPreflight.Blocked,null -> {
                    val code=preflight?.code ?: "physical_preflight"
                    finish(Activity.RESULT_CANCELED,Bundle().apply {putString("goatrPhysicalOutcome",JSONObject().put("status","blocked").put("code",code).toString())})
                }
            }
            return
        }
        if(InstrumentationRegistry.getArguments().containsKey("goatrSdkForegroundBootstrap")) {
            val request=snapshotSdkOwnedFixture(InstrumentationRegistry.getArguments())
            if(request==null) {
                sdkBootstrapStatus("blocked_argument")
                finish(Activity.RESULT_CANCELED,Bundle())
                return
            }
            if(request.stockControl) prepareControlKey()
            val pid = android.os.Process.myPid()
            val stat = File("/proc/self/stat").readText()
            check(stat.substringBefore(' ').toInt() == pid) { "Instrumentation kernel PID mismatch" }
            val start = stat.substring(stat.lastIndexOf(')') + 2).trim().split(Regex("\\s+"))[19]
            check(start.toLong() > 0) { "Instrumentation kernel start unavailable" }
            sendStatus(1, Bundle().apply {
                putString("goatrOwner", "fresh-start");putInt("goatrPid", pid);putString("goatrStart", start)
                publicIdentity?.let { key -> putString("goatrControlPublic", key.joinToString("") { "%02x".format(it.toInt() and 255) }) }
            })
            super.onStart()
            return
        }
        val pid = android.os.Process.myPid()
        val stat = File("/proc/self/stat").readText()
        check(stat.substringBefore(' ').toInt() == pid) { "Instrumentation kernel PID mismatch" }
        val start = stat.substring(stat.lastIndexOf(')') + 2).trim().split(Regex("\\s+"))[19]
        check(start.toLong() > 0) { "Instrumentation kernel start unavailable" }
        if (InstrumentationRegistry.getArguments().containsKey("stockControlPort")) {
            prepareControlKey()
        }
        sendStatus(1, Bundle().apply {
            putString("goatrOwner", "fresh-start")
            putInt("goatrPid", pid)
            putString("goatrStart", start)
            publicIdentity?.let { key -> putString("goatrControlPublic", key.joinToString("") { "%02x".format(it.toInt() and 255) }) }
        })
        super.onStart()
    }

    fun controlIdentity(): ByteArray = publicIdentity?.copyOf()
        ?: error("Owned stock instrumentation signing identity unavailable")

    fun controlSignature(challenge: ByteArray, request: ByteArray): ByteArray {
        check(challenge.size == 32 && request.size in 1..65536) { "Owned stock signature bounds" }
        return Signature.getInstance("SHA256withECDSA").run {
            initSign(controlKey?.private ?: error("Owned stock instrumentation signing key unavailable"))
            update("GOATR-STOCK-CONTROL-v1\u0000".toByteArray(Charsets.US_ASCII))
            update(challenge)
            update(ByteBuffer.allocate(4).putInt(request.size).array())
            update(request)
            sign().also { check(it.size in 8..72) { "Platform P-256 signature bound" } }
        }
    }
}
