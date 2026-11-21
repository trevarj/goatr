package io.github.trevarj.goatr.app.session

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

internal class ConnectionRetention(
    private val scope: CoroutineScope,
    private val now: () -> Long,
    private val expire: () -> Unit,
) {
    private val _lease = MutableStateFlow<Any?>(null)
    val lease = _lease.asStateFlow()
    val active: Boolean get() = lease.value != null
    private val _foreground = MutableStateFlow(false)
    val foreground = _foreground.asStateFlow()
    private var deadline: Long? = null
    private var alerts = false
    private var timer: Job? = null

    fun enterForeground() {
        expireIfDue()
        timer?.cancel()
        _foreground.value = true
        deadline = null
        if (!active) _lease.value = Any()
    }
    fun leaveForeground() {
        _foreground.value = false
        deadline = now() + BACKGROUND_GRACE_MS
        scheduleExpiry()
    }

    fun keepForAlerts(enabled: Boolean) {
        if (enabled == alerts) return
        alerts = enabled
        if (enabled && !active) _lease.value = Any()
        scheduleExpiry()
    }

    private fun scheduleExpiry() {
        timer?.cancel()
        if (alerts || _foreground.value || !active) return
        val remaining = deadline?.minus(now())
        when {
            remaining == null || remaining <= 0 -> expireIfDue()
            else -> timer = scope.launch {
                delay(remaining)
                expireIfDue()
            }
        }
    }

    private fun expireIfDue() {
        if (alerts || _foreground.value || !active || deadline?.let { now() < it } == true) return
        _lease.value = null
        expire()
    }

    companion object {
        internal const val BACKGROUND_GRACE_MS = 3 * 60 * 1000L
    }
}
