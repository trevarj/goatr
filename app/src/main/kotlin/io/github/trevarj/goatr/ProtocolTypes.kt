// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.serialization.*
import kotlinx.serialization.json.encodeToJsonElement


@Serializable
enum class Provider {
    @SerialName("omp") Omp,
    @SerialName("codex") Codex,
}

@Serializable
enum class Availability {
    @SerialName("ready") Ready,
    @SerialName("setup_required") SetupRequired,
    @SerialName("incompatible") Incompatible,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class ServerConnectivity {
    @SerialName("online") Online,
    @SerialName("offline") Offline,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class PromptMode {
    @SerialName("submit_or_steer") SubmitOrSteer,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class CancelMode {
    @SerialName("current_work") CurrentWork,
    @SerialName("native_turn") NativeTurn,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class HistoryScope {
    @SerialName("all_branches") AllBranches,
    @SerialName("thread") Thread,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class HistorySource {
    @SerialName("persisted_and_live") PersistedAndLive,
    @SerialName("live_only") LiveOnly,
    @SerialName("native_pages") NativePages,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class SessionStatus {
    @SerialName("idle") Idle,
    @SerialName("working") Working,
    @SerialName("attention") Attention,
    @SerialName("unknown") Unknown,
}

@Serializable
enum class SessionConnectivity {
    @SerialName("ready") Ready,
    @SerialName("synchronizing") Synchronizing,
    @SerialName("offline") Offline,
    @SerialName("setup_required") SetupRequired,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class UnboundState {
    @SerialName("setup_required") SetupRequired,
    @SerialName("ambiguous") Ambiguous,
    @SerialName("unavailable") Unavailable,
}

@Serializable
enum class DesktopSelection {
    @SerialName("unknown") Unknown,
    @SerialName("bound_launch_thread") BoundLaunchThread,
}

@Serializable
enum class ActivityState {
    @SerialName("idle") Idle,
    @SerialName("working") Working,
    @SerialName("unknown") Unknown,
}

@Serializable
enum class Mime {
    @SerialName("text/plain") TextPlain,
    @SerialName("application/json") ApplicationJson,
}

@Serializable
enum class ContentUnavailableReason {
    @SerialName("source_truncated") SourceTruncated,
    @SerialName("not_cached") NotCached,
    @SerialName("unsupported") Unsupported,
    @SerialName("too_large") TooLarge,
}

@Serializable
enum class Role {
    @SerialName("user") User,
    @SerialName("assistant") Assistant,
    @SerialName("tool") Tool,
    @SerialName("system") System,
    @SerialName("unknown") Unknown,
}

@Serializable
enum class RecordKind {
    @SerialName("message") Message,
    @SerialName("tool") Tool,
    @SerialName("notice") Notice,
    @SerialName("unsupported") Unsupported,
}

@Serializable
enum class RecordSource {
    @SerialName("persisted") Persisted,
    @SerialName("live") Live,
    @SerialName("native_page") NativePage,
}

@Serializable
enum class ToolStatus {
    @SerialName("running") Running,
    @SerialName("succeeded") Succeeded,
    @SerialName("failed") Failed,
    @SerialName("unknown") Unknown,
}

@Serializable
enum class ApprovalType {
    @SerialName("command") Command,
    @SerialName("file") File,
    @SerialName("permissions") Permissions,
}

@Serializable
enum class ApprovalScope {
    @SerialName("once") Once,
    @SerialName("turn") Turn,
    @SerialName("session") Session,
    @SerialName("policy") Policy,
}

@Serializable
enum class FieldType {
    @SerialName("string") String,
    @SerialName("number") Number,
    @SerialName("integer") Integer,
    @SerialName("boolean") Boolean,
    @SerialName("single") Single,
    @SerialName("multi") Multi,
}

@Serializable
enum class FieldFormat {
    @SerialName("email") Email,
    @SerialName("uri") Uri,
    @SerialName("date") Date,
    @SerialName("date-time") DateTime,
}

@Serializable
enum class ReceiptState {
    @SerialName("reserved") Reserved,
    @SerialName("dispatching") Dispatching,
    @SerialName("dispatched") Dispatched,
    @SerialName("succeeded") Succeeded,
    @SerialName("failed") Failed,
    @SerialName("uncertain") Uncertain,
}

@Serializable
enum class ResponseResolution {
    @SerialName("written") Written,
    @SerialName("acknowledged") Acknowledged,
    @SerialName("resolved_elsewhere_or_unknown") ResolvedElsewhereOrUnknown,
}

@Serializable
enum class CreationStage {
    @SerialName("workspace") Workspace,
    @SerialName("owner") Owner,
    @SerialName("thread") Thread,
    @SerialName("relay") Relay,
    @SerialName("launch") Launch,
    @SerialName("ready") Ready,
}

@Serializable
enum class StageState {
    @SerialName("not_started") NotStarted,
    @SerialName("dispatching") Dispatching,
    @SerialName("confirmed") Confirmed,
    @SerialName("uncertain") Uncertain,
    @SerialName("failed") Failed,
}

@Serializable
enum class AttentionCategory {
    @SerialName("attention") Attention,
    @SerialName("completed") Completed,
}

@Serializable
enum class RequestClosedReason {
    @SerialName("resolved") Resolved,
    @SerialName("cancelled") Cancelled,
    @SerialName("generation_changed") GenerationChanged,
    @SerialName("desktop_only") DesktopOnly,
}

@Serializable
enum class SyncReason {
    @SerialName("overflow") Overflow,
    @SerialName("source_gap") SourceGap,
    @SerialName("generation_changed") GenerationChanged,
}

@Serializable
enum class ActionMethod {
    @SerialName("session.controls.apply") SessionControlsApply,
    @SerialName("session.queue.update") SessionQueueUpdate,
    @SerialName("session.queue.delete") SessionQueueDelete,
    @SerialName("session.queue.reorder") SessionQueueReorder,
    @SerialName("session.queue.start") SessionQueueStart,
    @SerialName("terminal.create") TerminalCreate,
    @SerialName("attachments.begin") AttachmentsBegin,
    @SerialName("attachments.chunk") AttachmentsChunk,
    @SerialName("attachments.complete") AttachmentsComplete,
    @SerialName("attachments.discard") AttachmentsDiscard,
    @SerialName("session.create") SessionCreate,
    @SerialName("shell.create") ShellCreate,
    @SerialName("shell.input") ShellInput,
    @SerialName("shell.resize") ShellResize,
    @SerialName("shell.scroll") ShellScroll,
    @SerialName("worktrees.remove") WorktreesRemove,
    @SerialName("agent.rename") AgentRename,
    @SerialName("workspace.rename") WorkspaceRename,
    @SerialName("prompt.submit") PromptSubmit,
    @SerialName("turn.cancel") TurnCancel,
    @SerialName("request.respond") RequestRespond,
}

@Serializable
enum class ErrorCode {
    @SerialName("invalid_request") InvalidRequest,
    @SerialName("unsupported_version") UnsupportedVersion,
    @SerialName("unsupported_method") UnsupportedMethod,
    @SerialName("unauthorized") Unauthorized,
    @SerialName("pairing_failed") PairingFailed,
    @SerialName("stale_host") StaleHost,
    @SerialName("stale_session") StaleSession,
    @SerialName("not_found") NotFound,
    @SerialName("not_ready") NotReady,
    @SerialName("invalid_answer") InvalidAnswer,
    @SerialName("already_resolved") AlreadyResolved,
    @SerialName("action_id_conflict") ActionIdConflict,
    @SerialName("action_capacity") ActionCapacity,
    @SerialName("content_expired") ContentExpired,
    @SerialName("cursor_expired") CursorExpired,
    @SerialName("busy") Busy,
    @SerialName("resource_limit") ResourceLimit,
    @SerialName("rate_limited") RateLimited,
    @SerialName("timeout") Timeout,
    @SerialName("backend_error") BackendError,
    @SerialName("push_rejected") PushRejected,
    @SerialName("stale_registration") StaleRegistration,
}

@Serializable
enum class SubmitAction {
    @SerialName("submit") Submit,
    @SerialName("cancel") Cancel,
}

@Serializable
enum class FormAction {
    @SerialName("accept") Accept,
    @SerialName("decline") Decline,
    @SerialName("cancel") Cancel,
}

@Serializable
enum class UrlAction {
    @SerialName("accept") Accept,
    @SerialName("decline") Decline,
    @SerialName("cancel") Cancel,
}

@Serializable
enum class PresentationKind {
    @SerialName("omp_select") OmpSelect,
    @SerialName("omp_editor") OmpEditor,
    @SerialName("codex_approval") CodexApproval,
    @SerialName("codex_questions") CodexQuestions,
    @SerialName("codex_form") CodexForm,
    @SerialName("codex_url") CodexUrl,
    @SerialName("desktop_only") DesktopOnly,
}

@Serializable
enum class Method {
    @SerialName("session.controls.read") SessionControlsRead,
    @SerialName("session.controls.apply") SessionControlsApply,
    @SerialName("session.queue.list") SessionQueueList,
    @SerialName("session.queue.update") SessionQueueUpdate,
    @SerialName("session.queue.delete") SessionQueueDelete,
    @SerialName("session.queue.reorder") SessionQueueReorder,
    @SerialName("session.queue.start") SessionQueueStart,
    @SerialName("artifacts.list") ArtifactsList,
    @SerialName("artifacts.open") ArtifactsOpen,
    @SerialName("artifacts.read") ArtifactsRead,
    @SerialName("artifacts.close") ArtifactsClose,
    @SerialName("terminal.create") TerminalCreate,
    @SerialName("attachments.begin") AttachmentsBegin,
    @SerialName("attachments.chunk") AttachmentsChunk,
    @SerialName("attachments.complete") AttachmentsComplete,
    @SerialName("attachments.discard") AttachmentsDiscard,
    @SerialName("attachments.status") AttachmentsStatus,
    @SerialName("pair.confirm") PairConfirm,
    @SerialName("hello") Hello,
    @SerialName("host.snapshot") HostSnapshot,
    @SerialName("session.open") SessionOpen,
    @SerialName("session.history") SessionHistory,
    @SerialName("session.unsubscribe") SessionUnsubscribe,
    @SerialName("creation.directories") CreationDirectories,
    @SerialName("session.create") SessionCreate,
    @SerialName("shell.input") ShellInput,
    @SerialName("shell.resize") ShellResize,
    @SerialName("shell.scroll") ShellScroll,
    @SerialName("agent.rename") AgentRename,
    @SerialName("workspace.rename") WorkspaceRename,
    @SerialName("prompt.submit") PromptSubmit,
    @SerialName("turn.cancel") TurnCancel,
    @SerialName("request.respond") RequestRespond,
    @SerialName("action.status") ActionStatus,
    @SerialName("content.read") ContentRead,
    @SerialName("terminal.grid.subscribe") TerminalGridSubscribe,
    @SerialName("shell.grid.subscribe") ShellGridSubscribe,
    @SerialName("worktrees.list") WorktreesList,
    @SerialName("changes.read") ChangesRead,
    @SerialName("skills.list") SkillsList,
    @SerialName("skills.preview.open") SkillsPreviewOpen,
    @SerialName("skills.preview.read") SkillsPreviewRead,
    @SerialName("skills.preview.close") SkillsPreviewClose,

    @SerialName("notifications.sync") NotificationsSync,
    @SerialName("notifications.read") NotificationsRead,
    @SerialName("push.register") PushRegister,
    @SerialName("push.probe.ack") PushProbeAck,
    @SerialName("push.unregister") PushUnregister,
    @SerialName("device.unpair") DeviceUnpair,
}

@Serializable
data class Target(
    val sessionId: Uuid,
    val generation: Uuid,
) : WireValue {
    override fun validate() {
        sessionId.validateWire()
        generation.validateWire()
    }
}

@Serializable
data class Backend(
    val provider: Provider,
    val version: Text,
    val availability: Availability,
    val reason: Text? = null,
) : WireValue {
    override fun validate() {
        version.validateWire()
        reason.validateWire()
    }
}

@Serializable
data class CanCreate(
    val omp: Boolean,
    val codex: Boolean,
) : WireValue {
    override fun validate() {
        omp.validateWire()
        codex.validateWire()
    }
}

@Serializable
data class Server(
    val serverId: Uuid,
    val name: Label,
    val connectivity: ServerConnectivity,
    val canCreate: CanCreate,
    val reason: Text? = null,
) : WireValue {
    override fun validate() {
        serverId.validateWire()
        name.validateWire()
        canCreate.validateWire()
        reason.validateWire()
    }
}

@Serializable
enum class NativeAgentStatus {
    @SerialName("idle") Idle,
    @SerialName("working") Working,
    @SerialName("blocked") Blocked,
    @SerialName("done") Done,
    @SerialName("unknown") Unknown,
}
@Serializable
enum class NativeFreshness {
    @SerialName("current") Current,
    @SerialName("stale") Stale,
    @SerialName("unavailable") Unavailable,
}
@Serializable
enum class NativeUnavailableReason {
    @SerialName("source_unavailable") SourceUnavailable,
    @SerialName("too_large") TooLarge,
}
@Serializable
enum class NativeDisplayUnavailable {
    @SerialName("too_large") TooLarge,
    @SerialName("unsafe") Unsafe,
}
@Serializable
enum class NativeRestoreReason {
    @SerialName("restore_failed") RestoreFailed,
}
@Serializable
data class NativeWorkspace(
    val workspaceId: Id,
    val number: Uint,
    val label: Label? = null,
    val focused: Boolean,
    val activeTabId: Id? = null,
    val agentStatus: NativeAgentStatus,
    val displayUnavailable: NativeDisplayUnavailable? = null,
) : WireValue {
    override fun validate() {
        workspaceId.validateWire(); number.validateWire(); label.validateWire(); activeTabId.validateWire()
        requireWire(label != null || displayUnavailable != null)
    }
}
@Serializable
data class NativeTab(
    val tabId: Id,
    val workspaceId: Id,
    val number: Uint,
    val label: Label? = null,
    val focused: Boolean,
    val agentStatus: NativeAgentStatus,
    val displayUnavailable: NativeDisplayUnavailable? = null,
) : WireValue {
    override fun validate() {
        tabId.validateWire(); workspaceId.validateWire(); number.validateWire(); label.validateWire()
        requireWire(label != null || displayUnavailable != null)
    }
}
@Serializable
data class NativePane(
    val paneId: Id,
    val terminalId: Id,
    val workspaceId: Id,
    val tabId: Id,
    val label: Label? = null,
    val title: Label? = null,
    val terminalTitle: Label? = null,
    val displayAgent: Label? = null,
    val agent: Label? = null,
    val agentName: Label? = null,
    val focused: Boolean,
    val agentStatus: NativeAgentStatus,
    val restoreReason: NativeRestoreReason? = null,
    val displayUnavailable: NativeDisplayUnavailable? = null,
    val target: Target? = null,
    val unboundRowId: Uuid? = null,
    val shellTarget: ShellTarget? = null,
) : WireValue {
    override fun validate() {
        paneId.validateWire(); terminalId.validateWire(); workspaceId.validateWire(); tabId.validateWire()
        label.validateWire(); title.validateWire(); terminalTitle.validateWire(); displayAgent.validateWire()
        agent.validateWire(); agentName.validateWire(); target.validateWire(); unboundRowId.validateWire()
        shellTarget.validateWire()
        requireWire(listOf(target,unboundRowId,shellTarget).count { it != null } <= 1)
    }
}
@Serializable
data class NativeHierarchy(
    val serverId: Uuid,
    val serverIncarnation: Uuid,
    val freshness: NativeFreshness,
    val reason: NativeUnavailableReason? = null,
    val workspaces: List<NativeWorkspace>,
    val tabs: List<NativeTab>,
    val panes: List<NativePane>,
) : WireValue {
    override fun validate() {
        serverId.validateWire(); serverIncarnation.validateWire()
        workspaces.validateWire(); tabs.validateWire(); panes.validateWire()
        validateNativeHierarchy(this)
    }
}

@Serializable
data class Project(
    val projectId: Uuid,
    val label: Label,
    val available: Boolean,
    val reason: Text? = null,
) : WireValue {
    override fun validate() {
        projectId.validateWire()
        label.validateWire()
        available.validateWire()
        reason.validateWire()
    }
}

@Serializable
data class Capabilities(
    val history: Boolean,
    val prompt: Boolean,
    val cancel: Boolean,
    val respond: Boolean,
    val promptMode: PromptMode,
    val cancelMode: CancelMode,
    val terminalRead: Boolean = false,
    val worktreesRead: Boolean = false,
    val changesRead: Boolean = false,
    val workspaceRename: Boolean = false,
    val agentRename: Boolean = false,
    val skillsRead: Boolean = false,
    val attachments:Boolean=false,
    val terminalCreate:Boolean=false,
    val artifacts:Boolean=false,
    val controls:Boolean=false,
) : WireValue {
    override fun validate() {
        history.validateWire()
        prompt.validateWire()
        cancel.validateWire()
        respond.validateWire()
        terminalRead.validateWire()
        worktreesRead.validateWire()
        changesRead.validateWire()
        workspaceRename.validateWire()
        agentRename.validateWire()
        skillsRead.validateWire()
        controls.validateWire()
    }
}

@Serializable
data class HistoryCoverage(
    val scope: HistoryScope,
    val source: HistorySource,
    val olderAvailable: Boolean,
    val sourceTruncated: Boolean,
    val desktopBranchKnown: Boolean,
) : WireValue {
    override fun validate() {
        olderAvailable.validateWire()
        sourceTruncated.validateWire()
        desktopBranchKnown.validateWire()
    }
}

@Serializable
data class SessionSummary(
    val sessionId: Uuid,
    val generation: Uuid,
    val serverId: Uuid,
    val workspaceId: Id,
    val terminalId: Id,
    val paneId: Id,
    val provider: Provider,
    val nativeId: Id,
    val title: Text,
    val status: SessionStatus,
    val connectivity: SessionConnectivity,
    val capabilities: Capabilities,
    val historyCoverage: HistoryCoverage,
    val reason: Text? = null,
    val skillsGeneration: Uuid? = null,
) : WireValue {
    override fun validate() {
        sessionId.validateWire()
        generation.validateWire()
        serverId.validateWire()
        workspaceId.validateWire()
        terminalId.validateWire()
        paneId.validateWire()
        nativeId.validateWire()
        title.validateWire()
        capabilities.validateWire()
        historyCoverage.validateWire()
        skillsGeneration.validateWire()
        reason.validateWire()
    }
}

@Serializable
data class UnboundRow(
    val rowId: Uuid,
    val serverId: Uuid,
    val workspaceId: Id,
    val terminalId: Id,
    val paneId: Id,
    val provider: Provider,
    val title: Text,
    val state: UnboundState,
    val reason: Text,
) : WireValue {
    override fun validate() {
        rowId.validateWire()
        serverId.validateWire()
        workspaceId.validateWire()
        terminalId.validateWire()
        paneId.validateWire()
        title.validateWire()
        reason.validateWire()
    }
}

@Serializable
data class SessionState(
    val summary: SessionSummary,
    val nativeTurnId: Id? = null,
    val desktopSelection: DesktopSelection,
    val activity: Activity,
) : WireValue {
    override fun validate() {
        summary.validateWire()
        nativeTurnId.validateWire()
        activity.validateWire()
    }
}

@Serializable
data class Activity(
    val state: ActivityState,
    val text: Content? = null,
    val toolRecordId: Id? = null,
) : WireValue {
    override fun validate() {
        text.validateWire()
        toolRecordId.validateWire()
    }
}

@Serializable
data class HistoryRecord(
    val id: Id,
    val parentId: Id? = null,
    val turnId: Id? = null,
    val order: Uint,
    val timestamp: Uint? = null,
    val role: Role,
    val kind: RecordKind,
    val text: Content? = null,
    val tool: Tool? = null,
    val original: Content? = null,
    val source: RecordSource,
    val complete: Boolean,
    val sourceTruncated: Boolean,
) : WireValue {
    override fun validate() {
        id.validateWire()
        parentId.validateWire()
        turnId.validateWire()
        order.validateWire()
        timestamp.validateWire()
        text.validateWire()
        tool.validateWire()
        original.validateWire()
        complete.validateWire()
        sourceTruncated.validateWire()
    }
}

@Serializable
data class Tool(
    val name: Text,
    val callId: Id? = null,
    val status: ToolStatus,
    val input: Content? = null,
    val output: Content? = null,
) : WireValue {
    override fun validate() {
        name.validateWire()
        callId.validateWire()
        input.validateWire()
        output.validateWire()
    }
}

@Serializable
data class PendingRequest(
    val requestKey: Uuid,
    val provider: Provider,
    val kind: PresentationKind,
    val presentation: Presentation,
) : WireValue {
    override fun validate() {
        requestKey.validateWire()
        presentation.validateWire()
        validatePendingRequest(this)
    }
}

@Serializable
data class WireError(
    val code: ErrorCode,
    val message: SafeMessage,
) : WireValue {
    override fun validate() {
        message.validateWire()
    }
}

@Serializable
data class SelectOption(
    val value: Text,
    val label: Label,
    val description: Content? = null,
) : WireValue {
    override fun validate() {
        value.validateWire()
        label.validateWire()
        description.validateWire()
    }
}

@Serializable
data class ApprovalChoice(
    val choiceId: Id,
    val label: Label,
    val scope: ApprovalScope,
    val details: Content? = null,
) : WireValue {
    override fun validate() {
        choiceId.validateWire()
        label.validateWire()
        details.validateWire()
    }
}

@Serializable
data class QuestionOption(
    val label: Label,
    val description: Content,
) : WireValue {
    override fun validate() {
        label.validateWire()
        description.validateWire()
    }
}

@Serializable
data class Question(
    val id: Id,
    val header: Text,
    val prompt: Content,
    val isOther: Boolean,
    val isSecret: Boolean,
    val options: List<QuestionOption>,
) : WireValue {
    override fun validate() {
        id.validateWire()
        header.validateWire()
        prompt.validateWire()
        isOther.validateWire()
        isSecret.validateWire()
        options.validateWire()
        validateQuestion(this)
    }
}

@Serializable
data class FormOption(
    val value: Text,
    val label: Label,
) : WireValue {
    override fun validate() {
        value.validateWire()
        label.validateWire()
    }
}

@Serializable
data class FormField(
    val id: Id,
    val title: Text,
    val description: Content? = null,
    val required: Boolean,
    val type: FieldType,
    val `default`: FieldValue? = null,
    val format: FieldFormat? = null,
    val minLength: Uint? = null,
    val maxLength: Uint? = null,
    val minimum: Decimal? = null,
    val maximum: Decimal? = null,
    val minItems: Uint? = null,
    val maxItems: Uint? = null,
    val options: List<FormOption>? = null,
) : WireValue {
    override fun validate() {
        id.validateWire()
        title.validateWire()
        description.validateWire()
        required.validateWire()
        `default`.validateWire()
        minLength.validateWire()
        maxLength.validateWire()
        minimum.validateWire()
        maximum.validateWire()
        minItems.validateWire()
        maxItems.validateWire()
        options.validateWire()
        validateFormField(this)
    }
}

@Serializable
data class QuestionAnswer(
    val id: Id,
    val values: List<Text>,
) : WireValue {
    override fun validate() {
        id.validateWire()
        values.validateWire()
    }
}

@Serializable
data class FormAnswerValue(
    val id: Id,
    val value: FieldValue,
) : WireValue {
    override fun validate() {
        id.validateWire()
        value.validateWire()
    }
}

@Serializable
data class Receipt(
    val actionId: Uuid,
    val method: ActionMethod,
    val bootId: Uuid,
    val target: Target? = null,
    val createdAt: Uint,
    val updatedAt: Uint,
    val state: ReceiptState,
    val error: WireError? = null,
    val result: ActionResult? = null,
    val creation: CreationProgress? = null,
    val shellTarget: ShellTarget? = null,
) : WireValue {
    override fun validate() {
        actionId.validateWire()
        bootId.validateWire()
        target.validateWire()
        shellTarget.validateWire()
        createdAt.validateWire()
        updatedAt.validateWire()
        error.validateWire()
        result.validateWire()
        creation.validateWire()
        validateReceipt(this)
    }
}

@Serializable
data class CreationProgress(
    val serverId: Uuid,
    val projectId: Uuid,
    val provider: Provider,
    val stage: CreationStage,
    val stageState: StageState,
    val resources: CreationResources,
    val sourceTarget: Target? = null,
) : WireValue {
    override fun validate() {
        sourceTarget.validateWire()
        serverId.validateWire()
        projectId.validateWire()
        resources.validateWire()
        requireWire(resources.checkout==null || sourceTarget!=null)
    }
}

@Serializable
data class CreationResources(
    val workspaceId: Id? = null,
    val paneId: Id? = null,
    val terminalId: Id? = null,
    val nativeId: Id? = null,
    val bindingId: Uuid? = null,
    val sessionId: Uuid? = null,
    val generation: Uuid? = null,
    val ownerStarted: Boolean? = null,
    val relayStarted: Boolean? = null,
    val checkout: CreationCheckoutResources? = null,
) : WireValue {
    override fun validate() {
        workspaceId.validateWire()
        paneId.validateWire()
        terminalId.validateWire()
        nativeId.validateWire()
        bindingId.validateWire()
        sessionId.validateWire()
        generation.validateWire()
        ownerStarted.validateWire()
        relayStarted.validateWire()
        checkout.validateWire()
    }
}

@Serializable
data class AttentionEvent(
    val eventId: Uuid,
    val sequence: Uint,
    val sessionId: Uuid,
    val generation: Uuid,
    val category: AttentionCategory,
    val createdAt: Uint,
) : WireValue {
    override fun validate() {
        eventId.validateWire()
        sequence.validateWire()
        sessionId.validateWire()
        generation.validateWire()
        createdAt.validateWire()
    }
}

@Serializable
data class ReadWatermark(
    val sessionId: Uuid,
    val throughSequence: Uint,
) : WireValue {
    override fun validate() {
        sessionId.validateWire()
        throughSequence.validateWire()
    }
}

@Serializable
@SerialName("text")
data class ContentText(
    val text: InlineText,
) : Content {
    override fun validate() {
        text.validateWire()
    }
}

@Serializable
@SerialName("ref")
data class ContentRef(
    val handle: Uuid,
    val byteLength: Uint,
    val mime: Mime,
) : Content {
    override fun validate() {
        handle.validateWire()
        byteLength.validateWire()
        validateContentRef(this)
    }
}

@Serializable
@SerialName("unavailable")
data class ContentUnavailable(
    val reason: ContentUnavailableReason,
    val byteLength: Uint? = null,
) : Content {
    override fun validate() {
        byteLength.validateWire()
    }
}

@Serializable
enum class SelectionMarker {
    @SerialName("radio") Radio,
    @SerialName("checkbox") Checkbox,
}

@Serializable
@SerialName("omp_select")
data class PresentationOmpSelect(
    val title: Content,
    val options: List<SelectOption>,
    val selectionMarker: SelectionMarker,
    val helpText: Content? = null,
    val markableCount: Uint? = null,
    val selected: Uint? = null,
    val checked: List<Uint>,
) : Presentation {
    override fun validate() {
        title.validateWire()
        options.validateWire()
        selected.validateWire()
        checked.validateWire()
        helpText.validateWire()
        markableCount.validateWire()
        validatePresentationOmpSelect(this)
    }
}

@Serializable
@SerialName("omp_editor")
data class PresentationOmpEditor(
    val title: Content,
    val prefill: Content,
) : Presentation {
    override fun validate() {
        title.validateWire()
        prefill.validateWire()
    }
}

@Serializable
@SerialName("codex_approval")
data class PresentationCodexApproval(
    val approvalType: ApprovalType,
    val title: Content,
    val details: Content,
    val choices: List<ApprovalChoice>,
) : Presentation {
    override fun validate() {
        title.validateWire()
        details.validateWire()
        choices.validateWire()
        validatePresentationCodexApproval(this)
    }
}

@Serializable
@SerialName("codex_questions")
data class PresentationCodexQuestions(
    val isBlocking: Boolean,
    val questions: List<Question>,
) : Presentation {
    override fun validate() {
        isBlocking.validateWire()
        questions.validateWire()
        validatePresentationCodexQuestions(this)
    }
}

@Serializable
@SerialName("codex_form")
data class PresentationCodexForm(
    val title: Content,
    val fields: List<FormField>,
) : Presentation {
    override fun validate() {
        title.validateWire()
        fields.validateWire()
        validatePresentationCodexForm(this)
    }
}

@Serializable
@SerialName("codex_url")
data class PresentationCodexUrl(
    val title: Content,
    val url: Text,
) : Presentation {
    override fun validate() {
        title.validateWire()
        url.validateWire()
        validatePresentationCodexUrl(this)
    }
}

@Serializable
@SerialName("desktop_only")
data class PresentationDesktopOnly(
    val title: Content,
    val reason: Text,
) : Presentation {
    override fun validate() {
        title.validateWire()
        reason.validateWire()
    }
}

@Serializable
@SerialName("omp_select")
data class AnswerOmpSelect(
    val action: SubmitAction,
    val value: Text? = null,
) : Answer {
    override fun validate() {
        value.validateWire()
        validateAnswerOmpSelect(this)
    }
}

@Serializable
@SerialName("omp_editor")
data class AnswerOmpEditor(
    val action: SubmitAction,
    val text: PromptText? = null,
) : Answer {
    override fun validate() {
        text.validateWire()
        validateAnswerOmpEditor(this)
    }
}

@Serializable
@SerialName("codex_approval")
data class AnswerCodexApproval(
    val choiceId: Id,
) : Answer {
    override fun validate() {
        choiceId.validateWire()
    }
}

@Serializable
@SerialName("codex_questions")
data class AnswerCodexQuestions(
    val answers: List<QuestionAnswer>,
) : Answer {
    override fun validate() {
        answers.validateWire()
        validateAnswerCodexQuestions(this)
    }
}

@Serializable
@SerialName("codex_form")
data class AnswerCodexForm(
    val action: FormAction,
    val values: List<FormAnswerValue>? = null,
) : Answer {
    override fun validate() {
        values.validateWire()
        validateAnswerCodexForm(this)
    }
}

@Serializable
@SerialName("codex_url")
data class AnswerCodexUrl(
    val action: UrlAction,
) : Answer {
    override fun validate() {
    }
}

@Serializable
@SerialName("prompt")
data class ActionResultPrompt(
    val nativeTurnId: Id? = null,
) : ActionResult {
    override fun validate() {
        nativeTurnId.validateWire()
    }
}

@Serializable
@SerialName("cancel")
data class ActionResultCancel(
    val nativeTurnId: Id? = null,
) : ActionResult {
    override fun validate() {
        nativeTurnId.validateWire()
    }
}

@Serializable
@SerialName("response")
data class ActionResultResponse(
    val resolution: ResponseResolution,
) : ActionResult {
    override fun validate() {
    }
}

@Serializable
@SerialName("created")
data class ActionResultCreated(
    val target: Target,
    val workspaceId: Id,
    val terminalId: Id,
    val paneId: Id,
    val nativeId: Id,
    val bindingId: Uuid? = null,
) : ActionResult {
    override fun validate() {
        target.validateWire()
        workspaceId.validateWire()
        terminalId.validateWire()
        paneId.validateWire()
        nativeId.validateWire()
        bindingId.validateWire()
    }
}

@Serializable
data class PairConfirmParams(
    val token: Key32,
    val deviceName: Label,
) : WireValue {
    override fun validate() {
        token.validateWire()
        deviceName.validateWire()
    }
}

@Serializable
data class PairConfirmResult(
    val hostId: EndpointId,
    val deviceId: Uuid,
    val paired: TrueValue,
) : WireValue {
    override fun validate() {
        hostId.validateWire()
        deviceId.validateWire()
        paired.validateWire()
    }
}

@Serializable
data class HelloParams(
    val appVersion: Text,
    val deviceName: Label,
) : WireValue {
    override fun validate() {
        appVersion.validateWire()
        deviceName.validateWire()
    }
}

@Serializable
data class HelloFeatures(val agentAnsi:Boolean=false,val shellHistory:Boolean=false)
@Serializable
data class HelloResult(
    val hostId: EndpointId,
    val deviceId: Uuid,
    val bootId: Uuid,
    val protocol: ProtocolVersion,
    val backends: List<Backend>,
    val features:HelloFeatures=HelloFeatures(),
) : WireValue {
    override fun validate() {
        hostId.validateWire()
        deviceId.validateWire()
        bootId.validateWire()
        protocol.validateWire()
        backends.validateWire()
        validateHelloResult(this)
    }
}

@Serializable
class HostSnapshotParams : WireValue {
    override fun validate() {
    }
}

@Serializable
data class HostSnapshotResult(
    val subscription: Uuid,
    val seq: ZeroValue,
    val servers: List<Server>,
    val projects: List<Project>,
    val sessions: List<SessionSummary>,
    val unbound: List<UnboundRow>,
    val hierarchy: List<NativeHierarchy>,
    val shells: List<ShellSummary> = emptyList(),
) : WireValue {
    override fun validate() {
        subscription.validateWire()
        seq.validateWire()
        servers.validateWire()
        projects.validateWire()
        sessions.validateWire()
        shells.validateWire()
        unbound.validateWire()
        hierarchy.validateWire()
        validateHostSnapshotResult(this)
    }
}

@Serializable
data class SessionOpenParams(
    val target: Target,
) : WireValue {
    override fun validate() {
        target.validateWire()
    }
}

@Serializable
data class SessionOpenResult(
    val subscription: Uuid,
    val seq: ZeroValue,
    val state: SessionState,
    val pendingRequests: List<PendingRequest>,
    val history: List<HistoryRecord>,
) : WireValue {
    override fun validate() {
        subscription.validateWire()
        seq.validateWire()
        state.validateWire()
        pendingRequests.validateWire()
        history.validateWire()
        validateSessionOpenResult(this)
    }
}

@Serializable
data class SessionHistoryParams(
    val target: Target,
    val cursor: Id? = null,
    val limit: Uint? = null,
    val viewId: Uuid? = null,
) : WireValue {
    override fun validate() {
        viewId.validateWire()
        target.validateWire()
        cursor.validateWire()
        limit.validateWire()
        validateSessionHistoryParams(this)
    }
}

@Serializable
data class SessionHistoryResult(
    val records: List<HistoryRecord>,
    val viewId: Uuid,
    val nextCursor: Id? = null,
) : WireValue {
    override fun validate() {
        records.validateWire()
        nextCursor.validateWire()
        viewId.validateWire()
        validateSessionHistoryResult(this)
    }
}

@Serializable
data class SessionUnsubscribeParams(
    val subscription: Uuid,
) : WireValue {
    override fun validate() {
        subscription.validateWire()
    }
}

@Serializable
data class SessionUnsubscribeResult(
    val unsubscribed: TrueValue,
) : WireValue {
    override fun validate() {
        unsubscribed.validateWire()
    }
}

@Serializable
data class SessionCreateParams(
    val bootId: Uuid,
    val serverId: Uuid,
    val projectId: Uuid,
    val provider: Provider,
    val name: Label,
    val directoryRef: Uuid? = null,
    val placement: SessionPlacement? = null,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        serverId.validateWire()
        projectId.validateWire()
        name.validateWire()
        directoryRef.validateWire()
        placement.validateWire()
    }
}

@Serializable
data class SessionPlacement(val sourceTarget:Target):WireValue {
    override fun validate(){sourceTarget.validateWire()}
}
@Serializable
data class CreationDirectoriesParams(
    val serverId:Uuid,val projectId:Uuid,val source:ArtifactSource?=null,
    val parent:Uuid?=null,val cursor:Uuid?=null,val limit:Uint?=null,
):WireValue {
    override fun validate(){
        serverId.validateWire();projectId.validateWire();source.validateWire()
        parent.validateWire();cursor.validateWire();limit.validateWire()
        requireWire(limit==null || limit.value in 1L..100L)
    }
}
@Serializable
data class CreationDirectoryEntry(val reference:Uuid,val name:Label):WireValue {
    override fun validate(){reference.validateWire();name.validateWire()}
}
@Serializable
data class CreationDirectoriesResult(
    val serverId:Uuid,val projectId:Uuid,val source:ArtifactSource?=null,
    val entries:List<CreationDirectoryEntry>,val nextCursor:Uuid?=null,
):WireValue {
    override fun validate(){
        serverId.validateWire();projectId.validateWire();source.validateWire()
        requireWire(entries.size<=100);entries.validateWire();nextCursor.validateWire()
        val seen=HashSet<Uuid>(entries.size*4/3+1);requireWire(entries.all {seen.add(it.reference)})
    }
}

@Serializable
data class SessionCreateResult(
    val receipt: Receipt,
) : WireValue {
    override fun validate() {
        receipt.validateWire()
    }
}

@Serializable
data class AgentRenameParams(
    val bootId: Uuid,
    val target: Target,
    val name: AgentName,
) : WireValue {
    override fun validate() { bootId.validateWire(); target.validateWire(); name.validateWire() }
}

@Serializable
data class AgentRenameResult(
    val receipt: Receipt,
) : WireValue {
    override fun validate() { receipt.validateWire() }
}

@Serializable
data class WorkspaceRenameParams(
    val bootId: Uuid,
    val target: Target,
    val label: Label,
) : WireValue {
    override fun validate() { bootId.validateWire(); target.validateWire(); label.validateWire() }
}

@Serializable
data class WorkspaceRenameResult(
    val receipt: Receipt,
) : WireValue {
    override fun validate() { receipt.validateWire() }
}

@Serializable
data class SkillSelector(val catalogueGeneration:Uuid,val reference:Uuid):WireValue {
    override fun validate(){catalogueGeneration.validateWire();reference.validateWire()}
}

@Serializable
data class PromptSubmitParams(
    val bootId: Uuid,
    val target: Target,
    val text: PromptText,
    val skill:SkillSelector?=null,
    val delivery:PromptDelivery?=null,
    val nativeTurnId:Id?=null,
    val images:List<Uuid>?=null,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        target.validateWire()
        text.validateWire()
        skill.validateWire()
        delivery.validateWire();nativeTurnId.validateWire();images.validateWire()
        requireWire(nativeTurnId==null||delivery==PromptDelivery.Steer)
        images?.let {ids->val unique=HashSet<Uuid>();requireWire(ids.size in 1..4&&ids.all(unique::add))}
        requireWire(skill==null||text.value.isEmpty()&&delivery==null&&nativeTurnId==null&&images==null)
    }
}

@Serializable
data class PromptSubmitResult(
    val receipt: Receipt,
) : WireValue {
    override fun validate() {
        receipt.validateWire()
    }
}

@Serializable
data class TurnCancelParams(
    val bootId: Uuid,
    val target: Target,
    val nativeTurnId: Id? = null,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        target.validateWire()
        nativeTurnId.validateWire()
    }
}

@Serializable
data class TurnCancelResult(
    val receipt: Receipt,
) : WireValue {
    override fun validate() {
        receipt.validateWire()
    }
}

@Serializable
data class RequestRespondParams(
    val bootId: Uuid,
    val target: Target,
    val requestKey: Uuid,
    val answer: Answer,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        target.validateWire()
        requestKey.validateWire()
        answer.validateWire()
    }
}

@Serializable
data class RequestRespondResult(
    val receipt: Receipt,
) : WireValue {
    override fun validate() {
        receipt.validateWire()
    }
}

@Serializable
data class ActionStatusParams(
    val actionId: Uuid,
) : WireValue {
    override fun validate() {
        actionId.validateWire()
    }
}

@Serializable
data class ContentReadParams(
    val target: Target,
    val handle: Uuid,
    val offsetBytes: Uint,
    val limitBytes: Uint,
) : WireValue {
    override fun validate() {
        target.validateWire()
        handle.validateWire()
        offsetBytes.validateWire()
        limitBytes.validateWire()
        validateContentReadParams(this)
    }
}

@Serializable
data class ContentReadResult(
    val bytes: Base64Bytes,
    val nextOffsetBytes: Uint,
    val eof: Boolean,
) : WireValue {
    override fun validate() {
        bytes.validateWire()
        nextOffsetBytes.validateWire()
        eof.validateWire()
    }
}

@Serializable
data class TerminalGridSubscribeParams(val target:Target,val ansiObserver:Boolean=false):WireValue {override fun validate(){target.validateWire()}}
@Serializable
data class TerminalGridSubscribeResult(val target:Target,val subscription:Uuid,val seq:ZeroValue,val frame:GridBegin):WireValue {
    override fun validate(){target.validateWire();subscription.validateWire();seq.validateWire();frame.validate();requireWire(frame.revision.value==0L)}
}
@Serializable
sealed interface GridColor:WireValue {
    @Serializable @SerialName("default") data object Default:GridColor {override fun validate(){}}
    @Serializable @SerialName("indexed") data class Indexed(val index:Uint):GridColor {override fun validate(){requireWire(index.value<=255)}}
    @Serializable @SerialName("rgb") data class Rgb(val value:Uint):GridColor {override fun validate(){requireWire(value.value<=0xffffff)}}
}
@Serializable
enum class GridUnderline {
    @SerialName("none") None,@SerialName("single") Single,@SerialName("double") Double,
    @SerialName("curly") Curly,@SerialName("dotted") Dotted,@SerialName("dashed") Dashed,
}
@Serializable
data class GridAttributes(val bold:Boolean,val faint:Boolean,val italic:Boolean,val blink:Boolean,val strike:Boolean,val underline:GridUnderline,val underlineColor:GridColor?=null,val inverse:Boolean=false,val concealed:Boolean=false):WireValue {
    override fun validate(){underlineColor.validateWire()}
}
@Serializable
class GridCell(val text:String,val width:Uint,val foreground:GridColor,val background:GridColor,val attributes:GridAttributes):WireValue {
    override fun validate(){
        requireWire(validUnicode(text) && utf8Size(text)<=260 && width.value<=2 && (width.value==0L)==text.isEmpty() && text.none {it.isISOControl() || it=='\u2028' || it=='\u2029'})
        foreground.validate();background.validate();attributes.validate()
    }
    override fun equals(other:Any?)=other is GridCell && text==other.text && width==other.width && foreground==other.foreground && background==other.background && attributes==other.attributes
    override fun hashCode()=31*text.hashCode()+width.hashCode()
    override fun toString()="GridCell(redacted)"
}
@Serializable
data class GridCursor(val column:Uint,val row:Uint,val visible:Boolean,val shape:Uint)
@Serializable
enum class GridMode {@SerialName("replace") Replace}
@Serializable
data class GridBegin(val mode:GridMode,val revision:Uint,val columns:Uint,val rows:Uint,val cursor:GridCursor?=null,val alternateScreen:Boolean,val graphicsOmitted:Boolean,val partCount:Uint,val popupBlocked:Boolean=false,val scroll:GridScroll?=null):WireValue {
    override fun validate(){
        requireWire(columns.value in 1..512 && rows.value in 1..256 && columns.value*rows.value<=32768 && partCount.value in 1..128)
        cursor?.let {requireWire(it.column.value<columns.value && it.row.value<rows.value && it.shape.value<=6)}
        scroll?.validate()
    }
}
@Serializable
data class GridScroll(val offsetFromBottom:Uint,val maxOffsetFromBottom:Uint,val viewportRows:Uint):WireValue {
    override fun validate(){validateGridScroll(offsetFromBottom,maxOffsetFromBottom,viewportRows)}
}
private fun validateGridScroll(offset:Uint,maximum:Uint,rows:Uint) {
    offset.validate();maximum.validate();rows.validate();requireWire(offset.value<=maximum.value && rows.value in 1..256)
}
@Serializable
data class GridSpan(val row:Uint,val column:Uint,val cells:List<GridCell>)
@Serializable
data class GridPart(val revision:Uint,val index:Uint,val spans:List<GridSpan>):WireValue {
    override fun validate(){
        requireWire(index.value<128 && spans.size in 1..512)
        var cells=0
        for(span in spans){cells+=span.cells.size;requireWire(span.row.value<256 && span.column.value<512 && span.cells.size in 1..512 && cells<=512);span.cells.forEach {it.validate()}}
    }
}
@Serializable
data class GridCommit(val revision:Uint):WireValue {override fun validate(){}}
@Serializable
data class GridBeginEvent(val frame:GridBegin):WireValue {override fun validate(){frame.validate()}}

@Serializable
data class WorktreesListParams(
    val target: Target,
) : WireValue {
    override fun validate() { target.validateWire() }
}

@Serializable
data class WorktreeEntry(
    val path: Text,
    val label: Text,
    val branch: Text? = null,
    val head: Text? = null,
    val isBare: Boolean,
    val isDetached: Boolean,
    val isLocked: Boolean,
    val isPrunable: Boolean,
    val isLinkedWorktree: Boolean? = null,
) : WireValue {
    override fun validate() {
        path.validateWire(); label.validateWire(); branch.validateWire(); head.validateWire()
        isBare.validateWire(); isDetached.validateWire(); isLocked.validateWire(); isPrunable.validateWire(); isLinkedWorktree.validateWire()
        validateWorktreeText(path, 4096, true)
        validateWorktreeText(label, 1024, false)
        branch?.let { validateWorktreeText(it, 1024, false) }
        head?.let { validateWorktreeText(it, 1024, false) }
    }
}

@Serializable
data class WorktreesListResult(
    val target: Target,
    val worktrees: List<WorktreeEntry>,
) : WireValue {
    override fun validate() {
        target.validateWire(); worktrees.validateWire()
        requireWire(worktrees.size <= 128)
    }
}

@Serializable
data class ChangesReadParams(val target: Target) : WireValue {
    override fun validate() { target.validateWire() }
}

@Serializable
enum class ChangeState { @SerialName("unchanged") Unchanged, @SerialName("added") Added, @SerialName("modified") Modified, @SerialName("deleted") Deleted, @SerialName("renamed") Renamed, @SerialName("copied") Copied, @SerialName("type_changed") TypeChanged, @SerialName("unmerged") Unmerged }

@Serializable
data class ChangeSubmodule(val commitChanged: Boolean, val trackedDirty: Boolean, val untrackedDirty: Boolean) : WireValue {
    override fun validate() { commitChanged.validateWire(); trackedDirty.validateWire(); untrackedDirty.validateWire() }
}

@Serializable
sealed interface ChangeEntry : WireValue {
    val path: Text
    @Serializable @SerialName("tracked")
    data class Tracked(
        override val path: Text, val originalPath: Text? = null, val index: ChangeState, val worktree: ChangeState,
        val submodule: ChangeSubmodule? = null, val stagedBinary: Boolean, val unstagedBinary: Boolean,
    ) : ChangeEntry {
        override fun validate() {
            path.validateWire(); originalPath.validateWire(); index.validateWire(); worktree.validateWire(); submodule.validateWire(); stagedBinary.validateWire(); unstagedBinary.validateWire()
            validateWorktreeText(path, 4096, false); originalPath?.let { validateWorktreeText(it, 4096, false) }
            requireWire((index == ChangeState.Renamed || index == ChangeState.Copied || worktree == ChangeState.Renamed || worktree == ChangeState.Copied) == (originalPath != null))
        }
    }
    @Serializable @SerialName("untracked") data class Untracked(override val path: Text) : ChangeEntry {
        override fun validate() { path.validateWire(); validateWorktreeText(path, 4096, false) }
    }
}

@Serializable
data class ChangesRepositoryContext(val unborn:Boolean,val upstream:ChangesUpstream?=null,val latestCommit:ChangesLatestCommit?=null):WireValue {
    override fun validate(){upstream.validateWire();latestCommit.validateWire()}
}
@Serializable
data class ChangesUpstream(val name:Text,val ahead:Uint?=null,val behind:Uint?=null):WireValue {
    override fun validate(){validateWorktreeText(name,1024,false);ahead.validateWire();behind.validateWire();requireWire((ahead==null)==(behind==null))}
}
@Serializable
data class ChangesLatestCommit(val oid:Text,val subject:Text):WireValue {
    override fun validate(){requireWire(validChangesOid(oid) && validUnicode(subject) && utf8Size(subject)<=1024 && subject.none {it.isISOControl()})}
}
private fun validChangesOid(value:String)=value.length in setOf(40,64) && value.all {it in '0'..'9' || it in 'a'..'f'}

@Serializable
data class ChangesInfo(
    val branch: Text? = null, val head: Text? = null, val detached: Boolean,
    val entries: List<ChangeEntry>, val stagedPatch: Text, val unstagedPatch: Text,
    val repositoryContext:ChangesRepositoryContext?=null,
) : WireValue {
    override fun validate() {
        branch.validateWire(); head.validateWire(); detached.validateWire(); entries.validateWire(); stagedPatch.validateWire(); unstagedPatch.validateWire()
        repositoryContext?.let {context->
            context.validate();requireWire(!detached || context.upstream==null)
            if(context.unborn)requireWire(!detached && branch!=null && head==null && context.latestCommit==null && context.upstream?.let {it.ahead==null && it.behind==null}!=false)
            else requireWire(head!=null && validChangesOid(head) && context.latestCommit?.oid==head)
        }
        requireWire(entries.size <= 128 && (!detached || branch == null)); branch?.let { validateWorktreeText(it, 1024, false) }; head?.let { validateWorktreeText(it, 1024, false) }
        validatePatchText(stagedPatch); validatePatchText(unstagedPatch); requireWire(entries.map { it.path }.distinct().size == entries.size)
    }
}

@Serializable
data class ChangesReadResult(val target: Target, val changes: ChangesInfo) : WireValue {
    override fun validate() { target.validateWire(); changes.validateWire() }
}

private fun validatePatchText(value: Text) { requireWire(utf8Size(value) <= 256 * 1024 && validUnicode(value) && value.all { !it.isISOControl() || it == '\n' || it == '\r' || it == '\t' }) }

private fun validateWorktreeText(value: Text, maxBytes: Int, absolute: Boolean) {
    requireWire(value.isNotEmpty() && validUnicode(value) && utf8Size(value) <= maxBytes && value.none { it.isISOControl() } && (!absolute || value.startsWith('/')))
}

@Serializable
data class NotificationsSyncParams(
    val afterSequence: Uint? = null,
    val limit: Uint? = null,
) : WireValue {
    override fun validate() {
        afterSequence.validateWire()
        limit.validateWire()
        validateNotificationsSyncParams(this)
    }
}

@Serializable
data class NotificationsSyncResult(
    val events: List<AttentionEvent>,
    val read: List<ReadWatermark>,
    val latestSequence: Uint,
    val retainedFrom: Uint,
    val nextSequence: Uint? = null,
    val resetRequired: Boolean,
) : WireValue {
    override fun validate() {
        events.validateWire()
        read.validateWire()
        latestSequence.validateWire()
        retainedFrom.validateWire()
        nextSequence.validateWire()
        resetRequired.validateWire()
        validateNotificationsSyncResult(this)
    }
}

@Serializable
data class NotificationsReadParams(
    val bootId: Uuid,
    val sessionId: Uuid,
    val throughSequence: Uint,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        sessionId.validateWire()
        throughSequence.validateWire()
    }
}

@Serializable
data class NotificationsReadResult(
    val read: ReadWatermark,
) : WireValue {
    override fun validate() {
        read.validateWire()
    }
}

@Serializable
data class PushRegisterParams(
    val bootId: Uuid,
    val endpoint: Text,
    val endpointGeneration: Uuid,
    val pushKey: Key32,
    val probe: Uuid,
) : WireValue {
    override fun toString()="PushRegisterParams(redacted)"
    override fun validate() {
        bootId.validateWire()
        endpoint.validateWire()
        endpointGeneration.validateWire()
        pushKey.validateWire()
        probe.validateWire()
    }
}

@Serializable
data class PushRegisterResult(
    val registered: TrueValue,
    val endpointGeneration: Uuid,
    val probe: Uuid,
    val verified: Boolean,
) : WireValue {
    override fun validate() {
        registered.validateWire()
        endpointGeneration.validateWire()
        probe.validateWire()
        verified.validateWire()
    }
}

@Serializable
data class PushProbeAckParams(
    val bootId: Uuid,
    val endpointGeneration: Uuid,
    val probe: Uuid,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        endpointGeneration.validateWire()
        probe.validateWire()
    }
}

@Serializable
data class PushProbeAckResult(
    val verified: TrueValue,
) : WireValue {
    override fun validate() {
        verified.validateWire()
    }
}

@Serializable
data class PushUnregisterParams(
    val bootId: Uuid,
    val endpointGeneration: Uuid,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
        endpointGeneration.validateWire()
    }
}

