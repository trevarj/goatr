// The tablet's one screen, as the desktop's main layout: a sidebar (brand and
// connection, spaces over agents, settings at the foot) beside the open
// terminal and its tabs. The phone's lists and terminal, arranged side by side.
// The sidebar folds to a rail, as the desktop's does.

package io.github.trevarj.goatr.app

import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.app.session.Prefs
import io.github.trevarj.goatr.app.ui.AgentsList
import io.github.trevarj.goatr.app.ui.ConnectionBadge
import io.github.trevarj.goatr.app.ui.Lucide
import io.github.trevarj.goatr.app.ui.SettingsScreen
import io.github.trevarj.goatr.app.ui.SpacesList
import io.github.trevarj.goatr.app.ui.TerminalHost
import io.github.trevarj.goatr.app.ui.TerminalScreen

/** The desktop sidebar's width (layout `main`), and its rail's, sized for a finger. */
private val SIDEBAR = 268.dp
private val RAIL = 64.dp

@Composable
fun WideApp(main: MainViewModel, host: TerminalHost, prefs: Prefs) {
    val session by main.session.collectAsStateWithLifecycle()
    var settings by rememberSaveable { mutableStateOf(false) }
    val rail = !prefs.sidebar
    val width by animateDpAsState(if (rail) RAIL else SIDEBAR, label = "sidebar")
    val open: (String) -> Unit = { address ->
        main.select(address)
        main.start()
        settings = false
    }

    // Links (a notification, goatr://…) pick what the one screen shows.
    fun follow(intent: Intent?) {
        val uri = intent?.data?.takeIf { it.scheme == BuildConfig.SCHEME } ?: return
        when (uri.host) {
            "terminal" -> uri.getQueryParameter("address")?.let(open) ?: run { settings = false }
            "settings" -> settings = true
            else -> settings = false
        }
    }
    // Back closes Settings first, as leaving it on the phone does.
    BackHandler(enabled = settings) { settings = false }
    val activity = LocalActivity.current
    LaunchedEffect(Unit) { follow(activity?.intent) }
    OnNewIntent(::follow)

    Row(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.systemBars)) {
        Column(Modifier.width(width).fillMaxHeight()) {
            Row(
                Modifier.fillMaxWidth().height(46.dp).padding(start = if (rail) 0.dp else 16.dp, end = if (rail) 0.dp else 4.dp),
                horizontalArrangement = if (rail) Arrangement.Center else Arrangement.Start,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (!rail) {
                    Image(painterResource(R.drawable.ic_goatr), null, Modifier.size(20.dp))
                    Text(
                        "Goatr",
                        Modifier.padding(start = 8.dp).weight(1f),
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                    )
                    ConnectionBadge(session.host, session.connection, end = 0.dp)
                }
                IconButton(main::toggleSidebar) {
                    Icon(Lucide.PanelLeft, if (rail) "Show sidebar" else "Hide sidebar", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Section("Spaces", rail, Modifier.weight(1f)) { SpacesList(appViewModel(), open, dense = true, rail = rail) }
            HorizontalDivider()
            Section("Agents", rail, Modifier.weight(1f)) {
                AgentsList(appViewModel(), open, onAddHost = { settings = true }, dense = true, rail = rail)
            }
            HorizontalDivider()
            val footer = Modifier.selectable(settings, role = Role.Tab) { settings = !settings }
            val on = if (settings) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface
            if (rail) {
                Box(Modifier.fillMaxWidth().height(56.dp), contentAlignment = Alignment.Center) {
                    Box(
                        Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(on).then(footer),
                        contentAlignment = Alignment.Center,
                    ) { Icon(Lucide.Settings, "Settings", Modifier.size(18.dp)) }
                }
            } else {
                ListItem(
                    headlineContent = { Text("Settings", maxLines = 1) },
                    leadingContent = { Icon(Lucide.Settings, null, Modifier.size(18.dp)) },
                    colors = ListItemDefaults.colors(containerColor = on),
                    modifier = footer,
                )
            }
        }
        VerticalDivider()
        // Settings sits on the surface its rows and fields are drawn on, as on the phone.
        val pane = if (settings) MaterialTheme.colorScheme.surface else MaterialTheme.colorScheme.surfaceDim
        Surface(Modifier.weight(1f).fillMaxHeight(), color = pane) {
            if (settings) {
                SettingsScreen(appViewModel())
            } else {
                Box {
                    TerminalScreen(appViewModel(), host, prefs, open, onBack = {}, header = false)
                    Opening(main)
                }
            }
        }
    }
}

/** A titled half of the sidebar (untitled on the rail); its list scrolls on its own. */
@Composable
private fun Section(title: String, rail: Boolean, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Column(modifier) {
        if (rail) Spacer(Modifier.height(8.dp))
        else Text(
            title,
            Modifier.padding(start = 16.dp, top = 12.dp, bottom = 4.dp),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
        )
        Box(Modifier.weight(1f)) { content() }
    }
}
