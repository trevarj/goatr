// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.os.SystemClock
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role as SemanticsRole
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp

/** Phone-only editing state, never a wire modifier mode or permission. */
enum class TerminalModifier { Ctrl, Alt, Shift, Super }
enum class TerminalModifierMode { Off, Once, Locked }
data class TerminalModifierState(
    val ctrl: TerminalModifierMode = TerminalModifierMode.Off,
    val alt: TerminalModifierMode = TerminalModifierMode.Off,
    val shift: TerminalModifierMode = TerminalModifierMode.Off,
    val superKey: TerminalModifierMode = TerminalModifierMode.Off,
    val lastTap: TerminalModifier? = null,
    val lastTapAt: Long? = null,
) {
    fun mode(modifier: TerminalModifier): TerminalModifierMode = when(modifier) {
        TerminalModifier.Ctrl -> ctrl
        TerminalModifier.Alt -> alt
        TerminalModifier.Shift -> shift
        TerminalModifier.Super -> superKey
    }
    fun toggle(modifier: TerminalModifier, lock: Boolean): TerminalModifierState {
        val previous=mode(modifier)
        val next=if(lock) {
            if(previous==TerminalModifierMode.Locked) TerminalModifierMode.Off else TerminalModifierMode.Locked
        } else {
            if(previous==TerminalModifierMode.Off) TerminalModifierMode.Once else TerminalModifierMode.Off
        }
        return when(modifier) {
            TerminalModifier.Ctrl -> copy(ctrl=next,lastTap=null,lastTapAt=null)
            TerminalModifier.Alt -> copy(alt=next,lastTap=null,lastTapAt=null)
            TerminalModifier.Shift -> copy(shift=next,lastTap=null,lastTapAt=null)
            TerminalModifier.Super -> copy(superKey=next,lastTap=null,lastTapAt=null)
        }
    }
    fun tap(modifier: TerminalModifier, at: Long, doubleTapWindow: Long): TerminalModifierState {
        // First release arms Once immediately. Only an unconsumed, consecutive second
        // release may lock it; there is no delayed click that can arm after a key.
        val previous=lastTapAt
        if(mode(modifier)==TerminalModifierMode.Once && lastTap==modifier && previous!=null &&
            at>=previous && at-previous<=doubleTapWindow && doubleTapWindow>0) return toggle(modifier,true)
        val next=toggle(modifier,false)
        return if(next.mode(modifier)==TerminalModifierMode.Once) next.copy(lastTap=modifier,lastTapAt=at) else next
    }
    fun afterKey(): TerminalModifierState {
        fun retain(mode: TerminalModifierMode)=if(mode==TerminalModifierMode.Locked) mode else TerminalModifierMode.Off
        return TerminalModifierState(retain(ctrl),retain(alt),retain(shift),retain(superKey))
    }
    fun wire()=ShellKeyModifiers(shift!=TerminalModifierMode.Off,ctrl!=TerminalModifierMode.Off,alt!=TerminalModifierMode.Off,superKey!=TerminalModifierMode.Off)
}

/** One row, explicit gestures only. Double-tap/long-press changes local state, never sends a key. */
@Composable
internal fun TerminalKeyBar(state: TerminalModifierState, enabled: Boolean, onModifier: (TerminalModifier,Boolean)->Unit, onTap: (TerminalModifier,Long,Long)->Unit, onKey: (ShellKeyName)->Unit) {
    val doubleTapWindow=LocalViewConfiguration.current.doubleTapTimeoutMillis
    LazyRow(Modifier.fillMaxWidth().testTag("shell_key_bar"),horizontalArrangement=Arrangement.spacedBy(4.dp)) {
        items(TerminalModifier.entries,key={"modifier_${it.name}"}) {modifier->
            val mode=state.mode(modifier)
            val locked=mode==TerminalModifierMode.Locked
            val label=modifier.name+when(mode) {TerminalModifierMode.Off->"";TerminalModifierMode.Once->" (next)";TerminalModifierMode.Locked->" (locked)"}
            Surface(
                shape=MaterialTheme.shapes.small,
                color=when(mode) {TerminalModifierMode.Off->MaterialTheme.colorScheme.surfaceVariant;TerminalModifierMode.Once->MaterialTheme.colorScheme.primaryContainer;TerminalModifierMode.Locked->MaterialTheme.colorScheme.tertiaryContainer},
                modifier=Modifier.testTag("shell_modifier_${modifier.name}").combinedClickable(
                    enabled=enabled,role=SemanticsRole.Button,
                    // No onDoubleClick: combinedClickable must deliver first-up immediately.
                    onClick={onTap(modifier,SystemClock.uptimeMillis(),doubleTapWindow)},
                    onLongClickLabel=if(locked) "Unlock modifier" else "Lock modifier",
                    onLongClick={onModifier(modifier,true)},
                ).semantics {
                    selected=mode!=TerminalModifierMode.Off
                    stateDescription=when(mode) {TerminalModifierMode.Off->"Off";TerminalModifierMode.Once->"Next key only";TerminalModifierMode.Locked->"Locked locally"}
                    customActions=listOf(CustomAccessibilityAction(if(locked) "Unlock ${modifier.name}" else "Lock ${modifier.name}") {if(enabled){onModifier(modifier,true);true}else false})
                },
            ) {
                Box(Modifier.heightIn(min=48.dp).widthIn(min=48.dp).padding(horizontal=12.dp),contentAlignment=androidx.compose.ui.Alignment.Center) {UiText(label,style=MaterialTheme.typography.labelLarge)}
            }
        }
        items(ShellKeyName.entries.filter {it!=ShellKeyName.Character},key={"key_${it.name}"}) {key->
            OutlinedButton(onClick={onKey(key)},enabled=enabled,modifier=Modifier.heightIn(min=48.dp).testTag("shell_key_${key.name}")) {UiText(if(key==ShellKeyName.Escape) "Esc" else key.name)}
        }
    }
}
