// The phone's SSH key: ed25519, made here, one per device. The private
// half is kept sealed (AES-GCM, with a key the Android Keystore holds and
// never hands out) in no_backup/, which Android leaves out of every backup
// and transfer. It's opened for each login, given to JSch and wiped; the only
// other way out is [PhoneKey.privateKey] for showing it, which the app gates
// behind the device's lock.

package dev.herdr.ssh

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** `SHA256:…`, as `ssh-keygen -l` prints it, for a public key line. */
fun fingerprintOf(publicKey: String): String =
    fingerprint(Base64.decode(publicKey.trim().split(Regex("""\s+""")).getOrElse(1) { "" }, Base64.DEFAULT))

/** This install's key. [comment] names it in authorized_keys. */
class PhoneKey(context: Context, private val comment: String) {
    private val file = File(context.noBackupFilesDir, "ssh-key")
    private val lock = Mutex()

    /** What's on disk, and all that's kept in memory. */
    private class Saved(val public: String, val sealed: String)

    private var saved: Saved? = null

    private val _public = MutableStateFlow<String?>(null)

    /** The public half (`ssh-ed25519 AAAA… comment`) once loaded, and again each time it changes. */
    val public: StateFlow<String?> = _public.asStateFlow()

    /** The public half, made on first use. */
    suspend fun publicKey(): String = load().public

    /** The private half for logins (JSch only). Wipe it once handed over. */
    internal suspend fun privateBytes(): ByteArray {
        val sealed = load().sealed
        return withContext(Dispatchers.IO) { Vault.open(sealed) }
    }

    /** The private half as text (OpenSSH format), to show. Ask for the device's lock first. */
    suspend fun privateKey(): String {
        val bytes = privateBytes()
        return String(bytes, Charsets.UTF_8).also { bytes.fill(0) }
    }

    /** A new ed25519 key in place of this one. Hosts that trusted the old one won't take this. */
    suspend fun regenerate() = lock.withLock {
        keep(withContext(Dispatchers.IO) { make() })
    }

    private suspend fun load(): Saved = lock.withLock {
        saved ?: withContext(Dispatchers.IO) { read() ?: make() }.also {
            // The public half isn't secret; it's in the log for setup scripts.
            Log.i("Goatr", "public key: ${it.public}")
            keep(it)
        }
    }

    private fun keep(key: Saved) {
        saved = key
        _public.value = key.public
    }

    private fun read(): Saved? {
        if (!file.exists()) return null
        val lines = file.readLines()
        if (lines.size < 2) return null
        val key = Saved(lines[0], lines[1])
        // Sealed by this device's Keystore: unreadable after a restore elsewhere.
        return key.takeIf { runCatching { Vault.open(it.sealed).fill(0) }.isSuccess }
    }

    // A new key, sealed and saved: a new file, then renamed over the old, so
    // it's never half written.
    private fun make(): Saved {
        val (private, public) = Ssh.generateKey(comment)
        val key = Saved(public, Vault.seal(private))
        private.fill(0)
        val next = File(file.parentFile, "${file.name}.new")
        next.writeText("${key.public}\n${key.sealed}\n")
        if (!next.renameTo(file)) error("couldn't save the SSH key")
        return key
    }
}

// AES-GCM with a key held by the Android Keystore.
private object Vault {
    private const val ALIAS = "herdr.ssh"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"

    private fun secret(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return gen.generateKey()
    }

    fun seal(plain: ByteArray): String {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secret())
        return Base64.encodeToString(cipher.iv + cipher.doFinal(plain), Base64.NO_WRAP)
    }

    fun open(sealed: String): ByteArray {
        val bytes = Base64.decode(sealed, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, secret(), GCMParameterSpec(128, bytes, 0, 12))
        return cipher.doFinal(bytes, 12, bytes.size - 12)
    }
}