@Serializable
data class PushUnregisterResult(
    val unregistered: Boolean,
) : WireValue {
    override fun validate() {
        unregistered.validateWire()
    }
}

@Serializable
data class DeviceUnpairParams(
    val bootId: Uuid,
) : WireValue {
    override fun validate() {
        bootId.validateWire()
    }
}

@Serializable
data class DeviceUnpairResult(
    val unpaired: TrueValue,
) : WireValue {
    override fun validate() {
        unpaired.validateWire()
    }
}

@Serializable
data class UnknownActionStatus(
    val state: UnknownValue,
) : WireValue {
    override fun validate() {
        state.validateWire()
    }
}

@Serializable
data class KnownActionStatus(
    val receipt: Receipt,
) : WireValue {
    override fun validate() {
        receipt.validateWire()
    }
}

@Serializable
class HostChangedEvent : WireValue {
    override fun validate() {
    }
}

@Serializable
data class SessionStateEvent(
    val state: SessionState,
) : WireValue {
    override fun validate() {
        state.validateWire()
    }
}

@Serializable
data class HistoryUpsertEvent(
    val records: List<HistoryRecord>,
) : WireValue {
    override fun validate() {
        records.validateWire()
        validateHistoryUpsertEvent(this)
    }
}

@Serializable
data class ActivityUpdateEvent(
    val activity: Activity,
) : WireValue {
    override fun validate() {
        activity.validateWire()
    }
}

