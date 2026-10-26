// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.text.Selection
import android.view.KeyEvent
import android.view.View
import android.view.Window
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class TerminalInputConnectionTest {
    @get:Rule val ui=createComposeRule()
    private fun receipt()=Receipt(actionId=Uuid.random(),method=ActionMethod.ShellInput,bootId=Uuid.random(),
        createdAt=Uint(1),updatedAt=Uint(1),state=ReceiptState.Dispatched,shellTarget=ShellTarget(Uuid.random(),Uuid.random()),
        result=ActionResultShellDispatch(ShellDispatchProgress(19,19,true)))

    @Test fun compositionCommitsOnceRealSdkPhasesStayKeysAndRetirementNeverCommits() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val writes=mutableListOf<ShellInput>()
        var fresh=true;val revalidated=kotlinx.coroutines.CompletableDeferred<Unit>()
        lateinit var heldPress:KeyEvent
        lateinit var priorConnection:android.view.inputmethod.InputConnection
        lateinit var view:DirectTerminalInputView
        lateinit var owner:TerminalInputOwner
        ui.setContent {
            AndroidView(modifier=Modifier.fillMaxWidth().height(64.dp).testTag("terminal-input-bridge"),factory={context->
                owner=TerminalInputOwner(scope,{true},{fresh},{if(!fresh)revalidated.await()},{0},{input,_->assertTrue(fresh);writes.add(input);receipt()},{_,_->},{},{})
                DirectTerminalInputView(context).also {view=it;it.bind(owner)}
            })
        }
        lateinit var originalWindow:Window;lateinit var originalRoot:View
        ui.runOnIdle {
            var context=view.context
            while(context is android.content.ContextWrapper&&context !is android.app.Activity) {
                val parent=context.baseContext;check(parent!==context);context=parent
            }
            originalWindow=(context as android.app.Activity).window;originalRoot=originalWindow.decorView.rootView
        }
        // Natural focus on this SAME attached owner. No foreground/window replacement or focus forcing.
        try {
            ui.waitUntil(10000) {
                var current=false
                instrumentation.runOnMainSync {current=view.isAttachedToWindow&&view.rootView===originalRoot&&originalWindow.decorView.rootView===originalRoot&&view.hasWindowFocus()}
                current
            }
            ui.onNodeWithTag("terminal-input-bridge").performTouchInput {click()}
            ui.runOnIdle {
                assertTrue(view.isAttachedToWindow&&view.rootView===originalRoot&&view.hasWindowFocus())
                val connection=checkNotNull(view.onCreateInputConnection(EditorInfo()))
                assertTrue(connection.setComposingText("transient",1));assertTrue(writes.isEmpty())
                assertTrue(connection.commitText("x😀",1));assertTrue(connection.finishComposingText())
                assertTrue(connection.deleteSurroundingText(2,0))
                assertTrue(connection.setComposingText("not committed on close",1))
                val next=checkNotNull(view.onCreateInputConnection(EditorInfo()))
                assertTrue(next.setComposingText("replacement transient",1))
                val replacement=view.editableText;val replacementText=replacement.toString()
                val selectionStart=Selection.getSelectionStart(replacement);val selectionEnd=Selection.getSelectionEnd(replacement)
                val composingStart=BaseInputConnection.getComposingSpanStart(replacement);val composingEnd=BaseInputConnection.getComposingSpanEnd(replacement)
                val pendingEvents=owner.pendingEvents;val beforeWrites=writes.toList()
                connection.closeConnection()
                assertFalse(connection.commitText("late",1));assertFalse(connection.finishComposingText())
                // BaseInputConnection may report true with a null retired editable; assert isolation, not its return.
                connection.setSelection(0,0);connection.setComposingRegion(0,1)
                assertNull(connection.getTextBeforeCursor(32,0))
                assertNull(connection.getTextAfterCursor(32,0));assertNull(connection.getSelectedText(0))
                assertSame(replacement,view.editableText);assertEquals(replacementText,view.editableText.toString())
                assertEquals(selectionStart,Selection.getSelectionStart(replacement));assertEquals(selectionEnd,Selection.getSelectionEnd(replacement))
                assertEquals(composingStart,BaseInputConnection.getComposingSpanStart(replacement));assertEquals(composingEnd,BaseInputConnection.getComposingSpanEnd(replacement))
                assertEquals(pendingEvents,owner.pendingEvents);assertEquals(beforeWrites,writes)
                assertTrue(next.setComposingText("",1))
                val now=android.os.SystemClock.uptimeMillis()
                val press=KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_LEFT,0,KeyEvent.META_CTRL_ON,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                val repeat=KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_LEFT,7,KeyEvent.META_CTRL_ON,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                val release=KeyEvent(now,now,KeyEvent.ACTION_UP,KeyEvent.KEYCODE_DPAD_LEFT,0,KeyEvent.META_CTRL_ON,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                assertTrue(view.onKeyDown(press.keyCode,press));assertTrue(view.onKeyDown(repeat.keyCode,repeat));assertTrue(view.onKeyUp(release.keyCode,release))
                assertFalse(next.deleteSurroundingText(1,0)) // Navigation invalidated UTF-16 context, not a native Backspace gesture.
                assertTrue(next.sendKeyEvent(KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DEL,0,0,android.view.KeyCharacterMap.VIRTUAL_KEYBOARD,0,KeyEvent.FLAG_SOFT_KEYBOARD,android.view.InputDevice.SOURCE_KEYBOARD)))
            }
            ui.waitUntil(10000) {
                var count=0;instrumentation.runOnMainSync {count=writes.size};count==6
            }
            ui.runOnIdle {
                assertEquals(ShellInput.Text("x😀"),writes.first())
                assertEquals(1,(writes[1] as ShellInput.Keys).events.size)
                assertEquals(ShellKey(ShellKeyName.Backspace),(writes[1] as ShellInput.Keys).events.single().key)
                val events=writes.subList(2,5).flatMap {(it as ShellInput.Keys).events}
                assertEquals(listOf(ShellKeyPhase.Press,ShellKeyPhase.Repeat,ShellKeyPhase.Release),events.map {it.phase})
                assertTrue(events.all {it.key==ShellKey(ShellKeyName.Left)&&it.modifiers.ctrl})
                assertEquals(1,events.map {it.physicalKeyId}.distinct().size);assertNotNull(events.first().physicalKeyId)
                assertFalse(owner.hardwareOutstanding())
                val deletion=(writes.last() as ShellInput.Keys).events.single()
                assertEquals(ShellKey(ShellKeyName.Backspace),deletion.key);assertEquals(ShellKeyPhase.Press,deletion.phase);assertNull(deletion.physicalKeyId)
                val now=android.os.SystemClock.uptimeMillis()
                heldPress=KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_LEFT,0,0,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                assertTrue(view.dispatchKeyEvent(heldPress));priorConnection=checkNotNull(view.onCreateInputConnection(EditorInfo()))
                assertTrue(priorConnection.setComposingText("discard on refresh",1))
            }
            ui.waitUntil(10000){var count=0;instrumentation.runOnMainSync{count=writes.size};count==7}
            ui.runOnIdle {
                fresh=false;view.bind(owner);assertTrue(owner.retained());assertFalse(owner.accepting())
                assertTrue(view.isFocused&&view.isAttachedToWindow&&view.rootView===originalRoot&&originalWindow.decorView.rootView===originalRoot&&view.hasWindowFocus())
                assertTrue(view.editableText.isEmpty())
                assertEquals(-1,BaseInputConnection.getComposingSpanStart(view.editableText));assertEquals(-1,BaseInputConnection.getComposingSpanEnd(view.editableText))
                assertFalse(view.onCheckIsTextEditor())
                assertFalse(priorConnection.commitText("not admitted while dirty",1));assertFalse(priorConnection.setComposingText("not composed while dirty",1));assertFalse(priorConnection.finishComposingText())
                assertFalse(view.onTextContextMenuItem(android.R.id.paste))
                val newPress=KeyEvent(heldPress.downTime+1,heldPress.eventTime+1,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_RIGHT,0,0,heldPress.deviceId,heldPress.scanCode+1,0,heldPress.source)
                assertFalse(view.dispatchKeyEvent(newPress))
                val staleRelease=KeyEvent(heldPress.downTime-1,heldPress.eventTime,KeyEvent.ACTION_UP,heldPress.keyCode,0,0,heldPress.deviceId,heldPress.scanCode,0,heldPress.source)
                assertTrue(view.dispatchKeyEvent(staleRelease));assertEquals(7,writes.size);assertEquals(0,owner.pendingEvents)
                val release=KeyEvent.changeAction(heldPress,KeyEvent.ACTION_UP)
                assertTrue(priorConnection.sendKeyEvent(release))
                assertTrue(view.dispatchKeyEvent(release)) // The same release is not dispatched twice.
                assertEquals(7,writes.size);assertEquals(1,owner.pendingEvents)
                fresh=true;revalidated.complete(Unit);view.bind(owner)
            }
            ui.waitUntil(10000){var count=0;instrumentation.runOnMainSync{count=writes.size};count==8}
            ui.runOnIdle {
                assertEquals(ShellKeyPhase.Release,(writes.last() as ShellInput.Keys).events.single().phase)
                assertFalse(owner.hardwareOutstanding())
                assertTrue(view.isAttachedToWindow&&view.rootView===originalRoot&&view.hasWindowFocus())
                assertTrue(view.editableText.isEmpty())
                assertTrue(priorConnection.finishComposingText());assertEquals(8,writes.size)
                assertTrue(priorConnection.commitText("after refresh",1))
            }
            ui.waitUntil(10000){var count=0;instrumentation.runOnMainSync{count=writes.size};count==9}
            ui.runOnIdle {
                assertEquals(ShellInput.Text("after refresh"),writes.last())
                val next=checkNotNull(view.onCreateInputConnection(EditorInfo()))
                val oversized=object:CharSequence {
                    override val length=16385
                    override fun get(index:Int):Char=throw AssertionError("Oversized commit was read")
                    override fun subSequence(startIndex:Int,endIndex:Int):CharSequence=throw AssertionError("Oversized commit was copied")
                    override fun toString():String=throw AssertionError("Oversized commit was copied")
                }
                val before=writes.toList()
                assertFalse(next.commitText(oversized,1));assertTrue(owner.paused)
                assertFalse(next.commitText("not accepted while paused",1));assertEquals(before,writes)
                next.closeConnection();assertEquals(before,writes)
            }
        }finally {
            instrumentation.runOnMainSync {view.release();owner.retire(discard=true);scope.cancel()}
        }
    }

    @Test fun realEditorFocusLossRetiresHeldInputWithoutAReleaseReplay() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        var writes=0;var stopped:TerminalInputPause?=null
        lateinit var view:DirectTerminalInputView;lateinit var owner:TerminalInputOwner
        lateinit var connection:android.view.inputmethod.InputConnection;lateinit var heldPress:KeyEvent
        ui.setContent {
            AndroidView(modifier=Modifier.fillMaxWidth().height(64.dp).testTag("terminal-focus-loss"),factory={context->
                owner=TerminalInputOwner(scope,{true},{true},{},{0},{_,_->writes++;receipt()},{_,_->},{},{stopped=it})
                DirectTerminalInputView(context).also {view=it;it.bind(owner)}
            })
        }
        lateinit var originalWindow:Window;lateinit var originalRoot:View
        ui.runOnIdle {
            var context=view.context
            while(context is android.content.ContextWrapper&&context !is android.app.Activity) {
                val parent=context.baseContext;check(parent!==context);context=parent
            }
            originalWindow=(context as android.app.Activity).window;originalRoot=originalWindow.decorView.rootView
        }
        try {
            ui.waitUntil(10000) {
                var current=false
                instrumentation.runOnMainSync {current=view.isAttachedToWindow&&view.rootView===originalRoot&&originalWindow.decorView.rootView===originalRoot&&view.hasWindowFocus()}
                current
            }
            ui.onNodeWithTag("terminal-focus-loss").performTouchInput {click()}
            ui.runOnIdle {
                assertTrue(view.isFocused&&view.isAttachedToWindow&&view.rootView===originalRoot&&view.hasWindowFocus())
                connection=checkNotNull(view.onCreateInputConnection(EditorInfo()))
                val now=android.os.SystemClock.uptimeMillis()
                heldPress=KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_LEFT,0,0,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                assertTrue(view.dispatchKeyEvent(heldPress));assertTrue(owner.hardwareOutstanding())
            }
            ui.waitUntil(10000){var count=0;instrumentation.runOnMainSync{count=writes};count==1}
            ui.runOnIdle {
                view.clearFocus() // Exercise real SDK focus loss, never force focus or a window replacement.
                assertFalse(owner.retained());assertFalse(owner.accepting());assertEquals(TerminalInputPause.SourceLost,stopped)
                assertFalse(connection.commitText("not admitted after focus loss",1));assertNull(connection.getTextBeforeCursor(32,0))
                assertNull(view.onCreateInputConnection(EditorInfo()))
                assertFalse(view.dispatchKeyEvent(KeyEvent.changeAction(heldPress,KeyEvent.ACTION_UP)))
                assertEquals(0,owner.pendingEvents);assertEquals(1,writes)
            }
        }finally {
            instrumentation.runOnMainSync {view.release();owner.retire(discard=true);scope.cancel()}
        }
    }
    @Test fun focusedHistoryCleanupReleasesOnlyItsConfirmedSdkHold() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val releaseAck=kotlinx.coroutines.CompletableDeferred<Unit>();val writes=mutableListOf<ShellInput>();var stops=0
        lateinit var view:DirectTerminalInputView;lateinit var owner:TerminalInputOwner
        lateinit var originalPress:KeyEvent
        ui.setContent {
            AndroidView(modifier=Modifier.fillMaxWidth().height(64.dp).testTag("terminal-history-cleanup"),factory={context->
                owner=TerminalInputOwner(scope,{true},{true},{},{0},{input,_->
                    writes.add(input);if((input as? ShellInput.Keys)?.events?.any {it.phase==ShellKeyPhase.Release}==true)releaseAck.await()
                    receipt()
                },{_,_->},{},{stops++})
                DirectTerminalInputView(context).also {view=it;it.bind(owner)}
            })
        }
        lateinit var root:View;lateinit var window:Window
        ui.runOnIdle {
            var context=view.context
            while(context is android.content.ContextWrapper&&context !is android.app.Activity){val parent=context.baseContext;check(parent!==context);context=parent}
            window=(context as android.app.Activity).window;root=window.decorView.rootView
        }
        try {
            ui.waitUntil(10000){var ready=false;instrumentation.runOnMainSync{ready=view.isAttachedToWindow&&view.rootView===root&&window.decorView.rootView===root&&view.hasWindowFocus()};ready}
            ui.onNodeWithTag("terminal-history-cleanup").performTouchInput {click()}
            ui.runOnIdle {
                val now=android.os.SystemClock.uptimeMillis()
                originalPress=KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_LEFT,0,0,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                assertTrue(view.dispatchKeyEvent(originalPress))
            }
            ui.waitUntil(10000){var done=false;instrumentation.runOnMainSync{done=writes.size==1&&owner.pendingEvents==0};done}
            ui.runOnIdle {
                owner.blockForHistory(true);view.bind(owner)
                assertTrue(owner.retained());assertFalse(owner.accepting());assertFalse(owner.paused);assertEquals(0,stops)
                assertTrue(view.isEnabled&&view.isFocused&&view.isAttachedToWindow&&view.hasWindowFocus())
            }
            ui.waitUntil(10000){var done=false;instrumentation.runOnMainSync{done=writes.size==2};done}
            ui.runOnIdle {
                val press=(writes[0] as ShellInput.Keys).events.single();val release=(writes[1] as ShellInput.Keys).events.single()
                assertEquals(ShellKeyPhase.Press,press.phase);assertEquals(ShellKeyPhase.Release,release.phase)
                assertNotNull(press.physicalKeyId);assertEquals(press.physicalKeyId,release.physicalKeyId);assertEquals(press.key,release.key)
                assertTrue(owner.hardwareOutstanding());assertTrue(owner.retained());assertEquals(0,stops)
                releaseAck.complete(Unit)
            }
            ui.waitUntil(10000){var done=false;instrumentation.runOnMainSync{done=owner.pendingEvents==0};done}
            ui.runOnIdle {
                assertFalse(owner.hardwareOutstanding());assertTrue(owner.retained());assertFalse(owner.paused);assertEquals(2,writes.size)
                owner.blockForHistory(false);assertTrue(owner.accepting());assertEquals(0,stops)
                view.bind(owner)
                val repeat=KeyEvent(originalPress.downTime,android.os.SystemClock.uptimeMillis(),KeyEvent.ACTION_DOWN,originalPress.keyCode,1,0,originalPress.deviceId,originalPress.scanCode,0,android.view.InputDevice.SOURCE_KEYBOARD)
                val up=KeyEvent(originalPress.downTime,android.os.SystemClock.uptimeMillis(),KeyEvent.ACTION_UP,originalPress.keyCode,0,0,originalPress.deviceId,originalPress.scanCode,0,android.view.InputDevice.SOURCE_KEYBOARD)
                assertTrue(view.dispatchKeyEvent(repeat));assertTrue(view.dispatchKeyEvent(up))
                assertEquals(2,writes.size);assertTrue(owner.retained());assertFalse(owner.paused)
            }
        }finally{instrumentation.runOnMainSync{releaseAck.complete(Unit);view.release();owner.retire(discard=true);scope.cancel()}}
    }
    private fun historyLifecycleLossStopsCleanup(loss:String) {
        val instrumentation=InstrumentationRegistry.getInstrumentation();val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val revalidated=kotlinx.coroutines.CompletableDeferred<Unit>();var fresh=true;val writes=mutableListOf<ShellInput>();var stopped:TerminalInputPause?=null
        lateinit var view:DirectTerminalInputView;lateinit var owner:TerminalInputOwner;var replacement:TerminalInputOwner?=null;var dialog:android.app.Dialog?=null
        ui.setContent {
            AndroidView(modifier=Modifier.fillMaxWidth().height(64.dp).testTag("history-lifecycle"),factory={context->
                owner=TerminalInputOwner(scope,{true},{fresh},{if(!fresh)revalidated.await()},{0},{input,_->writes.add(input);receipt()},{_,_->},{},{stopped=it})
                DirectTerminalInputView(context).also{view=it;it.bind(owner)}
            })
        }
        lateinit var window:Window;lateinit var root:View;lateinit var activity:android.app.Activity
        ui.runOnIdle {
            var context=view.context
            while(context is android.content.ContextWrapper&&context !is android.app.Activity){val parent=context.baseContext;check(parent!==context);context=parent}
            activity=context as android.app.Activity;window=activity.window;root=window.decorView.rootView
        }
        try {
            ui.waitUntil(10000){var ready=false;instrumentation.runOnMainSync{ready=view.isAttachedToWindow&&view.rootView===root&&window.decorView.rootView===root&&view.hasWindowFocus()};ready}
            ui.onNodeWithTag("history-lifecycle").performTouchInput{click()}
            ui.runOnIdle {
                val now=android.os.SystemClock.uptimeMillis()
                val press=KeyEvent(now,now,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_DPAD_LEFT,0,0,1,21,0,android.view.InputDevice.SOURCE_KEYBOARD)
                assertTrue(view.dispatchKeyEvent(press))
            }
            ui.waitUntil(10000){var complete=false;instrumentation.runOnMainSync{complete=writes.size==1&&owner.pendingEvents==0};complete}
            ui.runOnIdle {
                fresh=false;owner.blockForHistory(true);view.bind(owner)
                assertTrue(view.isEnabled&&view.isFocused&&view.isAttachedToWindow&&view.hasWindowFocus())
                when(loss) {
                    "replacement"->{replacement=TerminalInputOwner(scope,{true},{true},{},{0},{input,_->writes.add(input);receipt()},{_,_->},{},{});view.bind(replacement)}
                    "detach"->{(view.parent as android.view.ViewGroup).removeView(view);assertFalse(view.isAttachedToWindow)}
                    "focus"->{view.clearFocus();assertFalse(view.isFocused)}
                    "window"->{dialog=android.app.Dialog(activity).also{it.setContentView(android.widget.TextView(activity).apply{text="Owned history window loss"});it.show()}}
                    else->error("unknown owned lifecycle loss")
                }
            }
            ui.waitUntil(10000){var paused=false;instrumentation.runOnMainSync{paused=owner.paused};paused}
            ui.runOnIdle {
                assertFalse(owner.retained());assertFalse(owner.accepting());assertEquals(TerminalInputPause.SourceLost,stopped)
                if(loss=="window")assertFalse(view.hasWindowFocus())
                fresh=true;revalidated.complete(Unit)
            }
            ui.waitForIdle()
            ui.runOnIdle {assertEquals(1,writes.size);assertEquals(0,owner.pendingEvents);assertFalse(owner.offer(ShellInput.Text("must not resume after real view loss")))}
        }finally{instrumentation.runOnMainSync{dialog?.dismiss();revalidated.complete(Unit);view.release();owner.retire(discard=true);replacement?.retire(discard=true);scope.cancel()}}
    }
    @Test fun historyOwnerReplacementStopsConfirmedCleanup()=historyLifecycleLossStopsCleanup("replacement")
    @Test fun historyNativeDetachStopsConfirmedCleanup()=historyLifecycleLossStopsCleanup("detach")
    @Test fun historyNativeFocusLossStopsConfirmedCleanup()=historyLifecycleLossStopsCleanup("focus")
    @Test fun historyNativeWindowLossStopsConfirmedCleanup()=historyLifecycleLossStopsCleanup("window")
}
