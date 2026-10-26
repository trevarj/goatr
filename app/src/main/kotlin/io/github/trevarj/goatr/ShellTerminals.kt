// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.compose.ui.text.style.TextOverflow

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.selects.select
import androidx.compose.ui.viewinterop.AndroidView

internal data class TerminalPendingText(val text:String,val draftRevision:Long?) {
    override fun toString()="TerminalPendingText(redacted)"
}
internal enum class TerminalInputPause {Capacity,SourceLost,Uncertain,Refused,Limit,Unsupported,Closed}
/** Text facts only, not a serializable/replayable key sequence. Shortcut and release events
 * never become text; a user must explicitly review/send any recovered draft. */
internal fun ShellInput.terminalTextSnapshot():String?=when(this) {
    is ShellInput.Text->text
    is ShellInput.Keys->{
        var result:StringBuilder?=null
        for(event in events)if(event.phase!=ShellKeyPhase.Release&&event.key.key==ShellKeyName.Character
            &&!event.modifiers.ctrl&&!event.modifiers.alt&&!event.modifiers.superKey) {
            val builder=result ?: StringBuilder(events.size*2).also {result=it}
            builder.append(event.key.value)
        }
        result?.toString()
    }
}


/** One memory-only ordered owner for one captured control lease, never a reconnect queue. */
internal class TerminalInputOwner(
    scope:CoroutineScope,
    private val current:()->Boolean,
    private val ready:()->Boolean,
    private val awaitReady:suspend ()->Unit,
    private val draftBytes:()->Int,
    private val write:suspend (ShellInput,Long?)->Receipt,
    private val completed:(Long?,Receipt)->Unit,
    private val retain:(List<TerminalPendingText>)->Unit,
    private val stopped:(TerminalInputPause)->Unit,
) {
    private class HeldRelease(val physical:Long,val gesture:Any)
    private data class Pending(val input:ShellInput,val revision:Long?,val events:Int,val bytes:Int) {
        fun text()=input.terminalTextSnapshot()?.let {TerminalPendingText(it,revision)}
        override fun toString()="PendingTerminalInput(redacted)"
        var releases:MutableList<HeldRelease>?=null
    }
    private val gate=Any()
    private val waiting=ArrayDeque<Pending>()
    private val wake=Channel<Unit>(Channel.CONFLATED)
    private val stopping=CompletableDeferred<Unit>()
    private var readyPending:Pending?=null
    private var writing=false
    private var deferredText=emptyList<TerminalPendingText>()
    private val held=mutableMapOf<Long,Any>()
    private var events=0;private var bytes=0
    private var retired=false;private var discardText=false
    val committedContext=TerminalCommittedContext()
    var paused by mutableStateOf(false);private set
    var pendingEvents by mutableStateOf(0);private set
    var pendingTextBytes by mutableStateOf(0);private set
    fun retained()=synchronized(gate){!retired&&!paused}&&!job.isCompleted&&current()
    fun accepting()=retained()&&ready()
    fun hardwareOutstanding()=synchronized(gate){held.isNotEmpty()}
    fun unavailable(){pause(TerminalInputPause.SourceLost)}
    fun limit(){pause(TerminalInputPause.Limit)}
    fun refuse(text:String?=null) {
        val pending=text?.takeIf {it.isNotEmpty()&&validUnicode(it)&&utf8Size(it)<=16384}?.let {Pending(ShellInput.Text(it),null,1,utf8Size(it))}
        pause(TerminalInputPause.Unsupported,pending)
    }
    private fun pause(reason:TerminalInputPause,extra:Pending?=null) {
        val texts=synchronized(gate) {
            if(paused){return}
            paused=true
            committedContext.invalidate()
            val recovered=buildList<TerminalPendingText> {
                if(!discardText) {
                    readyPending?.text()?.let(::add)
                    for(pending in waiting)pending.text()?.let(::add)
                    extra?.text()?.let(::add)
                }
            }
            readyPending?.let {events-=it.events;bytes-=it.bytes};readyPending=null
            for(pending in waiting){events-=pending.events;bytes-=pending.bytes}
            waiting.clear();pendingEvents=events;pendingTextBytes=bytes
            if(writing){deferredText=recovered;emptyList()}else recovered
        }
        stopping.complete(Unit);wake.close()
        if(texts.isNotEmpty())retain(texts)
        stopped(reason)
    }
    fun retire(discard:Boolean=false) {
        synchronized(gate){retired=true;discardText=discardText||discard;if(discardText)deferredText=emptyList()}
        pause(TerminalInputPause.Closed);wake.close()
    }
    fun offer(input:ShellInput,revision:Long?=null)=enqueue(input,revision,null)
    fun offerDeletion(deletion:TerminalCommittedContext.Deletion):Boolean {
        if(!committedContext.matches(deletion)||deletion.before<0||deletion.after<0||deletion.before+deletion.after !in 1..32)return false
        val modifiers=ShellKeyModifiers(false,false,false,false)
        val events=List(deletion.before){ShellKeyEvent(ShellKey(ShellKeyName.Backspace),modifiers,ShellKeyPhase.Press)}+
            List(deletion.after){ShellKeyEvent(ShellKey(ShellKeyName.Delete),modifiers,ShellKeyPhase.Press)}
        return enqueue(ShellInput.Keys(events),null,deletion)
    }
    private fun enqueue(input:ShellInput,revision:Long?,deletion:TerminalCommittedContext.Deletion?):Boolean {
        input.validate()
        val count=when(input){is ShellInput.Text->1;is ShellInput.Keys->input.events.size}
        val size=when(input){is ShellInput.Text->utf8Size(input.text);is ShellInput.Keys->input.events.sumOf {utf8Size(it.key.value.orEmpty())}}
        val pending=Pending(input,revision,count,size)
        if(!current()){pause(TerminalInputPause.SourceLost,pending);return false}
        val ownedRelease=synchronized(gate){input is ShellInput.Keys&&input.events.all {event->
            event.phase==ShellKeyPhase.Release&&event.physicalKeyId?.value?.let(held::containsKey)==true}}
        if(!ready()&&!ownedRelease)return false
        val accepted=synchronized(gate) {
            if(retired||paused)return false
            val keys=(input as? ShellInput.Keys)?.events.orEmpty()
            // ponytail: reserve one maximum16KiB rejected commit inside the SAME64KiB
            // draft/recovery budget; earlier backpressure needs no additional buffer.
            if(events+count>128||bytes+size>49152||draftBytes()+bytes+size>49152)false
            else{
                for(event in keys)event.physicalKeyId?.let {id->when(event.phase) {
                    ShellKeyPhase.Press->held[id.value]=Any()
                    ShellKeyPhase.Release->held[id.value]?.let {gesture->
                        val releases=pending.releases ?: ArrayList<HeldRelease>().also {pending.releases=it}
                        releases.add(HeldRelease(id.value,gesture))
                    }
                    ShellKeyPhase.Repeat->Unit
                }}
                if(held.size>128)false else{
                if(deletion!=null) {
                    if(!committedContext.deleted(deletion))return false
                }else if(input is ShellInput.Text)committedContext.commit(input.text)else committedContext.invalidate()
                waiting.addLast(pending);events+=count;bytes+=size
                pendingEvents=events;pendingTextBytes=bytes;true
                }
            }
        }
        if(!accepted){pause(TerminalInputPause.Capacity,pending);return false}
        wake.trySend(Unit);return true
    }
    private suspend fun waitReady():Boolean {
        if(ready())return current()
        return coroutineScope {
            val readiness=async(start=CoroutineStart.UNDISPATCHED){awaitReady()}
            try {select {stopping.onAwait {false};readiness.onAwait {current()&&ready()}}}
            finally {readiness.cancel()}
        }
    }
    private val job=scope.launch {
        try {
            for(signal in wake) {
                while(true) {
                    val pending=synchronized(gate){if(retired||paused)null else waiting.removeFirstOrNull()?.also {readyPending=it}} ?: break
                    if(!current()||!waitReady()){pause(TerminalInputPause.SourceLost);break}
                    val entered=synchronized(gate){if(retired||paused)false else{readyPending=null;writing=true;true}}
                    if(!entered)break
                    val receipt=try {write(pending.input,pending.revision)}catch(_:Exception){null}
                    val dispatched=receipt?.shellDispatched()==true
                    if(!dispatched)pause(if(receipt?.state==ReceiptState.Failed)TerminalInputPause.Refused else TerminalInputPause.Uncertain)
                    val texts=synchronized(gate) {
                        writing=false
                        events-=pending.events;bytes-=pending.bytes;pendingEvents=events;pendingTextBytes=bytes
                        if(dispatched)for(release in pending.releases.orEmpty())
                            if(held[release.physical]===release.gesture)held.remove(release.physical)
                        val recovered=if(paused&&!discardText) {
                            if(receipt?.state==ReceiptState.Failed)buildList<TerminalPendingText> {
                                pending.text()?.let(::add);addAll(deferredText)
                            }else deferredText
                        }else emptyList()
                        deferredText=emptyList()
                        recovered
                    }
                    if(texts.isNotEmpty())retain(texts)
                    if(dispatched){completed(pending.revision,checkNotNull(receipt))}
                    else break
                }
            }
        }finally{
            pause(TerminalInputPause.SourceLost)
        }
    }
}

