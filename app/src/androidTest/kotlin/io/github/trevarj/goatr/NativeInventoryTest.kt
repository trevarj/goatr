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
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.LayoutDirection
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
    @Test fun leadingBrandKeepsContextStatusAndControlsReachableWithoutRefreshLayoutShift() {
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
        var layoutDirection by mutableStateOf(LayoutDirection.Ltr)
        var pageTitle by mutableStateOf("")
        var width by mutableStateOf(320.dp)
        var horizontalPadding by mutableStateOf(0.dp)
        var hasActions by mutableStateOf(true)
        var rootCount by mutableStateOf<Int?>(null)
        var backs=0;var menus=0
        ui.setContent {
            val density=LocalDensity.current
            GoatrTheme {CompositionLocalProvider(LocalDensity provides Density(density.density,fontScale),LocalLayoutDirection provides layoutDirection) {
                // Filters uses this horizontal sheet inset: a 320dp screen supplies a 288dp bar.
                Column(Modifier.width(width).padding(horizontal=horizontalPadding)) {
                    val status=rootCount?.let {"$it connected"} ?: connectionLabel(host,view,connecting)
                    val compact=rootCount?.let {if(it==0)"Offline" else "$it online"} ?: compactConnectionLabel(host,view,connecting)
                    GoatrTitleBar(pageTitle,status,connecting || host in view.hello && view.inventoryReady[host]!=true,compact,modifier=Modifier.testTag("compact_header"),navigationIcon={
                        if(pageTitle.isNotEmpty()) androidx.compose.material3.IconButton(onClick={backs++},modifier=Modifier.size(48.dp).testTag("compact_back")) {GoatrIcon(MaterialGlyph.Back,"Back")}
                    },actions={
                        if(hasActions) androidx.compose.material3.IconButton(onClick={menus++},modifier=Modifier.size(48.dp).testTag("compact_menu")) {GoatrIcon(MaterialGlyph.Menu,"Menu")}
                    })
                }
            }}
        }
        val progress=hasProgressBarRangeInfo(androidx.compose.ui.semantics.ProgressBarRangeInfo.Indeterminate)
        val states:List<Triple<ConnectionView,Boolean,Int?>> = listOf(Triple(offline,false,null),Triple(offline,true,null),Triple(syncing,false,null),
            Triple(direct,false,null),Triple(relay,false,null),Triple(online,false,null))+listOf(0,1,12,128).map {Triple(online,false,it)}
        for(direction in listOf(LayoutDirection.Ltr,LayoutDirection.Rtl)) for(actionSlot in listOf(false,true)) for((screenWidth,sheetInset) in listOf(320.dp to 0.dp,360.dp to 0.dp,320.dp to 16.dp)) for(scale in listOf(1f,2f)) for(contextTitle in listOf("","Filters","Original desktop agent with a deliberately long conversation title")) {
            ui.runOnIdle {layoutDirection=direction;width=screenWidth;horizontalPadding=sheetInset;fontScale=scale;pageTitle=contextTitle;hasActions=actionSlot;rootCount=null;view=offline;connecting=false}
            val header=ui.onNodeWithTag("compact_header").fetchSemanticsNode().boundsInRoot
            val back=if(contextTitle.isNotEmpty()) ui.onNodeWithTag("compact_back").assertIsDisplayed().assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot else null
            val menu=if(actionSlot) ui.onNodeWithTag("compact_menu").assertIsDisplayed().assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(48.dp).fetchSemanticsNode().boundsInRoot else {ui.onNodeWithTag("compact_menu").assertDoesNotExist();null}
            val renderedTitle=contextTitle.ifEmpty {"Goatr"}
            val title=ui.onNodeWithText(renderedTitle,useUnmergedTree=true).assertIsDisplayed().assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)).fetchSemanticsNode().boundsInRoot
            val goat=ui.onNodeWithTag("titlebar_goat",useUnmergedTree=true).assertIsDisplayed().fetchSemanticsNode().boundsInRoot
            assertEquals("Goat must center on the title, not the title/status stack",title.center.y,goat.center.y,0.5f)
            assertTrue(goat.left>=header.left && goat.right<=header.right && goat.top>=header.top && goat.bottom<=header.bottom)
            assertTrue(if(direction==LayoutDirection.Ltr) goat.right<title.left else goat.left>title.right)
            back?.let {assertTrue(if(direction==LayoutDirection.Ltr) it.right<goat.left else it.left>goat.right)}
            menu?.let {assertTrue(if(direction==LayoutDirection.Ltr) title.right<it.left else title.left>it.right)}
            assertTrue(title.top>=header.top && title.bottom<=header.bottom)
            for((observed,inFlight,count) in states) {
                ui.runOnIdle {view=observed;connecting=inFlight;rootCount=count}
                val shortStatus=count?.let {if(it==0)"Offline" else "$it online"} ?: compactConnectionLabel(host,observed,inFlight)
                val fullStatus=count?.let {"$it connected"} ?: connectionLabel(host,observed,inFlight)
                val statusNode=ui.onNodeWithText(shortStatus,useUnmergedTree=true).assertIsDisplayed().assertContentDescriptionEquals(fullStatus)
                val status=statusNode.fetchSemanticsNode().boundsInRoot
                assertEquals(title.left,status.left,0.5f)
                assertEquals(title.right,status.right,0.5f)
                assertTrue(title.bottom<=status.top && if(direction==LayoutDirection.Ltr) status.right<=(menu?.left ?: header.right) else status.left>=(menu?.right ?: header.left))
                assertTrue("Status vertical containment: screenWidth=$screenWidth sheetInset=$sheetInset scale=$scale titleCategory=${when {contextTitle.isEmpty()->"blank";contextTitle=="Filters"->"filters";else->"long"}} actions=$actionSlot count=$count state=${when {count!=null->"root_count";inFlight->"linking";observed===offline->"offline";observed===syncing->"sync";observed===direct->"direct";observed===relay->"relay";else->"online"}} shortStatus=$shortStatus header=$header status=$status currentHeader=${ui.onNodeWithTag("compact_header").fetchSemanticsNode().boundsInRoot}",status.top>=header.top && status.bottom<=header.bottom)
                val layouts=mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
                statusNode.performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) {get->get(layouts)}
                assertFalse("Essential connection state must not ellipsize: screenWidth=$screenWidth sheetInset=$sheetInset scale=$scale contextTitle=$contextTitle shortStatus=$shortStatus layoutSize=${layouts.single().size} maxIntrinsicWidth=${layouts.single().multiParagraph.intrinsics.maxIntrinsicWidth}",layouts.single().hasVisualOverflow)
                // Loading is decorative; the complete observed status is the single spoken owner.
                ui.onAllNodes(progress).assertCountEquals(0)
                assertEquals(header,ui.onNodeWithTag("compact_header").fetchSemanticsNode().boundsInRoot)
                if(back!=null) assertEquals(back,ui.onNodeWithTag("compact_back").fetchSemanticsNode().boundsInRoot)
                if(menu!=null) assertEquals(menu,ui.onNodeWithTag("compact_menu").fetchSemanticsNode().boundsInRoot)
                assertEquals(title,ui.onNodeWithText(renderedTitle,useUnmergedTree=true).fetchSemanticsNode().boundsInRoot)
                assertEquals(goat,ui.onNodeWithTag("titlebar_goat",useUnmergedTree=true).fetchSemanticsNode().boundsInRoot)
            }
            val transportText=hasAnyAncestor(hasTestTag("compact_header")) and SemanticsMatcher("Rendered transport and its full description") {node->
                node.config.contains(SemanticsProperties.Text) && node.config.contains(SemanticsProperties.ContentDescription)
            }
            for(transport in listOf(direct,relay)) {
                ui.runOnIdle {view=transport;connecting=false;rootCount=null}
                val before=ui.onNode(transportText,useUnmergedTree=true).fetchSemanticsNode()
                val renderedBefore=before.config[SemanticsProperties.Text]
                val descriptionBefore=before.config[SemanticsProperties.ContentDescription]
                for(current in listOf(false,true)) {
                    ui.runOnIdle {view=ConnectionView(hello=transport.hello,selectedPath=transport.selectedPath,inventoryReady=mapOf(host to current))}
                    val after=ui.onNode(transportText,useUnmergedTree=true).fetchSemanticsNode()
                    assertEquals(renderedBefore,after.config[SemanticsProperties.Text])
                    assertEquals(descriptionBefore,after.config[SemanticsProperties.ContentDescription])
                    assertEquals(before.boundsInRoot,after.boundsInRoot)
                    assertEquals(goat,ui.onNodeWithTag("titlebar_goat",useUnmergedTree=true).fetchSemanticsNode().boundsInRoot)
                    assertEquals(header,ui.onNodeWithTag("compact_header").fetchSemanticsNode().boundsInRoot)
                    if(back!=null) assertEquals(back,ui.onNodeWithTag("compact_back").fetchSemanticsNode().boundsInRoot)
                    if(menu!=null) assertEquals(menu,ui.onNodeWithTag("compact_menu").fetchSemanticsNode().boundsInRoot)
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

    @Test fun productionWorkspaceItemsRetainExpandedPlacementDuringRefreshAndResetForReplacementIdentity() {
        val original=inventory()
        var current by mutableStateOf(original)
        var ready by mutableStateOf(true)
        var host by mutableStateOf("original-host")
        val hello=HelloResult(EndpointId("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"),Uuid.random(),Uuid.random(),ProtocolVersion(),emptyList())
        var liveHello by mutableStateOf<HelloResult?>(hello)
        var pairedDevice by mutableStateOf(Uuid.random())
        var savedAt by mutableStateOf(Uint(1))
        var opened: Target?=null
        var openCount=0
        val originalGroup=projectSessionDirectory(host,original,true,original.sessions).workspaces.single()
        val originalTarget=original.sessions.single().let {Target(it.sessionId,it.generation)}
        ui.setContent {MaterialTheme {
            val cached=CachedInventory(host,current,savedAt)
            val directory=rememberSessionDirectory(HostMetadata(host,"Desktop",pairedDevice),liveHello,cached.inventory,ready,current.sessions)
            LazyColumn(Modifier.fillMaxSize()) {
                workspaceDirectoryItems(directory,host,"Desktop",SessionListPreferences(),!ready,false,{}, {opened=it;openCount++},{fail("This inventory has no bound ShellTarget")})
            }
        }}
        val originalRow="session_${host}_${originalTarget.sessionId.value}_${originalTarget.generation.value}"
        ui.onNodeWithTag(originalRow).assertDoesNotExist()
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").assertHasClickAction().assert(SemanticsMatcher.expectValue(SemanticsProperties.Role,SemanticsRole.Button))
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details collapsed"))
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").performScrollTo().performClick()
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details collapsed"))
        ui.onNodeWithTag("${originalGroup.key.tag}_details").performScrollTo().performClick()
        ui.onNodeWithTag("${originalGroup.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details expanded"))
        ui.onNodeWithTag(originalRow).assertExists()
        ui.onNodeWithContentDescription("Hide workspace details").assertExists()
        assertEquals(0,openCount)
        val placement=ui.onNodeWithTag(originalRow).fetchSemanticsNode().boundsInRoot
        repeat(3) {
            ui.runOnIdle {ready=false}
            ui.onNodeWithTag(originalGroup.key.tag).assertExists()
            ui.onNodeWithText("Stale · Working").assertExists()
            ui.onNodeWithText("Last observed agents").assertDoesNotExist()
            assertEquals(placement,ui.onNodeWithTag(originalRow).fetchSemanticsNode().boundsInRoot)
            assertEquals(original.sessions,projectSessionDirectory(host,current,false,current.sessions).standalone)
            assertNull(opened)
            assertEquals(0,openCount)
            ui.runOnIdle {current=current.copy(subscription=Uuid.random());savedAt=Uint(savedAt.value+1);ready=true}
            assertEquals(placement,ui.onNodeWithTag(originalRow).fetchSemanticsNode().boundsInRoot)
        }
        for(contextChange in listOf("boot","pairing","offline")) {
            ui.runOnIdle {
                ready=false
                when(contextChange) {
                    "boot"->liveHello=hello.copy(bootId=Uuid.random())
                    "pairing"->pairedDevice=Uuid.random()
                    else->liveHello=null
                }
            }
            ui.onNodeWithTag(originalGroup.key.tag).assertDoesNotExist()
            ui.onNodeWithText("Agents").assertExists()
            ui.onNodeWithText("Stale · Working").assertExists()
            assertEquals(0,openCount)
            ui.runOnIdle {liveHello=hello;ready=true}
        }
        ui.onNodeWithTag(originalRow).assertExists()
        ui.onNodeWithContentDescription("Hide workspace details").assertExists()
        ui.onNodeWithTag("${originalGroup.key.tag}_agents").assert(SemanticsMatcher.expectValue(SemanticsProperties.Role,SemanticsRole.Button))
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
            ui.onNodeWithTag("${group.key.tag}_details").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Workspace details collapsed"))
            assertEquals(1,openCount)
            if(replacement!="workspace") {
                ui.onNodeWithTag("${group.key.tag}_agents").performScrollTo().performClick()
                ui.onNodeWithTag("${group.key.tag}_details").performScrollTo().performClick()
            }
        }
        ui.runOnIdle {ready=false;current=current.copy(hierarchy=current.hierarchy.map {it.copy(freshness=NativeFreshness.Unavailable,reason=NativeUnavailableReason.SourceUnavailable,workspaces=emptyList(),tabs=emptyList(),panes=emptyList())})}
        ui.onNodeWithContentDescription("Show workspace details").assertDoesNotExist()
        ui.onNodeWithText("Agents").assertExists()
        ui.onNodeWithText("Last observed agents").assertDoesNotExist()
        ui.runOnIdle {current=current.copy(sessions=emptyList())}
        ui.onNodeWithText("Agents").assertDoesNotExist()
        assertEquals(1,openCount)
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
        ui.onNodeWithTag("responsive_row_open").assertIsNotEnabled()
        ui.onNodeWithTag("responsive_row_open").performTouchInput {click()}
        assertEquals(0,openCount)
        ui.runOnIdle {shown=original}
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

    private fun mixedInventory():HostSnapshotResult {
        val base=inventory();val graph=base.hierarchy.single()
        val mixed=ShellSummary(ShellTarget(Uuid.random(),Uuid.random()),graph.serverId,Id("workspace"),Id("tab"),Id("pane-shell"),Id("terminal-shell"),
            Label("Ordinary shell"),SessionConnectivity.Ready,ShellCapabilities(true,false,true,false,grid=true))
        val solo=mixed.copy(target=ShellTarget(Uuid.random(),Uuid.random()),workspaceId=Id("shell-only-workspace"),tabId=Id("shell-only-tab"),
            paneId=Id("shell-only-pane"),terminalId=Id("shell-only-terminal"),title=Label("Shell-only workspace terminal"))
        return base.copy(shells=listOf(mixed,solo),hierarchy=listOf(graph.copy(
            workspaces=graph.workspaces+NativeWorkspace(solo.workspaceId,Uint(8),Label("Shell-only workspace"),false,solo.tabId,NativeAgentStatus.Idle),
            tabs=graph.tabs+NativeTab(solo.tabId,solo.workspaceId,Uint(4),Label("Shell tab"),false,NativeAgentStatus.Idle),
            panes=graph.panes.map {if(it.paneId==mixed.paneId)it.copy(agent=null,shellTarget=mixed.target)else it}+
                NativePane(solo.paneId,solo.terminalId,solo.workspaceId,solo.tabId,focused=false,agentStatus=NativeAgentStatus.Idle,shellTarget=solo.target)))).also {it.validate()}
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
                        SessionBrowser(host,snapshot,ready,SessionListPreferences(),"Same visible desktop",query,{query=it},{fail("This inventory has no bound ShellTarget")}) {opened=it;openedHost=host;openCount++}
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
            SessionBrowser("original-host",snapshot,true,SessionListPreferences(),null,query,{query=it},{fail("Setup-only panes never grant ShellTarget")}) {selected=it}
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
            CreateSession(current,hello,true,memory,{}) {_->fail("No project/current native source must not offer a create action")}
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
    @Test fun productionMixedDirectoryOpensShellOnlyRowsButNeverStaleOrChangedShellPresentation() {
        val original=mixedInventory();val host="display-host"
        var snapshot by mutableStateOf(original);var ready by mutableStateOf(true)
        val device=Uuid.random()
        val hello=HelloResult(EndpointId("523c7996bad77424e96786cf7a7205115337a5b4565cd25506a0f297b191a5ea"),Uuid.random(),Uuid.random(),ProtocolVersion(),emptyList())
        var opened:ShellTarget?=null;var opens=0
        ui.setContent {GoatrTheme {
            val directory=rememberSessionDirectory(HostMetadata(host,"Desktop",device),hello,snapshot,ready,snapshot.sessions)
            LazyColumn(Modifier.fillMaxSize()) {
                workspaceDirectoryItems(directory,host,"Desktop",SessionListPreferences(),!ready,false,
                    {fail("Both workspaces have actual terminals")},{fail("Shell selection must not become an agent Target")},{opened=it;opens++})
            }
        }}
        val groups=projectSessionDirectory(host,original,true,original.sessions).workspaces
        groups.forEach {ui.onNodeWithTag("${it.key.tag}_agents").performScrollTo().performClick()}
        val shell=original.shells.last();val tag="shell_${host}_${shell.target.shellId.value}_${shell.target.generation.value}"
        ui.onNodeWithTag(tag).performScrollTo().assertIsEnabled().assertHeightIsAtLeast(48.dp).performTouchInput {click()}
        assertEquals(shell.target,opened);assertEquals(1,opens)
        ui.runOnIdle {ready=false}
        ui.onNodeWithTag(tag).performScrollTo().assertIsNotEnabled().performTouchInput {click()}
        assertEquals(1,opens)
        ui.runOnIdle {snapshot=snapshot.copy(shells=snapshot.shells.map {if(it.target==shell.target)it.copy(title=Label("Changed while pending"))else it})}
        ui.onNodeWithTag(tag).assertDoesNotExist()
        groups.forEach {ui.onNodeWithTag(it.key.tag).assertDoesNotExist()}
        assertEquals(shell.target,opened);assertEquals(1,opens)
    }

    @Test fun productionWorkspaceStripSeparatesTypesSelectionAndExactReplacementGenerations() {
        val original=mixedInventory();val agent=original.sessions.single().let {Target(it.sessionId,it.generation)}
        val shell=original.shells.first()
        var snapshot by mutableStateOf(original)
        var selectedAgent by mutableStateOf<Target?>(agent);var selectedShell by mutableStateOf<ShellTarget?>(null)
        var agentOpens=0;var shellOpens=0
        ui.setContent {GoatrTheme {
            val density=LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density,2f)) {
                WorkspaceDirectoryStrip("host",snapshot,true,selectedAgent,selectedShell,"mixed_strip",
                    {selectedAgent=it;selectedShell=null;agentOpens++},{selectedShell=it;selectedAgent=null;shellOpens++})
            }
        }}
        val agentTag="mixed_strip_agent_${agent.sessionId.value}_${agent.generation.value}"
        val shellTag="mixed_strip_shell_${shell.target.shellId.value}_${shell.target.generation.value}"
        ui.onNodeWithTag(agentTag).assertIsSelected().assertHeightIsAtLeast(48.dp)
        ui.onNodeWithTag(shellTag).performScrollTo().assertIsNotSelected().assertHeightIsAtLeast(48.dp).performTouchInput {click()}
        ui.onNodeWithTag(shellTag).assertIsSelected()
        assertEquals(shell.target,selectedShell);assertEquals(1,shellOpens);assertEquals(0,agentOpens)
        ui.onNodeWithTag(agentTag).performScrollTo().performTouchInput {click()}
        assertEquals(agent,selectedAgent);assertEquals(1,agentOpens)
        val replacement=shell.target.copy(generation=Uuid.random())
        ui.runOnIdle {snapshot=original.copy(shells=original.shells.map {if(it.target==shell.target)it.copy(target=replacement)else it},
            hierarchy=original.hierarchy.map {graph->graph.copy(panes=graph.panes.map {if(it.shellTarget==shell.target)it.copy(shellTarget=replacement)else it})})}
        ui.onNodeWithTag(shellTag).assertDoesNotExist()
        ui.onNodeWithTag("mixed_strip_shell_${replacement.shellId.value}_${replacement.generation.value}").performScrollTo().performTouchInput {click()}
        assertEquals(replacement,selectedShell);assertEquals(2,shellOpens);assertEquals(1,agentOpens)
    }
}

class TitleGoatAssetTest {
    @Test fun titleGoatKeepsTransparentGutterAroundCompleteForeground() {
        val resources=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext.resources
        val icon=checkNotNull(android.graphics.BitmapFactory.decodeResource(resources,R.drawable.ic_goatr_title,android.graphics.BitmapFactory.Options().apply {inScaled=false}))
        try {
            assertEquals(96,icon.width)
            assertEquals(96,icon.height)
            var left=icon.width;var top=icon.height;var right=-1;var bottom=-1
            for(y in 0 until icon.height) for(x in 0 until icon.width) {
                if(android.graphics.Color.alpha(icon.getPixel(x,y))!=0) {
                    left=minOf(left,x);top=minOf(top,y);right=maxOf(right,x);bottom=maxOf(bottom,y)
                }
            }
            assertTrue("Title goat must have visible foreground",right>=left && bottom>=top)
            assertTrue("Title goat needs at least 8px transparent gutter on every edge",left>=8 && top>=8 && right<icon.width-8 && bottom<icon.height-8)
        } finally {
            icon.recycle()
        }
    }
}
