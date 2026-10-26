// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.test.platform.app.InstrumentationRegistry
import computer.iroh.IrohAndroid
import org.junit.BeforeClass
import org.junit.Test

class ProtocolFixtureTest {
    companion object {
        @BeforeClass @JvmStatic fun nativeContext() {
            IrohAndroid.installAndroidContext(InstrumentationRegistry.getInstrumentation().targetContext.applicationContext)
        }
    }
    private fun checks(): ProtocolFixtureChecks {
        val context = InstrumentationRegistry.getInstrumentation().context
        return ProtocolFixtureChecks(context.assets.open("v2.json").bufferedReader(Charsets.UTF_8).use { it.readText() })
    }
    @Test fun completeEnvelopesAndHashesIncludingRealNativeEndpointParsing() = checks().envelopes(includeNative = true)
    @Test fun rawAndWireBoundaries() = checks().boundaries()
    @Test fun finiteAnswersAndFormats() = checks().answersAndFormats()
    @Test fun exactDecimalAndRawNumericTokens() = checks().decimalsAndNativeTokens()
    @Test fun contextGuardsAndCanonicalExpansion() = checks().contextsAndCanonicalExpansion()
    @Test fun partialFrames() = checks().framing()
    @Test fun hierarchyFreshnessAndIndependentSelection() = checks().hierarchyRecoveryKeepsIndependentPhoneSelection()
    @Test fun browseViewsPreserveSeparateOrderAndRejectStaleResponses() = checks().browseViewsRejectStalePagesAndPreserveTheirOrderDomain()
    @Test fun hierarchyOverflowPreservesSourceUnavailability() = checks().hierarchyOverflowKeepsSourceUnavailability()
    @Test fun gridCommitAndUncertainBoundaries()=checks().gridCommitOwnsPixelsAndRejectsUncertainBoundaries()
    @Test fun gridViewportWideEdgeAndQuietBlink()=checks().gridViewportUsesWideExtentAndOnlyVisibleBlinkingContent()
    @Test fun gridMainDeliveryBarrier()=kotlinx.coroutines.runBlocking {checks().gridMainDeliveryCannotResurrectDismissedOrReplacedViewers()}
}