/** Bound before copying an SDK CharSequence; never truncate or normalize a committed scalar. */
internal fun boundedTerminalCommit(value:CharSequence?):String? {
    if(value==null)return ""
    if(value.length>16384)return null
    if(value is String)return value.takeIf {validUnicode(it)&&utf8Size(it)<=16384}
    val result=StringBuilder(value.length);var i=0;var bytes=0
    while(i<value.length) {
        if(i>=16384)return null
        val c=value[i++]
        val extra=when {
            c.isHighSurrogate()->{
                if(i>=value.length||!value[i].isLowSurrogate())return null
                if(bytes+4>16384)return null
                result.append(c).append(value[i++]);bytes+=4;continue
            }
            c.isLowSurrogate()->return null
            c.code<128->1
            c.code<2048->2
            else->3
        }
        bytes+=extra;if(bytes>16384)return null;result.append(c)
    }
    return result.toString()
}

/** A64-UTF16-unit suffix of this original control owner's own accepted commits, never native cells.
 * Unknown cursor/selection/source state discards it. No prefix or grapheme behavior is guessed. */
internal class TerminalCommittedContext {
    internal class Deletion(val before:Int,val after:Int,internal val remaining:Int,internal val context:TerminalCommittedContext,internal val version:Long)
    private val suffix=CharArray(64)
    private var count=0
    private var known=false
    private var version=0L
    fun invalidate(){suffix.fill('\u0000');count=0;known=false;version++}
    fun commit(text:String) {
        if(text.isEmpty())return
        if(!validUnicode(text)||text.any {Character.isISOControl(it.code)}){invalidate();return}
        if(!known)count=0
        if(text.length>=suffix.size) {
            var start=text.length-suffix.size
            if(text[start].isLowSurrogate())start++
            count=text.length-start;text.toCharArray(suffix,0,start,text.length)
        }else{
            var start=(count-(suffix.size-text.length)).coerceAtLeast(0)
            if(start<count&&suffix[start].isLowSurrogate())start++
            val keep=count-start
            suffix.copyInto(suffix,0,start,count);text.toCharArray(suffix,keep,0,text.length);count=keep+text.length
        }
        suffix.fill('\u0000',count);known=true;version++
    }
    fun deletion(before:Int,after:Int,codePoints:Boolean):Deletion? {
        if(before<0||after<0||before>32||after>32||before+after>32)return null
        if(codePoints) {
            val available=if(known)Character.codePointCount(suffix,0,count)else 0
            val remaining=if(known&&after==0&&before<=available)Character.offsetByCodePoints(suffix,0,count,count,-before)else -1
            return Deletion(before,after,remaining,this,version)
        }
        if(!known||after!=0||before>count)return null
        val start=count-before
        if(start<count&&suffix[start].isLowSurrogate())return null
        return Deletion(Character.codePointCount(suffix,start,before),0,start,this,version)
    }
    internal fun matches(deletion:Deletion)=deletion.context===this&&deletion.version==version&&deletion.remaining in -1..count
    fun deleted(deletion:Deletion):Boolean {
        if(!matches(deletion))return false
        if(deletion.remaining<0){invalidate();return true}
        suffix.fill('\u0000',deletion.remaining);count=deletion.remaining;known=true;version++;return true
    }
}

/** Standard Android editor surface; it retains only transient composition, never terminal output.
 * SDK commits and hardware events enter the same bounded original-lease owner exactly once. */