@Serializable
data class RequestOpenedEvent(
    val request: PendingRequest,
) : WireValue {
    override fun validate() {
        request.validateWire()
    }
}

@Serializable
data class RequestClosedEvent(
    val requestKey: Uuid,
    val reason: RequestClosedReason,
) : WireValue {
    override fun validate() {
        requestKey.validateWire()
    }
}

@Serializable
data class ActionStatusEvent(
    val receipt: Receipt,
) : WireValue {
    override fun validate() {
        receipt.validateWire()
    }
}

@Serializable
data class NotificationUpsertEvent(
    val event: AttentionEvent,
) : WireValue {
    override fun validate() {
        event.validateWire()
    }
}

@Serializable
data class NotificationReadEvent(
    val read: ReadWatermark,
) : WireValue {
    override fun validate() {
        read.validateWire()
    }
}

@Serializable
data class SyncRequiredEvent(
    val reason: SyncReason,
) : WireValue {
    override fun validate() {
    }
}

@Serializable
sealed interface Content : WireValue

@Serializable
sealed interface Presentation : WireValue

@Serializable
sealed interface Answer : WireValue

@Serializable
@SerialName("agent_renamed")
data class ActionResultAgentRenamed(
    val target: Target,
    val terminalId: Id,
) : ActionResult {
    override fun validate() { target.validateWire(); terminalId.validateWire() }
}

