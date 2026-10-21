// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Actual production hierarchy/create/partial-receipt presentation with finite projection data.
 * These display fixtures are not native bindings, Herdr parity or stock-provider conformance. */
class NativeInventoryTest {
    @get:Rule val ui=createComposeRule()
    private fun inventory(): HostSnapshotResult {
        val server=Uuid.random();val target=Target(Uuid.random(),Uuid.random())
        val summary=SessionSummary(target.sessionId,target.generation,server,Id("workspace"),Id("terminal-agent"),Id("pane-agent"),Provider.Omp,Id("native-conversation"),"Provider conversation",SessionStatus.Working,SessionConnectivity.Ready,Capabilities(true,true,true,true,PromptMode.SubmitOrSteer,CancelMode.CurrentWork),HistoryCoverage(HistoryScope.AllBranches,HistorySource.PersistedAndLive,false,false,false))
        val graph=NativeHierarchy(server,Uuid.random(),NativeFreshness.Current,workspaces=listOf(NativeWorkspace(Id("workspace"),Uint(7),Label("Native workspace"),true,Id("tab"),NativeAgentStatus.Done)),tabs=listOf(NativeTab(Id("tab"),Id("workspace"),Uint(3),Label("Native tab"),true,NativeAgentStatus.Blocked)),panes=listOf(NativePane(Id("pane-shell"),Id("terminal-shell"),Id("workspace"),Id("tab"),label=Label("Shell only"),agent=Label("bash"),focused=true,agentStatus=NativeAgentStatus.Idle),NativePane(Id("pane-agent"),Id("terminal-agent"),Id("workspace"),Id("tab"),label=Label("Native pane label"),title=Label("Native pane display"),agent=Label("omp"),focused=false,agentStatus=NativeAgentStatus.Done,target=target)))
        return HostSnapshotResult(Uuid.random(),ZeroValue(),listOf(Server(server,Label("Named server"),ServerConnectivity.Online,CanCreate(false,false))),emptyList(),listOf(summary),emptyList(),listOf(graph)).also {it.validate()}
    }
    @Test fun actualHierarchyKeepsNativeFocusDecorationsSeparateAndStaleAssociationsReadOnly() {
        var current by mutableStateOf(inventory())
        var ready by mutableStateOf(true)
        var selected: Target?=null
        ui.setContent {MaterialTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {InventoryRows(current,ready) {selected=it}}}}
        ui.onNodeWithText("Server Named server · Online").assertExists()
        ui.onNodeWithText("Workspace 7: Native workspace · Done · desktop focused").assertExists()
        ui.onNodeWithText("Tab 3: Native tab · Blocked · desktop focused").assertExists()
        ui.onNodeWithText("Pane Shell only · native Idle · desktop focused").assertExists()
        ui.onNodeWithText("Native pane display").assertExists()
        ui.onNodeWithText("Provider conversation · Omp · provider Working · Ready").assertExists()
        assertNull("Desktop focus is display metadata, not a phone selection",selected)
        ui.runOnIdle {current=current.copy(hierarchy=current.hierarchy.map {h->h.copy(panes=h.panes.map {it.copy(focused=!it.focused)})})}
        assertNull(selected)
        ui.runOnIdle {ready=false}
        ui.onNodeWithTag("inventory_stale").assertExists()
        ui.onNodeWithTag("pane_open").assertDoesNotExist()
        ui.onAllNodesWithText("Read-only pane · no current mutable phone Target.").assertCountEquals(2)
    }
    @Test fun unboundRowsAreVisibleWithoutOpeningAndBoundSelectionRemainsExplicit() {
        val original=inventory()
        val omp=UnboundRow(Uuid.random(),original.servers.single().serverId,Id("workspace"),Id("omp-terminal"),Id("omp-pane"),Provider.Omp,"OMP desktop session",UnboundState.SetupRequired,"desktop collaboration is not active")
        val codex=UnboundRow(Uuid.random(),original.servers.single().serverId,Id("workspace"),Id("codex-terminal"),Id("codex-pane"),Provider.Codex,"Codex desktop session",UnboundState.Unavailable,"shared owner binding is unavailable")
        val snapshot=original.copy(unbound=listOf(omp,codex)).also {it.validate()}
        var selected: Target?=null
        ui.setContent {MaterialTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {SessionBrowser(snapshot,true,SessionListPreferences(),null) {selected=it}}}}
        ui.onNodeWithText("OMP desktop session · Omp · SetupRequired").assertExists()
        ui.onNodeWithText("desktop collaboration is not active").assertExists()
        ui.onAllNodesWithText("Resolve the reported desktop setup issue, then refresh.").assertCountEquals(2)
        ui.onNodeWithText("Codex desktop session · Codex · Unavailable").assertExists()
        ui.onNodeWithText("shared owner binding is unavailable").assertExists()
        ui.onNode(hasTestTag("unbound_${omp.rowId.value}") and hasClickAction()).assertDoesNotExist()
        ui.onNode(hasTestTag("unbound_${codex.rowId.value}") and hasClickAction()).assertDoesNotExist()
        assertNull("Unbound rows never fabricate a phone Target",selected)
        val summary=snapshot.sessions.single()
        ui.onNodeWithTag("session_${summary.sessionId.value}").performScrollTo().performClick()
        assertEquals(Target(summary.sessionId,summary.generation),selected)
        ui.onNodeWithTag("session_search").performScrollTo().performTextReplacement("does not match")
        ui.onNodeWithText("No matching conversations.").assertExists()
    }
    @Test fun noProjectsAndUnavailableNativeSourceDoNotOfferCreationAndPartialResourcesRemainVisible() {
        val context=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext
        computer.iroh.IrohAndroid.installAndroidContext(context)
        val snapshot=inventory().let {it.copy(servers=it.servers.map {server->server.copy(canCreate=CanCreate(true,true))})}
        var current by mutableStateOf(snapshot)
        val hello=HelloResult(EndpointId("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"),Uuid.random(),Uuid.random(),ProtocolVersion(),listOf(Backend(Provider.Omp,"18.6.1",Availability.Ready),Backend(Provider.Codex,"0.159.1",Availability.Ready)))
        val progress=CreationProgress(snapshot.servers.single().serverId,Uuid.random(),Provider.Codex,CreationStage.Launch,StageState.Failed,CreationResources(workspaceId=Id("known-workspace"),paneId=Id("known-pane"),terminalId=Id("known-terminal"),nativeId=Id("known-thread"),bindingId=Uuid.random(),ownerStarted=false,relayStarted=true))
        val metadata=ActionMetadata(Uuid.random(),ActionMethod.SessionCreate,Uuid.random(),null,Uint(1))
        val receipt=Receipt(metadata.actionId,metadata.method,metadata.bootId,null,Uint(1),Uint(2),ReceiptState.Failed,creation=progress).also {it.validate()}
        ui.setContent {MaterialTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            val memory=remember {FormMemory().apply {text["name"]="Explicit new session"}}
            CreateSession(current,hello,true,memory) {_,_,_,_->fail("No project/current native source must not offer a create action")}
            ReceiptPanel("synthetic-display-host",LocalAction(metadata,receipt,true,false))
        }}}
        ui.onNodeWithTag("session_create").performScrollTo().assertIsNotEnabled()
        ui.onNodeWithText("No configured projects. New creation is unavailable; existing sessions remain host-wide accessible.").assertExists()
        ui.runOnIdle {current=snapshot.copy(projects=listOf(Project(Uuid.random(),Label("Available project"),true)),hierarchy=snapshot.hierarchy.map {it.copy(freshness=NativeFreshness.Unavailable,reason=NativeUnavailableReason.SourceUnavailable,workspaces=emptyList(),tabs=emptyList(),panes=emptyList())}).also {it.validate()}}
        ui.onNodeWithTag("session_create").performScrollTo().assertIsNotEnabled()
        ui.onNodeWithTag("receipt_exact").assertDoesNotExist()
        ui.onNodeWithTag("receipt_details_${metadata.actionId.value}").performScrollTo().performClick()
        val displayed=ui.onNodeWithTag("receipt_exact").performScrollTo().assertIsDisplayed().fetchSemanticsNode()
            .config[androidx.compose.ui.semantics.SemanticsProperties.Text].joinToString("") {it.text}
        val actual=Protocol.strictJson.decodeFromString(Receipt.serializer(),displayed)
        assertEquals(receipt,actual)
        assertEquals(progress.resources,actual.creation!!.resources)
    }
    @Test fun threeInspectedCreationResetsKeepTheMountedOwnedForm() {
        val host="synthetic-creation-display"
        val snapshot=inventory().let {it.copy(servers=it.servers.map {s->s.copy(canCreate=CanCreate(true,true))},projects=listOf(Project(Uuid.random(),Label("Synthetic project"),true)))}
        val forms=mutableMapOf(host to FormMemory())
        val actions=mutableMapOf<String,Uuid>()
        val owned=forms.getValue(host)
        var submissions=0
        ui.setContent {GoatrTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            val mounted=remember {forms.getValue(host)}
            CreateSession(snapshot,null,true,mounted) {_,_,_,_->submissions++;actions[host]=Uuid.random()}
        }}}
        repeat(3) {index->
            ui.onNodeWithTag("create_name").performScrollTo().performTextReplacement("Synthetic creation $index")
            ui.onNodeWithTag("session_create").performScrollTo().assertIsEnabled().performClick()
            ui.onNodeWithTag("session_create").assertIsNotEnabled()
            val metadata=ActionMetadata(actions.getValue(host),ActionMethod.SessionCreate,Uuid.random(),null,Uint(1))
            val progress=CreationProgress(snapshot.servers.single().serverId,snapshot.projects.single().projectId,Provider.Omp,CreationStage.Launch,StageState.Failed,CreationResources())
            val receipt=Receipt(metadata.actionId,metadata.method,metadata.bootId,null,Uint(1),Uint(2),ReceiptState.Failed,creation=progress)
            val last=LocalAction(metadata,receipt,true,false)
            ui.runOnIdle {resetCompletedCreation(last,host,actions,forms)}
            ui.onNodeWithTag("session_create").assertIsNotEnabled()
            ui.runOnIdle {resetCompletedCreation(last.copy(outcomeShown=true),host,actions,forms)}
            assertSame(owned,forms.getValue(host))
            assertFalse(owned.text.containsKey("submitted"))
            assertFalse(actions.containsKey(host))
        }
        assertEquals(3,submissions)
    }
}
