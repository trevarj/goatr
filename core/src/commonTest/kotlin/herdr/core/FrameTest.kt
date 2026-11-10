package herdr.core

import kotlin.test.Test
import kotlin.test.assertEquals

class FrameTest {
    private fun want(frames: List<Frame>) =
        frames.map { Triple(it.channel, it.kind, it.payload.decodeToString()) }

    @Test
    fun aFrameIsChannelKindLengthPayload() {
        assertEquals(
            listOf(4, 3, 2, 1, Frame.CREDIT, 4, 0, 0, 0, 0, 4, 0, 0),
            encode(0x01020304, Frame.CREDIT, u32(1024)).map { it.toInt() and 0xff },
        )
        assertEquals(listOf(80, 0, 44, 1), size(80, 300).map { it.toInt() and 0xff })
        assertEquals(9, encode(7, Frame.RESIZE).size)
    }

    @Test
    fun framesSurviveBeingSplitAtEveryByte() {
        val wire = encode(0, Frame.DATA, """{"id":1}""".encodeToByteArray()) +
            encode(3, Frame.DATA) +
            encode(9, Frame.DATA, "héllo".encodeToByteArray())
        for (at in 0..wire.size) {
            val decode = FrameDecoder()
            val got = decode.push(wire.copyOfRange(0, at)) + decode.push(wire.copyOfRange(at, wire.size))
            assertEquals(
                listOf(
                    Triple(0, Frame.DATA, """{"id":1}"""),
                    Triple(3, Frame.DATA, ""),
                    Triple(9, Frame.DATA, "héllo"),
                ),
                want(got),
            )
        }
        val decode = FrameDecoder()
        assertEquals(3, wire.flatMap { decode.push(byteArrayOf(it)) }.size)
    }

    @Test
    fun decodedPayloadsAreCopies() {
        val wire = encode(1, Frame.DATA, "abc".encodeToByteArray())
        val f = FrameDecoder().push(wire).single()
        wire.fill(0)
        assertEquals("abc", f.payload.decodeToString())
    }

    @Test
    fun highChannelsRoundTrip() {
        val f = FrameDecoder().push(encode(-1, Frame.CLOSE)).single()
        assertEquals(-1, f.channel)
        assertEquals(0xffffffffu, f.channel.toUInt())
    }
}
