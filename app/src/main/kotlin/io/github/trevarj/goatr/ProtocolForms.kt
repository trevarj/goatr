// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.serialization.*
import kotlinx.serialization.json.*

internal fun decodeRequestBody(method: Method, params: JsonElement): RequestBody = when(method) {
    Method.SessionControlsRead->RequestBody.SessionControlsRead(Protocol.typed(params,true))
    Method.SessionControlsApply->RequestBody.SessionControlsApply(Protocol.typed(params,true))
    Method.SessionQueueList->RequestBody.SessionQueueList(Protocol.typed(params,true))
    Method.SessionQueueUpdate->RequestBody.SessionQueueUpdate(Protocol.typed(params,true))
    Method.SessionQueueDelete->RequestBody.SessionQueueDelete(Protocol.typed(params,true))
    Method.SessionQueueReorder->RequestBody.SessionQueueReorder(Protocol.typed(params,true))
    Method.SessionQueueStart->RequestBody.SessionQueueStart(Protocol.typed(params,true))
    Method.ArtifactsList -> RequestBody.ArtifactsList(Protocol.typed(params,true))
    Method.ArtifactsOpen -> RequestBody.ArtifactsOpen(Protocol.typed(params,true))
    Method.ArtifactsRead -> RequestBody.ArtifactsRead(Protocol.typed(params,true))
    Method.ArtifactsClose -> RequestBody.ArtifactsClose(Protocol.typed(params,true))
    Method.TerminalCreate->RequestBody.TerminalCreate(Protocol.typed(params,true))
    Method.AttachmentsBegin->RequestBody.AttachmentsBegin(Protocol.typed(params,true))
    Method.AttachmentsChunk->RequestBody.AttachmentsChunk(Protocol.typed(params,true))
    Method.AttachmentsComplete->RequestBody.AttachmentsComplete(Protocol.typed(params,true))
    Method.AttachmentsDiscard->RequestBody.AttachmentsDiscard(Protocol.typed(params,true))
    Method.AttachmentsStatus->RequestBody.AttachmentsStatus(Protocol.typed(params,true))
    Method.PairConfirm -> RequestBody.PairConfirm(Protocol.typed(params, true))
    Method.Hello -> RequestBody.Hello(Protocol.typed(params, true))
    Method.HostSnapshot -> RequestBody.HostSnapshot(Protocol.typed(params, true))
    Method.SessionOpen -> RequestBody.SessionOpen(Protocol.typed(params, true))
    Method.SessionHistory -> RequestBody.SessionHistory(Protocol.typed(params, true))
    Method.SessionUnsubscribe -> RequestBody.SessionUnsubscribe(Protocol.typed(params, true))
    Method.CreationDirectories -> RequestBody.CreationDirectories(Protocol.typed(params,true))
    Method.SessionCreate -> RequestBody.SessionCreate(Protocol.typed(params, true))
    Method.AgentRename -> RequestBody.AgentRename(Protocol.typed(params, true))
    Method.WorkspaceRename -> RequestBody.WorkspaceRename(Protocol.typed(params, true))
    Method.PromptSubmit -> RequestBody.PromptSubmit(Protocol.typed(params, true))
    Method.TurnCancel -> RequestBody.TurnCancel(Protocol.typed(params, true))
    Method.RequestRespond -> RequestBody.RequestRespond(Protocol.typed(params, true))
    Method.ActionStatus -> RequestBody.ActionStatus(Protocol.typed(params, true))
    Method.ContentRead -> RequestBody.ContentRead(Protocol.typed(params, true))
    Method.TerminalGridSubscribe -> RequestBody.TerminalGridSubscribe(Protocol.typed(params, true))
    Method.ShellGridSubscribe -> RequestBody.ShellGridSubscribe(Protocol.typed(params, true))
    Method.ShellInput -> RequestBody.ShellInput(Protocol.typed(params, true))
    Method.ShellResize->RequestBody.ShellResize(Protocol.typed(params,true))
    Method.SkillsList -> RequestBody.SkillsList(Protocol.typed(params, true))
    Method.SkillsPreviewOpen->RequestBody.SkillsPreviewOpen(Protocol.typed(params,true))
    Method.SkillsPreviewRead->RequestBody.SkillsPreviewRead(Protocol.typed(params,true))
    Method.SkillsPreviewClose->RequestBody.SkillsPreviewClose(Protocol.typed(params,true))

    Method.WorktreesList -> RequestBody.WorktreesList(Protocol.typed(params, true))
    Method.ChangesRead -> RequestBody.ChangesRead(Protocol.typed(params, true))
    Method.NotificationsSync -> RequestBody.NotificationsSync(Protocol.typed(params, true))
    Method.NotificationsRead -> RequestBody.NotificationsRead(Protocol.typed(params, true))
    Method.PushRegister -> RequestBody.PushRegister(Protocol.typed(params, true))
    Method.PushProbeAck -> RequestBody.PushProbeAck(Protocol.typed(params, true))
    Method.PushUnregister -> RequestBody.PushUnregister(Protocol.typed(params, true))
    Method.DeviceUnpair -> RequestBody.DeviceUnpair(Protocol.typed(params, true))
}
internal fun decodeResultBody(method: Method, result: JsonElement): ResultBody = when(method) {
    Method.SessionControlsRead->ResultBody.SessionControlsRead(Protocol.typed(result,false))
    Method.SessionControlsApply->ResultBody.SessionControlsApply(Protocol.typed(result,false))
    Method.SessionQueueList->ResultBody.SessionQueueList(Protocol.typed(result,false))
    Method.SessionQueueUpdate->ResultBody.SessionQueueUpdate(Protocol.typed(result,false))
    Method.SessionQueueDelete->ResultBody.SessionQueueDelete(Protocol.typed(result,false))
    Method.SessionQueueReorder->ResultBody.SessionQueueReorder(Protocol.typed(result,false))
    Method.SessionQueueStart->ResultBody.SessionQueueStart(Protocol.typed(result,false))
    Method.ArtifactsList -> ResultBody.ArtifactsList(Protocol.typed(result,false))
    Method.ArtifactsOpen -> ResultBody.ArtifactsOpen(Protocol.typed(result,false))
    Method.ArtifactsRead -> ResultBody.ArtifactsRead(Protocol.typed(result,false))
    Method.ArtifactsClose -> ResultBody.ArtifactsClose(Protocol.typed(result,false))
    Method.TerminalCreate->ResultBody.TerminalCreate(Protocol.typed(result,false))
    Method.AttachmentsBegin->ResultBody.AttachmentsBegin(Protocol.typed(result,false))
    Method.AttachmentsChunk->ResultBody.AttachmentsChunk(Protocol.typed(result,false))
    Method.AttachmentsComplete->ResultBody.AttachmentsComplete(Protocol.typed(result,false))
    Method.AttachmentsDiscard->ResultBody.AttachmentsDiscard(Protocol.typed(result,false))
    Method.AttachmentsStatus->ResultBody.AttachmentsStatus(Protocol.typed(result,false))
    Method.PairConfirm -> ResultBody.PairConfirm(Protocol.typed(result, false))
    Method.Hello -> ResultBody.Hello(Protocol.typed(result, false))
    Method.HostSnapshot -> ResultBody.HostSnapshot(Protocol.typed(result, false))
    Method.SessionOpen -> ResultBody.SessionOpen(Protocol.typed(result, false))
    Method.SessionHistory -> ResultBody.SessionHistory(Protocol.typed(result, false))
    Method.SessionUnsubscribe -> ResultBody.SessionUnsubscribe(Protocol.typed(result, false))
    Method.CreationDirectories -> ResultBody.CreationDirectories(Protocol.typed(result,false))
    Method.SessionCreate -> ResultBody.SessionCreate(Protocol.typed(result, false))
    Method.AgentRename -> ResultBody.AgentRename(Protocol.typed(result, false))
    Method.WorkspaceRename -> ResultBody.WorkspaceRename(Protocol.typed(result, false))
    Method.PromptSubmit -> ResultBody.PromptSubmit(Protocol.typed(result, false))
    Method.TurnCancel -> ResultBody.TurnCancel(Protocol.typed(result, false))
    Method.RequestRespond -> ResultBody.RequestRespond(Protocol.typed(result, false))
    Method.ActionStatus -> { val o = result.jsonObject; requireWire(("receipt" in o) != ("state" in o)); ResultBody.ActionStatus(if ("receipt" in o) ActionStatusResult.Known(Protocol.typed(result, false)) else ActionStatusResult.Unknown(Protocol.typed(result, false))) }
    Method.ContentRead -> ResultBody.ContentRead(Protocol.typed(result, false))
    Method.TerminalGridSubscribe -> ResultBody.TerminalGridSubscribe(Protocol.typed(result, false))
    Method.ShellGridSubscribe -> ResultBody.ShellGridSubscribe(Protocol.typed(result, false))
    Method.ShellInput -> ResultBody.ShellInput(Protocol.typed(result, false))
    Method.ShellResize->ResultBody.ShellResize(Protocol.typed(result,false))
    Method.SkillsList -> { requireWire(result.toString().toByteArray(Charsets.UTF_8).size<=256*1024); ResultBody.SkillsList(Protocol.typed(result, false)) }
    Method.SkillsPreviewOpen->ResultBody.SkillsPreviewOpen(Protocol.typed(result,false))
    Method.SkillsPreviewRead->ResultBody.SkillsPreviewRead(Protocol.typed(result,false))
    Method.SkillsPreviewClose->ResultBody.SkillsPreviewClose(Protocol.typed(result,false))

    Method.WorktreesList -> ResultBody.WorktreesList(Protocol.typed(result, false))
    Method.ChangesRead -> { validateChangesReadResult(result); ResultBody.ChangesRead(Protocol.typed(result, false)) }
    Method.NotificationsSync -> ResultBody.NotificationsSync(Protocol.typed(result, false))
    Method.NotificationsRead -> ResultBody.NotificationsRead(Protocol.typed(result, false))
    Method.PushRegister -> ResultBody.PushRegister(Protocol.typed(result, false))
    Method.PushProbeAck -> ResultBody.PushProbeAck(Protocol.typed(result, false))
    Method.PushUnregister -> ResultBody.PushUnregister(Protocol.typed(result, false))
    Method.DeviceUnpair -> ResultBody.DeviceUnpair(Protocol.typed(result, false))
}

