// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test

class TerminalInputOwnerTest {
    private enum class Proof {Current,Stale,Retired}
    private val modifiers=ShellKeyModifiers(false,false,false,false)
    private fun key(name:ShellKeyName,phase:ShellKeyPhase=ShellKeyPhase.Press,id:Long?=null,value:String?=null,ctrl:Boolean=false)=
        ShellKeyEvent(ShellKey(name,value),modifiers.copy(ctrl=ctrl),phase,id?.let(::Uint))
    private fun receipt(state:ReceiptState=ReceiptState.Dispatched,progress:ShellDispatchProgress=ShellDispatchProgress(19,19,true))=Receipt(
        actionId=Uuid.random(),method=ActionMethod.ShellInput,
        bootId=Uuid.random(),createdAt=Uint(1),updatedAt=Uint(1),state=state,
        shellTarget=ShellTarget(Uuid.random(),Uuid.random()),
        result=ActionResultShellDispatch(progress))

    @Test fun healthyRapidCommitsAndKeysHaveOneOrderedWriter()=runBlocking {
        val written=mutableListOf<ShellInput>();var completed=0
        val owner=TerminalInputOwner(this,{true},{true},{},{0},{input,_->written.add(input);yield();receipt()},
            {_,_->completed++},{fail("Healthy input was not retained as unsent")},{})
        try {
            val inputs=listOf(ShellInput.Text("first"),ShellInput.Keys(listOf(key(ShellKeyName.Left))),ShellInput.Text("😀界"))
            inputs.forEach {assertTrue(owner.offer(it))}
            withTimeout(1000){while(completed!=inputs.size)yield()}
            assertEquals(inputs,written);assertEquals(0,owner.pendingEvents);assertEquals(0,owner.pendingTextBytes)
            assertFalse(owner.paused)
        }finally {owner.retire()}
    }

    @Test fun firstValidRejectedCommitUsesOnlyTheReservedPartOfTheSame64KiBDraft()=runBlocking {
        for(initialDraft in listOf(0,16384,49152)) {
            var writes=0;val recovered=mutableListOf<TerminalPendingText>();var reason:TerminalInputPause?=null
            val owner=TerminalInputOwner(this,{true},{true},{},{initialDraft},{_,_->writes++;receipt()},{_,_->},
                {recovered.addAll(it)},{reason=it})
            try {
                val accepted=(49152-initialDraft)/16384
                repeat(accepted){index->assertTrue(owner.offer(ShellInput.Text(('a'.code+index).toChar().toString().repeat(16384))))}
                assertEquals(accepted,owner.pendingEvents)
                val rejected="z".repeat(16384)
                assertFalse(owner.offer(ShellInput.Text(rejected)))
                assertEquals(TerminalInputPause.Capacity,reason);assertTrue(owner.paused)
                assertEquals(initialDraft+(accepted+1)*16384,initialDraft+recovered.sumOf {utf8Size(it.text)})
                assertTrue(initialDraft+recovered.sumOf {utf8Size(it.text)}<=65536)
                assertEquals(rejected,recovered.last().text)
                val before=recovered.toList();assertFalse(owner.offer(ShellInput.Text("later")));assertEquals(before,recovered)
                assertEquals(0,writes);assertEquals(0,owner.pendingEvents);assertEquals(0,owner.pendingTextBytes)
            }finally {owner.retire()}
        }
    }

