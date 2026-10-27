// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Activity
import android.app.Application
import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.AndroidJUnitRunner
import java.io.File
import java.nio.ByteBuffer
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.json.JSONObject

/** Test-only original-process rendezvous; the signing private key never leaves this invocation. */
class OwnedInstrumentationRunner : AndroidJUnitRunner() {
    private var controlKey: KeyPair? = null
    private var publicIdentity: ByteArray? = null
    private var physicalRequested=false
    private var physicalArguments:JSONObject?=null
    private var physicalBaseline:JSONObject?=null
    private var physicalApplication:Application?=null
    private val applicationCreated=CountDownLatch(1)

    override fun onCreate(arguments:Bundle?) {
        val supplied=arguments ?: Bundle()
        physicalRequested=PhysicalMvpPlatform.requested(supplied)
        if(!physicalRequested) {super.onCreate(arguments);return}
        try {
            val request=PhysicalMvpPlatform.arguments(supplied)
            PhysicalMvpPlatform.qualify(context,targetContext,request)
            physicalBaseline=PhysicalMvpPlatform.preRuntime(targetContext,request)
            physicalArguments=request
            prepareControlKey()
            // Instrumentation.start calls this class's plain onStart, not JUnit.
            start()
        }catch(error:Throwable) {
            val code=if(error.message=="physical_retained_snapshot_unavailable")"retained_state_snapshot_unavailable" else "physical_preflight"
            finish(Activity.RESULT_CANCELED,Bundle().apply {putString("goatrPhysicalOutcome",JSONObject().put("status","blocked").put("code",code).toString())})
        }
    }

    override fun callApplicationOnCreate(app:Application) {
        if(physicalRequested) {
            val mode=physicalArguments?.getString("mode")
            if(mode!="observe")return // Includes failed baseline admission: never initialize the old product runtime.
            app.onCreate()
            physicalApplication=app
            applicationCreated.countDown()
            return
        }
        super.callApplicationOnCreate(app)
    }

    private fun prepareControlKey() {
        if(controlKey!=null)return
        val pair=KeyPairGenerator.getInstance("EC").apply {initialize(ECGenParameterSpec("secp256r1"))}.generateKeyPair()
        val prefix=byteArrayOf(0x30,0x59,0x30,0x13,0x06,0x07,0x2a,0x86.toByte(),0x48,0xce.toByte(),0x3d,0x02,0x01,0x06,0x08,0x2a,0x86.toByte(),0x48,0xce.toByte(),0x3d,0x03,0x01,0x07,0x03,0x42,0x00)
        val encoded=pair.public.encoded
        check(encoded.size==prefix.size+65&&encoded.copyOfRange(0,prefix.size).contentEquals(prefix))
        publicIdentity=encoded.copyOfRange(prefix.size,encoded.size).also {check(it[0]==4.toByte())}
        controlKey=pair
    }

    private fun physicalStart(request:JSONObject) {
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
                    .put("report",JSONObject().put("kind","baseline").put("process",process).put("facts",checkNotNull(physicalBaseline))
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

    override fun onStart() {
        physicalArguments?.let {physicalStart(it);return}
        check(!physicalRequested)
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
