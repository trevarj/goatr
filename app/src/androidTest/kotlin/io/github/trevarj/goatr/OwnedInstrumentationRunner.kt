// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.AndroidJUnitRunner
import java.io.File
import java.nio.ByteBuffer
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/** Test-only original-process rendezvous; the signing private key never leaves this invocation. */
class OwnedInstrumentationRunner : AndroidJUnitRunner() {
    private var controlKey: KeyPair? = null
    private var publicIdentity: ByteArray? = null

    override fun onStart() {
        val pid = android.os.Process.myPid()
        val stat = File("/proc/self/stat").readText()
        check(stat.substringBefore(' ').toInt() == pid) { "Instrumentation kernel PID mismatch" }
        val start = stat.substring(stat.lastIndexOf(')') + 2).trim().split(Regex("\\s+"))[19]
        check(start.toLong() > 0) { "Instrumentation kernel start unavailable" }
        if (InstrumentationRegistry.getArguments().containsKey("stockControlPort")) {
            val pair = KeyPairGenerator.getInstance("EC").apply {
                initialize(ECGenParameterSpec("secp256r1"))
            }.generateKeyPair()
            // Standard P-256 SubjectPublicKeyInfo prefix followed by its SEC1 uncompressed point.
            val prefix = byteArrayOf(0x30,0x59,0x30,0x13,0x06,0x07,0x2a,0x86.toByte(),0x48,0xce.toByte(),0x3d,0x02,0x01,0x06,0x08,0x2a,0x86.toByte(),0x48,0xce.toByte(),0x3d,0x03,0x01,0x07,0x03,0x42,0x00)
            val encoded = pair.public.encoded
            check(encoded.size == prefix.size + 65 && encoded.copyOfRange(0,prefix.size).contentEquals(prefix)) { "Unsupported platform P-256 public encoding" }
            publicIdentity = encoded.copyOfRange(prefix.size,encoded.size).also {
                check(it[0] == 4.toByte()) { "Platform public point is not uncompressed" }
            }
            controlKey = pair
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
