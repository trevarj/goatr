package herdr.core

import kotlinx.io.buffered
import kotlinx.io.files.Path
import kotlinx.io.files.SystemFileSystem
import kotlinx.io.readByteArray

/** The directory the build points `HERDR_FIXTURES` at. */
expect fun fixturesDir(): String

fun fixture(name: String): ByteArray =
    SystemFileSystem.source(Path(fixturesDir(), name)).buffered().use { it.readByteArray() }
