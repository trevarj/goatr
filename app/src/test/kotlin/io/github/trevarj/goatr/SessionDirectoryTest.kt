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

    @Test fun narrowedDirectoryDoesNotInventAnEmptyWorkspaceMatch() {
        val inventory=fixture()
        assertEquals(1,projectSessionDirectory("host",inventory,true,emptyList()).workspaces.size)
        assertTrue(projectSessionDirectory("host",inventory,true,emptyList(),narrowed=true).workspaces.isEmpty())
        assertEquals(inventory.sessions,projectSessionDirectory("host",inventory,true,inventory.sessions,narrowed=true).workspaces.single().agents)
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
