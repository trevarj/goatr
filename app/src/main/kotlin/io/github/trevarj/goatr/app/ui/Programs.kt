// What runs in a terminal, drawn as the desktop's tabs and agents list draw
// it: the program's icon alone, a one-color glyph in the text color or a logo
// in its own. The icons are the desktop's (herdr-app defaults/icons, copied
// into assets/icons, drawn by Svg.kt).

package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.unit.Dp
import herdr.core.Program

/** A glyph in `color`, or the program's logo as is. */
@Composable
fun ProgramIcon(program: Program, size: Dp, color: Color = MaterialTheme.colorScheme.onSurface) {
    val icon = program.icon ?: return
    Image(svgPainter(icon), null, Modifier.size(size), colorFilter = if (program.tint) ColorFilter.tint(color) else null)
}
