// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TerminalDisplayScaleTest {
    @Test fun storedValuesRefuseInvalidTypesAndNormalizeOnlyFiniteBoundedFloats() {
        assertEquals(TERMINAL_DISPLAY_SCALE_DEFAULT,terminalDisplayScaleFromStored(null),0f)
        assertEquals(TERMINAL_DISPLAY_SCALE_DEFAULT,terminalDisplayScaleFromStored("1.5"),0f)
        assertEquals(TERMINAL_DISPLAY_SCALE_DEFAULT,terminalDisplayScale(Float.NaN),0f)
        assertEquals(TERMINAL_DISPLAY_SCALE_DEFAULT,terminalDisplayScale(Float.POSITIVE_INFINITY),0f)
        assertEquals(TERMINAL_DISPLAY_SCALE_MIN,terminalDisplayScale(.1f),0f)
        assertEquals(TERMINAL_DISPLAY_SCALE_MAX,terminalDisplayScale(8f),0f)
        assertEquals(1.25f,terminalDisplayScaleFromStored(1.25f),0f)
    }

    @Test fun statePersistsOnlyCommittedValuesRestoresThemAndResetsTheLocalView() {
        var stored:Float?=null
        var writes=0
        val state=TerminalDisplayScaleState(terminalDisplayScaleFromStored(stored)) {value->stored=value;writes++}
        state.set(1.5f)
        assertEquals(1.5f,state.value,0f)
        assertEquals(0,writes)
        state.persist()
        assertEquals(1,writes)
        assertEquals(1.5f,checkNotNull(stored),0f)
        state.persist()
        assertEquals(1,writes)
        val restored=TerminalDisplayScaleState(terminalDisplayScaleFromStored(stored)) {value->stored=value;writes++}
        assertEquals(1.5f,restored.value,0f)
        val epoch=restored.resetEpoch
        restored.reset()
        assertEquals(TERMINAL_DISPLAY_SCALE_DEFAULT,restored.value,0f)
        assertEquals(epoch+1,restored.resetEpoch)
        assertEquals(2,writes)
        assertTrue(stored==TERMINAL_DISPLAY_SCALE_DEFAULT)
    }
}
