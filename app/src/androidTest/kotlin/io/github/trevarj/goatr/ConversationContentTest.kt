// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.ClipboardManager
import androidx.compose.material3.MaterialTheme
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class ConversationContentTest {
    @get:Rule val ui=createComposeRule()
    @Test fun productionConversationFrameReservesActionsWithOmpRequestsAtTwoHundredPercent() {
        var text by mutableStateOf("")
        var sends=0;var stops=0
        var admitted by mutableStateOf(false)
        val request=PendingRequest(Uuid.random(),Provider.Omp,PresentationKind.OmpEditor,
            PresentationOmpEditor(ContentText(InlineText("Synthetic pending question")),ContentText(InlineText(""))))
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,2f)) {
                // Available conversation space after app bar and IME, using the production frame.
                BoxWithConstraints(Modifier.width(320.dp).height(240.dp)) {
                    ConversationFrame(chrome={
                        RequestDock(listOf(request)) {}
                        androidx.compose.material3.Text("All branches; desktop-selected branch unavailable",modifier=Modifier.testTag("omp_branches"))
                    },history={androidx.compose.material3.Text("Synthetic long transcript\n".repeat(20))},editor={
                        ConversationControlRail(admitted,"Current original source is read-only",{stops++},{}) {}
                    },actions={
                        ConversationDraftControls(text,{text=it},"Recovery copy deleted by 2030-01-01T12:00:00Z (≤7 days).",true,(maxHeight-56.dp).coerceAtLeast(0.dp),admitted,{sends++})
                    })
                }
            }}
        }
        ui.onNodeWithTag("prompt_send").assertIsDisplayed().assertIsNotEnabled()
        ui.onNodeWithTag("turn_stop").assertIsDisplayed().assertIsNotEnabled()
        ui.onNodeWithTag("omp_branches").performScrollTo().assertIsDisplayed()
        ui.onNodeWithTag("composer").assertIsDisplayed().performTextReplacement("Original\ndraft")
        ui.onNodeWithTag("send_deadline").performScrollTo().assertIsDisplayed()
        val frame=ui.onNodeWithTag("conversation_frame").getUnclippedBoundsInRoot()
        val send=ui.onNodeWithTag("prompt_send").assertIsDisplayed().getUnclippedBoundsInRoot()
        val stop=ui.onNodeWithTag("turn_stop").assertIsDisplayed().getUnclippedBoundsInRoot()
        assertTrue(send.left>=frame.left && send.right<=frame.right && send.bottom<=frame.bottom)
        assertTrue(stop.left>=frame.left && stop.right<=frame.right && stop.top>=frame.top && stop.bottom<=send.top)
        assertEquals(0,sends);assertEquals(0,stops)
        ui.runOnIdle {admitted=true}
        ui.onNodeWithTag("prompt_send").assertIsEnabled().performClick()
        ui.onNodeWithTag("turn_stop").assertIsEnabled().performClick()
        assertEquals("Original\ndraft",text);assertEquals(1,sends);assertEquals(1,stops)
    }
    @Test fun actualRailKeepsMoreAndStopReachableWhileFrequentControlsScroll() {
        var large by mutableStateOf(false)
        var enabled by mutableStateOf(false)
        var more=0
        var stops=0
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,if(large) 2f else 1f)) {
                Box(Modifier.width(320.dp)) {
                    ConversationControlRail(enabled,"Current native action is unavailable",{stops++},{more++}) {
                        repeat(8) {index->ConversationControl(MaterialGlyph.Chat,"Existing control $index","rail_control_$index") {}}
                    }
                }
            }}
        }
        for(scale in listOf(false,true)) {
            ui.runOnIdle {large=scale}
            val moreBounds=ui.onNodeWithTag("conversation_more").assertIsDisplayed().assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot
            val stopBounds=ui.onNodeWithTag("turn_stop").assertIsDisplayed().assertIsNotEnabled().assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot
            ui.onNodeWithTag("rail_control_7").performScrollTo().assertIsDisplayed()
            assertEquals(moreBounds,ui.onNodeWithTag("conversation_more").assertIsDisplayed().fetchSemanticsNode().boundsInRoot)
            assertEquals(stopBounds,ui.onNodeWithTag("turn_stop").assertIsDisplayed().fetchSemanticsNode().boundsInRoot)
            ui.onNodeWithTag("conversation_more").performClick()
        }
        assertEquals(2,more)
        assertEquals(0,stops)
        ui.runOnIdle {enabled=true}
        ui.onNodeWithTag("turn_stop").assertIsDisplayed().assertIsEnabled().performClick()
        assertEquals(1,stops)
    }

    @Test fun syntheticGalleryShowsStateBaselineAndLightDarkTextContrast() {
        var appearance by mutableStateOf(Appearance.Light)
        var contrast=0f
        ui.setContent {GoatrTheme(appearance) {
            val colors=MaterialTheme.colorScheme
            val pairs=listOf(colors.onSurface to colors.surface,colors.onPrimary to colors.primary,
                colors.onPrimaryContainer to colors.primaryContainer,colors.onSurfaceVariant to colors.surfaceVariant,colors.error to colors.surface)
            SideEffect {contrast=pairs.minOf {(ink,paper)->val a=ink.luminance();val b=paper.luminance();(maxOf(a,b)+0.05f)/(minOf(a,b)+0.05f)}}
            GoatrComponentGallery()
        }}
        for(theme in listOf(Appearance.Light,Appearance.Dark)) {
            ui.runOnIdle {appearance=theme}
            ui.onNodeWithText("Synthetic design gallery").performScrollTo().assertIsDisplayed()
            ui.onNodeWithText("Changed session — review before opening").performScrollTo().assertIsDisplayed()
            ui.onNodeWithText("Outcome unknown — check status; never resend automatically").performScrollTo().assertIsDisplayed()
            ui.onNodeWithTag("gallery_disabled").performScrollTo().assertIsNotEnabled()
            ui.runOnIdle {assertTrue("Actual palette ordinary text contrast",contrast>=4.5f)}
        }
    }
    @Test fun actualNativeMarkdownCopiesExactBackingAndRejectsNonHttpNavigation() {
        val backing="Assistant **Markdown**\n\n```text\n  preserve\tcode  \n```\n\n<div>raw HTML must not render</div>\n\n![remote](https://example.invalid/not-loaded.png)"
        lateinit var writerContext:android.content.Context
        lateinit var writerView:android.view.View
        ui.setContent {
            val context=LocalContext.current
            val view=LocalView.current
            SideEffect {writerContext=context;writerView=view}
            GoatrTheme {ContentView(ContentText(InlineText(backing)),null,null,null,completed=true,markdown=true)}
        }
        ui.onNodeWithTag("markdown_native").assertExists()
        ui.runOnIdle {
            assertTrue("Copy writer window must be focused: focused=${writerView.hasWindowFocus()} callerUid=${android.os.Process.myUid()} writerUid=${writerContext.applicationInfo.uid} writerPackage=${writerContext.packageName}",writerView.hasWindowFocus())
        }
        ui.onNodeWithTag("content_copy").performClick()
        var copied: String?=null
        ui.runOnIdle {
            assertTrue("Clipboard reader must retain writer focus: focused=${writerView.hasWindowFocus()} callerUid=${android.os.Process.myUid()} writerUid=${writerContext.applicationInfo.uid} writerPackage=${writerContext.packageName}",writerView.hasWindowFocus())
            copied=writerContext.getSystemService(ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text?.toString()
        }
        assertEquals(backing,copied)
        for(url in listOf("javascript:alert(1)","file:///private","intent://run","https://trusted.example@evil.example/","https://example.test/%broken","https://example.test/\nspoof","//example.test/no-scheme")) assertNull(safeHttpUrl(url))
        assertNotNull(safeHttpUrl("https://example.test/path?q=value#fragment"))
        assertNotNull(safeHttpUrl("http://127.0.0.1:8080/path"))
        assertTrue(validFormat(FieldFormat.Uri,"mailto:user@example.test"))
        assertNull(safeHttpUrl("mailto:user@example.test"))
    }
    @Test fun nativeTextSelectionCopyPreservesOriginalMarkdownAndCodeWhitespace() {
        val backing="Assistant **exact source**\n\n```text\n  leading\tcode  \n```\n"
        ui.setContent {GoatrTheme {MarkdownText(backing,{fail("Copy must not navigate")})}}
        ui.onNodeWithTag("markdown_native").assertExists()
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        var copied: String?=null
        instrumentation.runOnMainSync {
            val activity=androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).single()
            fun textView(root: android.view.View):android.widget.TextView? {
                if(root is android.widget.TextView && root.isTextSelectable) return root
                if(root is android.view.ViewGroup) for(index in 0 until root.childCount) textView(root.getChildAt(index))?.let {return it}
                return null
            }
            val view=textView(activity.window.decorView) ?: error("Actual selectable native Markdown TextView was not found")
            assertTrue("Native copy writer window must be focused: focused=${view.hasWindowFocus()} callerUid=${android.os.Process.myUid()} writerUid=${view.context.applicationInfo.uid} writerPackage=${view.context.packageName}",view.hasWindowFocus())
            view.requestFocus()
            assertTrue(view.onTextContextMenuItem(android.R.id.selectAll))
            assertTrue(view.onTextContextMenuItem(android.R.id.copy))
            assertTrue("Native clipboard reader must retain writer focus: focused=${view.hasWindowFocus()} callerUid=${android.os.Process.myUid()} writerUid=${view.context.applicationInfo.uid} writerPackage=${view.context.packageName}",view.hasWindowFocus())
            copied=view.context.getSystemService(ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text?.toString()
        }
        assertEquals(backing,copied)
    }
}