internal class DirectTerminalInputView(context:android.content.Context):android.widget.EditText(context) {
    private var owner:TerminalInputOwner?=null
    private var editor:TerminalConnection?=null
    private val committed get()=owner?.committedContext
    private var cursorFrame:GridBegin?=null
    private class HeldKey(var key:ShellKey,val downTime:Long)
    private val pressed=mutableMapOf<Long,HeldKey>()
    private val deadKeys=mutableMapOf<Long,Long>()
    private var accent=0
    init {
        hint="Direct terminal input · tap for keyboard"
        contentDescription="Direct terminal input"
        isSaveEnabled=false;setSaveFromParentEnabled(false)
        importantForAutofill=android.view.View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        inputType=android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE or android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        imeOptions=android.view.inputmethod.EditorInfo.IME_ACTION_NONE or android.view.inputmethod.EditorInfo.IME_FLAG_NO_FULLSCREEN or
            android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI or android.view.inputmethod.EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        maxLines=2
        filters=arrayOf(android.text.InputFilter.LengthFilter(16384))
    }
    fun bind(value:TerminalInputOwner?) {
        if(owner!==value){val previous=owner;reset();owner=value;if(previous?.hardwareOutstanding()==true)previous.unavailable()}
        val allowed=value?.accepting()==true
        val retained=allowed||value?.retained()==true
        // Same-owner freshness pauses admission, not the issued IME connection.
        if(!retained){editor?.retire();editor=null}
        if(!allowed){android.view.inputmethod.BaseInputConnection.removeComposingSpans(editableText);editableText.clear();accent=0;committed?.invalidate()}
        // Disabling a retained focused editor clears Android focus and loses its owned key releases.
        isEnabled=retained
        hint=if(allowed)"Direct terminal input · tap for keyboard" else "Direct terminal input paused · new input unavailable"
        contentDescription=if(allowed)"Direct terminal input" else "Direct terminal input paused · new input unavailable"
    }
    fun observeCursor(frame:GridBegin?) {
        val prior=cursorFrame
        if(frame?.cursor==null||!frame.cursor.visible||prior!=null&&(prior.cursor!=frame.cursor
            ||prior.columns!=frame.columns||prior.rows!=frame.rows||prior.alternateScreen!=frame.alternateScreen))committed?.invalidate()
        cursorFrame=frame
    }
    private fun active()=isEnabled&&isFocused&&isAttachedToWindow&&hasWindowFocus()&&owner?.accepting()==true
    private fun reset() {editor?.retire();editor=null;editableText.clear();pressed.clear();deadKeys.clear();accent=0;committed?.invalidate();cursorFrame=null;error=null}
    fun release() {
        val original=owner;reset();owner=null
        if(original?.hardwareOutstanding()==true)original.unavailable()
    }
    override fun onWindowFocusChanged(hasWindowFocus:Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if(!hasWindowFocus){val original=owner;reset();if(original?.hardwareOutstanding()==true)original.unavailable()}
    }
    override fun onFocusChanged(gainFocus:Boolean,direction:Int,previouslyFocusedRect:android.graphics.Rect?) {
        super.onFocusChanged(gainFocus,direction,previouslyFocusedRect)
        if(!gainFocus&&owner!=null){val original=owner;reset();if(original?.hardwareOutstanding()==true)original.unavailable()}
    }
    override fun onDetachedFromWindow(){release();super.onDetachedFromWindow()}
    override fun onCheckIsTextEditor()=active()
    override fun onTextContextMenuItem(id:Int)=active()&&super.onTextContextMenuItem(id)
    override fun onCreateInputConnection(outAttrs:android.view.inputmethod.EditorInfo):android.view.inputmethod.InputConnection? {
        if(!active())return null
        editor?.retire();editor=null
        outAttrs.inputType=inputType;outAttrs.imeOptions=imeOptions;outAttrs.initialSelStart=0;outAttrs.initialSelEnd=0
        return TerminalConnection().also {editor=it}
    }
    private fun offer(input:ShellInput,ownedRelease:Boolean=false):Boolean {
        if(!active()&&!(ownedRelease&&isFocused&&isAttachedToWindow&&hasWindowFocus()&&owner?.retained()==true))return false
        val original=owner ?: return false
        return try {
            val accepted=original.offer(input)
            // First capacity/source refusal has retained the commit in the original draft.
            // A retired connection is never called again or automatically re-enqueued.
            accepted||original.paused
        }catch(_:ProtocolFailure){original.refuse();false}
    }
    override fun dispatchKeyEvent(event:android.view.KeyEvent):Boolean {
        // Observe only a real release for this retained owner while new input is disabled.
        // The bounded writer still waits for current proof; no synthetic cleanup keys.
        if(!active())return event.action==android.view.KeyEvent.ACTION_UP&&owner?.retained()==true&&hardware(event)
        return super.dispatchKeyEvent(event)
    }
    override fun onKeyDown(keyCode:Int,event:android.view.KeyEvent)=hardware(event)
    override fun onKeyUp(keyCode:Int,event:android.view.KeyEvent)=hardware(event)
    override fun onKeyMultiple(keyCode:Int,repeatCount:Int,event:android.view.KeyEvent)=hardware(event)
    private fun physical(event:android.view.KeyEvent):Long? {
        if(event.deviceId<0||event.flags and android.view.KeyEvent.FLAG_SOFT_KEYBOARD!=0)return null
        val scan=if(event.scanCode==0)event.keyCode else event.scanCode
        if(event.deviceId>65535||scan !in 0..65535){owner?.refuse();return null}
        return event.deviceId.toLong().shl(16) or scan.toLong()
    }
    private fun hardware(event:android.view.KeyEvent):Boolean {
        val ownedRelease=event.action==android.view.KeyEvent.ACTION_UP&&isFocused&&isAttachedToWindow&&hasWindowFocus()&&owner?.retained()==true
        if(!active()&&!ownedRelease)return false
        if(android.view.KeyEvent.isModifierKey(event.keyCode))return true // Chord state comes from actual SDK meta bits, not invented modifier key codes.
        if(event.action==android.view.KeyEvent.ACTION_MULTIPLE) {
            val text=event.characters
            return if(event.keyCode==android.view.KeyEvent.KEYCODE_UNKNOWN&&text!=null) {
                val bounded=boundedTerminalCommit(text)
                if(bounded==null){owner?.limit();false}else offer(ShellInput.Text(bounded))
            }else{owner?.refuse();true}
        }
        val id=physical(event)
        if(owner?.retained()!=true)return true
        if(event.action==android.view.KeyEvent.ACTION_UP) {
            if(id==null)return true // IME/on-screen keys are press-only.
            if(deadKeys[id]==event.downTime){deadKeys.remove(id);return true}
            val held=pressed[id]?.takeIf {it.downTime==event.downTime} ?: return true
            pressed.remove(id)
            return offer(ShellInput.Keys(listOf(ShellKeyEvent(held.key,modifiers(event),ShellKeyPhase.Release,Uint(id)))),ownedRelease=true)
        }
        if(owner?.accepting()!=true)return false
        if(event.action!=android.view.KeyEvent.ACTION_DOWN)return false
        val phase=if(event.repeatCount>0)ShellKeyPhase.Repeat else ShellKeyPhase.Press
        if(phase==ShellKeyPhase.Repeat&&id!=null&&pressed[id]?.downTime!=event.downTime){owner?.refuse();return true}
        if(phase==ShellKeyPhase.Press&&id!=null)deadKeys.remove(id)
        val key=key(event,id) ?: return true
        if(id!=null){val held=pressed[id];if(phase==ShellKeyPhase.Press||held==null)pressed[id]=HeldKey(key,event.downTime)else held.key=key}
        return offer(ShellInput.Keys(listOf(ShellKeyEvent(key,modifiers(event),phase,id?.let(::Uint)))))
    }
    private fun modifiers(event:android.view.KeyEvent)=ShellKeyModifiers(event.isShiftPressed,event.isCtrlPressed,event.isAltPressed,event.isMetaPressed)
    private fun key(event:android.view.KeyEvent,id:Long?):ShellKey? {
        val named=when(event.keyCode) {
            android.view.KeyEvent.KEYCODE_ENTER,android.view.KeyEvent.KEYCODE_NUMPAD_ENTER->ShellKeyName.Enter
            android.view.KeyEvent.KEYCODE_TAB->ShellKeyName.Tab
            android.view.KeyEvent.KEYCODE_ESCAPE->ShellKeyName.Escape
            android.view.KeyEvent.KEYCODE_DEL->ShellKeyName.Backspace
            android.view.KeyEvent.KEYCODE_FORWARD_DEL->ShellKeyName.Delete
            android.view.KeyEvent.KEYCODE_INSERT->ShellKeyName.Insert
            android.view.KeyEvent.KEYCODE_DPAD_LEFT->ShellKeyName.Left
            android.view.KeyEvent.KEYCODE_DPAD_RIGHT->ShellKeyName.Right
            android.view.KeyEvent.KEYCODE_DPAD_UP->ShellKeyName.Up
            android.view.KeyEvent.KEYCODE_DPAD_DOWN->ShellKeyName.Down
            android.view.KeyEvent.KEYCODE_MOVE_HOME->ShellKeyName.Home
            android.view.KeyEvent.KEYCODE_MOVE_END->ShellKeyName.End
            android.view.KeyEvent.KEYCODE_PAGE_UP->ShellKeyName.PageUp
            android.view.KeyEvent.KEYCODE_PAGE_DOWN->ShellKeyName.PageDown
            android.view.KeyEvent.KEYCODE_F1->ShellKeyName.F1
            android.view.KeyEvent.KEYCODE_F2->ShellKeyName.F2
            android.view.KeyEvent.KEYCODE_F3->ShellKeyName.F3
            android.view.KeyEvent.KEYCODE_F4->ShellKeyName.F4
            android.view.KeyEvent.KEYCODE_F5->ShellKeyName.F5
            android.view.KeyEvent.KEYCODE_F6->ShellKeyName.F6
            android.view.KeyEvent.KEYCODE_F7->ShellKeyName.F7
            android.view.KeyEvent.KEYCODE_F8->ShellKeyName.F8
            android.view.KeyEvent.KEYCODE_F9->ShellKeyName.F9
            android.view.KeyEvent.KEYCODE_F10->ShellKeyName.F10
            android.view.KeyEvent.KEYCODE_F11->ShellKeyName.F11
            android.view.KeyEvent.KEYCODE_F12->ShellKeyName.F12
            else->null
        }
        if(named!=null){if(accent!=0){accent=0;if(named==ShellKeyName.Backspace)return null;owner?.refuse();return null};return ShellKey(named)}
        val shortcut=event.isCtrlPressed||event.isAltPressed||event.isMetaPressed
        val baseMeta=event.metaState and (android.view.KeyEvent.META_CTRL_MASK or android.view.KeyEvent.META_ALT_MASK or android.view.KeyEvent.META_META_MASK).inv()
        var code=event.getUnicodeChar(if(shortcut)baseMeta else event.metaState)
        val actual=event.unicodeChar
        if(shortcut&&actual!=0&&!Character.isISOControl(actual)&&actual!=code){owner?.refuse();return null} // SDK layout/chord ambiguity, never guess AltGr text.
        if(code and android.view.KeyCharacterMap.COMBINING_ACCENT!=0) {
            if(shortcut){owner?.refuse();return null}
            accent=code and android.view.KeyCharacterMap.COMBINING_ACCENT_MASK
            id?.let {deadKeys[it]=event.downTime};return null
        }
        if(code==0||!Character.isValidCodePoint(code)||Character.isISOControl(code)){owner?.refuse();return null}
        if(accent!=0) {
            val previous=accent;accent=0
            code=android.view.KeyCharacterMap.getDeadChar(previous,code)
            if(code==0){owner?.refuse(String(Character.toChars(previous))+String(Character.toChars(actual)));return null}
        }
        return ShellKey(ShellKeyName.Character,String(Character.toChars(code)))
    }
    private inner class TerminalConnection:android.view.inputmethod.BaseInputConnection(this@DirectTerminalInputView,true) {
        private val buffer=editableText
        private val originalOwner=owner
        private var live=true
        fun retire(){live=false;if(editor===this&&owner===originalOwner)buffer.clear()}
        override fun closeConnection(){retire();super.closeConnection()}
        private fun current()=live&&editor===this&&owner===originalOwner&&active()
        override fun getEditable():android.text.Editable?=if(current())buffer else null
        private fun clear(){if(editor!==this||owner!==originalOwner)return;android.view.inputmethod.BaseInputConnection.removeComposingSpans(buffer);buffer.clear();android.text.Selection.setSelection(buffer,0)}
        override fun setSelection(start:Int,end:Int):Boolean {
            if(!current())return false
            committed?.invalidate();return super.setSelection(start,end)
        }
        override fun setComposingText(text:CharSequence?,newCursorPosition:Int):Boolean {
            if(!current())return false
            val value=boundedTerminalCommit(text) ?: run {owner?.limit();return false}
            return super.setComposingText(value,newCursorPosition)
        }
        override fun commitText(text:CharSequence?,newCursorPosition:Int):Boolean {
            if(!current())return false
            val value=boundedTerminalCommit(text) ?: run {owner?.limit();return false}
            if(value.isEmpty()){clear();return true}
            val accepted=offer(ShellInput.Text(value));if(accepted)clear();return accepted
        }
        override fun finishComposingText():Boolean {
            if(!current()){clear();return false}
            val start=getComposingSpanStart(buffer);val end=getComposingSpanEnd(buffer)
            if(start>=0&&end>=start) {
                val value=buffer.subSequence(start,end).toString()
                if(value.isNotEmpty()&&!offer(ShellInput.Text(value)))return false
            }
            clear();return true
        }
        private fun delete(before:Int,after:Int,codePoints:Boolean):Boolean {
            if(!current())return false
            if(before<0||after<0||before>32||after>32||before+after>32){owner?.refuse();return false}
            if(buffer.isNotEmpty()){committed?.invalidate();return if(codePoints)super.deleteSurroundingTextInCodePoints(before,after)else super.deleteSurroundingText(before,after)}
            if(before+after==0)return true
            val original=owner ?: return false
            val deletion=original.committedContext.deletion(before,after,codePoints) ?: run{
                original.committedContext.invalidate();this@DirectTerminalInputView.error="IME deletion cannot be mapped safely here. Use the terminal Backspace/Delete keys.";return false
            }
            val accepted=original.offerDeletion(deletion)
            if(accepted)this@DirectTerminalInputView.error=null
            else if(original.accepting())this@DirectTerminalInputView.error="IME deletion context changed. Use the terminal Backspace/Delete keys."
            return accepted||original.paused
        }
        override fun deleteSurroundingText(beforeLength:Int,afterLength:Int)=delete(beforeLength,afterLength,false)
        override fun deleteSurroundingTextInCodePoints(beforeLength:Int,afterLength:Int)=delete(beforeLength,afterLength,true)
        override fun sendKeyEvent(event:android.view.KeyEvent):Boolean {
            if(current())return hardware(event)
            return event.action==android.view.KeyEvent.ACTION_UP&&owner===originalOwner&&originalOwner?.retained()==true&&hardware(event)
        }
        override fun performEditorAction(actionCode:Int):Boolean {
            if(!current())return false
            if(actionCode!=android.view.inputmethod.EditorInfo.IME_ACTION_NONE&&actionCode!=android.view.inputmethod.EditorInfo.IME_ACTION_UNSPECIFIED)return false
            return offer(ShellInput.Keys(listOf(ShellKeyEvent(ShellKey(ShellKeyName.Enter),ShellKeyModifiers(false,false,false,false),ShellKeyPhase.Press))))
        }
        override fun getExtractedText(request:android.view.inputmethod.ExtractedTextRequest?,flags:Int):android.view.inputmethod.ExtractedText?=null
    }
}
internal const val STOCK_CLOSE_LIMITATION="Native Close is unavailable: stock Herdr pane.close can cascade to an entire worktree group. Back or dismiss only releases this Goatr viewer; it does not close native resources."
internal const val NATIVE_CONTROL_DISCLOSURE="Native control may resize every tab in this Herdr session and let Herdr resume deferred agents. Goatr does not send terminal input to known agent panes. With the sole active frontend this can affect every workspace/tab on this server. This is observed logical non-agent exclusion, not immutable PTY identity or consume-time fencing. A complete socket frame is Dispatched, not a native per-write acknowledgement. Already queued bytes may be consumed after a later agent transition and cannot be recalled. Releasing control reapplies surviving native controllers; it does not restore an assumed saved size."