@Serializable
@SerialName("workspace_renamed")
data class ActionResultWorkspaceRenamed(
    val target: Target,
    val workspaceId: Id,
) : ActionResult {
    override fun validate() { target.validateWire(); workspaceId.validateWire() }
}

@Serializable
sealed interface ActionResult : WireValue

@Serializable(with = FieldValueSerializer::class)
sealed interface FieldValue {
    data class Text(val value: String) : FieldValue
    data class Flag(val value: Boolean) : FieldValue
    data class Multi(val values: List<String>) : FieldValue
}

sealed interface RequestBody {
    data class SessionControlsRead(val params:SessionControlsReadParams):RequestBody {override val method=Method.SessionControlsRead}
    data class SessionControlsApply(val params:SessionControlsApplyParams):RequestBody {override val method=Method.SessionControlsApply}
    data class SessionQueueList(val params:SessionQueueListParams):RequestBody {override val method=Method.SessionQueueList}
    data class SessionQueueUpdate(val params:SessionQueueUpdateParams):RequestBody {override val method=Method.SessionQueueUpdate}
    data class SessionQueueDelete(val params:SessionQueueDeleteParams):RequestBody {override val method=Method.SessionQueueDelete}
    data class SessionQueueReorder(val params:SessionQueueReorderParams):RequestBody {override val method=Method.SessionQueueReorder}
    data class SessionQueueStart(val params:SessionQueueStartParams):RequestBody {override val method=Method.SessionQueueStart}
    data class ArtifactsList(val params:ArtifactsListParams):RequestBody{override val method=Method.ArtifactsList}
    data class ArtifactsOpen(val params:ArtifactsOpenParams):RequestBody{override val method=Method.ArtifactsOpen}
    data class ArtifactsRead(val params:ArtifactsReadParams):RequestBody{override val method=Method.ArtifactsRead}
    data class ArtifactsClose(val params:ArtifactsCloseParams):RequestBody{override val method=Method.ArtifactsClose}
    data class TerminalCreate(val params:TerminalCreateParams):RequestBody {override val method=Method.TerminalCreate}
    data class AttachmentsBegin(val params:AttachmentsBeginParams):RequestBody {override val method=Method.AttachmentsBegin}
    data class AttachmentsChunk(val params:AttachmentsChunkParams):RequestBody {override val method=Method.AttachmentsChunk}
    data class AttachmentsComplete(val params:AttachmentsCompleteParams):RequestBody {override val method=Method.AttachmentsComplete}
    data class AttachmentsDiscard(val params:AttachmentsDiscardParams):RequestBody {override val method=Method.AttachmentsDiscard}
    data class AttachmentsStatus(val params:AttachmentsStatusParams):RequestBody {override val method=Method.AttachmentsStatus}
    val method: Method
    data class PairConfirm(val params: PairConfirmParams) : RequestBody { override val method = Method.PairConfirm }
    data class Hello(val params: HelloParams) : RequestBody { override val method = Method.Hello }
    data class HostSnapshot(val params: HostSnapshotParams) : RequestBody { override val method = Method.HostSnapshot }
    data class SessionOpen(val params: SessionOpenParams) : RequestBody { override val method = Method.SessionOpen }
    data class SessionHistory(val params: SessionHistoryParams) : RequestBody { override val method = Method.SessionHistory }
    data class SessionUnsubscribe(val params: SessionUnsubscribeParams) : RequestBody { override val method = Method.SessionUnsubscribe }
    data class CreationDirectories(val params:CreationDirectoriesParams):RequestBody {override val method=Method.CreationDirectories}
    data class SessionCreate(val params: SessionCreateParams) : RequestBody { override val method = Method.SessionCreate }
    data class AgentRename(val params: AgentRenameParams) : RequestBody { override val method = Method.AgentRename }
    data class WorkspaceRename(val params: WorkspaceRenameParams) : RequestBody { override val method = Method.WorkspaceRename }
    data class PromptSubmit(val params: PromptSubmitParams) : RequestBody { override val method = Method.PromptSubmit }
    data class TurnCancel(val params: TurnCancelParams) : RequestBody { override val method = Method.TurnCancel }
    data class RequestRespond(val params: RequestRespondParams) : RequestBody { override val method = Method.RequestRespond }
    data class ActionStatus(val params: ActionStatusParams) : RequestBody { override val method = Method.ActionStatus }
    data class ContentRead(val params: ContentReadParams) : RequestBody { override val method = Method.ContentRead }
    data class TerminalGridSubscribe(val params:TerminalGridSubscribeParams):RequestBody {override val method=Method.TerminalGridSubscribe}
    data class ShellGridSubscribe(val params:ShellGridSubscribeParams):RequestBody {override val method=Method.ShellGridSubscribe}
    data class ShellInput(val params: ShellInputParams) : RequestBody { override val method = Method.ShellInput }
    data class ShellResize(val params:ShellResizeParams):RequestBody {override val method=Method.ShellResize}
    data class ShellScroll(val params:ShellScrollParams):RequestBody {override val method=Method.ShellScroll}
    data class WorktreesList(val params: WorktreesListParams) : RequestBody { override val method = Method.WorktreesList }
    data class ChangesRead(val params: ChangesReadParams) : RequestBody { override val method = Method.ChangesRead }
    data class SkillsList(val params: SkillsListParams) : RequestBody { override val method = Method.SkillsList }
    data class SkillsPreviewOpen(val params:SkillsPreviewOpenParams):RequestBody{override val method=Method.SkillsPreviewOpen}
    data class SkillsPreviewRead(val params:SkillsPreviewReadParams):RequestBody{override val method=Method.SkillsPreviewRead}
    data class SkillsPreviewClose(val params:SkillsPreviewCloseParams):RequestBody{override val method=Method.SkillsPreviewClose}

