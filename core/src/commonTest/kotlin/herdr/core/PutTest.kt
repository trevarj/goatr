package herdr.core

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

@OptIn(ExperimentalCoroutinesApi::class)
class PutTest {
    @Test
    fun sendsTheBytesAndAnswersWhereTheyLanded() = runTest {
        var asked: JsonObject? = null
        val core = FakeCore(this, mutableMapOf("terminal.put" to { p ->
            asked = p
            buildJsonObject {
                put("path", "/home/dev/.local/state/herdr/put/t3/shot.png")
                put("bytes", 3)
            }
        }))
        val client = connect(core.duplex, dispatcher = StandardTestDispatcher(testScheduler))
        val path = put(client, "box/t3", "shot.png", byteArrayOf(1, 2, 3))
        assertEquals("/home/dev/.local/state/herdr/put/t3/shot.png", path)
        assertEquals("box/t3", asked!!.string("terminal"))
        assertEquals("shot.png", asked!!.string("name"))
        assertEquals(listOf<Byte>(1, 2, 3), Base64.decode(asked!!.string("data")!!).toList())
    }

    @Test
    fun anOlderCoreSaysSo() = runTest {
        val older = FAKE_HELLO.copy(methods = FAKE_HELLO.methods - "terminal.put")
        val client = connect(FakeCore(this, mutableMapOf(), older).duplex, dispatcher = StandardTestDispatcher(testScheduler))
        assertFailsWith<IllegalStateException> { put(client, "t1", "a.png", byteArrayOf(1)) }
    }

    @Test
    fun pastesPathsAsDroppedFiles() {
        assertEquals("/tmp/a.png ", pasted(listOf("/tmp/a.png")))
        assertEquals("/tmp/a.png /tmp/b.jpg ", pasted(listOf("/tmp/a.png", "/tmp/b.jpg")))
        assertEquals("/Users/dev/Library/Application\\ Support/herdr/put/t1/IMG\\ \\(1\\).png ",
            pasted(listOf("/Users/dev/Library/Application Support/herdr/put/t1/IMG (1).png")))
        assertEquals("\"C:\\Users\\dev\\App Data\\a.png\" ", pasted(listOf("C:\\Users\\dev\\App Data\\a.png")))
        assertEquals("C:\\Users\\dev\\a.png ", pasted(listOf("C:\\Users\\dev\\a.png")))
    }
}
