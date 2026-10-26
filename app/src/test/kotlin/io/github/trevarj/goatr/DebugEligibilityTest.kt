// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class DebugEligibilityTest {
    @Test fun reservedArgumentsUnknownGatesAndReplacementRemainUnavailable() {
        val nonce="11111111-1111-4111-8111-111111111111"
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val server="22222222-2222-4222-8222-222222222222"
        val incarnation="33333333-3333-4333-8333-333333333333"
        val project="44444444-4444-4444-8444-444444444444"
        val args=arrayOf(DEBUG_ELIGIBILITY_NAMESPACE,nonce,"12345",host,server,incarnation,project)
        val parsed=DebugEligibilityArgs.parse(args)!!
        assertEquals(12345,parsed.pid);assertEquals(nonce,parsed.nonce)
        assertNull(DebugEligibilityArgs.parse(args+"extra"))
        assertNull(DebugEligibilityArgs.parse(args.copyOf().also {it[1]=nonce.uppercase().replaceFirst('1','A')}))
        assertNull(DebugEligibilityArgs.parse(args.copyOf().also {it[1]="11111111-1111-1111-8111-111111111111"}))
        assertNull(DebugEligibilityArgs.parse(args.copyOf().also {it[2]="2147483648"}))
        assertNull(DebugEligibilityArgs.parse(args.copyOf().also {it[2]="012345"}))
        assertNull(DebugEligibilityArgs.parse(args.copyOf().also {it[3]="x".repeat(65)}))
        assertNull(DebugEligibilityArgs.parse(args.copyOf().also {it[0]="--goatr-debug-eligibility-v2"}))
        assertFalse(debugEligibilityDelegate(true,null))
        assertFalse(debugEligibilityDelegate(true,arrayOf("unrecognized")))
        assertFalse(debugEligibilityDelegate(false,args+"malformed"))
        assertFalse(debugEligibilityDelegate(false,arrayOf("other","--goatr-debug-eligibility-v99")))
        assertTrue(debugEligibilityDelegate(false,arrayOf("unrelated-release-dump")))
        assertNull(debugEligibilityAnd(null,true,true,true,true))
        assertNull(debugEligibilityAnd(true,true,true,true,null))
        assertEquals(false,debugEligibilityAnd(null,true,true,true,false))
        assertEquals(true,debugEligibilityAnd(true,true,true,true,true))
        val publication=Any()
        assertTrue(debugEligibilityCoherent(200,199,publication,publication,true,true,true))
        assertFalse(debugEligibilityCoherent(200,200,publication,publication,true,true,true))
        assertFalse(debugEligibilityCoherent(200,199,publication,Any(),true,true,true))
        assertFalse(debugEligibilityCoherent(200,199,publication,publication,false,true,true))
        val main=DebugEligibilityMain(true,false,null,true){true}
        fun sample(mutation:Boolean)=DebugEligibilityConnection(
            helloPresent=true,inventoryPresent=true,scopeMatched=true,inventoryReady=true,mutationReady=mutation,
            serverOnline=true,sourceCurrent=true,projectAvailable=true,ompBackendReady=true,codexBackendReady=false,
            ompCanCreate=true,codexCanCreate=false,
        )
        fun record(coherent:Boolean)=debugEligibilityLine(nonce,true,true,coherent,main,sample(true),null)
        val line=record(true)
        val data=Json.parseToJsonElement(line.removePrefix("GOATR_DEBUG_ELIGIBILITY_V1 ")).jsonObject
        assertFalse(data.getValue("ongoingCreationKnown").jsonPrimitive.boolean)
        val rows=data.getValue("providers").jsonArray
        assertFalse(rows[0].jsonObject.getValue("commonAttemptEligibleKnown").jsonPrimitive.boolean)
        assertTrue(rows[1].jsonObject.getValue("providerCanCreateKnown").jsonPrimitive.boolean)
        assertFalse(rows[1].jsonObject.getValue("providerCanCreate").jsonPrimitive.boolean)
        assertFalse(rows[1].jsonObject.getValue("commonAttemptEligible").jsonPrimitive.boolean)
        val replaced=Json.parseToJsonElement(record(debugEligibilityCoherent(200,199,publication,Any(),true,true,true)).removePrefix("GOATR_DEBUG_ELIGIBILITY_V1 ")).jsonObject
        assertFalse(replaced.getValue("observed").jsonPrimitive.boolean)
        assertTrue(replaced.getValue("scopeUnproved").jsonPrimitive.boolean)
        assertTrue(replaced.getValue("providers").jsonArray.all { !it.jsonObject.getValue("providerCanCreateKnown").jsonPrimitive.boolean })
        assertEquals(true,sample(true).receiptsRecovered)
        assertNull(sample(false).receiptsRecovered)
        val unknownReceipt=Json.parseToJsonElement(debugEligibilityLine(nonce,true,true,true,main,sample(false),false).removePrefix("GOATR_DEBUG_ELIGIBILITY_V1 ")).jsonObject
        assertFalse(unknownReceipt.getValue("receiptsRecoveredKnown").jsonPrimitive.boolean)
        val serverId=Uuid(server)
        val graph=NativeHierarchy(serverId,Uuid(incarnation),NativeFreshness.Unavailable,NativeUnavailableReason.TooLarge,emptyList(),emptyList(),emptyList())
        val inventory=HostSnapshotResult(Uuid.random(),ZeroValue(),listOf(Server(serverId,Label("Fixture"),ServerConnectivity.Online,CanCreate(true,false))),emptyList(),emptyList(),emptyList(),listOf(graph))
        assertTrue(hostSourceCurrent(true,inventory,serverId))
        assertFalse(hostSourceCurrent(false,inventory,serverId))
        assertFalse(hostSourceCurrent(true,inventory.copy(hierarchy=listOf(graph.copy(freshness=NativeFreshness.Stale))),serverId))
        assertFalse(hostSourceCurrent(true,inventory.copy(hierarchy=listOf(graph.copy(reason=NativeUnavailableReason.SourceUnavailable))),serverId))
        assertNull(debugEligibilityPublished(parsed,ConnectionView()))
    }
}