    data class NotificationsSync(val params: NotificationsSyncParams) : RequestBody { override val method = Method.NotificationsSync }
    data class NotificationsRead(val params: NotificationsReadParams) : RequestBody { override val method = Method.NotificationsRead }
    data class PushRegister(val params: PushRegisterParams) : RequestBody { override val method = Method.PushRegister }
    data class PushProbeAck(val params: PushProbeAckParams) : RequestBody { override val method = Method.PushProbeAck }
    data class PushUnregister(val params: PushUnregisterParams) : RequestBody { override val method = Method.PushUnregister }
    data class DeviceUnpair(val params: DeviceUnpairParams) : RequestBody { override val method = Method.DeviceUnpair }
}

sealed interface ResultBody {
    data class SessionControlsRead(val result:SessionControlsReadResult):ResultBody
    data class SessionControlsApply(val result:SessionControlsApplyResult):ResultBody
    data class SessionQueueList(val result:SessionQueueListResult):ResultBody
    data class SessionQueueUpdate(val result:SessionQueueResult):ResultBody
    data class SessionQueueDelete(val result:SessionQueueResult):ResultBody
    data class SessionQueueReorder(val result:SessionQueueResult):ResultBody
    data class SessionQueueStart(val result:SessionQueueResult):ResultBody
    data class ArtifactsList(val result:ArtifactsListResult):ResultBody
    data class ArtifactsOpen(val result:ArtifactsOpenResult):ResultBody
    data class ArtifactsRead(val result:ArtifactsReadResult):ResultBody
    data class ArtifactsClose(val result:ArtifactsCloseResult):ResultBody
    data class TerminalCreate(val result:TerminalCreateResult):ResultBody
    data class AttachmentsBegin(val result:AttachmentsResult):ResultBody
    data class AttachmentsChunk(val result:AttachmentsResult):ResultBody
    data class AttachmentsComplete(val result:AttachmentsResult):ResultBody
    data class AttachmentsDiscard(val result:AttachmentsResult):ResultBody
    data class AttachmentsStatus(val result:AttachmentsStatusResult):ResultBody
    data class PairConfirm(val result: PairConfirmResult) : ResultBody
    data class Hello(val result: HelloResult) : ResultBody
    data class HostSnapshot(val result: HostSnapshotResult) : ResultBody
    data class SessionOpen(val result: SessionOpenResult) : ResultBody
    data class SessionHistory(val result: SessionHistoryResult) : ResultBody
    data class SessionUnsubscribe(val result: SessionUnsubscribeResult) : ResultBody
    data class CreationDirectories(val result:CreationDirectoriesResult):ResultBody
    data class SessionCreate(val result: SessionCreateResult) : ResultBody
    data class AgentRename(val result: AgentRenameResult) : ResultBody
    data class WorkspaceRename(val result: WorkspaceRenameResult) : ResultBody
    data class PromptSubmit(val result: PromptSubmitResult) : ResultBody
    data class TurnCancel(val result: TurnCancelResult) : ResultBody
    data class RequestRespond(val result: RequestRespondResult) : ResultBody
    data class ActionStatus(val result: ActionStatusResult) : ResultBody
    data class ContentRead(val result: ContentReadResult) : ResultBody
    data class TerminalGridSubscribe(val result:TerminalGridSubscribeResult):ResultBody
    data class ShellGridSubscribe(val result:ShellGridSubscribeResult):ResultBody
    data class ShellInput(val result: ShellInputResult) : ResultBody
    data class ShellResize(val result:ShellResizeResult):ResultBody
    data class ShellScroll(val result:ShellScrollResult):ResultBody
    data class WorktreesList(val result: WorktreesListResult) : ResultBody
    data class ChangesRead(val result: ChangesReadResult) : ResultBody
    data class SkillsList(val result: SkillsListResult) : ResultBody
    data class SkillsPreviewOpen(val result:SkillsPreviewOpenResult):ResultBody
    data class SkillsPreviewRead(val result:SkillsPreviewReadResult):ResultBody
    data class SkillsPreviewClose(val result:SkillsPreviewCloseResult):ResultBody

