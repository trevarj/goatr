// The framed wire format of `hc connect` after `hello` (herdr-app/src/hc.rs):
// a 9-byte header (channel u32 LE, kind u8, length u32 LE) and the payload.
// JSON-RPC runs on channel 0; streams (attach, events) get their own.

package herdr.core

/** One frame. Channels are u32 on the wire; they round-trip through [Int]. */
class Frame(val channel: Int, val kind: Int, val payload: ByteArray) {
    companion object {
        const val DATA = 0
        const val CREDIT = 1
        const val CLOSE = 2
        const val RESIZE = 3
        const val FOCUS = 4
        const val FACTS = 5

        const val HEADER = 9
    }
}

internal val EMPTY = ByteArray(0)

internal const val NL: Byte = 10

fun encode(channel: Int, kind: Int, payload: ByteArray = EMPTY): ByteArray {
    val out = ByteArray(Frame.HEADER + payload.size)
    out.putU32(0, channel)
    out[4] = kind.toByte()
    out.putU32(5, payload.size)
    payload.copyInto(out, Frame.HEADER)
    return out
}

fun u32(n: Int): ByteArray = ByteArray(4).also { it.putU32(0, n) }

/** RESIZE's payload: cols then rows, u16 LE each. */
fun size(cols: Int, rows: Int): ByteArray {
    val out = ByteArray(4)
    out.putU16(0, cols)
    out.putU16(2, rows)
    return out
}

/**
 * Turns bytes, split anywhere, into whole frames. Payloads are copies, so a
 * consumer may keep them after the transport reuses its buffers.
 */
class FrameDecoder {
    private var pending = EMPTY

    fun push(bytes: ByteArray): List<Frame> {
        pending = if (pending.isEmpty()) bytes else pending + bytes
        val frames = mutableListOf<Frame>()
        var at = 0
        while (pending.size - at >= Frame.HEADER) {
            val len = pending.getU32(at + 5)
            if (pending.size - at - Frame.HEADER < len) break
            val start = at + Frame.HEADER
            frames += Frame(
                channel = pending.getU32(at),
                kind = pending[at + 4].toInt() and 0xff,
                payload = pending.copyOfRange(start, start + len),
            )
            at = start + len
        }
        pending = if (at == pending.size) EMPTY else pending.copyOfRange(at, pending.size)
        return frames
    }
}

internal fun ByteArray.putU32(at: Int, n: Int) {
    for (i in 0..3) this[at + i] = (n ushr (8 * i)).toByte()
}

internal fun ByteArray.putU16(at: Int, n: Int) {
    this[at] = n.toByte()
    this[at + 1] = (n ushr 8).toByte()
}

internal fun ByteArray.getU32(at: Int): Int =
    (0..3).fold(0) { n, i -> n or ((this[at + i].toInt() and 0xff) shl (8 * i)) }
