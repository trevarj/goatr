// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import org.junit.Assert.*
import org.junit.Test

/** Presentation projection only; these fixtures supply no native/provider evidence. */
class SessionDirectoryTest {
    private fun fixture(): HostSnapshotResult {
        val server=Uuid.random()
        val target=Target(Uuid.random(),Uuid.random())
        val workspace=Id("workspace")
        val summary=SessionSummary(target.sessionId,target.generation,server,workspace,Id("terminal"),Id("pane"),Provider.Omp,Id("native"),"Agent",SessionStatus.Working,SessionConnectivity.Ready,Capabilities(true,true,true,true,PromptMode.SubmitOrSteer,CancelMode.CurrentWork),HistoryCoverage(HistoryScope.AllBranches,HistorySource.PersistedAndLive,false,false,false))
        val graph=NativeHierarchy(server,Uuid.random(),NativeFreshness.Current,
            workspaces=listOf(NativeWorkspace(workspace,Uint(1),Label("Workspace"),false,Id("tab"),NativeAgentStatus.Working)),
            tabs=listOf(NativeTab(Id("tab"),workspace,Uint(1),Label("Tab"),false,NativeAgentStatus.Working)),
            panes=listOf(NativePane(Id("pane"),Id("terminal"),workspace,Id("tab"),agent=Label("omp"),focused=false,agentStatus=NativeAgentStatus.Working,target=target)))
        return HostSnapshotResult(Uuid.random(),ZeroValue(),listOf(Server(server,Label("Server"),ServerConnectivity.Online,CanCreate(false,false))),emptyList(),listOf(summary),emptyList(),listOf(graph))
    }

    private fun mixedFixture():HostSnapshotResult {
        val base=fixture();val graph=base.hierarchy.single();val agent=base.sessions.single()
        // Deliberate UUID overlap is not permission to coerce ShellTarget into Target.
        val mixed=ShellSummary(ShellTarget(agent.sessionId,agent.generation),graph.serverId,agent.workspaceId,Id("tab"),
            Id("shell-pane"),Id("shell-terminal"),Label("Shell"),SessionConnectivity.Ready,ShellCapabilities(true,false,true,false,grid=true))
        val solo=mixed.copy(target=ShellTarget(Uuid.random(),Uuid.random()),workspaceId=Id("shell-only"),tabId=Id("shell-only-tab"),
            paneId=Id("shell-only-pane"),terminalId=Id("shell-only-terminal"),title=Label("Only shell"))
        val panes=listOf(mixed,solo).map {NativePane(it.paneId,it.terminalId,it.workspaceId,it.tabId,
            title=it.title,focused=false,agentStatus=NativeAgentStatus.Idle,shellTarget=it.target)}
        return base.copy(shells=listOf(mixed,solo),hierarchy=listOf(graph.copy(
            workspaces=graph.workspaces+NativeWorkspace(solo.workspaceId,Uint(2),Label("Shell only"),false,solo.tabId,NativeAgentStatus.Idle),
            tabs=graph.tabs+NativeTab(solo.tabId,solo.workspaceId,Uint(2),Label("Shell"),false,NativeAgentStatus.Idle),panes=graph.panes+panes)))
    }

    @Test fun exactNativeChainGroupsAgentsWithoutChangingTheirTarget() {
        val inventory=fixture()
        val row=inventory.sessions.single()
        val projected=projectSessionDirectory("host",inventory,true,inventory.sessions)
        assertEquals(listOf(row),projected.workspaces.single().agents)
        assertTrue(projected.standalone.isEmpty())
        assertEquals(Target(row.sessionId,row.generation),projected.workspaces.single().agents.single().let {Target(it.sessionId,it.generation)})
    }

    @Test fun workspaceKeysSeparateHostsServersAndIncarnations() {
        val inventory=fixture()
        fun key(host:String,snapshot:HostSnapshotResult)=projectSessionDirectory(host,snapshot,true,snapshot.sessions).workspaces.single().key
        val original=key("host-a",inventory)
        assertNotEquals(original,key("host-b",inventory))
        assertNotEquals(original.lazyKey,key("host-b",inventory).lazyKey)
        assertEquals("${original.host.length}:${original.host}${original.server.value.length}:${original.server.value}${original.incarnation.value.length}:${original.incarnation.value}${original.workspace.value.length}:${original.workspace.value}",original.lazyKey)
        assertNotEquals(original,key("host-a",inventory.copy(hierarchy=inventory.hierarchy.map {it.copy(serverIncarnation=Uuid.random())})))
        val server=Uuid.random()
        assertNotEquals(original,key("host-a",inventory.copy(servers=inventory.servers.map {it.copy(serverId=server)},sessions=inventory.sessions.map {it.copy(serverId=server)},hierarchy=inventory.hierarchy.map {it.copy(serverId=server)})))
    }

