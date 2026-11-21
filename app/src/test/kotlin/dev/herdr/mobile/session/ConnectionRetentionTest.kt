package io.github.trevarj.goatr.app.session

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotSame
import kotlin.test.assertSame
import kotlin.test.assertTrue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel

class ConnectionRetentionTest {
    @Test
    fun quickReturnRetainsConnectionButReturningAfterSleepStartsANewLease() {
        val scope = CoroutineScope(Dispatchers.Unconfined)
        var now = 100L
        var expirations = 0
        val retention = ConnectionRetention(scope, { now }) { expirations++ }
        try {
            assertFalse(retention.active)
            retention.enterForeground()
            val first = retention.lease.value
            retention.leaveForeground()
            now += ConnectionRetention.BACKGROUND_GRACE_MS - 1
            retention.enterForeground()
            assertSame(first, retention.lease.value)
            assertEquals(0, expirations)

            retention.leaveForeground()
            now += ConnectionRetention.BACKGROUND_GRACE_MS
            retention.enterForeground()
            assertTrue(retention.active)
            assertNotSame(first, retention.lease.value)
            assertEquals(1, expirations)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun alertServiceExtendsTheLeaseOnlyWhileNeeded() {
        val scope = CoroutineScope(Dispatchers.Unconfined)
        var now = 100L
        var expirations = 0
        val retention = ConnectionRetention(scope, { now }) { expirations++ }
        try {
            retention.enterForeground()
            retention.keepForAlerts(true)
            retention.keepForAlerts(false)
            assertTrue(retention.active)
            retention.keepForAlerts(true)
            retention.leaveForeground()
            now += ConnectionRetention.BACKGROUND_GRACE_MS
            assertTrue(retention.active)
            retention.keepForAlerts(false)
            assertFalse(retention.active)
            assertEquals(1, expirations)

            retention.enterForeground()
            assertTrue(retention.active)
        } finally {
            scope.cancel()
        }
    }
}
