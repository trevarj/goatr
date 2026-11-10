// One byte stream to `hc connect`: an SSH exec channel on the phone, a child
// process on the JVM (ProcessDuplex), an in-memory pipe in tests.

package herdr.core

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

interface Duplex {
    /** Thread-safe; bytes are not kept after the call. */
    fun write(bytes: ByteArray)

    /** One listener; bytes may be split anywhere. */
    fun onData(listener: (ByteArray) -> Unit)

    /** Called once, when the stream ends for any reason (`reason` for the UI). */
    fun onClose(listener: (String?) -> Unit)

    fun close()
}

// One end: what arrives before a listener is kept for it.
private class Endpoint {
    private var data: ((ByteArray) -> Unit)? = null
    private var closed: ((String?) -> Unit)? = null
    private var ended = false
    private val queue = mutableListOf<ByteArray>()

    fun deliver(bytes: ByteArray) {
        if (ended) return
        val listener = data
        if (listener != null) listener(bytes) else queue.add(bytes)
    }

    fun end(reason: String?) {
        if (ended) return
        ended = true
        closed?.invoke(reason)
    }

    fun onData(listener: (ByteArray) -> Unit) {
        data = listener
        queue.forEach(listener)
        queue.clear()
    }

    fun onClose(listener: (String?) -> Unit) {
        closed = listener
        if (ended) listener(null)
    }
}

/**
 * Two connected duplexes, for tests and fakes: what one writes the other
 * reads. Delivery is launched on `scope`, so replies never arrive inside write().
 */
fun pipe(scope: CoroutineScope): Pair<Duplex, Duplex> {
    val a = Endpoint()
    val b = Endpoint()
    fun side(me: Endpoint, other: Endpoint) = object : Duplex {
        override fun write(bytes: ByteArray) {
            val copy = bytes.copyOf()
            scope.launch { other.deliver(copy) }
        }
        override fun onData(listener: (ByteArray) -> Unit) = me.onData(listener)
        override fun onClose(listener: (String?) -> Unit) = me.onClose(listener)

        // After the writes already in flight, like a socket.
        override fun close() {
            scope.launch {
                me.end("closed")
                other.end("closed by peer")
            }
        }
    }
    return side(a, b) to side(b, a)
}