    @Test fun eventCapacityIncludesQueuedEventsAndRetainsTheFirstRejectedTextCommit()=runBlocking {
        var writes=0;val recovered=mutableListOf<TerminalPendingText>()
        val owner=TerminalInputOwner(this,{true},{true},{},{0},{_,_->writes++;receipt()},{_,_->},{recovered.addAll(it)},{})
        try {
            repeat(128){assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left)))));assertTrue(owner.pendingEvents<=128)}
            assertEquals(128,owner.pendingEvents)
            assertFalse(owner.offer(ShellInput.Text("recover me")))
            assertEquals(listOf("recover me"),recovered.map {it.text});assertEquals(0,writes);assertTrue(owner.paused)
        }finally {owner.retire()}
    }

    @Test fun definitiveRefusalRecoversCurrentTextBeforeQueuedTextButUncertaintyNeverReplays()=runBlocking {
        for(state in listOf(ReceiptState.Failed,ReceiptState.Uncertain)) {
            val entered=CompletableDeferred<Unit>();val result=CompletableDeferred<Receipt>();val stopped=CompletableDeferred<TerminalInputPause>()
            val written=mutableListOf<ShellInput>();val recovered=mutableListOf<TerminalPendingText>();var completed=0;var current=true
            val owner=TerminalInputOwner(this,{current},{true},{},{0},{input,_->written.add(input);entered.complete(Unit);result.await()},
                {_,_->completed++},{recovered.addAll(it)},{stopped.complete(it)})
            try {
                val first=TerminalPendingText("current 😀界\n",7);val later=TerminalPendingText("later\ttext",8)
                assertTrue(owner.offer(ShellInput.Text(first.text),first.draftRevision))
                withTimeout(1000){entered.await()}
                assertEquals(1,owner.pendingEvents)
                assertTrue(owner.offer(ShellInput.Text(later.text),later.draftRevision))
                result.complete(receipt(state,if(state==ReceiptState.Failed)ShellDispatchProgress()else ShellDispatchProgress(19,7,false)))
                assertEquals(if(state==ReceiptState.Failed)TerminalInputPause.Refused else TerminalInputPause.Uncertain,withTimeout(1000){stopped.await()})
                assertEquals(if(state==ReceiptState.Failed)listOf(first,later)else listOf(later),recovered);assertEquals(0,completed)
                assertEquals(0,owner.pendingEvents);assertEquals(0,owner.pendingTextBytes)
                current=false;current=true
                assertFalse(owner.offer(ShellInput.Text("not a new connection queue")))
                assertEquals(listOf(ShellInput.Text(first.text)),written)
            }finally {result.complete(receipt(ReceiptState.Uncertain));owner.retire()}
        }
    }

    @Test fun pauseBeforeSettlementRecoversFailedHeadInOrderButNeverUncertainOrDiscardedText()=runBlocking {
        val texts=listOf("a","b","c","z").mapIndexed {index,text->TerminalPendingText(text.repeat(16384),index.toLong())}
        for(reason in listOf(TerminalInputPause.Capacity,TerminalInputPause.SourceLost))
        for(state in listOf(ReceiptState.Failed,ReceiptState.Uncertain))
        for(discard in listOf(false,true)) {
            val entered=CompletableDeferred<Unit>();val result=CompletableDeferred<Receipt>();val stopped=CompletableDeferred<TerminalInputPause>()
            val written=mutableListOf<ShellInput>();val recovered=mutableListOf<TerminalPendingText>();var recoveries=0;var completed=0
            val owner=TerminalInputOwner(this,{true},{true},{},{0},{input,_->written.add(input);entered.complete(Unit);result.await()},
                {_,_->completed++},{recoveries++;recovered.addAll(it)},{stopped.complete(it)})
            try {
                assertTrue(owner.offer(ShellInput.Text(texts[0].text),texts[0].draftRevision))
                withTimeout(1000){entered.await()}
                for(text in texts.subList(1,3))assertTrue(owner.offer(ShellInput.Text(text.text),text.draftRevision))
                if(reason==TerminalInputPause.Capacity)assertFalse(owner.offer(ShellInput.Text(texts[3].text),texts[3].draftRevision))
                else owner.unavailable()
                assertEquals(reason,withTimeout(1000){stopped.await()})
                assertTrue(owner.paused);assertTrue(recovered.isEmpty());assertEquals(0,recoveries)
                assertEquals(1,owner.pendingEvents);assertEquals(16384,owner.pendingTextBytes)
                if(discard){owner.retire(discard=true);owner.retire()}
                result.complete(receipt(state,if(state==ReceiptState.Failed)ShellDispatchProgress()else ShellDispatchProgress(19,7,false)))
                withTimeout(1000){while(owner.pendingEvents!=0)yield()}
                val expected=if(discard)emptyList()else texts.subList(if(state==ReceiptState.Failed)0 else 1,if(reason==TerminalInputPause.Capacity)4 else 3)
                assertEquals(expected,recovered);assertEquals(if(discard)0 else 1,recoveries)
                assertEquals(0,owner.pendingTextBytes);assertEquals(0,completed)
                assertFalse(owner.offer(ShellInput.Text("never replay or resume")))
                assertEquals(listOf(ShellInput.Text(texts[0].text)),written)
            }finally {result.complete(receipt(ReceiptState.Uncertain));owner.retire()}
        }
    }

    @Test fun lossBeforeTheWriterRetainsOnlyKnownTextAndNeverRevives()=runBlocking {
        var current=true;var writes=0;val recovered=mutableListOf<TerminalPendingText>();val stopped=CompletableDeferred<TerminalInputPause>()
        val owner=TerminalInputOwner(this,{current},{true},{},{0},{_,_->writes++;receipt()},{_,_->},{recovered.addAll(it)},{stopped.complete(it)})
        try {
            assertTrue(owner.offer(ShellInput.Keys(listOf(
                key(ShellKeyName.Character,value="A"),key(ShellKeyName.Character,ShellKeyPhase.Repeat,value="界"),
                key(ShellKeyName.Character,ShellKeyPhase.Release,value="x"),key(ShellKeyName.Character,value="c",ctrl=true),key(ShellKeyName.Enter)))))
            current=false
            assertEquals(TerminalInputPause.SourceLost,withTimeout(1000){stopped.await()})
            assertEquals(listOf("A界"),recovered.map {it.text});assertEquals(0,writes)
            current=true;assertFalse(owner.offer(ShellInput.Text("later")))
        }finally {owner.retire()}
    }

    @Test fun realHeldKeysRemainOwnedUntilTheirActualReleaseIsDispatched()=runBlocking {
        val written=mutableListOf<ShellKeyEvent>();var completed=0
        val owner=TerminalInputOwner(this,{true},{true},{},{0},{input,_->written.addAll((input as ShellInput.Keys).events);receipt()},
            {_,_->completed++},{},{})
        try {
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,id=42)))))
            withTimeout(1000){while(completed!=1)yield()};assertTrue(owner.hardwareOutstanding())
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Repeat,42)))))
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,42)))))
            withTimeout(1000){while(completed!=3)yield()}
            assertEquals(listOf(ShellKeyPhase.Press,ShellKeyPhase.Repeat,ShellKeyPhase.Release),written.map {it.phase})
            assertEquals(listOf(42L,42L,42L),written.map {it.physicalKeyId?.value});assertFalse(owner.hardwareOutstanding())
        }finally {owner.retire()}
    }

    @Test fun sameOriginalStalenessDisablesAdmissionAndWaitsButRealLossNeverRevives()=runBlocking {
        val proof=MutableStateFlow(Proof.Current);val waiting=CompletableDeferred<Unit>()
        val written=mutableListOf<String>();val recovered=mutableListOf<TerminalPendingText>();var completed=0
        val owner=TerminalInputOwner(this,{proof.value!=Proof.Retired},{proof.value==Proof.Current},
            {waiting.complete(Unit);proof.first {it!=Proof.Stale}},{0},
            {input,_->assertEquals(Proof.Current,proof.value);written.add((input as ShellInput.Text).text);receipt()},
            {_,_->completed++},{recovered.addAll(it)},{})
        try {
            assertTrue(owner.offer(ShellInput.Text("a")));assertTrue(owner.offer(ShellInput.Text("b")))
            proof.value=Proof.Stale;withTimeout(1000){waiting.await()}
            assertTrue(owner.retained());assertFalse(owner.accepting());assertFalse(owner.paused)
            assertFalse(owner.offer(ShellInput.Text("not admitted while dirty")));assertTrue(written.isEmpty());assertTrue(recovered.isEmpty())
            assertEquals(2,owner.pendingEvents)
            proof.value=Proof.Current;withTimeout(1000){while(completed!=2)yield()}
            assertEquals(listOf("a","b"),written);assertTrue(owner.accepting())
            assertTrue(owner.offer(ShellInput.Text("c")));proof.value=Proof.Retired
            withTimeout(1000){while(!owner.paused)yield()}
            assertEquals(listOf("c"),recovered.map {it.text});assertEquals(listOf("a","b"),written)
            proof.value=Proof.Current;assertFalse(owner.offer(ShellInput.Text("never a new generation queue")))
        }finally {owner.retire()}
    }

    @Test fun capacityDuringAReadinessWaitRecoversTheWaitingHeadBeforeLaterTextAndNeverExceeds64KiB()=runBlocking {
        val proof=MutableStateFlow(Proof.Current);val waiting=CompletableDeferred<Unit>()
        val recovered=mutableListOf<TerminalPendingText>();var writes=0
        val owner=TerminalInputOwner(this,{true},{proof.value==Proof.Current},
            {waiting.complete(Unit);proof.first {it==Proof.Current}},{0},{_,_->writes++;receipt()},{_,_->},{recovered.addAll(it)},{})
        try {
            val values=listOf("a".repeat(16384),"b".repeat(16384),"c".repeat(16384),"z".repeat(16384))
            values.take(3).forEach {assertTrue(owner.offer(ShellInput.Text(it)))}
            proof.value=Proof.Stale;withTimeout(1000){waiting.await()};assertEquals(3,owner.pendingEvents)
            proof.value=Proof.Current
            assertFalse(owner.offer(ShellInput.Text(values.last())))
            assertEquals(values,recovered.map {it.text});assertEquals(65536,recovered.sumOf {utf8Size(it.text)})
            assertEquals(0,owner.pendingEvents);assertEquals(0,writes)
        }finally {owner.retire()}
    }

    @Test fun onlyAnAlreadyOwnedHardwareReleaseMayQueueLocallyWhileDirty()=runBlocking {
        val proof=MutableStateFlow(Proof.Current);val waiting=CompletableDeferred<Unit>()
        val written=mutableListOf<ShellKeyEvent>();var completed=0
        val owner=TerminalInputOwner(this,{true},{proof.value==Proof.Current},
            {waiting.complete(Unit);proof.first {it==Proof.Current}},{0},
            {input,_->assertEquals(Proof.Current,proof.value);written.addAll((input as ShellInput.Keys).events);receipt()},
            {_,_->completed++},{},{})
        try {
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,id=42)))))
            withTimeout(1000){while(completed!=1)yield()};assertTrue(owner.hardwareOutstanding())
            proof.value=Proof.Stale;assertFalse(owner.accepting())
            assertFalse(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,99)))))
            assertFalse(owner.offer(ShellInput.Text("new text")))
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,42)))))
            withTimeout(1000){waiting.await()};assertEquals(1,written.size)
            proof.value=Proof.Current;withTimeout(1000){while(completed!=2)yield()}
            assertEquals(listOf(ShellKeyPhase.Press,ShellKeyPhase.Release),written.map {it.phase});assertFalse(owner.hardwareOutstanding())
        }finally {owner.retire()}
    }

    @Test fun olderReleaseCompletionCannotEraseAnAlreadyOfferedNewGestureOnTheSamePhysicalKey()=runBlocking {
        val proof=MutableStateFlow(Proof.Current);val waiting=CompletableDeferred<Unit>();var completed=0
        val written=mutableListOf<ShellKeyEvent>()
        val owner=TerminalInputOwner(this,{true},{proof.value==Proof.Current},
            {waiting.complete(Unit);proof.first {it==Proof.Current}},{0},
            {input,_->written.addAll((input as ShellInput.Keys).events);receipt()},{_,_->completed++},{},{})
        try {
            for(phase in listOf(ShellKeyPhase.Press,ShellKeyPhase.Release,ShellKeyPhase.Press))
                assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,phase,42)))))
            withTimeout(1000){while(completed!=3)yield()}
            assertTrue(owner.hardwareOutstanding())
            proof.value=Proof.Stale
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,42)))))
            withTimeout(1000){waiting.await()};assertEquals(3,written.size)
            proof.value=Proof.Current;withTimeout(1000){while(completed!=4)yield()}
            assertFalse(owner.hardwareOutstanding())
            assertEquals(listOf(ShellKeyPhase.Press,ShellKeyPhase.Release,ShellKeyPhase.Press,ShellKeyPhase.Release),written.map {it.phase})
            assertTrue(written.all {it.physicalKeyId?.value==42L})
        }finally {owner.retire()}
    }

    @Test fun utf16DeletionMapsOnlyKnownWholeCodePointsAndCodePointApiKeepsItsOwnUnits() {
        for(text in listOf("x😀","x𐐀")) {
            val context=TerminalCommittedContext();context.commit(text)
            assertNull(context.deletion(1,0,false))
            val utf16=checkNotNull(context.deletion(2,0,false));assertEquals(1,utf16.before);assertEquals(0,utf16.after)
            val points=checkNotNull(context.deletion(2,0,true));assertEquals(2,points.before)
            context.deleted(utf16)
            val ascii=checkNotNull(context.deletion(1,0,false));assertEquals(1,ascii.before);context.deleted(ascii)
            assertNull(context.deletion(1,0,false))
        }
    }

    @Test fun deletionContextIsBoundedAndUnknownPrefixForwardCursorOrRetirementNeverGuessesUtf16() {
        val context=TerminalCommittedContext()
        assertNull(context.deletion(1,0,false))
        context.commit("😀".repeat(40))
        assertEquals(16,checkNotNull(context.deletion(32,0,false)).before)
        assertNull(context.deletion(33,0,false));assertNull(context.deletion(0,1,false))
        val points=checkNotNull(context.deletion(32,0,true));assertEquals(32,points.before);context.deleted(points)
        assertNull(context.deletion(1,0,false))
        context.commit("a".repeat(63));context.commit("𐐀")
        val tail=checkNotNull(context.deletion(2,0,false));assertEquals(1,tail.before);context.deleted(tail)
        assertEquals(32,checkNotNull(context.deletion(32,0,false)).before)
        context.invalidate();assertNull(context.deletion(1,0,false))
        assertEquals(2,checkNotNull(context.deletion(2,0,true)).before)
        context.commit("known");context.commit("\nunknown cursor");assertNull(context.deletion(1,0,false))
    }

    @Test fun keyBarAdmissionInvalidatesTheSameOwnerContextBeforeAnyCursorFrameAndMappedImeDeletionUpdatesAtomically()=runBlocking {
        val written=mutableListOf<ShellInput>();var completed=0
        val owner=TerminalInputOwner(this,{true},{true},{},{0},{input,_->written.add(input);receipt()},{_,_->completed++},{},{})
        try {
            assertTrue(owner.offer(ShellInput.Text("😀xy")))
            val staleIme=checkNotNull(owner.committedContext.deletion(2,0,false));assertEquals(2,staleIme.before)
            // This is the key-bar/character-chord route used directly by the ViewModel,
            // not DirectTerminalInputView.offer and not a callback that echoes invalidation.
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Backspace)))))
            assertNull(owner.committedContext.deletion(2,0,false))
            assertFalse(owner.offerDeletion(staleIme))
            withTimeout(1000){while(completed!=2)yield()}
            assertEquals(2,written.size)
            assertEquals(1,(written[1] as ShellInput.Keys).events.size)
            assertTrue(owner.offer(ShellInput.Text("😀xy")))
            val removeAscii=checkNotNull(owner.committedContext.deletion(2,0,false))
            assertTrue(owner.offerDeletion(removeAscii))
            val removeSupplementary=checkNotNull(owner.committedContext.deletion(2,0,false));assertEquals(1,removeSupplementary.before)
            assertTrue(owner.offerDeletion(removeSupplementary))
            assertNull(owner.committedContext.deletion(1,0,false))
            val other=TerminalCommittedContext().also {it.commit("z")}
            assertFalse(owner.offerDeletion(checkNotNull(other.deletion(1,0,false))))
            withTimeout(1000){while(completed!=5)yield()}
            assertEquals(2,(written[3] as ShellInput.Keys).events.size)
            assertEquals(1,(written[4] as ShellInput.Keys).events.size)
            assertTrue((written[3] as ShellInput.Keys).events.all {it.key==ShellKey(ShellKeyName.Backspace)})
            assertEquals(0,owner.pendingEvents);assertFalse(owner.paused)
        }finally {owner.retire()}
    }

    @Test fun oversizedSdkTextIsRefusedBeforeCopyingAndOnlyACompleteFrameSettlesInput() {
        val oversized=object:CharSequence {
            override val length=16385
            override fun get(index:Int):Char=throw AssertionError("Oversized SDK text was read")
            override fun subSequence(startIndex:Int,endIndex:Int):CharSequence=throw AssertionError("Oversized SDK text was copied")
            override fun toString():String=throw AssertionError("Oversized SDK text was copied")
        }
        assertNull(boundedTerminalCommit(oversized))
        val boundary="😀".repeat(4096);assertSame(boundary,boundedTerminalCommit(boundary))
        assertNull(boundedTerminalCommit("😀".repeat(4097)));assertNull(boundedTerminalCommit("\uD800"))
        val complete=receipt();complete.validate();assertTrue(complete.shellDispatched())
        assertFalse(complete.copy(state=ReceiptState.Uncertain).shellDispatched())
        assertFalse(complete.copy(state=ReceiptState.Succeeded).shellDispatched())
        assertFalse(receipt(progress=ShellDispatchProgress(19,18,false)).shellDispatched())
        assertTrue(ShellDispatchProgress(19,19,true).retains(ShellDispatchProgress(19,7,false)))
        assertFalse(ShellDispatchProgress(19,6,false).retains(ShellDispatchProgress(19,7,false)))
    }
    @Test fun historyBarrierIsReversibleRecoversOnlyUnsentTextAndDropsOfferedKeys()=runBlocking {
        val writes=mutableListOf<ShellInput>();val recovered=mutableListOf<TerminalPendingText>();var completed=0;var stops=0
        val owner=TerminalInputOwner(this,{true},{true},{},{0},{input,_->writes.add(input);receipt()},{_,_->completed++},{recovered.addAll(it)},{stops++})
        try {
            assertTrue(owner.offer(ShellInput.Text("known unsent"),19))
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Character,id=71,value="x")))))
            owner.blockForHistory(true);owner.historyBarrier()
            assertEquals(listOf(TerminalPendingText("known unsent",19)),recovered);assertTrue(writes.isEmpty())
            assertTrue(owner.retained());assertFalse(owner.accepting());assertFalse(owner.paused);assertEquals(0,stops)
            assertFalse(owner.offer(ShellInput.Text("new text")));assertFalse(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,71)))))
            owner.blockForHistory(false);assertTrue(owner.offer(ShellInput.Text("fresh gesture")))
            withTimeout(1000){while(completed!=1)yield()}
            assertEquals(listOf(ShellInput.Text("fresh gesture")),writes);assertEquals(0,stops)
        }finally{owner.retire()}
    }
    @Test fun historyWaitsForInflightReceiptThenReleasesOnlyConfirmedPhysicalHoldsOnce()=runBlocking {
        val pressEntered=CompletableDeferred<Unit>();val pressResult=CompletableDeferred<Receipt>();val releaseEntered=CompletableDeferred<Unit>();val releaseResult=CompletableDeferred<Receipt>()
        val writes=mutableListOf<ShellInput>();val recovered=mutableListOf<TerminalPendingText>();var stops=0
        val owner=TerminalInputOwner(this,{true},{true},{},{0},{input,_->
            writes.add(input);val phase=(input as ShellInput.Keys).events.single().phase
            if(phase==ShellKeyPhase.Press){pressEntered.complete(Unit);pressResult.await()}else{releaseEntered.complete(Unit);releaseResult.await()}
        },{_,_->},{recovered.addAll(it)},{stops++})
        try {
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,id=42)))))
            withTimeout(1000){pressEntered.await()}
            assertTrue(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Right,id=77)))))
            assertTrue(owner.offer(ShellInput.Text("newer draft snapshot"),23))
            owner.blockForHistory(true)
            assertFalse(owner.accepting());assertTrue(owner.retained());assertEquals(1,writes.size);assertTrue(recovered.isEmpty())
            pressResult.complete(receipt())
            withTimeout(1000){releaseEntered.await()}
            val release=(writes.last() as ShellInput.Keys).events.single()
            assertEquals(ShellKeyPhase.Release,release.phase);assertEquals(42L,release.physicalKeyId!!.value);assertEquals(ShellKeyName.Left,release.key.key)
            assertFalse(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,42)))))
            assertFalse(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Right,ShellKeyPhase.Release,77)))))
            assertFalse(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Repeat,42)))))
            assertEquals(listOf(TerminalPendingText("newer draft snapshot",23)),recovered)
            releaseResult.complete(receipt());withTimeout(1000){owner.historyBarrier()}
            assertEquals(2,writes.size);assertFalse(owner.hardwareOutstanding());assertFalse(owner.paused);assertEquals(0,stops)
            assertFalse(owner.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,ShellKeyPhase.Release,42)))))
            owner.blockForHistory(false);assertTrue(owner.accepting())
        }finally{pressResult.complete(receipt(ReceiptState.Uncertain));releaseResult.complete(receipt(ReceiptState.Uncertain));owner.retire()}
    }
    @Test fun historyBarrierNeverReplaysInflightUncertaintyAndKeepsFailedTextBeforeLaterUnsentText()=runBlocking {
        for(state in listOf(ReceiptState.Failed,ReceiptState.Uncertain)) {
            val entered=CompletableDeferred<Unit>();val result=CompletableDeferred<Receipt>();val recovered=mutableListOf<TerminalPendingText>();var writes=0
            val owner=TerminalInputOwner(this,{true},{true},{},{0},{_,_->writes++;entered.complete(Unit);result.await()},{_,_->},{recovered.addAll(it)},{})
            try {
                assertTrue(owner.offer(ShellInput.Text("possibly entered"),30));withTimeout(1000){entered.await()}
                assertTrue(owner.offer(ShellInput.Text("definitely unsent"),31));owner.blockForHistory(true)
                result.complete(receipt(state,if(state==ReceiptState.Failed)ShellDispatchProgress()else ShellDispatchProgress(19,7,false)))
                withTimeout(1000){while(owner.pendingEvents!=0)yield()}
                assertEquals(if(state==ReceiptState.Failed)listOf(TerminalPendingText("possibly entered",30),TerminalPendingText("definitely unsent",31))else listOf(TerminalPendingText("definitely unsent",31)),recovered)
                owner.blockForHistory(false);assertFalse(owner.offer(ShellInput.Text("not an automatic retry")));assertEquals(1,writes)
            }finally{result.complete(receipt(ReceiptState.Uncertain));owner.retire()}
        }
    }
    @Test fun freshOwnerBlockedBeforeFirstExposureCannotOfferUntilExplicitUnblock()=runBlocking {
        val written=mutableListOf<ShellInput>();var completed=0;var stops=0
        val prepared=TerminalInputOwner(this,{true},{true},{},{0},{input,_->written.add(input);receipt()},{_,_->completed++},{fail("A fresh blocked owner has no unsent text")},{stops++})
        prepared.blockForHistory(true)
        val exposed=prepared
        try {
            assertTrue(exposed.retained());assertFalse(exposed.accepting())
            assertFalse(exposed.offer(ShellInput.Text("must stay local")))
            assertFalse(exposed.offer(ShellInput.Keys(listOf(key(ShellKeyName.Left,id=42)))))
            yield();assertTrue(written.isEmpty());assertEquals(0,exposed.pendingEvents);assertEquals(0,stops)
            exposed.blockForHistory(false);assertTrue(exposed.accepting())
            assertTrue(exposed.offer(ShellInput.Text("fresh explicit gesture")))
            withTimeout(1000){while(completed!=1)yield()}
            assertEquals(listOf(ShellInput.Text("fresh explicit gesture")),written);assertEquals(0,stops)
        }finally{exposed.retire()}
    }
    @Test fun draftPasteReplacesOnlyItsSelectionAndPreservesQueuedRecoveryFacts() {
        val value=androidx.compose.ui.text.input.TextFieldValue("before old after",androidx.compose.ui.text.TextRange(10,7))
        val modifiers=TerminalModifierState(ctrl=TerminalModifierMode.Locked)
        val original=ShellInputDraft(value,revision=7,keyCharacter="q",modifiers=modifiers,queuedRevision=6)
        val paste=ShellInput.Text("界\n😀\t")
        val next=checkNotNull(original.withPastedText(paste,0))
        assertEquals("before 界\n😀\t after",next.value.text)
        assertEquals(androidx.compose.ui.text.TextRange(7+paste.text.length),next.value.selection)
        assertEquals(8L,next.revision);assertEquals(6L,next.queuedRevision)
        assertEquals(modifiers,next.modifiers);assertEquals("q",next.keyCharacter);assertFalse(next.direct)
        assertEquals(value,original.value);assertEquals(7L,original.revision)
    }

    @Test fun draftPasteRejectsTheEntireItemAtCombinedQueueAndRecoveryBounds() {
        val original=ShellInputDraft(androidx.compose.ui.text.input.TextFieldValue("a".repeat(49149)),revision=4,queuedRevision=3)
        val text=ShellInput.Text("界")
        assertEquals(49152,utf8Size(checkNotNull(original.withPastedText(text,0)).value.text))
        assertNull(original.withPastedText(text,1))
        assertEquals(49149,original.value.text.length);assertEquals(4L,original.revision);assertEquals(3L,original.queuedRevision)
        val offline=ShellInputDraft(androidx.compose.ui.text.input.TextFieldValue("a".repeat(65533)))
        assertEquals(65536,utf8Size(checkNotNull(offline.withPastedText(text,null)).value.text))
        assertNull(offline.copy(value=androidx.compose.ui.text.input.TextFieldValue("a".repeat(65534))).withPastedText(text,null))
    }

    @Test fun draftPasteCannotChangeLiveModeOrAdmitInvalidFullText() {
        val draft=ShellInputDraft(androidx.compose.ui.text.input.TextFieldValue("keep"),revision=9,queuedRevision=8)
        assertNull(draft.copy(direct=true).withPastedText(ShellInput.Text("valid"),0))
        listOf("", "\r", "\u001b", "\u0085", "\ud800", "a".repeat(16385), "😀".repeat(4097)).forEach {text->
            assertNull(draft.withPastedText(ShellInput.Text(text),null))
            assertEquals("keep",draft.value.text);assertEquals(9L,draft.revision);assertEquals(8L,draft.queuedRevision)
        }
    }
    @Test fun draftPasteRefusesAnInsertionThatWouldSplitAnExistingUnicodeScalar() {
        val original=ShellInputDraft(androidx.compose.ui.text.input.TextFieldValue("😀",androidx.compose.ui.text.TextRange(1)),revision=3)
        assertNull(original.withPastedText(ShellInput.Text("x"),null))
        assertEquals("😀",original.value.text);assertEquals(3L,original.revision)
        val whole=original.copy(value=original.value.copy(selection=androidx.compose.ui.text.TextRange(0,2)))
        assertEquals("x",checkNotNull(whole.withPastedText(ShellInput.Text("x"),null)).value.text)
    }
}
