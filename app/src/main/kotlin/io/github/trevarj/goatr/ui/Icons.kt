// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.annotation.DrawableRes

/** Official Material vectors kept as local resources so the presentation has no icon-library dependency. */
internal enum class MaterialGlyph(@DrawableRes val drawable: Int) {
    Chat(R.drawable.ic_material_chat),
    Folder(R.drawable.ic_material_folder),
    Visibility(R.drawable.ic_material_visibility),
    Desktop(R.drawable.ic_material_desktop_windows),
    Settings(R.drawable.ic_material_settings),
    Add(R.drawable.ic_material_add),
    Scan(R.drawable.ic_material_center_focus_strong),
    Refresh(R.drawable.ic_material_refresh),
    Send(R.drawable.ic_material_send),
    Stop(R.drawable.ic_material_stop),
    Copy(R.drawable.ic_material_content_copy),
    ExpandMore(R.drawable.ic_material_expand_more),
    Back(R.drawable.ic_material_arrow_back),
    Menu(R.drawable.ic_material_menu),
    Close(R.drawable.ic_material_close),
    Terminal(R.drawable.ic_material_terminal),
    Save(R.drawable.ic_material_save),
    Edit(R.drawable.ic_material_edit),
    Delete(R.drawable.ic_material_delete),
    Notification(R.drawable.ic_material_notifications),
    Play(R.drawable.ic_material_play_arrow),
}

@Composable
internal fun GoatrIcon(glyph: MaterialGlyph, contentDescription: String?, modifier: Modifier = Modifier) {
    Icon(painter = painterResource(glyph.drawable), contentDescription = contentDescription, modifier = modifier.size(18.dp))
}

@Composable
internal fun IconLabel(glyph: MaterialGlyph, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        GoatrIcon(glyph, contentDescription = null)
        // Give text the remaining width so a long action never pushes its icon out of bounds.
        UiText(label, color = LocalContentColor.current, modifier = Modifier.weight(1f, fill = false))
    }
}

