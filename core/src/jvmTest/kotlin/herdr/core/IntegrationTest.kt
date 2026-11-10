// Against an explicitly selected owned test core (`hc connect`, or HERDR_CONNECT for an owned remote).
// Read-only: never point this at an existing user terminal.
//   HERDR_IT=1 ./gradlew :core:jvmTest --tests '*IntegrationTest*'

package herdr.core

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assume.assumeTrue
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class IntegrationTest {
    @Test
    fun aRealCoreHelloListSubscribeWatchATerminal() {
        assumeTrue("Requires an explicitly owned hc target and HERDR_IT=1", System.getenv("HERDR_IT") == "1")
        val argv = System.getenv("HERDR_CONNECT")?.split(" ") ?: HC_CONNECT
        runBlocking {
            withTimeout(30_000) {
                val c = connect(ProcessDuplex(argv))
                try {
                    assertEquals(1, c.hello.generation)
                    val list = c.callAs<Snapshot>("terminal.list").terminals
                    val target = list.find { '/' !in it.address } ?: list.first()

                    val sub = c.subscribe()
                    assertTrue(sub.lines.live().any { it.address == target.address })

                    // Its own size and observe: the core never takes a size or input from us.
                    val watch = c.attach(
                        target.address,
                        AttachOptions(target.cols, target.rows, history = 200, observe = true),
                    )
                    val n = AtomicInteger()
                    val first = CompletableDeferred<Unit>()
                    watch.onData {
                        n.addAndGet(it.size)
                        watch.consumed(it.size)
                        first.complete(Unit)
                    }
                    first.await()
                    delay(300)
                    assertTrue(n.get() > 0, "history and screen bytes")
                    watch.close()
                    sub.close()

                    val session = Session(CoroutineScope(Dispatchers.Default.limitedParallelism(1)))
                    session.use(c)
                    val snap = session.snapshot.first { it.terminals.isNotEmpty() }
                    assertEquals(Connection.Connected, snap.connection)
                    session.dispose()
                } finally {
                    c.close()
                }
            }
        }
    }
}
