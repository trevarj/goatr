// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import org.junit.Assert.*
import org.junit.Test

class TerminalModifierStateTest {
    @Test fun firstTapArmsBeforeImmediateKeyAndCannotRetroactivelyLock() {
        val first=TerminalModifierState().tap(TerminalModifier.Ctrl,1000,300)
        assertEquals(ShellKeyModifiers(false,true,false,false),first.wire())
        val afterKey=first.afterKey()
        assertEquals(ShellKeyModifiers(false,false,false,false),afterKey.wire())
        assertNull(afterKey.lastTap)
        // A new tap inside the old double window follows the key, so it arms Once afresh.
        assertEquals(TerminalModifierMode.Once,afterKey.tap(TerminalModifier.Ctrl,1100,300).ctrl)
    }

    @Test fun consecutiveSecondTapLocksButContextResetOrDifferentTapCannot() {
        val first=TerminalModifierState().tap(TerminalModifier.Ctrl,1000,300)
        val locked=first.tap(TerminalModifier.Ctrl,1100,300)
        assertEquals(TerminalModifierMode.Locked,locked.ctrl)
        assertEquals(ShellKeyModifiers(false,true,false,false),locked.afterKey().wire())
        assertNull(locked.lastTap)
        assertEquals(TerminalModifierMode.Once,TerminalModifierState().tap(TerminalModifier.Ctrl,1100,300).ctrl)
        val other=first.tap(TerminalModifier.Shift,1050,300)
        assertEquals(TerminalModifierMode.Off,other.tap(TerminalModifier.Ctrl,1100,300).ctrl)
        assertEquals(TerminalModifierMode.Off,first.tap(TerminalModifier.Ctrl,1400,300).ctrl)
    }

    @Test fun nextKeyConsumesOneShotWithoutDroppingOtherLocks() {
        val state=TerminalModifierState().toggle(TerminalModifier.Ctrl,false).toggle(TerminalModifier.Alt,true)
        assertEquals(ShellKeyModifiers(false,true,true,false),state.wire())
        val consumed=state.afterKey()
        assertEquals(TerminalModifierMode.Off,consumed.ctrl)
        assertEquals(TerminalModifierMode.Locked,consumed.alt)
        assertEquals(ShellKeyModifiers(false,false,true,false),consumed.wire())
        assertEquals(consumed,consumed.afterKey())
    }

    @Test fun explicitLockAndReleaseNeverChangeAnotherModifier() {
        for(modifier in TerminalModifier.entries) {
            val once=TerminalModifierState().toggle(modifier,false)
            assertEquals(TerminalModifierMode.Once,once.mode(modifier))
            val locked=once.toggle(modifier,true)
            assertEquals(TerminalModifierMode.Locked,locked.mode(modifier))
            assertEquals(TerminalModifierState(),locked.toggle(modifier,false))
            assertEquals(TerminalModifierState(),locked.toggle(modifier,true))
            assertEquals(TerminalModifierState(),once.toggle(modifier,false))
            TerminalModifier.entries.filter {it!=modifier}.forEach {assertEquals(TerminalModifierMode.Off,locked.mode(it))}
        }
    }

    @Test fun eachModifierMapsToOnlyItsExistingWireBoolean() {
        val expected=mapOf(
            TerminalModifier.Ctrl to ShellKeyModifiers(false,true,false,false),
            TerminalModifier.Alt to ShellKeyModifiers(false,false,true,false),
            TerminalModifier.Shift to ShellKeyModifiers(true,false,false,false),
            TerminalModifier.Super to ShellKeyModifiers(false,false,false,true),
        )
        expected.forEach {(modifier,wire)->
            val once=TerminalModifierState().toggle(modifier,false)
            val locked=TerminalModifierState().toggle(modifier,true)
            assertEquals(wire,once.wire())
            assertEquals(wire,locked.afterKey().wire())
            assertEquals(ShellKeyModifiers(false,false,false,false),once.afterKey().wire())
        }
    }
}
