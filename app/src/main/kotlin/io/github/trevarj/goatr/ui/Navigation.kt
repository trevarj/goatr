// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.*
import androidx.compose.ui.unit.dp
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId

@Composable
internal fun NavigationMenu(model: GoatrViewModel) {
    var expanded by remember {mutableStateOf(false)}
    Box {
        IconButton(onClick={expanded=true},modifier=Modifier.size(48.dp).testTag("navigation_menu")) {GoatrIcon(MaterialGlyph.Menu,"Navigation menu")}
        DropdownMenu(expanded=expanded,onDismissRequest={expanded=false},modifier=Modifier.semantics {testTagsAsResourceId=true}) {
            DropdownMenuItem(text={UiText("Sessions")},leadingIcon={GoatrIcon(MaterialGlyph.Chat,null)},modifier=Modifier.testTag("navigation_sessions_menu"),onClick={expanded=false;model.sessions()})
            DropdownMenuItem(text={UiText("Hosts")},leadingIcon={GoatrIcon(MaterialGlyph.Desktop,null)},modifier=Modifier.testTag("navigation_hosts_menu"),onClick={expanded=false;model.connections()})
            DropdownMenuItem(text={UiText("Settings")},leadingIcon={GoatrIcon(MaterialGlyph.Settings,null)},modifier=Modifier.testTag("navigation_settings_menu"),onClick={expanded=false;model.settings()})
            model.host?.let {host->DropdownMenuItem(text={UiText("Host settings")},modifier=Modifier.testTag("settings"),onClick={expanded=false;model.settings(host)})}
        }
    }
}