/** Closed Changes variants reject fields from another variant before tolerant response decoding. */
private fun validateChangesReadResult(result: JsonElement) {
    val outer=result.jsonObject
    val changes=outer.getValue("changes").jsonObject
    changes.getValue("entries").jsonArray.forEach { element ->
        val entry=element.jsonObject
        when(entry.getValue("kind").jsonPrimitive.content) {
            "tracked" -> {
                exactChangesKeys(entry,setOf("kind","path","index","worktree","stagedBinary","unstagedBinary"),setOf("kind","path","originalPath","index","worktree","submodule","stagedBinary","unstagedBinary"))
                val renamed=listOf("index","worktree").any { entry.getValue(it).jsonPrimitive.content in setOf("renamed","copied") }
                requireWire(renamed == ("originalPath" in entry))
            }
            "untracked" -> exactChangesKeys(entry,setOf("kind","path"),setOf("kind","path"))
            else -> invalidWire()
        }
    }
}

private fun exactChangesKeys(value: JsonObject, required: Set<String>, allowed: Set<String>) {
    requireWire(required.all { it in value } && value.keys.all { it in allowed })
}
internal fun decodeEventBody(kind: String, data: JsonElement): EventBody = when(kind) {
    "controls.changed"->EventBody.ControlsChanged(Protocol.typed(data,true))
    "queue.changed"->EventBody.QueueChanged(Protocol.typed(data,true))
    "grid.begin" -> EventBody.GridBegin(Protocol.typed(data,true))
    "grid.part" -> EventBody.GridPart(Protocol.gridPart(data))
    "grid.commit" -> EventBody.GridCommit(Protocol.typed(data,true))
    "host.changed" -> EventBody.HostChanged(Protocol.typed(data, true))
    "session.state" -> EventBody.SessionState(Protocol.typed(data, true))
    "history.upsert" -> EventBody.HistoryUpsert(Protocol.typed(data, true))
    "activity.update" -> EventBody.ActivityUpdate(Protocol.typed(data, true))
    "request.opened" -> EventBody.RequestOpened(Protocol.typed(data, true))
    "request.closed" -> EventBody.RequestClosed(Protocol.typed(data, true))
    "action.status" -> EventBody.ActionStatus(Protocol.typed(data, true))
    "notification.upsert" -> EventBody.NotificationUpsert(Protocol.typed(data, true))
    "notification.read" -> EventBody.NotificationRead(Protocol.typed(data, true))
    "sync.required" -> EventBody.SyncRequired(Protocol.typed(data, true))
    else -> invalidWire()
}
internal fun requestJson(body: RequestBody): JsonObject {
    val params = when(body) {
        is RequestBody.SessionControlsRead->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionControlsApply->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionQueueList->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionQueueUpdate->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionQueueDelete->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionQueueReorder->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionQueueStart->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.PairConfirm -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.Hello -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.HostSnapshot -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.SessionOpen -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.SessionHistory -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.SessionUnsubscribe -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.CreationDirectories -> {body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.SessionCreate -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.ArtifactsList -> {body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.ArtifactsOpen -> {body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.ArtifactsRead -> {body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.ArtifactsClose -> {body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.TerminalCreate->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.AttachmentsBegin->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.AttachmentsChunk->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.AttachmentsComplete->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.AttachmentsDiscard->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.AttachmentsStatus->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
        is RequestBody.AgentRename -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.WorkspaceRename -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.PromptSubmit -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.TurnCancel -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.RequestRespond -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.ActionStatus -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.ContentRead -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.TerminalGridSubscribe -> {body.params.validate();buildJsonObject {
            put("target",Protocol.strictJson.encodeToJsonElement(body.params.target))
            if(body.params.ansiObserver)put("ansiObserver",JsonPrimitive(true))
        }}
        is RequestBody.ShellGridSubscribe -> {body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.ShellInput -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
    is RequestBody.ShellResize->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.SkillsList -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
    is RequestBody.SkillsPreviewOpen->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.SkillsPreviewRead->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}
    is RequestBody.SkillsPreviewClose->{body.params.validate();Protocol.strictJson.encodeToJsonElement(body.params)}

    is RequestBody.WorktreesList -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
    is RequestBody.ChangesRead -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.NotificationsSync -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.NotificationsRead -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.PushRegister -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.PushProbeAck -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.PushUnregister -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
        is RequestBody.DeviceUnpair -> { body.params.validate(); Protocol.strictJson.encodeToJsonElement(body.params) }
    }
    return buildJsonObject { put("method", Protocol.strictJson.encodeToJsonElement(body.method)); put("params", params) }
}

private fun unique(ids: List<Any>) { requireWire(ids.distinct().size == ids.size) }
private fun countOptions(counts: List<Int>) { requireWire(counts.all { it <= 256 } && counts.sum() <= 1024) }
internal fun validateContentRef(x: ContentRef) { requireWire(x.byteLength.value <= Protocol.CONTENT_MAX) }
internal fun validatePendingRequest(x: PendingRequest) {
    val kind = when (x.presentation) { is PresentationOmpSelect -> PresentationKind.OmpSelect; is PresentationOmpEditor -> PresentationKind.OmpEditor; is PresentationCodexApproval -> PresentationKind.CodexApproval; is PresentationCodexQuestions -> PresentationKind.CodexQuestions; is PresentationCodexForm -> PresentationKind.CodexForm; is PresentationCodexUrl -> PresentationKind.CodexUrl; is PresentationDesktopOnly -> PresentationKind.DesktopOnly }
    requireWire(kind == x.kind && (kind == PresentationKind.DesktopOnly || (x.provider == Provider.Omp) == (kind == PresentationKind.OmpSelect || kind == PresentationKind.OmpEditor)))
}
internal fun validateQuestion(x: Question) { countOptions(listOf(x.options.size)) }
internal fun validatePresentationOmpSelect(x: PresentationOmpSelect) { countOptions(listOf(x.options.size)); requireWire((x.selected == null || x.selected.value < x.options.size) && x.checked.all { it.value < x.options.size } && (x.markableCount == null || x.markableCount.value <= x.options.size)) }
internal fun validatePresentationCodexApproval(x: PresentationCodexApproval) { requireWire(x.choices.isNotEmpty()); countOptions(listOf(x.choices.size)); unique(x.choices.map { it.choiceId }) }
internal fun validatePresentationCodexQuestions(x: PresentationCodexQuestions) { requireWire(x.questions.size <= 64); unique(x.questions.map { it.id }); countOptions(x.questions.map { it.options.size }) }
internal fun validatePresentationCodexForm(x: PresentationCodexForm) { requireWire(x.fields.size <= 64); unique(x.fields.map { it.id }); countOptions(x.fields.map { it.options?.size ?: 0 }) }
internal fun validatePresentationCodexUrl(x: PresentationCodexUrl) { requireWire(validHttpUrl(x.url)) }
internal fun validateAnswerOmpSelect(x: AnswerOmpSelect) { requireWire((x.action == SubmitAction.Submit) == (x.value != null)); requireWire(Protocol.canonicalJsonSize(Protocol.strictJson.encodeToJsonElement<Answer>(x)) <= Protocol.ANSWER_MAX) }
internal fun validateAnswerOmpEditor(x: AnswerOmpEditor) { requireWire((x.action == SubmitAction.Submit) == (x.text != null)) }
internal fun validateAnswerCodexQuestions(x: AnswerCodexQuestions) { requireWire(x.answers.size <= 64); unique(x.answers.map { it.id }); requireWire(Protocol.canonicalJsonSize(Protocol.strictJson.encodeToJsonElement<Answer>(x)) <= Protocol.ANSWER_MAX) }
internal fun validateAnswerCodexForm(x: AnswerCodexForm) { requireWire((x.action == FormAction.Accept) == (x.values != null)); x.values?.let { requireWire(it.size <= 64); unique(it.map { v -> v.id }) }; requireWire(Protocol.canonicalJsonSize(Protocol.strictJson.encodeToJsonElement<Answer>(x)) <= Protocol.ANSWER_MAX) }
internal fun validateSessionHistoryParams(x: SessionHistoryParams) { requireWire((x.cursor == null) == (x.viewId == null)); requireWire(x.limit == null || x.limit.value in 1..100) }
internal fun validateContentReadParams(x: ContentReadParams) { requireWire(x.limitBytes.value in 1..65536 && x.offsetBytes.value <= Protocol.CONTENT_MAX) }
internal fun validateNotificationsSyncParams(x: NotificationsSyncParams) { requireWire(x.limit == null || x.limit.value in 1..100) }
internal fun validateHelloResult(x: HelloResult) { requireWire(x.backends.size <= 2); unique(x.backends.map { it.provider }) }
internal fun validateNativeHierarchy(x: NativeHierarchy) {
    requireWire(x.workspaces.size + x.tabs.size + x.panes.size <= 512)
    requireWire(when(x.freshness) {
        NativeFreshness.Current -> x.reason == null
        NativeFreshness.Stale -> x.reason == NativeUnavailableReason.SourceUnavailable
        NativeFreshness.Unavailable -> x.reason != null && x.workspaces.isEmpty() && x.tabs.isEmpty() && x.panes.isEmpty()
    })
    unique(x.workspaces.map { it.workspaceId }); unique(x.tabs.map { it.tabId })
    unique(x.panes.map { it.paneId }); unique(x.panes.map { it.terminalId })
    val workspaces = x.workspaces.map { it.workspaceId }.toSet()
    val tabs = x.tabs.associate { it.tabId to it.workspaceId }
    requireWire(x.tabs.all { it.workspaceId in workspaces })
    requireWire(x.workspaces.all { it.activeTabId == null || tabs[it.activeTabId] == it.workspaceId })
    requireWire(x.panes.all { tabs[it.tabId] == it.workspaceId })
    requireWire(x.freshness == NativeFreshness.Current || x.panes.all { it.target == null && it.unboundRowId == null && it.shellTarget == null })
}
internal fun validateHostSnapshotResult(x: HostSnapshotResult) {
    requireWire(x.servers.size <= 64 && x.projects.size <= 128 && x.sessions.size + x.unbound.size + x.shells.size <= 512
        && x.hierarchy.size <= 64 && x.hierarchy.sumOf { it.workspaces.size + it.tabs.size + it.panes.size } <= 512)
    unique(x.servers.map { it.serverId }); unique(x.sessions.map { it.sessionId })
    unique(x.shells.map { it.target.shellId })
    unique(x.unbound.map { it.rowId }); unique(x.hierarchy.map { it.serverId })
    requireWire(x.shells.all {shell->x.servers.any {it.serverId==shell.serverId} && (shell.connectivity!=SessionConnectivity.Ready || x.hierarchy.any {graph->graph.serverId==shell.serverId && graph.freshness==NativeFreshness.Current && graph.panes.any {pane->pane.shellTarget==shell.target && pane.workspaceId==shell.workspaceId && pane.tabId==shell.tabId && pane.paneId==shell.paneId && pane.terminalId==shell.terminalId}})})
    if(x.hierarchy.isEmpty()) return
    val servers = x.servers.map { it.serverId }.toSet()
    val sessions = x.sessions.associateBy { it.sessionId }
    val shells = x.shells.associateBy { it.target.shellId }
    val shellTargets = mutableSetOf<Uuid>()
    val unbound = x.unbound.associateBy { it.rowId }
    val targets = mutableSetOf<Uuid>()
    val rows = mutableSetOf<Uuid>()
    x.hierarchy.forEach { h ->
        requireWire(h.serverId in servers)
        h.panes.forEach { p ->
            val provider = when(p.agent?.value) { "omp" -> Provider.Omp; "codex" -> Provider.Codex; else -> null }
            p.target?.let { t ->
                val s = sessions[t.sessionId] ?: invalidWire()
                requireWire(targets.add(t.sessionId) && s.generation == t.generation && s.serverId == h.serverId
                    && s.workspaceId == p.workspaceId && s.terminalId == p.terminalId && s.paneId == p.paneId
                    && provider == s.provider && (s.connectivity == SessionConnectivity.Ready || s.connectivity == SessionConnectivity.Synchronizing))
            }
            p.shellTarget?.let { t ->
                val shell=shells[t.shellId] ?: invalidWire()
                requireWire(shellTargets.add(t.shellId) && shell.target==t && shell.serverId==h.serverId && shell.workspaceId==p.workspaceId && shell.tabId==p.tabId && shell.paneId==p.paneId && shell.terminalId==p.terminalId && shell.connectivity==SessionConnectivity.Ready && p.agent==null)
            }
            p.unboundRowId?.let { id ->
                val r = unbound[id] ?: invalidWire()
                requireWire(rows.add(id) && r.serverId == h.serverId && r.workspaceId == p.workspaceId
                    && r.terminalId == p.terminalId && r.paneId == p.paneId && provider == r.provider)
            }
        }
    }
}
internal fun validateSessionOpenResult(x: SessionOpenResult) { requireWire(x.pendingRequests.size <= 64 && x.history.size <= 100) }
internal fun validateSessionHistoryResult(x: SessionHistoryResult) { requireWire(x.records.size <= 100) }
internal fun validateNotificationsSyncResult(x: NotificationsSyncResult) { requireWire(x.events.size <= 100 && x.read.size <= 512 && x.retainedFrom.value <= x.latestSequence.value + 1); requireWire(x.events.zipWithNext().all { (a,b) -> a.sequence.value < b.sequence.value } && x.events.all { it.sequence.value <= x.latestSequence.value }); unique(x.read.map { it.sessionId }); if (x.nextSequence != null) requireWire(x.events.lastOrNull()?.sequence == x.nextSequence) }
internal fun validateHistoryUpsertEvent(x: HistoryUpsertEvent) { requireWire(x.records.isNotEmpty() && x.records.size <= 100) }
internal fun validateReceipt(x: Receipt) {
    requireWire(x.createdAt.value <= x.updatedAt.value)
    requireWire((x.method in listOf(ActionMethod.ShellCreate,ActionMethod.ShellInput,ActionMethod.ShellResize))==(x.shellTarget!=null))
    if(x.method==ActionMethod.ShellCreate){requireWire(x.target==null && x.creation==null);if(x.state==ReceiptState.Succeeded)requireWire(x.result is ActionResultShellCreation && x.result.resources.complete())}
    if(x.method==ActionMethod.ShellInput){requireWire(x.target==null && x.creation==null);if(x.state==ReceiptState.Succeeded)requireWire(x.result is ActionResultShellInput && x.result.progress.accepted())}
    if(x.method==ActionMethod.ShellResize){requireWire(x.target==null && x.creation==null);if(x.state==ReceiptState.Succeeded)requireWire(x.result is ActionResultShellResize)}
    if(x.method in setOf(ActionMethod.AttachmentsBegin,ActionMethod.AttachmentsChunk,ActionMethod.AttachmentsComplete,ActionMethod.AttachmentsDiscard)){requireWire(x.target!=null&&x.creation==null);if(x.result is ActionResultAttachment)requireWire(x.result.target==x.target);if(x.state==ReceiptState.Succeeded)requireWire(x.result is ActionResultAttachment)}
    // Provider-target historical terminal.create receipts remain readable.
    if(x.method==ActionMethod.TerminalCreate){requireWire(x.shellTarget==null&&x.creation==null);if(x.state==ReceiptState.Succeeded)requireWire((x.result as? ActionResultShellCreation)?.resources?.complete()==true)}
    if(x.method==ActionMethod.WorktreesRemove){requireWire(x.target!=null && x.creation==null);if(x.result is ActionResultWorktreeRemoval)requireWire(x.result.target==x.target);if(x.state==ReceiptState.Succeeded)requireWire(x.result is ActionResultWorktreeRemoval && x.result.progress.complete())}
    if(x.method==ActionMethod.AgentRename) {requireWire(x.target!=null && x.creation==null);if(x.result is ActionResultAgentRenamed)requireWire(x.result.target==x.target);if(x.state==ReceiptState.Succeeded) requireWire(x.result is ActionResultAgentRenamed && x.result.target==x.target)}
    if(x.method==ActionMethod.SessionControlsApply)requireWire(x.target!=null&&x.creation==null&&x.result==null)
    if(x.method in setOf(ActionMethod.SessionQueueUpdate,ActionMethod.SessionQueueDelete,ActionMethod.SessionQueueReorder,ActionMethod.SessionQueueStart)) {
        requireWire(x.target!=null&&x.creation==null)
        if(x.state==ReceiptState.Succeeded)requireWire(x.result is ActionResultQueue)
    }
    x.result?.let { result -> requireWire(when(x.method) {
        ActionMethod.SessionControlsApply->false
        ActionMethod.SessionQueueUpdate,ActionMethod.SessionQueueDelete,ActionMethod.SessionQueueReorder,ActionMethod.SessionQueueStart->result is ActionResultQueue
        ActionMethod.TerminalCreate->result is ActionResultShellCreation
        ActionMethod.AttachmentsBegin,ActionMethod.AttachmentsChunk,ActionMethod.AttachmentsComplete,ActionMethod.AttachmentsDiscard->result is ActionResultAttachment
        ActionMethod.SessionCreate->result is ActionResultCreated
        ActionMethod.ShellCreate->result is ActionResultShellCreation
        ActionMethod.ShellInput->result is ActionResultShellInput||result is ActionResultShellDispatch
        ActionMethod.ShellResize->result is ActionResultShellResize
        ActionMethod.WorktreesRemove->result is ActionResultWorktreeRemoval
        ActionMethod.AgentRename->result is ActionResultAgentRenamed
        ActionMethod.WorkspaceRename->result is ActionResultWorkspaceRenamed
        ActionMethod.PromptSubmit->result is ActionResultPrompt||result is ActionResultQueue
        ActionMethod.TurnCancel->result is ActionResultCancel
        ActionMethod.RequestRespond->result is ActionResultResponse
    })
        if(result is ActionResultQueue)requireWire(when(x.method) {
            ActionMethod.PromptSubmit,ActionMethod.SessionQueueUpdate->result.entryId!=null&&result.deleted==null&&result.nativeTurnId==null
            ActionMethod.SessionQueueDelete->result.entryId!=null&&result.nativeTurnId==null&&(x.state!=ReceiptState.Succeeded||result.deleted!=null)
            ActionMethod.SessionQueueStart->result.entryId!=null&&result.deleted==null&&(x.state!=ReceiptState.Succeeded||result.nativeTurnId!=null)
            ActionMethod.SessionQueueReorder->result.entryId==null&&result.deleted==null&&result.nativeTurnId==null
            else->false
        })
        if(result is ActionResultShellDispatch){requireWire(x.state!=ReceiptState.Succeeded);if(x.state==ReceiptState.Dispatched)requireWire(result.progress.complete())}
    }
    if (x.method == ActionMethod.SessionCreate && x.state == ReceiptState.Succeeded) requireWire(x.result is ActionResultCreated && x.creation?.stage == CreationStage.Ready && x.creation.stageState == StageState.Confirmed)
}
internal fun validateFormField(f: FormField) {
    val string = f.type == FieldType.String; val numeric = f.type == FieldType.Number || f.type == FieldType.Integer; val multi = f.type == FieldType.Multi; val choice = multi || f.type == FieldType.Single
    requireWire((string || (f.format == null && f.minLength == null && f.maxLength == null)) && (numeric || (f.minimum == null && f.maximum == null)) && (multi || (f.minItems == null && f.maxItems == null)) && choice == (f.options != null))
    if (f.minLength != null && f.maxLength != null) requireWire(f.minLength.value <= f.maxLength.value)
    if (f.minimum != null && f.maximum != null) requireWire(f.minimum <= f.maximum)
    if (f.minItems != null && f.maxItems != null) requireWire(f.minItems.value <= f.maxItems.value)
    f.options?.let { countOptions(listOf(it.size)) }
    f.`default`?.let { validateFieldValue(f, it) }
}
fun validateFieldValue(f: FormField, value: FieldValue) {
    when (f.type) {
        FieldType.String -> { val s = (value as? FieldValue.Text)?.value ?: invalidWire(); requireWire(validUnicode(s)); val n = s.codePointCount(0,s.length).toLong(); requireWire((f.minLength == null || n >= f.minLength.value) && (f.maxLength == null || n <= f.maxLength.value)); f.format?.let { requireWire(validFormat(it,s)) } }
        FieldType.Number, FieldType.Integer -> { val s = (value as? FieldValue.Text)?.value ?: invalidWire(); val n = Decimal.parse(s); requireWire(n.value == s && (f.type != FieldType.Integer || n.isInteger) && (f.minimum == null || n >= f.minimum) && (f.maximum == null || n <= f.maximum)) }
        FieldType.Boolean -> requireWire(value is FieldValue.Flag)
        FieldType.Single -> { val s = (value as? FieldValue.Text)?.value ?: invalidWire(); requireWire(f.options?.any { it.value == s } == true) }
        FieldType.Multi -> { val xs = (value as? FieldValue.Multi)?.values ?: invalidWire(); requireWire((f.minItems == null || xs.size >= f.minItems.value) && (f.maxItems == null || xs.size <= f.maxItems.value) && f.options?.let { os -> xs.all { s -> os.any { it.value == s } } } == true) }
    }
}
fun validateAnswer(presentation: Presentation, answer: Answer) {
    presentation.validate(); answer.validate()
    when {
        presentation is PresentationOmpSelect && answer is AnswerOmpSelect -> answer.value?.let { requireWire(presentation.options.any { o -> o.value == it }) }
        presentation is PresentationOmpEditor && answer is AnswerOmpEditor -> {}
        presentation is PresentationCodexApproval && answer is AnswerCodexApproval -> requireWire(presentation.choices.any { it.choiceId == answer.choiceId })
        presentation is PresentationCodexQuestions && answer is AnswerCodexQuestions -> { requireWire(presentation.questions.size == answer.answers.size); for (q in presentation.questions) { val a = answer.answers.find { it.id == q.id } ?: invalidWire(); requireWire(q.options.isEmpty() || q.isOther || a.values.all { s -> q.options.any { it.label.value == s } }) } }
        presentation is PresentationCodexForm && answer is AnswerCodexForm -> answer.values?.let { values -> for (v in values) validateFieldValue(presentation.fields.find { it.id == v.id } ?: invalidWire(),v.value); requireWire(presentation.fields.all { !it.required || values.any { v -> v.id == it.id } }) }
        presentation is PresentationCodexUrl && answer is AnswerCodexUrl -> {}
        else -> invalidWire()
    }
}

private fun ipv4(s: String, allowLeadingZeroes: Boolean): Boolean { val xs = s.split('.'); return xs.size == 4 && xs.all { it.isNotEmpty() && it.length <= 3 && it.all { c -> c in '0'..'9' } && (it.toIntOrNull() ?: 256) <= 255 && (allowLeadingZeroes || it.length == 1 || !it.startsWith('0')) } }
private fun ipv6(s: String, allowIpv4LeadingZeroes: Boolean): Boolean {
    val parts = s.split("::"); if (parts.size > 2) return false
    var groups = 0
    for ((i,part) in parts.withIndex()) { if (part.isEmpty()) continue; val xs = part.split(':'); for ((j,x) in xs.withIndex()) {
        if (x.contains('.')) { if (i != parts.lastIndex || j != xs.lastIndex || !ipv4(x,allowIpv4LeadingZeroes)) return false; groups += 2 }
        else { if (x.isEmpty() || x.length > 4 || !x.all { it in "0123456789abcdefABCDEF" }) return false; groups++ }
    } }
    return if (parts.size == 2) groups < 8 else groups == 8
}
private fun ldh(s: String) = s.isNotEmpty() && s.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '-' } && s.last().let { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' }
private fun validEmail(s: String): Boolean {
    if (s.any { it.code > 127 }) return false
    val local: String; val domain: String
    if (s.startsWith('"')) {
        var i = 1
        while (i < s.length) { if (s[i] == '"') break; if (s[i] == '\\') { i++; if (i == s.length || s[i].code !in 32..126) return false } else if (!(s[i].code == 32 || s[i].code == 33 || s[i].code in 35..91 || s[i].code in 93..126)) return false; i++ }
        if (i == s.length || s.getOrNull(i+1) != '@') return false
        local = s.substring(0,i+1); domain = s.substring(i+2)
    } else { val at = s.indexOf('@'); if (at < 0) return false; local = s.substring(0,at); domain = s.substring(at+1) }
    if (local.isEmpty() || domain.isEmpty()) return false
    if (!local.startsWith('"') && !local.split('.').all { a -> a.isNotEmpty() && a.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "!#$%&'*+-/=?^_`{|}~" } }) return false
    if (domain.startsWith('[') && domain.endsWith(']')) {
        val inner = domain.substring(1,domain.length-1)
        if (ipv4(inner,true)) return true
        if (inner.startsWith("IPv6:",ignoreCase = true)) return ipv6(inner.substring(5),true)
        val at = inner.indexOf(':'); if (at < 0) return false
        val tag = inner.substring(0,at); val content = inner.substring(at+1)
        return ldh(tag) && content.isNotEmpty() && content.all { it.code in 33..90 || it.code in 94..126 }
    }
    return domain.split('.').all { ldh(it) && it.first() != '-' }
}
private fun uriChars(s: String, extra: String): Boolean {
    var i = 0
    while (i < s.length) { if (s[i] == '%') { if (i+2 >= s.length || s[i+1] !in "0123456789abcdefABCDEF" || s[i+2] !in "0123456789abcdefABCDEF") return false; i += 3 }
        else { if (!(s[i] in 'a'..'z' || s[i] in 'A'..'Z' || s[i] in '0'..'9' || s[i] in "-._~!$&'()*+,;=" || s[i] in extra)) return false; i++ } }
    return true
}
private fun validAuthority(authority: String): Boolean {
    var host = authority
    val user = authority.lastIndexOf('@')
    if (user >= 0) { if (!uriChars(authority.substring(0,user),":")) return false; host = authority.substring(user+1) }
    if (host.startsWith('[')) {
        val end = host.indexOf(']'); if (end < 0) return false
        val literal = host.substring(1,end); val suffix = host.substring(end+1)
        if (suffix.isNotEmpty() && !(suffix.startsWith(':') && suffix.substring(1).all { it in '0'..'9' })) return false
        if (ipv6(literal,false)) return true
        if (literal.startsWith('v',true)) { val dot = literal.indexOf('.'); if (dot < 0) return false; val version = literal.substring(1,dot); val address = literal.substring(dot+1); return version.isNotEmpty() && version.all { it in "0123456789abcdefABCDEF" } && address.isNotEmpty() && !address.contains('%') && uriChars(address,":") }
        return false
    }
    val colon = host.lastIndexOf(':')
    if (colon >= 0) { if (!host.substring(colon+1).all { it in '0'..'9' }) return false; host = host.substring(0,colon) }
    return uriChars(host,"")
}
fun validUri(s: String): Boolean {
    if (s.any { it.code > 127 }) return false
    val colon = s.indexOf(':'); if (colon < 0) return false
    val scheme = s.substring(0,colon); var rest = s.substring(colon+1)
    if (scheme.isEmpty() || !(scheme.first() in 'a'..'z' || scheme.first() in 'A'..'Z') || !scheme.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "+-." }) return false
    val fragment = rest.indexOf('#')
    if (fragment >= 0) { if (!uriChars(rest.substring(fragment+1),":@/?")) return false; rest = rest.substring(0,fragment) }
    val query = rest.indexOf('?')
    if (query >= 0) { if (!uriChars(rest.substring(query+1),":@/?")) return false; rest = rest.substring(0,query) }
    if (rest.startsWith("//")) { val authorityPath = rest.substring(2); val slash = authorityPath.indexOf('/'); val authority = if (slash < 0) authorityPath else authorityPath.substring(0,slash); val path = if (slash < 0) "" else authorityPath.substring(slash); return validAuthority(authority) && uriChars(path,":@/") }
    return uriChars(rest,":@/")
}
private fun days(y: Int, m: Int) = when(m) { 2 -> if (y%4 == 0 && (y%100 != 0 || y%400 == 0)) 29 else 28; 4,6,9,11 -> 30; 1,3,5,7,8,10,12 -> 31; else -> 0 }
private fun date(s: String): Triple<Int,Int,Int>? {
    if (s.length != 10 || s[4] != '-' || s[7] != '-' || !s.withIndex().all { (i,c) -> i == 4 || i == 7 || c in '0'..'9' }) return null
    val y = s.substring(0,4).toInt(); val m = s.substring(5,7).toInt(); val d = s.substring(8).toInt()
    return if (d in 1..days(y,m)) Triple(y,m,d) else null
}
private fun validDateTime(s: String): Boolean {
    if (s.any { it.code > 127 } || s.length < 20) return false
    val parsed = date(s.substring(0,10)) ?: return false
    var (y,m,d) = parsed
    if (s[10] !in "Tt" || s[13] != ':' || s[16] != ':' || !listOf(11,12,14,15,17,18).all { s[it] in '0'..'9' }) return false
    val h = s.substring(11,13).toInt(); val minute = s.substring(14,16).toInt(); val second = s.substring(17,19).toInt()
    if (h > 23 || minute > 59 || second > 60) return false
    var at = 19
    if (s[at] == '.') { at++; val start = at; while (at < s.length && s[at] in '0'..'9') at++; if (at == start) return false }
    val offset: Int
    if (s.getOrNull(at) in listOf('Z','z') && at+1 == s.length) offset = 0 else {
        if (at+6 != s.length || s[at] !in "+-" || s[at+3] != ':' || !listOf(at+1,at+2,at+4,at+5).all { s[it] in '0'..'9' }) return false
        val oh = s.substring(at+1,at+3).toInt(); val om = s.substring(at+4).toInt(); if (oh > 23 || om > 59) return false
        offset = (oh*60+om) * if (s[at] == '+') 1 else -1
    }
    if (second != 60) return true
    val utc = h*60+minute-offset; val day = Math.floorDiv(utc,1440)
    if (day < 0) { d--; if (d == 0) { m--; if (m == 0) { m = 12; y-- }; d = days(y,m) } } else if (day > 0) { d++; if (d > days(y,m)) { d = 1; m++; if (m == 13) { m = 1; y++ } } }
    return Math.floorMod(utc,1440) == 1439 && d == days(y,m)
}
fun validFormat(format: FieldFormat, s: String): Boolean = when(format) { FieldFormat.Email -> validEmail(s); FieldFormat.Uri -> validUri(s); FieldFormat.Date -> s.all { it.code <= 127 } && date(s) != null; FieldFormat.DateTime -> validDateTime(s) }


internal fun resultJson(body: ResultBody): JsonElement = when(body) {
    is ResultBody.SessionControlsRead->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionControlsApply->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionQueueList->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionQueueUpdate->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionQueueDelete->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionQueueReorder->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionQueueStart->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.PairConfirm -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.Hello -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.HostSnapshot -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.SessionOpen -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.SessionHistory -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.SessionUnsubscribe -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.CreationDirectories -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SessionCreate -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.ArtifactsList -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.ArtifactsOpen -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.ArtifactsRead -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.ArtifactsClose -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.TerminalCreate->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.AttachmentsBegin->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.AttachmentsChunk->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.AttachmentsComplete->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.AttachmentsDiscard->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.AttachmentsStatus->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.AgentRename -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.WorkspaceRename -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.PromptSubmit -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.TurnCancel -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.RequestRespond -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.ActionStatus -> when(val value = body.result) { is ActionStatusResult.Known -> { value.value.validate(); Protocol.strictJson.encodeToJsonElement(value.value) }; is ActionStatusResult.Unknown -> { value.value.validate(); Protocol.strictJson.encodeToJsonElement(value.value) } }
    is ResultBody.ContentRead -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.TerminalGridSubscribe -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.ShellGridSubscribe -> {body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.ShellInput -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.ShellResize->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SkillsList -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.SkillsPreviewOpen->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SkillsPreviewRead->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}
    is ResultBody.SkillsPreviewClose->{body.result.validate();Protocol.strictJson.encodeToJsonElement(body.result)}

    is ResultBody.WorktreesList -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.ChangesRead -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.NotificationsSync -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.NotificationsRead -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.PushRegister -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.PushProbeAck -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.PushUnregister -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
    is ResultBody.DeviceUnpair -> { body.result.validate(); Protocol.strictJson.encodeToJsonElement(body.result) }
}
internal fun eventJson(body: EventBody): JsonObject {
    val (kind,data) = when(body) {
        is EventBody.ControlsChanged->{body.data.validate();"controls.changed" to Protocol.strictJson.encodeToJsonElement(body.data)}
        is EventBody.QueueChanged->{body.data.validate();"queue.changed" to Protocol.strictJson.encodeToJsonElement(body.data)}
        is EventBody.GridBegin -> {body.data.validate();"grid.begin" to Protocol.strictJson.encodeToJsonElement(body.data)}
        is EventBody.GridPart -> {body.data.validate();"grid.part" to Protocol.strictJson.encodeToJsonElement(body.data)}
        is EventBody.GridCommit -> {body.data.validate();"grid.commit" to Protocol.strictJson.encodeToJsonElement(body.data)}
        is EventBody.HostChanged -> { body.data.validate(); "host.changed" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.SessionState -> { body.data.validate(); "session.state" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.HistoryUpsert -> { body.data.validate(); "history.upsert" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.ActivityUpdate -> { body.data.validate(); "activity.update" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.RequestOpened -> { body.data.validate(); "request.opened" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.RequestClosed -> { body.data.validate(); "request.closed" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.ActionStatus -> { body.data.validate(); "action.status" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.NotificationUpsert -> { body.data.validate(); "notification.upsert" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.NotificationRead -> { body.data.validate(); "notification.read" to Protocol.strictJson.encodeToJsonElement(body.data) }
        is EventBody.SyncRequired -> { body.data.validate(); "sync.required" to Protocol.strictJson.encodeToJsonElement(body.data) }
    }
    return buildJsonObject { put("kind",kind); put("data",data) }
}

private fun validHttpUrl(s: String): Boolean {
    if (!validUri(s)) return false
    val colon = s.indexOf(':'); val scheme = s.substring(0,colon)
    if (!scheme.equals("http",true) && !scheme.equals("https",true)) return false
    val rest = s.substring(colon+1); if (!rest.startsWith("//")) return false
    val authority = rest.substring(2).substringBefore('/').substringBefore('?').substringBefore('#')
    return authority.substringAfterLast('@').substringBefore(':').isNotEmpty()
}

fun validateTurnCancel(provider: Provider, params: TurnCancelParams) {
    params.validate()
    requireWire((provider == Provider.Codex) == (params.nativeTurnId != null))
}
fun validateContentRead(params: ContentReadParams, byteLength: Uint) {
    params.validate(); byteLength.validate()
    requireWire(byteLength.value <= Protocol.CONTENT_MAX && params.offsetBytes.value <= byteLength.value)
}
