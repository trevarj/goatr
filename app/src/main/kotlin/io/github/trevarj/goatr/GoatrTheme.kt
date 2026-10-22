// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

enum class Appearance { System, Light, Dark }

// Warm neutral surfaces, teal actions and compact type that follows system font scaling.
private val LightPalette = lightColorScheme(
    primary=Color(0xFF006A60),onPrimary=Color.White,
    primaryContainer=Color(0xFFB8F2E6),onPrimaryContainer=Color(0xFF00201C),
    secondary=Color(0xFF38665F),onSecondary=Color.White,
    secondaryContainer=Color(0xFFC1EDE4),onSecondaryContainer=Color(0xFF00201C),
    tertiary=Color(0xFF4E6179),onTertiary=Color.White,
    tertiaryContainer=Color(0xFFD6E3FF),onTertiaryContainer=Color(0xFF071C33),
    background=Color(0xFFFAF9F5),onBackground=Color(0xFF1B1C19),
    surface=Color(0xFFFAF9F5),onSurface=Color(0xFF1B1C19),
    surfaceVariant=Color(0xFFE1E5DE),onSurfaceVariant=Color(0xFF404943),
    outline=Color(0xFF707973),error=Color(0xFFBA1A1A),
)
private val DarkPalette = darkColorScheme(
    primary=Color(0xFF92D5C7),onPrimary=Color(0xFF003730),
    primaryContainer=Color(0xFF005047),onPrimaryContainer=Color(0xFFB8F2E6),
    secondary=Color(0xFFA5D1C8),onSecondary=Color(0xFF0B3731),
    secondaryContainer=Color(0xFF204E47),onSecondaryContainer=Color(0xFFC1EDE4),
    tertiary=Color(0xFFB6C8E5),onTertiary=Color(0xFF203248),
    tertiaryContainer=Color(0xFF364960),onTertiaryContainer=Color(0xFFD6E3FF),
    background=Color(0xFF121411),onBackground=Color(0xFFE3E3DD),
    surface=Color(0xFF121411),onSurface=Color(0xFFE3E3DD),
    surfaceVariant=Color(0xFF404943),onSurfaceVariant=Color(0xFFC0C9C1),
    outline=Color(0xFF8A938C),error=Color(0xFFFFB4AB),
)

private val GoatrTypography=Typography().let {base->
    Typography(
        titleLarge=base.titleLarge.copy(fontWeight=FontWeight.SemiBold,fontSize=22.sp,lineHeight=28.sp),
        titleMedium=base.titleMedium.copy(fontWeight=FontWeight.SemiBold,fontSize=16.sp,lineHeight=22.sp),
        titleSmall=base.titleSmall.copy(fontWeight=FontWeight.SemiBold,fontSize=15.sp,lineHeight=20.sp),
        bodyLarge=base.bodyLarge.copy(fontSize=16.sp,lineHeight=24.sp),
        bodyMedium=base.bodyMedium.copy(fontSize=14.sp,lineHeight=20.sp),
        bodySmall=base.bodySmall.copy(fontSize=13.sp,lineHeight=18.sp),
        labelLarge=base.labelLarge.copy(fontWeight=FontWeight.SemiBold,fontSize=14.sp,lineHeight=20.sp),
        labelMedium=base.labelMedium.copy(fontWeight=FontWeight.SemiBold,fontSize=12.sp,lineHeight=16.sp),
        labelSmall=base.labelSmall.copy(fontWeight=FontWeight.SemiBold,fontSize=11.sp,lineHeight=16.sp),
    )
}
private val GoatrShapes=Shapes(small=RoundedCornerShape(8.dp),medium=RoundedCornerShape(16.dp),large=RoundedCornerShape(24.dp))

@Composable
fun GoatrTheme(appearance: Appearance=Appearance.System,content: @Composable ()->Unit) {
    val dark=when(appearance) {Appearance.System->isSystemInDarkTheme();Appearance.Light->false;Appearance.Dark->true}
    MaterialTheme(colorScheme=if(dark) DarkPalette else LightPalette,
        typography=GoatrTypography,
        shapes=GoatrShapes,content=content)
}

/** Synthetic component fixture retained for the existing Compose contrast and reachability checks. */
@Composable
fun GoatrComponentGallery() {
    var draft by remember {mutableStateOf("")}
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp).testTag("synthetic_gallery"),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        GoatrSection("Synthetic design gallery","Synthetic presentation states only; no provider connection or native action.")
        listOf(
            "Ready — send a message" to StatusEmphasis.Positive,
            "Working — Stop remains separate" to StatusEmphasis.Attention,
            "Needs input — choose a request" to StatusEmphasis.Attention,
            "Offline — reconnect before sending" to StatusEmphasis.Muted,
            "Changed session — review before opening" to StatusEmphasis.Muted,
            "Outcome unknown — check status; never resend automatically" to StatusEmphasis.Muted,
        ).forEach {(label,emphasis)->GroupedSurface {UiText(label);StatusPill("Synthetic",emphasis)}}
        Action("Normal control","gallery_action") {}
        Action("Unavailable","gallery_disabled",enabled=false) {}
        ConversationComposer(draft,{draft=it},true,true,"Synthetic recovery deadline — no data is stored",false,{}, {},null)
    }
}

