// SSH for the phone: one ed25519 key per install (PhoneKey), and `hc connect`
// over an exec channel (no PTY) as the byte stream the protocol client uses.

package dev.herdr.ssh

import android.util.Base64
import android.util.Log
import com.jcraft.jsch.ChannelExec
import com.jcraft.jsch.HostKey
import com.jcraft.jsch.HostKeyRepository
import com.jcraft.jsch.JSch
import com.jcraft.jsch.JSchException
import com.jcraft.jsch.KeyPair
import com.jcraft.jsch.Session
import com.jcraft.jsch.SocketFactory
import com.jcraft.jsch.UIKeyboardInteractive
import com.jcraft.jsch.UserInfo
import herdr.core.Duplex
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.net.UnknownHostException
import java.nio.ByteBuffer
import java.security.MessageDigest
import kotlin.concurrent.thread

data class Target(val host: String, val port: Int, val user: String)

/** A login [Ssh.open] can't fix by trying again: it waits on the person. */
sealed class Refused(cause: JSchException) : Exception(cause.message, cause) {
    /** The host doesn't take the phone's key: it isn't installed there yet. */
    class Key(cause: JSchException) : Refused(cause)

    /** The host shows a key other than the trusted one. */
    class HostKeyChanged(cause: JSchException) : Refused(cause)
}

/** A host's key (base64), with its SHA256 fingerprint as `ssh-keygen -l` prints it and its type. */
data class Seen(val hostKey: String, val fingerprint: String, val hostKeyType: String)

private fun seen(key: ByteArray) = Seen(
    hostKey = Base64.encodeToString(key, Base64.NO_WRAP),
    fingerprint = fingerprint(key),
    // The key blob starts with its type as an SSH string.
    hostKeyType = String(key, 4, ByteBuffer.wrap(key).int, Charsets.US_ASCII),
)

/** `SHA256:…` for a key blob, as `ssh-keygen -l` prints it. */
internal fun fingerprint(blob: ByteArray): String =
    "SHA256:" + Base64.encodeToString(
        MessageDigest.getInstance("SHA-256").digest(blob),
        Base64.NO_WRAP or Base64.NO_PADDING,
    )

private const val TAG = "HerdrSsh"

object Ssh {
    // JSch's defaults for Ed25519, X25519 and ML-KEM are java.security classes
    // from Java 11/15/24, which Android doesn't have; its Bouncy Castle
    // versions do the same with the bcprov we ship.
    init {
        for ((key, impl) in listOf(
            "keypairgen.eddsa" to "KeyPairGenEdDSA",
            "keypairgen_fromprivate.eddsa" to "KeyPairGenEdDSA",
            "ssh-ed25519" to "SignatureEd25519",
            "ssh-ed448" to "SignatureEd448",
            "xdh" to "XDH",
            "mlkem768" to "MLKEM768",
            "mlkem1024" to "MLKEM1024",
        )) {
            JSch.setConfig(key, "com.jcraft.jsch.bc.$impl")
        }
    }

    /** Logs the whole cause chain (the UI only shows the message). */
    private inline fun <T> logged(what: String, block: () -> T): T =
        try {
            block()
        } catch (e: Throwable) {
            Log.e(TAG, "$what failed", e)
            throw e
        }

    /** [logged], on the IO dispatcher, named after what it does to [t]. */
    private suspend fun <T> io(what: String, t: Target, block: () -> T): T =
        withContext(Dispatchers.IO) { logged("$what ${t.user}@${t.host}:${t.port}", block) }

    /** A session with [t] that only accepts the host keys [hosts] does. */
    private fun JSch.session(t: Target, hosts: HostKeyRepository, ui: UserInfo, auth: String? = null): Session {
        hostKeyRepository = hosts
        return getSession(t.user, t.host, t.port).apply {
            setConfig("StrictHostKeyChecking", "yes")
            auth?.let { setConfig("PreferredAuthentications", it) }
            userInfo = ui
            setSocketFactory(EveryAddress)
        }
    }

    /** A new key: its private half (OpenSSH format, for [PhoneKey] to seal) and public line. */
    internal fun generateKey(comment: String): Pair<ByteArray, String> = logged("generateKey") {
        val pair = KeyPair.genKeyPair(JSch(), KeyPair.ED25519)
        val private = ByteArrayOutputStream().also { pair.writeOpenSSHv1PrivateKey(it, null) }.toByteArray()
        val public = ByteArrayOutputStream().also { pair.writePublicKey(it, comment) }.toString("UTF-8").trim()
        pair.dispose()
        private to public
    }

    /**
     * The host's key, without logging in: the key exchange runs and the
     * connection is dropped before authentication, so it costs no login attempt.
     */
    suspend fun probe(t: Target): Seen = io("probe", t) {
        var key: ByteArray? = null
        val session = JSch().session(t, pinned(null) { key = it }, Quiet())
        try {
            session.connect(10_000)
        } catch (e: JSchException) {
            // Rejected as unknown, which is the point; anything else is a real failure.
            key?.let { return@io seen(it) }
            throw e
        } finally {
            session.disconnect()
        }
        error("the host accepted a login it was never offered")
    }

