// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    #[serde(rename = "omp")]
    Omp,
    #[serde(rename = "codex")]
    Codex,
}
impl WireValidate for Provider {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Availability {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "setup_required")]
    SetupRequired,
    #[serde(rename = "incompatible")]
    Incompatible,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for Availability {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServerConnectivity {
    #[serde(rename = "online")]
    Online,
    #[serde(rename = "offline")]
    Offline,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for ServerConnectivity {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PromptMode {
    #[serde(rename = "submit_or_steer")]
    SubmitOrSteer,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for PromptMode {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CancelMode {
    #[serde(rename = "current_work")]
    CurrentWork,
    #[serde(rename = "native_turn")]
    NativeTurn,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for CancelMode {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HistoryScope {
    #[serde(rename = "all_branches")]
    AllBranches,
    #[serde(rename = "thread")]
    Thread,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for HistoryScope {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HistorySource {
    #[serde(rename = "persisted_and_live")]
    PersistedAndLive,
    #[serde(rename = "live_only")]
    LiveOnly,
    #[serde(rename = "native_pages")]
    NativePages,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for HistorySource {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionStatus {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "attention")]
    Attention,
    #[serde(rename = "unknown")]
    Unknown,
}
impl WireValidate for SessionStatus {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionConnectivity {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "synchronizing")]
    Synchronizing,
    #[serde(rename = "offline")]
    Offline,
    #[serde(rename = "setup_required")]
    SetupRequired,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for SessionConnectivity {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnboundState {
    #[serde(rename = "setup_required")]
    SetupRequired,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "unavailable")]
    Unavailable,
}
impl WireValidate for UnboundState {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DesktopSelection {
    #[serde(rename = "unknown")]
    Unknown,
    #[serde(rename = "bound_launch_thread")]
    BoundLaunchThread,
}
impl WireValidate for DesktopSelection {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActivityState {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "unknown")]
    Unknown,
}
impl WireValidate for ActivityState {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mime {
    #[serde(rename = "text/plain")]
    TextPlain,
    #[serde(rename = "application/json")]
    ApplicationJson,
}
impl WireValidate for Mime {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContentUnavailableReason {
    #[serde(rename = "source_truncated")]
    SourceTruncated,
    #[serde(rename = "not_cached")]
    NotCached,
    #[serde(rename = "unsupported")]
    Unsupported,
    #[serde(rename = "too_large")]
    TooLarge,
}
impl WireValidate for ContentUnavailableReason {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "tool")]
    Tool,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "unknown")]
    Unknown,
}
impl WireValidate for Role {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordKind {
    #[serde(rename = "message")]
    Message,
    #[serde(rename = "tool")]
    Tool,
    #[serde(rename = "notice")]
    Notice,
    #[serde(rename = "unsupported")]
    Unsupported,
}
impl WireValidate for RecordKind {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordSource {
    #[serde(rename = "persisted")]
    Persisted,
    #[serde(rename = "live")]
    Live,
    #[serde(rename = "native_page")]
    NativePage,
}
impl WireValidate for RecordSource {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolStatus {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "succeeded")]
    Succeeded,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "unknown")]
    Unknown,
}
impl WireValidate for ToolStatus {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApprovalType {
    #[serde(rename = "command")]
    Command,
    #[serde(rename = "file")]
    File,
    #[serde(rename = "permissions")]
    Permissions,
}
impl WireValidate for ApprovalType {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApprovalScope {
    #[serde(rename = "once")]
    Once,
    #[serde(rename = "turn")]
    Turn,
    #[serde(rename = "session")]
    Session,
    #[serde(rename = "policy")]
    Policy,
}
impl WireValidate for ApprovalScope {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "integer")]
    Integer,
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "multi")]
    Multi,
}
impl WireValidate for FieldType {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldFormat {
    #[serde(rename = "email")]
    Email,
    #[serde(rename = "uri")]
    Uri,
    #[serde(rename = "date")]
    Date,
    #[serde(rename = "date-time")]
    DateTime,
}
impl WireValidate for FieldFormat {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReceiptState {
    #[serde(rename = "reserved")]
    Reserved,
    #[serde(rename = "dispatching")]
    Dispatching,
    #[serde(rename = "dispatched")]
    Dispatched,
    #[serde(rename = "succeeded")]
    Succeeded,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "uncertain")]
    Uncertain,
}
impl WireValidate for ReceiptState {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResponseResolution {
    #[serde(rename = "written")]
    Written,
    #[serde(rename = "acknowledged")]
    Acknowledged,
    #[serde(rename = "resolved_elsewhere_or_unknown")]
    ResolvedElsewhereOrUnknown,
}
impl WireValidate for ResponseResolution {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CreationStage {
    #[serde(rename = "workspace")]
    Workspace,
    #[serde(rename = "owner")]
    Owner,
    #[serde(rename = "thread")]
    Thread,
    #[serde(rename = "relay")]
    Relay,
    #[serde(rename = "launch")]
    Launch,
    #[serde(rename = "ready")]
    Ready,
}
impl WireValidate for CreationStage {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StageState {
    #[serde(rename = "not_started")]
    NotStarted,
    #[serde(rename = "dispatching")]
    Dispatching,
    #[serde(rename = "confirmed")]
    Confirmed,
    #[serde(rename = "uncertain")]
    Uncertain,
    #[serde(rename = "failed")]
    Failed,
}
impl WireValidate for StageState {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttentionCategory {
    #[serde(rename = "attention")]
    Attention,
    #[serde(rename = "completed")]
    Completed,
}
impl WireValidate for AttentionCategory {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequestClosedReason {
    #[serde(rename = "resolved")]
    Resolved,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "generation_changed")]
    GenerationChanged,
    #[serde(rename = "desktop_only")]
    DesktopOnly,
}
impl WireValidate for RequestClosedReason {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SyncReason {
    #[serde(rename = "overflow")]
    Overflow,
    #[serde(rename = "source_gap")]
    SourceGap,
    #[serde(rename = "generation_changed")]
    GenerationChanged,
}
impl WireValidate for SyncReason {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionMethod {
    #[serde(rename = "session.controls.apply")]
    SessionControlsApply,
    #[serde(rename = "session.queue.update")]
    SessionQueueUpdate,
    #[serde(rename = "session.queue.delete")]
    SessionQueueDelete,
    #[serde(rename = "session.queue.reorder")]
    SessionQueueReorder,
    #[serde(rename = "session.queue.start")]
    SessionQueueStart,
    #[serde(rename = "terminal.create")]
    TerminalCreate,
    #[serde(rename = "attachments.begin")]
    AttachmentsBegin,
    #[serde(rename = "attachments.chunk")]
    AttachmentsChunk,
    #[serde(rename = "attachments.complete")]
    AttachmentsComplete,
    #[serde(rename = "attachments.discard")]
    AttachmentsDiscard,
    #[serde(rename = "session.create")]
    SessionCreate,
    #[serde(rename = "shell.create")]
    ShellCreate,
    #[serde(rename = "shell.input")]
    ShellInput,
    #[serde(rename = "shell.resize")]
    ShellResize,
    #[serde(rename = "shell.scroll")]
    ShellScroll,
    #[serde(rename = "worktrees.remove")]
    WorktreesRemove,
    #[serde(rename = "agent.rename")]
    AgentRename,
    #[serde(rename = "workspace.rename")]
    WorkspaceRename,
    #[serde(rename = "prompt.submit")]
    PromptSubmit,
    #[serde(rename = "turn.cancel")]
    TurnCancel,
    #[serde(rename = "request.respond")]
    RequestRespond,
}
impl WireValidate for ActionMethod {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "invalid_request")]
    InvalidRequest,
    #[serde(rename = "unsupported_version")]
    UnsupportedVersion,
    #[serde(rename = "unsupported_method")]
    UnsupportedMethod,
    #[serde(rename = "unauthorized")]
    Unauthorized,
    #[serde(rename = "pairing_failed")]
    PairingFailed,
    #[serde(rename = "stale_host")]
    StaleHost,
    #[serde(rename = "stale_session")]
    StaleSession,
    #[serde(rename = "not_found")]
    NotFound,
    #[serde(rename = "not_ready")]
    NotReady,
    #[serde(rename = "invalid_answer")]
    InvalidAnswer,
    #[serde(rename = "already_resolved")]
    AlreadyResolved,
    #[serde(rename = "action_id_conflict")]
    ActionIdConflict,
    #[serde(rename = "action_capacity")]
    ActionCapacity,
    #[serde(rename = "content_expired")]
    ContentExpired,
    #[serde(rename = "cursor_expired")]
    CursorExpired,
    #[serde(rename = "busy")]
    Busy,
    #[serde(rename = "resource_limit")]
    ResourceLimit,
    #[serde(rename = "rate_limited")]
    RateLimited,
    #[serde(rename = "timeout")]
    Timeout,
    #[serde(rename = "backend_error")]
    BackendError,
    #[serde(rename = "push_rejected")]
    PushRejected,
    #[serde(rename = "stale_registration")]
    StaleRegistration,
}
impl WireValidate for ErrorCode {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubmitAction {
    #[serde(rename = "submit")]
    Submit,
    #[serde(rename = "cancel")]
    Cancel,
}
impl WireValidate for SubmitAction {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormAction {
    #[serde(rename = "accept")]
    Accept,
    #[serde(rename = "decline")]
    Decline,
    #[serde(rename = "cancel")]
    Cancel,
}
impl WireValidate for FormAction {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UrlAction {
    #[serde(rename = "accept")]
    Accept,
    #[serde(rename = "decline")]
    Decline,
    #[serde(rename = "cancel")]
    Cancel,
}
impl WireValidate for UrlAction {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PresentationKind {
    #[serde(rename = "omp_select")]
    OmpSelect,
    #[serde(rename = "omp_editor")]
    OmpEditor,
    #[serde(rename = "codex_approval")]
    CodexApproval,
    #[serde(rename = "codex_questions")]
    CodexQuestions,
    #[serde(rename = "codex_form")]
    CodexForm,
    #[serde(rename = "codex_url")]
    CodexUrl,
    #[serde(rename = "desktop_only")]
    DesktopOnly,
}
impl WireValidate for PresentationKind {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    #[serde(rename = "session.controls.read")]
    SessionControlsRead,
    #[serde(rename = "session.controls.apply")]
    SessionControlsApply,
    #[serde(rename = "session.queue.list")]
    SessionQueueList,
    #[serde(rename = "session.queue.update")]
    SessionQueueUpdate,
    #[serde(rename = "session.queue.delete")]
    SessionQueueDelete,
    #[serde(rename = "session.queue.reorder")]
    SessionQueueReorder,
    #[serde(rename = "session.queue.start")]
    SessionQueueStart,
    #[serde(rename = "artifacts.list")]
    ArtifactsList,
    #[serde(rename = "artifacts.open")]
    ArtifactsOpen,
    #[serde(rename = "artifacts.read")]
    ArtifactsRead,
    #[serde(rename = "artifacts.close")]
    ArtifactsClose,
    #[serde(rename = "terminal.create")]
    TerminalCreate,
    #[serde(rename = "attachments.begin")]
    AttachmentsBegin,
    #[serde(rename = "attachments.chunk")]
    AttachmentsChunk,
    #[serde(rename = "attachments.complete")]
    AttachmentsComplete,
    #[serde(rename = "attachments.discard")]
    AttachmentsDiscard,
    #[serde(rename = "attachments.status")]
    AttachmentsStatus,
    #[serde(rename = "pair.confirm")]
    PairConfirm,
    #[serde(rename = "hello")]
    Hello,
    #[serde(rename = "host.snapshot")]
    HostSnapshot,
    #[serde(rename = "session.open")]
    SessionOpen,
    #[serde(rename = "session.history")]
    SessionHistory,
    #[serde(rename = "session.unsubscribe")]
    SessionUnsubscribe,
    #[serde(rename = "creation.directories")]
    CreationDirectories,
    #[serde(rename = "session.create")]
    SessionCreate,
    #[serde(rename = "shell.input")]
    ShellInput,
    #[serde(rename = "shell.resize")]
    ShellResize,
    #[serde(rename = "shell.scroll")]
    ShellScroll,
    #[serde(rename = "agent.rename")]
    AgentRename,
    #[serde(rename = "workspace.rename")]
    WorkspaceRename,
    #[serde(rename = "prompt.submit")]
    PromptSubmit,
    #[serde(rename = "turn.cancel")]
    TurnCancel,
    #[serde(rename = "request.respond")]
    RequestRespond,
    #[serde(rename = "action.status")]
    ActionStatus,
    #[serde(rename = "content.read")]
    ContentRead,
    #[serde(rename = "terminal.grid.subscribe")]
    TerminalGridSubscribe,
    #[serde(rename = "shell.grid.subscribe")]
    ShellGridSubscribe,
    #[serde(rename = "worktrees.list")]
    WorktreesList,
    #[serde(rename = "changes.read")]
    ChangesRead,
    #[serde(rename = "skills.list")]
    SkillsList,
    #[serde(rename = "skills.preview.open")]
    SkillsPreviewOpen,
    #[serde(rename = "skills.preview.read")]
    SkillsPreviewRead,
    #[serde(rename = "skills.preview.close")]
    SkillsPreviewClose,
    #[serde(rename = "notifications.sync")]
    NotificationsSync,
    #[serde(rename = "notifications.read")]
    NotificationsRead,
    #[serde(rename = "push.register")]
    PushRegister,
    #[serde(rename = "push.probe.ack")]
    PushProbeAck,
    #[serde(rename = "push.unregister")]
    PushUnregister,
    #[serde(rename = "device.unpair")]
    DeviceUnpair,
}
impl WireValidate for Method {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub session_id: Uuid,
    pub generation: Uuid,
}
impl WireValidate for Target {
    fn validate(&self) -> Validation {
        self.session_id.validate()?;
        self.generation.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backend {
    pub provider: Provider,
    pub version: Text,
    pub availability: Availability,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub reason: Option<Text>,
}
impl WireValidate for Backend {
    fn validate(&self) -> Validation {
        self.provider.validate()?;
        self.version.validate()?;
        self.availability.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanCreate {
    pub omp: bool,
    pub codex: bool,
}
impl WireValidate for CanCreate {
    fn validate(&self) -> Validation {
        self.omp.validate()?;
        self.codex.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub server_id: Uuid,
    pub name: Label,
    pub connectivity: ServerConnectivity,
    pub can_create: CanCreate,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub reason: Option<Text>,
}
impl WireValidate for Server {
    fn validate(&self) -> Validation {
        self.server_id.validate()?;
        self.name.validate()?;
        self.connectivity.validate()?;
        self.can_create.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}
impl WireValidate for NativeAgentStatus {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeFreshness {
    Current,
    Stale,
    Unavailable,
}
impl WireValidate for NativeFreshness {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeUnavailableReason {
    SourceUnavailable,
    TooLarge,
}
impl WireValidate for NativeUnavailableReason {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDisplayUnavailable {
    TooLarge,
    Unsafe,
}
impl WireValidate for NativeDisplayUnavailable {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRestoreReason {
    RestoreFailed,
}
impl WireValidate for NativeRestoreReason {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeWorkspace {
    pub workspace_id: Id,
    pub number: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub label: Option<Label>,
    pub focused: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub active_tab_id: Option<Id>,
    pub agent_status: NativeAgentStatus,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub display_unavailable: Option<NativeDisplayUnavailable>,
}
impl WireValidate for NativeWorkspace {
    fn validate(&self) -> Validation {
        self.workspace_id.validate()?;
        self.number.validate()?;
        self.label.validate()?;
        self.active_tab_id.validate()?;
        ensure(self.label.is_some() || self.display_unavailable.is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTab {
    pub tab_id: Id,
    pub workspace_id: Id,
    pub number: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub label: Option<Label>,
    pub focused: bool,
    pub agent_status: NativeAgentStatus,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub display_unavailable: Option<NativeDisplayUnavailable>,
}
impl WireValidate for NativeTab {
    fn validate(&self) -> Validation {
        self.tab_id.validate()?;
        self.workspace_id.validate()?;
        self.number.validate()?;
        self.label.validate()?;
        ensure(self.label.is_some() || self.display_unavailable.is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePane {
    pub pane_id: Id,
    pub terminal_id: Id,
    pub workspace_id: Id,
    pub tab_id: Id,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub label: Option<Label>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub title: Option<Label>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub terminal_title: Option<Label>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub display_agent: Option<Label>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub agent: Option<Label>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub agent_name: Option<Label>,
    pub focused: bool,
    pub agent_status: NativeAgentStatus,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub restore_reason: Option<NativeRestoreReason>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub display_unavailable: Option<NativeDisplayUnavailable>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub target: Option<Target>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub unbound_row_id: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub shell_target: Option<ShellTarget>,
}
impl WireValidate for NativePane {
    fn validate(&self) -> Validation {
        self.pane_id.validate()?;
        self.terminal_id.validate()?;
        self.workspace_id.validate()?;
        self.tab_id.validate()?;
        self.label.validate()?;
        self.title.validate()?;
        self.terminal_title.validate()?;
        self.display_agent.validate()?;
        self.agent.validate()?;
        self.agent_name.validate()?;
        self.target.validate()?;
        self.unbound_row_id.validate()?;
        self.shell_target.validate()?;
        ensure(
            usize::from(self.target.is_some())
                + usize::from(self.unbound_row_id.is_some())
                + usize::from(self.shell_target.is_some())
                <= 1,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHierarchy {
    pub server_id: Uuid,
    pub server_incarnation: Uuid,
    pub freshness: NativeFreshness,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub reason: Option<NativeUnavailableReason>,
    pub workspaces: Vec<NativeWorkspace>,
    pub tabs: Vec<NativeTab>,
    pub panes: Vec<NativePane>,
}
impl WireValidate for NativeHierarchy {
    fn validate(&self) -> Validation {
        self.server_id.validate()?;
        self.server_incarnation.validate()?;
        self.workspaces.validate()?;
        self.tabs.validate()?;
        self.panes.validate()?;
        validate_native_hierarchy(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub project_id: Uuid,
    pub label: Label,
    pub available: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub reason: Option<Text>,
}
impl WireValidate for Project {
    fn validate(&self) -> Validation {
        self.project_id.validate()?;
        self.label.validate()?;
        self.available.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub controls: bool,
    #[serde(default)]
    pub terminal_create: bool,
    #[serde(default)]
    pub artifacts: bool,
    #[serde(default)]
    pub attachments: bool,
    pub history: bool,
    pub prompt: bool,
    pub cancel: bool,
    pub respond: bool,
    pub prompt_mode: PromptMode,
    pub cancel_mode: CancelMode,
    #[serde(default)]
    pub terminal_read: bool,
    #[serde(default)]
    pub worktrees_read: bool,
    #[serde(default)]
    pub changes_read: bool,
    #[serde(default)]
    pub workspace_rename: bool,
    #[serde(default)]
    pub agent_rename: bool,
    #[serde(default)]
    pub skills_read: bool,
}
impl WireValidate for Capabilities {
    fn validate(&self) -> Validation {
        self.history.validate()?;
        self.prompt.validate()?;
        self.cancel.validate()?;
        self.respond.validate()?;
        self.prompt_mode.validate()?;
        self.cancel_mode.validate()?;
        self.terminal_read.validate()?;
        self.worktrees_read.validate()?;
        self.changes_read.validate()?;
        self.workspace_rename.validate()?;
        self.agent_rename.validate()?;
        self.skills_read.validate()?;
        self.controls.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCoverage {
    pub scope: HistoryScope,
    pub source: HistorySource,
    pub older_available: bool,
    pub source_truncated: bool,
    pub desktop_branch_known: bool,
}
impl WireValidate for HistoryCoverage {
    fn validate(&self) -> Validation {
        self.scope.validate()?;
        self.source.validate()?;
        self.older_available.validate()?;
        self.source_truncated.validate()?;
        self.desktop_branch_known.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: Uuid,
    pub generation: Uuid,
    pub server_id: Uuid,
    pub workspace_id: Id,
    pub terminal_id: Id,
    pub pane_id: Id,
    pub provider: Provider,
    pub native_id: Id,
    pub title: Text,
    pub status: SessionStatus,
    pub connectivity: SessionConnectivity,
    pub capabilities: Capabilities,
    pub history_coverage: HistoryCoverage,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub skills_generation: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub reason: Option<Text>,
}
impl WireValidate for SessionSummary {
    fn validate(&self) -> Validation {
        self.session_id.validate()?;
        self.generation.validate()?;
        self.server_id.validate()?;
        self.workspace_id.validate()?;
        self.terminal_id.validate()?;
        self.pane_id.validate()?;
        self.provider.validate()?;
        self.native_id.validate()?;
        self.title.validate()?;
        self.status.validate()?;
        self.connectivity.validate()?;
        self.capabilities.validate()?;
        self.history_coverage.validate()?;
        self.skills_generation.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnboundRow {
    pub row_id: Uuid,
    pub server_id: Uuid,
    pub workspace_id: Id,
    pub terminal_id: Id,
    pub pane_id: Id,
    pub provider: Provider,
    pub title: Text,
    pub state: UnboundState,
    pub reason: Text,
}
impl WireValidate for UnboundRow {
    fn validate(&self) -> Validation {
        self.row_id.validate()?;
        self.server_id.validate()?;
        self.workspace_id.validate()?;
        self.terminal_id.validate()?;
        self.pane_id.validate()?;
        self.provider.validate()?;
        self.title.validate()?;
        self.state.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub summary: SessionSummary,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_turn_id: Option<Id>,
    pub desktop_selection: DesktopSelection,
    pub activity: Activity,
}
impl WireValidate for SessionState {
    fn validate(&self) -> Validation {
        self.summary.validate()?;
        self.native_turn_id.validate()?;
        self.desktop_selection.validate()?;
        self.activity.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub state: ActivityState,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub text: Option<Content>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub tool_record_id: Option<Id>,
}
impl WireValidate for Activity {
    fn validate(&self) -> Validation {
        self.state.validate()?;
        self.text.validate()?;
        self.tool_record_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub id: Id,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub parent_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub turn_id: Option<Id>,
    pub order: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub timestamp: Option<Uint>,
    pub role: Role,
    pub kind: RecordKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub text: Option<Content>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub tool: Option<Tool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub original: Option<Content>,
    pub source: RecordSource,
    pub complete: bool,
    pub source_truncated: bool,
}
impl WireValidate for HistoryRecord {
    fn validate(&self) -> Validation {
        self.id.validate()?;
        self.parent_id.validate()?;
        self.turn_id.validate()?;
        self.order.validate()?;
        self.timestamp.validate()?;
        self.role.validate()?;
        self.kind.validate()?;
        self.text.validate()?;
        self.tool.validate()?;
        self.original.validate()?;
        self.source.validate()?;
        self.complete.validate()?;
        self.source_truncated.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub call_id: Option<Id>,
    pub status: ToolStatus,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub input: Option<Content>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub output: Option<Content>,
}
impl WireValidate for Tool {
    fn validate(&self) -> Validation {
        self.name.validate()?;
        self.call_id.validate()?;
        self.status.validate()?;
        self.input.validate()?;
        self.output.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    pub request_key: Uuid,
    pub provider: Provider,
    pub kind: PresentationKind,
    pub presentation: Presentation,
}
impl WireValidate for PendingRequest {
    fn validate(&self) -> Validation {
        self.request_key.validate()?;
        self.provider.validate()?;
        self.kind.validate()?;
        self.presentation.validate()?;
        validate_pending_request(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireError {
    pub code: ErrorCode,
    pub message: SafeMessage,
}
impl WireValidate for WireError {
    fn validate(&self) -> Validation {
        self.code.validate()?;
        self.message.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectOption {
    pub value: Text,
    pub label: Label,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub description: Option<Content>,
}
impl WireValidate for SelectOption {
    fn validate(&self) -> Validation {
        self.value.validate()?;
        self.label.validate()?;
        self.description.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalChoice {
    pub choice_id: Id,
    pub label: Label,
    pub scope: ApprovalScope,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub details: Option<Content>,
}
impl WireValidate for ApprovalChoice {
    fn validate(&self) -> Validation {
        self.choice_id.validate()?;
        self.label.validate()?;
        self.scope.validate()?;
        self.details.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: Label,
    pub description: Content,
}
impl WireValidate for QuestionOption {
    fn validate(&self) -> Validation {
        self.label.validate()?;
        self.description.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: Id,
    pub header: Text,
    pub prompt: Content,
    pub is_other: bool,
    pub is_secret: bool,
    pub options: Vec<QuestionOption>,
}
impl WireValidate for Question {
    fn validate(&self) -> Validation {
        self.id.validate()?;
        self.header.validate()?;
        self.prompt.validate()?;
        self.is_other.validate()?;
        self.is_secret.validate()?;
        self.options.validate()?;
        validate_question(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormOption {
    pub value: Text,
    pub label: Label,
}
impl WireValidate for FormOption {
    fn validate(&self) -> Validation {
        self.value.validate()?;
        self.label.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub id: Id,
    pub title: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub description: Option<Content>,
    pub required: bool,
    pub r#type: FieldType,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub default: Option<FieldValue>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub format: Option<FieldFormat>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub min_length: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub max_length: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub minimum: Option<Decimal>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub maximum: Option<Decimal>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub min_items: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub max_items: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub options: Option<Vec<FormOption>>,
}
impl WireValidate for FormField {
    fn validate(&self) -> Validation {
        self.id.validate()?;
        self.title.validate()?;
        self.description.validate()?;
        self.required.validate()?;
        self.r#type.validate()?;
        self.default.validate()?;
        self.format.validate()?;
        self.min_length.validate()?;
        self.max_length.validate()?;
        self.minimum.validate()?;
        self.maximum.validate()?;
        self.min_items.validate()?;
        self.max_items.validate()?;
        self.options.validate()?;
        validate_form_field(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub id: Id,
    pub values: Vec<Text>,
}
impl WireValidate for QuestionAnswer {
    fn validate(&self) -> Validation {
        self.id.validate()?;
        self.values.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormAnswerValue {
    pub id: Id,
    pub value: FieldValue,
}
impl WireValidate for FormAnswerValue {
    fn validate(&self) -> Validation {
        self.id.validate()?;
        self.value.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub action_id: Uuid,
    pub method: ActionMethod,
    pub boot_id: Uuid,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub target: Option<Target>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub shell_target: Option<ShellTarget>,
    pub created_at: Uint,
    pub updated_at: Uint,
    pub state: ReceiptState,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub error: Option<WireError>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub result: Option<ActionResult>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub creation: Option<Box<CreationProgress>>,
}
impl WireValidate for Receipt {
    fn validate(&self) -> Validation {
        self.action_id.validate()?;
        self.method.validate()?;
        self.boot_id.validate()?;
        self.target.validate()?;
        self.created_at.validate()?;
        self.updated_at.validate()?;
        self.state.validate()?;
        self.error.validate()?;
        self.result.validate()?;
        self.creation.validate()?;
        self.shell_target.validate()?;
        validate_receipt(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationProgress {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub source_target: Option<Target>,
    pub server_id: Uuid,
    pub project_id: Uuid,
    pub provider: Provider,
    pub stage: CreationStage,
    pub stage_state: StageState,
    pub resources: CreationResources,
}
impl WireValidate for CreationProgress {
    fn validate(&self) -> Validation {
        self.source_target.validate()?;
        self.server_id.validate()?;
        self.project_id.validate()?;
        self.provider.validate()?;
        self.stage.validate()?;
        self.stage_state.validate()?;
        self.resources.validate()?;
        ensure(self.resources.checkout.is_none() || self.source_target.is_some())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationResources {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub checkout: Option<CreationCheckoutResources>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub workspace_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub pane_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub terminal_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub binding_id: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub session_id: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub generation: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub owner_started: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub relay_started: Option<bool>,
}
impl WireValidate for CreationResources {
    fn validate(&self) -> Validation {
        self.checkout.validate()?;
        self.workspace_id.validate()?;
        self.pane_id.validate()?;
        self.terminal_id.validate()?;
        self.native_id.validate()?;
        self.binding_id.validate()?;
        self.session_id.validate()?;
        self.generation.validate()?;
        self.owner_started.validate()?;
        self.relay_started.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionEvent {
    pub event_id: Uuid,
    pub sequence: Uint,
    pub session_id: Uuid,
    pub generation: Uuid,
    pub category: AttentionCategory,
    pub created_at: Uint,
}
impl WireValidate for AttentionEvent {
    fn validate(&self) -> Validation {
        self.event_id.validate()?;
        self.sequence.validate()?;
        self.session_id.validate()?;
        self.generation.validate()?;
        self.category.validate()?;
        self.created_at.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadWatermark {
    pub session_id: Uuid,
    pub through_sequence: Uint,
}
impl WireValidate for ReadWatermark {
    fn validate(&self) -> Validation {
        self.session_id.validate()?;
        self.through_sequence.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentText {
    pub text: InlineText,
}
impl WireValidate for ContentText {
    fn validate(&self) -> Validation {
        self.text.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRef {
    pub handle: Uuid,
    pub byte_length: Uint,
    pub mime: Mime,
}
impl WireValidate for ContentRef {
    fn validate(&self) -> Validation {
        self.handle.validate()?;
        self.byte_length.validate()?;
        self.mime.validate()?;
        validate_content_ref(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentUnavailable {
    pub reason: ContentUnavailableReason,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub byte_length: Option<Uint>,
}
impl WireValidate for ContentUnavailable {
    fn validate(&self) -> Validation {
        self.reason.validate()?;
        self.byte_length.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectionMarker {
    Radio,
    Checkbox,
}
impl WireValidate for SelectionMarker {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationOmpSelect {
    pub title: Content,
    pub options: Vec<SelectOption>,
    pub selection_marker: SelectionMarker,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub help_text: Option<Content>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub markable_count: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub selected: Option<Uint>,
    pub checked: Vec<Uint>,
}
impl WireValidate for PresentationOmpSelect {
    fn validate(&self) -> Validation {
        self.title.validate()?;
        self.options.validate()?;
        self.selected.validate()?;
        self.checked.validate()?;
        self.selection_marker.validate()?;
        self.help_text.validate()?;
        self.markable_count.validate()?;
        validate_presentation_omp_select(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationOmpEditor {
    pub title: Content,
    pub prefill: Content,
}
impl WireValidate for PresentationOmpEditor {
    fn validate(&self) -> Validation {
        self.title.validate()?;
        self.prefill.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCodexApproval {
    pub approval_type: ApprovalType,
    pub title: Content,
    pub details: Content,
    pub choices: Vec<ApprovalChoice>,
}
impl WireValidate for PresentationCodexApproval {
    fn validate(&self) -> Validation {
        self.approval_type.validate()?;
        self.title.validate()?;
        self.details.validate()?;
        self.choices.validate()?;
        validate_presentation_codex_approval(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCodexQuestions {
    pub is_blocking: bool,
    pub questions: Vec<Question>,
}
impl WireValidate for PresentationCodexQuestions {
    fn validate(&self) -> Validation {
        self.is_blocking.validate()?;
        self.questions.validate()?;
        validate_presentation_codex_questions(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCodexForm {
    pub title: Content,
    pub fields: Vec<FormField>,
}
impl WireValidate for PresentationCodexForm {
    fn validate(&self) -> Validation {
        self.title.validate()?;
        self.fields.validate()?;
        validate_presentation_codex_form(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCodexUrl {
    pub title: Content,
    pub url: Text,
}
impl WireValidate for PresentationCodexUrl {
    fn validate(&self) -> Validation {
        self.title.validate()?;
        self.url.validate()?;
        validate_presentation_codex_url(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationDesktopOnly {
    pub title: Content,
    pub reason: Text,
}
impl WireValidate for PresentationDesktopOnly {
    fn validate(&self) -> Validation {
        self.title.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerOmpSelect {
    pub action: SubmitAction,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub value: Option<Text>,
}
impl WireValidate for AnswerOmpSelect {
    fn validate(&self) -> Validation {
        self.action.validate()?;
        self.value.validate()?;
        validate_answer_omp_select(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerOmpEditor {
    pub action: SubmitAction,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub text: Option<PromptText>,
}
impl WireValidate for AnswerOmpEditor {
    fn validate(&self) -> Validation {
        self.action.validate()?;
        self.text.validate()?;
        validate_answer_omp_editor(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCodexApproval {
    pub choice_id: Id,
}
impl WireValidate for AnswerCodexApproval {
    fn validate(&self) -> Validation {
        self.choice_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCodexQuestions {
    pub answers: Vec<QuestionAnswer>,
}
impl WireValidate for AnswerCodexQuestions {
    fn validate(&self) -> Validation {
        self.answers.validate()?;
        validate_answer_codex_questions(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCodexForm {
    pub action: FormAction,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub values: Option<Vec<FormAnswerValue>>,
}
impl WireValidate for AnswerCodexForm {
    fn validate(&self) -> Validation {
        self.action.validate()?;
        self.values.validate()?;
        validate_answer_codex_form(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCodexUrl {
    pub action: UrlAction,
}
impl WireValidate for AnswerCodexUrl {
    fn validate(&self) -> Validation {
        self.action.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultWorkspaceRenamed {
    pub target: Target,
    pub workspace_id: Id,
}
impl WireValidate for ActionResultWorkspaceRenamed {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.workspace_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultAgentRenamed {
    pub target: Target,
    pub terminal_id: Id,
}
impl WireValidate for ActionResultAgentRenamed {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.terminal_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultPrompt {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_turn_id: Option<Id>,
}
impl WireValidate for ActionResultPrompt {
    fn validate(&self) -> Validation {
        self.native_turn_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultCancel {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_turn_id: Option<Id>,
}
impl WireValidate for ActionResultCancel {
    fn validate(&self) -> Validation {
        self.native_turn_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultResponse {
    pub resolution: ResponseResolution,
}
impl WireValidate for ActionResultResponse {
    fn validate(&self) -> Validation {
        self.resolution.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultCreated {
    pub target: Target,
    pub workspace_id: Id,
    pub terminal_id: Id,
    pub pane_id: Id,
    pub native_id: Id,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub binding_id: Option<Uuid>,
}
impl WireValidate for ActionResultCreated {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.workspace_id.validate()?;
        self.terminal_id.validate()?;
        self.pane_id.validate()?;
        self.native_id.validate()?;
        self.binding_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairConfirmParams {
    pub token: Key32,
    pub device_name: Label,
}
impl WireValidate for PairConfirmParams {
    fn validate(&self) -> Validation {
        self.token.validate()?;
        self.device_name.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairConfirmResult {
    pub host_id: EndpointId,
    pub device_id: Uuid,
    pub paired: TrueValue,
}
impl WireValidate for PairConfirmResult {
    fn validate(&self) -> Validation {
        self.host_id.validate()?;
        self.device_id.validate()?;
        self.paired.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    pub app_version: Text,
    pub device_name: Label,
}
impl WireValidate for HelloParams {
    fn validate(&self) -> Validation {
        self.app_version.validate()?;
        self.device_name.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloFeatures {
    #[serde(default)]
    pub agent_ansi: bool,
    #[serde(default)]
    pub shell_history: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub host_id: EndpointId,
    pub device_id: Uuid,
    pub boot_id: Uuid,
    pub protocol: ProtocolVersion,
    pub backends: Vec<Backend>,
    #[serde(default)]
    pub features: HelloFeatures,
}
impl WireValidate for HelloResult {
    fn validate(&self) -> Validation {
        self.host_id.validate()?;
        self.device_id.validate()?;
        self.boot_id.validate()?;
        self.protocol.validate()?;
        self.backends.validate()?;
        validate_hello_result(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSnapshotParams {}
impl WireValidate for HostSnapshotParams {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSnapshotResult {
    pub subscription: Uuid,
    pub seq: ZeroValue,
    pub servers: Vec<Server>,
    pub projects: Vec<Project>,
    pub sessions: Vec<SessionSummary>,
    #[serde(default)]
    pub shells: Vec<ShellSummary>,
    pub unbound: Vec<UnboundRow>,
    pub hierarchy: Vec<NativeHierarchy>,
}
impl WireValidate for HostSnapshotResult {
    fn validate(&self) -> Validation {
        self.subscription.validate()?;
        self.seq.validate()?;
        self.servers.validate()?;
        self.projects.validate()?;
        self.sessions.validate()?;
        self.unbound.validate()?;
        self.hierarchy.validate()?;
        self.shells.validate()?;
        validate_host_snapshot_result(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpenParams {
    pub target: Target,
}
impl WireValidate for SessionOpenParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpenResult {
    pub subscription: Uuid,
    pub seq: ZeroValue,
    pub state: Box<SessionState>,
    pub pending_requests: Vec<PendingRequest>,
    pub history: Vec<HistoryRecord>,
}
impl WireValidate for SessionOpenResult {
    fn validate(&self) -> Validation {
        self.subscription.validate()?;
        self.seq.validate()?;
        self.state.validate()?;
        self.pending_requests.validate()?;
        self.history.validate()?;
        validate_session_open_result(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryParams {
    pub target: Target,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub view_id: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub cursor: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub limit: Option<Uint>,
}
impl WireValidate for SessionHistoryParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.cursor.validate()?;
        self.limit.validate()?;
        self.view_id.validate()?;
        validate_session_history_params(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryResult {
    pub view_id: Uuid,
    pub records: Vec<HistoryRecord>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub next_cursor: Option<Id>,
}
impl WireValidate for SessionHistoryResult {
    fn validate(&self) -> Validation {
        self.records.validate()?;
        self.next_cursor.validate()?;
        self.view_id.validate()?;
        validate_session_history_result(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUnsubscribeParams {
    pub subscription: Uuid,
}
impl WireValidate for SessionUnsubscribeParams {
    fn validate(&self) -> Validation {
        self.subscription.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUnsubscribeResult {
    pub unsubscribed: TrueValue,
}
impl WireValidate for SessionUnsubscribeResult {
    fn validate(&self) -> Validation {
        self.unsubscribed.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateParams {
    pub boot_id: Uuid,
    pub server_id: Uuid,
    pub project_id: Uuid,
    pub provider: Provider,
    pub name: Label,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub directory_ref: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub placement: Option<SessionPlacement>,
}
impl WireValidate for SessionCreateParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.server_id.validate()?;
        self.project_id.validate()?;
        self.provider.validate()?;
        self.name.validate()?;
        self.directory_ref.validate()?;
        self.placement.validate()?;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlacement {
    pub source_target: Target,
}
impl WireValidate for SessionPlacement {
    fn validate(&self) -> Validation {
        self.source_target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationDirectoriesParams {
    pub server_id: Uuid,
    pub project_id: Uuid,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub source: Option<ArtifactSource>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub parent: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub cursor: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub limit: Option<Uint>,
}
impl WireValidate for CreationDirectoriesParams {
    fn validate(&self) -> Validation {
        self.server_id.validate()?;
        self.project_id.validate()?;
        self.source.validate()?;
        self.parent.validate()?;
        self.cursor.validate()?;
        self.limit.validate()?;
        ensure(self.limit.is_none_or(|limit| (1..=100).contains(&limit.0)))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationDirectoryEntry {
    pub reference: Uuid,
    pub name: Label,
}
impl WireValidate for CreationDirectoryEntry {
    fn validate(&self) -> Validation {
        self.reference.validate()?;
        self.name.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationDirectoriesResult {
    pub server_id: Uuid,
    pub project_id: Uuid,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub source: Option<ArtifactSource>,
    pub entries: Vec<CreationDirectoryEntry>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub next_cursor: Option<Uuid>,
}
impl WireValidate for CreationDirectoriesResult {
    fn validate(&self) -> Validation {
        self.server_id.validate()?;
        self.project_id.validate()?;
        self.source.validate()?;
        ensure(self.entries.len() <= 100)?;
        self.entries.validate()?;
        self.next_cursor.validate()?;
        let mut seen = HashSet::with_capacity(self.entries.len());
        ensure(
            self.entries
                .iter()
                .all(|entry| seen.insert(&entry.reference)),
        )
    }
}

/// Retained serialized checkout choices in historical native-creation metadata only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreationCheckout {
    New {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "optional_non_null"
        )]
        branch: Option<Text>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "optional_non_null"
        )]
        base: Option<Text>,
    },
    Existing {
        branch: Text,
    },
}
impl WireValidate for CreationCheckout {
    fn validate(&self) -> Validation {
        match self {
            Self::New { branch, base } => {
                branch
                    .as_ref()
                    .map(|value| validate_worktree_text(value, 1024, false))
                    .transpose()?;
                base.as_ref()
                    .map(|value| validate_worktree_text(value, 1024, false))
                    .transpose()?;
            }
            Self::Existing { branch } => validate_worktree_text(branch, 1024, false)?,
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateResult {
    pub receipt: Receipt,
}
impl WireValidate for SessionCreateResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRenameParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub name: AgentName,
}
impl WireValidate for AgentRenameParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.name.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRenameResult {
    pub receipt: Receipt,
}
impl WireValidate for AgentRenameResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRenameParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub label: Label,
}
impl WireValidate for WorkspaceRenameParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.label.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRenameResult {
    pub receipt: Receipt,
}
impl WireValidate for WorkspaceRenameResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillSelector {
    pub catalogue_generation: Uuid,
    pub reference: Uuid,
}
impl WireValidate for SkillSelector {
    fn validate(&self) -> Validation {
        self.catalogue_generation.validate()?;
        self.reference.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSubmitParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub text: PromptText,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub skill: Option<SkillSelector>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub delivery: Option<PromptDelivery>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_turn_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub images: Option<Vec<Uuid>>,
}
impl WireValidate for PromptSubmitParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.text.validate()?;
        self.skill.validate()?;
        self.delivery.validate()?;
        self.native_turn_id.validate()?;
        self.images.validate()?;
        ensure(self.native_turn_id.is_none() || self.delivery == Some(PromptDelivery::Steer))?;
        if let Some(images) = &self.images {
            let mut seen = std::collections::HashSet::new();
            ensure(
                !images.is_empty() && images.len() <= 4 && images.iter().all(|id| seen.insert(id)),
            )?;
        }
        ensure(
            self.skill.is_none()
                || self.text.0.is_empty()
                    && self.delivery.is_none()
                    && self.native_turn_id.is_none()
                    && self.images.is_none(),
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSubmitResult {
    pub receipt: Receipt,
}
impl WireValidate for PromptSubmitResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCancelParams {
    pub boot_id: Uuid,
    pub target: Target,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_turn_id: Option<Id>,
}
impl WireValidate for TurnCancelParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.native_turn_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCancelResult {
    pub receipt: Receipt,
}
impl WireValidate for TurnCancelResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRespondParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub request_key: Uuid,
    pub answer: Answer,
}
impl WireValidate for RequestRespondParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.request_key.validate()?;
        self.answer.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRespondResult {
    pub receipt: Receipt,
}
impl WireValidate for RequestRespondResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionStatusParams {
    pub action_id: Uuid,
}
impl WireValidate for ActionStatusParams {
    fn validate(&self) -> Validation {
        self.action_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReadParams {
    pub target: Target,
    pub handle: Uuid,
    pub offset_bytes: Uint,
    pub limit_bytes: Uint,
}
impl WireValidate for ContentReadParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.handle.validate()?;
        self.offset_bytes.validate()?;
        self.limit_bytes.validate()?;
        validate_content_read_params(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReadResult {
    pub bytes: Base64Bytes,
    pub next_offset_bytes: Uint,
    pub eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalGridSubscribeParams {
    pub target: Target,
    #[serde(default)]
    pub ansi_observer: bool,
}
impl WireValidate for TerminalGridSubscribeParams {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalGridSubscribeResult {
    pub target: Target,
    pub subscription: Uuid,
    pub seq: ZeroValue,
    pub frame: GridBegin,
}
impl WireValidate for TerminalGridSubscribeResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.subscription.validate()?;
        self.seq.validate()?;
        self.frame.validate()?;
        ensure(self.frame.revision.0 == 0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GridColor {
    Default,
    Indexed { index: Uint },
    Rgb { value: Uint },
}
impl WireValidate for GridColor {
    fn validate(&self) -> Validation {
        match self {
            Self::Default => Ok(()),
            Self::Indexed { index } => ensure(index.0 <= 255),
            Self::Rgb { value } => ensure(value.0 <= 0xffffff),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridUnderline {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridAttributes {
    pub bold: bool,
    pub faint: bool,
    pub italic: bool,
    pub blink: bool,
    pub strike: bool,
    #[serde(default)]
    pub inverse: bool,
    #[serde(default)]
    pub concealed: bool,
    pub underline: GridUnderline,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<GridColor>,
}
impl WireValidate for GridAttributes {
    fn validate(&self) -> Validation {
        if let Some(color) = &self.underline_color {
            color.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridCell {
    pub text: String,
    pub width: Uint,
    pub foreground: GridColor,
    pub background: GridColor,
    pub attributes: GridAttributes,
}
impl std::fmt::Debug for GridCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GridCell(redacted)")
    }
}
impl WireValidate for GridCell {
    fn validate(&self) -> Validation {
        ensure(
            self.width.0 <= 2
                && self.text.len() <= 260
                && (self.width.0 == 0) == self.text.is_empty()
                && !self
                    .text
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        )?;
        self.foreground.validate()?;
        self.background.validate()?;
        self.attributes.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridCursor {
    pub column: Uint,
    pub row: Uint,
    pub visible: bool,
    pub shape: Uint,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridBegin {
    pub mode: GridMode,
    pub revision: Uint,
    pub columns: Uint,
    pub rows: Uint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<GridCursor>,
    pub alternate_screen: bool,
    pub graphics_omitted: bool,
    #[serde(default)]
    pub popup_blocked: bool,
    pub part_count: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub scroll: Option<GridScroll>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridMode {
    Replace,
}
impl WireValidate for GridBegin {
    fn validate(&self) -> Validation {
        self.revision.validate()?;
        ensure(
            (1..=512).contains(&self.columns.0)
                && (1..=256).contains(&self.rows.0)
                && self.columns.0 * self.rows.0 <= 32768
                && (1..=128).contains(&self.part_count.0),
        )?;
        if let Some(c) = &self.cursor {
            ensure(c.column.0 < self.columns.0 && c.row.0 < self.rows.0 && c.shape.0 <= 6)?;
        }
        if let Some(scroll) = &self.scroll {
            scroll.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridScroll {
    pub offset_from_bottom: Uint,
    pub max_offset_from_bottom: Uint,
    pub viewport_rows: Uint,
}
impl WireValidate for GridScroll {
    fn validate(&self) -> Validation {
        self.offset_from_bottom.validate()?;
        self.max_offset_from_bottom.validate()?;
        self.viewport_rows.validate()?;
        ensure(
            self.offset_from_bottom.0 <= self.max_offset_from_bottom.0
                && (1..=256).contains(&self.viewport_rows.0),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridSpan {
    pub row: Uint,
    pub column: Uint,
    pub cells: Vec<GridCell>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridPart {
    pub revision: Uint,
    pub index: Uint,
    pub spans: Vec<GridSpan>,
}
impl WireValidate for GridPart {
    fn validate(&self) -> Validation {
        self.revision.validate()?;
        ensure(self.index.0 < 128 && !self.spans.is_empty() && self.spans.len() <= 512)?;
        let mut count = 0;
        for span in &self.spans {
            ensure(
                span.row.0 < 256
                    && span.column.0 < 512
                    && !span.cells.is_empty()
                    && span.cells.len() <= 512,
            )?;
            count += span.cells.len();
            ensure(count <= 512)?;
            for cell in &span.cells {
                cell.validate()?;
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridCommit {
    pub revision: Uint,
}
impl WireValidate for GridCommit {
    fn validate(&self) -> Validation {
        self.revision.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridBeginEvent {
    pub frame: GridBegin,
}
impl WireValidate for GridBeginEvent {
    fn validate(&self) -> Validation {
        self.frame.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreesListParams {
    pub target: Target,
}
impl WireValidate for WorktreesListParams {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeEntry {
    pub path: Text,
    pub label: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub branch: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub head: Option<Text>,
    pub is_bare: bool,
    pub is_detached: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub is_linked_worktree: Option<bool>,
}
impl WireValidate for WorktreeEntry {
    fn validate(&self) -> Validation {
        self.path.validate()?;
        self.label.validate()?;
        self.branch.validate()?;
        self.head.validate()?;
        self.is_bare.validate()?;
        self.is_detached.validate()?;
        self.is_locked.validate()?;
        self.is_prunable.validate()?;
        self.is_linked_worktree.validate()?;
        validate_worktree_text(&self.path, 4096, true)?;
        validate_worktree_text(&self.label, 1024, false)?;
        if let Some(branch) = &self.branch {
            validate_worktree_text(branch, 1024, false)?;
        }
        if let Some(head) = &self.head {
            validate_worktree_text(head, 1024, false)?;
        }
        Ok(())
    }
}

fn validate_worktree_text(value: &str, max_bytes: usize, absolute: bool) -> Validation {
    ensure(
        !value.is_empty()
            && value.len() <= max_bytes
            && !value.chars().any(char::is_control)
            && (!absolute || value.starts_with('/')),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreesListResult {
    pub target: Target,
    pub worktrees: Vec<WorktreeEntry>,
}
impl WireValidate for WorktreesListResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.worktrees.validate()?;
        ensure(self.worktrees.len() <= 128)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesReadParams {
    pub target: Target,
}
impl WireValidate for ChangesReadParams {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeState {
    Unchanged,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}
impl WireValidate for ChangeState {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSubmodule {
    pub commit_changed: bool,
    pub tracked_dirty: bool,
    pub untracked_dirty: bool,
}
impl WireValidate for ChangeSubmodule {
    fn validate(&self) -> Validation {
        self.commit_changed.validate()?;
        self.tracked_dirty.validate()?;
        self.untracked_dirty.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChangeEntry {
    Tracked {
        path: Text,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "optional_non_null"
        )]
        original_path: Option<Text>,
        index: ChangeState,
        worktree: ChangeState,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "optional_non_null"
        )]
        submodule: Option<ChangeSubmodule>,
        staged_binary: bool,
        unstaged_binary: bool,
    },
    Untracked {
        path: Text,
    },
}
impl WireValidate for ChangeEntry {
    fn validate(&self) -> Validation {
        match self {
            Self::Tracked {
                path,
                original_path,
                index,
                worktree,
                submodule,
                staged_binary,
                unstaged_binary,
            } => {
                path.validate()?;
                original_path.validate()?;
                index.validate()?;
                worktree.validate()?;
                submodule.validate()?;
                staged_binary.validate()?;
                unstaged_binary.validate()?;
                validate_worktree_text(path, 4096, false)?;
                if let Some(old) = original_path {
                    validate_worktree_text(old, 4096, false)?;
                }
                let requires_original = matches!(index, ChangeState::Renamed | ChangeState::Copied)
                    || matches!(worktree, ChangeState::Renamed | ChangeState::Copied);
                ensure(requires_original == original_path.is_some())
            }
            Self::Untracked { path } => {
                path.validate()?;
                validate_worktree_text(path, 4096, false)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesRepositoryContext {
    pub unborn: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub upstream: Option<ChangesUpstream>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub latest_commit: Option<ChangesLatestCommit>,
}
impl WireValidate for ChangesRepositoryContext {
    fn validate(&self) -> Validation {
        self.upstream.validate()?;
        self.latest_commit.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesUpstream {
    pub name: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub ahead: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub behind: Option<Uint>,
}
impl WireValidate for ChangesUpstream {
    fn validate(&self) -> Validation {
        validate_worktree_text(&self.name, 1024, false)?;
        self.ahead.validate()?;
        self.behind.validate()?;
        ensure(self.ahead.is_some() == self.behind.is_some())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesLatestCommit {
    pub oid: Text,
    pub subject: Text,
}
fn valid_changes_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
impl WireValidate for ChangesLatestCommit {
    fn validate(&self) -> Validation {
        ensure(
            valid_changes_oid(&self.oid)
                && self.subject.len() <= 1024
                && !self.subject.chars().any(char::is_control),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesInfo {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub repository_context: Option<ChangesRepositoryContext>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub branch: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub head: Option<Text>,
    pub detached: bool,
    pub entries: Vec<ChangeEntry>,
    pub staged_patch: Text,
    pub unstaged_patch: Text,
}
impl WireValidate for ChangesInfo {
    fn validate(&self) -> Validation {
        self.branch.validate()?;
        self.head.validate()?;
        self.detached.validate()?;
        self.repository_context.validate()?;
        if let Some(context) = &self.repository_context {
            ensure(!self.detached || context.upstream.is_none())?;
            if context.unborn {
                ensure(
                    !self.detached
                        && self.branch.is_some()
                        && self.head.is_none()
                        && context.latest_commit.is_none()
                        && context.upstream.as_ref().is_none_or(|upstream| {
                            upstream.ahead.is_none() && upstream.behind.is_none()
                        }),
                )?;
            } else {
                ensure(
                    self.head.as_deref().is_some_and(valid_changes_oid)
                        && context
                            .latest_commit
                            .as_ref()
                            .is_some_and(|commit| Some(&commit.oid) == self.head.as_ref()),
                )?;
            }
        }
        self.entries.validate()?;
        self.staged_patch.validate()?;
        self.unstaged_patch.validate()?;
        ensure(self.entries.len() <= 128 && (!self.detached || self.branch.is_none()))?;
        if let Some(branch) = &self.branch {
            validate_worktree_text(branch, 1024, false)?;
        }
        if let Some(head) = &self.head {
            validate_worktree_text(head, 1024, false)?;
        }
        validate_patch_text(&self.staged_patch)?;
        validate_patch_text(&self.unstaged_patch)?;
        let mut paths = std::collections::HashSet::new();
        for entry in &self.entries {
            let path = match entry {
                ChangeEntry::Tracked { path, .. } | ChangeEntry::Untracked { path } => path,
            };
            ensure(paths.insert(path))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesReadResult {
    pub target: Target,
    pub changes: ChangesInfo,
}
impl WireValidate for ChangesReadResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.changes.validate()
    }
}

fn validate_patch_text(value: &str) -> Validation {
    ensure(
        value.len() <= 256 * 1024
            && value
                .chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')),
    )
}
impl WireValidate for ContentReadResult {
    fn validate(&self) -> Validation {
        self.bytes.validate()?;
        self.next_offset_bytes.validate()?;
        self.eof.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsSyncParams {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub after_sequence: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub limit: Option<Uint>,
}
impl WireValidate for NotificationsSyncParams {
    fn validate(&self) -> Validation {
        self.after_sequence.validate()?;
        self.limit.validate()?;
        validate_notifications_sync_params(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsSyncResult {
    pub events: Vec<AttentionEvent>,
    pub read: Vec<ReadWatermark>,
    pub latest_sequence: Uint,
    pub retained_from: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub next_sequence: Option<Uint>,
    pub reset_required: bool,
}
impl WireValidate for NotificationsSyncResult {
    fn validate(&self) -> Validation {
        self.events.validate()?;
        self.read.validate()?;
        self.latest_sequence.validate()?;
        self.retained_from.validate()?;
        self.next_sequence.validate()?;
        self.reset_required.validate()?;
        validate_notifications_sync_result(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsReadParams {
    pub boot_id: Uuid,
    pub session_id: Uuid,
    pub through_sequence: Uint,
}
impl WireValidate for NotificationsReadParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.session_id.validate()?;
        self.through_sequence.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsReadResult {
    pub read: ReadWatermark,
}
impl WireValidate for NotificationsReadResult {
    fn validate(&self) -> Validation {
        self.read.validate()?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRegisterParams {
    pub boot_id: Uuid,
    pub endpoint: Text,
    pub endpoint_generation: Uuid,
    pub push_key: Key32,
    pub probe: Uuid,
}
impl std::fmt::Debug for PushRegisterParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PushRegisterParams(redacted)")
    }
}
impl WireValidate for PushRegisterParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.endpoint.validate()?;
        self.endpoint_generation.validate()?;
        self.push_key.validate()?;
        self.probe.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRegisterResult {
    pub registered: TrueValue,
    pub endpoint_generation: Uuid,
    pub probe: Uuid,
    pub verified: bool,
}
impl WireValidate for PushRegisterResult {
    fn validate(&self) -> Validation {
        self.registered.validate()?;
        self.endpoint_generation.validate()?;
        self.probe.validate()?;
        self.verified.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushProbeAckParams {
    pub boot_id: Uuid,
    pub endpoint_generation: Uuid,
    pub probe: Uuid,
}
impl WireValidate for PushProbeAckParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.endpoint_generation.validate()?;
        self.probe.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushProbeAckResult {
    pub verified: TrueValue,
}
impl WireValidate for PushProbeAckResult {
    fn validate(&self) -> Validation {
        self.verified.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushUnregisterParams {
    pub boot_id: Uuid,
    pub endpoint_generation: Uuid,
}
impl WireValidate for PushUnregisterParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.endpoint_generation.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushUnregisterResult {
    pub unregistered: bool,
}
impl WireValidate for PushUnregisterResult {
    fn validate(&self) -> Validation {
        self.unregistered.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceUnpairParams {
    pub boot_id: Uuid,
}
impl WireValidate for DeviceUnpairParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceUnpairResult {
    pub unpaired: TrueValue,
}
impl WireValidate for DeviceUnpairResult {
    fn validate(&self) -> Validation {
        self.unpaired.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownActionStatus {
    pub state: UnknownValue,
}
impl WireValidate for UnknownActionStatus {
    fn validate(&self) -> Validation {
        self.state.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownActionStatus {
    pub receipt: Receipt,
}
impl WireValidate for KnownActionStatus {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostChangedEvent {}
impl WireValidate for HostChangedEvent {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateEvent {
    pub state: SessionState,
}
impl WireValidate for SessionStateEvent {
    fn validate(&self) -> Validation {
        self.state.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryUpsertEvent {
    pub records: Vec<HistoryRecord>,
}
impl WireValidate for HistoryUpsertEvent {
    fn validate(&self) -> Validation {
        self.records.validate()?;
        validate_history_upsert_event(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityUpdateEvent {
    pub activity: Activity,
}
impl WireValidate for ActivityUpdateEvent {
    fn validate(&self) -> Validation {
        self.activity.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOpenedEvent {
    pub request: PendingRequest,
}
impl WireValidate for RequestOpenedEvent {
    fn validate(&self) -> Validation {
        self.request.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestClosedEvent {
    pub request_key: Uuid,
    pub reason: RequestClosedReason,
}
impl WireValidate for RequestClosedEvent {
    fn validate(&self) -> Validation {
        self.request_key.validate()?;
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionStatusEvent {
    pub receipt: Receipt,
}
impl WireValidate for ActionStatusEvent {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationUpsertEvent {
    pub event: AttentionEvent,
}
impl WireValidate for NotificationUpsertEvent {
    fn validate(&self) -> Validation {
        self.event.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationReadEvent {
    pub read: ReadWatermark,
}
impl WireValidate for NotificationReadEvent {
    fn validate(&self) -> Validation {
        self.read.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequiredEvent {
    pub reason: SyncReason,
}
impl WireValidate for SyncRequiredEvent {
    fn validate(&self) -> Validation {
        self.reason.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Content {
    #[serde(rename = "text")]
    Text(ContentText),
    #[serde(rename = "ref")]
    Ref(ContentRef),
    #[serde(rename = "unavailable")]
    Unavailable(ContentUnavailable),
}
impl WireValidate for Content {
    fn validate(&self) -> Validation {
        match self {
            Self::Text(x) => x.validate(),
            Self::Ref(x) => x.validate(),
            Self::Unavailable(x) => x.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Presentation {
    #[serde(rename = "omp_select")]
    OmpSelect(PresentationOmpSelect),
    #[serde(rename = "omp_editor")]
    OmpEditor(PresentationOmpEditor),
    #[serde(rename = "codex_approval")]
    CodexApproval(PresentationCodexApproval),
    #[serde(rename = "codex_questions")]
    CodexQuestions(PresentationCodexQuestions),
    #[serde(rename = "codex_form")]
    CodexForm(PresentationCodexForm),
    #[serde(rename = "codex_url")]
    CodexUrl(PresentationCodexUrl),
    #[serde(rename = "desktop_only")]
    DesktopOnly(PresentationDesktopOnly),
}
impl WireValidate for Presentation {
    fn validate(&self) -> Validation {
        match self {
            Self::OmpSelect(x) => x.validate(),
            Self::OmpEditor(x) => x.validate(),
            Self::CodexApproval(x) => x.validate(),
            Self::CodexQuestions(x) => x.validate(),
            Self::CodexForm(x) => x.validate(),
            Self::CodexUrl(x) => x.validate(),
            Self::DesktopOnly(x) => x.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Answer {
    #[serde(rename = "omp_select")]
    OmpSelect(AnswerOmpSelect),
    #[serde(rename = "omp_editor")]
    OmpEditor(AnswerOmpEditor),
    #[serde(rename = "codex_approval")]
    CodexApproval(AnswerCodexApproval),
    #[serde(rename = "codex_questions")]
    CodexQuestions(AnswerCodexQuestions),
    #[serde(rename = "codex_form")]
    CodexForm(AnswerCodexForm),
    #[serde(rename = "codex_url")]
    CodexUrl(AnswerCodexUrl),
}
impl WireValidate for Answer {
    fn validate(&self) -> Validation {
        match self {
            Self::OmpSelect(x) => x.validate(),
            Self::OmpEditor(x) => x.validate(),
            Self::CodexApproval(x) => x.validate(),
            Self::CodexQuestions(x) => x.validate(),
            Self::CodexForm(x) => x.validate(),
            Self::CodexUrl(x) => x.validate(),
        }?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ActionResult {
    #[serde(rename = "queue")]
    Queue(ActionResultQueue),
    #[serde(rename = "attachment")]
    Attachment(ActionResultAttachment),
    #[serde(rename = "worktree_removal")]
    WorktreeRemoval(ActionResultWorktreeRemoval),
    #[serde(rename = "shell_creation")]
    ShellCreation(ActionResultShellCreation),
    #[serde(rename = "shell_input")]
    ShellInput(ActionResultShellInput),
    #[serde(rename = "shell_dispatch")]
    ShellDispatch(ActionResultShellDispatch),
    #[serde(rename = "shell_resize")]
    ShellResize(ActionResultShellResize),
    #[serde(rename = "shell_scroll")]
    ShellScroll(GridScroll),
    #[serde(rename = "agent_renamed")]
    AgentRenamed(ActionResultAgentRenamed),
    #[serde(rename = "workspace_renamed")]
    WorkspaceRenamed(ActionResultWorkspaceRenamed),
    #[serde(rename = "prompt")]
    Prompt(ActionResultPrompt),
    #[serde(rename = "cancel")]
    Cancel(ActionResultCancel),
    #[serde(rename = "response")]
    Response(ActionResultResponse),
    #[serde(rename = "created")]
    Created(Box<ActionResultCreated>),
}
impl WireValidate for ActionResult {
    fn validate(&self) -> Validation {
        match self {
            Self::Queue(x) => x.validate(),
            Self::Attachment(x) => x.validate(),
            Self::ShellCreation(x) => x.validate(),
            Self::ShellInput(x) => x.validate(),
            Self::ShellDispatch(x) => x.validate(),
            Self::ShellResize(x) => x.validate(),
            Self::ShellScroll(x) => x.validate(),
            Self::WorktreeRemoval(x) => x.validate(),
            Self::AgentRenamed(x) => x.validate(),
            Self::WorkspaceRenamed(x) => x.validate(),
            Self::Prompt(x) => x.validate(),
            Self::Cancel(x) => x.validate(),
            Self::Response(x) => x.validate(),
            Self::Created(x) => x.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldValue {
    String(String),
    Boolean(bool),
    Multi(Vec<String>),
}
impl WireValidate for FieldValue {
    fn validate(&self) -> Validation {
        match self {
            Self::String(s) => s.validate(),
            Self::Boolean(_) => Ok(()),
            Self::Multi(xs) => xs.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum RequestBody {
    #[serde(rename = "session.controls.read")]
    SessionControlsRead(SessionControlsReadParams),
    #[serde(rename = "session.controls.apply")]
    SessionControlsApply(SessionControlsApplyParams),
    #[serde(rename = "session.queue.list")]
    SessionQueueList(SessionQueueListParams),
    #[serde(rename = "session.queue.update")]
    SessionQueueUpdate(SessionQueueUpdateParams),
    #[serde(rename = "session.queue.delete")]
    SessionQueueDelete(SessionQueueDeleteParams),
    #[serde(rename = "session.queue.reorder")]
    SessionQueueReorder(SessionQueueReorderParams),
    #[serde(rename = "session.queue.start")]
    SessionQueueStart(SessionQueueStartParams),
    #[serde(rename = "artifacts.list")]
    ArtifactsList(ArtifactsListParams),
    #[serde(rename = "artifacts.open")]
    ArtifactsOpen(ArtifactsOpenParams),
    #[serde(rename = "artifacts.read")]
    ArtifactsRead(ArtifactsReadParams),
    #[serde(rename = "artifacts.close")]
    ArtifactsClose(ArtifactsCloseParams),
    #[serde(rename = "terminal.create")]
    TerminalCreate(TerminalCreateParams),
    #[serde(rename = "attachments.begin")]
    AttachmentsBegin(AttachmentsBeginParams),
    #[serde(rename = "attachments.chunk")]
    AttachmentsChunk(AttachmentsChunkParams),
    #[serde(rename = "attachments.complete")]
    AttachmentsComplete(AttachmentsCompleteParams),
    #[serde(rename = "attachments.discard")]
    AttachmentsDiscard(AttachmentsDiscardParams),
    #[serde(rename = "attachments.status")]
    AttachmentsStatus(AttachmentsStatusParams),
    #[serde(rename = "pair.confirm")]
    PairConfirm(PairConfirmParams),
    #[serde(rename = "hello")]
    Hello(HelloParams),
    #[serde(rename = "host.snapshot")]
    HostSnapshot(HostSnapshotParams),
    #[serde(rename = "session.open")]
    SessionOpen(SessionOpenParams),
    #[serde(rename = "session.history")]
    SessionHistory(SessionHistoryParams),
    #[serde(rename = "session.unsubscribe")]
    SessionUnsubscribe(SessionUnsubscribeParams),
    #[serde(rename = "creation.directories")]
    CreationDirectories(CreationDirectoriesParams),
    #[serde(rename = "session.create")]
    SessionCreate(SessionCreateParams),
    #[serde(rename = "shell.input")]
    ShellInput(ShellInputParams),
    #[serde(rename = "shell.resize")]
    ShellResize(ShellResizeParams),
    #[serde(rename = "shell.scroll")]
    ShellScroll(ShellScrollParams),
    #[serde(rename = "agent.rename")]
    AgentRename(AgentRenameParams),
    #[serde(rename = "workspace.rename")]
    WorkspaceRename(WorkspaceRenameParams),
    #[serde(rename = "prompt.submit")]
    PromptSubmit(PromptSubmitParams),
    #[serde(rename = "turn.cancel")]
    TurnCancel(TurnCancelParams),
    #[serde(rename = "request.respond")]
    RequestRespond(RequestRespondParams),
    #[serde(rename = "action.status")]
    ActionStatus(ActionStatusParams),
    #[serde(rename = "content.read")]
    ContentRead(ContentReadParams),
    #[serde(rename = "terminal.grid.subscribe")]
    TerminalGridSubscribe(TerminalGridSubscribeParams),
    #[serde(rename = "shell.grid.subscribe")]
    ShellGridSubscribe(ShellGridSubscribeParams),
    #[serde(rename = "worktrees.list")]
    WorktreesList(WorktreesListParams),
    #[serde(rename = "changes.read")]
    ChangesRead(ChangesReadParams),
    #[serde(rename = "skills.list")]
    SkillsList(SkillsListParams),
    #[serde(rename = "skills.preview.open")]
    SkillsPreviewOpen(SkillsPreviewOpenParams),
    #[serde(rename = "skills.preview.read")]
    SkillsPreviewRead(SkillsPreviewReadParams),
    #[serde(rename = "skills.preview.close")]
    SkillsPreviewClose(SkillsPreviewCloseParams),
    #[serde(rename = "notifications.sync")]
    NotificationsSync(NotificationsSyncParams),
    #[serde(rename = "notifications.read")]
    NotificationsRead(NotificationsReadParams),
    #[serde(rename = "push.register")]
    PushRegister(PushRegisterParams),
    #[serde(rename = "push.probe.ack")]
    PushProbeAck(PushProbeAckParams),
    #[serde(rename = "push.unregister")]
    PushUnregister(PushUnregisterParams),
    #[serde(rename = "device.unpair")]
    DeviceUnpair(DeviceUnpairParams),
}
impl RequestBody {
    pub fn method(&self) -> Method {
        match self {
            Self::SessionControlsRead(_) => Method::SessionControlsRead,
            Self::SessionControlsApply(_) => Method::SessionControlsApply,
            Self::SessionQueueList(_) => Method::SessionQueueList,
            Self::SessionQueueUpdate(_) => Method::SessionQueueUpdate,
            Self::SessionQueueDelete(_) => Method::SessionQueueDelete,
            Self::SessionQueueReorder(_) => Method::SessionQueueReorder,
            Self::SessionQueueStart(_) => Method::SessionQueueStart,
            Self::ArtifactsList(_) => Method::ArtifactsList,
            Self::ArtifactsOpen(_) => Method::ArtifactsOpen,
            Self::ArtifactsRead(_) => Method::ArtifactsRead,
            Self::ArtifactsClose(_) => Method::ArtifactsClose,
            Self::TerminalCreate(_) => Method::TerminalCreate,
            Self::AttachmentsBegin(_) => Method::AttachmentsBegin,
            Self::AttachmentsChunk(_) => Method::AttachmentsChunk,
            Self::AttachmentsComplete(_) => Method::AttachmentsComplete,
            Self::AttachmentsDiscard(_) => Method::AttachmentsDiscard,
            Self::AttachmentsStatus(_) => Method::AttachmentsStatus,
            Self::PairConfirm(_) => Method::PairConfirm,
            Self::Hello(_) => Method::Hello,
            Self::HostSnapshot(_) => Method::HostSnapshot,
            Self::SessionOpen(_) => Method::SessionOpen,
            Self::SessionHistory(_) => Method::SessionHistory,
            Self::SessionUnsubscribe(_) => Method::SessionUnsubscribe,
            Self::CreationDirectories(_) => Method::CreationDirectories,
            Self::SessionCreate(_) => Method::SessionCreate,
            Self::ShellInput(_) => Method::ShellInput,
            Self::ShellResize(_) => Method::ShellResize,
            Self::ShellScroll(_) => Method::ShellScroll,
            Self::ShellGridSubscribe(_) => Method::ShellGridSubscribe,
            Self::AgentRename(_) => Method::AgentRename,
            Self::WorkspaceRename(_) => Method::WorkspaceRename,
            Self::PromptSubmit(_) => Method::PromptSubmit,
            Self::TurnCancel(_) => Method::TurnCancel,
            Self::RequestRespond(_) => Method::RequestRespond,
            Self::ActionStatus(_) => Method::ActionStatus,
            Self::ContentRead(_) => Method::ContentRead,
            Self::TerminalGridSubscribe(_) => Method::TerminalGridSubscribe,
            Self::WorktreesList(_) => Method::WorktreesList,
            Self::ChangesRead(_) => Method::ChangesRead,
            Self::SkillsList(_) => Method::SkillsList,
            Self::SkillsPreviewOpen(_) => Method::SkillsPreviewOpen,
            Self::SkillsPreviewRead(_) => Method::SkillsPreviewRead,
            Self::SkillsPreviewClose(_) => Method::SkillsPreviewClose,
            Self::NotificationsSync(_) => Method::NotificationsSync,
            Self::NotificationsRead(_) => Method::NotificationsRead,
            Self::PushRegister(_) => Method::PushRegister,
            Self::PushProbeAck(_) => Method::PushProbeAck,
            Self::PushUnregister(_) => Method::PushUnregister,
            Self::DeviceUnpair(_) => Method::DeviceUnpair,
        }
    }
}
impl WireValidate for RequestBody {
    fn validate(&self) -> Validation {
        match self {
            Self::SessionControlsRead(x) => x.validate(),
            Self::SessionControlsApply(x) => x.validate(),
            Self::SessionQueueList(x) => x.validate(),
            Self::SessionQueueUpdate(x) => x.validate(),
            Self::SessionQueueDelete(x) => x.validate(),
            Self::SessionQueueReorder(x) => x.validate(),
            Self::SessionQueueStart(x) => x.validate(),
            Self::ArtifactsList(x) => x.validate(),
            Self::ArtifactsOpen(x) => x.validate(),
            Self::ArtifactsRead(x) => x.validate(),
            Self::ArtifactsClose(x) => x.validate(),
            Self::TerminalCreate(x) => x.validate(),
            Self::AttachmentsBegin(x) => x.validate(),
            Self::AttachmentsChunk(x) => x.validate(),
            Self::AttachmentsComplete(x) => x.validate(),
            Self::AttachmentsDiscard(x) => x.validate(),
            Self::AttachmentsStatus(x) => x.validate(),
            Self::PairConfirm(x) => x.validate(),
            Self::Hello(x) => x.validate(),
            Self::HostSnapshot(x) => x.validate(),
            Self::SessionOpen(x) => x.validate(),
            Self::SessionHistory(x) => x.validate(),
            Self::SessionUnsubscribe(x) => x.validate(),
            Self::CreationDirectories(x) => x.validate(),
            Self::SessionCreate(x) => x.validate(),
            Self::ShellInput(x) => x.validate(),
            Self::ShellResize(x) => x.validate(),
            Self::ShellScroll(x) => x.validate(),
            Self::ShellGridSubscribe(x) => x.validate(),
            Self::AgentRename(x) => x.validate(),
            Self::WorkspaceRename(x) => x.validate(),
            Self::PromptSubmit(x) => x.validate(),
            Self::TurnCancel(x) => x.validate(),
            Self::RequestRespond(x) => x.validate(),
            Self::ActionStatus(x) => x.validate(),
            Self::ContentRead(x) => x.validate(),
            Self::TerminalGridSubscribe(x) => x.validate(),
            Self::WorktreesList(x) => x.validate(),
            Self::ChangesRead(x) => x.validate(),
            Self::SkillsList(x) => x.validate(),
            Self::SkillsPreviewOpen(x) => x.validate(),
            Self::SkillsPreviewRead(x) => x.validate(),
            Self::SkillsPreviewClose(x) => x.validate(),
            Self::NotificationsSync(x) => x.validate(),
            Self::NotificationsRead(x) => x.validate(),
            Self::PushRegister(x) => x.validate(),
            Self::PushProbeAck(x) => x.validate(),
            Self::PushUnregister(x) => x.validate(),
            Self::DeviceUnpair(x) => x.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultBody {
    SessionControlsRead(SessionControlsReadResult),
    SessionControlsApply(SessionControlsApplyResult),
    SessionQueueList(SessionQueueListResult),
    SessionQueueUpdate(SessionQueueResult),
    SessionQueueDelete(SessionQueueResult),
    SessionQueueReorder(SessionQueueResult),
    SessionQueueStart(SessionQueueResult),
    ArtifactsList(ArtifactsListResult),
    ArtifactsOpen(ArtifactsOpenResult),
    ArtifactsRead(ArtifactsReadResult),
    ArtifactsClose(ArtifactsCloseResult),
    TerminalCreate(TerminalCreateResult),
    AttachmentsBegin(AttachmentsResult),
    AttachmentsChunk(AttachmentsResult),
    AttachmentsComplete(AttachmentsResult),
    AttachmentsDiscard(AttachmentsResult),
    AttachmentsStatus(AttachmentsStatusResult),
    PairConfirm(PairConfirmResult),
    Hello(HelloResult),
    HostSnapshot(HostSnapshotResult),
    SessionOpen(SessionOpenResult),
    SessionHistory(SessionHistoryResult),
    SessionUnsubscribe(SessionUnsubscribeResult),
    CreationDirectories(CreationDirectoriesResult),
    SessionCreate(SessionCreateResult),
    ShellInput(ShellInputResult),
    ShellResize(ShellResizeResult),
    ShellScroll(ShellScrollResult),
    ShellGridSubscribe(ShellGridSubscribeResult),
    AgentRename(AgentRenameResult),
    WorkspaceRename(WorkspaceRenameResult),
    PromptSubmit(PromptSubmitResult),
    TurnCancel(TurnCancelResult),
    RequestRespond(RequestRespondResult),
    ActionStatus(ActionStatusResult),
    ContentRead(ContentReadResult),
    TerminalGridSubscribe(TerminalGridSubscribeResult),
    WorktreesList(WorktreesListResult),
    ChangesRead(ChangesReadResult),
    SkillsList(SkillsListResult),
    SkillsPreviewOpen(SkillsPreviewOpenResult),
    SkillsPreviewRead(SkillsPreviewReadResult),
    SkillsPreviewClose(SkillsPreviewCloseResult),
    NotificationsSync(NotificationsSyncResult),
    NotificationsRead(NotificationsReadResult),
    PushRegister(PushRegisterResult),
    PushProbeAck(PushProbeAckResult),
    PushUnregister(PushUnregisterResult),
    DeviceUnpair(DeviceUnpairResult),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActionStatusResult {
    // Keep the untagged wire shape without embedding a receipt in every status variant.
    Known(Box<KnownActionStatus>),
    Unknown(UnknownActionStatus),
}
impl WireValidate for ActionStatusResult {
    fn validate(&self) -> Validation {
        match self {
            Self::Known(x) => x.validate(),
            Self::Unknown(x) => x.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum EventBody {
    #[serde(rename = "controls.changed")]
    ControlsChanged(ControlsChangedEvent),
    #[serde(rename = "queue.changed")]
    QueueChanged(QueueChangedEvent),
    #[serde(rename = "grid.begin")]
    GridBegin(GridBeginEvent),
    #[serde(rename = "grid.part")]
    GridPart(GridPart),
    #[serde(rename = "grid.commit")]
    GridCommit(GridCommit),
    #[serde(rename = "host.changed")]
    HostChanged(HostChangedEvent),
    #[serde(rename = "session.state")]
    SessionState(SessionStateEvent),
    #[serde(rename = "history.upsert")]
    HistoryUpsert(HistoryUpsertEvent),
    #[serde(rename = "activity.update")]
    ActivityUpdate(ActivityUpdateEvent),
    #[serde(rename = "request.opened")]
    RequestOpened(RequestOpenedEvent),
    #[serde(rename = "request.closed")]
    RequestClosed(RequestClosedEvent),
    #[serde(rename = "action.status")]
    ActionStatus(ActionStatusEvent),
    #[serde(rename = "notification.upsert")]
    NotificationUpsert(NotificationUpsertEvent),
    #[serde(rename = "notification.read")]
    NotificationRead(NotificationReadEvent),
    #[serde(rename = "sync.required")]
    SyncRequired(SyncRequiredEvent),
}
impl WireValidate for EventBody {
    fn validate(&self) -> Validation {
        match self {
            Self::ControlsChanged(x) => x.validate(),
            Self::QueueChanged(x) => x.validate(),
            Self::GridBegin(x) => x.validate(),
            Self::GridPart(x) => x.validate(),
            Self::GridCommit(x) => x.validate(),
            Self::HostChanged(x) => x.validate(),
            Self::SessionState(x) => x.validate(),
            Self::HistoryUpsert(x) => x.validate(),
            Self::ActivityUpdate(x) => x.validate(),
            Self::RequestOpened(x) => x.validate(),
            Self::RequestClosed(x) => x.validate(),
            Self::ActionStatus(x) => x.validate(),
            Self::NotificationUpsert(x) => x.validate(),
            Self::NotificationRead(x) => x.validate(),
            Self::SyncRequired(x) => x.validate(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    New,
    Existing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationCheckoutResources {
    pub kind: CheckoutKind,
    pub branch: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub requested_base: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub resolved_commit: Option<Text>,
}
impl WireValidate for CreationCheckoutResources {
    fn validate(&self) -> Validation {
        validate_worktree_text(&self.branch, 1024, false)?;
        self.requested_base
            .as_ref()
            .map(|base| validate_worktree_text(base, 1024, false))
            .transpose()?;
        ensure((self.kind == CheckoutKind::New) == self.requested_base.is_some())?;
        if let Some(commit) = &self.resolved_commit {
            ensure(
                [40, 64].contains(&commit.len())
                    && commit
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListParams {
    pub target: Target,
}
impl WireValidate for SkillsListParams {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: Text,
    pub description: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub path: Option<Text>,
    pub scope: Text,
    pub enabled: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub display_name: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub short_description: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub command_name: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub document_ref: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub invocation_ref: Option<Uuid>,
}
/// Pinned producer grammar: exact linked mentions only; no escaping or bare-name fallback.
pub fn skill_insert_text(skill: &SkillEntry) -> Option<String> {
    if let Some(command) = &skill.command_name {
        let name = command.strip_prefix("skill:")?;
        return (skill.enabled
            && !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
        .then(|| format!("/{command}"));
    }
    let name = skill.name.as_str();
    let path = skill.path.as_deref()?;
    if !skill.enabled
        || name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b))
        || [
            "PATH",
            "HOME",
            "USER",
            "SHELL",
            "PWD",
            "TMPDIR",
            "TEMP",
            "TMP",
            "LANG",
            "TERM",
            "XDG_CONFIG_HOME",
        ]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
        || !path.starts_with('/')
        || path.trim() != path
        || path.contains(')')
        || path.chars().any(char::is_control)
    {
        return None;
    }
    Some(format!("[${name}]({path})"))
}
impl WireValidate for SkillEntry {
    fn validate(&self) -> Validation {
        self.name.validate()?;
        self.description.validate()?;
        self.path.validate()?;
        self.scope.validate()?;
        self.display_name.validate()?;
        self.short_description.validate()?;
        self.command_name.validate()?;
        self.document_ref.validate()?;
        self.invocation_ref.validate()?;
        ensure(self.invocation_ref.is_none() || self.enabled && self.command_name.is_none())?;
        if self.command_name.as_ref().is_some_and(|command| {
            command.is_empty() || command.len() > 256 || command.chars().any(char::is_control)
        }) || self.document_ref.is_some() && !self.enabled
        {
            return Err(invalid());
        }
        if self.name.is_empty()
            || self.name.len() > 256
            || self.description.len() > 16384
            || self
                .path
                .as_ref()
                .is_some_and(|path| path.is_empty() || path.len() > 4096)
            || !["user", "repo", "system", "admin", "enabled"].contains(&self.scope.as_str())
            || self.display_name.as_ref().is_some_and(|s| s.len() > 1024)
            || self
                .short_description
                .as_ref()
                .is_some_and(|s| s.len() > 4096)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLoadError {
    pub path: Text,
    pub message: Text,
}
impl WireValidate for SkillLoadError {
    fn validate(&self) -> Validation {
        self.path.validate()?;
        self.message.validate()?;
        if self.path.len() > 4096 || self.message.len() > 4096 {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListResult {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub skills: Vec<SkillEntry>,
    pub errors: Vec<SkillLoadError>,
    #[serde(default = "skills_provider_default")]
    pub provider: Provider,
}
fn skills_provider_default() -> Provider {
    Provider::Codex
}
impl WireValidate for SkillsListResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.skills.validate()?;
        self.errors.validate()?;
        let mut invocations = std::collections::HashSet::new();
        if self.skills.iter().any(|entry| {
            entry.invocation_ref.as_ref().is_some_and(|r| {
                self.provider != Provider::Codex || !entry.enabled || !invocations.insert(r)
            })
        }) {
            return Err(invalid());
        }
        let mut document_refs = std::collections::HashSet::new();
        ensure(self.skills.iter().all(|entry| {
            entry
                .document_ref
                .as_ref()
                .is_none_or(|reference| entry.enabled && document_refs.insert(reference))
        }))?;
        if self.skills.iter().any(|entry| match self.provider {
            Provider::Codex => {
                entry.path.is_none() || entry.command_name.is_some() || entry.scope == "enabled"
            }
            Provider::Omp => {
                entry.path.is_some()
                    || !entry.enabled
                    || entry.command_name.is_none()
                    || entry.scope != "enabled"
            }
        }) {
            return Err(invalid());
        }
        if self.provider == Provider::Omp {
            if !self.errors.is_empty() {
                return Err(invalid());
            }
            let mut commands = std::collections::HashSet::new();
            let mut documents = std::collections::HashSet::new();
            if self.skills.iter().any(|entry| {
                entry.command_name.as_ref().is_none_or(|command| {
                    command.strip_prefix("skill:") != Some(entry.name.as_str())
                        || !commands.insert(command)
                }) || entry
                    .document_ref
                    .as_ref()
                    .is_some_and(|reference| !documents.insert(reference))
            }) {
                return Err(invalid());
            }
        }
        if self.skills.len() > 128
            || self.errors.len() > 32
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > 256 * 1024
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPreviewOpenParams {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub document_ref: Uuid,
}
impl WireValidate for SkillsPreviewOpenParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.document_ref.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPreviewOpenResult {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub document_ref: Uuid,
    pub read_id: Uuid,
    pub size: Uint,
    pub sha256: String,
}
impl WireValidate for SkillsPreviewOpenResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.document_ref.validate()?;
        self.read_id.validate()?;
        self.size.validate()?;
        ensure(
            self.size.0 <= 1048576
                && self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPreviewReadParams {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub read_id: Uuid,
    pub offset: Uint,
    pub limit: Uint,
}
impl WireValidate for SkillsPreviewReadParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.read_id.validate()?;
        self.offset.validate()?;
        self.limit.validate()?;
        ensure(self.offset.0 <= 1048576 && self.limit.0 > 0 && self.limit.0 <= 65536)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPreviewReadResult {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub read_id: Uuid,
    pub offset: Uint,
    pub data: Base64Bytes,
    pub eof: bool,
}
impl std::fmt::Debug for SkillsPreviewReadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SkillsPreviewReadResult(redacted)")
    }
}
impl WireValidate for SkillsPreviewReadResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.read_id.validate()?;
        self.offset.validate()?;
        self.data.validate()?;
        ensure(
            self.offset.0 <= 1048576
                && STANDARD.decode(&self.data.0).map_err(|_| invalid())?.len() <= 65536,
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPreviewCloseParams {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub read_id: Uuid,
}
impl WireValidate for SkillsPreviewCloseParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.read_id.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPreviewCloseResult {
    pub closed: bool,
}
impl WireValidate for SkillsPreviewCloseResult {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

/// Ordinary shells never resolve through provider session IDs or history reducers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellTarget {
    pub shell_id: Uuid,
    pub generation: Uuid,
}
impl WireValidate for ShellTarget {
    fn validate(&self) -> Validation {
        self.shell_id.validate()?;
        self.generation.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCapabilities {
    pub read: bool,
    pub create: bool,
    pub input: bool,
    pub close: bool,
    #[serde(default)]
    pub artifacts: bool,
    #[serde(default)]
    pub grid: bool,
}
impl WireValidate for ShellCapabilities {
    fn validate(&self) -> Validation {
        ensure(!self.close && (!(self.create || self.input || self.grid) || self.read))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSummary {
    pub target: ShellTarget,
    pub server_id: Uuid,
    pub workspace_id: Id,
    pub tab_id: Id,
    pub pane_id: Id,
    pub terminal_id: Id,
    pub title: Label,
    pub connectivity: SessionConnectivity,
    pub capabilities: ShellCapabilities,
}
impl WireValidate for ShellSummary {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.server_id.validate()?;
        self.workspace_id.validate()?;
        self.tab_id.validate()?;
        self.pane_id.validate()?;
        self.terminal_id.validate()?;
        self.title.validate()?;
        self.capabilities.validate()?;
        ensure(
            matches!(
                self.connectivity,
                SessionConnectivity::Ready | SessionConnectivity::Offline
            ) && (self.connectivity == SessionConnectivity::Ready
                || (!self.capabilities.read
                    && !self.capabilities.create
                    && !self.capabilities.input
                    && !self.capabilities.artifacts
                    && !self.capabilities.grid)),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellGridSubscribeParams {
    pub target: ShellTarget,
    #[serde(default)]
    pub history: bool,
}
impl WireValidate for ShellGridSubscribeParams {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellGridSubscribeResult {
    pub target: ShellTarget,
    pub subscription: Uuid,
    pub seq: ZeroValue,
    pub frame: GridBegin,
}
impl WireValidate for ShellGridSubscribeResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.subscription.validate()?;
        self.seq.validate()?;
        self.frame.validate()?;
        ensure(self.frame.revision.0 == 0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellResizeParams {
    pub boot_id: Uuid,
    pub target: ShellTarget,
    pub subscription: Uuid,
    pub columns: Uint,
    pub rows: Uint,
}
fn shell_dimensions(columns: &Uint, rows: &Uint) -> Validation {
    ensure(
        (1..=512).contains(&columns.0)
            && (1..=256).contains(&rows.0)
            && columns.0 * rows.0 <= 32768,
    )
}
impl WireValidate for ShellResizeParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.subscription.validate()?;
        shell_dimensions(&self.columns, &self.rows)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellResizeResult {
    pub receipt: Receipt,
}
impl WireValidate for ShellResizeResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultShellResize {
    pub columns: Uint,
    pub rows: Uint,
}
impl WireValidate for ActionResultShellResize {
    fn validate(&self) -> Validation {
        shell_dimensions(&self.columns, &self.rows)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellScrollParams {
    pub boot_id: Uuid,
    pub target: ShellTarget,
    pub subscription: Uuid,
    pub offset_from_bottom: Uint,
}
impl WireValidate for ShellScrollParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.subscription.validate()?;
        self.offset_from_bottom.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellScrollResult {
    pub receipt: Receipt,
}
impl WireValidate for ShellScrollResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()?;
        ensure(self.receipt.method == ActionMethod::ShellScroll)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCreationResources {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub server_id: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub workspace_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub tab_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub pane_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub terminal_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub created_target: Option<ShellTarget>,
}
impl ShellCreationResources {
    pub fn complete(&self) -> bool {
        self.server_id.is_some()
            && self.workspace_id.is_some()
            && self.tab_id.is_some()
            && self.pane_id.is_some()
            && self.terminal_id.is_some()
    }
}
impl WireValidate for ShellCreationResources {
    fn validate(&self) -> Validation {
        self.server_id.validate()?;
        self.workspace_id.validate()?;
        self.tab_id.validate()?;
        self.pane_id.validate()?;
        self.terminal_id.validate()?;
        self.created_target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultShellCreation {
    pub resources: ShellCreationResources,
}
impl WireValidate for ActionResultShellCreation {
    fn validate(&self) -> Validation {
        self.resources.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShellInput {
    Text { text: String },
    Keys { events: Vec<ShellKeyEvent> },
}
impl WireValidate for ShellInput {
    fn validate(&self) -> Validation {
        match self {
            Self::Text { text } => ensure(
                !text.is_empty()
                    && text.len() <= 16384
                    && text
                        .chars()
                        .all(|c| !c.is_control() || c == '\n' || c == '\t'),
            ),
            Self::Keys { events } => {
                ensure(!events.is_empty() && events.len() <= 32)?;
                for event in events {
                    event.validate()?;
                }
                Ok(())
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "key",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ShellKey {
    Character(char),
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellKeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    #[serde(rename = "super")]
    pub super_key: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellKeyPhase {
    Press,
    Repeat,
    Release,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellKeyEvent {
    pub key: ShellKey,
    pub modifiers: ShellKeyModifiers,
    pub phase: ShellKeyPhase,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub physical_key_id: Option<Uint>,
}
impl WireValidate for ShellKeyEvent {
    fn validate(&self) -> Validation {
        if let ShellKey::Character(c) = self.key {
            ensure(!c.is_control())?;
        }
        self.physical_key_id.validate()?;
        ensure(
            self.physical_key_id
                .is_none_or(|id| id.0 <= u64::from(u32::MAX)),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInputParams {
    pub boot_id: Uuid,
    pub target: ShellTarget,
    pub subscription: Uuid,
    pub input: ShellInput,
}
impl WireValidate for ShellInputParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.subscription.validate()?;
        self.input.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInputResult {
    pub receipt: Receipt,
}
impl WireValidate for ShellInputResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInputProgress {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub encoded_bytes: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub written_bytes: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub flush_completed: Option<bool>,
}
impl ShellInputProgress {
    pub fn accepted(&self) -> bool {
        self.encoded_bytes
            .is_some_and(|n| n > 0 && self.written_bytes == Some(n))
            && self.flush_completed == Some(true)
    }
}
impl WireValidate for ShellInputProgress {
    fn validate(&self) -> Validation {
        ensure(
            self.encoded_bytes.is_none_or(|n| n > 0 && n <= 16396)
                && self
                    .written_bytes
                    .is_none_or(|n| self.encoded_bytes.is_some_and(|total| n <= total))
                && (self.flush_completed != Some(true) || self.accepted()),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultShellInput {
    pub progress: ShellInputProgress,
}
impl WireValidate for ActionResultShellInput {
    fn validate(&self) -> Validation {
        self.progress.validate()
    }
}

/// Full native socket frame progress, including its four-byte length prefix; never PTY bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellDispatchProgress {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub frame_bytes: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub socket_bytes_written: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub frame_complete: Option<bool>,
}
impl ShellDispatchProgress {
    pub fn complete(&self) -> bool {
        self.frame_bytes
            .is_some_and(|total| total > 4 && self.socket_bytes_written == Some(total))
            && self.frame_complete == Some(true)
    }
    pub(crate) fn retains(&self, old: &Self) -> bool {
        old.frame_bytes
            .is_none_or(|total| self.frame_bytes == Some(total))
            && old.socket_bytes_written.is_none_or(|written| {
                self.socket_bytes_written
                    .is_some_and(|next| next >= written)
            })
            && (old.frame_complete != Some(true) || self.frame_complete == Some(true))
    }
}
impl WireValidate for ShellDispatchProgress {
    fn validate(&self) -> Validation {
        ensure(
            self.frame_bytes
                .is_none_or(|total| total > 4 && total <= 2_097_156)
                && self
                    .socket_bytes_written
                    .is_none_or(|written| self.frame_bytes.is_some_and(|total| written <= total))
                && (self.frame_complete != Some(true) || self.complete()),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultShellDispatch {
    pub progress: ShellDispatchProgress,
}
impl WireValidate for ActionResultShellDispatch {
    fn validate(&self) -> Validation {
        self.progress.validate()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRemovalProgress {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub effect_started: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub git_remove_succeeded: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub checkout_absent: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub registration_absent: Option<bool>,
}
impl WorktreeRemovalProgress {
    pub fn complete(&self) -> bool {
        self.effect_started == Some(true)
            && self.git_remove_succeeded == Some(true)
            && self.checkout_absent == Some(true)
            && self.registration_absent == Some(true)
    }
    pub fn retains(&self, old: &Self) -> bool {
        let keeps = |old: Option<bool>, new: Option<bool>| {
            old.is_none() || new.is_some() && (old != Some(true) || new == Some(true))
        };
        keeps(old.effect_started, self.effect_started)
            && keeps(old.git_remove_succeeded, self.git_remove_succeeded)
            && keeps(old.checkout_absent, self.checkout_absent)
            && keeps(old.registration_absent, self.registration_absent)
    }
}
impl WireValidate for WorktreeRemovalProgress {
    fn validate(&self) -> Validation {
        ensure(self.git_remove_succeeded != Some(true) || self.effect_started == Some(true))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultWorktreeRemoval {
    pub target: Target,
    pub candidate_id: Uuid,
    pub preview_id: Uuid,
    pub progress: WorktreeRemovalProgress,
}
impl WireValidate for ActionResultWorktreeRemoval {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.candidate_id.validate()?;
        self.preview_id.validate()?;
        self.progress.validate()
    }
}

fn attachment_digest(value: &str) -> Validation {
    ensure(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
    )
}
fn attachment_label(value: &str) -> Validation {
    ensure(!value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control))
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentsBeginParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub upload_id: Uuid,
    pub display_name: Text,
    pub media_type: Text,
    pub size: Uint,
    pub sha256: Text,
}
impl std::fmt::Debug for AttachmentsBeginParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttachmentsBeginParams(redacted)")
    }
}
impl WireValidate for AttachmentsBeginParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.upload_id.validate()?;
        attachment_label(&self.display_name)?;
        attachment_label(&self.media_type)?;
        ensure((1..=8 * 1024 * 1024).contains(&self.size.0))?;
        ensure(!self.display_name.contains(['/', '\\']))?;
        attachment_digest(&self.sha256)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentsChunkParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub upload_id: Uuid,
    pub offset: Uint,
    pub data: Text,
    pub sha256: Text,
}
impl std::fmt::Debug for AttachmentsChunkParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttachmentsChunkParams(redacted)")
    }
}
impl WireValidate for AttachmentsChunkParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.upload_id.validate()?;
        ensure(self.offset.0 < 8 * 1024 * 1024 && self.data.len() <= 174764)?;
        let bytes = STANDARD.decode(&self.data).map_err(|_| invalid())?;
        ensure(
            !bytes.is_empty()
                && bytes.len() <= 128 * 1024
                && STANDARD.encode(&bytes) == self.data
                && crate::pairing::hex(&sha256(&bytes)) == self.sha256,
        )?;
        attachment_digest(&self.sha256)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentsCompleteParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub upload_id: Uuid,
}
impl WireValidate for AttachmentsCompleteParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.upload_id.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentsDiscardParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub upload_id: Uuid,
}
impl WireValidate for AttachmentsDiscardParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.upload_id.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentsStatusParams {
    pub target: Target,
    pub upload_id: Uuid,
}
impl WireValidate for AttachmentsStatusParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.upload_id.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentProgress {
    pub upload_id: Uuid,
    pub committed_bytes: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub expires_at: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub provider_path: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub published: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub discarded: Option<bool>,
}
impl AttachmentProgress {
    pub fn retains(&self, old: &Self) -> bool {
        self.upload_id == old.upload_id
            && self.committed_bytes >= old.committed_bytes
            && old.expires_at.as_ref().is_none_or(|x| {
                self.expires_at.as_ref() == Some(x)
                    || old.published != Some(true)
                        && self.published == Some(true)
                        && self.provider_path.is_some()
                        && self.expires_at.as_ref().is_some_and(|new| new > x)
            })
            && old
                .provider_path
                .as_ref()
                .is_none_or(|x| self.provider_path.as_ref() == Some(x))
            && old
                .published
                .is_none_or(|x| self.published == Some(x) || !x && self.published == Some(true))
            && old
                .discarded
                .is_none_or(|x| self.discarded == Some(x) || !x && self.discarded == Some(true))
    }
}
impl WireValidate for AttachmentProgress {
    fn validate(&self) -> Validation {
        self.upload_id.validate()?;
        ensure(self.committed_bytes.0 <= 8 * 1024 * 1024)?;
        self.expires_at.validate()?;
        if let Some(path) = &self.provider_path {
            ensure(
                path.len() <= 4096
                    && path.starts_with('/')
                    && !path.chars().any(char::is_control)
                    && self.published == Some(true),
            )?
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultAttachment {
    pub target: Target,
    pub progress: AttachmentProgress,
}
impl WireValidate for ActionResultAttachment {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.progress.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentUploadState {
    Receiving,
    Complete,
    Discarded,
    Uncertain,
}
impl WireValidate for AttachmentUploadState {
    fn validate(&self) -> Validation {
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImagePreparationReason {
    PendingOriginalRead,
    OriginalSourceUnavailable,
    OwnershipProofUnavailable,
    CapacityInUse,
}
impl WireValidate for ImagePreparationReason {
    fn validate(&self) -> Validation {
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeImagePreparation {
    pub reason: ImagePreparationReason,
    pub actions: Vec<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub peer_owners: Option<Uint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub host_owners: Option<Uint>,
}
impl WireValidate for NativeImagePreparation {
    fn validate(&self) -> Validation {
        self.actions.validate()?;
        self.peer_owners.validate()?;
        self.host_owners.validate()?;
        let mut unique = std::collections::HashSet::new();
        ensure(self.actions.len() <= 4 && self.actions.iter().all(|action| unique.insert(action)))?;
        ensure(
            self.reason == ImagePreparationReason::OwnershipProofUnavailable
                || self.peer_owners.is_some()
                    && self.host_owners.is_some()
                    && (self.reason == ImagePreparationReason::CapacityInUse
                        || !self.actions.is_empty()),
        )?;
        ensure(
            self.peer_owners
                .is_none_or(|count| count.0 <= 4 && count.0 >= self.actions.len() as u64)
                && self.host_owners.is_none_or(|count| {
                    count.0 <= 16 && self.peer_owners.is_none_or(|peer| peer <= count)
                }),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentsStatusResult {
    #[serde(default)]
    pub insertion_ready: bool,
    pub target: Target,
    pub progress: AttachmentProgress,
    pub state: AttachmentUploadState,
    pub size: Uint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_preparation: Option<NativeImagePreparation>,
}
impl WireValidate for AttachmentsStatusResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.progress.validate()?;
        self.native_preparation.validate()?;
        ensure(
            self.size.0 > 0
                && self.size.0 <= 8 * 1024 * 1024
                && self.progress.committed_bytes <= self.size,
        )?;
        ensure(
            !self.insertion_ready
                || self.state == AttachmentUploadState::Complete
                    && self.progress.published == Some(true)
                    && self.progress.discarded != Some(true)
                    && self.progress.provider_path.is_some(),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentsResult {
    pub receipt: Receipt,
}
impl WireValidate for AttachmentsResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()
    }
}
impl RequestBody {
    pub fn attachment_action(&self) -> Option<(ActionMethod, &Uuid, &Target, &Uuid)> {
        match self {
            Self::AttachmentsBegin(p) => Some((
                ActionMethod::AttachmentsBegin,
                &p.boot_id,
                &p.target,
                &p.upload_id,
            )),
            Self::AttachmentsChunk(p) => Some((
                ActionMethod::AttachmentsChunk,
                &p.boot_id,
                &p.target,
                &p.upload_id,
            )),
            Self::AttachmentsComplete(p) => Some((
                ActionMethod::AttachmentsComplete,
                &p.boot_id,
                &p.target,
                &p.upload_id,
            )),
            Self::AttachmentsDiscard(p) => Some((
                ActionMethod::AttachmentsDiscard,
                &p.boot_id,
                &p.target,
                &p.upload_id,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalCreateParams {
    pub boot_id: Uuid,
    pub server_id: Uuid,
    pub workspace_id: Id,
    pub project_id: Uuid,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub directory_ref: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub source: Option<ArtifactSource>,
}
impl WireValidate for TerminalCreateParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.server_id.validate()?;
        self.workspace_id.validate()?;
        self.project_id.validate()?;
        self.directory_ref.validate()?;
        self.source.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateResult {
    pub receipt: Receipt,
}
impl WireValidate for TerminalCreateResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()
    }
}

/// An exact original source, never a phone-selected path or native process proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactSource {
    Session { target: Target },
    Shell { target: ShellTarget },
}
impl WireValidate for ArtifactSource {
    fn validate(&self) -> Validation {
        match self {
            Self::Session { target } => target.validate(),
            Self::Shell { target } => target.validate(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Directory,
    File,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPreviewKind {
    Text,
    Binary,
    Unavailable,
}
fn artifact_name(name: &str) -> Validation {
    ensure(!name.is_empty() && name.len() <= 255 && !name.chars().any(char::is_control))
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactEntry {
    pub reference: Uuid,
    pub name: String,
    pub kind: ArtifactKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Uint>,
}
impl WireValidate for ArtifactEntry {
    fn validate(&self) -> Validation {
        self.reference.validate()?;
        artifact_name(&self.name)?;
        self.size.validate()?;
        ensure((self.kind == ArtifactKind::File) == self.size.is_some())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactsListParams {
    pub source: ArtifactSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Uuid>,
    pub limit: Uint,
}
impl WireValidate for ArtifactsListParams {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.parent.validate()?;
        self.cursor.validate()?;
        self.limit.validate()?;
        ensure(self.parent.is_none() || self.cursor.is_none())?;
        ensure(self.limit.0 > 0 && self.limit.0 <= 100)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsListResult {
    pub source: ArtifactSource,
    pub entries: Vec<ArtifactEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<Uuid>,
}
impl WireValidate for ArtifactsListResult {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.entries.validate()?;
        self.next_cursor.validate()?;
        ensure(self.entries.len() <= 100)?;
        let mut refs = std::collections::HashSet::new();
        ensure(
            self.entries
                .iter()
                .all(|entry| refs.insert(&entry.reference)),
        )
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactsOpenParams {
    pub source: ArtifactSource,
    pub file: Uuid,
}
impl WireValidate for ArtifactsOpenParams {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.file.validate()
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsOpenResult {
    pub source: ArtifactSource,
    pub read_id: Uuid,
    pub name: String,
    pub size: Uint,
    pub sha256: String,
    pub preview_kind: ArtifactPreviewKind,
}
impl WireValidate for ArtifactsOpenResult {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.read_id.validate()?;
        artifact_name(&self.name)?;
        self.size.validate()?;
        ensure(
            (self.preview_kind != ArtifactPreviewKind::Text || self.size.0 <= 1024 * 1024)
                && self.size.0 <= 32 * 1024 * 1024
                && self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        )
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactsReadParams {
    pub source: ArtifactSource,
    pub read_id: Uuid,
    pub offset: Uint,
    pub limit: Uint,
}
impl WireValidate for ArtifactsReadParams {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.read_id.validate()?;
        self.offset.validate()?;
        self.limit.validate()?;
        ensure(self.offset.0 <= 32 * 1024 * 1024 && self.limit.0 > 0 && self.limit.0 <= 65536)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsReadResult {
    pub source: ArtifactSource,
    pub read_id: Uuid,
    pub offset: Uint,
    pub data: Base64Bytes,
    pub eof: bool,
}
impl WireValidate for ArtifactsReadResult {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.read_id.validate()?;
        self.offset.validate()?;
        self.data.validate()?;
        ensure(self.offset.0 <= 32 * 1024 * 1024)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactsCloseParams {
    pub source: ArtifactSource,
    pub read_id: Uuid,
}
impl WireValidate for ArtifactsCloseParams {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.read_id.validate()
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsCloseResult {
    pub source: ArtifactSource,
    pub read_id: Uuid,
    pub closed: bool,
}
impl WireValidate for ArtifactsCloseResult {
    fn validate(&self) -> Validation {
        self.source.validate()?;
        self.read_id.validate()
    }
}
impl std::fmt::Debug for ArtifactEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactEntry(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsListParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsListParams(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsListResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsListResult(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsOpenParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsOpenParams(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsOpenResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsOpenResult(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsReadParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsReadParams(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsReadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsReadResult(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsCloseParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsCloseParams(redacted)")
    }
}
impl std::fmt::Debug for ArtifactsCloseResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArtifactsCloseResult(redacted)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlMode {
    Default,
    Plan,
}
impl WireValidate for ControlMode {
    fn validate(&self) -> Validation {
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptDelivery {
    Native,
    Steer,
    FollowUp,
    Aside,
    Queue,
}
impl WireValidate for PromptDelivery {
    fn validate(&self) -> Validation {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlModel {
    pub reference: Uuid,
    pub label: Label,
    pub efforts: Vec<Text>,
}
impl WireValidate for SessionControlModel {
    fn validate(&self) -> Validation {
        self.reference.validate()?;
        self.label.validate()?;
        self.efforts.validate()?;
        ensure(
            self.efforts.len() <= 64
                && self
                    .efforts
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !self.efforts[..index].contains(value)),
        )
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlsCurrent {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub model_ref: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub effort: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub mode: Option<ControlMode>,
}
impl WireValidate for SessionControlsCurrent {
    fn validate(&self) -> Validation {
        self.model_ref.validate()?;
        self.effort.validate()?;
        self.mode.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlsReadParams {
    pub target: Target,
}
impl WireValidate for SessionControlsReadParams {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlsReadResult {
    pub target: Target,
    pub catalogue_generation: Uuid,
    pub models: Vec<SessionControlModel>,
    pub current: SessionControlsCurrent,
    pub modes: Vec<ControlMode>,
    pub deliveries: Vec<PromptDelivery>,
    pub queue_available: bool,
}
impl WireValidate for SessionControlsReadResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.models.validate()?;
        self.current.validate()?;
        self.modes.validate()?;
        self.deliveries.validate()?;
        let mut references = std::collections::HashSet::new();
        ensure(
            self.models.len() <= 256
                && self
                    .models
                    .iter()
                    .all(|model| references.insert(&model.reference)),
        )?;
        ensure(
            self.current
                .model_ref
                .as_ref()
                .is_none_or(|reference| references.contains(reference)),
        )?;
        ensure(
            self.modes.len() <= 2
                && self
                    .modes
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !self.modes[..index].contains(value)),
        )?;
        ensure(
            self.deliveries.len() <= 5
                && self
                    .deliveries
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !self.deliveries[..index].contains(value)),
        )?;
        let members = 13
            + self.models.len() * 4
            + self
                .models
                .iter()
                .map(|model| model.efforts.len())
                .sum::<usize>()
            + usize::from(self.current.model_ref.is_some())
            + usize::from(self.current.effort.is_some())
            + usize::from(self.current.mode.is_some())
            + self.modes.len()
            + self.deliveries.len();
        ensure(members <= JSON_MEMBERS_MAX)?;
        encoded_limit(self, 256 * 1024)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlsApplyParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub catalogue_generation: Uuid,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub model_ref: Option<Uuid>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub effort: Option<Text>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub mode: Option<ControlMode>,
}
impl WireValidate for SessionControlsApplyParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.catalogue_generation.validate()?;
        self.model_ref.validate()?;
        self.effort.validate()?;
        self.mode.validate()?;
        ensure(self.model_ref.is_some() || self.effort.is_some() || self.mode.is_some())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlsApplyResult {
    pub receipt: Receipt,
}
impl WireValidate for SessionControlsApplyResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlsChangedEvent {
    pub target: Target,
}
impl WireValidate for ControlsChangedEvent {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueChangedEvent {
    pub target: Target,
}
impl WireValidate for QueueChangedEvent {
    fn validate(&self) -> Validation {
        self.target.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueListParams {
    pub target: Target,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub cursor: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub limit: Option<Uint>,
}
impl WireValidate for SessionQueueListParams {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.cursor.validate()?;
        self.limit.validate()?;
        ensure(self.limit.is_none_or(|limit| limit.0 > 0 && limit.0 <= 100))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueEntry {
    pub id: Id,
    pub text: Content,
    pub editable: bool,
}
impl WireValidate for SessionQueueEntry {
    fn validate(&self) -> Validation {
        self.id.validate()?;
        self.text.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueListResult {
    pub target: Target,
    pub entries: Vec<SessionQueueEntry>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub next_cursor: Option<Id>,
}
impl WireValidate for SessionQueueListResult {
    fn validate(&self) -> Validation {
        self.target.validate()?;
        self.entries.validate()?;
        self.next_cursor.validate()?;
        let mut ids = std::collections::HashSet::new();
        ensure(
            self.entries.len() <= 100 && self.entries.iter().all(|entry| ids.insert(&entry.id)),
        )?;
        encoded_limit(self, 256 * 1024)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueUpdateParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub entry_id: Id,
    pub text: PromptText,
}
impl WireValidate for SessionQueueUpdateParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.entry_id.validate()?;
        self.text.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueDeleteParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub entry_id: Id,
}
impl WireValidate for SessionQueueDeleteParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.entry_id.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueStartParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub entry_id: Id,
}
impl WireValidate for SessionQueueStartParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.entry_id.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueReorderParams {
    pub boot_id: Uuid,
    pub target: Target,
    pub entry_ids: Vec<Id>,
}
impl WireValidate for SessionQueueReorderParams {
    fn validate(&self) -> Validation {
        self.boot_id.validate()?;
        self.target.validate()?;
        self.entry_ids.validate()?;
        let mut ids = std::collections::HashSet::new();
        ensure(self.entry_ids.len() <= 128 && self.entry_ids.iter().all(|id| ids.insert(id)))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQueueResult {
    pub receipt: Receipt,
}
impl WireValidate for SessionQueueResult {
    fn validate(&self) -> Validation {
        self.receipt.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultQueue {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub entry_id: Option<Id>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub deleted: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub native_turn_id: Option<Id>,
}
impl WireValidate for ActionResultQueue {
    fn validate(&self) -> Validation {
        self.entry_id.validate()?;
        self.deleted.validate()?;
        self.native_turn_id.validate()?;
        ensure(
            (self.deleted.is_none() && self.native_turn_id.is_none() || self.entry_id.is_some())
                && !(self.deleted.is_some() && self.native_turn_id.is_some()),
        )
    }
}