private fun shellWorkspace(inventory:HostSnapshotResult?,shell:ShellSummary):String =
    inventory?.hierarchy?.singleOrNull {it.serverId==shell.serverId}?.workspaces?.singleOrNull {it.workspaceId==shell.workspaceId}?.label?.value ?: "Workspace ${shell.workspaceId.value}"

/** Positive shell rows are a separate resource kind, never fake provider sessions. */
@Composable
internal fun ShellTerminalsScreen(host:String,store:StoreView,view:ConnectionView,model:GoatrViewModel) {
    val inventory=view.inventory[host] ?: store.inventory.firstOrNull {it.hostId==host}?.inventory
    val ready=view.inventoryReady[host]==true && host in view.hello
    val hostName=store.hosts.firstOrNull {it.hostId==host}?.name ?: "Desktop"
    val terminalAppearance=rememberTerminalAppearanceState()
    var choosingWorkspace by remember(host){mutableStateOf(false)}
    LaunchedEffect(ready,model.foreground){if(!ready||!model.foreground)choosingWorkspace=false}
    LazyColumn(Modifier.fillMaxSize().testTag("shell_terminals"),verticalArrangement=Arrangement.spacedBy(16.dp),contentPadding=PaddingValues(bottom=24.dp)) {
        item {
            GroupedSurface {
                UiText(hostName,style=MaterialTheme.typography.titleMedium)
                UiText(if(ready) "Ordinary terminals" else "Last observed terminal list",style=MaterialTheme.typography.bodySmall)
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    IconButton(onClick=model::refreshTerminals,enabled=!model.busy && model.foreground && host in view.hello,modifier=Modifier.size(48.dp).testTag("shell_inventory_refresh")) {GoatrIcon(MaterialGlyph.Refresh,"Refresh terminals")}
                    IconButton(onClick={model.requestClearShellDrafts(host)},enabled=model.foreground && model.shellInputDrafts.keys.any {it.host==host},modifier=Modifier.size(48.dp).testTag("shell_clear_local_drafts")) {GoatrIcon(MaterialGlyph.Edit,"Clear local drafts")}
                    IconButton(onClick={choosingWorkspace=true},enabled=ready&&model.foreground&&!model.busy,modifier=Modifier.size(48.dp).testTag("terminal_create_workspace")){GoatrIcon(MaterialGlyph.Add,"New terminal in an existing workspace")}
                    IconButton(onClick={model.sessions(host)},enabled=!model.busy,modifier=Modifier.size(48.dp).testTag("shell_back_sessions")) {GoatrIcon(MaterialGlyph.Chat,"Sessions")}
                }
                TerminalAppearancePicker(terminalAppearance)
            }
        }
        val rows=inventory?.shells.orEmpty()
        if(rows.isEmpty()) item {
            GroupedSurface {
                UiText(when {inventory==null->"Terminal list unavailable";!ready->"No rows in the last observed terminal list";else->"No terminal rows in this list"},style=MaterialTheme.typography.titleSmall)
                UiText(when {host in view.hello && inventory==null->"Connected. A terminal list has not been received yet.";model.connectingHosts[host]==true->"Connecting to this desktop.";!ready->"Reconnect or refresh to request its current terminal list.";else->"Refresh to request another snapshot. This list does not establish desktop shell grants."},style=MaterialTheme.typography.bodySmall)
            }
        }
        rows.groupBy {it.serverId to it.workspaceId}.forEach {(_,shells)->
            item {UiText(shellWorkspace(inventory,shells.first()),style=MaterialTheme.typography.titleSmall)}
            items(shells,key={"${it.target.shellId.value}_${it.target.generation.value}"}) {shell->
                val actionable=ready && model.foreground && shell.connectivity==SessionConnectivity.Ready && shell.capabilities.read
                GroupedSurface(Modifier.testTag("shell_row_${shell.target.shellId.value}")) {
                    UiText(shell.title.value,style=MaterialTheme.typography.titleSmall,maxLines=1,overflow=TextOverflow.Ellipsis)
                    var details by remember(host,shell.target) {mutableStateOf(false)}
                    if(!actionable) UiText(when { !ready->"Last observed terminal · current inventory is unavailable";shell.connectivity!=SessionConnectivity.Ready->"${shell.connectivity} · terminal access unavailable";!shell.capabilities.read->"Terminal read capability is unavailable";else->"Terminal access is paused while this app is inactive"},style=MaterialTheme.typography.bodySmall)
                    if(!shell.capabilities.grid)UiText("Stock terminal control unavailable · requires the local terminal-control grant and a current native layout",style=MaterialTheme.typography.bodySmall)
                    DetailDisclosure(details,{details=!details},"shell_details_${shell.target.shellId.value}")
                    if(details) {
                        UiText(shell.title.value,style=MaterialTheme.typography.bodySmall)
                        UiText("Tab ${shell.tabId.value} · pane ${shell.paneId.value}",style=MaterialTheme.typography.bodySmall)
                        UiText(STOCK_CLOSE_LIMITATION,style=MaterialTheme.typography.bodySmall)
                    }
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                        if(shell.capabilities.artifacts)IconAction(MaterialGlyph.Folder,"Files","shell_files_${shell.target.shellId.value}",enabled=ready&&model.foreground&&shell.connectivity==SessionConnectivity.Ready&&!model.busy){model.beginArtifacts(host,ArtifactSource.Shell(shell.target))}
                        IconAction(MaterialGlyph.Terminal,"Open terminal","shell_open_${shell.target.shellId.value}",enabled=actionable && !model.busy) {model.openShell(host,shell.target)}
                    }
                }
            }
        }
        items(model.actions[host].orEmpty().filter {it.metadata.method in listOf(ActionMethod.TerminalCreate,ActionMethod.ShellCreate)},key={"shell_action_${it.metadata.actionId.value}"}) {action->
            ReceiptPanel(host,action,model)
            ShellCreatedAction(host,action,inventory,ready,host in view.hello,model)
        }
    }
    model.shell?.takeIf {it.selection.host==host}?.let {snapshot->ShellViewer(snapshot,hostName,inventory,model,terminalAppearance.value,connectionLabel(snapshot.selection.host,view,model.connectingHosts[snapshot.selection.host]==true),compactConnectionLabel(snapshot.selection.host,view,model.connectingHosts[snapshot.selection.host]==true))}
    if(model.clearShellDraftHost==host) AlertDialog(
        onDismissRequest=model::cancelClearShellDrafts,title={UiText("Clear local terminal drafts?")},
        text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)){UiText(hostName);UiText("Remove all local terminal text, cursor positions, key characters and modes for this desktop, including offline and older terminal generations. Pending inputs and receipts are retained; no terminal operation is sent.")}},
        confirmButton={TextButton(onClick=model::confirmClearShellDrafts,modifier=Modifier.heightIn(min=48.dp).testTag("shell_clear_local_drafts_confirm")){UiText("Clear this desktop’s drafts")}},
        dismissButton={TextButton(onClick=model::cancelClearShellDrafts,modifier=Modifier.heightIn(min=48.dp)){UiText("Keep drafts")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
    )
    if(choosingWorkspace)AlertDialog(onDismissRequest={choosingWorkspace=false},title={UiText("Choose an original workspace")},
        text={Column(Modifier.heightIn(max=400.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)){
            val graphs=inventory?.hierarchy.orEmpty().filter {it.freshness==NativeFreshness.Current}
            if(graphs.all {it.workspaces.isEmpty()})UiText("Create a workspace in native Herdr first. Goatr does not invent another shell owner.")
            for(graph in graphs)for(workspace in graph.workspaces)TextButton(
                onClick={choosingWorkspace=false;model.beginTerminalCreation(host,graph.serverId,workspace.workspaceId)},
                enabled=ready&&model.foreground&&!model.busy,modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)){
                UiText((inventory?.servers?.singleOrNull {it.serverId==graph.serverId}?.name?.value ?: "Original server")+" · "+(workspace.label?.value ?: "Workspace ${workspace.number.value}"))
            }
        }},confirmButton={TextButton(onClick={choosingWorkspace=false},modifier=Modifier.heightIn(min=48.dp)){UiText("Cancel")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))
}