    @Test fun routeHintsNeverReplaceMissingStaleAmbiguousOrChangedNativeAssociations() {
        val inventory=fixture()
        val graph=inventory.hierarchy.single()
        val pane=graph.panes.single()
        val changedGraphs=listOf(
            graph.copy(panes=emptyList()),
            graph.copy(freshness=NativeFreshness.Stale),
            graph.copy(panes=listOf(pane,pane.copy(paneId=Id("second-pane")))),
            graph.copy(panes=listOf(pane.copy(target=pane.target!!.copy(generation=Uuid.random())))),
            graph.copy(panes=listOf(pane.copy(terminalId=Id("different-terminal")))),
            graph.copy(tabs=graph.tabs.map {it.copy(workspaceId=Id("different-workspace"))}),
        )
        changedGraphs.forEach {changed->
            val projected=projectSessionDirectory("host",inventory.copy(hierarchy=listOf(changed)),true,inventory.sessions)
            assertTrue(projected.workspaces.all {it.agents.isEmpty()})
            assertEquals(inventory.sessions,projected.standalone)
        }
        assertEquals(inventory.sessions,projectSessionDirectory("host",inventory,false,inventory.sessions).standalone)
        assertEquals(inventory.sessions,projectSessionDirectory("host",null,false,inventory.sessions).standalone)
        assertEquals(inventory.sessions,projectSessionDirectory("host",inventory.copy(hierarchy=listOf(graph,graph.copy(serverIncarnation=Uuid.random()))),true,inventory.sessions).standalone)
    }

    @Test fun pendingReplacementRetainsOnlyIdenticalAcceptedPresentation() {
        val original=fixture()
        val accepted=presentSessionDirectory(null,"host",original,true,original.sessions)
        val pending=presentSessionDirectory(accepted,"host",original,false,original.sessions)
        assertSame(accepted,pending)
        val replacement=original.copy(subscription=Uuid.random())
        assertSame(accepted,presentSessionDirectory(pending,"host",replacement,true,replacement.sessions))
        assertEquals(original.sessions,projectSessionDirectory("host",original,false,original.sessions).standalone)
        assertTrue(presentSessionDirectory(accepted,"other-host",original,false,original.sessions).directory.workspaces.isEmpty())

        val graph=original.hierarchy.single()
        val changed=listOf(
            original.copy(sessions=emptyList()),
            original.copy(sessions=original.sessions.map {it.copy(generation=Uuid.random())}),
            original.copy(sessions=original.sessions.map {it.copy(title="Renamed immediately")}),
            original.copy(sessions=original.sessions.map {it.copy(capabilities=it.capabilities.copy(prompt=false))}),
            original.copy(hierarchy=listOf(graph.copy(serverIncarnation=Uuid.random()))),
            original.copy(hierarchy=listOf(graph.copy(freshness=NativeFreshness.Unavailable,reason=NativeUnavailableReason.SourceUnavailable,workspaces=emptyList(),tabs=emptyList(),panes=emptyList()))),
            original.copy(servers=original.servers.map {it.copy(connectivity=ServerConnectivity.Offline)}),
        )
        for(snapshot in changed) {
            val withheld=presentSessionDirectory(accepted,"host",snapshot,false,snapshot.sessions)
            assertNull(withheld.inventory)
            assertTrue(withheld.directory.workspaces.isEmpty())
            assertEquals(snapshot.sessions,withheld.directory.standalone)
            val authoritative=presentSessionDirectory(accepted,"host",snapshot,true,snapshot.sessions)
            assertEquals(projectSessionDirectory("host",snapshot,true,snapshot.sessions),authoritative.directory)
        }
        val missing=presentSessionDirectory(accepted,"host",null,false,emptyList())
        assertTrue(missing.directory.workspaces.isEmpty())
        assertTrue(missing.directory.standalone.isEmpty())
        assertTrue(presentSessionDirectory(missing,"host",original,false,original.sessions).directory.workspaces.isEmpty())
        assertTrue(presentSessionDirectory(null,"host",original,false,original.sessions).directory.workspaces.isEmpty())
    }

    @Test fun narrowedDirectoryDoesNotInventAnEmptyWorkspaceMatch() {
        val inventory=fixture()
        assertEquals(1,projectSessionDirectory("host",inventory,true,emptyList()).workspaces.size)
        assertTrue(projectSessionDirectory("host",inventory,true,emptyList(),narrowed=true).workspaces.isEmpty())
        assertEquals(inventory.sessions,projectSessionDirectory("host",inventory,true,inventory.sessions,narrowed=true).workspaces.single().agents)
    }