    data class NotificationsSync(val result: NotificationsSyncResult) : ResultBody
    data class NotificationsRead(val result: NotificationsReadResult) : ResultBody
    data class PushRegister(val result: PushRegisterResult) : ResultBody
    data class PushProbeAck(val result: PushProbeAckResult) : ResultBody
    data class PushUnregister(val result: PushUnregisterResult) : ResultBody
    data class DeviceUnpair(val result: DeviceUnpairResult) : ResultBody
}
sealed interface ActionStatusResult { data class Known(val value: KnownActionStatus) : ActionStatusResult; data class Unknown(val value: UnknownActionStatus) : ActionStatusResult }

sealed interface EventBody {
    data class ControlsChanged(val data:ControlsChangedEvent):EventBody
    data class QueueChanged(val data:QueueChangedEvent):EventBody
    data class GridBegin(val data:GridBeginEvent):EventBody
    data class GridPart(val data:io.github.trevarj.goatr.GridPart):EventBody
    data class GridCommit(val data:io.github.trevarj.goatr.GridCommit):EventBody
    data class HostChanged(val data: HostChangedEvent) : EventBody
    data class SessionState(val data: SessionStateEvent) : EventBody
    data class HistoryUpsert(val data: HistoryUpsertEvent) : EventBody
    data class ActivityUpdate(val data: ActivityUpdateEvent) : EventBody
    data class RequestOpened(val data: RequestOpenedEvent) : EventBody
    data class RequestClosed(val data: RequestClosedEvent) : EventBody
    data class ActionStatus(val data: ActionStatusEvent) : EventBody
    data class NotificationUpsert(val data: NotificationUpsertEvent) : EventBody
    data class NotificationRead(val data: NotificationReadEvent) : EventBody
    data class SyncRequired(val data: SyncRequiredEvent) : EventBody
}

@Serializable
enum class CheckoutKind { @SerialName("new") New, @SerialName("existing") Existing }
@Serializable
data class CreationCheckoutResources(val kind: CheckoutKind, val branch: Text, val requestedBase: Text? = null, val resolvedCommit: Text? = null) : WireValue {
    override fun validate() {
        validateWorktreeText(branch,1024,false)
        requestedBase?.let {validateWorktreeText(it,1024,false)}
        requireWire((kind==CheckoutKind.New)==(requestedBase!=null))
        resolvedCommit?.let {requireWire(it.length in setOf(40,64) && it.all {c->c in '0'..'9'||c in 'a'..'f'})}
    }
}

@Serializable
data class SkillsListParams(val target:Target):WireValue { override fun validate() {target.validateWire()} }
@Serializable
data class SkillEntry(val name:Text,val description:Text,val path:Text?=null,val scope:Text,val enabled:Boolean,val displayName:Text?=null,val shortDescription:Text?=null,val commandName:Text?=null,val documentRef:Uuid?=null,val invocationRef:Uuid?=null):WireValue {
    override fun validate() {
        name.validateWire();description.validateWire();path.validateWire();scope.validateWire();displayName.validateWire();shortDescription.validateWire();commandName.validateWire();documentRef.validateWire()
        invocationRef.validateWire();requireWire(invocationRef==null || enabled&&commandName==null)
        requireWire(commandName?.let{it.isNotEmpty()&&utf8Size(it)<=256&&!it.any{char->Character.isISOControl(char)}}!=false&&(documentRef==null||enabled))
        requireWire(name.isNotEmpty() && name.toByteArray().size<=256 && description.toByteArray().size<=16384 && (path==null || path.isNotEmpty() && path.toByteArray().size<=4096))
        requireWire(scope in listOf("user","repo","system","admin","enabled") && (displayName?.toByteArray()?.size?:0)<=1024 && (shortDescription?.toByteArray()?.size?:0)<=4096)
    }
}
/** The pinned native grammar has no safe escaping or bare-name fallback. */
fun SkillEntry.exactInsertText():String? {
    commandName?.let{command->val name=command.removePrefix("skill:");return if(enabled&&command.startsWith("skill:")&&name.isNotEmpty()&&name.length<=128&&name.all{it in 'a'..'z'||it in 'A'..'Z'||it in '0'..'9'||it in "._-"})"/$command" else null}
    val path=path ?: return null
    if(!enabled || name.isEmpty() || name.any {it !in 'a'..'z' && it !in 'A'..'Z' && it !in '0'..'9' && it !in "_:-"} || listOf("PATH","HOME","USER","SHELL","PWD","TMPDIR","TEMP","TMP","LANG","TERM","XDG_CONFIG_HOME").any {it.equals(name,true)} || !path.startsWith('/') || path.trim()!=path || ')' in path || path.any {it.isISOControl()}) return null
    return "[\$$name]($path)"
}
@Serializable
data class SkillLoadError(val path:Text,val message:Text):WireValue {override fun validate() {path.validateWire();message.validateWire();requireWire(path.toByteArray().size<=4096 && message.toByteArray().size<=4096)} }
@Serializable
data class SkillsListResult(val target:Target,val catalogueGeneration:Uuid,val skills:List<SkillEntry>,val errors:List<SkillLoadError>,val provider:Provider=Provider.Codex):WireValue {
    override fun validate() {
        target.validateWire();catalogueGeneration.validateWire();requireWire(skills.size<=128&&errors.size<=32)
        skills.validateWire();errors.validateWire()
        var invocations:HashSet<Uuid>?=null;var documents:HashSet<Uuid>?=null
        val commands=if(provider==Provider.Omp)HashSet<Text>()else null
        for(entry in skills) {
            entry.invocationRef?.let {reference->requireWire(provider==Provider.Codex&&entry.enabled);val refs=invocations?:HashSet<Uuid>().also{invocations=it};requireWire(refs.add(reference))}
            entry.documentRef?.let {reference->val refs=documents?:HashSet<Uuid>().also{documents=it};requireWire(refs.add(reference))}
            requireWire(when(provider) {
                Provider.Codex->entry.path!=null&&entry.commandName==null&&entry.scope!="enabled"
                Provider.Omp->errors.isEmpty()&&entry.path==null&&entry.enabled&&entry.commandName!=null&&entry.scope=="enabled"
                    &&commands!!.add(entry.commandName)&&entry.commandName.startsWith("skill:")&&entry.commandName.removePrefix("skill:")==entry.name
            })
        }
        requireWire(Protocol.canonicalJsonSize(Protocol.strictJson.encodeToJsonElement(this))<=256L*1024)
    }
}


@Serializable
data class SkillsPreviewOpenParams(val target:Target,val catalogueGeneration:Uuid,val documentRef:Uuid):WireValue {override fun validate(){target.validateWire();catalogueGeneration.validateWire();documentRef.validateWire()}}
@Serializable
data class SkillsPreviewOpenResult(val target:Target,val catalogueGeneration:Uuid,val documentRef:Uuid,val readId:Uuid,val size:Uint,val sha256:String):WireValue {override fun validate(){target.validateWire();catalogueGeneration.validateWire();documentRef.validateWire();readId.validateWire();size.validateWire();requireWire(size.value<=1048576&&sha256.matches(Regex("[a-f0-9]{64}")))}}
@Serializable
data class SkillsPreviewReadParams(val target:Target,val catalogueGeneration:Uuid,val readId:Uuid,val offset:Uint,val limit:Uint):WireValue {override fun validate(){target.validateWire();catalogueGeneration.validateWire();readId.validateWire();offset.validateWire();limit.validateWire();requireWire(offset.value<=1048576&&limit.value in 1..65536)}}
@Serializable
data class SkillsPreviewReadResult(val target:Target,val catalogueGeneration:Uuid,val readId:Uuid,val offset:Uint,val data:Base64Bytes,val eof:Boolean):WireValue {override fun toString()="SkillsPreviewReadResult(redacted)";override fun validate(){target.validateWire();catalogueGeneration.validateWire();readId.validateWire();offset.validateWire();data.validateWire();requireWire(offset.value<=1048576&&java.util.Base64.getDecoder().decode(data.value).size<=65536)}}
@Serializable
data class SkillsPreviewCloseParams(val target:Target,val catalogueGeneration:Uuid,val readId:Uuid):WireValue {override fun validate(){target.validateWire();catalogueGeneration.validateWire();readId.validateWire()}}
@Serializable
data class SkillsPreviewCloseResult(val closed:Boolean):WireValue {override fun validate(){}}

@Serializable
data class ShellTarget(val shellId:Uuid,val generation:Uuid):WireValue {
    override fun validate(){shellId.validateWire();generation.validateWire()}
}
@Serializable
data class ShellCapabilities(val read:Boolean,val create:Boolean,val input:Boolean,val close:Boolean,val artifacts:Boolean=false,val grid:Boolean=false):WireValue {
    override fun validate(){requireWire(!close && (!(create || input || grid) || read))}
}
@Serializable
data class ShellSummary(val target:ShellTarget,val serverId:Uuid,val workspaceId:Id,val tabId:Id,val paneId:Id,val terminalId:Id,val title:Label,val connectivity:SessionConnectivity,val capabilities:ShellCapabilities):WireValue {
    override fun validate(){target.validateWire();serverId.validateWire();workspaceId.validateWire();tabId.validateWire();paneId.validateWire();terminalId.validateWire();title.validateWire();capabilities.validateWire();requireWire((connectivity==SessionConnectivity.Ready || connectivity==SessionConnectivity.Offline) && (connectivity==SessionConnectivity.Ready || !capabilities.read && !capabilities.create && !capabilities.input && !capabilities.artifacts && !capabilities.grid))}
}
internal fun ShellSummary.sameLogicalIdentity(other:ShellSummary):Boolean =
    target==other.target && serverId==other.serverId && workspaceId==other.workspaceId &&
    tabId==other.tabId && paneId==other.paneId && terminalId==other.terminalId