@Composable
private fun ShellCreatedAction(host:String,action:LocalAction,inventory:HostSnapshotResult?,ready:Boolean,connected:Boolean,model:GoatrViewModel) {
    val receipt=action.receipt ?: return
    val resources=(receipt.result as? ActionResultShellCreation)?.resources ?: return
    if(receipt.state!=ReceiptState.Succeeded) return
    val current=resources.createdTarget?.let {target->inventory?.shells?.singleOrNull {shell->shell.target==target && shell.serverId==resources.serverId && shell.workspaceId==resources.workspaceId && shell.tabId==resources.tabId && shell.paneId==resources.paneId && shell.terminalId==resources.terminalId && shell.connectivity==SessionConnectivity.Ready && shell.capabilities.read}}
    if(current!=null && ready) IconAction(MaterialGlyph.Terminal,"Open created terminal","shell_open_created",enabled=model.foreground && !model.busy) {model.openShell(host,current.target)}
    else if(connected) IconAction(MaterialGlyph.Refresh,"Refresh terminals to find it","shell_refresh_created",enabled=model.foreground && !model.busy,onClick=model::refreshTerminals)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShellViewer(snapshot:ShellSnapshot,hostName:String,inventory:HostSnapshotResult?,model:GoatrViewModel,terminalAppearance:TerminalAppearance,connectionStatus:String,compactConnectionStatus:String) {
    val keyboard=LocalSoftwareKeyboardController.current
    LaunchedEffect(snapshot.selection,snapshot.ticket) {keyboard?.hide()}
    val scope=ShellControlScope(snapshot.selection,snapshot.source,snapshot.ticket)
    var beginConfirm by remember(scope){mutableStateOf(false)}
    var resizeConfirm by remember(scope){mutableStateOf(false)}
    var columns by remember(scope){mutableStateOf("")}
    var rows by remember(scope){mutableStateOf("")}
    Dialog(onDismissRequest=model::closeShell,properties=DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
        Surface(Modifier.fillMaxSize().safeDrawingPadding()) {
            Scaffold(
                contentWindowInsets=WindowInsets(0,0,0,0),
                topBar={GoatrTitleBar(snapshot.summary.title.value,connectionStatus,snapshot.loading,compactStatus=compactConnectionStatus,navigationIcon={
                    IconButton(onClick=model::closeShell,modifier=Modifier.size(48.dp).testTag("shell_viewer_back")){GoatrIcon(MaterialGlyph.Back,"Back to terminals")}
                })},
                bottomBar={if(snapshot.summary.capabilities.input) ShellInputPanel(snapshot,model)},
                floatingActionButton={if(snapshot.summary.capabilities.create) PrimaryActionFab(MaterialGlyph.Add,"New terminal here","terminal_create_here",
                    enabled=model.foreground&&!model.busy&&model.shellControlAllowed(scope),
                    onClick={model.beginTerminalCreation(snapshot.selection.host,snapshot.summary.serverId,snapshot.summary.workspaceId,ArtifactSource.Shell(snapshot.selection.target))})},
            ) {padding->
                LazyColumn(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding).padding(horizontal=16.dp),verticalArrangement=Arrangement.spacedBy(8.dp),contentPadding=PaddingValues(bottom=112.dp)) {
                    item {Column {UiText(hostName,style=MaterialTheme.typography.bodySmall);UiText(shellWorkspace(inventory,snapshot.summary),style=MaterialTheme.typography.bodySmall)}}
                    item {UiText(STOCK_CLOSE_LIMITATION,style=MaterialTheme.typography.bodySmall)}
                    item {
                        if(!snapshot.controlBegun)Button(onClick={beginConfirm=true},enabled=model.shellControlAllowed(scope),modifier=Modifier.heightIn(min=48.dp).testTag("shell_begin_control")){UiText("Begin control")}
                        else Row(horizontalArrangement=Arrangement.spacedBy(8.dp)){
                            TextButton(onClick={model.releaseShellControl(scope)},modifier=Modifier.heightIn(min=48.dp)){UiText("Release control")}
                            OutlinedButton(onClick={resizeConfirm=true},enabled=!model.busy&&model.shellControlAllowed(scope)&&snapshot.grid!=null&&snapshot.grid.begin.popupBlocked==false,modifier=Modifier.heightIn(min=48.dp).testTag("shell_resize")){UiText("Resize…")}
                        }
                    }
                    model.actions[snapshot.selection.host].orEmpty().lastOrNull {it.metadata.method==ActionMethod.ShellResize&&it.metadata.shellTarget==snapshot.selection.target}?.let {action->item{ReceiptPanel(snapshot.selection.host,action,model)}}
                    item {
                        UiText("Live grid · pan and pinch locally",style=MaterialTheme.typography.labelMedium)
                        Box(Modifier.fillMaxWidth().height(360.dp)) {
                            when {
                                !snapshot.controlBegun->UiText("Begin control explicitly to open the native frontend. Merely opening this panel sends nothing.")
                                snapshot.error->UiText("Control lease is unavailable. Release it before a fresh explicit Begin; no native action is replayed.",color=MaterialTheme.colorScheme.error)
                                snapshot.grid==null->UiText("Waiting for current source and a complete native grid · no input is enabled",style=MaterialTheme.typography.bodySmall)
                                else->TerminalGridCanvas(snapshot.grid,terminalAppearance,Modifier.fillMaxSize())
                            }
                        }
                    }
                }
            }
        }
    }
    if(beginConfirm)AlertDialog(onDismissRequest={beginConfirm=false},title={UiText("Begin native control?")},
        text={UiText(NATIVE_CONTROL_DISCLOSURE)},
        confirmButton={TextButton(onClick={beginConfirm=false;model.beginShellControl(scope)},enabled=model.shellControlAllowed(scope),modifier=Modifier.heightIn(min=48.dp)){UiText("Begin control")}},
        dismissButton={TextButton(onClick={beginConfirm=false},modifier=Modifier.heightIn(min=48.dp)){UiText("Cancel")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))
    if(resizeConfirm)AlertDialog(onDismissRequest={resizeConfirm=false},title={UiText("Resize native frontend?")},
        text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)){
            UiText(NATIVE_CONTROL_DISCLOSURE)
            UiText("These are frontend surface dimensions, not this pane’s crop or a phone fit. This is a new action, never a replay of a prior uncertain resize.")
            OutlinedTextField(columns,{columns=it.filter(Char::isDigit)},label={UiText("Columns 1–512")},singleLine=true)
            OutlinedTextField(rows,{rows=it.filter(Char::isDigit)},label={UiText("Rows 1–256 · at most 32768 cells")},singleLine=true)
        }},
        confirmButton={val c=columns.toLongOrNull();val r=rows.toLongOrNull();TextButton(onClick={if(c!=null&&r!=null){resizeConfirm=false;model.resizeShell(scope,c,r)}},enabled=!model.busy&&model.shellControlAllowed(scope)&&c!=null&&r!=null&&c in 1..512&&r in 1..256&&c*r<=32768,modifier=Modifier.heightIn(min=48.dp)){UiText("Resize native frontend")}},
        dismissButton={TextButton(onClick={resizeConfirm=false},modifier=Modifier.heightIn(min=48.dp)){UiText("Cancel")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))
}

