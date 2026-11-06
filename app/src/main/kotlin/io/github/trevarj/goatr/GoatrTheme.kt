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

internal data class UserMessageColors(val container:Color,val content:Color)
internal val LocalUserMessageColors=staticCompositionLocalOf {UserMessageColors(Color(0xFFE8E8E8),Color(0xFF3E3E3E))}
internal data class ComposerColors(val surface:Color,val control:Color,val content:Color,val mutedContent:Color,val send:Color,val sendContent:Color,val hairline:Color)
internal val LocalComposerColors=staticCompositionLocalOf {ComposerColors(Color(0xFFE8E8E8),Color(0xFFF5F5F5),Color(0xFF3E3E3E),Color(0xFF5F6368),Color(0xFF202124),Color.White,Color(0xFF747474).copy(alpha=.25f))}
internal data class StatusColors(val positiveContainer:Color,val positiveContent:Color,val attentionContainer:Color,val attentionContent:Color,val neutralContainer:Color,val neutralContent:Color,val mutedContainer:Color,val mutedContent:Color)
internal val LocalStatusColors=staticCompositionLocalOf {StatusColors(Color(0xFFDCFCE7),Color(0xFF14532D),Color(0xFFFEF3C7),Color(0xFF78350F),Color(0xFFE8E8E8),Color(0xFF3E3E3E),Color(0xFFF2F2F2),Color(0xFF5F6368))}

// Neutral surfaces keep provider and status meaning in their labeled components instead of hue.
private val LightPalette = lightColorScheme(
    primary=Color(0xFF202124),onPrimary=Color.White,
    primaryContainer=Color(0xFFE4E4E4),onPrimaryContainer=Color(0xFF1B1B1B),inversePrimary=Color(0xFFE8E8E8),
    secondary=Color(0xFF5F6368),onSecondary=Color.White,
    secondaryContainer=Color(0xFFE7E7E7),onSecondaryContainer=Color(0xFF1F1F1F),
    tertiary=Color(0xFF1E40AF),onTertiary=Color.White,
    tertiaryContainer=Color(0xFFDBEAFE),onTertiaryContainer=Color(0xFF172554),
    background=Color(0xFFFAF9F5),onBackground=Color(0xFF1B1B1B),
    surface=Color(0xFFFAF9F5),onSurface=Color(0xFF1B1B1B),
    surfaceVariant=Color(0xFFE8E8E8),onSurfaceVariant=Color(0xFF3E3E3E),surfaceTint=Color(0xFFFAF9F5),
    inverseSurface=Color(0xFF303030),inverseOnSurface=Color(0xFFF5F5F5),
    error=Color(0xFFBA1A1A),onError=Color.White,errorContainer=Color(0xFFFFDAD6),onErrorContainer=Color(0xFF410002),
    outline=Color(0xFF747474),outlineVariant=Color(0xFFC4C4C4),scrim=Color.Black,
    surfaceBright=Color(0xFFFAF9F5),surfaceDim=Color(0xFFE0E0DC),surfaceContainerLowest=Color.White,
    surfaceContainerLow=Color(0xFFF5F4F0),surfaceContainer=Color(0xFFF0EFEB),surfaceContainerHigh=Color(0xFFEBEAE6),surfaceContainerHighest=Color(0xFFE5E4E0),
)
private val DarkPalette = darkColorScheme(
    primary=Color(0xFFF5F5F5),onPrimary=Color(0xFF111111),
    primaryContainer=Color(0xFF2A2A2A),onPrimaryContainer=Color(0xFFF5F5F5),inversePrimary=Color(0xFF202124),
    secondary=Color(0xFFA1A1A1),onSecondary=Color(0xFF111111),
    secondaryContainer=Color(0xFF242424),onSecondaryContainer=Color(0xFFE0E0E0),
    tertiary=Color(0xFF8AB4F8),onTertiary=Color(0xFF0B1F3A),
    tertiaryContainer=Color(0xFF1E3A5F),onTertiaryContainer=Color(0xFFD8E2FF),
    background=Color(0xFF111111),onBackground=Color(0xFFE0E0E0),
    surface=Color(0xFF1A1A1A),onSurface=Color(0xFFE0E0E0),
    surfaceVariant=Color(0xFF242424),onSurfaceVariant=Color(0xFFE0E0E0),surfaceTint=Color(0xFF1A1A1A),
    inverseSurface=Color(0xFFE0E0E0),inverseOnSurface=Color(0xFF1A1A1A),
    error=Color(0xFFFFB4AB),onError=Color(0xFF690005),errorContainer=Color(0xFF93000A),onErrorContainer=Color(0xFFFFDAD6),
    outline=Color(0xFF2A2A2A),outlineVariant=Color(0xFF2A2A2A),scrim=Color.Black,
    surfaceBright=Color(0xFF242424),surfaceDim=Color(0xFF111111),surfaceContainerLowest=Color(0xFF111111),
    surfaceContainerLow=Color(0xFF1A1A1A),surfaceContainer=Color(0xFF1E1E1E),surfaceContainerHigh=Color(0xFF242424),surfaceContainerHighest=Color(0xFF2A2A2A),
)
private val LightUserMessageColors=UserMessageColors(LightPalette.surfaceVariant,LightPalette.onSurfaceVariant)
private val DarkUserMessageColors=UserMessageColors(Color(0xFF323232),DarkPalette.onSurface)
private val LightComposerColors=ComposerColors(LightPalette.surfaceVariant,LightPalette.surface,LightPalette.onSurfaceVariant,LightPalette.secondary,LightPalette.primary,LightPalette.onPrimary,LightPalette.outline.copy(alpha=.25f))
private val DarkComposerColors=ComposerColors(Color(0xFF212121),Color(0xFF2A2A2A),Color(0xFFE0E0E0),Color(0xFFA1A1A1),Color(0xFFF5F5F5),Color(0xFF111111),Color.White.copy(alpha=.08f))
private val LightStatusColors=StatusColors(Color(0xFFDCFCE7),Color(0xFF14532D),Color(0xFFFEF3C7),Color(0xFF78350F),LightPalette.secondaryContainer,LightPalette.onSecondaryContainer,Color(0xFFF2F2F2),LightPalette.secondary)
private val DarkStatusColors=StatusColors(Color(0xFF143D22),Color(0xFFD7F8D7),Color(0xFF4D3300),Color(0xFFFFE2A8),DarkPalette.surfaceVariant,DarkPalette.onSurfaceVariant,DarkPalette.surface,DarkPalette.secondary)

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
private val GoatrShapes=Shapes(small=RoundedCornerShape(8.dp),medium=RoundedCornerShape(14.dp),large=RoundedCornerShape(24.dp))

@Composable
fun GoatrTheme(appearance: Appearance=Appearance.System,content: @Composable ()->Unit) {
    val dark=when(appearance) {Appearance.System->isSystemInDarkTheme();Appearance.Light->false;Appearance.Dark->true}
    CompositionLocalProvider(LocalUserMessageColors provides if(dark) DarkUserMessageColors else LightUserMessageColors,
        LocalComposerColors provides if(dark) DarkComposerColors else LightComposerColors,
        LocalStatusColors provides if(dark) DarkStatusColors else LightStatusColors) {
        MaterialTheme(colorScheme=if(dark) DarkPalette else LightPalette,
            typography=GoatrTypography,
            shapes=GoatrShapes,content=content)
    }
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

