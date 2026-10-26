// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.io.File
import org.junit.Test

class ProtocolFixtureTest {
    private fun checks() = ProtocolFixtureChecks(File(System.getProperty("goatr.fixtures"),"v2.json").readText(Charsets.UTF_8))
    @Test fun nonNativeEnvelopesAndHashes() = checks().envelopes(includeNative = false)
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
}