    @Test fun mixedAndShellOnlyWorkspacesKeepDistinctTargetsAndExactNativeChains() {
        val inventory=mixedFixture()
        val directory=projectSessionDirectory("host",inventory,true,inventory.sessions)
        assertEquals(2,directory.workspaces.size)
        val mixed=directory.workspaces.single {it.workspace.workspaceId==inventory.sessions.single().workspaceId}
        assertEquals(inventory.sessions,mixed.agents)
        assertEquals(listOf(inventory.shells.first()),mixed.shells)
        val solo=directory.workspaces.single {it.workspace.workspaceId==Id("shell-only")}
        assertTrue(solo.agents.isEmpty());assertEquals(listOf(inventory.shells.last()),solo.shells)
        assertTrue(directory.standalone.isEmpty())
        val narrowed=projectSessionDirectory("host",inventory,true,emptyList(),narrowed=true)
        assertEquals(inventory.shells.toSet(),narrowed.workspaces.flatMap {it.shells}.toSet())
    }

    @Test fun shellLabelsNeverRepairMissingAmbiguousGenerationOrPaneAssociations() {
        val inventory=mixedFixture();val graph=inventory.hierarchy.single()
        val shell=inventory.shells.first();val pane=graph.panes.single {it.shellTarget==shell.target}
        val changes=listOf(
            graph.copy(panes=graph.panes-pane),
            graph.copy(panes=graph.panes+pane.copy(paneId=Id("alias"))),
            graph.copy(panes=graph.panes.map {if(it==pane)it.copy(shellTarget=shell.target.copy(generation=Uuid.random()))else it}),
            graph.copy(panes=graph.panes.map {if(it==pane)it.copy(terminalId=Id("wrong"))else it}),
            graph.copy(panes=graph.panes.map {if(it==pane)it.copy(tabId=Id("missing-tab"))else it}),
            graph.copy(tabs=graph.tabs.map {if(it.tabId==shell.tabId)it.copy(workspaceId=Id("wrong-workspace"))else it}),
            graph.copy(freshness=NativeFreshness.Stale),
        )
        changes.forEach {changed->
            val directory=projectSessionDirectory("host",inventory.copy(hierarchy=listOf(changed)),true,inventory.sessions)
            assertTrue(directory.workspaces.none {shell in it.shells})
        }
        assertTrue(projectSessionDirectory("host",inventory,false,inventory.sessions).workspaces.isEmpty())
        assertTrue(projectSessionDirectory("host",inventory.copy(hierarchy=listOf(graph,graph)),true,inventory.sessions).workspaces.isEmpty())
    }

    @Test fun shellOnlyChangesRetireAcceptedDisplayBeforeReplacementBecomesReady() {
        val original=mixedFixture()
        val accepted=presentSessionDirectory(null,"host",original,true,original.sessions)
        assertSame(accepted,presentSessionDirectory(accepted,"host",original.copy(subscription=Uuid.random()),false,original.sessions))
        val shell=original.shells.last()
        val changes=listOf(
            emptyList(),
            listOf(shell.copy(title=Label("Changed shell only"))),
            listOf(shell.copy(target=shell.target.copy(generation=Uuid.random()))),
            listOf(shell.copy(connectivity=SessionConnectivity.Offline,capabilities=ShellCapabilities(false,false,false,false))),
            listOf(shell.copy(paneId=Id("replacement-pane"))),
        )
        changes.forEach {changed->
            val next=original.copy(shells=listOf(original.shells.first())+changed)
            val pending=presentSessionDirectory(accepted,"host",next,false,next.sessions)
            assertNull(pending.inventory);assertTrue(pending.directory.workspaces.isEmpty())
            val ready=presentSessionDirectory(accepted,"host",next,true,next.sessions)
            assertEquals(projectSessionDirectory("host",next,true,next.sessions),ready.directory)
        }
    }

    @Test fun recentOrderingUsesOnlyExactHostAndGenerationAndKeepsRemainderStable() {
        val inventory=fixture()
        val a=inventory.sessions.single()
        val b=a.copy(sessionId=Uuid.random(),title="B")
        val c=a.copy(sessionId=Uuid.random(),title="C")
        val rows=listOf(a,b,c)
        val bKey=ConversationKey("host",Target(b.sessionId,b.generation))
        assertEquals(listOf(b,a,c),orderRecentlyOpened("host",rows,listOf(bKey)))
        assertEquals(rows,orderRecentlyOpened("other-host",rows,listOf(bKey)))
        assertEquals(rows,orderRecentlyOpened("host",rows,listOf(bKey.copy(target=bKey.target.copy(generation=Uuid.random())))))
    }

    @Test fun localOpenHistoryIsBoundedAndPromotesOnlyTheExplicitExactKey() {
        val keys=(1..70).map {ConversationKey("host",Target(Uuid.random(),Uuid.random()))}
        val opened=keys.fold(emptyList<ConversationKey>()) {prior,key->rememberExplicitSessionOpen(prior,key)}
        assertEquals(64,opened.size)
        assertEquals(keys.last(),opened.first())
        assertFalse(opened.contains(keys.first()))
        val promoted=rememberExplicitSessionOpen(opened,opened.last())
        assertEquals(opened.last(),promoted.first())
        assertEquals(64,promoted.distinct().size)
    }
}
