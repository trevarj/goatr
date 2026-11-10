package herdr.core

actual fun fixturesDir(): String = checkNotNull(System.getenv("HERDR_FIXTURES"))
