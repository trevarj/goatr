// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.ClipboardManager
import androidx.compose.material3.MaterialTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import kotlinx.coroutines.*
import androidx.compose.foundation.background
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextRange
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
    private data class FocusAdmissionState(val resumed:Boolean,val sameWindow:Boolean,val sameDecorRoot:Boolean,
        val attached:Boolean,val sameWriterRoot:Boolean,val focused:Boolean) {
        fun admitted()=resumed&&sameWindow&&sameDecorRoot&&attached&&sameWriterRoot&&focused
    }
    private fun awaitOriginalWindowFocus(writer:android.view.View) {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        lateinit var originalActivity:android.app.Activity
        lateinit var originalWindow:android.view.Window
        lateinit var originalRoot:android.view.View
        instrumentation.runOnMainSync {
            var context=writer.context
            while(context is android.content.ContextWrapper&&context !is android.app.Activity) {
                val parent=context.baseContext;check(parent!==context);context=parent
            }
            originalActivity=context as? android.app.Activity ?: error("Original writer Activity unavailable")
            originalWindow=originalActivity.window
            originalRoot=originalWindow.decorView.rootView
        }
        var last:FocusAdmissionState?=null
        try {
            ui.waitUntil(10000) {
                var ready=false
                instrumentation.runOnMainSync {
                    // Evaluate every original-owner predicate in this poll, without short-circuit
                    // blindness. Never substitute a later window sample for the failed admission.
                    val resumed=androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                        .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).any {it===originalActivity}
                    val sameWindow=originalActivity.window===originalWindow
                    val sameDecorRoot=originalWindow.decorView.rootView===originalRoot
                    val attached=writer.isAttachedToWindow
                    val sameWriterRoot=writer.rootView===originalRoot
                    val focused=writer.hasWindowFocus()
                    val sample=FocusAdmissionState(resumed,sameWindow,sameDecorRoot,attached,sameWriterRoot,focused)
                    last=sample;ready=sample.admitted()
                }
                ready
            }
        }catch(timeout:ComposeTimeoutException) {
            val sample=last
            fail("focus_admission_timeout observed=${sample!=null} resumed=${sample?.resumed} same_window=${sample?.sameWindow} same_decor_root=${sample?.sameDecorRoot} attached=${sample?.attached} same_writer_root=${sample?.sameWriterRoot} focused=${sample?.focused}")
        }
    }
    @Test fun productionConversationFrameReservesActionsWithOmpRequestsAtTwoHundredPercent() {
        var text by mutableStateOf("")
        var sends=0;var stops=0
        var admitted by mutableStateOf(false)
        var availableHeight by mutableStateOf(240.dp)
        val request=PendingRequest(Uuid.random(),Provider.Omp,PresentationKind.OmpEditor,
            PresentationOmpEditor(ContentText(InlineText("Synthetic pending question")),ContentText(InlineText(""))))
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,2f)) {
                // Available conversation space after app bar and IME, using the production frame.
                BoxWithConstraints(Modifier.width(320.dp).height(availableHeight)) {
                    ConversationFrame(chrome={
                        RequestDock(listOf(request)) {}
                        androidx.compose.material3.Text("All branches; desktop-selected branch unavailable",modifier=Modifier.testTag("omp_branches"))
                        ComposerRecoveryNotice("Recovery copy deleted by 2030-01-01T12:00:00Z (≤7 days).")
                    },history={androidx.compose.material3.Text("Synthetic long transcript\n".repeat(20))},editor={
                        ConversationControlRail(admitted,"Current original source is read-only",{stops++},{}) {}
                    },actions={
                        ConversationDraftControls(text,{text=it},admitted,{sends++})
                    })
                }
            }}
        }
        fun assertPinnedComposerVisible() {
            val frame=ui.onNodeWithTag("conversation_frame").fetchSemanticsNode().boundsInRoot
            for(tag in listOf("composer","prompt_send","turn_stop")) {
                val node=ui.onNodeWithTag(tag).assertIsDisplayed().fetchSemanticsNode()
                val visible=node.boundsInRoot
                // Nonempty clipped bounds pass assertIsDisplayed(); require the whole control.
                assertEquals("$tag visible height",node.size.height.toFloat(),visible.height,1f)
                assertEquals("$tag visible width",node.size.width.toFloat(),visible.width,1f)
                assertTrue("$tag stays in the viewport",visible.left>=frame.left&&visible.right<=frame.right&&visible.top>=frame.top&&visible.bottom<=frame.bottom)
            }
            ui.onNodeWithTag("prompt_send").assertHeightIsAtLeast(56.dp)
            ui.onNodeWithTag("turn_stop").assertHeightIsAtLeast(48.dp)
            assertTrue(ui.onNodeWithTag("turn_stop").fetchSemanticsNode().boundsInRoot.bottom<=ui.onNodeWithTag("prompt_send").fetchSemanticsNode().boundsInRoot.top)
        }
        ui.onNodeWithTag("prompt_send").assertIsDisplayed().assertIsNotEnabled()
        ui.onNodeWithTag("turn_stop").assertIsDisplayed().assertIsNotEnabled()
        assertPinnedComposerVisible()
        ui.onNodeWithTag("omp_branches").performScrollTo().assertIsDisplayed()
        ui.onNodeWithTag("composer").assertIsDisplayed().performTextReplacement("Original\ndraft")
        ui.onNodeWithTag("send_deadline").performScrollTo().assertIsDisplayed()
        assertPinnedComposerVisible()
        assertEquals(0,sends);assertEquals(0,stops)
        ui.runOnIdle {admitted=true}
        ui.onNodeWithTag("prompt_send").assertIsEnabled().performClick()
        ui.onNodeWithTag("turn_stop").assertIsEnabled().performClick()
        assertEquals("Original\ndraft",text);assertEquals(1,sends);assertEquals(1,stops)
        ui.runOnIdle {availableHeight=180.dp}
        assertPinnedComposerVisible()
        ui.onNodeWithTag("composer").assertTextContains("Original\ndraft")
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
        awaitOriginalWindowFocus(writerView)
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
        lateinit var originalWriter:android.view.View
        ui.setContent {val writer=LocalView.current;SideEffect{originalWriter=writer};GoatrTheme {MarkdownText(backing,{fail("Copy must not navigate")})}}
        ui.onNodeWithTag("markdown_native").assertExists()
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        var copied: String?=null
        lateinit var view:android.widget.TextView
        instrumentation.runOnMainSync {
            fun textView(root: android.view.View):android.widget.TextView? {
                if(root is android.widget.TextView && root.isTextSelectable) return root
                if(root is android.view.ViewGroup) for(index in 0 until root.childCount) textView(root.getChildAt(index))?.let {return it}
                return null
            }
            view=textView(originalWriter.rootView) ?: error("Actual selectable native Markdown TextView was not found")
        }
        awaitOriginalWindowFocus(view)
        instrumentation.runOnMainSync {
            assertTrue("Native copy writer window must be focused: focused=${view.hasWindowFocus()} callerUid=${android.os.Process.myUid()} writerUid=${view.context.applicationInfo.uid} writerPackage=${view.context.packageName}",view.hasWindowFocus())
            view.requestFocus()
            assertTrue(view.onTextContextMenuItem(android.R.id.selectAll))
            assertTrue(view.onTextContextMenuItem(android.R.id.copy))
            assertTrue("Native clipboard reader must retain writer focus: focused=${view.hasWindowFocus()} callerUid=${android.os.Process.myUid()} writerUid=${view.context.applicationInfo.uid} writerPackage=${view.context.packageName}",view.hasWindowFocus())
            copied=view.context.getSystemService(ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text?.toString()
        }
        assertEquals(backing,copied)
    }

    private fun actualEditorLayout():TextLayoutResult {
        val result=ArrayList<TextLayoutResult>()
        ui.onNodeWithTag("composer").performSemanticsAction(SemanticsActions.GetTextLayoutResult) {read->assertTrue(read(result))}
        return result.last()
    }

    @Test fun productionEditorGrowsThroughSixMeasuredNativeLinesAtNormalAndLargeFont() {
        var fontScale by mutableStateOf(1f)
        var text by mutableStateOf("A")
        ui.setContent {val density=LocalDensity.current;GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,fontScale)) {
            BoxWithConstraints(Modifier.width(360.dp).height(760.dp)) {
                ConversationFrame(chrome={},history={androidx.compose.material3.Text("Original transcript")},editor={ConversationControlRail(false,"Read-only",{},{}) {}},actions={ConversationDraftControls(text,{text=it},false,{})})
            }
        }}}
        for(scale in listOf(1f,2f)) {
            ui.runOnIdle {fontScale=scale;text="A"}
            val one=ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.height
            ui.runOnIdle {text="A\nB\nC\nD\nE\nF"}
            val layout=actualEditorLayout()
            assertEquals(6,layout.lineCount)
            val six=ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.height
            assertTrue("All six real font lines must fit when the viewport permits",six+1f>=one+layout.getLineBottom(5)-layout.getLineBottom(0))
            ui.runOnIdle {text+="\nG"}
            assertEquals(7,actualEditorLayout().lineCount)
            assertEquals(six,ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.height,1f)
            ui.onNodeWithTag("prompt_send").assertIsDisplayed().assertHeightIsAtLeast(48.dp)
            ui.onNodeWithTag("turn_stop").assertIsDisplayed().assertHeightIsAtLeast(48.dp)
        }
    }

    @Test fun actualEditorRestoresRetainedSelectionAndAcceptsExternalTextAndCursorChanges() {
        var text by mutableStateOf("Original draft selection")
        var selection by mutableStateOf(2 to 8)
        var shown by mutableStateOf(true)
        ui.setContent {GoatrTheme {Column(Modifier.width(360.dp).height(240.dp)) {
            if(shown)ComposerEditor(text,{text=it},null,null,onCursor={start,end->selection=start to end},selection=selection)
        }}}
        fun selectionIs(range:TextRange)=ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(SemanticsProperties.TextSelectionRange,range))
        selectionIs(TextRange(2,8))
        ui.onNodeWithTag("composer").performTextInputSelection(TextRange(1,7))
        ui.runOnIdle {assertEquals(1 to 7,selection)}
        selectionIs(TextRange(1,7))
        // External ordered selection must reach the actual editor before its removal.
        ui.runOnIdle {selection=8 to 3}
        selectionIs(TextRange(8,3))
        ui.runOnIdle {shown=false}
        ui.onNodeWithTag("composer").assertDoesNotExist()
        ui.runOnIdle {shown=true}
        selectionIs(TextRange(8,3))
        ui.runOnIdle {assertEquals(8 to 3,selection)}
        ui.runOnIdle {text="External insertion";selection=4 to 4}
        ui.onNodeWithTag("composer").assertTextContains("External insertion")
        selectionIs(TextRange(4))
        ui.runOnIdle {selection=1 to 7}
        selectionIs(TextRange(1,7))
    }

    @Test fun assistantMessageBackgroundIsTransparentWhileUserAndNativeToolStayContained() {
        var user=Color.Unspecified;var tool=Color.Unspecified
        ui.setContent {GoatrTheme {
            user=MaterialTheme.colorScheme.primaryContainer;tool=MaterialTheme.colorScheme.surface
            Column(Modifier.width(240.dp).background(Color.Magenta)) {
                Box(Modifier.testTag("assistant_style")){HistoryRecordContainer(false,true){Box(Modifier.fillMaxWidth().height(64.dp))}}
                Box(Modifier.testTag("user_style")){HistoryRecordContainer(true,false){Box(Modifier.fillMaxWidth().height(64.dp))}}
                Box(Modifier.testTag("tool_style")){HistoryRecordContainer(false,false){Box(Modifier.fillMaxWidth().height(64.dp))}}
            }
        }}
        fun middle(tag:String):Int {val pixels=ui.onNodeWithTag(tag).captureToImage().toPixelMap();return pixels[pixels.width/2,pixels.height/2].toArgb()}
        assertEquals(Color.Magenta.toArgb(),middle("assistant_style"))
        assertEquals(user.toArgb(),middle("user_style"))
        assertEquals(tool.toArgb(),middle("tool_style"))
        assertTrue(ui.onNodeWithTag("history_user_message").fetchSemanticsNode().boundsInRoot.width<ui.onNodeWithTag("assistant_style").fetchSemanticsNode().boundsInRoot.width)
    }

    @Test fun presentationSwitchHasOneLabelledLargeTouchOwnerAndTogglesExactlyOnce() {
        var checked by mutableStateOf(false);var changes=0
        ui.setContent {GoatrTheme {Box(Modifier.width(300.dp)) {PresentationSwitchRow("Grouped workspace view","A local view preference, not a desktop change",checked,"presentation_switch"){checked=it;changes++}}}}
        val row=ui.onNodeWithTag("presentation_switch")
        row.assertHeightIsAtLeast(48.dp).assertTextContains("Grouped workspace view").assertTextContains("A local view preference, not a desktop change")
        row.assert(SemanticsMatcher.expectValue(SemanticsProperties.Role,androidx.compose.ui.semantics.Role.Switch))
        row.assertIsOff().performClick().assertIsOn()
        ui.runOnIdle {assertTrue(checked);assertEquals(1,changes)}
        ui.onAllNodes(hasClickAction()).assertCountEquals(1)
    }

    private fun samePixels(first:ImageBitmap,second:ImageBitmap):Boolean {
        if(first.width!=second.width||first.height!=second.height)return false
        val a=first.toPixelMap();val b=second.toPixelMap()
        for(y in 0 until a.height)for(x in 0 until a.width)if(a[x,y]!=b[x,y])return false
        return true
    }

    @Test fun actualCanvasAccessibilityChangesOnlyLocalViewAndReducedMotionKeepsBlinkingCellsVisible() {
        val attributes=GridAttributes(false,false,false,true,false,GridUnderline.None)
        val begin=GridBegin(GridMode.Replace,Uint(0),Uint(2),Uint(1),null,false,false,Uint(1),false)
        val cells=listOf(GridCell("X",Uint(1),GridColor.Default,GridColor.Default,attributes),GridCell(" ",Uint(1),GridColor.Default,GridColor.Default,attributes.copy(blink=false)))
        val grid=TerminalGrid(begin,cells,true)
        ui.setContent {GoatrTheme {TerminalGridCanvas(grid,TerminalAppearance.Dark,Modifier.width(240.dp).height(96.dp),motionEnabled=false)}}
        val canvas=ui.onNodeWithTag("terminal_grid")
        ui.onNodeWithText("X").assertDoesNotExist()
        val actions=canvas.fetchSemanticsNode().config[SemanticsActions.CustomActions]
        assertEquals(7,actions.size)
        fun action(label:String){ui.runOnIdle {assertTrue(actions.single {it.label==label}.action())}}
        val steady=canvas.captureToImage()
        ui.mainClock.advanceTimeBy(750)
        assertTrue(samePixels(steady,canvas.captureToImage()))
        repeat(20){action("Zoom in locally")}
        canvas.assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"400 percent zoom, view origin"))
        repeat(30){action("Zoom out locally")}
        canvas.assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"50 percent zoom, view origin"))
        action("Reset local view");action("Move view right")
        canvas.assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"100 percent zoom, local pan active"))
        assertFalse(samePixels(steady,canvas.captureToImage()))
        action("Reset local view")
        assertTrue(samePixels(steady,canvas.captureToImage()))
        assertSame(begin,grid.begin);assertSame(cells,grid.cells)
    }

    private fun readerRecord(index:Int)=HistoryRecord(Id("reader-$index"),order=Uint(index.toLong()),role=Role.Assistant,kind=RecordKind.Message,source=RecordSource.Live,complete=true,sourceTruncated=false)

    private fun actualReaderScroll(scope:CoroutineScope,transcript:LazyListState,index:Int,offset:Int) {
        lateinit var finished:Deferred<Unit>
        ui.runOnIdle {finished=scope.async {transcript.scrollToItem(index,offset)}}
        ui.waitUntil(5000){finished.isCompleted}
        runBlocking {finished.await()}
    }

    @Test fun actualPaddedListAnchorRoundtripsWithoutCumulativeReentryOrReconnectDrift() {
        val history=List(10,::readerRecord)
        val reader=ConversationReaderPosition(null)
        var ticket=reader.attach()
        lateinit var transcript:LazyListState
        lateinit var scope:CoroutineScope
        var wanted=0
        ui.setContent {GoatrTheme {
            transcript=rememberLazyListState();scope=rememberCoroutineScope()
            wanted=with(LocalDensity.current){24.dp.roundToPx()}
            LazyColumn(Modifier.width(300.dp).height(180.dp),state=transcript,contentPadding=PaddingValues(vertical=8.dp)) {
                item(key="conversation_header"){Box(Modifier.height(48.dp))}
                items(history,key={"history_${it.id.value}"}) {record->Box(Modifier.fillMaxWidth().height(64.dp)){androidx.compose.material3.Text(record.id.value)}}
                item(key="conversation_activity_tail"){Box(Modifier.height(48.dp))}
            }
        }}
        actualReaderScroll(scope,transcript,3,wanted)
        fun capture() {ui.runOnIdle {
            assertTrue(transcript.layoutInfo.viewportStartOffset<0)
            val shown=transcript.layoutInfo.visibleItemsInfo.first {it.size>0&&it.key is String}
            val record=history[shown.index-1]
            assertTrue(reader.capture(ticket,shown.key as String,record.order.value,shown.offset,false,true,false,history.last().id))
            assertEquals(3,transcript.firstVisibleItemIndex)
            assertEquals(wanted,transcript.firstVisibleItemScrollOffset)
            assertEquals(wanted,reader.anchor!!.offset)
        }}
        capture()
        repeat(4) {
            ui.runOnIdle {ticket=reader.attach()}
            actualReaderScroll(scope,transcript,0,0)
            var index=0;var offset=0
            ui.runOnIdle {index=reader.locate(history,null,emptyList())!!;offset=reader.anchor!!.offset}
            actualReaderScroll(scope,transcript,index,offset)
            capture()
        }
    }

    @Test fun actualFollowingDrainsAppendDuringRestorationAndUsesTheFinalNativeTailAndCurrentTicket() {
        var records by mutableStateOf(List(4,::readerRecord))
        var listVisible by mutableStateOf(false)
        val reader=ConversationReaderPosition(null)
        val ticket=reader.attach()
        val restoring=mutableStateOf(true)
        val restoration=CompletableDeferred<Unit>()
        lateinit var transcript:LazyListState
        ui.setContent {GoatrTheme {
            transcript=rememberLazyListState()
            val history=rememberUpdatedState(records)
            val focused=rememberUpdatedState<Uuid?>(null)
            val older=rememberUpdatedState(false)
            LaunchedEffect(ticket){followConversationReader(reader,ticket,transcript,history,focused,older,restoring)}
            LaunchedEffect(Unit){restoration.await();restoring.value=false}
            if(listVisible)LazyColumn(Modifier.width(300.dp).height(180.dp).testTag("following_list"),state=transcript,contentPadding=PaddingValues(vertical=8.dp)) {
                item(key="conversation_header"){Box(Modifier.height(48.dp))}
                items(records,key={"history_${it.id.value}"}) {record->Box(Modifier.fillMaxWidth().height(64.dp).testTag("follow_${record.id.value}")){androidx.compose.material3.Text(record.id.value)}}
                item(key="conversation_activity_tail"){Box(Modifier.height(48.dp))}
                item(key="retained_receipt"){Box(Modifier.height(300.dp))}
            } else Box(Modifier.width(300.dp).height(180.dp))
        }}
        ui.runOnIdle {assertTrue(restoring.value);records=records+readerRecord(4)}
        restoration.complete(Unit)
        ui.waitUntil(5000){!restoring.value}
        // The follower must also take current state after its real layout wait, not the
        // history/index captured before either append or before this list was attached.
        ui.runOnIdle {records=records+readerRecord(5);listVisible=true}
        ui.waitUntil(5000){
            val layout=transcript.layoutInfo
            layout.visibleItemsInfo.any {it.key=="history_reader-5"&&it.offset+it.size<=layout.viewportEndOffset}
        }
        ui.onNodeWithTag("follow_reader-5").assertIsDisplayed()
        val viewport=ui.onNodeWithTag("following_list").getUnclippedBoundsInRoot()
        val tail=ui.onNodeWithTag("follow_reader-5").getUnclippedBoundsInRoot()
        assertTrue(tail.top>=viewport.top&&tail.bottom<=viewport.bottom)
        assertEquals(viewport.bottom.value,tail.bottom.value,.5f)
        ui.runOnIdle {assertTrue(reader.following);assertEquals(Id("reader-5"),reader.readTail)}
        var beforeIndex=0;var beforeOffset=0
        ui.runOnIdle {
            reader.retire();beforeIndex=transcript.firstVisibleItemIndex;beforeOffset=transcript.firstVisibleItemScrollOffset
            records=records+readerRecord(6)
        }
        ui.waitForIdle()
        ui.runOnIdle {
            assertFalse(reader.current(ticket))
            assertEquals(beforeIndex,transcript.firstVisibleItemIndex)
            assertEquals(beforeOffset,transcript.firstVisibleItemScrollOffset)
            assertEquals(Id("reader-5"),reader.readTail)
        }
    }

    @Test fun olderHistoryActualCollapseGestureKeepsRoutineRowsHiddenAcrossNativeUpdates() {
        val reader=ConversationReaderPosition(null)
        var older by mutableStateOf(false)
        val routine=readerRecord(0).copy(role=Role.Tool,kind=RecordKind.Tool,tool=Tool("Routine",status=ToolStatus.Succeeded))
        val failed=routine.copy(id=Id("failed"),order=Uint(1),tool=Tool("Failed",status=ToolStatus.Failed))
        val running=routine.copy(id=Id("running"),order=Uint(2),complete=false,tool=Tool("Running",status=ToolStatus.Running))
        val distinct=routine.copy(id=Id("distinct"),order=Uint(3),text=ContentText(InlineText("Distinct native text")))
        var history by mutableStateOf(listOf(routine,failed,running,distinct))
        val request=PendingRequest(Uuid.random(),Provider.Omp,PresentationKind.OmpEditor,PresentationOmpEditor(ContentText(InlineText("Native pending request")),ContentText(InlineText(""))))
        ui.setContent {GoatrTheme {Column(Modifier.width(300.dp).height(500.dp).verticalScroll(androidx.compose.foundation.rememberScrollState())) {
            val expanded=observeReaderToolExpansion(reader,older)
            val routineIds=history.filter {isRoutineToolRecord(it,null)}.map {it.id}.toSet()
            history.forEach {record->if(expanded||record.id !in routineIds)androidx.compose.material3.Text(record.id.value,modifier=Modifier.testTag("older_${record.id.value}"))}
            RequestDock(listOf(request)) {}
            ToolActivitySummary(routineIds.size,expanded){reader.toolsExpanded=!expanded}
        }}}
        ui.runOnIdle {older=true}
        ui.onNodeWithTag("older_reader-0").assertExists()
        ui.onNodeWithTag("tool_activity_summary").performScrollTo().performClick()
        ui.onNodeWithTag("older_reader-0").assertDoesNotExist()
        ui.runOnIdle {history=history+readerRecord(4)}
        ui.onNodeWithTag("older_reader-0").assertDoesNotExist()
        for(id in listOf("failed","running","distinct","reader-4"))ui.onNodeWithTag("older_$id").assertExists()
        ui.onNodeWithTag("pending_cards").assertExists()
        ui.runOnIdle {assertFalse(reader.toolsExpanded);older=false}
        ui.runOnIdle {older=true}
        ui.onNodeWithTag("older_reader-0").assertExists()
    }
}