@Serializable
data class ShellGridSubscribeParams(val target:ShellTarget,val history:Boolean=false):WireValue {override fun validate(){target.validateWire()}}
@Serializable
data class ShellGridSubscribeResult(val target:ShellTarget,val subscription:Uuid,val seq:ZeroValue,val frame:GridBegin):WireValue {override fun validate(){target.validateWire();subscription.validateWire();seq.validateWire();frame.validate();requireWire(frame.revision.value==0L)}}
@Serializable
data class ShellResizeParams(val bootId:Uuid,val target:ShellTarget,val subscription:Uuid,val columns:Uint,val rows:Uint):WireValue {
    override fun validate(){bootId.validate();target.validate();subscription.validate();validateShellDimensions(columns,rows)}
}
private fun validateShellDimensions(columns:Uint,rows:Uint){requireWire(columns.value in 1..512 && rows.value in 1..256 && columns.value*rows.value<=32768)}
@Serializable
data class ShellResizeResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validate()}}
@Serializable @SerialName("shell_resize")
data class ActionResultShellResize(val columns:Uint,val rows:Uint):ActionResult {override fun validate(){validateShellDimensions(columns,rows)}}
@Serializable
data class ShellScrollParams(val bootId:Uuid,val target:ShellTarget,val subscription:Uuid,val offsetFromBottom:Uint):WireValue {
    override fun validate(){bootId.validate();target.validate();subscription.validate();offsetFromBottom.validate()}
}
@Serializable
data class ShellScrollResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validate();requireWire(receipt.method==ActionMethod.ShellScroll)}}
@Serializable @SerialName("shell_scroll")
data class ActionResultShellScroll(val offsetFromBottom:Uint,val maxOffsetFromBottom:Uint,val viewportRows:Uint):ActionResult {
    override fun validate(){validateGridScroll(offsetFromBottom,maxOffsetFromBottom,viewportRows)}
}
@Serializable
data class ShellCreationResources(val serverId:Uuid?=null,val workspaceId:Id?=null,val tabId:Id?=null,val paneId:Id?=null,val terminalId:Id?=null,val createdTarget:ShellTarget?=null):WireValue {
    fun complete()=serverId!=null && workspaceId!=null && tabId!=null && paneId!=null && terminalId!=null
    override fun validate(){serverId.validateWire();workspaceId.validateWire();tabId.validateWire();paneId.validateWire();terminalId.validateWire();createdTarget.validateWire()}
}
@Serializable
@SerialName("shell_creation")
data class ActionResultShellCreation(val resources:ShellCreationResources):ActionResult {override fun validate(){resources.validate()}}

@Serializable
sealed interface ShellInput:WireValue {
 @Serializable @SerialName("text") data class Text(val text:String):ShellInput {override fun validate(){requireWire(text.isNotEmpty()&&text.length<=16384&&validUnicode(text)&&utf8Size(text)<=16384&&text.none {it.isISOControl()&&it!='\n'&&it!='\t'})}}
 @Serializable @SerialName("keys") data class Keys(val events:List<ShellKeyEvent>):ShellInput {override fun validate(){requireWire(events.isNotEmpty() && events.size<=32);events.validateWire()}}
}
@Serializable
enum class ShellKeyName {@SerialName("character") Character,@SerialName("enter") Enter,@SerialName("tab") Tab,@SerialName("escape") Escape,@SerialName("backspace") Backspace,@SerialName("delete") Delete,@SerialName("insert") Insert,@SerialName("left") Left,@SerialName("right") Right,@SerialName("up") Up,@SerialName("down") Down,@SerialName("home") Home,@SerialName("end") End,@SerialName("page_up") PageUp,@SerialName("page_down") PageDown,@SerialName("f1") F1,@SerialName("f2") F2,@SerialName("f3") F3,@SerialName("f4") F4,@SerialName("f5") F5,@SerialName("f6") F6,@SerialName("f7") F7,@SerialName("f8") F8,@SerialName("f9") F9,@SerialName("f10") F10,@SerialName("f11") F11,@SerialName("f12") F12}
@Serializable
data class ShellKey(val key:ShellKeyName,val value:String?=null):WireValue {override fun validate(){requireWire(if(key==ShellKeyName.Character)value!=null&&value.length in 1..2&&validUnicode(value)&&value.codePointCount(0,value.length)==1&&!Character.isISOControl(value.codePointAt(0)) else value==null)}}
@Serializable
data class ShellKeyModifiers(val shift:Boolean,val ctrl:Boolean,val alt:Boolean,@SerialName("super")val superKey:Boolean)
@Serializable
enum class ShellKeyPhase {@SerialName("press") Press,@SerialName("repeat") Repeat,@SerialName("release") Release}
@Serializable
data class ShellKeyEvent(val key:ShellKey,val modifiers:ShellKeyModifiers,val phase:ShellKeyPhase,val physicalKeyId:Uint?=null):WireValue {
 override fun validate(){key.validate();physicalKeyId.validateWire();requireWire(physicalKeyId==null || physicalKeyId.value<=0xffff_ffffL)}
}
@Serializable
data class ShellInputParams(val bootId:Uuid,val target:ShellTarget,val subscription:Uuid,val input:ShellInput):WireValue {
 override fun validate(){bootId.validateWire();target.validateWire();subscription.validateWire();input.validate()}
}
@Serializable
data class ShellInputResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validate()}}
@Serializable
data class ShellInputProgress(val encodedBytes:Int?=null,val writtenBytes:Int?=null,val flushCompleted:Boolean?=null):WireValue {
 fun accepted()=encodedBytes!=null && encodedBytes>0 && writtenBytes==encodedBytes && flushCompleted==true
 override fun validate(){requireWire((encodedBytes==null || encodedBytes in 1..16396) && (writtenBytes==null || encodedBytes!=null && writtenBytes in 0..encodedBytes) && (flushCompleted!=true || accepted()))}
 fun retains(old:ShellInputProgress)=(old.encodedBytes==null || encodedBytes==old.encodedBytes) && (old.writtenBytes==null || writtenBytes!=null && writtenBytes>=old.writtenBytes) && (old.flushCompleted!=true || flushCompleted==true)
}
@Serializable @SerialName("shell_input")
data class ActionResultShellInput(val progress:ShellInputProgress):ActionResult {override fun validate(){progress.validate()}}

@Serializable
data class ShellDispatchProgress(val frameBytes:Int?=null,val socketBytesWritten:Int?=null,val frameComplete:Boolean?=null):WireValue {
 fun complete()=frameBytes!=null && frameBytes>4 && socketBytesWritten==frameBytes && frameComplete==true
 override fun validate(){requireWire((frameBytes==null || frameBytes in 5..2_097_156)
     &&(socketBytesWritten==null || frameBytes!=null && socketBytesWritten in 0..frameBytes)
     &&(frameComplete!=true || complete()))}
 fun retains(old:ShellDispatchProgress)=(old.frameBytes==null || frameBytes==old.frameBytes)
     &&(old.socketBytesWritten==null || socketBytesWritten!=null && socketBytesWritten>=old.socketBytesWritten)
     &&(old.frameComplete!=true || frameComplete==true)
}
@Serializable @SerialName("shell_dispatch")
data class ActionResultShellDispatch(val progress:ShellDispatchProgress):ActionResult {override fun validate(){progress.validate()}}

/** Only this content-free complete-frame outcome settles a new terminal input handoff. */
fun Receipt.shellDispatched()=method==ActionMethod.ShellInput && state==ReceiptState.Dispatched
    &&(result as? ActionResultShellDispatch)?.progress?.complete()==true

@Serializable
data class WorktreeRemovalProgress(val effectStarted:Boolean?=null,val gitRemoveSucceeded:Boolean?=null,val checkoutAbsent:Boolean?=null,val registrationAbsent:Boolean?=null):WireValue {
    fun complete()=effectStarted==true && gitRemoveSucceeded==true && checkoutAbsent==true && registrationAbsent==true
    fun retains(old:WorktreeRemovalProgress):Boolean {
        fun keeps(old:Boolean?,new:Boolean?)=old==null || new!=null && (old!=true || new==true)
        return keeps(old.effectStarted,effectStarted) && keeps(old.gitRemoveSucceeded,gitRemoveSucceeded) && keeps(old.checkoutAbsent,checkoutAbsent) && keeps(old.registrationAbsent,registrationAbsent)
    }
    override fun validate(){requireWire(gitRemoveSucceeded!=true || effectStarted==true)}
}
@Serializable
@SerialName("worktree_removal")
data class ActionResultWorktreeRemoval(val target:Target,val candidateId:Uuid,val previewId:Uuid,val progress:WorktreeRemovalProgress):ActionResult {override fun validate(){target.validateWire();candidateId.validateWire();previewId.validateWire();progress.validateWire()}}

private fun attachmentDigest(value:String){requireWire(value.matches(Regex("[0-9a-f]{64}")))}
private fun attachmentLabel(value:String){requireWire(validUnicode(value) && value.toByteArray(Charsets.UTF_8).size in 1..128 && value.none {it.isISOControl()})}
@Serializable
data class AttachmentsBeginParams(val bootId:Uuid,val target:Target,val uploadId:Uuid,val displayName:Text,val mediaType:Text,val size:Uint,val sha256:Text):WireValue {
    override fun toString()="AttachmentsBeginParams(redacted)"
    override fun validate(){bootId.validate();target.validate();uploadId.validate();attachmentLabel(displayName);attachmentLabel(mediaType);requireWire(size.value in 1..8L*1024*1024 && displayName.none {it=='/'||it=='\\'});attachmentDigest(sha256)}
}
@Serializable
data class AttachmentsChunkParams(val bootId:Uuid,val target:Target,val uploadId:Uuid,val offset:Uint,val data:Text,val sha256:Text):WireValue {
    override fun toString()="AttachmentsChunkParams(redacted)"
    override fun validate(){bootId.validate();target.validate();uploadId.validate();attachmentDigest(sha256);requireWire(offset.value<8L*1024*1024 && data.length<=174764)
        val bytes=try{java.util.Base64.getDecoder().decode(data)}catch(_:Exception){invalidWire()}
        try {requireWire(bytes.size in 1..128*1024 && java.util.Base64.getEncoder().encodeToString(bytes)==data && Protocol.sha256(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}==sha256)}finally{bytes.fill(0)}
    }
}
@Serializable data class AttachmentsCompleteParams(val bootId:Uuid,val target:Target,val uploadId:Uuid):WireValue {override fun validate(){bootId.validate();target.validate();uploadId.validate()}}
@Serializable data class AttachmentsDiscardParams(val bootId:Uuid,val target:Target,val uploadId:Uuid):WireValue {override fun validate(){bootId.validate();target.validate();uploadId.validate()}}
@Serializable data class AttachmentsStatusParams(val target:Target,val uploadId:Uuid):WireValue {override fun validate(){target.validate();uploadId.validate()}}
@Serializable data class AttachmentProgress(val uploadId:Uuid,val committedBytes:Uint,val expiresAt:Uint?=null,val providerPath:Text?=null,val published:Boolean?=null,val discarded:Boolean?=null):WireValue {
    override fun validate(){uploadId.validate();requireWire(committedBytes.value<=8L*1024*1024);expiresAt?.validate();providerPath?.let {requireWire(validUnicode(it)&&it.toByteArray(Charsets.UTF_8).size<=4096&&it.startsWith('/')&&it.none(Char::isISOControl)&&published==true)}}
    fun retains(old:AttachmentProgress)=uploadId==old.uploadId && committedBytes.value>=old.committedBytes.value && (old.expiresAt==null||expiresAt==old.expiresAt||old.published!=true&&published==true&&providerPath!=null&&expiresAt!=null&&expiresAt.value>old.expiresAt.value) && (old.providerPath==null||providerPath==old.providerPath) && (old.published==null||published==old.published||old.published==false&&published==true) && (old.discarded==null||discarded==old.discarded||old.discarded==false&&discarded==true)
}
@Serializable @SerialName("attachment") data class ActionResultAttachment(val target:Target,val progress:AttachmentProgress):ActionResult {override fun validate(){target.validate();progress.validate()}}
@Serializable enum class AttachmentUploadState {@SerialName("receiving") Receiving,@SerialName("complete") Complete,@SerialName("discarded") Discarded,@SerialName("uncertain") Uncertain}
@Serializable
enum class ImagePreparationReason {
    @SerialName("pending_original_read") PendingOriginalRead,
    @SerialName("original_source_unavailable") OriginalSourceUnavailable,
    @SerialName("ownership_proof_unavailable") OwnershipProofUnavailable,
    @SerialName("capacity_in_use") CapacityInUse,
}
@Serializable
data class NativeImagePreparation(
    val reason:ImagePreparationReason,val actions:List<Uuid>,val peerOwners:Uint?=null,val hostOwners:Uint?=null,
):WireValue {
    override fun validate() {
        actions.validateWire();peerOwners.validateWire();hostOwners.validateWire()
        val unique=HashSet<Uuid>();requireWire(actions.size<=4&&actions.all(unique::add))
        requireWire(reason==ImagePreparationReason.OwnershipProofUnavailable||peerOwners!=null&&hostOwners!=null&&(reason==ImagePreparationReason.CapacityInUse||actions.isNotEmpty()))
        requireWire(peerOwners==null||peerOwners.value<=4&&peerOwners.value>=actions.size)
        requireWire(hostOwners==null||hostOwners.value<=16&&(peerOwners==null||peerOwners.value<=hostOwners.value))
    }
}
@Serializable
data class AttachmentsStatusResult(
    val insertionReady:Boolean=false,val target:Target,val progress:AttachmentProgress,val state:AttachmentUploadState,val size:Uint,
    val nativePreparation:NativeImagePreparation?=null,
):WireValue {
    override fun validate() {
        target.validate();progress.validate();nativePreparation.validateWire()
        requireWire(size.value in 1..8L*1024*1024&&progress.committedBytes.value<=size.value)
        requireWire(!insertionReady||state==AttachmentUploadState.Complete&&progress.published==true&&progress.discarded!=true&&progress.providerPath!=null)
    }
}
@Serializable data class AttachmentsResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validate()}}
internal fun RequestBody.attachmentTarget():Target?=when(this){is RequestBody.AttachmentsBegin->params.target;is RequestBody.AttachmentsChunk->params.target;is RequestBody.AttachmentsComplete->params.target;is RequestBody.AttachmentsDiscard->params.target;else->null}
internal fun RequestBody.attachmentUpload():Uuid?=when(this){is RequestBody.AttachmentsBegin->params.uploadId;is RequestBody.AttachmentsChunk->params.uploadId;is RequestBody.AttachmentsComplete->params.uploadId;is RequestBody.AttachmentsDiscard->params.uploadId;else->null}
internal fun RequestBody.attachmentBoot():Uuid?=when(this){is RequestBody.AttachmentsBegin->params.bootId;is RequestBody.AttachmentsChunk->params.bootId;is RequestBody.AttachmentsComplete->params.bootId;is RequestBody.AttachmentsDiscard->params.bootId;else->null}

