// JVM only: `hc connect` (or `ssh host hc connect`) as a child process.

package herdr.core

import java.io.IOException
import kotlin.concurrent.thread

val HC_CONNECT = listOf("hc", "connect")

/** `argv`: e.g. `listOf("ssh", "host", "hc", "connect")` for another machine. */
class ProcessDuplex(argv: List<String> = HC_CONNECT) : Duplex {
    private val cmd = argv.first()
    private var closed: ((String?) -> Unit)? = null
    private var why: String? = null
    private var ended = false
    private val process: Process? = try {
        ProcessBuilder(argv).redirectError(ProcessBuilder.Redirect.INHERIT).start()
    } catch (e: IOException) {
        end("cannot run $cmd: ${e.message}")
        null
    }

    @Synchronized
    private fun end(reason: String) {
        if (ended) return
        ended = true
        why = reason
        closed?.invoke(reason)
    }

    @Synchronized
    override fun write(bytes: ByteArray) {
        if (ended) return
        // A write after the child died must not throw: the close says it.
        runCatching {
            process?.outputStream?.apply { write(bytes); flush() }
        }
    }

    override fun onData(listener: (ByteArray) -> Unit) {
        val p = process ?: return
        thread(isDaemon = true, name = "hc-read") {
            val buf = ByteArray(64 * 1024)
            val input = p.inputStream
            while (true) {
                val n = runCatching { input.read(buf) }.getOrDefault(-1)
                if (n < 0) break
                listener(buf.copyOf(n))
            }
            val code = runCatching { p.waitFor() }.getOrNull()
            end(if (code != null) "$cmd exited $code" else "$cmd closed its output")
        }
    }

    @Synchronized
    override fun onClose(listener: (String?) -> Unit) {
        closed = listener
        if (ended) listener(why)
    }

    override fun close() {
        runCatching { process?.outputStream?.close() }
        process?.destroy()
    }
}