    /**
     * Opens `command` on the host. Throws on auth or a host key other than
     * [hostKey] (pinned, base64); after that, problems arrive as the duplex's close.
     */
    suspend fun open(
        t: Target,
        key: PhoneKey,
        hostKey: String,
        command: String = "hc connect",
    ): Duplex {
        val private = key.privateBytes()
        return io("open", t) {
            val jsch = JSch()
            // JSch parses it into its own copy; ours goes at once.
            try {
                jsch.addIdentity("goatr", private, null, null)
            } finally {
                private.fill(0)
            }
            val session = jsch.session(t, pinned(hostKey), Quiet(), auth = "publickey")
            session.setServerAliveInterval(15_000)
            try {
                session.connect(10_000)
            } catch (e: JSchException) {
                val message = e.message.orEmpty()
                throw when {
                    KEY_REFUSED.containsMatchIn(message) -> Refused.Key(e)
                    message.contains("HostKey", ignoreCase = true) -> Refused.HostKeyChanged(e)
                    else -> e
                }
            }
            val channel = session.openChannel("exec") as ChannelExec
            channel.setCommand(command)
            val output = channel.inputStream
            val input = channel.outputStream
            channel.connect(10_000)
            ChannelDuplex(session, channel, output, input)
        }
    }

    /**
     * Whether the host (showing [hostKey], pinned) takes a password: asked
     * for one, the login is called off before anything is sent, so this costs
     * at most one keyboard-interactive try and never a password.
     */
    suspend fun takesPassword(t: Target, hostKey: String): Boolean = io("takesPassword", t) {
        val asked = Asked()
        val session = JSch().session(t, pinned(hostKey), asked, auth = PASSWORD_AUTH)
        try {
            session.connect(10_000)
        } catch (e: JSchException) {
            // Cancelled at the prompt, or no method left: either way, done.
            if (!asked.password && !e.isAuthFailure) throw e
        } finally {
            session.disconnect()
        }
        asked.password
    }

    /**
     * ssh-copy-id: logs in once with a password (or keyboard-interactive,
     * answered with it) and adds `line` to ~/.ssh/authorized_keys once. Only
     * to a host showing [hostKey]; the password is not kept anywhere.
     */
    suspend fun installKey(t: Target, password: String, hostKey: String, line: String) = io("installKey", t) {
        val session = JSch().session(t, pinned(hostKey), PasswordOnly(password), auth = PASSWORD_AUTH)
        session.setPassword(password.toByteArray())
        try {
            session.connect(15_000)
        } catch (e: JSchException) {
            if (e.isAuthFailure) {
                throw JSchException("password login refused (wrong password, or the host only allows keys)")
            }
            throw e
        }
        try {
            val channel = session.openChannel("exec") as ChannelExec
            channel.setCommand("sh -s")
            val out = channel.inputStream
            val err = ByteArrayOutputStream()
            channel.setErrStream(err)
            val input = channel.outputStream
            channel.connect(10_000)
            input.write(installScript(line).toByteArray())
            input.close()
            val text = out.readBytes().toString(Charsets.UTF_8)
            while (!channel.isClosed) Thread.sleep(20)
            if (channel.exitStatus != 0 || !text.contains("herdr-key-installed")) {
                val why = err.toString("UTF-8").trim().ifEmpty { "exit ${channel.exitStatus}" }
                throw IllegalStateException("could not write ~/.ssh/authorized_keys: $why")
            }
            channel.disconnect()
        } finally {
            session.disconnect()
        }
    }

    private const val PASSWORD_AUTH = "password,keyboard-interactive"

    private val JSchException.isAuthFailure get() = message?.startsWith("Auth") == true

    private val KEY_REFUSED = Regex("Auth (fail|cancel)|publickey", RegexOption.IGNORE_CASE)

    /** Accepts only [hostKey] (base64). */
    private fun pinned(hostKey: String) = pinned(Base64.decode(hostKey, Base64.NO_WRAP))

    /**
     * Accepts only `expected`. With nothing expected it accepts nothing (JSch
     * then drops the connection before authenticating); `saw` gets what the host showed.
     */
    private fun pinned(expected: ByteArray?, saw: (ByteArray) -> Unit = {}) = object : HostKeyRepository {
        override fun check(host: String?, key: ByteArray?): Int {
            if (key != null) saw(key)
            return when {
                expected == null -> HostKeyRepository.NOT_INCLUDED
                expected.contentEquals(key) -> HostKeyRepository.OK
                else -> HostKeyRepository.CHANGED
            }
        }
        override fun add(hostkey: HostKey?, ui: UserInfo?) {}
        override fun remove(host: String?, type: String?) {}
        override fun remove(host: String?, type: String?, key: ByteArray?) {}
        override fun getKnownHostsRepositoryID() = "herdr"
        override fun getHostKey(): Array<HostKey> = emptyArray()
        override fun getHostKey(host: String?, type: String?): Array<HostKey> = emptyArray()
    }
}

