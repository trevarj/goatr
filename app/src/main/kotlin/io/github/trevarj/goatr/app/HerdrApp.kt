package io.github.trevarj.goatr.app

import android.app.Application
import android.content.Context
import io.github.trevarj.goatr.app.ui.Svgs
import kotlinx.coroutines.MainScope

class HerdrApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        container = AppContainer(this)
        Svgs.preload(this, MainScope())
        // The phone's key exists from the first launch; the last host reconnects.
        container.live.boot()
    }
}

/** The app's dependencies, from any of its contexts. */
val Context.container: AppContainer get() = (applicationContext as HerdrApp).container
