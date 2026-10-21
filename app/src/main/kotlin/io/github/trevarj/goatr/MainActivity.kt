// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Intent
import android.os.Bundle
import android.os.Build
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.core.view.WindowCompat
import androidx.lifecycle.ViewModelProvider

class MainActivity : ComponentActivity() {
    private val owner by lazy {ViewModelProvider(this,ViewModelProvider.AndroidViewModelFactory.getInstance(application))[GoatrViewModel::class.java]}
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // Compose selects its own palette; prevent the light platform theme from recoloring it.
            window.decorView.setForceDarkAllowed(false)
        }
        setContent {GoatrTheme(owner.appearance) {
            val background = MaterialTheme.colorScheme.background
            val lightSystemBars = background.luminance() > 0.5f
            SideEffect {
                WindowCompat.getInsetsController(window, window.decorView).apply {
                    isAppearanceLightStatusBars = lightSystemBars
                    isAppearanceLightNavigationBars = lightSystemBars
                }
                @Suppress("DEPRECATION")
                if (Build.VERSION.SDK_INT in Build.VERSION_CODES.O..Build.VERSION_CODES.P) {
                    window.navigationBarColor = background.toArgb()
                }
            }
            GoatrApp(owner) {sensitive->if(sensitive) window.addFlags(WindowManager.LayoutParams.FLAG_SECURE) else window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)}
        }}
        handleAttentionIntent(intent)
    }
    override fun onStart() {super.onStart();owner.foreground(true)}
    override fun onStop() {if(!isChangingConfigurations) owner.foreground(false);super.onStop()}
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleAttentionIntent(intent)
    }
    private fun handleAttentionIntent(intent: Intent?) {
        if(intent?.action!=NotificationCoordinator.ACTION_OPEN_ATTENTION) return
        val host=intent.getStringExtra(NotificationCoordinator.EXTRA_HOST) ?: return
        val event=intent.getStringExtra(NotificationCoordinator.EXTRA_EVENT) ?: return
        val route=intent.data
        if(route?.scheme!="goatr" || route.authority!="attention" || route.pathSegments?.size!=2 || route.pathSegments?.get(0)!=host || route.pathSegments?.get(1)!=event) return
        owner.openNotification(host,event)
    }
}