/**
 * Tries each address the host resolves to, IPv4 first, instead of only the
 * first one: phones resolve `name.local` to an IPv6 link-local address that
 * often doesn't route, while its IPv4 address does.
 */
private object EveryAddress : SocketFactory {
    override fun createSocket(host: String, port: Int): Socket {
        val addresses = InetAddress.getAllByName(host).sortedBy { if (it is Inet4Address) 0 else 1 }
        var last: Exception? = null
        for (a in addresses) {
            val socket = Socket()
            try {
                socket.connect(InetSocketAddress(a, port), 5_000)
                socket.tcpNoDelay = true
                return socket
            } catch (e: Exception) {
                socket.close()
                last = e
            }
        }
        throw last ?: UnknownHostException(host)
    }
    override fun getInputStream(socket: Socket) = socket.getInputStream()
    override fun getOutputStream(socket: Socket) = socket.getOutputStream()
}

// One exec channel: its stdin/stdout carry hc's framed protocol as raw bytes.
private class ChannelDuplex(
    private val session: Session,
    private val channel: ChannelExec,
    private val output: InputStream,
    private val input: OutputStream,
) : Duplex {
    private var closed: ((String?) -> Unit)? = null
    private var ended = false
    private var why: String? = null

    @Synchronized
    override fun write(bytes: ByteArray) {
        if (ended) return
        runCatching {
            input.write(bytes)
            input.flush()
        }
    }

    // The reader starts with the listener: nothing is read (or lost) before it.
    override fun onData(listener: (ByteArray) -> Unit) {
        thread(name = "herdr-ssh", isDaemon = true) {
            val buf = ByteArray(32 * 1024)
            var reason: String? = null
            try {
                while (true) {
                    val n = output.read(buf)
                    if (n < 0) break
                    listener(buf.copyOf(n))
                }
            } catch (e: Exception) {
                reason = e.message ?: e.javaClass.simpleName
            }
            if (reason == null && channel.exitStatus > 0) reason = "exited ${channel.exitStatus}"
            end(reason ?: "closed")
        }
    }

    @Synchronized
    override fun onClose(listener: (String?) -> Unit) {
        closed = listener
        if (ended) listener(why)
    }

    override fun close() {
        runCatching { channel.disconnect() }
        runCatching { session.disconnect() }
        end("closed")
    }

    private fun end(reason: String) {
        val l = synchronized(this) {
            if (ended) return
            ended = true
            why = reason
            closed
        }
        l?.invoke(reason)
    }
}

// ssh-copy-id: append the line once, never twice, keeping a missing final
// newline from gluing it to the last key. Run by `sh -s` from stdin, so the
// account's own shell (fish, …) never parses it.
private fun installScript(line: String): String {
    val quoted = "'" + line.replace("'", "'\\''") + "'"
    return """
        set -e
        umask 077
        mkdir -p "${'$'}HOME/.ssh"
        f="${'$'}HOME/.ssh/authorized_keys"
        touch "${'$'}f"
        line=$quoted
        if ! grep -qxF "${'$'}line" "${'$'}f"; then
          [ -s "${'$'}f" ] && [ -n "${'$'}(tail -c 1 "${'$'}f")" ] && echo >> "${'$'}f"
          printf '%s\n' "${'$'}line" >> "${'$'}f"
        fi
        echo herdr-key-installed
    """.trimIndent() + "\n"
}

/** Answers nothing: no password, passphrase or yes/no. */
private open class Quiet : UserInfo {
    override fun getPassphrase(): String? = null
    override fun getPassword(): String? = null
    override fun promptPassword(message: String?) = false
    override fun promptPassphrase(message: String?) = false
    override fun promptYesNo(message: String?) = false
    override fun showMessage(message: String?) {}
}

/** Answers password and keyboard-interactive prompts with the one password. */
private class PasswordOnly(private val password: String) : Quiet(), UIKeyboardInteractive {
    override fun getPassword(): String = password
    override fun promptPassword(message: String?) = true

    // Hidden prompts get the password; anything echoed (a username, a
    // one-time code) can't be answered here, so the login fails cleanly.
    override fun promptKeyboardInteractive(
        destination: String?,
        name: String?,
        instruction: String?,
        prompt: Array<String>,
        echo: BooleanArray,
    ): Array<String>? = if (echo.any { it }) null else Array(prompt.size) { password }
}

/** Notes a prompt for a password (hidden input) and cancels it. */
private class Asked : Quiet(), UIKeyboardInteractive {
    var password = false
    override fun promptPassword(message: String?): Boolean {
        password = true
        return false
    }
    override fun promptKeyboardInteractive(
        destination: String?,
        name: String?,
        instruction: String?,
        prompt: Array<String>,
        echo: BooleanArray,
    ): Array<String>? {
        if (echo.any { !it }) password = true
        return null
    }
}