/** Direct commits and staged drafts share one original-lease writer; insertion never submits. */
@Composable
private fun ShellInputPanel(snapshot:ShellSnapshot,model:GoatrViewModel) {
    val scope=ShellControlScope(snapshot.selection,snapshot.source,snapshot.ticket)
    val draft=model.shellDraft(snapshot.selection)
    val gate=model.shellInputGate?.takeIf {it.scope==scope}
    val owner=model.terminalOwner(scope)
    val enabled=model.shellInputAllowed(scope)
    LaunchedEffect(scope,snapshot.controlBegun,snapshot.grid,gate?.loaded,gate?.pending,gate?.observation){model.prepareTerminalInput(scope)}
    Surface(tonalElevation=3.dp) {
        Column(Modifier.fillMaxWidth().imePadding().padding(horizontal=16.dp,vertical=8.dp).heightIn(max=360.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            gate?.prior?.let {action->
                val receipt=action.receipt;val observation=gate.observation
                val label=when {
                    observation==null->"Current status needs checking · last receipt retained"
                    observation.unknown->"Current host status unknown · never repeat the old action"
                    action.authoritative&&receipt?.shellDispatched()==true->"Native socket frame dispatched · not PTY acknowledgement"
                    action.authoritative&&receipt?.state==ReceiptState.Succeeded->"Historical bound-input acceptance"
                    action.authoritative&&receipt?.state==ReceiptState.Failed->"Input refused before effect"
                    receipt?.state in listOf(ReceiptState.Reserved,ReceiptState.Dispatching)->"Input pending"
                    else->"Input outcome uncertain · inspect before a fresh gesture"
                }
                UiText(label,style=MaterialTheme.typography.labelMedium,modifier=Modifier.testTag("shell_input_outcome"))
                LaunchedEffect(scope,action,observation){withFrameNanos {};model.shellInputDisplayed(scope,action)}
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick={model.inspect(snapshot.selection.host,action.metadata.actionId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Check receipt")}
                    if(observation?.unknown==true||receipt?.state==ReceiptState.Uncertain)TextButton(onClick={model.requestSeparateShellInput(scope,action.metadata.actionId)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Separate action…")}
                }
                (receipt?.result as? ActionResultShellDispatch)?.progress?.let {
                    UiText("Socket frame ${it.socketBytesWritten ?: "unknown"} / ${it.frameBytes ?: "unknown"} bytes · complete ${it.frameComplete ?: "unknown"}",style=MaterialTheme.typography.labelSmall)
                }
                (receipt?.result as? ActionResultShellInput)?.progress?.let {
                    UiText("Historical facts only: ${it.writtenBytes ?: "unknown"} / ${it.encodedBytes ?: "unknown"} bytes · flush ${it.flushCompleted ?: "unknown"}",style=MaterialTheme.typography.labelSmall)
                }
            }
            if(model.terminalInputPause!=null)UiText("Input is paused. Known-unsent text stays in this original target’s draft; possibly entered text is retained separately below. Unsent key gestures are discarded, never automatically replayed. Begin fresh control and inspect uncertain receipts.",style=MaterialTheme.typography.bodySmall)
            if(!snapshot.controlBegun)UiText("Drafts stay local. Begin control explicitly before any direct input or Send.",style=MaterialTheme.typography.bodySmall)
            if(snapshot.grid?.begin?.popupBlocked==true)UiText("Native popup needs desktop attention. No terminal input is forwarded.",style=MaterialTheme.typography.bodySmall)
            if(owner!=null&&!enabled&&model.terminalInputPause==null&&snapshot.grid?.begin?.popupBlocked!=true)
                UiText("Input is disabled until this same original source/view is current. Already queued input stays local and bounded; no new input is admitted.",style=MaterialTheme.typography.bodySmall)
            FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                FilterChip(selected=draft.direct,onClick={model.shellInputMode(scope,true)},label={UiText("Direct")})
                FilterChip(selected=!draft.direct,onClick={model.shellInputMode(scope,false)},label={UiText("Draft")})
                TextButton(onClick={model.clearShellDraft(scope)},modifier=Modifier.heightIn(min=48.dp)){UiText("Clear draft")}
            }
            if(draft.direct) {
                UiText("Committed IME and hardware keys go directly to this original terminal. Composition sends nothing.",style=MaterialTheme.typography.labelSmall)
                AndroidView(factory={context->DirectTerminalInputView(context)},update={it.bind(owner);it.observeCursor(snapshot.grid?.begin)},onReset=null,onRelease={it.release()},
                    modifier=Modifier.fillMaxWidth().heightIn(min=56.dp).testTag("terminal_direct_input"))
            }else{
                UiText("Staged multiline text · Send adds no Enter · at most16KiB per explicit Send",style=MaterialTheme.typography.labelSmall)
                Row(verticalAlignment=Alignment.Bottom,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(value=draft.value,onValueChange={model.editShellDraft(scope,it)},modifier=Modifier.weight(1f).heightIn(min=56.dp,max=144.dp).testTag("shell_input_text"),
                        placeholder={UiText("Unsent terminal draft")},maxLines=5)
                    IconButton(onClick={model.sendShellText(scope)},enabled=model.shellDraftSendAllowed(scope),modifier=Modifier.size(48.dp).testTag("shell_input_send")){GoatrIcon(MaterialGlyph.Send,"Send exact terminal draft")}
                }
            }
            TerminalKeyBar(draft.modifiers,enabled,{modifier,lock->model.shellInputModifier(scope,modifier,lock)},
                {modifier,at,window->model.shellInputModifierTap(scope,modifier,at,window)},{key->model.sendShellKey(scope,key)})
            Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(value=draft.keyCharacter,onValueChange={model.shellKeyCharacter(scope,it)},label={UiText("One character chord")},singleLine=true,modifier=Modifier.weight(1f))
                IconButton(onClick={model.sendShellCharacter(scope)},enabled=enabled&&draft.keyCharacter.isNotEmpty(),modifier=Modifier.size(48.dp)){GoatrIcon(MaterialGlyph.Send,"Send character key with modifiers")}
            }
            UiText("Modifiers apply to explicit key/chord buttons; IME text is TextCommit. Input uses an ordered128-event owner, with48KiB ordinary draft/queue plus16KiB reserved recovery inside64KiB.",style=MaterialTheme.typography.labelSmall)
            owner?.takeIf {it.pendingEvents>0}?.let {UiText("${it.pendingEvents} pending events · no automatic reconnect requeue",style=MaterialTheme.typography.labelSmall)}
            model.attemptedShellText(snapshot.selection)?.let {(action,text)->
                OutlinedTextField(value=text,onValueChange={},readOnly=true,label={UiText("Attempted text · may already have entered")},maxLines=3,modifier=Modifier.fillMaxWidth().heightIn(max=112.dp))
                TextButton(onClick={model.forgetAttemptedShellText(scope,action)},enabled=(owner?.pendingEvents ?: 0)==0&&gate?.prior?.receipt?.state !in listOf(ReceiptState.Reserved,ReceiptState.Dispatching),modifier=Modifier.heightIn(min=48.dp)){UiText("Forget attempted text locally · retain receipt")}
            }
            if(gate?.loaded!=true)UiText("Checking original target and retained receipts…",style=MaterialTheme.typography.labelSmall)
        }
    }
    model.separateShellInput?.takeIf {it.first==scope}?.let {(_,action)->AlertDialog(
        onDismissRequest=model::cancelSeparateShellInput,title={UiText("Allow a fresh input action?")},
        text={Column(verticalArrangement=Arrangement.spacedBy(8.dp)){UiText(snapshot.summary.title.value);UiText("Prior action ${action.value}");UiText("Prior input may already have entered. This allows fresh gestures only, never replay or automatic restoration of attempted text.")}},
        confirmButton={TextButton(onClick=model::confirmSeparateShellInput,enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){UiText("Allow separate action")}},
        dismissButton={TextButton(onClick=model::cancelSeparateShellInput,modifier=Modifier.heightIn(min=48.dp)){UiText("Keep prior action")}},properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))}
}
