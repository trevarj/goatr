// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag

@Composable
internal fun TopLevelNav(selected: Screen, onSessions: () -> Unit, onHosts: () -> Unit, onSettings: () -> Unit) {
    val colors=NavigationBarItemDefaults.colors(
        selectedIconColor=MaterialTheme.colorScheme.onPrimaryContainer,
        selectedTextColor=MaterialTheme.colorScheme.onPrimaryContainer,
        indicatorColor=MaterialTheme.colorScheme.primaryContainer,
        unselectedIconColor=MaterialTheme.colorScheme.onSurfaceVariant,
        unselectedTextColor=MaterialTheme.colorScheme.onSurfaceVariant,
    )
    NavigationBar(containerColor=MaterialTheme.colorScheme.surface) {
        NavigationBarItem(selected = selected == Screen.Sessions || selected == Screen.Conversation, onClick = onSessions, icon = { GoatrIcon(MaterialGlyph.Chat, null) }, label = { UiText("Sessions") }, colors=colors, modifier = Modifier.testTag("nav_sessions"))
        NavigationBarItem(selected = selected == Screen.Connections || selected == Screen.Pairing, onClick = onHosts, icon = { GoatrIcon(MaterialGlyph.Desktop, null) }, label = { UiText("Hosts") }, colors=colors, modifier = Modifier.testTag("nav_hosts"))
        NavigationBarItem(selected = selected == Screen.Settings, onClick = onSettings, icon = { GoatrIcon(MaterialGlyph.Settings, null) }, label = { UiText("Settings") }, colors=colors, modifier = Modifier.testTag("nav_settings"))
    }
}

