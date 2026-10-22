// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.Role as SemanticsRole
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Actual production hierarchy/create/partial-receipt presentation with finite projection data.
 * These display fixtures are not native bindings, Herdr parity or stock-provider conformance. */
class NativeInventoryTest {
    @get:Rule val ui=createComposeRule()
    @Test fun compactProductionTitleBarKeepsLogoNavigationAndTitleAtTwoHundredPercent() {
        val host="523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"
        val hello=HelloResult(EndpointId(host),Uuid.random(),Uuid.random(),ProtocolVersion(),Provider.entries.map {Backend(it,"component geometry",Availability.SetupRequired,"No native binding")})
        val offline=ConnectionView()
        val syncing=ConnectionView(hello=mapOf(host to hello))
        val online=ConnectionView(hello=mapOf(host to hello),inventoryReady=mapOf(host to true))
        val direct=ConnectionView(hello=mapOf(host to hello),inventoryReady=mapOf(host to true),selectedPath=mapOf(host to "direct"))
        val relay=ConnectionView(hello=mapOf(host to hello),inventoryReady=mapOf(host to true),selectedPath=mapOf(host to "relay"))
        var view by mutableStateOf(offline)
        var connecting by mutableStateOf(false)
        var fontScale by mutableStateOf(1f)
        var pageTitle by mutableStateOf("goatr")
        var backs=0;var menus=0
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,fontScale)) {
                Box(Modifier.width(320.dp)) {
                    GoatrTitleBar(pageTitle,connectionLabel(host,view,connecting),connecting || host in view.hello && view.inventoryReady[host]!=true,compactConnectionLabel(host,view,connecting),modifier=Modifier.testTag("compact_header"),navigationIcon={
                        androidx.compose.material3.IconButton(onClick={backs++},modifier=Modifier.size(48.dp).testTag("compact_back")) {GoatrIcon(MaterialGlyph.Back,"Back")}
                    },actions={
                        androidx.compose.material3.IconButton(onClick={menus++},modifier=Modifier.size(48.dp).testTag("compact_menu")) {GoatrIcon(MaterialGlyph.Menu,"Menu")}
                    })
                }
            }}
        }
        val progress=hasProgressBarRangeInfo(androidx.compose.ui.semantics.ProgressBarRangeInfo.Indeterminate)
        for(scale in listOf(1f,2f)) for(contextTitle in listOf("goatr","Original desktop agent with a deliberately long conversation title")) {
            ui.runOnIdle {fontScale=scale;pageTitle=contextTitle}
            val header=ui.onNodeWithTag("compact_header").assertHeightIsEqualTo(48.dp).fetchSemanticsNode().boundsInRoot
            val back=ui.onNodeWithTag("compact_back").assertIsDisplayed().assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot
            val menu=ui.onNodeWithTag("compact_menu").assertIsDisplayed().assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot
            val logo=ui.onNodeWithContentDescription("Goatr",useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            val title=ui.onNodeWithText(contextTitle,useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            assertTrue(back.right<=logo.left && logo.right<=title.left)
            assertTrue(logo.top>=header.top && logo.bottom<=header.bottom)
            if(contextTitle=="goatr") {
                val layouts=mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
                ui.onNodeWithText(contextTitle,useUnmergedTree=true).performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) {get->get(layouts)}
                assertFalse(layouts.single().hasVisualOverflow)
            }
            for((observed,inFlight) in listOf(offline to false,offline to true,syncing to false,direct to false,relay to false,online to false)) {
                ui.runOnIdle {view=observed;connecting=inFlight}
                val shortStatus=compactConnectionLabel(host,observed,inFlight)
                val statusNode=ui.onNodeWithText(shortStatus,useUnmergedTree=true).assertIsDisplayed()
                val status=statusNode.fetchSemanticsNode().boundsInRoot
                assertTrue(title.right<=status.left && status.right<=menu.left)
                assertTrue(status.top>=header.top && status.bottom<=header.bottom)
                val layouts=mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
                statusNode.performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) {get->get(layouts)}
                assertFalse("Essential connection state must not ellipsize: scale=$scale contextTitle=$contextTitle shortStatus=$shortStatus layoutSize=${layouts.single().size} maxIntrinsicWidth=${layouts.single().multiParagraph.intrinsics.maxIntrinsicWidth}",layouts.single().hasVisualOverflow)
                ui.onAllNodes(progress).assertCountEquals(if(inFlight || host in observed.hello && observed.inventoryReady[host]!=true)1 else 0)
                assertEquals(header,ui.onNodeWithTag("compact_header").fetchSemanticsNode().boundsInRoot)
                assertEquals(back,ui.onNodeWithTag("compact_back").fetchSemanticsNode().boundsInRoot)
                assertEquals(menu,ui.onNodeWithTag("compact_menu").fetchSemanticsNode().boundsInRoot)
                assertEquals(logo,ui.onNodeWithContentDescription("Goatr",useUnmergedTree=true).fetchSemanticsNode().boundsInRoot)
                assertEquals(title,ui.onNodeWithText(contextTitle,useUnmergedTree=true).fetchSemanticsNode().boundsInRoot)
            }
            val transportText=hasAnyAncestor(hasTestTag("compact_header")) and SemanticsMatcher("Rendered transport and its full description") {node->
                node.config.contains(SemanticsProperties.Text) && node.config.contains(SemanticsProperties.ContentDescription)
            }
            for(transport in listOf(direct,relay)) {
                ui.runOnIdle {view=transport;connecting=false}
                val before=ui.onNode(transportText,useUnmergedTree=true).fetchSemanticsNode()
                val renderedBefore=before.config[SemanticsProperties.Text]
                val descriptionBefore=before.config[SemanticsProperties.ContentDescription]
                for(current in listOf(false,true)) {
                    ui.runOnIdle {view=ConnectionView(hello=transport.hello,selectedPath=transport.selectedPath,inventoryReady=mapOf(host to current))}
                    val after=ui.onNode(transportText,useUnmergedTree=true).fetchSemanticsNode()
                    assertEquals(renderedBefore,after.config[SemanticsProperties.Text])
                    assertEquals(descriptionBefore,after.config[SemanticsProperties.ContentDescription])
                    assertEquals(before.boundsInRoot,after.boundsInRoot)
                    assertEquals(header,ui.onNodeWithTag("compact_header").fetchSemanticsNode().boundsInRoot)
                    assertEquals(back,ui.onNodeWithTag("compact_back").fetchSemanticsNode().boundsInRoot)
                    assertEquals(menu,ui.onNodeWithTag("compact_menu").fetchSemanticsNode().boundsInRoot)
                }
            }
        }
        assertEquals(0,backs);assertEquals(0,menus)
        ui.onNodeWithTag("compact_back").performClick()
        ui.onNodeWithTag("compact_menu").performClick()
        assertEquals(1,backs);assertEquals(1,menus)
    }

    @Test fun refreshHintKeepsLayoutStableAndProgressOnlyWhileRefreshing() {
        var refreshing by mutableStateOf(false)
        ui.setContent {MaterialTheme {Column(Modifier.fillMaxWidth()) {
            Box(Modifier.testTag("refresh_slot")) {RefreshHint(refreshing)}
            androidx.compose.material3.Text("Next session",modifier=Modifier.testTag("below_refresh"))
        }}}
        val slot=ui.onNodeWithTag("refresh_slot").fetchSemanticsNode().boundsInRoot
        val below=ui.onNodeWithTag("below_refresh").fetchSemanticsNode().boundsInRoot
        val progress=hasProgressBarRangeInfo(androidx.compose.ui.semantics.ProgressBarRangeInfo.Indeterminate)
        ui.onAllNodes(progress).assertCountEquals(0)
        for(active in listOf(true,false,true,false)) {
            ui.runOnIdle {refreshing=active}
            ui.onAllNodes(progress).assertCountEquals(if(active) 1 else 0)
            assertEquals(slot,ui.onNodeWithTag("refresh_slot").fetchSemanticsNode().boundsInRoot)
            assertEquals(below,ui.onNodeWithTag("below_refresh").fetchSemanticsNode().boundsInRoot)
        }
    }

    @Test fun workspaceDisclosuresSurviveLazyRemovalButResetForReplacementIdentity() {
        val original=inventory()
        var current by mutableStateOf(original)
        var ready by mutableStateOf(true)
        var host by mutableStateOf("original-host")
        var opened: Target?=null
        var openCount=0
        val originalGroup=projectSessionDirectory(host,original,true,original.sessions).workspaces.single()
        val originalTarget=original.sessions.single().let {Target(it.sessionId,it.generation)}
        ui.setContent {MaterialTheme {LazyColumn(Modifier.fillMaxSize()) {
            val directory=projectSessionDirectory(host,current,ready,current.sessions)
            directory.workspaces.forEach {group->
                item(key=group.key.lazyKey) {
                    WorkspaceDirectoryCard(group,host,SessionListPreferences()) {opened=it;openCount++}
                }
            }
            directory.standalone.forEach {summary->
                item(key="session_${host}_${summary.sessionId.value}_${summary.generation.value}") {
                    DirectoryAgentRow(summary,host,null,SessionListPreferences(),!ready,"session_${host}_${summary.sessionId.value}_${summary.generation.value}",{opened=it;openCount++})
                }
            }
        }}}
        val originalRow="session_${host}_${originalTarget.sessionId.value}_${originalTarget.generation.value}"
        ui.onNodeWithTag(originalRow).assertDoesNotExist()
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").assertHasClickAction().assert(SemanticsMatcher.expectValue(SemanticsProperties.Role,SemanticsRole.Button)).assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Agents collapsed"))
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details collapsed"))
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").performScrollTo().performClick()
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Agents expanded"))
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details collapsed"))
        ui.onNodeWithTag("${originalGroup.key.tag}_details").performScrollTo().performClick()
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details expanded"))
        ui.onNodeWithTag(originalRow).assertExists()
        ui.onNodeWithContentDescription("Hide workspace details").assertExists()
        assertEquals(0,openCount)
        ui.runOnIdle {ready=false}
        ui.onNodeWithTag(originalGroup.key.tag).assertDoesNotExist()
        ui.onNodeWithText("Stale · Working").assertExists()
        assertEquals(original.sessions,projectSessionDirectory(host,current,false,current.sessions).standalone)
        assertNull(opened)
        ui.runOnIdle {ready=true}
        ui.onNodeWithTag(originalRow).assertExists()
        ui.onNodeWithContentDescription("Hide workspace details").assertExists()
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").assert(SemanticsMatcher.expectValue(SemanticsProperties.Role,SemanticsRole.Button)).assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Agents expanded"))
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details expanded"))
        assertEquals(0,openCount)
        ui.onNodeWithTag(originalRow).performScrollTo().performClick()
        assertEquals(originalTarget,opened)
        assertEquals(1,openCount)
        for(replacement in listOf("host","incarnation","workspace")) {
            ui.runOnIdle {
                when(replacement) {
                    "host"->host="replacement-host"
                    "incarnation"->current=current.copy(hierarchy=current.hierarchy.map {it.copy(serverIncarnation=Uuid.random())})
                    else->{
                        val workspace=Id("replacement-workspace")
                        current=current.copy(
                            sessions=current.sessions.map {it.copy(workspaceId=workspace)},
                            hierarchy=current.hierarchy.map {graph->graph.copy(
                                workspaces=graph.workspaces.map {it.copy(workspaceId=workspace)},
                                tabs=graph.tabs.map {it.copy(workspaceId=workspace)},
                                panes=graph.panes.map {it.copy(workspaceId=workspace)},
                            )},
                        )
                    }
                }
            }
            val group=projectSessionDirectory(host,current,true,current.sessions).workspaces.single()
            val row="session_${host}_${originalTarget.sessionId.value}_${originalTarget.generation.value}"
            ui.onNodeWithTag(row).assertDoesNotExist()
            ui.onNodeWithContentDescription("Hide workspace details").assertDoesNotExist()
            ui.onNodeWithContentDescription("Show workspace details").assertExists()
            ui.onNodeWithTag("${group.key.tag}_agents").assert(SemanticsMatcher.expectValue(SemanticsProperties.Role,SemanticsRole.Button)).assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Agents collapsed"))
            ui.onNodeWithTag("${group.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details collapsed"))
            assertEquals(1,openCount)
            if(replacement!="workspace") {
                ui.onNodeWithTag("${group.key.tag}_agents").performScrollTo().performClick()
                ui.onNodeWithTag("${group.key.tag}_details").performScrollTo().performClick()
            }
        }
    }

    @Test fun directoryRowConnectivityTransitionsKeepGeometryAndNeverOpenPassively() {
        val original=inventory().sessions.single()
        var shown by mutableStateOf(original)
        var stale by mutableStateOf(false)
        var cachePartial by mutableStateOf<Boolean?>(null)
        var opened: Target?=null
        var openCount=0
        ui.setContent {GoatrTheme {Column(Modifier.fillMaxWidth()) {
            DirectoryAgentRow(shown,"Desktop","Workspace",SessionListPreferences(),stale,"transition_row",{opened=it;openCount++},cachePartial)
            androidx.compose.material3.Text("Next agent",modifier=Modifier.testTag("below_transition"))
        }}}
        val rowBounds=ui.onNodeWithTag("transition_row").fetchSemanticsNode().boundsInRoot
        val belowBounds=ui.onNodeWithTag("below_transition").fetchSemanticsNode().boundsInRoot
        val progress=hasProgressBarRangeInfo(androidx.compose.ui.semantics.ProgressBarRangeInfo.Indeterminate)
        ui.onAllNodes(progress).assertCountEquals(0)
        ui.onNodeWithTag("transition_row_details").performClick()
        ui.onNodeWithText("Current status: Working").assertExists()
        ui.onNodeWithText("Close").performClick()
        val transitions=listOf(
            Triple(SessionConnectivity.Synchronizing,false,null) to 1,
            Triple(SessionConnectivity.Offline,false,null) to 0,
            Triple(SessionConnectivity.Synchronizing,true,null) to 0,
            Triple(SessionConnectivity.Synchronizing,false,false) to 0,
            Triple(SessionConnectivity.Ready,true,null) to 0,
        )
        for((state,expectedProgress) in transitions) {
            ui.runOnIdle {shown=original.copy(connectivity=state.first);stale=state.second;cachePartial=state.third}
            ui.onAllNodes(progress).assertCountEquals(expectedProgress)
            assertEquals(rowBounds,ui.onNodeWithTag("transition_row").fetchSemanticsNode().boundsInRoot)
            assertEquals(belowBounds,ui.onNodeWithTag("below_transition").fetchSemanticsNode().boundsInRoot)
            ui.onNodeWithTag("transition_row_details").performClick()
            ui.onNodeWithText("Last observed status: Working").assertExists()
            ui.onNodeWithText("Current status: Working").assertDoesNotExist()
            ui.onNodeWithText("Close").performClick()
            assertEquals(0,openCount)
            assertNull(opened)
        }
        ui.onNodeWithTag("transition_row").performClick()
        assertEquals(Target(original.sessionId,original.generation),opened)
        assertEquals(1,openCount)
    }

    @Test fun directoryRowKeepsStatusAndDetailsContainedAtCompactTwoHundredPercent() {
        val original=inventory().sessions.single().copy(provider=Provider.Codex,title="Original conversation with desktop context",reason="Full desktop context remains available in details")
        var shown by mutableStateOf(original)
        var opened: Target?=null
        var openCount=0
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,2f)) {
                Column(Modifier.width(320.dp).testTag("responsive_container")) {
                    DirectoryAgentRow(shown,"Desktop","Workspace",SessionListPreferences(),false,"responsive_row",{opened=it;openCount++})
                    androidx.compose.material3.Text("Next agent",modifier=Modifier.testTag("below_responsive"))
                }
            }}
        }
        val rowBounds=ui.onNodeWithTag("responsive_row").fetchSemanticsNode().boundsInRoot
        val belowBounds=ui.onNodeWithTag("below_responsive").fetchSemanticsNode().boundsInRoot
        for(connectivity in listOf(SessionConnectivity.Ready,SessionConnectivity.Synchronizing,SessionConnectivity.Offline)) {
            ui.runOnIdle {shown=original.copy(connectivity=connectivity)}
            val container=ui.onNodeWithTag("responsive_container").fetchSemanticsNode().boundsInRoot
            val provider=ui.onNodeWithTag("responsive_row_provider",useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            val title=ui.onNodeWithTag("responsive_row_title",useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            val subtitle=ui.onNodeWithTag("responsive_row_subtitle",useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            val status=ui.onNodeWithTag("responsive_row_status",useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            val details=ui.onNodeWithTag("responsive_row_details").assertIsDisplayed().assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot
            for(bounds in listOf(provider,title,subtitle,status,details)) {
                assertTrue(bounds.left>=rowBounds.left && bounds.right<=rowBounds.right)
                assertTrue(bounds.top>=rowBounds.top && bounds.bottom<=rowBounds.bottom)
            }
            assertTrue(rowBounds.left>=container.left && rowBounds.right<=container.right)
            assertTrue(rowBounds.top>=container.top && rowBounds.bottom<=container.bottom)
            assertTrue(provider.right<=title.left)
            assertEquals(provider.center.y,title.center.y,1f)
            assertTrue(title.bottom<=subtitle.top && subtitle.bottom<=status.top)
            assertTrue(provider.bottom<=status.top && status.right<=details.left)
            assertEquals(rowBounds,ui.onNodeWithTag("responsive_row").fetchSemanticsNode().boundsInRoot)
            assertEquals(belowBounds,ui.onNodeWithTag("below_responsive").fetchSemanticsNode().boundsInRoot)
            assertEquals(0,openCount)
        }
        ui.onNodeWithTag("responsive_row_details").performClick()
        ui.onNodeWithTag("responsive_row_dialog").assertExists()
        ui.onNode(hasText(original.title) and hasAnyAncestor(hasTestTag("responsive_row_dialog")),useUnmergedTree=true).assertExists()
        ui.onNodeWithText(original.reason!!).performScrollTo().assertIsDisplayed()
        assertEquals(0,openCount)
        ui.onNodeWithTag("responsive_row_open").assertIsDisplayed().performClick()
        assertEquals(Target(original.sessionId,original.generation),opened)
        assertEquals(1,openCount)
    }

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

    @Test fun browserKeepsOriginalRowDetailsAcrossReorderAndScopesUiStateByHost() {
        val original=inventory().copy(hierarchy=emptyList()).also {it.validate()}
        val first=original.sessions.single()
        val second=first.copy(sessionId=Uuid.random(),generation=Uuid.random(),title="Provider second")
        val setup=UnboundRow(Uuid.random(),original.servers.single().serverId,Id("workspace"),Id("setup-terminal"),Id("setup-pane"),Provider.Omp,"Desktop setup",UnboundState.SetupRequired,"Original desktop setup reason")
        val baseSnapshot=original.copy(sessions=listOf(first,second),unbound=listOf(setup)).also {it.validate()}
        var snapshot by mutableStateOf(baseSnapshot)
        var ready by mutableStateOf(true)
        var large by mutableStateOf(false)
        lateinit var scroll:androidx.compose.foundation.ScrollState
        var host by mutableStateOf("original-host")
        val hostHello=HelloResult(EndpointId("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"),Uuid.random(),Uuid.random(),ProtocolVersion(),Provider.entries.map {Backend(it,"component geometry",Availability.SetupRequired,"No native binding")})
        var hostView by mutableStateOf(ConnectionView(hello=mapOf(host to hostHello),inventoryReady=mapOf(host to true),selectedPath=mapOf(host to "direct")))
        var hostTitle by mutableStateOf("Same visible desktop")
        var hostFailure by mutableStateOf("")
        var consoleCard by mutableStateOf(true)
        var hostClicks=0
        var opened: Target?=null
        var openedHost: String?=null
        var openCount=0
        val row="session_${first.sessionId.value}_${first.generation.value}"
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,if(large)2f else 1f)) {
                key(host) {
                    var query by remember(host) {mutableStateOf("")}
                    scroll=rememberScrollState()
                    Column(Modifier.width(320.dp).height(480.dp).verticalScroll(scroll).testTag("browser_parent")) {
                        val online=host in hostView.hello
                        SessionsHostCard(if(consoleCard)hostTitle else null,connectionLabel(host,hostView),online,online && hostView.inventoryReady[host]!=true,if(online) "" else hostFailure,modifier=Modifier.testTag("browser_host_card")) {
                            repeat(3) {index->androidx.compose.material3.IconButton(onClick={hostClicks++},modifier=Modifier.size(48.dp).testTag("browser_host_action_$index")) {GoatrIcon(MaterialGlyph.Settings,"Existing host action $index")}}
                        }
                        SessionBrowser(host,snapshot,ready,SessionListPreferences(),"Same visible desktop",query,{query=it}) {opened=it;openedHost=host;openCount++}
                        androidx.compose.material3.Text("After browser",modifier=Modifier.testTag("browser_after"))
                    }
                }
            }}
        }
        val secondRow="session_${second.sessionId.value}_${second.generation.value}"
        val setupRow="unbound_${setup.rowId.value}"
        val setupDetails="unbound_details_${setup.rowId.value}"
        val longTitle="Provider original conversation "+"long native title context ".repeat(8)
        val longReason="Full original provider reason ".repeat(6)
        val setupTitle="Original setup-only desktop row ".repeat(6)
        val setupReason="Complete original setup-only explanation ".repeat(5)
        val measured=listOf("browser_parent","browser_host_card","browser_host_action_0","browser_host_action_1","browser_host_action_2","session_search",row,secondRow,setupRow,setupDetails,"browser_after")
        for(largeText in listOf(false,true)) for(consoleStyle in listOf(true,false)) {
            ui.runOnIdle {large=largeText;consoleCard=consoleStyle;ready=true;snapshot=baseSnapshot;hostTitle="Same visible desktop";hostFailure="";hostView=ConnectionView(hello=mapOf(host to hostHello),inventoryReady=mapOf(host to true),selectedPath=mapOf(host to "direct"))}
            ui.onNodeWithTag(row).performScrollTo()
            val before=measured.map {ui.onNodeWithTag(it).getUnclippedBoundsInRoot()}
            val scrollBefore=scroll.value
            val extentBefore=scroll.maxValue
            for(step in 0..3) {
                ui.runOnIdle {
                    val expandedText=step==1 || step==2
                    val status=when(step) {0->SessionStatus.Idle;1->SessionStatus.Attention;2->SessionStatus.Unknown;else->first.status}
                    ready=step!=2
                    hostTitle=if(expandedText)"Same original paired desktop with a long display name" else "Same visible desktop"
                    hostFailure=if(step==2)"Reconnect timed out. Check the desktop service, then reconnect." else ""
                    hostView=if(step==2)ConnectionView() else ConnectionView(hello=mapOf(host to hostHello),inventoryReady=mapOf(host to (step!=1)),selectedPath=mapOf(host to if(step==1)"relay" else "direct"))
                    snapshot=baseSnapshot.copy(
                        sessions=listOf(first.copy(title=if(expandedText)longTitle else first.title,reason=if(expandedText)longReason else null,status=status,connectivity=if(ready)SessionConnectivity.Ready else SessionConnectivity.Offline),second.copy(status=status)),
                        unbound=listOf(setup.copy(title=if(expandedText)setupTitle else setup.title,reason=if(expandedText)setupReason else setup.reason)),
                    ).also {it.validate()}
                }
                assertEquals(before,measured.map {ui.onNodeWithTag(it).getUnclippedBoundsInRoot()})
                assertEquals(scrollBefore,scroll.value)
                assertEquals(extentBefore,scroll.maxValue)
                ui.onNodeWithTag("${row}_details").assertHeightIsAtLeast(48.dp)
                ui.onNodeWithTag(setupDetails).assertHeightIsAtLeast(48.dp)
                assertEquals(0,openCount)
                assertEquals(0,hostClicks)
                if(step==2) ui.onNodeWithText(hostFailure).assertExists()
            }
            ui.runOnIdle {ready=true;snapshot=baseSnapshot}
            ui.onNodeWithTag(row).performScrollTo()
            val firstBefore=ui.onNodeWithTag(row).getUnclippedBoundsInRoot()
            val secondBefore=ui.onNodeWithTag(secondRow).getUnclippedBoundsInRoot()
            val rowStatus=hasAnyAncestor(hasTestTag("${row}_status")) and SemanticsMatcher("Actual currentness text") {it.config.contains(SemanticsProperties.Text)}
            ui.mainClock.autoAdvance=false
            try {
                ui.runOnIdle {snapshot=baseSnapshot.copy(sessions=listOf(first,second.copy(status=SessionStatus.Attention))).also {it.validate()}}
                ui.mainClock.advanceTimeByFrame()
                ui.mainClock.advanceTimeBy(90)
                val entering=ui.onNodeWithTag(secondRow).getUnclippedBoundsInRoot()
                val currentText=ui.onNode(rowStatus,useUnmergedTree=true).fetchSemanticsNode().config[SemanticsProperties.Text]
                ui.runOnIdle {ready=false}
                ui.mainClock.advanceTimeByFrame()
                assertNotEquals(currentText,ui.onNode(rowStatus,useUnmergedTree=true).fetchSemanticsNode().config[SemanticsProperties.Text])
                assertEquals(0,openCount);assertEquals(0,hostClicks)
                ui.runOnIdle {ready=true}
                ui.mainClock.advanceTimeBy(1000)
                val entered=ui.onNodeWithTag(secondRow).getUnclippedBoundsInRoot()
                assertTrue(secondBefore.top<entering.top && entering.top<entered.top)
                assertEquals(firstBefore,ui.onNodeWithTag(row).getUnclippedBoundsInRoot())
                ui.runOnIdle {snapshot=baseSnapshot}
                ui.mainClock.advanceTimeByFrame()
                ui.mainClock.advanceTimeBy(90)
                val exiting=ui.onNodeWithTag(secondRow).getUnclippedBoundsInRoot()
                ui.mainClock.advanceTimeBy(1000)
                assertTrue(secondBefore.top<exiting.top && exiting.top<entered.top)
                assertEquals(secondBefore,ui.onNodeWithTag(secondRow).getUnclippedBoundsInRoot())
                assertEquals(0,openCount);assertEquals(0,hostClicks)
            } finally {ui.mainClock.autoAdvance=true}
            ui.runOnIdle {snapshot=baseSnapshot.copy(sessions=listOf(second,first)).also {it.validate()}}
            assertTrue(ui.onNodeWithTag(secondRow).getUnclippedBoundsInRoot().top<ui.onNodeWithTag(row).getUnclippedBoundsInRoot().top)
            assertEquals(0,openCount)
            ui.runOnIdle {snapshot=baseSnapshot.copy(unbound=listOf(setup.copy(title=setupTitle,reason=setupReason))).also {it.validate()}}
            ui.onNodeWithTag(setupDetails).performScrollTo().performClick()
            ui.onNodeWithText(setupReason).performScrollTo().assertIsDisplayed()
            assertEquals(0,openCount)
            ui.onNodeWithTag(setupDetails).performScrollTo().performClick()
        }
        ui.runOnIdle {large=false;ready=true;snapshot=baseSnapshot}
        ui.onNodeWithTag("session_search").performTextReplacement("Provider")
        ui.onNodeWithTag("${row}_details").performScrollTo().performClick()
        ui.onNodeWithTag("${row}_dialog").assertExists()
        ui.runOnIdle {snapshot=snapshot.copy(sessions=listOf(second,first)).also {it.validate()}}
        ui.onNodeWithTag("${row}_dialog").assertExists()
        assertEquals(0,openCount)
        ui.runOnIdle {host="replacement-host"}
        ui.onNodeWithTag("${row}_dialog").assertDoesNotExist()
        ui.onNodeWithTag("session_search").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
        ui.onNodeWithTag(row).performScrollTo().performClick()
        assertEquals("replacement-host",openedHost)
        assertEquals(Target(first.sessionId,first.generation),opened)
        ui.runOnIdle {host="original-host"}
        ui.onNodeWithTag("${row}_dialog").assertDoesNotExist()
        ui.onNodeWithTag("session_search").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
        ui.onNodeWithTag("${row}_details").performScrollTo().performClick()
        assertEquals(1,openCount)
        ui.onNodeWithTag("${row}_open").performClick()
        assertEquals(Target(first.sessionId,first.generation),opened)
        assertEquals("original-host",openedHost)
        assertEquals(2,openCount)
        ui.onNodeWithTag("${row}_details").performScrollTo().performClick()
        val replacement=first.copy(generation=Uuid.random())
        ui.runOnIdle {snapshot=snapshot.copy(sessions=listOf(second,replacement)).also {it.validate()}}
        ui.onNodeWithTag("${row}_dialog").assertDoesNotExist()
        ui.onNodeWithTag(row).assertDoesNotExist()
        assertEquals(2,openCount)
        ui.onNodeWithTag("session_${replacement.sessionId.value}_${replacement.generation.value}").performScrollTo().performClick()
        assertEquals(Target(replacement.sessionId,replacement.generation),opened)
        assertEquals(3,openCount)
    }

    @Test fun unboundRowsAreVisibleWithoutOpeningAndBoundSelectionRemainsExplicit() {
        val original=inventory()
        val omp=UnboundRow(Uuid.random(),original.servers.single().serverId,Id("workspace"),Id("omp-terminal"),Id("omp-pane"),Provider.Omp,"OMP desktop session",UnboundState.SetupRequired,"desktop collaboration is not active")
        val codex=UnboundRow(Uuid.random(),original.servers.single().serverId,Id("workspace"),Id("codex-terminal"),Id("codex-pane"),Provider.Codex,"Codex desktop session",UnboundState.Unavailable,"shared owner binding is unavailable")
        val snapshot=original.copy(unbound=listOf(omp,codex)).also {it.validate()}
        var selected: Target?=null
        ui.setContent {GoatrTheme {Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            var query by remember {mutableStateOf("")}
            SessionBrowser("original-host",snapshot,true,SessionListPreferences(),null,query,{query=it}) {selected=it}
        }}}
        for(row in listOf(omp,codex)) {
            ui.onNodeWithTag("unbound_${row.rowId.value}").performScrollTo().assertExists()
            ui.onNode(hasTestTag("unbound_${row.rowId.value}") and hasClickAction()).assertDoesNotExist()
            ui.onNodeWithTag("unbound_details_${row.rowId.value}").performScrollTo().performClick()
            ui.onNodeWithText(row.reason).performScrollTo().assertIsDisplayed()
        }
        assertNull("Unbound rows never fabricate a phone Target",selected)
        val summary=snapshot.sessions.single()
        ui.onNodeWithTag("session_${summary.sessionId.value}_${summary.generation.value}").performScrollTo().performClick()
        assertEquals(Target(summary.sessionId,summary.generation),selected)
        ui.onNodeWithTag("session_search").performScrollTo().performTextReplacement("does not match")
        ui.onNodeWithTag("session_${summary.sessionId.value}_${summary.generation.value}").assertDoesNotExist()
        assertEquals(Target(summary.sessionId,summary.generation),selected)
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
            CreateSession(current,hello,true,memory,{}) {_,_,_,_->fail("No project/current native source must not offer a create action")}
            ReceiptPanel("synthetic-display-host",LocalAction(metadata,receipt,true,false))
        }}}
        ui.onNodeWithTag("session_create").performScrollTo().assertIsNotEnabled()
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
}
