// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Application

/** Process-wide owner for the one durable store, key provider, and Iroh endpoint. */
class GoatrApplication : Application() {
    lateinit var runtime: GoatrRuntime
        private set

    override fun onCreate() {
        super.onCreate()
        runtime=GoatrRuntime(this)
    }
}
