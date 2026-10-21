// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.nio.file.Files
import java.nio.file.LinkOption
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

class ProtectedKeyUnavailable : Exception("Protected installation material is unavailable; restore access or explicitly Forget")

/** Actual AndroidKeyStore AES-GCM; private atomic ciphertext, never plaintext preferences/cache.
 * A retained alias, ciphertext or host reference prohibits silent identity/key replacement. */
class SecretStore(context: Context) {
    companion object {
        const val ENDPOINT_ALIAS = "goatr.installation.endpoint"
        fun hostAlias(host: String, kind: String): String {
            EndpointId(host); require(kind == "receipt" || kind == "push")
            val bytes = MessageDigest.getInstance("SHA-256").digest(host.toByteArray(Charsets.UTF_8))
            val digest = StringBuilder(64)
            for(byte in bytes) {val n=byte.toInt() and 255;digest.append("0123456789abcdef"[n ushr 4]).append("0123456789abcdef"[n and 15])}
            return "goatr.host.$digest.$kind"
        }
    }
    // Android's trusted app-data base may use a platform alias; reject aliases below that base.
    private val directory = File(context.applicationContext.noBackupFilesDir.canonicalFile, "goatr-keys")
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val random = SecureRandom()
    private val mutex = Mutex()
    init {
        val created = !Files.exists(directory.toPath(),LinkOption.NOFOLLOW_LINKS)
        if (created && !directory.mkdir()) throw ProtectedKeyUnavailable()
        if (directory.canonicalFile != directory.absoluteFile) throw ProtectedKeyUnavailable()
        val stat = Os.lstat(directory.path)
        if (!OsConstants.S_ISDIR(stat.st_mode) || stat.st_uid != Os.getuid() || !created && stat.st_mode and 0xfff != 0x1c0) throw ProtectedKeyUnavailable()
        if (created) Os.chmod(directory.path, 0x1c0)
    }
    private fun file(alias: String): File {
        require(alias == ENDPOINT_ALIAS || alias.matches(Regex("goatr\\.host\\.[0-9a-f]{64}\\.(receipt|push)")))
        return File(directory, "$alias.gcm")
    }
    private fun checkFile(file: File) {
        val stat = Os.lstat(file.path)
        if (!OsConstants.S_ISREG(stat.st_mode) || stat.st_uid != Os.getuid() || stat.st_mode and 0xfff != 0x180 || stat.st_nlink != 1L) throw ProtectedKeyUnavailable()
    }
    private fun exists(file: File): Boolean = Files.exists(file.toPath(),LinkOption.NOFOLLOW_LINKS)
    suspend fun material(alias: String, allowCreate: Boolean): ByteArray = withContext(Dispatchers.IO) {
        mutex.withLock {
            val base = file(alias)
            val atomic = AtomicFile(base)
            val marker = File(base.path+".initialized")
            val initialized = exists(marker)
            if (initialized) {checkFile(marker); if (marker.length()!=4L || !marker.readBytes().contentEquals(byteArrayOf(71,65,73,49))) throw ProtectedKeyUnavailable()}
            // AtomicFile's backup restores an interrupted write, not a replacement identity.
            for (path in listOf(base,File(base.path+".bak"),File(base.path+".new"))) if (exists(path)) checkFile(path)
            val exists = exists(base) || exists(File(base.path + ".bak"))
            if (!exists) {
                if (!allowCreate || initialized || keyStore.containsAlias(alias)) throw ProtectedKeyUnavailable()
                // A durable nonsecret marker makes loss of BOTH ciphertext and alias fail closed.
                if (!marker.createNewFile()) throw ProtectedKeyUnavailable()
                Os.chmod(marker.path,0x180)
                java.io.FileOutputStream(marker).use {out->out.write(byteArrayOf(71,65,73,49));out.fd.sync()}
                val key = try {
                    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
                    generator.init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                        .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setRandomizedEncryptionRequired(true).build())
                    generator.generateKey()
                } catch (_:Exception) {throw ProtectedKeyUnavailable()}
                val material = ByteArray(32).also(random::nextBytes)
                var stream: java.io.FileOutputStream? = null
                try {
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key); updateAAD(alias.toByteArray(Charsets.UTF_8)) }
                    val iv = cipher.iv
                    if (iv.size != 12) throw ProtectedKeyUnavailable()
                    val encrypted = cipher.doFinal(material)
                    stream = atomic.startWrite(); Os.fchmod(stream.fd, 0x180) // API26 has no .new convention.
                    stream.write(byteArrayOf(1)); stream.write(iv); stream.write(encrypted)
                    atomic.finishWrite(stream); stream = null; checkFile(base)
                    return@withLock material
                } catch (_: Exception) {
                    stream?.let(atomic::failWrite); material.fill(0)
                    // Keep the alias: interrupted creation is a fail-closed repair obligation.
                    throw ProtectedKeyUnavailable()
                }
            }
            try {
                if (!initialized || !keyStore.containsAlias(alias)) throw ProtectedKeyUnavailable()
                val backup = File(base.path + ".bak"); if (backup.exists()) checkFile(backup)
                if (base.exists()) checkFile(base)
                val encrypted = atomic.openRead().use { input ->
                    val bytes = ByteArray(62); var at = 0
                    while (at < bytes.size) { val count = input.read(bytes,at,bytes.size-at); if (count < 0) break; at += count }
                    if (at != 61 || bytes[0].toInt() != 1) throw ProtectedKeyUnavailable()
                    bytes.copyOf(61)
                }
                checkFile(base)
                val key = keyStore.getKey(alias,null) as? SecretKey ?: throw ProtectedKeyUnavailable()
                val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE,key,GCMParameterSpec(128,encrypted.copyOfRange(1,13))); updateAAD(alias.toByteArray(Charsets.UTF_8)) }
                cipher.doFinal(encrypted.copyOfRange(13,61)).also { if (it.size != 32) {it.fill(0);throw ProtectedKeyUnavailable()} }
            } catch (_: Exception) { throw ProtectedKeyUnavailable() }
        }
    }
    suspend fun endpoint(store: SessionStore): ByteArray {
        store.open()
        return material(ENDPOINT_ALIAS, store.state.value.hosts.isEmpty() && store.state.value.keyDeletion.none { ENDPOINT_ALIAS in it.aliases })
    }
    suspend fun receipt(host: String, store: SessionStore): ByteArray {
        val alias = hostAlias(host,"receipt")
        val retained = store.state.value.hosts.any { it.hostId == host && alias in it.keyAliases } || store.state.value.keyDeletion.any {alias in it.aliases}
        return material(alias,!retained)
    }
    internal suspend fun pushSeal(host:String,generation:Uuid,bytes:ByteArray):ByteArray {
        require(bytes.size<=16384);val key=material(hostAlias(host,"push"),false)
        return try{val cipher=Cipher.getInstance("AES/GCM/NoPadding").apply{init(Cipher.ENCRYPT_MODE,javax.crypto.spec.SecretKeySpec(key,"AES"));updateAAD("goatr push local $host ${generation.value}".toByteArray(Charsets.UTF_8))};byteArrayOf(1)+cipher.iv+cipher.doFinal(bytes)}finally{key.fill(0)}
    }
    internal suspend fun pushOpen(host:String,generation:Uuid,bytes:ByteArray):ByteArray {
        if(bytes.size !in 29..16413||bytes[0].toInt()!=1)throw ProtectedKeyUnavailable()
        val key=material(hostAlias(host,"push"),false)
        return try{Cipher.getInstance("AES/GCM/NoPadding").run{init(Cipher.DECRYPT_MODE,javax.crypto.spec.SecretKeySpec(key,"AES"),GCMParameterSpec(128,bytes.copyOfRange(1,13)));updateAAD("goatr push local $host ${generation.value}".toByteArray(Charsets.UTF_8));doFinal(bytes.copyOfRange(13,bytes.size))}}catch(_:Exception){throw ProtectedKeyUnavailable()}finally{key.fill(0)}
    }
    suspend fun delete(obligation: KeyDeletion, store: SessionStore) = withContext(Dispatchers.IO) {
        mutex.withLock {
            for (alias in obligation.aliases) {
                if (store.state.value.hosts.any {alias in it.keyAliases} || alias == ENDPOINT_ALIAS && GoatrConnection.endpointIsOwned()) throw ProtectedKeyUnavailable()
                val base = file(alias)
                for (path in listOf(base,File(base.path+".bak"),File(base.path+".new"))) {
                    if (exists(path)) {checkFile(path); if (!path.delete()) throw ProtectedKeyUnavailable()}
                }
                if (keyStore.containsAlias(alias)) keyStore.deleteEntry(alias)
                val marker=File(base.path+".initialized")
                if(exists(marker)) {checkFile(marker);if(!marker.delete()) throw ProtectedKeyUnavailable()}
                if (keyStore.containsAlias(alias) || exists(base) || exists(File(base.path+".bak")) || exists(File(base.path+".new")) || exists(marker)) throw ProtectedKeyUnavailable()
            }
            // No acknowledgement until the actual app-owned ciphertext and wrapping aliases are gone.
            store.keysDeleted(obligation)
        }
    }
    suspend fun completeDeletions(store: SessionStore) {store.open(); for (obligation in store.state.value.keyDeletion) delete(obligation,store)}
}