@Serializable data class TerminalCreateParams(val bootId:Uuid,val serverId:Uuid,val workspaceId:Id,val projectId:Uuid,
 val directoryRef:Uuid?=null,val source:ArtifactSource?=null):WireValue {
 override fun validate(){bootId.validate();serverId.validate();workspaceId.validate();projectId.validate();directoryRef.validateWire();source.validateWire()}
}
@Serializable data class TerminalCreateResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validate()}}

@Serializable
sealed interface ArtifactSource:WireValue {
 @Serializable @SerialName("session") data class Session(val target:Target):ArtifactSource{override fun validate(){target.validateWire()}}
 @Serializable @SerialName("shell") data class Shell(val target:ShellTarget):ArtifactSource{override fun validate(){target.validateWire()}}
}
@Serializable enum class ArtifactKind{@SerialName("directory") Directory,@SerialName("file") File}
@Serializable enum class ArtifactPreviewKind{@SerialName("text") Text,@SerialName("binary") Binary,@SerialName("unavailable") Unavailable}
private fun artifactName(name:String){requireWire(validUnicode(name)&&name.isNotEmpty()&&utf8Size(name)<=255&&!name.any {Character.isISOControl(it)})}
@Serializable data class ArtifactEntry(val reference:Uuid,val name:String,val kind:ArtifactKind,val size:Uint?=null):WireValue{
 override fun validate(){reference.validateWire();artifactName(name);size.validateWire();requireWire((kind==ArtifactKind.File)==(size!=null))}
 override fun toString()="ArtifactEntry(redacted)"
}
@Serializable data class ArtifactsListParams(val source:ArtifactSource,val parent:Uuid?=null,val cursor:Uuid?=null,val limit:Uint):WireValue{
 override fun validate(){source.validateWire();parent.validateWire();cursor.validateWire();limit.validateWire();requireWire((parent==null||cursor==null)&&limit.value in 1..100)}
 override fun toString()="ArtifactsListParams(redacted)"
}
@Serializable data class ArtifactsListResult(val source:ArtifactSource,val entries:List<ArtifactEntry>,val nextCursor:Uuid?=null):WireValue{
 override fun validate(){source.validateWire();entries.validateWire();nextCursor.validateWire();val references=HashSet<Uuid>();requireWire(entries.size<=100&&entries.all {references.add(it.reference)})}
 override fun toString()="ArtifactsListResult(redacted)"
}
@Serializable data class ArtifactsOpenParams(val source:ArtifactSource,val file:Uuid):WireValue{
 override fun validate(){source.validateWire();file.validateWire()};override fun toString()="ArtifactsOpenParams(redacted)"
}
@Serializable data class ArtifactsOpenResult(val source:ArtifactSource,val readId:Uuid,val name:String,val size:Uint,val sha256:String,val previewKind:ArtifactPreviewKind):WireValue{
 override fun validate(){source.validateWire();readId.validateWire();artifactName(name);size.validateWire();requireWire((previewKind!=ArtifactPreviewKind.Text||size.value<=1024L*1024)&&size.value<=32L*1024*1024&&Regex("[0-9a-f]{64}").matches(sha256))}
 override fun toString()="ArtifactsOpenResult(redacted)"
}
@Serializable data class ArtifactsReadParams(val source:ArtifactSource,val readId:Uuid,val offset:Uint,val limit:Uint):WireValue{
 override fun validate(){source.validateWire();readId.validateWire();offset.validateWire();limit.validateWire();requireWire(offset.value<=32L*1024*1024&&limit.value in 1..65536)}
 override fun toString()="ArtifactsReadParams(redacted)"
}
@Serializable data class ArtifactsReadResult(val source:ArtifactSource,val readId:Uuid,val offset:Uint,val data:Base64Bytes,val eof:Boolean):WireValue{
 override fun validate(){source.validateWire();readId.validateWire();offset.validateWire();data.validateWire();requireWire(offset.value<=32L*1024*1024)}
 override fun toString()="ArtifactsReadResult(redacted)"
}
@Serializable data class ArtifactsCloseParams(val source:ArtifactSource,val readId:Uuid):WireValue{
 override fun validate(){source.validateWire();readId.validateWire()};override fun toString()="ArtifactsCloseParams(redacted)"
}
@Serializable data class ArtifactsCloseResult(val source:ArtifactSource,val readId:Uuid,val closed:Boolean):WireValue{
 override fun validate(){source.validateWire();readId.validateWire()};override fun toString()="ArtifactsCloseResult(redacted)"
}

@Serializable
enum class ControlMode {@SerialName("default") Default,@SerialName("plan") Plan}
@Serializable
enum class PromptDelivery {
    @SerialName("native") Native,@SerialName("steer") Steer,@SerialName("follow_up") FollowUp,
    @SerialName("aside") Aside,@SerialName("queue") Queue,
}
@Serializable
data class SessionControlModel(val reference:Uuid,val label:Label,val efforts:List<Text>):WireValue {
    override fun validate() {
        reference.validateWire();label.validateWire();requireWire(efforts.size<=64);efforts.validateWire()
        val unique=HashSet<Text>();requireWire(efforts.all(unique::add))
    }
}
@Serializable
data class SessionControlsCurrent(val modelRef:Uuid?=null,val effort:Text?=null,val mode:ControlMode?=null):WireValue {
    override fun validate(){modelRef.validateWire();effort.validateWire();mode.validateWire()}
}
@Serializable
data class SessionControlsReadParams(val target:Target):WireValue {override fun validate(){target.validateWire()}}
@Serializable
data class SessionControlsReadResult(
    val target:Target,val catalogueGeneration:Uuid,val models:List<SessionControlModel>,val current:SessionControlsCurrent,
    val modes:List<ControlMode>,val deliveries:List<PromptDelivery>,val queueAvailable:Boolean,
):WireValue {
    override fun validate() {
        target.validateWire();catalogueGeneration.validateWire();requireWire(models.size<=256&&modes.size<=2&&deliveries.size<=5)
        models.validateWire();current.validateWire()
        val references=HashSet<Uuid>();requireWire(models.all{references.add(it.reference)})
        requireWire(current.modelRef==null||current.modelRef in references)
        requireWire(modes.distinct().size==modes.size&&deliveries.distinct().size==deliveries.size)
        val members=13L+models.size*4L+models.sumOf{it.efforts.size.toLong()}+
            (if(current.modelRef!=null)1 else 0)+(if(current.effort!=null)1 else 0)+(if(current.mode!=null)1 else 0)+modes.size+deliveries.size
        requireWire(members<=Protocol.JSON_MEMBERS_MAX)
        requireWire(Protocol.canonicalJsonSize(Protocol.strictJson.encodeToJsonElement(this))<=256L*1024)
    }
}
@Serializable
data class SessionControlsApplyParams(
    val bootId:Uuid,val target:Target,val catalogueGeneration:Uuid,val modelRef:Uuid?=null,val effort:Text?=null,val mode:ControlMode?=null,
):WireValue {
    override fun validate() {
        bootId.validateWire();target.validateWire();catalogueGeneration.validateWire();modelRef.validateWire();effort.validateWire();mode.validateWire()
        requireWire(modelRef!=null||effort!=null||mode!=null)
    }
}
@Serializable
data class SessionControlsApplyResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validateWire()}}
@Serializable
data class ControlsChangedEvent(val target:Target):WireValue {override fun validate(){target.validateWire()}}
@Serializable
data class QueueChangedEvent(val target:Target):WireValue {override fun validate(){target.validateWire()}}
@Serializable
data class SessionQueueListParams(val target:Target,val cursor:Id?=null,val limit:Uint?=null):WireValue {
    override fun validate(){target.validateWire();cursor.validateWire();limit.validateWire();requireWire(limit==null||limit.value in 1..100)}
}
@Serializable
data class SessionQueueEntry(val id:Id,val text:Content,val editable:Boolean):WireValue {override fun validate(){id.validateWire();text.validateWire()}}
@Serializable
data class SessionQueueListResult(val target:Target,val entries:List<SessionQueueEntry>,val nextCursor:Id?=null):WireValue {
    override fun validate() {
        target.validateWire();requireWire(entries.size<=100);entries.validateWire();nextCursor.validateWire()
        val ids=HashSet<Id>();requireWire(entries.all{ids.add(it.id)})
        requireWire(Protocol.canonicalJsonSize(Protocol.strictJson.encodeToJsonElement(this))<=256L*1024)
    }
}
@Serializable
data class SessionQueueUpdateParams(val bootId:Uuid,val target:Target,val entryId:Id,val text:PromptText):WireValue {
    override fun validate(){bootId.validateWire();target.validateWire();entryId.validateWire();text.validateWire()}
}
@Serializable
data class SessionQueueDeleteParams(val bootId:Uuid,val target:Target,val entryId:Id):WireValue {
    override fun validate(){bootId.validateWire();target.validateWire();entryId.validateWire()}
}
@Serializable
data class SessionQueueStartParams(val bootId:Uuid,val target:Target,val entryId:Id):WireValue {
    override fun validate(){bootId.validateWire();target.validateWire();entryId.validateWire()}
}
@Serializable
data class SessionQueueReorderParams(val bootId:Uuid,val target:Target,val entryIds:List<Id>):WireValue {
    override fun validate(){bootId.validateWire();target.validateWire();requireWire(entryIds.size<=128);entryIds.validateWire();val ids=HashSet<Id>();requireWire(entryIds.all(ids::add))}
}
@Serializable
data class SessionQueueResult(val receipt:Receipt):WireValue {override fun validate(){receipt.validateWire()}}
@Serializable
@SerialName("queue")
data class ActionResultQueue(val entryId:Id?=null,val deleted:Boolean?=null,val nativeTurnId:Id?=null):ActionResult {
    override fun validate() {
        entryId.validateWire();deleted.validateWire();nativeTurnId.validateWire()
        requireWire((deleted==null&&nativeTurnId==null||entryId!=null)&&!(deleted!=null&&nativeTurnId!=null))
    }
}

internal fun RequestBody.queueTarget():Target?=when(this) {
    is RequestBody.SessionQueueUpdate->params.target
    is RequestBody.SessionQueueDelete->params.target
    is RequestBody.SessionQueueReorder->params.target
    is RequestBody.SessionQueueStart->params.target
    else->null
}
